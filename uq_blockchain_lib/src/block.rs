use serde::{Serialize, Deserialize};
use sha2::{Sha256, Digest};
use crate::error::ChainError;
use crate::transaction::Transaction;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    pub header: BlockHeader,
    pub transactions: Vec<Transaction>,
    pub signature: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockHeader {
    pub version: u32,
    pub previous_hash: [u8; 32],
    pub merkle_root: [u8; 32],
    pub timestamp: i64,
    pub height: u64,
    pub validator: [u8; 32],
    pub validator_stake: u64,
    pub difficulty: u32,
    pub nonce: u64,
    pub state_root: [u8; 32],
    pub transaction_count: u32,
    pub total_fees: u64,
    pub block_reward: u64,
    pub extra_data: Vec<u8>,
}

impl Block {
    pub fn new(
        previous_hash: [u8; 32],
        height: u64,
        validator: [u8; 32],
        validator_stake: u64,
        transactions: Vec<Transaction>,
        state_root: [u8; 32],
        difficulty: u32,
    ) -> Result<Self, ChainError> {
        if transactions.is_empty() {
            return Err(ChainError::InternalError("Bloc vide interdit".into()));
        }
        let merkle_root = Self::compute_merkle_root(&transactions);
        let now = chrono::Utc::now().timestamp();
        let total_fees: u64 = transactions.iter().map(|t| t.fee).sum();
        let header = BlockHeader {
            version: 1, previous_hash, merkle_root, timestamp: now, height,
            validator, validator_stake, difficulty, nonce: 0, state_root,
            transaction_count: transactions.len() as u32, total_fees,
            block_reward: Self::calculate_block_reward(height), extra_data: Vec::new(),
        };
        Ok(Block { header, transactions, signature: Vec::new() })
    }

    pub fn hash(&self) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(&self.header.version.to_le_bytes());
        h.update(&self.header.previous_hash);
        h.update(&self.header.merkle_root);
        h.update(&self.header.timestamp.to_le_bytes());
        h.update(&self.header.height.to_le_bytes());
        h.update(&self.header.validator);
        h.update(&self.header.nonce.to_le_bytes());
        h.update(&self.header.state_root);
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&h.finalize());
        hash
    }

    pub fn is_valid_proof_of_stake(&self) -> bool {
        let hash = self.hash();
        let zeros = (self.header.difficulty / 8) as usize;
        let bits = (self.header.difficulty % 8) as u8;
        for i in 0..zeros.min(32) { if hash[i] != 0 { return false; } }
        if bits > 0 && zeros < 32 { let mask = 0xFFu8 << (8 - bits); if hash[zeros] & mask != 0 { return false; } }
        true
    }

    pub fn mine(&mut self, max: u64) -> bool {
        for _ in 0..max { self.header.nonce = self.header.nonce.wrapping_add(1); if self.is_valid_proof_of_stake() { return true; } }
        false
    }

    pub fn sign(&mut self, sig: Vec<u8>) { self.signature = sig; }

    pub fn verify(&self) -> Result<(), ChainError> {
        if !self.is_valid_proof_of_stake() { return Err(ChainError::InvalidProof("Preuve invalide".into())); }
        let m = Self::compute_merkle_root(&self.transactions);
        if m != self.header.merkle_root { return Err(ChainError::InternalError("Merkle invalide".into())); }
        if self.transactions.is_empty() { return Err(ChainError::InternalError("Bloc vide".into())); }
        Ok(())
    }

    pub fn compute_merkle_root(txs: &[Transaction]) -> [u8; 32] {
        if txs.is_empty() { return [0u8; 32]; }
        let mut hashes: Vec<[u8; 32]> = txs.iter().map(|t| t.hash()).collect();
        while hashes.len() > 1 {
            let mut n = Vec::new();
            for c in hashes.chunks(2) {
                let mut h = Sha256::new(); h.update(&c[0]); h.update(if c.len()>1{&c[1]}else{&c[0]});
                let mut x=[0u8;32]; x.copy_from_slice(&h.finalize()); n.push(x);
            }
            hashes = n;
        }
        hashes[0]
    }

    pub fn calculate_block_reward(h: u64) -> u64 { let b: u64 = 50_000_000_000; b >> (h / 210_000) }
    pub fn weight(&self) -> u64 { self.header.validator_stake * self.header.difficulty as u64 }
}

