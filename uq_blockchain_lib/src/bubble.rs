use serde::{Deserialize, Serialize};
use sha2::{Sha256, Digest};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Bubble {
    pub id: [u8; 32],
    pub owner: [u8; 32],
    pub creator: [u8; 32],
    pub value_micro: u64,
    pub created_at: u64,
    pub updated_at: u64,
    pub sequence: u64,
    pub signature: Vec<u8>,
    pub state: BubbleState,
    pub metadata: Vec<u8>,
    pub prev_bubble_hash: Option<[u8; 32]>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum BubbleState {
    Active,
    Frozen { reason: String, frozen_at: u64 },
    Lent { loan_id: [u8; 32], lent_at: u64 },
    PendingRecovery { since: u64 },
    Burned { burned_at: u64 },
    InTransit { tx_id: [u8; 32] },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BubbleRegistry {
    pub bubbles: HashMap<[u8; 32], Bubble>,
    pub by_owner: HashMap<[u8; 32], Vec<[u8; 32]>>,
    pub frozen: Vec<[u8; 32]>,
    pub lent: Vec<[u8; 32]>,
    pub total_active: u64,
    pub TOTAL_BORB: u64,
    pub total_value_micro: u64,
}

impl BubbleRegistry {
    pub fn new() -> Self {
        BubbleRegistry {
            bubbles: HashMap::new(), by_owner: HashMap::new(),
            frozen: Vec::new(), lent: Vec::new(),
            total_active: 0, TOTAL_BORB: 0, total_value_micro: 0,
        }
    }

    pub fn create_bubble(&mut self, owner: [u8; 32], creator: [u8; 32],
        value_micro: u64, metadata: Vec<u8>, sign_fn: impl Fn(&[u8]) -> Vec<u8>) -> Result<Bubble, String> {
        let now = Self::now();
        let mut bubble = Bubble {
            id: [0u8; 32], owner, creator, value_micro, created_at: now, updated_at: now,
            sequence: 0, signature: Vec::new(), state: BubbleState::Active,
            metadata, prev_bubble_hash: None,
        };
        bubble.id = bubble.compute_id();
        let payload = bubble.serialize_for_signing();
        bubble.signature = sign_fn(&payload);
        self.bubbles.insert(bubble.id, bubble.clone());
        self.by_owner.entry(owner).or_default().push(bubble.id);
        self.TOTAL_BORB += 1; self.total_active += 1;
        self.total_value_micro += value_micro;
        Ok(bubble)
    }

    pub fn transfer_bubble(&mut self, bubble_id: &[u8; 32], from: &[u8; 32],
        to: [u8; 32], sign_fn: impl Fn(&[u8]) -> Vec<u8>) -> Result<(), String> {
        let bubble = self.bubbles.get_mut(bubble_id).ok_or("Bulle introuvable")?;
        if bubble.owner != *from { return Err("Pas le propriétaire".into()); }
        if bubble.state != BubbleState::Active { return Err("Bulle non transférable".into()); }
        let prev = bubble.owner;
        bubble.owner = to; bubble.sequence += 1; bubble.updated_at = Self::now();
        bubble.prev_bubble_hash = Some(bubble.id);
        bubble.id = bubble.compute_id();
        let payload = bubble.serialize_for_signing();
        bubble.signature = sign_fn(&payload);
        if let Some(list) = self.by_owner.get_mut(&prev) { list.retain(|id| id != bubble_id); }
        self.by_owner.entry(to).or_default().push(bubble.id);
        Ok(())
    }

    pub fn freeze_bubble(&mut self, bubble_id: &[u8; 32], reason: &str) -> Result<(), String> {
        let bubble = self.bubbles.get_mut(bubble_id).ok_or("Bulle introuvable")?;
        bubble.state = BubbleState::Frozen { reason: reason.into(), frozen_at: Self::now() };
        self.frozen.push(*bubble_id);
        if self.total_active > 0 { self.total_active -= 1; }
        Ok(())
    }

    pub fn unfreeze_bubble(&mut self, bubble_id: &[u8; 32]) -> Result<(), String> {
        let bubble = self.bubbles.get_mut(bubble_id).ok_or("Bulle introuvable")?;
        bubble.state = BubbleState::Active; bubble.updated_at = Self::now();
        self.frozen.retain(|id| id != bubble_id); self.total_active += 1;
        Ok(())
    }

    pub fn lend_bubble(&mut self, bubble_id: &[u8; 32], loan_id: [u8; 32]) -> Result<(), String> {
        let bubble = self.bubbles.get_mut(bubble_id).ok_or("Bulle introuvable")?;
        bubble.state = BubbleState::Lent { loan_id, lent_at: Self::now() };
        self.lent.push(*bubble_id);
        if self.total_active > 0 { self.total_active -= 1; }
        Ok(())
    }


    pub fn get_balance(&self, owner: &[u8; 32]) -> u64 {
        self.by_owner.get(owner).map(|ids| {
            ids.iter().filter_map(|id| self.bubbles.get(id))
                .filter(|b| b.state == BubbleState::Active)
                .map(|b| b.value_micro).sum()
        }).unwrap_or(0)
    }

    fn now() -> u64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()
    }
}

impl Bubble {
    pub fn compute_id(&self) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(&self.owner); h.update(&self.creator);
        h.update(&self.value_micro.to_le_bytes()); h.update(&self.created_at.to_le_bytes());
        h.update(&self.sequence.to_le_bytes()); h.update(&self.metadata);
        if let Some(ref prev) = self.prev_bubble_hash { h.update(prev); }
        let r = h.finalize(); let mut id = [0u8; 32]; id.copy_from_slice(&r); id
    }
    pub fn serialize_for_signing(&self) -> Vec<u8> {
        let mut d = Vec::new();
        d.extend_from_slice(&self.owner); d.extend_from_slice(&self.value_micro.to_le_bytes());
        d.extend_from_slice(&self.created_at.to_le_bytes()); d.extend_from_slice(&self.sequence.to_le_bytes());
        d
    }
    pub fn is_active(&self) -> bool { matches!(self.state, BubbleState::Active) }
    pub fn is_frozen(&self) -> bool { matches!(self.state, BubbleState::Frozen { .. }) }
}

