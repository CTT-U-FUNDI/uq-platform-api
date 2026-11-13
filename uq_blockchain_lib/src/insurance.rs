use serde::{Serialize, Deserialize};
use sha2::{Sha256, Digest};
use crate::error::ChainError;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claim {
    pub id: [u8; 32], pub claimant: [u8; 32], pub trade_id: [u8; 32],
    pub amount_claimed: u64, pub reason: String, pub evidence: Vec<[u8; 32]>,
    pub status: ClaimStatus, pub votes_yes: u32, pub votes_no: u32,
    pub created_at: i64, pub resolved_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ClaimStatus { Pending, Approved, Rejected, PaidOut }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InsuranceFund {
    pub balance: u64, pub total_collected: u64, pub total_paid_out: u64,
    pub claims: HashMap<[u8; 32], Claim>,
    pub contribution_rate: u64, pub max_claim_amount: u64,
    pub max_claims_per_address: u32, pub addresses_claims: HashMap<[u8; 32], u32>,
}

impl InsuranceFund {
    pub fn new() -> Self {
        InsuranceFund { balance: 0, total_collected: 0, total_paid_out: 0,
            claims: HashMap::new(), contribution_rate: 10, max_claim_amount: 100_000_000,
            max_claims_per_address: 3, addresses_claims: HashMap::new() }
    }
    
    pub fn contribute(&mut self, amount: u64) -> u64 {
        let c = (amount * self.contribution_rate) / 10000;
        self.balance += c; self.total_collected += c; c
    }
    
    pub fn submit_claim(&mut self, claimant: [u8; 32], trade_id: [u8; 32], amount: u64, reason: String, evidence: Vec<[u8; 32]>) -> Result<[u8; 32], ChainError> {
        let count = self.addresses_claims.get(&claimant).copied().unwrap_or(0);
        if count >= self.max_claims_per_address { return Err(ChainError::InsuranceLimitReached("Max atteint".into())); }
        if amount > self.max_claim_amount { return Err(ChainError::InvalidClaim("Montant max dépassé".into())); }
        if evidence.is_empty() { return Err(ChainError::InvalidClaim("Preuves requises".into())); }
        let now = chrono::Utc::now().timestamp();
        let claim = Claim { id: [0u8;32], claimant, trade_id, amount_claimed: amount, reason, evidence, status: ClaimStatus::Pending, votes_yes: 0, votes_no: 0, created_at: now, resolved_at: None };
        let id = Self::compute_id(&claim);
        self.claims.insert(id, claim);
        *self.addresses_claims.entry(claimant).or_insert(0) += 1;
        Ok(id)
    }
    
    pub fn vote_claim(&mut self, claim_id: &[u8; 32], approve: bool, _v: [u8; 32]) -> Result<(), ChainError> {
        let claim = self.claims.get_mut(claim_id).ok_or_else(|| ChainError::InternalError("Introuvable".into()))?;
        if claim.status != ClaimStatus::Pending { return Err(ChainError::InternalError("Déjà résolue".into())); }
        if approve { claim.votes_yes += 1; } else { claim.votes_no += 1; }
        if claim.votes_yes >= 5 && claim.votes_yes > claim.votes_no { claim.status = ClaimStatus::Approved; claim.resolved_at = Some(chrono::Utc::now().timestamp()); }
        if claim.votes_no >= 5 && claim.votes_no > claim.votes_yes { claim.status = ClaimStatus::Rejected; claim.resolved_at = Some(chrono::Utc::now().timestamp()); }
        Ok(())
    }
    
    pub fn payout(&mut self, claim_id: &[u8; 32]) -> Result<u64, ChainError> {
        let claim = self.claims.get_mut(claim_id).ok_or_else(|| ChainError::InternalError("Introuvable".into()))?;
        if claim.status != ClaimStatus::Approved { return Err(ChainError::InternalError("Non approuvée".into())); }
        self.balance -= claim.amount_claimed; self.total_paid_out += claim.amount_claimed;
        claim.status = ClaimStatus::PaidOut; Ok(claim.amount_claimed)
    }
    
    pub fn get_balance(&self) -> u64 { self.balance }
    fn compute_id(claim: &Claim) -> [u8; 32] {
        let mut h = Sha256::new(); h.update(&claim.claimant); h.update(&claim.trade_id);
        h.update(&claim.amount_claimed.to_le_bytes()); h.update(&claim.created_at.to_le_bytes());
        let r = h.finalize(); let mut id = [0u8;32]; id.copy_from_slice(&r); id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_contribute() {
        let mut f = InsuranceFund::new();
        let c = f.contribute(1_000_000);
        assert!(c > 0); assert_eq!(f.get_balance(), c);
    }
    
    #[test]
    fn test_submit_claim() {
        let mut f = InsuranceFund::new();
        f.contribute(100_000_000);
        let id = f.submit_claim([1u8;32],[2u8;32],1000,"Test".into(),vec![[3u8;32]]).unwrap_or_default();
        assert!(f.claims.contains_key(&id));
    }
    
    #[test]
    fn test_vote_payout() {
        let mut f = InsuranceFund::new();
        f.contribute(100_000_000);
        let id = f.submit_claim([1u8;32],[2u8;32],1000,"Test".into(),vec![[3u8;32]]).unwrap_or_default();
        for _ in 0..5 { f.vote_claim(&id, true, [9u8;32]).unwrap_or_default(); }
        assert_eq!(f.claims.get(&id).unwrap_or_default().status, ClaimStatus::Approved);
        assert_eq!(f.payout(&id).unwrap_or_default(), 1000);
    }
    
    #[test]
    fn test_max() {
        let mut f = InsuranceFund::new();
        f.contribute(100_000_000);
        for _ in 0..3 { f.submit_claim([1u8;32],[2u8;32],1000,"T".into(),vec![[3u8;32]]).unwrap_or_default(); }
        assert!(f.submit_claim([1u8;32],[2u8;32],1000,"T".into(),vec![[3u8;32]]).is_err());
    }
}
