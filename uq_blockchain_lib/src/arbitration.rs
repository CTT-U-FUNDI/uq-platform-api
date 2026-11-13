use serde::{Serialize, Deserialize};
use sha2::{Sha256, Digest};
use crate::error::ChainError;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisputeCase {
    pub id: [u8; 32], pub trade_id: [u8; 32],
    pub initiator: [u8; 32], pub respondent: [u8; 32],
    pub amount_disputed: u64, pub reason: String,
    pub evidence_initiator: Vec<[u8; 32]>, pub evidence_respondent: Vec<[u8; 32]>,
    pub status: DisputeStatus, pub arbitrators: Vec<[u8; 32]>,
    pub votes_initiator: u32, pub votes_respondent: u32,
    pub created_at: i64, pub resolved_at: Option<i64>, pub winner: Option<[u8; 32]>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DisputeStatus { Open, InProgress, Resolved, Appealed, FinalResolved }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArbitrationVote {
    pub dispute_id: [u8; 32], pub arbitrator: [u8; 32],
    pub vote_for_initiator: bool, pub reason: String, pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArbitrationPool {
    pub cases: HashMap<[u8; 32], DisputeCase>,
    pub votes: HashMap<[u8; 32], Vec<ArbitrationVote>>,
    pub eligible_arbitrators: Vec<[u8; 32]>,
    pub total_cases_resolved: u64,
    pub arbitrator_reward: u64,
}

impl ArbitrationPool {
    pub fn new() -> Self {
        ArbitrationPool { cases: HashMap::new(), votes: HashMap::new(),
            eligible_arbitrators: Vec::new(), total_cases_resolved: 0, arbitrator_reward: 100_000 }
    }
    
    pub fn register_arbitrator(&mut self, a: [u8; 32]) { if !self.eligible_arbitrators.contains(&a) { self.eligible_arbitrators.push(a); } }
    
    pub fn open_dispute(&mut self, initiator: [u8; 32], respondent: [u8; 32], trade_id: [u8; 32], amount: u64, reason: String, evidence: Vec<[u8; 32]>) -> Result<[u8; 32], ChainError> {
        if self.eligible_arbitrators.len() < 5 { return Err(ChainError::ArbitrationDenied("Pas assez d'arbitres".into())); }
        if evidence.is_empty() { return Err(ChainError::InvalidProof("Preuves requises".into())); }
        let arbs = self.select_random_arbitrators(5);
        let now = chrono::Utc::now().timestamp();
        let case = DisputeCase { id: [0u8;32], trade_id, initiator, respondent, amount_disputed: amount, reason,
            evidence_initiator: evidence, evidence_respondent: vec![], status: DisputeStatus::Open,
            arbitrators: arbs, votes_initiator: 0, votes_respondent: 0, created_at: now, resolved_at: None, winner: None };
        let id = Self::compute_id(&case);
        self.cases.insert(id, case);
        Ok(id)
    }
    
    pub fn submit_defense(&mut self, case_id: &[u8; 32], respondent: [u8; 32], evidence: Vec<[u8; 32]>) -> Result<(), ChainError> {
        let case = self.cases.get_mut(case_id).ok_or_else(|| ChainError::InternalError("Introuvable".into()))?;
        if case.respondent != respondent { return Err(ChainError::InternalError("Non autorisé".into())); }
        case.evidence_respondent = evidence; case.status = DisputeStatus::InProgress; Ok(())
    }
    
    pub fn cast_vote(&mut self, case_id: &[u8; 32], arbitrator: [u8; 32], vote_for_initiator: bool, reason: String) -> Result<(), ChainError> {
        let case = self.cases.get_mut(case_id).ok_or_else(|| ChainError::InternalError("Introuvable".into()))?;
        if case.status != DisputeStatus::InProgress && case.status != DisputeStatus::Open { return Err(ChainError::InternalError("Non actif".into())); }
        if !case.arbitrators.contains(&arbitrator) { return Err(ChainError::ArbitrationDenied("Non sélectionné".into())); }
        let existing = self.votes.entry(*case_id).or_insert_with(Vec::new);
        if existing.iter().any(|v| v.arbitrator == arbitrator) { return Err(ChainError::InternalError("Déjà voté".into())); }
        existing.push(ArbitrationVote { dispute_id: *case_id, arbitrator, vote_for_initiator, reason, timestamp: chrono::Utc::now().timestamp() });
        if vote_for_initiator { case.votes_initiator += 1; } else { case.votes_respondent += 1; }
        if case.votes_initiator >= 3 { case.status = DisputeStatus::Resolved; case.winner = Some(case.initiator); case.resolved_at = Some(chrono::Utc::now().timestamp()); self.total_cases_resolved += 1; }
        else if case.votes_respondent >= 3 { case.status = DisputeStatus::Resolved; case.winner = Some(case.respondent); case.resolved_at = Some(chrono::Utc::now().timestamp()); self.total_cases_resolved += 1; }
        Ok(())
    }
    
    fn select_random_arbitrators(&self, count: usize) -> Vec<[u8; 32]> {
        let pool = &self.eligible_arbitrators;
        if pool.is_empty() { return vec![]; }
        let seed = chrono::Utc::now().timestamp() as u64;
        (0..count.min(pool.len())).map(|i| pool[((seed + i as u64) as usize) % pool.len()]).collect()
    }
    
    fn compute_id(case: &DisputeCase) -> [u8; 32] {
        let mut h = Sha256::new(); h.update(&case.initiator); h.update(&case.respondent);
        h.update(&case.trade_id); h.update(&case.created_at.to_le_bytes());
        let r = h.finalize(); let mut id = [0u8;32]; id.copy_from_slice(&r); id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    fn pool() -> ArbitrationPool {
        let mut p = ArbitrationPool::new();
        for i in 1..=10 { p.register_arbitrator([i as u8;32]); }
        p
    }
    
    #[test]
    fn test_open() {
        let mut p = pool();
        let id = p.open_dispute([1u8;32],[2u8;32],[3u8;32],1000,"T".into(),vec![[4u8;32]]).unwrap_or_default();
        assert!(p.cases.contains_key(&id));
    }
    
    #[test]
    fn test_vote() {
        let mut p = pool();
        let id = p.open_dispute([1u8;32],[2u8;32],[3u8;32],1000,"T".into(),vec![[4u8;32]]).unwrap_or_default();
        let arbs = p.cases.get(&id).unwrap_or_default().arbitrators.clone();
        for a in arbs.iter().take(3) { p.cast_vote(&id, *a, true, "OK".into()).unwrap_or_default(); }
        assert_eq!(p.cases.get(&id).unwrap_or_default().status, DisputeStatus::Resolved);
    }
}