#[cfg(test)]
    
    
mod tests {
    use uq_black_key_standalone::UqBlackKey;
    fn test_key() -> UqBlackKey { let mut k = UqBlackKey::generate(); k.activate(); k }
    fn sign(data: &[u8]) -> Vec<u8> { let key = test_key(); key.sign(data) }
    
    

    use super::*;
    
    fn owner() -> [u8; 32] { let mut a = [0u8; 32]; a[0]=1; a }
    fn owner2() -> [u8; 32] { let mut a = [0u8; 32]; a[0]=2; a }
    #[test] fn test_create() { let mut r = BubbleRegistry::new(); let b = r.create_bubble(owner(), owner(), 1_000_000, vec![0u8;64], sign).unwrap(); assert_eq!(b.value_micro, 1_000_000); }
    #[test] fn test_transfer() { let mut r = BubbleRegistry::new(); let b = r.create_bubble(owner(), owner(), 500_000, vec![0u8;64], sign).unwrap(); let id = b.id; r.transfer_bubble(&id, &owner(), owner2(), sign).unwrap(); assert_eq!(r.bubbles.get(&id).unwrap().owner, owner2()); }
    #[test] fn test_freeze() { let mut r = BubbleRegistry::new(); let b = r.create_bubble(owner(), owner(), 1_000_000, vec![0u8;64], sign).unwrap(); r.freeze_bubble(&b.id, "test").unwrap(); assert!(r.bubbles.get(&b.id).unwrap().is_frozen()); }
    #[test] fn test_balance() { let mut r = BubbleRegistry::new(); r.create_bubble(owner(), owner(), 1_000_000, vec![0u8;64], sign).unwrap(); r.create_bubble(owner(), owner(), 500_000, vec![0u8;64], sign).unwrap(); assert_eq!(r.get_balance(&owner()), 1_500_000); }
}


// ═══════════════════════════════════════════════════════════════
// FONCTIONS BULLES UQ — SYSTÈME DE POINTS
// ═══════════════════════════════════════════════════════════════

/// Convertit les bulles en POINTS (1 bulle = 100 points)
pub fn to_points(balance: u64, points: u8) -> u128 {
    (balance as u128) * (crate::constants::BULLES_PER_BORB as u128) + points as u128
}

/// Calcule la valeur USD d'un solde en points
pub fn value_usd(balance_points: u64, gold_price: u64) -> f64 {
    let bubbles = balance_points as f64 / crate::constants::BULLES_PER_BORB as f64;
    let bubble_value = gold_price as f64 / crate::constants::BORB_PER_OUNCE as f64;
    bubbles * bubble_value
}

/// Calcule les frais de transaction en points (0.1%)
pub fn calculate_fee(amount_points: u64) -> u64 {
    (amount_points as f64 * crate::constants::TX_FEE_BPS as f64 / 10000.0).ceil() as u64
}




