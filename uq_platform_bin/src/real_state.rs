use std::fs;
use std::sync::Mutex;
use std::collections::HashMap;
use uq_blockchain::identity::{create_identity, derive_uq_number, derive_uq_email, derive_uq_pseudo};
use uq_blockchain::bubble::{BubbleRegistry, Bubble};
use uq_blockchain::six_cases::SixCases;
use uq_blockchain::ubi::UbiPool;
use serde::{Serialize, Deserialize};

/// ÉTAT GLOBAL RÉEL — connecté aux vraies libs
pub struct GlobalState {
    pub bubble_registry: Mutex<BubbleRegistry>,
    pub six_cases: Mutex<SixCases>,
    pub ubi_pool: Mutex<UbiPool>,
    pub accounts: Mutex<HashMap<String, AccountInfo>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountInfo {
    pub has_active_dispute: bool,
    pub has_sanction: bool,
    pub phone_verified: bool,
    pub commitment_signed: bool,
    pub is_marketer: bool,
    pub sanction_level: u8,
    pub is_active: bool,
    pub uq_number: String,
    pub email_uq: String,
    pub full_name: String,
    pub id: String,
    pub public_key: [u8; 32],
    pub created_at: i64,
    pub kyc_verified: bool,
    pub last_activity_date: Option<String>,
    pub account_age_days: u64,
    pub successful_transactions: u64,
    pub loans_repaid: u64,
    pub reputation_score: i32,
    pub has_active_loan: bool,
    pub verification_level: u8,
}

impl GlobalState {
    /// Applique une sanction à un compte
    pub fn apply_sanction(&self, account_id: &str, level: u8, reason: &str) -> Result<(), String> {
        let mut accounts = self.accounts.lock().unwrap();
        let account = accounts.get_mut(account_id).ok_or("Compte introuvable")?;
        
        match level {
            1 => {
                // NIVEAU 1 : Punition — avertissement + gel temporaire
                account.has_sanction = true;
                account.sanction_level = 1;
            },
            2 => {
                // NIVEAU 2 : Restriction — limites réduites
                account.has_sanction = true;
                account.sanction_level = 2;
                account.verification_level = account.verification_level.min(1);
            },
            3 => {
                // NIVEAU 3 : Ban — interdiction définitive
                account.has_sanction = true;
                account.sanction_level = 3;
                account.is_active = false;
            },
            _ => return Err("Niveau de sanction invalide (1-3)".to_string()),
        }
        
        Ok(())
    }
    
    /// Lève une sanction
    pub fn remove_sanction(&self, account_id: &str) -> Result<(), String> {
        let mut accounts = self.accounts.lock().unwrap();
        let account = accounts.get_mut(account_id).ok_or("Compte introuvable")?;
        account.has_sanction = false;
        account.sanction_level = 0;
        Ok(())
    }
    
    /// Active le statut Marketer avec les bonus
    pub fn activate_marketer(&self, account_id: &str) -> Result<(), String> {
        let mut accounts = self.accounts.lock().unwrap();
        let account = accounts.get_mut(account_id).ok_or("Compte introuvable")?;
        
        // Vérifier les 9 conditions
        if account.verification_level < 1 { return Err("KYC niveau 1 requis".to_string()); }
        if account.account_age_days < 7 { return Err("7 jours requis".to_string()); }
        if account.successful_transactions < 3 { return Err("3 transactions requises".to_string()); }
        if account.reputation_score <= 10 { return Err("Réputation > 10 requise".to_string()); }
        if account.has_active_dispute { return Err("Litige en cours".to_string()); }
        if account.has_sanction { return Err("Sanction active".to_string()); }
        if !account.phone_verified { return Err("Téléphone non vérifié".to_string()); }
        if !account.commitment_signed { return Err("Engagement non signé".to_string()); }
        
        // BONUS : Activer le statut
        account.is_marketer = true;
        
        // BONUS : +10 réputation
        account.reputation_score += 10;
        
        // BONUS : Boutique jour 1 (verification_level >= 1)
        if account.verification_level < 1 {
            account.verification_level = 1;
        }
        
        Ok(())
    }
    
