use serde::{Serialize, Deserialize};
use sha2::{Sha256, Digest};
use crate::error::ChainError;
use crate::constants::*;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreditRequest {
    pub id: [u8; 32], pub borrower: [u8; 32], pub amount: u64,
    pub duration_days: u64, pub interest_rate: u64, pub purpose: String,
    pub status: CreditStatus, pub vouchers: Vec<[u8; 32]>,
    pub created_at: i64, pub expires_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CreditStatus { Open, Funded, Active, Repaid, Defaulted, Cancelled }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreditScore {
    pub address: [u8; 32], pub score: u64,
    pub total_borrowed: u64, pub total_repaid: u64,
    pub active_loans: u64, pub defaulted_loans: u64,
    pub vouches_received: u64, pub vouches_given: u64, pub last_updated: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveLoan {
    pub id: [u8; 32], pub credit_request_id: [u8; 32],
    pub lender: [u8; 32], pub borrower: [u8; 32],
    pub amount: u64, pub remaining: u64, pub interest_rate: u64,
    pub start_date: i64, pub due_date: i64,
    pub status: CreditStatus, pub payments: Vec<LoanPayment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoanPayment { pub amount: u64, pub timestamp: i64 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreditPool {
    pub requests: HashMap<[u8; 32], CreditRequest>,
    pub active_loans: HashMap<[u8; 32], ActiveLoan>,
    pub credit_scores: HashMap<[u8; 32], CreditScore>,
    pub total_lent: u64, pub total_repaid: u64, pub default_rate: u64,
}

impl CreditPool {
    pub fn new() -> Self {
        CreditPool { requests: HashMap::new(), active_loans: HashMap::new(), credit_scores: HashMap::new(), total_lent: 0, total_repaid: 0, default_rate: 0 }
    }
    
    pub fn create_request(&mut self, borrower: [u8; 32], amount: u64, duration_days: u64, interest_rate: u64, purpose: String) -> Result<[u8; 32], ChainError> {
        let score = self.get_score(&borrower);
        if score < 300 { return Err(ChainError::TrustTooLow(format!("Score: {}", score))); }
        if amount > MICRO_CREDIT_MAX_BORB * MICRO_PER_BORB { return Err(ChainError::InsufficientFunds("Montant trop élevé".into())); }
        let now = chrono::Utc::now().timestamp();
        let req = CreditRequest { id: [0u8;32], borrower, amount, duration_days, interest_rate, purpose, status: CreditStatus::Open, vouchers: vec![], created_at: now, expires_at: now + 7*86400 };
        let id = Self::hash_id(&[&req.borrower, &req.amount.to_le_bytes(), &req.created_at.to_le_bytes()]);
        self.requests.insert(id, req);
        Ok(id)
    }
    
    pub fn add_voucher(&mut self, request_id: &[u8; 32], voucher: [u8; 32]) -> Result<(), ChainError> {
        let vouch_score = self.get_score(&voucher);
        if vouch_score < 500 { return Err(ChainError::TrustTooLow("Score garant insuffisant".into())); }
        if let Some(req) = self.requests.get_mut(request_id) {
            if req.vouchers.contains(&voucher) { return Err(ChainError::InternalError("Déjà garant".into())); }
            req.vouchers.push(voucher);
        }
        Ok(())
    }
    
    pub fn fund_loan(&mut self, request_id: &[u8; 32], lender: [u8; 32]) -> Result<[u8; 32], ChainError> {
        let req = self.requests.get(request_id).cloned().ok_or_else(|| ChainError::InternalError("Demande introuvable".into()))?;
        if req.status != CreditStatus::Open { return Err(ChainError::InternalError("Non ouverte".into())); }
        let now = chrono::Utc::now().timestamp();
        let total_due = req.amount + ((req.amount * req.interest_rate) / 10000);
        let loan = ActiveLoan { id: [0u8;32], credit_request_id: *request_id, lender, borrower: req.borrower, amount: req.amount, remaining: total_due, interest_rate: req.interest_rate, start_date: now, due_date: now + req.duration_days as i64 * 86400, status: CreditStatus::Active, payments: vec![] };
        let loan_id = Self::hash_id(&[&loan.credit_request_id, &loan.lender, &loan.start_date.to_le_bytes()]);
        if let Some(r) = self.requests.get_mut(request_id) { r.status = CreditStatus::Funded; }
        self.active_loans.insert(loan_id, loan);
        self.total_lent += req.amount;
        Ok(loan_id)
    }
    
    pub fn make_payment(&mut self, loan_id: &[u8; 32], amount: u64) -> Result<u64, ChainError> {
        let loan = self.active_loans.get_mut(loan_id).ok_or_else(|| ChainError::InternalError("Prêt introuvable".into()))?;
        if loan.status != CreditStatus::Active { return Err(ChainError::InternalError("Non actif".into())); }
        if amount > loan.remaining { return Err(ChainError::InsufficientFunds("Trop élevé".into())); }
        loan.remaining -= amount;
        loan.payments.push(LoanPayment { amount, timestamp: chrono::Utc::now().timestamp() });
        self.total_repaid += amount;
        if loan.remaining == 0 { loan.status = CreditStatus::Repaid; }
        Ok(loan.remaining)
    }
    
    pub fn get_score(&self, address: &[u8; 32]) -> u64 {
        self.credit_scores.get(address).map(|s| s.score).unwrap_or(500)
    }
    
    fn hash_id(data: &[&[u8]]) -> [u8; 32] {
        let mut h = Sha256::new();
        for d in data { h.update(d); }
        let r = h.finalize(); let mut id = [0u8; 32]; id.copy_from_slice(&r); id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_create() {
        let mut p = CreditPool::new();
        assert!(p.create_request([1u8;32], 1_000_000, 30, 0, "Test".into()).is_ok());
    }
    
    #[test]
    fn test_fund_repay() {
        let mut p = CreditPool::new();
        let rid = p.create_request([1u8;32], 1_000_000, 30, 0, "Test".into()).unwrap_or_default();
        let lid = p.fund_loan(&rid, [9u8;32]).unwrap_or_default();
        let r = p.make_payment(&lid, 500_000).unwrap_or_default();
        assert!(r > 0);
        p.make_payment(&lid, r).unwrap_or_default();
    }
}


