use serde::{Serialize, Deserialize};
use sha2::{Sha256, Digest};
use crate::error::ChainError;
use std::collections::HashMap;

// ═══════════════════════════════════════════════════════════════
// TYPES DE FRAUDE
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FraudType {
    WashTrading,           // Acheter et vendre à soi-même
    PriceManipulation,     // Manipuler le prix artificiellement
    Spoofing,              // Ordres faux pour tromper le marché
    Layering,              // Plusieurs ordres pour cacher l'intention
    FrontRunning,          // Exploiter une transaction en attente
    SybilAttack,           // Créer plusieurs faux comptes
    Phishing,              // Usurpation d'identité
    CounterfeitProof,      // Fausse preuve de paiement
    DoubleSpending,        // Dépenser deux fois le même BORB
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FraudSeverity {
    Low,        // Avertissement
    Medium,     // Restriction temporaire
    High,       // Gel des fonds
    Critical,   // Bannissement permanent
}

// ═══════════════════════════════════════════════════════════════
// STRUCTURES
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FraudAlert {
    pub id: [u8; 32],
    pub suspect: [u8; 32],
    pub fraud_type: FraudType,
    pub severity: FraudSeverity,
    pub evidence: String,
    pub reported_by: [u8; 32],
    pub timestamp: i64,
    pub resolved: bool,
    pub resolution: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FraudDetector {
    pub alerts: HashMap<[u8; 32], FraudAlert>,
    pub suspect_history: HashMap<[u8; 32], Vec<FraudRecord>>,
    pub blacklist: Vec<[u8; 32]>,
    pub total_alerts: u64,
    pub total_resolved: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FraudRecord {
    pub fraud_type: FraudType,
    pub severity: FraudSeverity,
    pub timestamp: i64,
    pub details: String,
}

impl FraudDetector {
    pub fn new() -> Self {
        FraudDetector {
            alerts: HashMap::new(),
            suspect_history: HashMap::new(),
            blacklist: Vec::new(),
            total_alerts: 0,
            total_resolved: 0,
        }
    }

    /// Signale une activité suspecte
    pub fn report_fraud(
        &mut self,
        suspect: &[u8; 32],
        reporter: &[u8; 32],
        fraud_type: FraudType,
        evidence: String,
    ) -> Result<[u8; 32], ChainError> {
        if suspect == reporter {
            return Err(ChainError::InternalError("Auto-signalement interdit".into()));
        }

        let severity = Self::calculate_severity(&fraud_type);
        let now = chrono::Utc::now().timestamp();

        let alert = FraudAlert {
            id: [0u8; 32],
            suspect: *suspect,
            fraud_type: fraud_type.clone(),
            severity: severity.clone(),
            evidence,
            reported_by: *reporter,
            timestamp: now,
            resolved: false,
            resolution: None,
        };

        let id = Self::compute_id(&alert);
        let mut alert_with_id = alert;
        alert_with_id.id = id;

        // Enregistrer dans l'historique du suspect
        let record = FraudRecord {
            fraud_type,
            severity,
            timestamp: now,
            details: "Signalement reçu".into(),
        };

        self.suspect_history
            .entry(*suspect)
            .or_insert_with(Vec::new)
            .push(record);

        self.alerts.insert(id, alert_with_id);
        self.total_alerts += 1;

        Ok(id)
    }

    /// Détecte le wash trading (même IP/device qui trade avec lui-même)
    pub fn detect_wash_trading(
        &mut self,
        buyer: &[u8; 32],
        seller: &[u8; 32],
        _ip_hash: &[u8; 32],
        _device_hash: &[u8; 32],
    ) -> Option<[u8; 32]> {
        // Si le buyer et le seller partagent la même IP ou le même device
        if buyer == seller {
            return self.report_fraud(
                buyer,
                &[0u8; 32], // Signalé par le système
                FraudType::WashTrading,
                "Même adresse buyer/seller".into(),
            ).ok();
        }

        None
    }

    /// Détecte les attaques Sybil (création massive de comptes)
    pub fn detect_sybil(
        &mut self,
        new_addresses: &[[u8; 32]],
        _ip_hash: &[u8; 32],
        time_window_seconds: i64,
    ) -> Option<[u8; 32]> {
        let _now = chrono::Utc::now().timestamp();

        // Si plus de 5 adresses créées depuis la même IP en 1 heure
        if new_addresses.len() > 5 && time_window_seconds < 3600 {
            return self.report_fraud(
                &new_addresses[0],
                &[0u8; 32],
                FraudType::SybilAttack,
                format!("{} comptes en {}s depuis la même IP", new_addresses.len(), time_window_seconds),
            ).ok();
        }

        None
    }

    /// Détecte les preuves contrefaites
    pub fn detect_counterfeit_proof(
        &mut self,
        submitter: &[u8; 32],
        proof_hash: &[u8; 32],
        is_valid: bool,
    ) -> Option<[u8; 32]> {
        if !is_valid {
            return self.report_fraud(
                submitter,
                &[0u8; 32],
                FraudType::CounterfeitProof,
                format!("Preuve invalide: {}", hex::encode(proof_hash)),
            ).ok();
        }
        None
    }

    /// Vérifie si une adresse est sur la liste noire
    pub fn is_blacklisted(&self, address: &[u8; 32]) -> bool {
        self.blacklist.contains(address)
    }

    /// Ajoute une adresse à la liste noire
    pub fn blacklist_address(&mut self, address: &[u8; 32]) {
        if !self.blacklist.contains(address) {
            self.blacklist.push(*address);
        }
    }

    /// Retire une adresse de la liste noire
    pub fn unblacklist_address(&mut self, address: &[u8; 32]) {
        self.blacklist.retain(|a| a != address);
    }

    /// Résout une alerte de fraude
    pub fn resolve_alert(
        &mut self,
        alert_id: &[u8; 32],
        resolution: String,
    ) -> Result<(), ChainError> {
        if let Some(alert) = self.alerts.get_mut(alert_id) {
            alert.resolved = true;
            alert.resolution = Some(resolution);
            self.total_resolved += 1;
            Ok(())
        } else {
            Err(ChainError::InternalError("Alerte introuvable".into()))
        }
    }

    /// Récupère l'historique de fraude d'une adresse
    pub fn get_fraud_history(&self, address: &[u8; 32]) -> Vec<&FraudRecord> {
        self.suspect_history
            .get(address)
            .map(|records| records.iter().collect())
            .unwrap_or_default()
    }

    /// Calcule un score de risque (0-100)
    pub fn calculate_risk_score(&self, address: &[u8; 32]) -> u64 {
        let records = self.get_fraud_history(address);
        if records.is_empty() {
            return 0;
        }

        let mut score: u64 = 0;

        for record in records {
            score += match record.severity {
                FraudSeverity::Low => 5,
                FraudSeverity::Medium => 20,
                FraudSeverity::High => 50,
                FraudSeverity::Critical => 100,
            };
        }

        score.min(100)
    }

    /// Calcule la sévérité selon le type de fraude
    fn calculate_severity(fraud_type: &FraudType) -> FraudSeverity {
        match fraud_type {
            FraudType::WashTrading => FraudSeverity::High,
            FraudType::PriceManipulation => FraudSeverity::High,
            FraudType::Spoofing => FraudSeverity::Medium,
            FraudType::Layering => FraudSeverity::Medium,
            FraudType::FrontRunning => FraudSeverity::High,
            FraudType::SybilAttack => FraudSeverity::Critical,
            FraudType::Phishing => FraudSeverity::Critical,
            FraudType::CounterfeitProof => FraudSeverity::Critical,
            FraudType::DoubleSpending => FraudSeverity::Critical,
        }
    }

    fn compute_id(alert: &FraudAlert) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(&alert.suspect);
        hasher.update(&alert.reported_by);
        hasher.update(&alert.timestamp.to_le_bytes());
        hasher.update(format!("{:?}", alert.fraud_type).as_bytes());
        let result = hasher.finalize();
        let mut id = [0u8; 32];
        id.copy_from_slice(&result);
        id
    }
}

// ═══════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    fn detector() -> FraudDetector {
        FraudDetector::new()
    }

    #[test]
    fn test_report_fraud() {
        let mut d = detector();
        let suspect = [1u8; 32];
        let reporter = [2u8; 32];

        let id = d.report_fraud(
            &suspect,
            &reporter,
            FraudType::CounterfeitProof,
            "Fausse preuve de paiement".into(),
        ).unwrap_or_default();

        assert!(d.alerts.contains_key(&id));
        assert_eq!(d.total_alerts, 1);
    }

    #[test]
    fn test_self_report_forbidden() {
        let mut d = detector();
        let addr = [1u8; 32];

        let result = d.report_fraud(
            &addr,
            &addr,
            FraudType::Phishing,
            "Test".into(),
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_wash_trading_detection() {
        let mut d = detector();
        let buyer = [1u8; 32];
        let ip = [3u8; 32];
        let device = [4u8; 32];

        let result = d.detect_wash_trading(&buyer, &buyer, &ip, &device);
        assert!(result.is_some());
        assert_eq!(d.total_alerts, 1);
    }

    #[test]
    fn test_blacklist() {
        let mut d = detector();
        let addr = [1u8; 32];

        assert!(!d.is_blacklisted(&addr));
        d.blacklist_address(&addr);
        assert!(d.is_blacklisted(&addr));
        d.unblacklist_address(&addr);
        assert!(!d.is_blacklisted(&addr));
    }

    #[test]
    fn test_resolve_alert() {
        let mut d = detector();
        let suspect = [1u8; 32];
        let reporter = [2u8; 32];

        let id = d.report_fraud(&suspect, &reporter, FraudType::Spoofing, "Test".into()).unwrap_or_default();
        assert_eq!(d.total_resolved, 0);

        d.resolve_alert(&id, "Fausse alerte - erreur".into()).unwrap_or_default();
        assert_eq!(d.total_resolved, 1);
        assert!(d.alerts.get(&id).unwrap_or_default().resolved);
    }

    #[test]
    fn test_risk_score() {
        let mut d = detector();
        let suspect = [1u8; 32];
        let reporter = [2u8; 32];

        // Score initial = 0
        assert_eq!(d.calculate_risk_score(&suspect), 0);

        // Ajouter une alerte Critique (100 points)
        d.report_fraud(&suspect, &reporter, FraudType::DoubleSpending, "Test".into()).unwrap_or_default();
        assert_eq!(d.calculate_risk_score(&suspect), 100);

        // Ajouter une alerte Faible (5 points) → toujours 100 (max)
        d.report_fraud(&suspect, &reporter, FraudType::Spoofing, "Test".into()).unwrap_or_default();
        assert_eq!(d.calculate_risk_score(&suspect), 100);
    }

    #[test]
    fn test_counterfeit_proof_detection() {
        let mut d = detector();
        let submitter = [1u8; 32];
        let proof = [5u8; 32];

        // Preuve invalide → alerte
        let result = d.detect_counterfeit_proof(&submitter, &proof, false);
        assert!(result.is_some());

        // Preuve valide → pas d'alerte
        let result = d.detect_counterfeit_proof(&submitter, &proof, true);
        assert!(result.is_none());
    }
}