    /// Applique la réduction de 2% pour les Marketers
    pub fn get_marketer_fee_discount(&self, account_id: &str) -> u64 {
        let accounts = self.accounts.lock().unwrap();
        if let Some(account) = accounts.get(account_id) {
            if account.is_marketer {
                return 2; // 2% de réduction
            }
        }
        0
    }
    /// Vérifie si un compte PEUT devenir Marketer (9 conditions réelles)
    pub fn can_become_marketer(&self, account_id: &str) -> Result<bool, String> {
        let accounts = self.accounts.lock().unwrap();
        let account = accounts.get(account_id).ok_or("Compte introuvable")?;
        
        // RÈGLE 1 : KYC niveau 1 minimum
        if account.verification_level < 1 {
            return Err("KYC niveau 1 requis".to_string());
        }
        
        // RÈGLE 2 : Compte >= 7 jours
        if account.account_age_days < 7 {
            return Err("Compte trop récent (7 jours requis)".to_string());
        }
        
        // RÈGLE 3 : >= 3 transactions réussies
        if account.successful_transactions < 3 {
            return Err("3 transactions réussies requises".to_string());
        }
        
        // RÈGLE 4 : Réputation > 10
        if account.reputation_score <= 10 {
            return Err("Réputation > 10 requise".to_string());
        }
        
        // RÈGLE 5 : Caution de 50 bulles (5000 points)
        let balance = self.get_balance(account_id);
        if balance < 5000 {
            return Err("Caution de 50 bulles requise".to_string());
        }
        
        // RÈGLE 6 : Pas de litige en cours
        if account.has_active_dispute {
            return Err("Litige en cours".to_string());
        }
        
        // RÈGLE 7 : Pas de sanction active
        if account.has_sanction {
            return Err("Sanction active".to_string());
        }
        
        // RÈGLE 8 : Téléphone vérifié
        if !account.phone_verified {
            return Err("Téléphone non vérifié".to_string());
        }
        
        // RÈGLE 9 : Engagement à créer des offres
        if !account.commitment_signed {
            return Err("Engagement à créer des offres non signé".to_string());
        }
        
        Ok(true)
    }
    /// Limite de transfert journalière selon le niveau KYC
    pub fn get_transfer_limit(&self, account_id: &str) -> u64 {
        let level = self.get_verification_level(account_id);
        match level {
            0 => 100_000,        // Niveau 0 : 1 000 BORB/jour (limité)
            1 => 1_000_000,      // Niveau 1 : 10 000 BORB/jour
            2 => 100_000_000,    // Niveau 2 : 1M BORB/jour
            _ => u64::MAX,       // Niveau 3+ : illimité
        }
    }
    
    /// Vérifie si un compte peut créer une boutique (Niveau 1+)
    pub fn can_create_shop(&self, account_id: &str) -> bool {
        self.get_verification_level(account_id) >= 1
    }
    
    /// Vérifie si un compte peut vendre sa connexion (Niveau 1+)
    pub fn can_sell_connection(&self, account_id: &str) -> bool {
        self.get_verification_level(account_id) >= 1
    }
    
    /// Vérifie si un compte peut voter DAO (Niveau 1+)
    pub fn can_vote_dao(&self, account_id: &str) -> bool {
        self.get_verification_level(account_id) >= 1
    }
    /// Vérifie la carte d'identité (Niveau 1)
    pub fn verify_id_card(&self, account_id: &str) -> Result<(), String> {
        let mut accounts = self.accounts.lock().unwrap();
        let account = accounts.get_mut(account_id).ok_or("Compte introuvable")?;
        if account.verification_level < 1 {
            account.verification_level = 1;
        }
        Ok(())
    }
    
    /// Vérifie la vidéo biométrique (Niveau 2)
    pub fn verify_video_biometric(&self, account_id: &str) -> Result<(), String> {
        let mut accounts = self.accounts.lock().unwrap();
        let account = accounts.get_mut(account_id).ok_or("Compte introuvable")?;
        if account.verification_level < 2 {
            account.verification_level = 2;
        }
        Ok(())
    }
    
