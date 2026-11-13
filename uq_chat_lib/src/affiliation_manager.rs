use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub enum ReferralRewardType { AccountCreated, KycVerified, BORBPurchase }

#[derive(Debug, Clone, Serialize)]
pub struct RewardResult { pub success: bool, pub amount_micro_BORB: u64, pub tx_id: String }

#[derive(Debug, Clone, Serialize)]
pub struct AffiliationBalance { pub total_earned: u64, pub available: u64, pub pending: u64 }

#[derive(Debug, Clone, Serialize)]
pub struct AffiliationNode { pub uq_number: String, pub status: String, pub kyc_level: u8, pub joined_at: u64, pub total_commission: u64 }

pub struct AffiliationManager;

impl AffiliationManager {
    pub fn new() -> Self { AffiliationManager }

    /// F10 : Traite une récompense d'affiliation
    pub fn process_referral_reward(&self, _referral_code: &str, reward_type: ReferralRewardType) -> RewardResult {
        let amount = match reward_type {
            ReferralRewardType::AccountCreated => 1_000_000, // 1 USD en microBORB
            ReferralRewardType::KycVerified => 1_000_000,
            ReferralRewardType::BORBPurchase => 0, // Calculé ailleurs (5%)
        };
        RewardResult { success: true, amount_micro_BORB: amount, tx_id: format!("tx_{}", rand::random::<u64>()) }
    }

    /// F11 : Solde d'affiliation
    pub fn get_affiliation_balance(&self) -> AffiliationBalance {
        AffiliationBalance { total_earned: 0, available: 0, pending: 0 }
    }

    /// F12 : Réclamer les récompenses
    pub fn claim_affiliation_rewards(&self) -> String {
        format!("tx_claim_{}", rand::random::<u64>())
    }

    /// F13 : Réseau d'affiliation
    pub fn get_affiliation_network(&self) -> Vec<AffiliationNode> {
        vec![]
    }
}

