use serde::{Serialize, Deserialize};
use sha2::{Sha256, Digest};
use crate::error::ChainError;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LendingPool {
    pub loans: HashMap<[u8; 32], Loan>,
    pub offers: HashMap<[u8; 32], LendOffer>,
    pub total_lent: u64, pub total_repaid: u64, pub total_defaulted: u64, pub active_loans: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Loan {
    pub id: [u8; 32], pub borrower: [u8; 32], pub lender: [u8; 32],
    pub principal: u64, pub collateral: u64, pub interest_rate_bps: u64,
    pub duration_days: u64, pub start_time: i64, pub end_time: i64,
    pub repaid_amount: u64, pub status: LoanStatus, pub late_penalties: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LoanStatus { Active, Repaid, Defaulted, Liquidated, Cancelled }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LendOffer {
    pub id: [u8; 32], pub lender: [u8; 32], pub amount: u64,
    pub min_collateral_ratio: u64, pub max_duration_days: u64,
    pub interest_rate_bps: u64, pub is_active: bool, pub filled_amount: u64, pub created_at: i64,
}

impl LendingPool {
    pub fn new() -> Self { LendingPool { loans: HashMap::new(), offers: HashMap::new(), total_lent: 0, total_repaid: 0, total_defaulted: 0, active_loans: 0 } }

    pub fn create_offer(&mut self, lender: [u8;32], amount: u64, min_cr: u64, max_days: u64, rate: u64) -> Result<[u8;32], ChainError> {
        if amount == 0 { return Err(ChainError::InternalError("Montant nul".into())); }
        let now = chrono::Utc::now().timestamp();
        let mut h = Sha256::new(); h.update(&lender); h.update(&amount.to_le_bytes()); h.update(&now.to_le_bytes());
        let mut id = [0u8;32]; id.copy_from_slice(&h.finalize());
        self.offers.insert(id, LendOffer { id, lender, amount, min_collateral_ratio: min_cr, max_duration_days: max_days, interest_rate_bps: rate, is_active: true, filled_amount: 0, created_at: now });
        Ok(id)
    }

    pub fn borrow(&mut self, borrower: [u8;32], offer_id: &[u8;32], amount: u64, collateral: u64, days: u64) -> Result<[u8;32], ChainError> {
        let offer = self.offers.get_mut(offer_id).ok_or_else(|| ChainError::InternalError("Offre introuvable".into()))?;
        if !offer.is_active { return Err(ChainError::InternalError("Offre inactive".into())); }
        let avail = offer.amount.saturating_sub(offer.filled_amount);
        if amount > avail { return Err(ChainError::InternalError("Montant indisponible".into())); }
        let cr = (collateral as u128 * 10000 / amount as u128) as u64;
        if cr < offer.min_collateral_ratio { return Err(ChainError::InternalError("Collatéral insuffisant".into())); }
        let now = chrono::Utc::now().timestamp();
        let end = now + (days as i64 * 86400);
        let mut h = Sha256::new(); h.update(&borrower); h.update(&offer.lender); h.update(&amount.to_le_bytes()); h.update(&now.to_le_bytes());
        let mut id = [0u8;32]; id.copy_from_slice(&h.finalize());
        offer.filled_amount = offer.filled_amount.saturating_add(amount);
        if offer.filled_amount >= offer.amount { offer.is_active = false; }
        self.loans.insert(id, Loan { id, borrower, lender: offer.lender, principal: amount, collateral, interest_rate_bps: offer.interest_rate_bps, duration_days: days, start_time: now, end_time: end, repaid_amount: 0, status: LoanStatus::Active, late_penalties: 0 });
        self.total_lent = self.total_lent.saturating_add(amount); self.active_loans = self.active_loans.saturating_add(1);
        Ok(id)
    }

    pub fn repay(&mut self, loan_id: &[u8;32], amount: u64, current_time: i64) -> Result<u64, ChainError> {
        let loan = self.loans.get_mut(loan_id).ok_or_else(|| ChainError::InternalError("Prêt introuvable".into()))?;
        if loan.status != LoanStatus::Active { return Err(ChainError::InternalError("Prêt non actif".into())); }
        let interest = loan.principal * loan.interest_rate_bps / 10000 * loan.duration_days / 365;
        let total_due = loan.principal.saturating_add(interest).saturating_add(loan.late_penalties);
        let remaining = total_due.saturating_sub(loan.repaid_amount);
        let payment = amount.min(remaining);
        loan.repaid_amount = loan.repaid_amount.saturating_add(payment);
        if current_time > loan.end_time { let late = ((current_time - loan.end_time) / 86400) as u64; loan.late_penalties = loan.principal * late / 100; }
        if loan.repaid_amount >= total_due { loan.status = LoanStatus::Repaid; self.total_repaid = self.total_repaid.saturating_add(total_due); self.active_loans = self.active_loans.saturating_sub(1); }
        Ok(payment)
    }

    pub fn liquidate(&mut self, loan_id: &[u8;32], current_time: i64) -> Result<u64, ChainError> {
        let (can_liq, collat) = {
            let loan = self.loans.get(loan_id).ok_or_else(|| ChainError::InternalError("Prêt introuvable".into()))?;
            if loan.status != LoanStatus::Active { return Err(ChainError::InternalError("Prêt non actif".into())); }
            let total_due = loan.principal + (loan.principal * loan.interest_rate_bps / 10000);
            (current_time >= loan.end_time + 7*86400 || (current_time >= loan.end_time && loan.repaid_amount < total_due/2), loan.collateral)
        };
        if can_liq { let loan = self.loans.get_mut(loan_id).unwrap(); loan.status = LoanStatus::Liquidated; self.total_defaulted = self.total_defaulted.saturating_add(collat); self.active_loans = self.active_loans.saturating_sub(1); Ok(collat) }
        else { Err(ChainError::InternalError("Conditions non remplies".into())) }
    }

    pub fn calculate_total_due(&self, loan_id: &[u8;32]) -> Result<u64, ChainError> {
        let loan = self.loans.get(loan_id).ok_or_else(|| ChainError::InternalError("Prêt introuvable".into()))?;
        Ok(loan.principal.saturating_add(loan.principal * loan.interest_rate_bps / 10000 * loan.duration_days / 365).saturating_add(loan.late_penalties))
    }

    pub fn get_stats(&self) -> LendingStats { LendingStats { total_offers: self.offers.len() as u64, total_loans: self.loans.len() as u64, active_loans: self.active_loans, total_lent: self.total_lent, total_repaid: self.total_repaid, total_defaulted: self.total_defaulted, repaid_loans: self.loans.values().filter(|l| l.status == LoanStatus::Repaid).count() as u64, defaulted_loans: self.loans.values().filter(|l| l.status == LoanStatus::Liquidated).count() as u64, default_rate: 0 } }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LendingStats { pub total_offers: u64, pub total_loans: u64, pub active_loans: u64, pub total_lent: u64, pub total_repaid: u64, pub total_defaulted: u64, pub repaid_loans: u64, pub defaulted_loans: u64, pub default_rate: u64 }

#[cfg(test)]
mod tests {
    use super::*;
    fn pool() -> LendingPool { LendingPool::new() }
    #[test] fn test_create_offer() { let mut p = pool(); assert!(p.create_offer([1u8;32],100_000_000,15000,30,500).is_ok()); }
    #[test] fn test_borrow() { let mut p = pool(); let oid = p.create_offer([1u8;32],100_000_000,15000,30,500).unwrap_or_default(); assert!(p.borrow([2u8;32],&oid,50_000_000,100_000_000,30).is_ok()); }
    #[test] fn test_insufficient_collateral() { let mut p = pool(); let oid = p.create_offer([1u8;32],100_000_000,20000,30,500).unwrap_or_default(); assert!(p.borrow([2u8;32],&oid,50_000_000,50_000_000,30).is_err()); }
    #[test] fn test_repay() { let mut p = pool(); let oid = p.create_offer([1u8;32],100_000_000,15000,30,500).unwrap_or_default(); let lid = p.borrow([2u8;32],&oid,50_000_000,100_000_000,30).unwrap_or_default(); assert!(p.repay(&lid,50_000_000,100000).is_ok()); }
    #[test] fn test_liquidate() { let mut p = pool(); let now = chrono::Utc::now().timestamp(); let oid = p.create_offer([1u8;32],100_000_000,15000,1,500).unwrap_or_default(); let lid = p.borrow([2u8;32],&oid,50_000_000,100_000_000,1).unwrap_or_default(); let future = now + 15*86400; assert!(p.liquidate(&lid,future).is_ok()); }
    #[test] fn test_calculate_total_due() { let mut p = pool(); let oid = p.create_offer([1u8;32],100_000_000,15000,30,500).unwrap_or_default(); let lid = p.borrow([2u8;32],&oid,50_000_000,100_000_000,30).unwrap_or_default(); assert!(p.calculate_total_due(&lid).unwrap_or_default() >= 50_000_000); }
    #[test] fn test_get_stats() { let mut p = pool(); p.create_offer([1u8;32],100_000_000,15000,30,500).unwrap_or_default(); assert_eq!(p.get_stats().total_offers,1); }
}