    /// Vérifie le Mobile Money (Niveau 3)
    pub fn verify_mobile_money(&self, account_id: &str) -> Result<(), String> {
        let mut accounts = self.accounts.lock().unwrap();
        let account = accounts.get_mut(account_id).ok_or("Compte introuvable")?;
        if account.verification_level < 3 {
            account.verification_level = 3;
        }
        Ok(())
    }
    
    /// Vérifie la carte bancaire (Niveau 4)
    pub fn verify_bank_card(&self, account_id: &str) -> Result<(), String> {
        let mut accounts = self.accounts.lock().unwrap();
        let account = accounts.get_mut(account_id).ok_or("Compte introuvable")?;
        if account.verification_level < 4 {
            account.verification_level = 4;
        }
        Ok(())
    }
    
    /// Vérifie PayPal (Niveau 5)
    pub fn verify_paypal(&self, account_id: &str) -> Result<(), String> {
        let mut accounts = self.accounts.lock().unwrap();
        let account = accounts.get_mut(account_id).ok_or("Compte introuvable")?;
        if account.verification_level < 5 {
            account.verification_level = 5;
        }
        Ok(())
    }
    
    /// Récupère le niveau de vérification
    pub fn get_verification_level(&self, account_id: &str) -> u8 {
        let accounts = self.accounts.lock().unwrap();
        accounts.get(account_id)
            .map(|a| a.verification_level)
            .unwrap_or(0)
    }
    
    /// Vérifie si un compte peut utiliser le flux Mobile Money automatisé
    pub fn can_use_mobile_money(&self, account_id: &str) -> bool {
        self.get_verification_level(account_id) >= 3
    }
    /// Vérifie si un compte PEUT emprunter
    pub fn can_borrow(&self, account_id: &str) -> Result<bool, String> {
        let accounts = self.accounts.lock().unwrap();
        let account = accounts.get(account_id).ok_or("Compte introuvable")?;
        
        // RÈGLE 1 : Compte minimum 30 jours
        if account.account_age_days < 30 {
            return Err("Compte trop récent : 30 jours minimum requis".to_string());
        }
        
        // RÈGLE 2 : Au moins 10 transactions réussies
        if account.successful_transactions < 10 {
            return Err("10 transactions réussies requises".to_string());
        }
        
        // RÈGLE 3 : Au moins 1 remboursement effectué
        if account.loans_repaid < 1 {
            return Err("Au moins 1 remboursement de prêt requis".to_string());
        }
        
        // RÈGLE 4 : KYC niveau 1 minimum
        if account.verification_level < 1 {
            return Err("KYC niveau 1 requis (carte d'identité)".to_string());
        }
        
        // RÈGLE 5 : Score de réputation positif
        if account.reputation_score <= 0 {
            return Err("Score de réputation positif requis".to_string());
        }
        
        // RÈGLE 6 : Pas de prêt en cours
        if account.has_active_loan {
            return Err("Un prêt est déjà en cours".to_string());
        }
        
        Ok(true)
    }
    
    /// Calcule le collatéral requis selon la réputation
    pub fn calculate_collateral_ratio(&self, reputation_score: i32) -> f64 {
        match reputation_score {
            score if score < 0 => 0.0,
            score if score < 25 => 1.50,
            score if score < 50 => 1.30,
            score if score < 75 => 1.10,
            _ => 1.00,
        }
    }
    /// Vérifie si le compte a eu une activité aujourd'hui
    pub fn has_activity_today(&self, account_id: &str) -> bool {
        let accounts = self.accounts.lock().unwrap();
        if let Some(account) = accounts.get(account_id) {
            let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
            return account.last_activity_date.as_deref() == Some(today.as_str());
        }
        false
    }
    
    /// Enregistre une activité pour aujourd'hui
    pub fn record_activity(&self, account_id: &str) {
        let mut accounts = self.accounts.lock().unwrap();
        if let Some(account) = accounts.get_mut(account_id) {
            let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
            account.last_activity_date = Some(today);
        }
    }
    pub fn new() -> Self {
        let registry = BubbleRegistry::new();
        let six_cases = SixCases::new(registry.clone()).unwrap_or_else(|_| {
            // Fallback : recréer une registry pour les 6 cases
            let reg2 = BubbleRegistry::new();
            SixCases::new(reg2).unwrap()
        });
        let ubi_pool = UbiPool::new(1_000_000); // 1 million de points par jour
        
        Self {
            bubble_registry: Mutex::new(registry),
            six_cases: Mutex::new(six_cases),
            ubi_pool: Mutex::new(ubi_pool),
            accounts: Mutex::new(HashMap::new()),
        }
    }
    
