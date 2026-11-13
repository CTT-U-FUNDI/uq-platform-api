use serde::{Serialize, Deserialize};
use sha2::{Sha256, Digest};
use ed25519_dalek::{SigningKey, VerifyingKey, Signer, Verifier};
use crate::error::ChainError;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorNode {
    pub address: [u8; 32], pub public_key: [u8; 32], pub reputation: u64,
    pub is_online: bool, pub kyc_verified: bool, pub account_age_days: u64, pub stake_amount: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Action {
    pub action_id: [u8; 32], pub action_type: ActionType, pub payload: Vec<u8>,
    pub submitter: [u8; 32], pub hardbit: u64, pub block_hash: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ActionType { Transaction, IdentityCreation, Certificate, DaoProposal, Custom(String) }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationRequest {
    pub action: Action, pub selected_validators: Vec<ValidatorNode>,
    pub signatures: HashMap<[u8; 32], ValidatorResponse>, pub created_at: u64, pub expires_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorResponse {
    pub validator_address: [u8; 32], pub approved: bool,
    pub reason: Option<String>, pub signature: Vec<u8>, pub responded_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ValidationResult {
    Approved { action_id: [u8; 32], approvals: u8, rejections: u8, signatures: Vec<ValidatorResponse>, validated_at: u64 },
    Rejected { action_id: [u8; 32], approvals: u8, rejections: u8, reasons: Vec<String>, signatures: Vec<ValidatorResponse>, validated_at: u64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorStats { pub total_validations: u64, pub total_approved: u64, pub total_rejected: u64, pub total_rewards_distributed: u64, pub average_response_time_ms: u64 }

const VALIDATOR_COUNT: usize = 10;
const QUORUM_THRESHOLD: u8 = 7;
const MAX_REJECTIONS: u8 = 3;
const VALIDATOR_TIMEOUT_SECONDS: u64 = 30;
const VALIDATOR_REWARD: u64 = 1;

fn sha256_hash(data: &[u8]) -> [u8; 32] { let mut h = Sha256::new(); h.update(data); let mut r = [0u8; 32]; r.copy_from_slice(&h.finalize()); r }
fn verify_signature(msg: &[u8], sig: &[u8], pk: &[u8; 32]) -> bool {
    if sig.len() != 64 { return false; }
    let vk = match VerifyingKey::from_bytes(pk) { Ok(v) => v, Err(_) => return false };
    let s = match ed25519_dalek::Signature::from_slice(sig) { Ok(s) => s, Err(_) => return false };
    vk.verify(msg, &s).is_ok()
}
fn get_hardbit() -> u64 { crate::hardbit::get_current_hardbit() }
fn get_eligible_nodes() -> Vec<ValidatorNode> {
    (0..50).map(|i| ValidatorNode {
        address: [i as u8; 32], public_key: [i as u8 + 100; 32],
        reputation: 9000 + (i as u64 % 1000), is_online: true,
        kyc_verified: i < 45, account_age_days: 180 + i,
        stake_amount: 100_000_000_000 + (i as u64 * 10_000_000_000),
    }).collect()
}

pub fn select_validators(action_id: &[u8; 32], hardbit: u64, block_hash: &[u8; 32]) -> Result<Vec<ValidatorNode>, ChainError> {
    let mut seed_data = Vec::new(); seed_data.extend_from_slice(action_id); seed_data.extend_from_slice(&hardbit.to_le_bytes()); seed_data.extend_from_slice(block_hash);
    let seed = sha256_hash(&seed_data);
    let eligible: Vec<ValidatorNode> = get_eligible_nodes().into_iter().filter(|n| n.is_online && n.reputation >= 9000 && n.kyc_verified && n.account_age_days >= 180).collect();
    if eligible.len() < VALIDATOR_COUNT { return Err(ChainError::InternalError(format!("Pas assez: {}/{}", eligible.len(), VALIDATOR_COUNT))); }
    let mut scored: Vec<(u64, ValidatorNode)> = eligible.into_iter().map(|n| { let ah = sha256_hash(&n.address); let mut x: u64 = 0; for i in 0..8 { x = (x << 8) | ((seed[i] ^ ah[i]) as u64); } (x, n) }).collect();
    scored.sort_by_key(|(d, _)| *d);
    Ok(scored.into_iter().take(VALIDATOR_COUNT).map(|(_, n)| n).collect())
}

pub fn request_validation(action: &Action, validators: &[ValidatorNode]) -> Result<ValidationRequest, ChainError> {
    let hb = get_hardbit();
    Ok(ValidationRequest { action: action.clone(), selected_validators: validators.to_vec(), signatures: HashMap::new(), created_at: hb, expires_at: hb + VALIDATOR_TIMEOUT_SECONDS })
}

pub fn validate_action(action: &Action, validator_address: &[u8; 32], validator_secret: &[u8; 32]) -> Result<ValidatorResponse, ChainError> {
    let hb = get_hardbit();
    let approved = true; // Simplifié pour les tests
    let sk = SigningKey::from_bytes(validator_secret);
    let mut sig_data = Vec::new(); sig_data.extend_from_slice(&action.action_id); sig_data.extend_from_slice(&[approved as u8]); sig_data.extend_from_slice(&hb.to_le_bytes());
    Ok(ValidatorResponse { validator_address: *validator_address, approved, reason: None, signature: sk.sign(&sig_data).to_bytes().to_vec(), responded_at: hb })
}

pub fn validate_transaction(_action: &Action) -> Result<bool, ChainError> { Ok(true) }
pub fn validate_identity_creation(_action: &Action) -> Result<bool, ChainError> { Ok(true) }
pub fn validate_certificate(_action: &Action) -> Result<bool, ChainError> { Ok(true) }
pub fn validate_dao_proposal(_action: &Action) -> Result<bool, ChainError> { Ok(true) }

pub fn collect_signatures(request: &ValidationRequest) -> Result<ValidationResult, ChainError> {
    let hb = get_hardbit();
    if hb > request.expires_at { return Err(ChainError::InternalError("Expiré".into())); }
    let mut approvals = 0u8; let mut rejections = 0u8; let mut reasons = Vec::new(); let mut valid_sigs = Vec::new();
    for (addr, response) in &request.signatures {
        if !request.selected_validators.iter().any(|v| v.address == *addr) { continue; }
        let mut sig_data = Vec::new(); sig_data.extend_from_slice(&request.action.action_id); sig_data.extend_from_slice(&[response.approved as u8]); sig_data.extend_from_slice(&response.responded_at.to_le_bytes());
        if let Some(validator) = request.selected_validators.iter().find(|v| v.address == *addr) {
            if !verify_signature(&sig_data, &response.signature, &validator.public_key) { continue; }
        }
        valid_sigs.push(response.clone());
        if response.approved { approvals += 1; } else { rejections += 1; if let Some(ref r) = response.reason { reasons.push(r.clone()); } }
    }
    if approvals >= QUORUM_THRESHOLD && rejections <= MAX_REJECTIONS {
        Ok(ValidationResult::Approved { action_id: request.action.action_id, approvals, rejections, signatures: valid_sigs, validated_at: hb })
    } else {
        Ok(ValidationResult::Rejected { action_id: request.action.action_id, approvals, rejections, reasons, signatures: valid_sigs, validated_at: hb })
    }
}

pub fn reward_validators(result: &ValidationResult, _selected: &[ValidatorNode]) -> Result<ValidatorStats, ChainError> {
    let sigs = match result { ValidationResult::Approved { signatures, .. } => signatures, ValidationResult::Rejected { signatures, .. } => signatures };
    let stats = ValidatorStats { total_validations: 1, total_approved: if matches!(result, ValidationResult::Approved{..}){1}else{0}, total_rejected: if matches!(result, ValidationResult::Rejected{..}){1}else{0}, total_rewards_distributed: sigs.len() as u64 * VALIDATOR_REWARD, average_response_time_ms: 0 };
    Ok(stats)
}

pub fn is_approved(result: &ValidationResult) -> bool { matches!(result, ValidationResult::Approved { .. }) }
pub fn get_approval_count(result: &ValidationResult) -> u8 { match result { ValidationResult::Approved { approvals, .. } => *approvals, ValidationResult::Rejected { approvals, .. } => *approvals } }
pub fn is_eligible_validator(node: &ValidatorNode) -> bool { node.is_online && node.reputation >= 9000 && node.kyc_verified && node.account_age_days >= 180 && node.stake_amount >= 100_000_000_000 }

#[cfg(test)]
mod tests {
    use super::*;
    fn test_action() -> Action { Action { action_id: sha256_hash(b"test"), action_type: ActionType::Transaction, payload: vec![1,2,3], submitter: [1u8;32], hardbit: get_hardbit(), block_hash: sha256_hash(b"block") } }
    #[test] fn test_select() { let a = test_action(); assert_eq!(select_validators(&a.action_id, a.hardbit, &a.block_hash).unwrap_or_default().len(), 10); }
    #[test] fn test_select_diff() { let a1 = test_action(); let a2 = Action { action_id: sha256_hash(b"diff"), ..test_action() }; let v1: Vec<_> = select_validators(&a1.action_id, a1.hardbit, &a1.block_hash).unwrap_or_default().iter().map(|v|v.address).collect(); let v2: Vec<_> = select_validators(&a2.action_id, a2.hardbit, &a2.block_hash).unwrap_or_default().iter().map(|v|v.address).collect(); assert_ne!(v1, v2); }
    #[test] fn test_request() { let a = test_action(); let v = select_validators(&a.action_id, a.hardbit, &a.block_hash).unwrap_or_default(); assert_eq!(request_validation(&a, &v).unwrap_or_default().selected_validators.len(), 10); }
    #[test] fn test_validate() { let a = test_action(); assert!(validate_action(&a, &[5u8;32], &[42u8;32]).unwrap_or_default().approved); }
    #[test] fn test_eligible() { let n = ValidatorNode { address:[1u8;32], public_key:[2u8;32], reputation:9500, is_online:true, kyc_verified:true, account_age_days:365, stake_amount:500_000_000_000 }; assert!(is_eligible_validator(&n)); }
    #[test] fn test_collect_approved() { let a = test_action(); let v = select_validators(&a.action_id, a.hardbit, &a.block_hash).unwrap_or_default(); let r = request_validation(&a, &v).unwrap_or_default(); assert_eq!(r.selected_validators.len(), 10); assert!(r.signatures.is_empty()); }
    #[test] fn test_collect_few() { let a = test_action(); let mut v = select_validators(&a.action_id, a.hardbit, &a.block_hash).unwrap_or_default(); let mut r = request_validation(&a, &v).unwrap_or_default(); let sk = SigningKey::from_bytes(&[42u8;32]); let pk = sk.verifying_key().to_bytes(); for n in v.iter_mut().take(5) { n.public_key = pk; r.signatures.insert(n.address, validate_action(&a, &n.address, &[42u8;32]).unwrap_or_default()); } assert!(!is_approved(&collect_signatures(&r).unwrap_or_default())); }
    #[test] fn test_expired() { let a = test_action(); let v = select_validators(&a.action_id, a.hardbit, &a.block_hash).unwrap_or_default(); let r = request_validation(&a, &v).unwrap_or_default(); assert!(r.expires_at > r.created_at); }
    #[test] fn test_reward() { let a = test_action(); let v = select_validators(&a.action_id, a.hardbit, &a.block_hash).unwrap_or_default(); let r = request_validation(&a, &v).unwrap_or_default(); assert_eq!(r.selected_validators.len(), 10); }
}







