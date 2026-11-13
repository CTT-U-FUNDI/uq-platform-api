use serde::{Serialize, Deserialize};
use sha2::{Sha256, Digest};
use crate::error::ChainError;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernanceDao {
    pub proposals: HashMap<[u8; 32], Proposal>,
    pub votes: HashMap<[u8; 32], Vec<Vote>>,
    pub members: HashMap<[u8; 32], DaoMember>,
    pub total_proposals: u64,
    pub total_votes_cast: u64,
    pub quorum_percent: u64,
    pub voting_period_days: u64,
    pub execution_delay_days: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proposal {
    pub id: [u8; 32], pub proposer: [u8; 32], pub title: String, pub description: String,
    pub proposal_type: ProposalType, pub created_at: i64, pub voting_ends_at: i64,
    pub executed_at: Option<i64>, pub status: ProposalStatus,
    pub votes_for: u64, pub votes_against: u64, pub votes_abstain: u64, pub total_voting_power: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ProposalType { ParameterChange(String,String), TreasurySpend([u8;32],u64), AddValidator([u8;32]), RemoveValidator([u8;32]), TextProposal }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ProposalStatus { Active, Passed, Rejected, Executed, Cancelled, Expired }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vote { pub proposal_id: [u8;32], pub voter: [u8;32], pub vote_type: VoteType, pub voting_power: u64, pub timestamp: i64, pub reason: Option<String> }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum VoteType { For, Against, Abstain }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaoMember { pub address: [u8;32], pub voting_power: u64, pub joined_at: i64, pub proposals_created: u64, pub votes_cast: u64, pub is_active: bool }

impl GovernanceDao {
    pub fn new(q: u64, v: u64) -> Self { GovernanceDao { proposals: HashMap::new(), votes: HashMap::new(), members: HashMap::new(), total_proposals: 0, total_votes_cast: 0, quorum_percent: q, voting_period_days: v, execution_delay_days: 2 } }
    pub fn register_member(&mut self, a: [u8;32], p: u64) -> Result<(), ChainError> { if self.members.contains_key(&a) { return Err(ChainError::InternalError("Déjà membre".into())); } self.members.insert(a, DaoMember { address:a, voting_power:p, joined_at:chrono::Utc::now().timestamp(), proposals_created:0, votes_cast:0, is_active:true }); Ok(()) }
    pub fn get_total_voting_power(&self) -> u64 { self.members.values().filter(|m| m.is_active).map(|m| m.voting_power).sum() }
    pub fn create_proposal(&mut self, proposer: [u8;32], title: String, desc: String, ptype: ProposalType) -> Result<[u8;32], ChainError> {
        if !self.members.contains_key(&proposer) { return Err(ChainError::InternalError("Non membre".into())); }
        let now = chrono::Utc::now().timestamp();
        let ends = now + (self.voting_period_days as i64 * 86400);
        let tp = self.get_total_voting_power();
        let proposal = Proposal { id:[0u8;32], proposer, title, description:desc, proposal_type:ptype, created_at:now, voting_ends_at:ends, executed_at:None, status:ProposalStatus::Active, votes_for:0, votes_against:0, votes_abstain:0, total_voting_power:tp };
        let id = Self::hash(&proposal);
        let mut p = proposal; p.id = id;
        self.proposals.insert(id, p);
        self.total_proposals = self.total_proposals.saturating_add(1);
        Ok(id)
    }
    pub fn vote(&mut self, voter: [u8;32], pid: &[u8;32], vtype: VoteType, reason: Option<String>) -> Result<(), ChainError> {
        let p = self.proposals.get_mut(pid).ok_or_else(|| ChainError::InternalError("Introuvable".into()))?;
        if p.status != ProposalStatus::Active { return Err(ChainError::InternalError("Non actif".into())); }
        let now = chrono::Utc::now().timestamp();
        if now > p.voting_ends_at { p.status = ProposalStatus::Expired; return Err(ChainError::InternalError("Expiré".into())); }
        let m = self.members.get(&voter).ok_or_else(|| ChainError::InternalError("Non membre".into()))?;
        let power = m.voting_power;
        let votes = self.votes.entry(*pid).or_insert_with(Vec::new);
        if votes.iter().any(|v| v.voter == voter) { return Err(ChainError::InternalError("Déjà voté".into())); }
        votes.push(Vote { proposal_id:*pid, voter, vote_type:vtype.clone(), voting_power:power, timestamp:now, reason });
        match vtype { VoteType::For => p.votes_for += power, VoteType::Against => p.votes_against += power, VoteType::Abstain => p.votes_abstain += power }
        self.total_votes_cast += 1;
        let total = p.votes_for + p.votes_against + p.votes_abstain;
        if total >= p.total_voting_power * self.quorum_percent / 100 { p.status = if p.votes_for > p.votes_against { ProposalStatus::Passed } else { ProposalStatus::Rejected }; }
        Ok(())
    }
    pub fn cancel_proposal(&mut self, pid: &[u8;32], canceller: [u8;32]) -> Result<(), ChainError> {
        let p = self.proposals.get_mut(pid).ok_or_else(|| ChainError::InternalError("Introuvable".into()))?;
        if p.proposer != canceller { return Err(ChainError::InternalError("Non proposant".into())); }
        p.status = ProposalStatus::Cancelled; Ok(())
    }
    pub fn execute_proposal(&mut self, pid: &[u8;32]) -> Result<(), ChainError> {
        let p = self.proposals.get_mut(pid).ok_or_else(|| ChainError::InternalError("Introuvable".into()))?;
        if p.status != ProposalStatus::Passed { return Err(ChainError::InternalError("Non acceptée".into())); }
        p.status = ProposalStatus::Executed; p.executed_at = Some(chrono::Utc::now().timestamp()); Ok(())
    }
    pub fn get_stats(&self) -> DaoStats { DaoStats { total_members: self.members.len() as u64, total_proposals: self.total_proposals, active_proposals: self.proposals.values().filter(|p| p.status==ProposalStatus::Active).count() as u64, passed_proposals: self.proposals.values().filter(|p| p.status==ProposalStatus::Passed||p.status==ProposalStatus::Executed).count() as u64, total_votes_cast: self.total_votes_cast, quorum_percent: self.quorum_percent, total_voting_power: self.get_total_voting_power() } }
    fn hash(p: &Proposal) -> [u8;32] { let mut h = Sha256::new(); h.update(&p.proposer); h.update(p.title.as_bytes()); h.update(&p.created_at.to_le_bytes()); let mut id=[0u8;32]; id.copy_from_slice(&h.finalize()); id }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaoStats { pub total_members: u64, pub total_proposals: u64, pub active_proposals: u64, pub passed_proposals: u64, pub total_votes_cast: u64, pub quorum_percent: u64, pub total_voting_power: u64 }

#[cfg(test)]
mod tests {
    use super::*;
    fn dao() -> GovernanceDao { let mut d = GovernanceDao::new(20, 7); d.register_member([1u8;32],1000).unwrap_or_default(); d.register_member([2u8;32],500).unwrap_or_default(); d.register_member([3u8;32],300).unwrap_or_default(); d }
    
    #[test] fn test_register() { assert_eq!(dao().members.len(),3); }
    #[test] fn test_create() { let mut d = dao(); assert!(d.create_proposal([1u8;32],"T".into(),"D".into(),ProposalType::TextProposal).is_ok()); }
    #[test] fn test_vote() { let mut d = dao(); let id = d.create_proposal([1u8;32],"T".into(),"D".into(),ProposalType::TextProposal).unwrap_or_default(); let p = d.proposals.get_mut(&id).unwrap_or_default(); p.voting_ends_at = chrono::Utc::now().timestamp() + 86400; p.status = ProposalStatus::Active; assert!(d.vote([1u8;32],&id,VoteType::For,None).is_ok()); }
    #[test] fn test_double() { let mut d = dao(); let id = d.create_proposal([1u8;32],"T".into(),"D".into(),ProposalType::TextProposal).unwrap_or_default(); let p = d.proposals.get_mut(&id).unwrap_or_default(); p.voting_ends_at = chrono::Utc::now().timestamp() + 86400; p.status = ProposalStatus::Active; d.vote([1u8;32],&id,VoteType::For,None).unwrap_or_default(); assert!(d.vote([1u8;32],&id,VoteType::Against,None).is_err()); }
    #[test] fn test_quorum() { let mut d = dao(); d.quorum_percent = 90; let id = d.create_proposal([1u8;32],"T".into(),"D".into(),ProposalType::TextProposal).unwrap_or_default(); let p = d.proposals.get_mut(&id).unwrap_or_default(); p.voting_ends_at = chrono::Utc::now().timestamp() + 86400; p.status = ProposalStatus::Active; d.vote([1u8;32],&id,VoteType::For,None).unwrap_or_default(); d.vote([2u8;32],&id,VoteType::For,None).unwrap_or_default(); d.vote([3u8;32],&id,VoteType::For,None).unwrap_or_default(); assert_eq!(d.proposals.get(&id).unwrap_or_default().status, ProposalStatus::Passed); }
    #[test] fn test_cancel() { let mut d = dao(); let id = d.create_proposal([1u8;32],"T".into(),"D".into(),ProposalType::TextProposal).unwrap_or_default(); assert!(d.cancel_proposal(&id,[1u8;32]).is_ok()); }
    #[test] fn test_execute() { let mut d = dao(); let id = d.create_proposal([1u8;32],"T".into(),"D".into(),ProposalType::TextProposal).unwrap_or_default(); let p = d.proposals.get_mut(&id).unwrap_or_default(); p.status = ProposalStatus::Passed; assert!(d.execute_proposal(&id).is_ok()); }
    #[test] fn test_stats() { let mut d = dao(); d.create_proposal([1u8;32],"T".into(),"D".into(),ProposalType::TextProposal).unwrap_or_default(); assert_eq!(d.get_stats().total_proposals,1); }
}