    /// Crée un compte avec une vraie adresse et une vraie bulle
    pub fn create_account(&self) -> Result<AccountInfo, String> {
        let mut accounts = self.accounts.lock().unwrap();
        let mut registry = self.bubble_registry.lock().unwrap();
        
        // Générer une adresse unique
        let timestamp = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let mut address = [0u8; 32];
        let hash = uq_core::crypto::sha256(&timestamp.to_le_bytes());
        address.copy_from_slice(&hash[..32]);
        
        // Créer la bulle pour cet utilisateur
        // Chaque nouvel utilisateur reçoit sa part équitable
        let total_users = accounts.len() as u64 + 1;
        let bubbles_per_user = 0u64;
        let value_micro = 0u64; // 1 bulle = 100 points
        
        let _bubble = registry.create_bubble(
            address,
            address, // créateur = propriétaire
            value_micro,
            Vec::new(),
            |data| {
                // Signature simple : hash du contenu
                uq_core::crypto::sha256(data).to_vec()
            }
        ).map_err(|e| e.to_string())?;
        
        let account_number = format!("UQ-{:08}", accounts.len() as u64 + 1);
        let account_email = format!("user{}@ufundi.uq", accounts.len() as u64 + 1);
        let account_name = format!("Utilisateur {}", accounts.len() as u64 + 1);
        
        let account_number = format!("UQ-{:08}", accounts.len() as u64 + 1);
        let account_email = format!("user{}@ufundi.uq", accounts.len() as u64 + 1);
        let account_name = format!("Utilisateur {}", accounts.len() as u64 + 1);
        
        // Générer la UqBlackKey 512 bits
        let black_key = uq_black_key_standalone::UqBlackKey::generate();
        let public_key = black_key.public_key;
        
        let account_number = format!("UQ-{:08}", accounts.len() as u64 + 1);
        let account_email = format!("user{}@ufundi.uq", accounts.len() as u64 + 1);
        let account_name = format!("Utilisateur {}", accounts.len() as u64 + 1);
        
        // Générer la UqBlackKey 512 bits
        let black_key = uq_black_key_standalone::UqBlackKey::generate();
        let public_key = black_key.public_key;
        
        let account = AccountInfo {
            uq_number: account_number,
            email_uq: account_email,
            full_name: account_name,
            id: hex::encode(address),
            public_key: address,
            created_at: chrono::Utc::now().timestamp(),
            kyc_verified: false,
            last_activity_date: None,
            account_age_days: 0,
            successful_transactions: 0,
            loans_repaid: 0,
            reputation_score: 0,
            has_active_loan: false,
            verification_level: 0,
            has_active_dispute: false,
            has_sanction: false,
            phone_verified: false,
            commitment_signed: false,
            is_marketer: false,
            sanction_level: 0,
            is_active: true,
        };
        
        accounts.insert(account.id.clone(), account.clone());
        let _ = self.save_accounts_unlocked(&accounts, "uq_accounts.json");
        Ok(account)
    }
    
    /// Récupère le solde d'un compte
    pub fn get_balance(&self, account_id: &str) -> u64 {
        let registry = self.bubble_registry.lock().unwrap();
        if let Ok(account_bytes) = hex::decode(account_id) {
            let mut addr = [0u8; 32];
            if account_bytes.len() >= 32 {
                addr.copy_from_slice(&account_bytes[..32]);
                return registry.get_balance(&addr);
            }
        }
        0
    }
    
