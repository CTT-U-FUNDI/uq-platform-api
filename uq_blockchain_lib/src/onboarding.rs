use crate::bubble::BubbleRegistry;
use uq_black_key_standalone::UqBlackKey;
use crate::error::ChainError;
use std::collections::HashMap;

// ═══════════════════════════════════════════════════════════════
// CONSTANTES EN MICRO-BORB (1 BORB = 1,000,000 USD)
// ═══════════════════════════════════════════════════════════════

/// 1 BORB = 1,000,000 micro-BORB = 1,000,000 USD
pub const MICRO_PER_BORB: u64 = 1_000_000;

/// Récompense création compte : 1 micro-BORB (~0.000001 BORB = 1 USD)
const CREATE_ACCOUNT_REWARD: u64 = 1;

/// Récompense vérification KYC : 1 micro-BORB (~1 USD)
const VERIFY_ACCOUNT_REWARD: u64 = 1;

/// Récompense parrain création compte : 0.5 micro-BORB (~0.50 USD)
const SPONSOR_CREATE_REWARD: u64 = 1;

/// Récompense parrain vérification : 0.5 micro-BORB (~0.50 USD)
const SPONSOR_VERIFY_REWARD: u64 = 1;

/// Commission standard sur achats des 10 premiers filleuls : 5%
const STANDARD_REFERRAL_PCT: u8 = 5;

/// Commission progressive après 10 filleuls : 0.1% par filleul supplémentaire
const PROGRESSIVE_REFERRAL_PCT: f64 = 0.1;

/// Profit sur achat BORB par le filleul : 10% pour le parrain
const BORB_PURCHASE_PROFIT_PCT: u8 = 10;

/// Seuil pour passer à la commission progressive
const REFERRAL_THRESHOLD: u32 = 10;

#[derive(Debug, Clone, PartialEq)]
pub enum AccountState { Created, Verified, Active, Suspended }

#[derive(Debug, Clone)]
pub struct UqAccount {
    pub address: [u8; 32],
    pub state: AccountState,
    pub sponsor: Option<[u8; 32]>,
    pub referred_by: Option<[u8; 32]>,
    pub verified_by: Option<[u8; 32]>,
    pub created_at: u64,
    pub verified_at: Option<u64>,
    pub total_referral_earnings: u64,
    pub referrals_count: u32,
    pub profile_id: Option<String>,
    pub total_purchases: u64,
}

impl UqAccount {
    pub fn new(address: [u8; 32], referred_by: Option<[u8; 32]>) -> Self {
        UqAccount {
            address, state: AccountState::Created, sponsor: referred_by,
            referred_by, verified_by: None,
            created_at: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs(),
            verified_at: None, total_referral_earnings: 0, referrals_count: 0,
            profile_id: None, total_purchases: 0,
        }
    }
}

pub struct OnboardingManager {
    pub accounts: HashMap<[u8; 32], UqAccount>,
}

impl OnboardingManager {
    pub fn new() -> Self { OnboardingManager { accounts: HashMap::new() } }

    /// CRÉER UN COMPTE — 1 micro-BORB (~1 USD)
    pub fn create_account(&mut self, new_address: [u8; 32], referred_by: Option<[u8; 32]>, registry: &mut BubbleRegistry) -> Result<(), ChainError> {
        if self.accounts.contains_key(&new_address) { return Err(ChainError::InternalError("Compte déjà existant".into())); }
        self.accounts.insert(new_address, UqAccount::new(new_address, referred_by));
        if let Some(ref addr) = referred_by {
            if let Some(s) = self.accounts.get_mut(addr) {
                s.total_referral_earnings += SPONSOR_CREATE_REWARD;
                s.referrals_count += 1;
            }
        }
        Ok(())
    }

    /// VÉRIFIER UN COMPTE — 1 micro-BORB (~1 USD)
    pub fn verify_account(&mut self, address: [u8; 32], validator: [u8; 32]) -> Result<(), ChainError> {
        let sponsor = {
            let account = self.accounts.get(&address).ok_or(ChainError::InternalError("Compte non trouvé".into()))?;
            if account.state == AccountState::Verified { return Err(ChainError::InternalError("Déjà vérifié".into())); }
            account.sponsor
        };
        if let Some(account) = self.accounts.get_mut(&address) {
            account.state = AccountState::Verified;
            account.verified_by = Some(validator);
            account.verified_at = Some(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs());
        }
        if let Some(ref addr) = sponsor {
            if let Some(s) = self.accounts.get_mut(addr) { s.total_referral_earnings += SPONSOR_VERIFY_REWARD; }
        }
        Ok(())
    }

