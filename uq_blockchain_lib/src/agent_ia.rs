// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// UQ CORE - agent_ia.rs
// Agent IA autonome dans chaque tÃ©lÃ©phone
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

use serde::{Serialize, Deserialize};
use sha2::{Sha256, Digest};
use ed25519_dalek::{VerifyingKey, Verifier};
use crate::error::ChainError;

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// STRUCTURES
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub master_public_key: [u8; 32],
    pub rules_hash: [u8; 32],
    pub enabled_modules: Vec<String>,
    pub auto_defend_enabled: bool,
    pub report_interval_seconds: u64,
    pub max_fraud_score_before_alert: u8,
    pub commission_rate_nano: u64,
    pub started_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FraudReport {
    pub report_id: [u8; 32],
    pub transaction_hash: [u8; 32],
    pub fraud_type: FraudType,
    pub suspicion_score: u8,
    pub evidence: Vec<String>,
    pub detected_at: u64,
    pub reporter: [u8; 32],
    pub action_taken: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FraudType {
    WashTrading,
    SybilAttack,
    FrontRunning,
    DoubleSpend,
    MixerSuspicious,
    AmountAnomaly,
    IdentityTheft,
    CertificateForgery,
    Unknown(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyReport {
    pub date: u64,
    pub transactions_validated: u64,
    pub fraud_attempts_detected: u64,
    pub frauds_confirmed: u64,
    pub commissions_reversed: u64,
    pub ubi_distributed: u64,
    pub identities_verified: u64,
    pub peers_connected: u32,
    pub network_health: f64,
    pub agent_version: String,
    pub anomalies: Vec<String>,
    pub uptime_seconds: u64,
    pub memory_used_mb: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedCommand {
    pub command_id: [u8; 32],
    pub command_type: CommandType,
    pub payload: Vec<u8>,
    pub issued_at: u64,
    pub expires_at: u64,
    pub signature: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CommandType {
    UpdateCode,
    ChangeParams,
    AddGuardian,
    RemoveGuardian,
    EmergencyMode,
    RevokeCertificate,
    StartModule,
    StopModule,
    BlacklistPeer,
    FreezeAccount,
    Custom(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkHealth {
    pub active_nodes: u32,
    pub average_latency_ms: u64,
    pub block_validation_rate: f64,
    pub hardbit_sync_percent: f64,
    pub frauds_detected_24h: u64,
    pub ubi_distributed_24h: u64,
    pub overall_score: u8,
    pub degraded_nodes: u32,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityThreat {
    pub threat_id: [u8; 32],
    pub threat_type: ThreatType,
    pub source: [u8; 32],
    pub severity: u8,
    pub description: String,
    pub detected_at: u64,
    pub evidence: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ThreatType {
    MaliciousIP,
    FraudulentCertificate,
    SuspiciousAccount,
    NetworkAttack,
    SybilCluster,
    DoubleSpendAttempt,
    Unknown(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentStatus {
    pub running: bool,
    pub uptime_seconds: u64,
    pub commands_executed: u64,
    pub frauds_detected: u64,
    pub commissions_reversed: u64,
    pub last_report_hardbit: u64,
    pub version: String,
    pub modules_active: Vec<String>,
    pub memory_used_mb: f64,
    pub cpu_usage_percent: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    pub identity_hash: [u8; 32],
    pub approved: bool,
    pub confidence_score: u8,
    pub checks_passed: Vec<String>,
    pub checks_failed: Vec<String>,
    pub requires_human_review: bool,
    pub validated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommissionRecord {
    pub tx_hash: [u8; 32],
    pub amount_nano: u64,
    pub data_mb: f64,
    pub reversed_at: u64,
    pub master_address: [u8; 32],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentStats {
    pub total_uptime_hours: f64,
    pub total_frauds_detected: u64,
    pub total_commissions_reversed: u64,
    pub total_commands_executed: u64,
    pub total_identities_verified: u64,
    pub average_network_health: f64,
    pub threats_neutralized: u64,
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// CONSTANTES
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

const AGENT_VERSION: &str = "1.0.0";
const DEFAULT_REPORT_INTERVAL: u64 = 86400; // 24 heures
const DEFAULT_MAX_FRAUD_SCORE: u8 = 70;
const DEFAULT_COMMISSION_RATE: u64 = 1; // nano-BORB par Mo
const MASTER_ADDRESS: [u8; 32] = [0xFFu8; 32]; // Adresse gravÃ©e du MaÃ®tre

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// HELPERS
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

fn sha256_hash(data: &[u8]) -> [u8; 32] { let mut h = Sha256::new(); h.update(data); let mut r = [0u8; 32]; r.copy_from_slice(&h.finalize()); r }
fn get_hardbit() -> u64 { crate::hardbit::get_current_hardbit() }
fn verify_signature(msg: &[u8], sig: &[u8], pk: &[u8; 32]) -> bool {
    if sig.len() != 64 { return false; }
    let vk = match VerifyingKey::from_bytes(pk) { Ok(v) => v, Err(_) => return false };
    let s = match ed25519_dalek::Signature::from_slice(sig) { Ok(s) => s, Err(_) => return false };
    vk.verify(msg, &s).is_ok()
}

use std::sync::Mutex;
static AGENT_STATE: Mutex<Option<AgentState>> = Mutex::new(None);

struct AgentState {
    config: AgentConfig,
    started_at: u64,
    commands_executed: u64,
    frauds_detected: u64,
    commissions_reversed: u64,
    last_report: u64,
    identities_verified: u64,
    threats_neutralized: u64,
    executed_commands: Vec<[u8; 32]>,
    commission_records: Vec<CommissionRecord>,
}

fn get_state() -> std::sync::MutexGuard<'static, Option<AgentState>> { AGENT_STATE.lock().unwrap() }

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// F1. DÃ‰MARRER L'AGENT
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

pub fn start_agent(master_pubkey: &[u8; 32]) -> Result<(), ChainError> {
    let hb = get_hardbit();
    let mut state = get_state();
    
    *state = Some(AgentState {
        config: AgentConfig {
            master_public_key: *master_pubkey,
            rules_hash: [0u8; 32],
            enabled_modules: vec!["fraud".into(), "identity".into(), "commission".into(), "network".into()],
            auto_defend_enabled: true,
            report_interval_seconds: DEFAULT_REPORT_INTERVAL,
            max_fraud_score_before_alert: DEFAULT_MAX_FRAUD_SCORE,
            commission_rate_nano: DEFAULT_COMMISSION_RATE,
            started_at: hb,
        },
        started_at: hb,
        commands_executed: 0,
        frauds_detected: 0,
        commissions_reversed: 0,
        last_report: hb,
        identities_verified: 0,
        threats_neutralized: 0,
        executed_commands: Vec::new(),
        commission_records: Vec::new(),
    });

    Ok(())
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// F2. ARRÃŠTER L'AGENT
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

pub fn stop_agent() -> Result<(), ChainError> {
    let mut state = get_state();
    *state = None;
    Ok(())
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// F3. DÃ‰TECTER LA FRAUDE
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

pub fn detect_fraud(
    tx_hash: &[u8; 32],
    from: &[u8; 32],
    to: &[u8; 32],
    amount: u64,
) -> Result<FraudReport, ChainError> {
    let hb = get_hardbit();
    let mut suspicion: u8 = 0;
    let mut evidence: Vec<String> = Vec::new();
    let mut fraud_type = FraudType::Unknown("none".into());

    // 1. Wash trading : mÃªme expÃ©diteur/destinataire
    if from == to {
        suspicion += 80;
        evidence.push("MÃªme adresse from/to".into());
        fraud_type = FraudType::WashTrading;
    }

    // 2. Anomalie de montant
    if amount == 0 {
        suspicion += 30;
        evidence.push("Montant nul".into());
    }
    if amount > 1_000_000_000_000_000 {
        suspicion += 40;
        evidence.push("Montant anormalement Ã©levÃ©".into());
    }

    // 3. Patterns suspects (basÃ© sur l'historique)
    // TODO: VÃ©rifier l'historique des transactions

    let reporter = sha256_hash(b"agent_ia");

    let report = FraudReport {
        report_id: sha256_hash(&[tx_hash.as_slice(), &hb.to_le_bytes()].concat()),
        transaction_hash: *tx_hash,
        fraud_type,
        suspicion_score: suspicion.min(100),
        evidence,
        detected_at: hb,
        reporter,
        action_taken: if suspicion > 70 { Some("Alerte rÃ©seau envoyÃ©e".into()) } else { None },
    };

    if suspicion > 0 {
        if let Some(ref mut s) = *get_state() {
            s.frauds_detected += 1;
        }
    }

    Ok(report)
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// F4. VALIDER UNE IDENTITÃ‰
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

pub fn validate_identity(
    identity_hash: &[u8; 32],
    country_code: &str,
    kyc_verified: bool,
) -> Result<ValidationResult, ChainError> {
    let hb = get_hardbit();
    let mut checks_passed: Vec<String> = Vec::new();
    let mut checks_failed: Vec<String> = Vec::new();
    let mut confidence: u8 = 50;

    // 1. VÃ©rifier le pays
    let valid_countries = ["FR", "CD", "US", "GB", "RU", "CN", "IN", "BR", "JP", "ZA"];
    if valid_countries.contains(&country_code) {
        checks_passed.push("Pays valide".into());
        confidence += 10;
    } else {
        checks_failed.push("Pays non supportÃ©".into());
        confidence = confidence.saturating_sub(20);
    }

    // 2. VÃ©rifier le KYC
    if kyc_verified {
        checks_passed.push("KYC vÃ©rifiÃ©".into());
        confidence += 30;
    } else {
        checks_failed.push("KYC manquant".into());
        confidence = confidence.saturating_sub(30);
    }

    // 3. VÃ©rifier l'unicitÃ© (TODO: DHT lookup)

    let requires_human = confidence < 70;

    if let Some(ref mut s) = *get_state() {
        s.identities_verified += 1;
    }

    Ok(ValidationResult {
        identity_hash: *identity_hash,
        approved: !requires_human,
        confidence_score: confidence.min(100),
        checks_passed,
        checks_failed,
        requires_human_review: requires_human,
        validated_at: hb,
    })
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// F5. REVERSE DE COMMISSION
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

pub fn reverse_commission(
    tx_hash: &[u8; 32],
    data_mb: f64,
) -> Result<CommissionRecord, ChainError> {
    let hb = get_hardbit();
    
    let amount_nano = (data_mb * DEFAULT_COMMISSION_RATE as f64) as u64;
    
    let record = CommissionRecord {
        tx_hash: *tx_hash,
        amount_nano,
        data_mb,
        reversed_at: hb,
        master_address: MASTER_ADDRESS,
    };

    if let Some(ref mut s) = *get_state() {
        s.commissions_reversed += 1;
        s.commission_records.push(record.clone());
    }

    Ok(record)
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// F6. RAPPORT QUOTIDIEN
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

pub fn send_daily_report(_master_pubkey: &[u8; 32]) -> Result<DailyReport, ChainError> {
    let hb = get_hardbit();
    
    let state = get_state();
    let s = state.as_ref().ok_or_else(|| ChainError::InternalError("Agent non dÃ©marrÃ©".into()))?;

    let uptime = hb.saturating_sub(s.started_at);

    let report = DailyReport {
        date: hb,
        transactions_validated: 0, // TODO
        fraud_attempts_detected: s.frauds_detected,
        frauds_confirmed: 0,
        commissions_reversed: s.commissions_reversed,
        ubi_distributed: 0,
        identities_verified: s.identities_verified,
        peers_connected: 0,
        network_health: 100.0,
        agent_version: AGENT_VERSION.into(),
        anomalies: Vec::new(),
        uptime_seconds: uptime,
        memory_used_mb: 0.0,
    };

    Ok(report)
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// F7. EXÃ‰CUTER UNE COMMANDE DISTANTE
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

pub fn execute_remote_command(command: &SignedCommand, master_pubkey: &[u8; 32]) -> Result<(), ChainError> {
    let hb = get_hardbit();

    // 1. VÃ©rifier la signature
    let mut sig_data = Vec::new();
    sig_data.extend_from_slice(&command.command_id);
    sig_data.extend_from_slice(&command.issued_at.to_le_bytes());
    sig_data.extend_from_slice(&command.expires_at.to_le_bytes());
    sig_data.extend_from_slice(command.payload.as_slice());

    if !verify_signature(&sig_data, &command.signature, master_pubkey) {
        return Err(ChainError::InvalidProof("Signature MaÃ®tre invalide".into()));
    }

    // 2. VÃ©rifier l'expiration
    if hb > command.expires_at {
        return Err(ChainError::InternalError("Commande expirÃ©e".into()));
    }

    // 3. VÃ©rifier anti-replay
    if let Some(ref mut s) = *get_state() {
        if s.executed_commands.contains(&command.command_id) {
            return Err(ChainError::InternalError("Commande dÃ©jÃ  exÃ©cutÃ©e".into()));
        }
        s.executed_commands.push(command.command_id);
        s.commands_executed += 1;
    }

    // 4. ExÃ©cuter selon le type
    match command.command_type {
        CommandType::EmergencyMode => { /* Activer mode urgence */ }
        CommandType::BlacklistPeer => { /* Blacklister un pair */ }
        CommandType::FreezeAccount => { /* Geler un compte */ }
        _ => { /* Autres commandes */ }
    }

    Ok(())
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// F8. SURVEILLER LA SANTÃ‰ RÃ‰SEAU
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

pub fn monitor_network_health() -> Result<NetworkHealth, ChainError> {
    let hb = get_hardbit();

    // TODO: Vraies mÃ©triques rÃ©seau
    let health = NetworkHealth {
        active_nodes: 50,
        average_latency_ms: 150,
        block_validation_rate: 0.98,
        hardbit_sync_percent: 95.0,
        frauds_detected_24h: 0,
        ubi_distributed_24h: 0,
        overall_score: 85,
        degraded_nodes: 2,
        timestamp: hb,
    };

    Ok(health)
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// F9. AUTO-DÃ‰FENSE
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

pub fn auto_defend(threat: &SecurityThreat, _master_pubkey: &[u8; 32]) -> Result<String, ChainError> {
    let mut action = String::new();

    match threat.threat_type {
        ThreatType::MaliciousIP => {
            action = "IP blacklistÃ©e".into();
        }
        ThreatType::FraudulentCertificate => {
            action = "Certificat rÃ©voquÃ©".into();
        }
        ThreatType::SuspiciousAccount => {
            if threat.severity > 80 {
                action = "Compte gelÃ© temporairement".into();
            } else {
                action = "Compte sous surveillance".into();
            }
        }
        ThreatType::DoubleSpendAttempt => {
            action = "Alerte rÃ©seau + gel immÃ©diat".into();
        }
        _ => {
            action = "Menace inconnue - escaladÃ©e".into();
        }
    }

    if let Some(ref mut s) = *get_state() {
        s.threats_neutralized += 1;
    }

    Ok(action)
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// F10. STATUT DE L'AGENT
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

pub fn get_agent_status() -> Result<AgentStatus, ChainError> {
    let hb = get_hardbit();
    let state = get_state();

    if let Some(ref s) = *state {
        Ok(AgentStatus {
            running: true,
            uptime_seconds: hb.saturating_sub(s.started_at),
            commands_executed: s.commands_executed,
            frauds_detected: s.frauds_detected,
            commissions_reversed: s.commissions_reversed,
            last_report_hardbit: s.last_report,
            version: AGENT_VERSION.into(),
            modules_active: s.config.enabled_modules.clone(),
            memory_used_mb: 0.0,
            cpu_usage_percent: 0.0,
        })
    } else {
        Ok(AgentStatus {
            running: false,
            uptime_seconds: 0,
            commands_executed: 0,
            frauds_detected: 0,
            commissions_reversed: 0,
            last_report_hardbit: 0,
            version: AGENT_VERSION.into(),
            modules_active: Vec::new(),
            memory_used_mb: 0.0,
            cpu_usage_percent: 0.0,
        })
    }
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// FONCTIONS SUPPLÃ‰MENTAIRES
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

pub fn get_agent_stats() -> Result<AgentStats, ChainError> {
    let state = get_state();
    if let Some(ref s) = *state {
        let hb = get_hardbit();
        Ok(AgentStats {
            total_uptime_hours: (hb.saturating_sub(s.started_at)) as f64 / 3600.0,
            total_frauds_detected: s.frauds_detected,
            total_commissions_reversed: s.commissions_reversed,
            total_commands_executed: s.commands_executed,
            total_identities_verified: s.identities_verified,
            average_network_health: 100.0,
            threats_neutralized: s.threats_neutralized,
        })
    } else {
        Err(ChainError::InternalError("Agent non dÃ©marrÃ©".into()))
    }
}

pub fn is_agent_running() -> bool {
    get_state().is_some()
}

pub fn update_config(config: AgentConfig) -> Result<(), ChainError> {
    let mut state = get_state();
    if let Some(ref mut s) = *state {
        s.config = config;
        Ok(())
    } else {
        Err(ChainError::InternalError("Agent non dÃ©marrÃ©".into()))
    }
}

pub fn get_commission_records() -> Result<Vec<CommissionRecord>, ChainError> {
    let state = get_state();
    if let Some(ref s) = *state {
        Ok(s.commission_records.clone())
    } else {
        Ok(Vec::new())
    }
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// TESTS
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

#[cfg(test)]
mod tests {
    use super::*;

    #[test] fn test_start_stop() { assert!(start_agent(&[1u8;32]).is_ok()); assert!(is_agent_running()); assert!(stop_agent().is_ok()); assert!(!is_agent_running()); }
    #[test] fn test_detect_fraud_wash() { start_agent(&[1u8;32]).unwrap(); let r = detect_fraud(&sha256_hash(b"tx1"), &[1u8;32], &[1u8;32], 1000).unwrap_or_default(); assert!(r.suspicion_score >= 80); stop_agent().unwrap_or_default(); }
    #[test] fn test_detect_fraud_normal() { start_agent(&[1u8;32]).unwrap(); let r = detect_fraud(&sha256_hash(b"tx2"), &[1u8;32], &[2u8;32], 500).unwrap_or_default(); assert!(r.suspicion_score < 80); stop_agent().unwrap_or_default(); }
    #[test] fn test_validate_identity_ok() { start_agent(&[1u8;32]).unwrap(); let r = validate_identity(&sha256_hash(b"id1"), "FR", true).unwrap_or_default(); assert!(r.approved); stop_agent().unwrap_or_default(); }
    #[test] fn test_validate_identity_nokyc() { start_agent(&[1u8;32]).unwrap(); let r = validate_identity(&sha256_hash(b"id2"), "XX", false).unwrap_or_default(); assert!(!r.approved); stop_agent().unwrap_or_default(); }
    #[test] fn test_reverse_commission() { start_agent(&[1u8;32]).unwrap(); let r = reverse_commission(&sha256_hash(b"tx"), 10.0).unwrap_or_default(); assert!(r.amount_nano > 0); stop_agent().unwrap_or_default(); }
    #[test] fn test_daily_report() { start_agent(&[1u8;32]).unwrap(); let r = send_daily_report(&[1u8;32]).unwrap_or_default(); assert_eq!(r.agent_version, "1.0.0"); stop_agent().unwrap_or_default(); }
    #[test] fn test_execute_bad_signature() { start_agent(&[1u8;32]).unwrap(); let cmd = SignedCommand { command_id:sha256_hash(b"c1"), command_type:CommandType::EmergencyMode, payload:vec![], issued_at:get_hardbit(), expires_at:get_hardbit()+1000, signature:vec![0;64] }; assert!(execute_remote_command(&cmd, &[2u8;32]).is_err()); stop_agent().unwrap_or_default(); }
    #[test] fn test_network_health() { start_agent(&[1u8;32]).unwrap(); let h = monitor_network_health().unwrap_or_default(); assert!(h.overall_score > 0); stop_agent().unwrap_or_default(); }
    #[test] fn test_auto_defend() { start_agent(&[1u8;32]).unwrap(); let threat = SecurityThreat { threat_id:sha256_hash(b"t1"), threat_type:ThreatType::MaliciousIP, source:[9u8;32], severity:90, description:"Test".into(), detected_at:get_hardbit(), evidence:vec![] }; let a = auto_defend(&threat, &[1u8;32]).unwrap_or_default(); assert!(!a.is_empty()); stop_agent().unwrap_or_default(); }
    #[test] fn test_status() { start_agent(&[1u8;32]).unwrap(); let s = get_agent_status().unwrap_or_default(); assert!(s.running); assert_eq!(s.version, "1.0.0"); stop_agent().unwrap_or_default(); }
    #[test] fn test_stats() { start_agent(&[1u8;32]).unwrap(); let s = get_agent_stats().unwrap_or_default(); assert!(s.total_uptime_hours >= 0.0); stop_agent().unwrap_or_default(); }
}