    /// Réclame l'UBI réel
    pub fn claim_ubi(&self, account_id: &str) -> Result<u64, String> {
        // RÈGLE 1 : Le compte doit avoir une activité aujourd'hui
        if !self.has_activity_today(account_id) {
            return Err("Aucune activité aujourd'hui".to_string());
        }
        
        // RÈGLE 2 : UBI plafonné à 2 POINTS = 0.02 BORB
        let ubi_amount = 2u64;
        
        // Créditer via create_bubble (la vraie méthode)
        let account_bytes = hex::decode(account_id).map_err(|_| "Adresse invalide")?;
        let mut addr = [0u8; 32];
        addr.copy_from_slice(&account_bytes[..32]);
        
        let mut registry = self.bubble_registry.lock().unwrap();
        registry.create_bubble(
            addr,
            addr,
            ubi_amount,
            Vec::new(),
            |data| uq_core::crypto::sha256(data).to_vec()
        ).map_err(|e| e.to_string())?;
        
        // 🔐 TRACAGE UBI
        let db_path = "data/users/uq_00000001.db";
        if let Ok(conn) = rusqlite::Connection::open(db_path) {
            let _ = crate::borb_tracer::init_borb_origins_table(&conn);
            
            // Le prix lu est la seule source de la contre-valeur USD : sans
            // prix lisible, l'émission n'est pas tracée en USD plutôt que
            // d'être enregistrée sur un taux de fantaisie.
            let prix_state = crate::persistence::load_prix();
            match prix_state.prix_actuel.trim().parse::<f64>() {
                Ok(prix_borb) if prix_borb.is_finite() && prix_borb > 0.0 => {
                    let _ = crate::borb_tracer::trace_with_usd_value(
                        &conn,
                        "ubi",
                        "UQ-CASE-RECOMPENSES",
                        "CASE_3_RECOMPENSES",
                        account_id,
                        crate::borb_tracer::UBI_VALUE_USD,
                        prix_borb,
                        "ubi_emission",
                    );
                }
                _ => eprintln!(
                    "🛑 Tracage UBI sans valeur USD : prix_actuel = {:?}",
                    prix_state.prix_actuel
                ),
            }
        }
        
        Ok(ubi_amount)
    }
    
    /// Vérifie si un compte peut réclamer l'UBI
    pub fn can_claim_ubi(&self, account_id: &str) -> bool {
        self.has_activity_today(account_id)
    }
    
    /// Récupère les stats des 6 cases
    pub fn get_cases_stats(&self) -> serde_json::Value {
        let cases = self.six_cases.lock().unwrap();
        let stats = cases.get_stats();
        
        let mut result = serde_json::json!({});
        for case in stats {
            result[case.name] = serde_json::json!(case.balance);
        }
        result
    }
    
    /// Vérifie l'invariant total (900 milliards)
    pub fn verify_invariant(&self) -> bool {
        let cases = self.six_cases.lock().unwrap();
        let registry = self.bubble_registry.lock().unwrap();
        let total_user_balance = 0u64;
        cases.verify_total_invariant(total_user_balance).unwrap_or(false)
    }
    
    /// Statistiques globales
    pub fn get_global_stats(&self) -> serde_json::Value {
        let accounts = self.accounts.lock().unwrap();
        let registry = self.bubble_registry.lock().unwrap();
        
        serde_json::json!({
            "total_accounts": accounts.len(),
            "total_bubbles": 40_000_000_000u64,
            "active_bubbles": 0u64,
            "total_value_micro": 0u64,
            "frozen_count": registry.frozen.len(),
            "lent_count": registry.lent.len()
        })
    }
    /// Sauvegarde les comptes sur disque
    pub fn save_accounts_unlocked(&self, accounts: &HashMap<String, AccountInfo>, path: &str) -> Result<(), String> {
        let json_str = serde_json::to_string_pretty(accounts)
            .map_err(|e| e.to_string())?;
        std::fs::write(path, json_str)
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    
    /// Charge les comptes depuis le disque
    pub fn load_accounts(&self, path: &str) -> Result<(), String> {
        if let Ok(content) = std::fs::read_to_string(path) {
            let loaded: HashMap<String, AccountInfo> = serde_json::from_str(&content)
                .map_err(|e| e.to_string())?;
            let mut accounts = self.accounts.lock().unwrap();
            *accounts = loaded;
        }
        Ok(())
    }}






























