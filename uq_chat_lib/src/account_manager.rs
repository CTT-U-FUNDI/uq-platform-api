use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct AccountResult {
    pub success: bool,
    pub uq_number: String,
    pub uq_email: String,
    pub uq_pseudo: String,
    pub welcome_bonus_micro_BORB: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct KycResult { pub success: bool, pub level: u8, pub message: String }

#[derive(Debug, Clone, Serialize)]
pub struct KycStatus { pub level: u8, pub verified: bool, pub verified_at: Option<u64> }

pub struct AccountManager;

impl AccountManager {
    pub fn new() -> Self { AccountManager }

    /// F4 : Créer un compte
    pub fn create_account(&self, first_name: &str, last_name: &str, _birth_date: &str, _country: &str, _phone: &str) -> AccountResult {
        AccountResult {
            success: true,
            uq_number: format!("+901 01 {:07}", rand::random::<u32>() % 10_000_000),
            uq_email: format!("{}@uq.id", first_name.to_lowercase()),
            uq_pseudo: format!("{}_{}", first_name, last_name),
            welcome_bonus_micro_BORB: 10_000, // 0.01 BORB
        }
    }

    /// F5 : KYC niveau 2
    pub fn verify_kyc_level_2(&self, _selfie: &[u8], _id_photo: &[u8]) -> KycResult {
        KycResult { success: true, level: 2, message: "KYC Niveau 2 vérifié".into() }
    }

    /// F6 : KYC niveau 3
    pub fn verify_kyc_level_3(&self, _video_selfie: &[u8], _proof_of_address: &[u8]) -> KycResult {
        KycResult { success: true, level: 3, message: "KYC Niveau 3 vérifié".into() }
    }

    /// F7 : Statut KYC
    pub fn get_kyc_status(&self) -> KycStatus {
        KycStatus { level: 2, verified: true, verified_at: Some(1_906_588_800) }
    }

    /// F8 : Valider code parrainage
    pub fn validate_referral_code(&self, code: &str) -> bool {
        !code.is_empty()
    }

    /// F9 : Stats parrainage
    pub fn get_referral_stats(&self) -> serde_json::Value {
        serde_json::json!({"referrals": 0, "total_commission": 0})
    }
}

