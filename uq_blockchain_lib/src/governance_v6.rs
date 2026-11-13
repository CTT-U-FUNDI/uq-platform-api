use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use crate::error::ChainError;

pub const QUORUM_PERCENT: u64 = 20;
pub const APPROVAL_THRESHOLD_PERCENT: u64 = 60;
pub const VETO_THRESHOLD_PERCENT: u64 = 33;
pub const VOTING_PERIOD_DAYS: u64 = 7;
pub const ANNOUNCEMENT_PERIOD_DAYS: u64 = 3;
pub const MAX_PROPOSALS_PER_MEMBER: u64 = 10;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ProposalType {
    AdjustFee, AdjustUBI, RotateOracleValidators, EmergencyAid,
    FreezeAccount, UnfreezeAccount, TreasuryDistribution,
    ChangeQuorum, ChangeThreshold, AddValidator, RemoveValidator,
    UpgradeProtocol, RecycleInactiveBubbles, AdjustMaxConcentration,
    ModifyCaseAllocation, Custom(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernanceProposal {
    pub id: [u8; 32], pub proposer: [u8; 32],
    pub proposal_type: ProposalType, pub title: String, pub description: String,
    pub created_at: u64, pub voting_starts_at: u64, pub voting_ends_at: u64,
    pub status: ProposalStatus,
    pub votes_for: u64, pub votes_against: u64, pub votes_veto: u64,
    pub total_voting_power: u64, pub voters: HashMap<[u8; 32], Vote>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ProposalStatus {
    Draft, Announced { announced_at: u64 }, Voting,
    Approved { approved_at: u64 }, Rejected { rejected_at: u64 },
    Vetoed { vetoed_at: u64 }, Executed { executed_at: u64 }, Cancelled { cancelled_at: u64 },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Vote { For, Against, Veto }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernanceRegistry {
    pub proposals: HashMap<[u8; 32], GovernanceProposal>,
    pub members: HashMap<[u8; 32], GovernanceMember>,
    pub total_proposals: u64, pub total_votes_cast: u64,
    pub veto_power_active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernanceMember {
    pub address: [u8; 32], pub voting_power: u64,
    pub joined_at: u64, pub proposals_created: u64, pub votes_cast: u64,
    pub is_active: bool,
}

#[derive(Debug, Clone)]
pub struct GovernanceStats {
    pub total_proposals: u64, pub active_proposals: usize,
    pub approved_proposals: usize, pub total_members: usize,
    pub total_voting_power: u64, pub participation_rate: f64,
}

impl GovernanceRegistry {
    pub fn new() -> Self {
        GovernanceRegistry {
            proposals: HashMap::new(), members: HashMap::new(),
            total_proposals: 0, total_votes_cast: 0, veto_power_active: true,
        }
    }

    pub fn register_member(&mut self, address: [u8; 32], voting_power: u64) -> Result<(), ChainError> {
        self.members.insert(address, GovernanceMember {
            address, voting_power, joined_at: utc_now(),
            proposals_created: 0, votes_cast: 0, is_active: true,
        });
        Ok(())
    }

    pub fn create_proposal(&mut self, proposer: &[u8; 32], proposal_type: ProposalType,
        title: &str, description: &str) -> Result<GovernanceProposal, ChainError> {
        let total_power: u64 = self.members.values().filter(|m| m.is_active).map(|m| m.voting_power).sum();
        let member = self.members.get_mut(proposer)
            .ok_or(ChainError::InternalError("Non membre".into()))?;
        if !member.is_active { return Err(ChainError::InternalError("Membre inactif".into())); }
        if member.proposals_created >= MAX_PROPOSALS_PER_MEMBER { return Err(ChainError::InternalError("Max 10 propositions".into())); }
        let now = utc_now();
        let proposal = GovernanceProposal {
            id: hash(proposer, now), proposer: *proposer, proposal_type,
            title: title.to_string(), description: description.to_string(),
            created_at: now,
            voting_starts_at: now + ANNOUNCEMENT_PERIOD_DAYS * 86400,
            voting_ends_at: now + (ANNOUNCEMENT_PERIOD_DAYS + VOTING_PERIOD_DAYS) * 86400,
            status: ProposalStatus::Announced { announced_at: now },
            votes_for: 0, votes_against: 0, votes_veto: 0,
            total_voting_power: total_power, voters: HashMap::new(),
        };
        member.proposals_created += 1;
        self.proposals.insert(proposal.id, proposal.clone());
        self.total_proposals += 1;
        Ok(proposal)
    }

    pub fn vote(&mut self, proposal_id: &[u8; 32], voter: &[u8; 32], vote: Vote) -> Result<(), ChainError> {
        let proposal = self.proposals.get_mut(proposal_id).ok_or(ChainError::InternalError("Introuvable".into()))?;
        if proposal.status != ProposalStatus::Voting {
            let now = utc_now();
            if now >= proposal.voting_starts_at && now <= proposal.voting_ends_at { proposal.status = ProposalStatus::Voting; }
            else { return Err(ChainError::InternalError("Période fermée".into())); }
        }
        let member = self.members.get(voter).ok_or(ChainError::InternalError("Non membre".into()))?;
        if !member.is_active { return Err(ChainError::InternalError("Inactif".into())); }
        if proposal.voters.contains_key(voter) { return Err(ChainError::InternalError("Déjà voté".into())); }
        let power = member.voting_power;
        match vote { Vote::For => proposal.votes_for += power, Vote::Against => proposal.votes_against += power, Vote::Veto => proposal.votes_veto += power }
        proposal.voters.insert(*voter, vote);
        if let Some(m) = self.members.get_mut(voter) { m.votes_cast += 1; }
        self.total_votes_cast += 1;
        Ok(())
    }

    pub fn finalize_proposal(&mut self, proposal_id: &[u8; 32]) -> Result<ProposalStatus, ChainError> {
        let proposal = self.proposals.get_mut(proposal_id).ok_or(ChainError::InternalError("Introuvable".into()))?;
        if proposal.status != ProposalStatus::Voting { return Err(ChainError::InternalError("Pas en vote".into())); }
        let now = utc_now();
        if now < proposal.voting_ends_at { return Err(ChainError::InternalError("Pas terminé".into())); }
        let total = proposal.total_voting_power.max(1);
        let participation = (proposal.votes_for + proposal.votes_against + proposal.votes_veto) * 100 / total;
        if participation < QUORUM_PERCENT { proposal.status = ProposalStatus::Rejected { rejected_at: now }; return Ok(proposal.status.clone()); }
        if self.veto_power_active && proposal.votes_veto * 100 / total >= VETO_THRESHOLD_PERCENT { proposal.status = ProposalStatus::Vetoed { vetoed_at: now }; return Ok(proposal.status.clone()); }
        let voted = proposal.votes_for + proposal.votes_against;
        if voted > 0 && proposal.votes_for * 100 / voted >= APPROVAL_THRESHOLD_PERCENT { proposal.status = ProposalStatus::Approved { approved_at: now }; }
        else { proposal.status = ProposalStatus::Rejected { rejected_at: now }; }
        Ok(proposal.status.clone())
    }

    pub fn execute_proposal(&mut self, proposal_id: &[u8; 32]) -> Result<(), ChainError> {
        let proposal = self.proposals.get_mut(proposal_id).ok_or(ChainError::InternalError("Introuvable".into()))?;
        if !matches!(proposal.status, ProposalStatus::Approved { .. }) { return Err(ChainError::InternalError("Non approuvée".into())); }
        proposal.status = ProposalStatus::Executed { executed_at: utc_now() };
        Ok(())
    }

    pub fn get_stats(&self) -> GovernanceStats {
        GovernanceStats {
            total_proposals: self.total_proposals,
            active_proposals: self.proposals.values().filter(|p| p.status == ProposalStatus::Voting).count(),
            approved_proposals: self.proposals.values().filter(|p| matches!(p.status, ProposalStatus::Approved { .. })).count(),
            total_members: self.members.len(),
            total_voting_power: self.members.values().filter(|m| m.is_active).map(|m| m.voting_power).sum(),
            participation_rate: if self.members.len() > 0 { self.total_votes_cast as f64 / self.members.len() as f64 } else { 0.0 },
        }
    }
}

fn hash(data: &[u8; 32], ts: u64) -> [u8; 32] {
    use sha2::{Sha256, Digest};
    let mut h = Sha256::new(); h.update(data); h.update(&ts.to_le_bytes());
    let r = h.finalize(); let mut id = [0u8; 32]; id.copy_from_slice(&r); id
}
fn utc_now() -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() }

#[cfg(test)]
mod tests {
    use super::*;
    fn addr(i: u8) -> [u8; 32] { let mut a = [0u8; 32]; a[0] = i; a }
    #[test] fn test_new() { let r = GovernanceRegistry::new(); assert_eq!(r.total_proposals, 0); }
    #[test] fn test_register() { let mut r = GovernanceRegistry::new(); r.register_member(addr(1), 1000).unwrap(); assert_eq!(r.members.len(), 1); }
    #[test] fn test_create_proposal() { let mut r = GovernanceRegistry::new(); r.register_member(addr(1), 1000).unwrap(); let p = r.create_proposal(&addr(1), ProposalType::AdjustFee, "T", "D").unwrap(); assert_eq!(p.proposer, addr(1)); }
    #[test] fn test_vote() { let mut r = GovernanceRegistry::new(); r.register_member(addr(1), 500).unwrap(); r.register_member(addr(2), 500).unwrap(); let p = r.create_proposal(&addr(1), ProposalType::AdjustFee, "T", "D").unwrap(); if let Some(prop) = r.proposals.get_mut(&p.id) { prop.status = ProposalStatus::Voting; } r.vote(&p.id, &addr(1), Vote::For).unwrap(); r.vote(&p.id, &addr(2), Vote::Against).unwrap(); let prop = r.proposals.get(&p.id).unwrap(); assert_eq!(prop.votes_for, 500); }
    #[test] fn test_double_vote() { let mut r = GovernanceRegistry::new(); r.register_member(addr(1), 500).unwrap(); let p = r.create_proposal(&addr(1), ProposalType::AdjustFee, "T", "D").unwrap(); if let Some(prop) = r.proposals.get_mut(&p.id) { prop.status = ProposalStatus::Voting; } r.vote(&p.id, &addr(1), Vote::For).unwrap(); assert!(r.vote(&p.id, &addr(1), Vote::Against).is_err()); }
    #[test] fn test_stats() { let mut r = GovernanceRegistry::new(); r.register_member(addr(1), 500).unwrap(); let s = r.get_stats(); assert_eq!(s.total_members, 1); }
    #[test] fn test_max_proposals() { let mut r = GovernanceRegistry::new(); r.register_member(addr(1), 500).unwrap(); for _ in 0..10 { r.create_proposal(&addr(1), ProposalType::AdjustFee, "T", "D").unwrap(); } assert!(r.create_proposal(&addr(1), ProposalType::AdjustFee, "T", "D").is_err()); }
    #[test] fn test_16_types() { assert_eq!(16, [ProposalType::AdjustFee, ProposalType::AdjustUBI, ProposalType::RotateOracleValidators, ProposalType::EmergencyAid, ProposalType::FreezeAccount, ProposalType::UnfreezeAccount, ProposalType::TreasuryDistribution, ProposalType::ChangeQuorum, ProposalType::ChangeThreshold, ProposalType::AddValidator, ProposalType::RemoveValidator, ProposalType::UpgradeProtocol, ProposalType::RecycleInactiveBubbles, ProposalType::AdjustMaxConcentration, ProposalType::ModifyCaseAllocation, ProposalType::Custom("T".into())].len()); }
}
