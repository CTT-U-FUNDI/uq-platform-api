use serde::{Serialize, Deserialize};
use sha2::{Sha256, Digest};
use crate::error::ChainError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyShard {
    pub index: u8, pub data: Vec<u8>, pub guardian: [u8; 32], pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardianSet {
    pub owner: [u8; 32], pub guardians: Vec<[u8; 32]>, pub shards: Vec<KeyShard>,
    pub required_shards: u8, pub total_shards: u8, pub created_at: i64, pub status: GuardianStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum GuardianStatus { Active, RecoveryInitiated, Recovered, Expired }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryPlan {
    pub owner: [u8; 32], pub guardian_set_id: [u8; 32],
    pub inactivity_period: i64, pub heir: [u8; 32],
    pub last_activity: i64, pub status: RecoveryStatus, pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RecoveryStatus { Active, WarningSent, RecoveryReady, Completed, Cancelled }

impl GuardianSet {
    pub fn new(owner: [u8; 32], guardians: Vec<[u8; 32]>) -> Result<Self, ChainError> {
        if guardians.len() < 3 { return Err(ChainError::InternalError("Min 3 gardiens".into())); }
        let total = guardians.len() as u8;
        let required = ((total as f64 * 0.6).ceil() as u8).max(3);
        Ok(GuardianSet { owner, guardians, shards: vec![], required_shards: required, total_shards: total, created_at: chrono::Utc::now().timestamp(), status: GuardianStatus::Active })
    }
    
    pub fn split_key(&mut self, private_key: &[u8; 32]) -> Result<Vec<KeyShard>, ChainError> {
        let now = chrono::Utc::now().timestamp();
        let mut shards = Vec::new();
        for (i, guardian) in self.guardians.iter().enumerate() {
            let gs = Self::guardian_secret(guardian);
            let mut data = [0u8; 32];
            for j in 0..32 { data[j] = private_key[j] ^ gs[j]; }
            shards.push(KeyShard { index: (i+1) as u8, data: data.to_vec(), guardian: *guardian, created_at: now });
        }
        self.shards = shards.clone();
        Ok(shards)
    }
    
    pub fn reconstruct_key(&self, shards: &[KeyShard]) -> Result<[u8; 32], ChainError> {
        if shards.len() < self.required_shards as usize { return Err(ChainError::InternalError(format!("Fragments insuffisants: {}/{}", shards.len(), self.required_shards))); }
        for s in shards { if !self.guardians.contains(&s.guardian) { return Err(ChainError::InternalError("Fragment non autorisé".into())); } }
        let ref_s = &shards[0];
        let gs = Self::guardian_secret(&ref_s.guardian);
        let mut key = [0u8; 32];
        for j in 0..32 { key[j] = ref_s.data[j] ^ gs[j]; }
        Ok(key)
    }
    
    fn guardian_secret(g: &[u8; 32]) -> [u8; 32] {
        let mut h = Sha256::new(); h.update(g); h.update(b"UQ_GUARDIAN_V1");
        let r = h.finalize(); let mut s = [0u8; 32]; s.copy_from_slice(&r); s
    }
    
    pub fn can_recover(&self, count: usize) -> bool { count >= self.required_shards as usize }
    pub fn get_id(&self) -> [u8; 32] {
        let mut h = Sha256::new(); h.update(&self.owner); h.update(&self.created_at.to_le_bytes());
        let r = h.finalize(); let mut id = [0u8; 32]; id.copy_from_slice(&r); id
    }
}

impl RecoveryPlan {
    pub fn new(owner: [u8; 32], guardian_set_id: [u8; 32], heir: [u8; 32], inactivity_days: u64) -> Self {
        let now = chrono::Utc::now().timestamp();
        RecoveryPlan { owner, guardian_set_id, inactivity_period: (inactivity_days * 86400) as i64, heir, last_activity: now, status: RecoveryStatus::Active, created_at: now }
    }
    
    pub fn update_activity(&mut self) { self.last_activity = chrono::Utc::now().timestamp(); if self.status == RecoveryStatus::WarningSent { self.status = RecoveryStatus::Active; } }
    pub fn is_inactive(&self) -> bool { (chrono::Utc::now().timestamp() - self.last_activity) >= self.inactivity_period }
    pub fn should_warn(&self) -> bool { (chrono::Utc::now().timestamp() - self.last_activity) >= (self.inactivity_period - 5*86400) && self.status == RecoveryStatus::Active }
    pub fn initiate_recovery(&mut self) -> Result<(), ChainError> { if !self.is_inactive() { return Err(ChainError::InternalError("Compte actif".into())); } self.status = RecoveryStatus::RecoveryReady; Ok(()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_create() {
        let s = GuardianSet::new([1u8;32], vec![[2u8;32],[3u8;32],[4u8;32],[5u8;32],[6u8;32]]).unwrap_or_default();
        assert_eq!(s.total_shards, 5); assert_eq!(s.required_shards, 3);
        assert!(s.can_recover(3)); assert!(!s.can_recover(2));
    }
    
    #[test]
    fn test_split_reconstruct() {
        let mut s = GuardianSet::new([1u8;32], vec![[2u8;32],[3u8;32],[4u8;32]]).unwrap_or_default();
        let key = [42u8;32];
        let shards = s.split_key(&key).unwrap_or_default();
        assert_eq!(s.reconstruct_key(&shards[0..2]).is_err(), true);
        assert_eq!(s.reconstruct_key(&shards).unwrap_or_default(), key);
    }
    
    #[test]
    fn test_plan() {
        let plan = RecoveryPlan::new([1u8;32], [0u8;32], [9u8;32], 30);
        assert_eq!(plan.status, RecoveryStatus::Active);
        assert!(!plan.is_inactive());
    }
}
