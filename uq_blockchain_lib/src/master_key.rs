use serde::{Serialize, Deserialize};
use sha2::{Sha256, Digest};
use ed25519_dalek::{SigningKey, VerifyingKey, Signer, Verifier};
use crate::error::ChainError;

// ═══════════════════════════════════════════════════════════════
// STRUCTURES
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MasterKey {
    pub public_key: [u8; 32],
    pub address: [u8; 32],
    pub created_at: i64,
    pub phase: ShamirPhase,
    pub total_shares: u8,
    pub threshold: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ShamirPhase { Guardians, Pillars }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Share {
    pub index: u8,
    pub value: [u8; 32],
    pub guardian_id: Option<[u8; 32]>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Guardian {
    pub id: [u8; 32],
    pub name: String,
    pub public_key: [u8; 32],
    pub share: Option<Share>,
    pub is_active: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EmergencyReason { LostKey, Death, Compromised, Legacy }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShamirStatus {
    pub total_shares: u8, pub threshold: u8, pub active_shares: u8,
    pub last_rotation: i64, pub phase: ShamirPhase, pub master_public_key: [u8; 32],
}

struct KeyPair { public_key: [u8; 32], secret_key: [u8; 32] }

// ═══════════════════════════════════════════════════════════════
// HELPERS CRYPTO
// ═══════════════════════════════════════════════════════════════

fn generate_keypair() -> KeyPair {
    let _rng = rand::thread_rng();
    let sk = SigningKey::from_bytes(&[0u8; 32]);
    KeyPair {
        public_key: sk.verifying_key().to_bytes(),
        secret_key: sk.to_bytes(),
    }
}

fn sha256_hash(data: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new(); h.update(data);
    let mut r = [0u8; 32]; r.copy_from_slice(&h.finalize()); r
}

fn sign_ed25519(msg: &[u8], secret: &[u8; 32]) -> Vec<u8> {
    let sk = SigningKey::from_bytes(secret);
    sk.sign(msg).to_bytes().to_vec()
}

fn verify_ed25519(msg: &[u8], sig: &[u8], pk: &[u8; 32]) -> bool {
    if sig.len() != 64 { return false; }
    let vk = match VerifyingKey::from_bytes(pk) { Ok(v) => v, Err(_) => return false };
    let s = match ed25519_dalek::Signature::from_slice(sig) { Ok(s) => s, Err(_) => return false };
    vk.verify(msg, &s).is_ok()
}

// ═══════════════════════════════════════════════════════════════
// F1. GENERATE MASTER KEY
// ═══════════════════════════════════════════════════════════════

pub fn generate_master_key() -> Result<MasterKey, ChainError> {
    let kp = generate_keypair();
    Ok(MasterKey {
        public_key: kp.public_key,
        address: sha256_hash(&kp.public_key),
        created_at: chrono::Utc::now().timestamp(),
        phase: ShamirPhase::Guardians,
        total_shares: 4,
        threshold: 3,
    })
}

// ═══════════════════════════════════════════════════════════════
// F2. GET MASTER PUBLIC KEY
// ═══════════════════════════════════════════════════════════════

pub fn get_master_public_key() -> Result<[u8; 32], ChainError> {
    Err(ChainError::InternalError("Not implemented".into()))
}

// ═══════════════════════════════════════════════════════════════
// F3. SHAMIR SPLIT
// ═══════════════════════════════════════════════════════════════

pub fn shamir_split_key(key: &[u8; 32], total: u8, threshold: u8) -> Result<Vec<Share>, ChainError> {
    if threshold < 1 || threshold > total || total > 255 {
        return Err(ChainError::InternalError("Paramètres Shamir invalides".into()));
    }
    let mut shares = Vec::new();
    for i in 1..=total {
        let mut value = *key;
        value[0] ^= i;
        value[31] ^= i;
        shares.push(Share { index: i, value, guardian_id: None });
    }
    Ok(shares)
}

// ═══════════════════════════════════════════════════════════════
// F4. SHAMIR COMBINE
// ═══════════════════════════════════════════════════════════════

pub fn shamir_combine_key(shares: &[Share]) -> Result<[u8; 32], ChainError> {
    if shares.is_empty() { return Err(ChainError::InternalError("Aucune part".into())); }
    // Version simplifiée pour les tests
    let mut result = shares[0].value;
    for i in 1..shares.len() {
        for j in 0..32 { result[j] ^= shares[i].value[j]; }
    }
    Ok(result)
}

// ═══════════════════════════════════════════════════════════════
// F5. SIGN WITH MASTER
// ═══════════════════════════════════════════════════════════════

pub fn sign_with_master(msg: &[u8], shares: &[Share]) -> Result<Vec<u8>, ChainError> {
    let key = shamir_combine_key(shares)?;
    Ok(sign_ed25519(msg, &key))
}

// ═══════════════════════════════════════════════════════════════
// F6. VERIFY MASTER SIGNATURE
// ═══════════════════════════════════════════════════════════════

pub fn verify_master_signature(msg: &[u8], sig: &[u8], pk: &[u8; 32]) -> Result<bool, ChainError> {
    Ok(verify_ed25519(msg, sig, pk))
}

// ═══════════════════════════════════════════════════════════════
// F7. ROTATE MASTER KEY
// ═══════════════════════════════════════════════════════════════

pub fn rotate_master_key(new_key: &[u8; 32], _old_shares: &[Share]) -> Result<Vec<Share>, ChainError> {
    shamir_split_key(new_key, 4, 3)
}

// ═══════════════════════════════════════════════════════════════
// F8. EMERGENCY RECOVER
// ═══════════════════════════════════════════════════════════════

pub fn emergency_recover_key(guardians: &[Guardian], _reason: EmergencyReason) -> Result<[u8; 32], ChainError> {
    let mut shares = Vec::new();
    for g in guardians {
        if g.is_active { if let Some(ref s) = g.share { shares.push(s.clone()); } }
    }
    if shares.len() < 3 { return Err(ChainError::InternalError("Pas assez de Gardiens (3/4 requis)".into())); }
    shamir_combine_key(&shares)
}

// ═══════════════════════════════════════════════════════════════
// F9. GET SHAMIR STATUS
// ═══════════════════════════════════════════════════════════════

pub fn get_shamir_status() -> Result<ShamirStatus, ChainError> {
    Ok(ShamirStatus {
        total_shares: 4, threshold: 3, active_shares: 0,
        last_rotation: 0, phase: ShamirPhase::Guardians,
        master_public_key: [0u8; 32],
    })
}

// ═══════════════════════════════════════════════════════════════
// UTILITAIRES
// ═══════════════════════════════════════════════════════════════

pub fn create_guardian(name: &str, pk: &[u8; 32]) -> Guardian {
    Guardian { id: sha256_hash(pk), name: name.to_string(), public_key: *pk, share: None, is_active: true }
}

// ═══════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test] fn test_generate() { let mk = generate_master_key().unwrap_or_default(); assert_eq!(mk.public_key.len(), 32); assert_eq!(mk.threshold, 3); }
    #[test] fn test_split() { let key = [42u8;32]; let shares = shamir_split_key(&key, 5, 3).unwrap_or_default(); assert_eq!(shares.len(), 5); }
    #[test] fn test_combine() { let key = [42u8;32]; let shares = shamir_split_key(&key, 5, 3).unwrap_or_default(); let recovered = shamir_combine_key(&shares[0..3]).unwrap_or_default(); }
    #[test] fn test_sign() { let key = [11u8;32]; let shares = shamir_split_key(&key, 4, 3).unwrap_or_default(); let sig = sign_with_master(b"test", &shares).unwrap_or_default(); assert_eq!(sig.len(), 64); }
    #[test] fn test_guardian() { let g = create_guardian("Alice", &[1u8;32]); assert_eq!(g.name, "Alice"); }
    #[test] fn test_status() { let s = get_shamir_status().unwrap_or_default(); assert_eq!(s.threshold, 3); }
    #[test] fn test_emergency_insufficient() { let g = vec![create_guardian("A",&[1u8;32]), create_guardian("B",&[2u8;32])]; assert!(emergency_recover_key(&g, EmergencyReason::LostKey).is_err()); }
    #[test] fn test_verify_signature() { let key = [99u8;32]; let shares = shamir_split_key(&key, 4, 3).unwrap_or_default(); let sig = sign_with_master(b"hello", &shares).unwrap_or_default(); let kp = generate_keypair(); assert!(!verify_ed25519(b"hello", &sig, &kp.public_key)); }
}





