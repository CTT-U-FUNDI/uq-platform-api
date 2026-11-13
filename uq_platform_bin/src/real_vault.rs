use std::sync::Mutex;
use uq_security::vtee::VteeEnclave;
use uq_security::key_derivation::KeyDerivation;
use uq_chat::account_manager::AccountManager;
use uq_chat::affiliation_manager::AffiliationManager;

/// État réel pour Vault + Sécurité + Chat
pub struct SecurityState {
    pub vtee: Mutex<VteeEnclave>,
    pub key_derivation: Mutex<KeyDerivation>,
    pub account_manager: AccountManager,
    pub affiliation_manager: AffiliationManager,
}

impl SecurityState {
    pub fn new() -> Self {
        Self {
            vtee: Mutex::new(VteeEnclave::init()),
            key_derivation: Mutex::new(KeyDerivation::new().unwrap_or_else(|_| KeyDerivation::new().unwrap())),
            account_manager: AccountManager::new(),
            affiliation_manager: AffiliationManager::new(),
        }
    }
    
    /// Génère un code 2FA
    pub fn generate_2fa(&self) -> Result<String, String> {
        let vtee = self.vtee.lock().unwrap();
        vtee.generate_2fa().map_err(|e| format!("{:?}", e))
    }
    
    /// Vérifie un code 2FA
    pub fn verify_2fa(&self, code: &str) -> bool {
        let vtee = self.vtee.lock().unwrap();
        vtee.verify_2fa(code).unwrap_or(false)
    }
    
    /// Dérive une clé
    pub fn derive_key(&self, purpose: &str, index: u32) -> Result<String, String> {
        let mut kd = self.key_derivation.lock().unwrap();
        let key = kd.derive_key(purpose, index).map_err(|e| format!("{:?}", e))?;
        Ok(hex::encode(key))
    }
    
    /// Crée un compte KYC
    pub fn create_account(&self, first_name: &str, last_name: &str, birth_date: &str, country: &str, phone: &str) -> serde_json::Value {
        let result = self.account_manager.create_account(first_name, last_name, birth_date, country, phone);
        serde_json::json!({
            "uq_number": result.uq_number,
            "success": result.success
        })
    }
    
    /// Vérifie KYC niveau 2
    pub fn verify_kyc_level_2(&self) -> serde_json::Value {
        let result = self.account_manager.verify_kyc_level_2(&[], &[]);
        serde_json::json!({
            "success": result.success,
            "level": result.level,
            "message": result.message
        })
    }
    
    /// Récupère le statut KYC
    pub fn get_kyc_status(&self) -> serde_json::Value {
        let status = self.account_manager.get_kyc_status();
        serde_json::json!({
            "level": status.level,
            "verified": status.verified
        })
    }
    
    /// Traite une récompense de parrainage
    pub fn process_referral(&self, code: &str) -> serde_json::Value {
        let result = self.affiliation_manager.process_referral_reward(code, uq_chat::affiliation_manager::ReferralRewardType::AccountCreated);
        serde_json::json!({
            "success": result.success,
            "amount_micro_BORB": result.amount_micro_BORB
        })
    }
    
    /// Récupère le solde d'affiliation
    pub fn get_affiliation_balance(&self) -> serde_json::Value {
        let balance = self.affiliation_manager.get_affiliation_balance();
        serde_json::json!({
            "total_earned": balance.total_earned,
            "available": balance.available,
            "pending": balance.pending
        })
    }
}