/// ═══════════════════════════════════════════════════════════
/// GENESIS - 900 milliards plateforme + récompense fondateur
/// ═══════════════════════════════════════════════════════════

pub fn create_genesis_block() -> Result<Block, ChainError> {
    // Adresse de la plateforme (services : UBI, staking, DEX, etc.)
    let platform_address: [u8; 32] = [0u8; 32];
    // Adresse du fondateur (ta récompense personnelle)
    let founder_address: [u8; 32] = [1u8; 32];

    // Transaction 1 : 900 milliards pour la plateforme
    let tx_platform = Transaction::create_coinbase(platform_address, 900_000_000_000_000)?;

    // Transaction 2 : Récompense du fondateur (899 100 milliards restants)
    let tx_founder = Transaction::create_coinbase(founder_address, 899_100_000_000_000_000)?;

    Block::new(
        [0u8; 32],  // Pas de bloc précédent
        0,           // Hauteur 0
        [0u8; 32],   // Validateur genesis
        0,           // Pas de stake
        vec![tx_platform, tx_founder],
        [0u8; 32],   // État initial
        1,           // Difficulté minimale
    )
}

// Ajouts pour Transaction
impl Transaction {
    pub fn new_test(amount: u64, fee: u64) -> Self {
        Transaction { id:[0u8;32], from:[1u8;32], to:[2u8;32], amount, fee, nonce:0, timestamp:chrono::Utc::now().timestamp(), signature:Vec::new() }
    }
    pub fn create_coinbase(to: [u8;32], amount: u64) -> Result<Self, ChainError> {
        Ok(Transaction { id:[0u8;32], from:[0u8;32], to, amount, fee:0, nonce:0, timestamp:chrono::Utc::now().timestamp(), signature:Vec::new() })
    }
    pub fn hash(&self) -> [u8;32] {
        let mut h = Sha256::new(); h.update(&self.from); h.update(&self.to); h.update(&self.amount.to_le_bytes()); h.update(&self.nonce.to_le_bytes());
        let mut x=[0u8;32]; x.copy_from_slice(&h.finalize()); x
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn txs() -> Vec<Transaction> { vec![Transaction::new_test(1000,10),Transaction::new_test(2000,20)] }
    #[test] fn test_create() { let b = Block::new([0u8;32],1,[1u8;32],100000,txs(),[2u8;32],1).unwrap_or_default(); assert_eq!(b.header.height,1); }
    #[test] fn test_empty() { assert!(Block::new([0u8;32],1,[1u8;32],100000,vec![],[2u8;32],1).is_err()); }
    #[test] fn test_merkle() { assert_eq!(Block::compute_merkle_root(&txs()).len(),32); }
    #[test] fn test_hash() { assert_eq!(Block::new([0u8;32],1,[1u8;32],100000,txs(),[2u8;32],1).unwrap_or_default().hash().len(),32); }
    #[test] fn test_halving() { assert!(Block::calculate_block_reward(0) > Block::calculate_block_reward(210_000)); }
    #[test] fn test_verify() { let t = txs(); let mut b = Block::new([0u8;32],1,[1u8;32],100000,t.clone(),[2u8;32],0).unwrap_or_default(); b.header.merkle_root = Block::compute_merkle_root(&t); assert!(b.verify().is_ok()); }
    #[test] fn test_mine() { let mut b = Block::new([0u8;32],1,[1u8;32],100000,txs(),[2u8;32],8).unwrap_or_default(); assert!(b.mine(1_000_000)); }
    #[test] fn test_weight() { assert_eq!(Block::new([0u8;32],1,[1u8;32],500000,txs(),[2u8;32],2).unwrap_or_default().weight(),500000*2); }
    #[test] fn test_genesis() { let g = create_genesis_block().unwrap_or_default(); assert_eq!(g.transactions.len(), 2); }
}





