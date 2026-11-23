use std::sync::Mutex;
use uq_blockchain::governance_v6::{GovernanceRegistry, ProposalType, Vote};

pub struct GovState {
    pub registry: Mutex<GovernanceRegistry>,
}

impl GovState {
    pub fn new() -> Self {
        Self {
            registry: Mutex::new(GovernanceRegistry::new()),
        }
    }

    pub fn register_member(&self, address_hex: &str, voting_power: u64) -> Result<(), String> {
        let mut gov = self.registry.lock().unwrap();
        let addr_bytes = hex::decode(address_hex).map_err(|_| "Adresse invalide")?;
        let mut addr = [0u8; 32];
        addr.copy_from_slice(&addr_bytes[..32]);
        gov.register_member(addr, voting_power)
            .map_err(|e| format!("{:?}", e))
    }

    pub fn create_proposal(
        &self,
        proposer_hex: &str,
        proposal_type: u8,
        title: &str,
        description: &str,
    ) -> Result<String, String> {
        let mut gov = self.registry.lock().unwrap();
        let prop_bytes = hex::decode(proposer_hex).map_err(|_| "Adresse invalide")?;
        let mut proposer = [0u8; 32];
        proposer.copy_from_slice(&prop_bytes[..32]);

        let ptype = match proposal_type {
            1 => ProposalType::AdjustFee,
            2 => ProposalType::AdjustUBI,
            3 => ProposalType::RotateOracleValidators,
            4 => ProposalType::EmergencyAid,
            5 => ProposalType::FreezeAccount,
            6 => ProposalType::UnfreezeAccount,
            7 => ProposalType::TreasuryDistribution,
            8 => ProposalType::ChangeQuorum,
            9 => ProposalType::ChangeThreshold,
            10 => ProposalType::AddValidator,
            11 => ProposalType::RemoveValidator,
            12 => ProposalType::UpgradeProtocol,
            13 => ProposalType::RecycleInactiveBubbles,
            14 => ProposalType::AdjustMaxConcentration,
            15 => ProposalType::ModifyCaseAllocation,
            _ => ProposalType::Custom("custom".to_string()),
        };

        let proposal = gov
            .create_proposal(&proposer, ptype, title, description)
            .map_err(|e| format!("{:?}", e))?;

        Ok(hex::encode(proposal.id))
    }

    pub fn vote(
        &self,
        proposal_id_hex: &str,
        voter_hex: &str,
        vote_type: u8,
    ) -> Result<(), String> {
        let mut gov = self.registry.lock().unwrap();

        let pid_bytes = hex::decode(proposal_id_hex).map_err(|_| "ID proposition invalide")?;
        let mut pid = [0u8; 32];
        pid.copy_from_slice(&pid_bytes[..32]);

        let voter_bytes = hex::decode(voter_hex).map_err(|_| "Adresse votant invalide")?;
        let mut voter = [0u8; 32];
        voter.copy_from_slice(&voter_bytes[..32]);

        let vote = match vote_type {
            1 => Vote::For,
            2 => Vote::Against,
            _ => Vote::Veto,
        };

        gov.vote(&pid, &voter, vote).map_err(|e| format!("{:?}", e))
    }

    pub fn get_stats(&self) -> serde_json::Value {
        let gov = self.registry.lock().unwrap();
        let stats = gov.get_stats();
        serde_json::json!({
            "total_proposals": stats.total_proposals,
            "active_proposals": stats.active_proposals,
            "approved_proposals": stats.approved_proposals,
            "total_members": stats.total_members,
            "total_voting_power": stats.total_voting_power,
            "participation_rate": stats.participation_rate
        })
    }
}
