use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use crate::bubble::BubbleRegistry;
use crate::error::ChainError;

pub const MIN_GUARDIANS_FOR_LOSS: u8 = 3;
pub const MAX_AID_PER_RECIPIENT: u64 = 100;
pub const MIN_VALIDATORS_FOR_AID: u8 = 5;
pub const INACTIVITY_THRESHOLD: u64 = 100 * 365 * 24 * 3600;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum LossStatus { Pending, Validated { validated_at: u64 }, Rejected { reason: String }, Recycled { recycled_at: u64, amount_recycled: u64 } }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyLossDeclaration {
    pub id: [u8; 32], pub lost_address: [u8; 32], pub guardians: Vec<[u8; 32]>,
    pub guardian_signatures: Vec<Vec<u8>>, pub declared_at: u64, pub status: LossStatus,
    pub affected_bubbles: u64, pub total_value_micro: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebtEntry {
    pub original_owner: [u8; 32], pub amount_micro: u64,
    pub declared_at: u64, pub reclaim_deadline: u64, pub status: DebtStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DebtStatus { Pending, Reclaimed { reclaimed_at: u64 }, Expired }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AidRequest {
    pub id: [u8; 32], pub recipients: Vec<[u8; 32]>,
    pub amount_per_recipient: u64, pub reason: String,
    pub requested_at: u64, pub status: AidStatus, pub approvals: Vec<[u8; 32]>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AidStatus { Pending, Approved { approved_at: u64 }, Distributed { distributed_at: u64 }, Rejected { reason: String } }

pub struct ShieldEngine {
    pub loss_declarations: HashMap<[u8; 32], KeyLossDeclaration>,
    pub debt_registry: HashMap<[u8; 32], DebtEntry>,
    pub aid_requests: HashMap<[u8; 32], AidRequest>,
    pub total_recycled_micro: u64, pub total_aid_distributed_micro: u64, pub total_beneficiaries: u64,
}

impl ShieldEngine {
    pub fn new() -> Self {
        ShieldEngine {
            loss_declarations: HashMap::new(), debt_registry: HashMap::new(),
            aid_requests: HashMap::new(), total_recycled_micro: 0,
            total_aid_distributed_micro: 0, total_beneficiaries: 0,
        }
    }

    pub fn declare_key_lost(&mut self, _registry: &mut BubbleRegistry, lost_address: &[u8; 32],
        guardians: &[[u8; 32]], guardian_signatures: &[Vec<u8>]) -> Result<KeyLossDeclaration, ChainError> {
        if guardians.len() < MIN_GUARDIANS_FOR_LOSS as usize {
            return Err(ChainError::InternalError("3 guardians minimum".into()));
        }
        let now = Self::now();
        let declaration = KeyLossDeclaration {
            id: Self::hash(lost_address, now), lost_address: *lost_address,
            guardians: guardians.to_vec(), guardian_signatures: guardian_signatures.to_vec(),
            declared_at: now, status: LossStatus::Validated { validated_at: now },
            affected_bubbles: 0, total_value_micro: 0,
        };
        self.loss_declarations.insert(declaration.id, declaration.clone());
        Ok(declaration)
    }

    pub fn request_aid(&mut self, recipients: &[[u8; 32]], amount: u64, reason: &str) -> Result<AidRequest, ChainError> {
        if amount > MAX_AID_PER_RECIPIENT { return Err(ChainError::InternalError("Max 100 BORB".into())); }
        let now = Self::now();
        let req = AidRequest {
            id: Self::hash(&recipients[0], now), recipients: recipients.to_vec(),
            amount_per_recipient: amount, reason: reason.into(),
            requested_at: now, status: AidStatus::Pending, approvals: vec![],
        };
        self.aid_requests.insert(req.id, req.clone());
        Ok(req)
    }

    pub fn approve_aid(&mut self, request_id: &[u8; 32], validator: &[u8; 32]) -> Result<bool, ChainError> {
        let req = self.aid_requests.get_mut(request_id).ok_or(ChainError::InternalError("Introuvable".into()))?;
        if !req.approvals.iter().any(|a| a == validator) { req.approvals.push(*validator); }
        if req.approvals.len() >= MIN_VALIDATORS_FOR_AID as usize { req.status = AidStatus::Approved { approved_at: Self::now() }; return Ok(true); }
        Ok(false)
    }

    fn now() -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() }
    fn hash(data: &[u8; 32], ts: u64) -> [u8; 32] {
        use sha2::{Sha256, Digest};
        let mut h = Sha256::new(); h.update(data); h.update(&ts.to_le_bytes());
        let r = h.finalize(); let mut id = [0u8; 32]; id.copy_from_slice(&r); id
    }
}

#[cfg(test)]
    
mod tests {
    fn test_key() -> UqBlackKey { let mut k = UqBlackKey::generate(); k.activate(); k }
    use uq_black_key_standalone::UqBlackKey;

    use super::*;
    #[test] fn test_new() { let e = ShieldEngine::new(); assert_eq!(e.loss_declarations.len(), 0); }
    #[test] fn test_declare_loss() {
        let mut reg = BubbleRegistry::new(); let mut e = ShieldEngine::new();
        let owner = [1u8; 32]; let guardians = vec![[2u8;32],[3u8;32],[4u8;32]];
        let sigs = vec![vec![1u8;64],vec![2u8;64],vec![3u8;64]];
        let d = e.declare_key_lost(&mut reg, &owner, &guardians, &sigs).unwrap();
        assert_eq!(d.lost_address, owner);
    }
    #[test] fn test_declare_loss_fails() {
        let mut reg = BubbleRegistry::new(); let mut e = ShieldEngine::new();
        let owner = [1u8; 32]; let guardians = vec![[2u8;32]];
        let sigs = vec![vec![1u8;64]];
        assert!(e.declare_key_lost(&mut reg, &owner, &guardians, &sigs).is_err());
    }
    #[test] fn test_request_aid() {
        let e = ShieldEngine::new(); let mut e = e;
        let r = e.request_aid(&[[5u8;32]], 50, "Urgence").unwrap();
        assert_eq!(r.amount_per_recipient, 50);
    }
    #[test] fn test_aid_too_high() {
        let e = ShieldEngine::new(); let mut e = e;
        assert!(e.request_aid(&[[5u8;32]], 101, "Test").is_err());
    }
}

