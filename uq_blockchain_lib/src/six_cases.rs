use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use crate::bubble::BubbleRegistry;
use crate::error::ChainError;

pub const TOTAL_SUPPLY_BORB: u64 = 40_000_000_000;
pub const MICRO_PER_BORB: u64 = 1_000_000;
pub const CASE1_TREASURY: u64 = 31_120_000_000;
pub const CASE2_FEES: u64 = 2_240_000_000;
pub const CASE3_REWARDS: u64 = 2_240_000_000;
pub const CASE4_SAVINGS: u64 = 2_240_000_000;
pub const CASE5_CREDIT: u64 = 1_320_000_000;
pub const CASE6_SHIELD: u64 = 840_000_000;
pub const TX_FEE_RATE: u64 = 10;
pub const MAX_TREASURY_DURATION: u64 = 10 * 365 * 24 * 3600;
pub const MAX_CONCENTRATION_BASIS_POINTS: u64 = 1000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaseVault {
    pub name: String,
    pub balance_BORB: u64,
    pub bubble_ids: Vec<[u8; 32]>,
    pub inbound_tx: u64,
    pub outbound_tx: u64,
    pub rules: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SixCases {
    pub treasury: CaseVault, pub fees: CaseVault, pub rewards: CaseVault,
    pub savings: CaseVault, pub credit: CaseVault, pub shield: CaseVault,
    pub registry: BubbleRegistry, pub tx_count: u64, pub created_at: u64,
}

impl SixCases {
    pub fn new(registry: BubbleRegistry) -> Result<Self, ChainError> {
        let now = Self::now();
        let default_rules = HashMap::new();
        Ok(SixCases {
            treasury: CaseVault { name: "Case 1 - Tresor".into(), balance_BORB: CASE1_TREASURY, bubble_ids: vec![], inbound_tx: 0, outbound_tx: 0, rules: default_rules.clone() },
            fees: CaseVault { name: "Case 2 - Frais".into(), balance_BORB: CASE2_FEES, bubble_ids: vec![], inbound_tx: 0, outbound_tx: 0, rules: default_rules.clone() },
            rewards: CaseVault { name: "Case 3 - Recompenses".into(), balance_BORB: CASE3_REWARDS, bubble_ids: vec![], inbound_tx: 0, outbound_tx: 0, rules: default_rules.clone() },
            savings: CaseVault { name: "Case 4 - Epargne".into(), balance_BORB: CASE4_SAVINGS, bubble_ids: vec![], inbound_tx: 0, outbound_tx: 0, rules: default_rules.clone() },
            credit: CaseVault { name: "Case 5 - Credit".into(), balance_BORB: CASE5_CREDIT, bubble_ids: vec![], inbound_tx: 0, outbound_tx: 0, rules: default_rules.clone() },
            shield: CaseVault { name: "Case 6 - Bouclier".into(), balance_BORB: CASE6_SHIELD, bubble_ids: vec![], inbound_tx: 0, outbound_tx: 0, rules: default_rules },
            registry, tx_count: 0, created_at: now,
        })
    }

    pub fn verify_total_invariant(&self, all_users_balance: u64) -> Result<bool, ChainError> {
        let cases_total = self.treasury.balance_BORB + self.fees.balance_BORB + self.rewards.balance_BORB
            + self.savings.balance_BORB + self.credit.balance_BORB + self.shield.balance_BORB;
        let grand_total = cases_total + all_users_balance;
        if grand_total != TOTAL_SUPPLY_BORB {
            return Err(ChainError::InternalError(format!("INVARIANT VIOLÉ: {} != {}", grand_total, TOTAL_SUPPLY_BORB)));
        }
        Ok(true)
    }

    pub fn collect_fee(&mut self, amount_BORB: u64) -> Result<u64, ChainError> {
        let fee = (amount_BORB * TX_FEE_RATE / 100_000).max(1);
        self.fees.balance_BORB += fee / MICRO_PER_BORB;
        self.fees.inbound_tx += 1; self.tx_count += 1;
        Ok(fee)
    }

    pub fn is_concentration_warning(&self, address: &[u8; 32]) -> bool {
        let balance = self.registry.get_balance(address);
        balance / MICRO_PER_BORB > (TOTAL_SUPPLY_BORB * MAX_CONCENTRATION_BASIS_POINTS / 10000)
    }

    pub fn check_treasury_expiry(&mut self) -> Result<(), ChainError> {
        if Self::now() > self.created_at + MAX_TREASURY_DURATION && !self.treasury.rules.contains_key("expired") {
            self.treasury.rules.insert("expired".into(), "true".into());
            let remaining = self.treasury.balance_BORB;
            self.treasury.balance_BORB = 0;
            self.rewards.balance_BORB += remaining * 60 / 100;
            self.fees.balance_BORB += remaining * 40 / 100;
        }
        Ok(())
    }

    pub fn collect_fee_with_sender(&mut self, amount_BORB: u64, sender: &[u8; 32]) -> Result<u64, ChainError> {
        let rate = if self.is_concentration_warning(sender) { TX_FEE_RATE * 2 } else { TX_FEE_RATE };
        let fee = (amount_BORB * rate / 100_000).max(1);
        self.fees.balance_BORB += fee / MICRO_PER_BORB;
        self.fees.inbound_tx += 1; self.tx_count += 1;
        Ok(fee)
    }

    pub fn get_stats(&self) -> Vec<CaseStats> {
        vec![
            CaseStats { name: self.treasury.name.clone(), balance: self.treasury.balance_BORB },
            CaseStats { name: self.fees.name.clone(), balance: self.fees.balance_BORB },
            CaseStats { name: self.rewards.name.clone(), balance: self.rewards.balance_BORB },
            CaseStats { name: self.savings.name.clone(), balance: self.savings.balance_BORB },
            CaseStats { name: self.credit.name.clone(), balance: self.credit.balance_BORB },
            CaseStats { name: self.shield.name.clone(), balance: self.shield.balance_BORB },
        ]
    }

    fn now() -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaseStats { pub name: String, pub balance: u64 }

#[cfg(test)]
    
mod tests {
    fn test_key() -> UqBlackKey { let mut k = UqBlackKey::generate(); k.activate(); k }
    use uq_black_key_standalone::UqBlackKey;

    use super::*;
    #[test] fn test_create() { let r = BubbleRegistry::new(); let c = SixCases::new(r).unwrap(); assert_eq!(c.treasury.balance_BORB, CASE1_TREASURY); }
    #[test] fn test_invariant_ok() { let r = BubbleRegistry::new(); let c = SixCases::new(r).unwrap(); assert!(c.verify_total_invariant(0).unwrap()); }
    #[test] fn test_invariant_fail() { let r = BubbleRegistry::new(); let c = SixCases::new(r).unwrap(); assert!(c.verify_total_invariant(1).is_err()); }
    #[test] fn test_collect_fee() { let r = BubbleRegistry::new(); let mut c = SixCases::new(r).unwrap(); let fee = c.collect_fee(1000).unwrap(); assert!(fee > 0); }
    #[test] fn test_stats() { let r = BubbleRegistry::new(); let c = SixCases::new(r).unwrap(); let s = c.get_stats(); assert_eq!(s.len(), 6); }


    /// PASSERELLE TRÉSOR → FRAIS (automatique)
    /// Si Frais < 10% ET Trésor > plancher → renfloue
    pub fn auto_protect_fees(&mut self) -> u128 {
        if self.treasury > crate::constants::TREASURY_FLOOR 
            && self.fees < crate::constants::FEES_CRITICAL {
            let deficit = crate::constants::FEES_TARGET - self.fees;
            let max_possible = self.treasury - crate::constants::TREASURY_FLOOR;
            let transfer = deficit.min(max_possible);
            
            self.treasury -= transfer;
            self.fees += transfer;
            return transfer;
        }
        0
    }
    
    /// Vérifie si le Trésor est au-dessus du plancher
    pub fn is_treasury_safe(&self) -> bool {
        self.treasury >= crate::constants::TREASURY_FLOOR
    }
    
    /// DISTRIBUTION DES FRAIS (30% Trésor, 40% Validateurs, 30% Relais)
    // split_fees() SUPPRIMEE — Les 6 Cases gerent tout
    // Il n'y a plus de validateurs/relais/GPU a recompenser

    
    /// RÉPARTITION DU CONTENU 90/10
    /// Créateur 90%, 6 cases 10% (3.5/1.5/1.5/1/1.5/1)
    pub fn distribute_content_payment(&mut self, total: u128) -> u128 {
        let creator = total * 90 / 100;
        let cases = total * 10 / 100;
        
        self.treasury += cases * 35 / 100;   // 3.5%
        self.fees += cases * 15 / 100;        // 1.5%
        self.rewards += cases * 15 / 100;     // 1.5%
        self.savings += cases * 10 / 100;     // 1%
        self.credit += cases * 15 / 100;      // 1.5%
        self.shield += cases * 10 / 100;      // 1%
        
        creator
    }}