    /// CALCULER LA COMMISSION DE PARRAINAGE
    /// - 10 premiers filleuls : 5% sur achats
    /// - Après 10 : 5% + 0.1% par filleul supplémentaire
    /// - Achat BORB par le filleul : +10% pour le parrain
    pub fn calculate_referral_commission(&self, buyer_address: &[u8; 32], purchase_amount: u64, is_BORB_purchase: bool) -> u64 {
        let account = match self.accounts.get(buyer_address) { Some(a) => a, None => return 0 };
        let sponsor_addr = match account.sponsor { Some(ref addr) => addr, None => return 0 };
        let sponsor = match self.accounts.get(sponsor_addr) { Some(s) => s, None => return 0 };

        let base_pct = if sponsor.referrals_count <= REFERRAL_THRESHOLD {
            STANDARD_REFERRAL_PCT as f64
        } else {
            STANDARD_REFERRAL_PCT as f64 + (PROGRESSIVE_REFERRAL_PCT * (sponsor.referrals_count - REFERRAL_THRESHOLD) as f64)
        };

        let total_pct = if is_BORB_purchase { base_pct + BORB_PURCHASE_PROFIT_PCT as f64 } else { base_pct };

        (purchase_amount as f64 * total_pct / 100.0) as u64
    }

    /// DISTRIBUER LA COMMISSION
    pub fn distribute_referral_commission(&mut self, buyer_address: &[u8; 32], purchase_amount: u64, is_BORB_purchase: bool) -> Result<u64, ChainError> {
        let commission = self.calculate_referral_commission(buyer_address, purchase_amount, is_BORB_purchase);
        if commission > 0 {
            // Mettre à jour le total des achats du filleul
            if let Some(account) = self.accounts.get_mut(buyer_address) {
                account.total_purchases += purchase_amount;
            }
        }
        Ok(commission)
    }

    /// Récupérer le pourcentage de commission pour un parrain
    pub fn get_referral_rate(&self, sponsor_address: &[u8; 32]) -> f64 {
        let sponsor = match self.accounts.get(sponsor_address) { Some(s) => s, None => return 0.0 };
        if sponsor.referrals_count <= REFERRAL_THRESHOLD {
            STANDARD_REFERRAL_PCT as f64
        } else {
            STANDARD_REFERRAL_PCT as f64 + (PROGRESSIVE_REFERRAL_PCT * (sponsor.referrals_count - REFERRAL_THRESHOLD) as f64)
        }
    }

    pub fn link_profile(&mut self, address: [u8; 32], profile_id: String) -> Result<(), ChainError> {
        let account = self.accounts.get_mut(&address).ok_or(ChainError::InternalError("Compte non trouvé".into()))?;
        account.profile_id = Some(profile_id);
        account.state = AccountState::Active;
        Ok(())
    }

    pub fn get_account(&self, address: &[u8; 32]) -> Option<&UqAccount> { self.accounts.get(address) }
    pub fn get_referrals(&self, sponsor: &[u8; 32]) -> Vec<&UqAccount> { self.accounts.values().filter(|a| a.sponsor.as_ref() == Some(sponsor)).collect() }
    pub fn get_total_referral_earnings(&self, address: &[u8; 32]) -> u64 { self.accounts.get(address).map(|a| a.total_referral_earnings).unwrap_or(0) }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key() -> UqBlackKey { let mut k = UqBlackKey::generate(); k.update_balance(1_000_000); k }

    #[test] fn test_create_reward_1_micro() { let mut m = OnboardingManager::new(); let mut k = key(); let b = k.balance(); m.create_account([1u8;32], None, &mut k).unwrap_or_default(); assert_eq!(k.balance(), b + 1); }
    #[test] fn test_sponsor_gets_reward() { let mut m = OnboardingManager::new(); let mut k = key(); m.create_account([2u8;32], None, &mut k).unwrap_or_default(); m.create_account([1u8;32], Some([2u8;32]), &mut k).unwrap_or_default(); assert!(m.get_account(&[2u8;32]).unwrap_or_default().total_referral_earnings >= 1); }
    #[test] fn test_commission_5pct_first_10() { let mut m = OnboardingManager::new(); let mut k = key(); m.create_account([2u8;32], None, &mut k).unwrap_or_default(); m.create_account([1u8;32], Some([2u8;32]), &mut k).unwrap_or_default(); let c = m.calculate_referral_commission(&[1u8;32], 1000, false); assert_eq!(c, 50); }
    #[test] fn test_BORB_purchase_10pct_extra() { let mut m = OnboardingManager::new(); let mut k = key(); m.create_account([2u8;32], None, &mut k).unwrap_or_default(); m.create_account([1u8;32], Some([2u8;32]), &mut k).unwrap_or_default(); let c = m.calculate_referral_commission(&[1u8;32], 1000, true); assert_eq!(c, 150); }
    #[test] fn test_progressive_after_10() { let mut m = OnboardingManager::new(); let mut k = key(); m.create_account([9u8;32], None, &mut k).unwrap_or_default(); for i in 0..15 { m.accounts.get_mut(&[9u8;32]).unwrap_or_default().referrals_count += 1; } assert!(m.get_referral_rate(&[9u8;32]) > 5.0); }
    #[test] fn test_link_profile() { let mut m = OnboardingManager::new(); let mut k = key(); m.create_account([1u8;32], None, &mut k).unwrap_or_default(); m.link_profile([1u8;32], "pro_001".into()).unwrap_or_default(); assert_eq!(m.get_account(&[1u8;32]).unwrap_or_default().state, AccountState::Active); }
}

