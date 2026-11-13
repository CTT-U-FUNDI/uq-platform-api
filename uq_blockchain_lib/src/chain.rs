use serde::{Serialize, Deserialize};
use crate::error::ChainError;
use crate::block::Block;
use crate::transaction::Transaction;
use std::collections::{HashMap, VecDeque};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UqChain {
    pub blocks: Vec<Block>,
    pub pending_transactions: VecDeque<Transaction>,
    pub state: ChainState,
    pub validators: HashMap<[u8; 32], ValidatorInfo>,
    pub config: ChainConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainState {
    pub balances: HashMap<[u8; 32], u64>,
    pub nonces: HashMap<[u8; 32], u64>,
    pub total_supply: u64,
    pub total_burned: u64,
    pub total_staked: u64,
    pub height: u64,
    pub last_block_hash: [u8; 32],
    pub epoch: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorInfo {
    pub address: [u8; 32],
    pub staked_amount: u64,
    pub blocks_validated: u64,
    pub rewards_earned: u64,
    pub is_active: bool,
    pub joined_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainConfig {
    pub max_block_size: usize,
    pub max_pending_tx: usize,
    pub block_interval_seconds: i64,
    pub min_stake: u64,
    pub max_validators: usize,
    pub epoch_length: u64,
    pub ubi_amount: u64,
    pub burn_rate_bps: u64,
}

impl Default for ChainConfig {
    fn default() -> Self {
        ChainConfig {
            max_block_size: 1_000_000, max_pending_tx: 10_000,
            block_interval_seconds: 60, min_stake: 100_000_000_000,
            max_validators: 100, epoch_length: 210_000,
            ubi_amount: 10_000_000, burn_rate_bps: 10,
        }
    }
}

impl UqChain {
    pub fn new() -> Result<Self, ChainError> {
        let genesis = crate::block::create_genesis_block()?;
        let genesis_hash = genesis.hash();
        let mut balances = HashMap::new();
        let genesis_tx = &genesis.transactions[0];
        balances.insert(genesis_tx.to, genesis_tx.amount);
        let state = ChainState { balances, nonces: HashMap::new(), total_supply: genesis_tx.amount, total_burned: 0, total_staked: 0, height: 0, last_block_hash: genesis_hash, epoch: 0 };
        Ok(UqChain { blocks: vec![genesis], pending_transactions: VecDeque::new(), state, validators: HashMap::new(), config: ChainConfig::default() })
    }

    pub fn add_transaction(&mut self, tx: Transaction) -> Result<(), ChainError> {
        if self.pending_transactions.len() >= self.config.max_pending_tx { return Err(ChainError::InternalError("File pleine".into())); }
        if tx.from != [0u8;32] && tx.signature.is_empty() { return Err(ChainError::InvalidProof("Non signée".into())); }
        let nonce = self.state.nonces.get(&tx.from).copied().unwrap_or(0);
        if tx.nonce != nonce { return Err(ChainError::InternalError(format!("Nonce: attendu {} reçu {}", nonce, tx.nonce))); }
        if tx.from != [0u8;32] { let bal = self.get_balance(&tx.from); if bal < tx.amount + tx.fee { return Err(ChainError::InternalError("Solde insuffisant".into())); } }
        self.state.nonces.insert(tx.from, nonce + 1);
        self.pending_transactions.push_back(tx);
        Ok(())
    }

    pub fn create_block(&mut self, validator: [u8;32], state_root: [u8;32]) -> Result<Block, ChainError> {
        let info = self.validators.get(&validator).ok_or_else(|| ChainError::InternalError("Validateur inconnu".into()))?;
        if !info.is_active { return Err(ChainError::InternalError("Inactif".into())); }
        let max_tx = self.config.max_block_size / 200;
        let count = self.pending_transactions.len().min(max_tx);
        let txs: Vec<Transaction> = self.pending_transactions.drain(..count).collect();
        let diff = self.calculate_difficulty();
        let mut block = Block::new(self.state.last_block_hash, self.state.height + 1, validator, info.staked_amount, txs, state_root, diff)?;
        block.mine(1_000_000);
        Ok(block)
    }

    pub fn add_block(&mut self, block: Block) -> Result<(), ChainError> {
        block.verify()?;
        if block.header.height != self.state.height + 1 { return Err(ChainError::InternalError("Hauteur invalide".into())); }
        if block.header.previous_hash != self.state.last_block_hash { return Err(ChainError::InternalError("Hash précédent invalide".into())); }
        for tx in &block.transactions { self.apply_transaction(tx)?; }
        let reward = block.header.block_reward + block.header.total_fees;
        *self.state.balances.entry(block.header.validator).or_insert(0) += reward;
        if let Some(info) = self.validators.get_mut(&block.header.validator) { info.blocks_validated += 1; info.rewards_earned += reward; }
        let burn = block.header.total_fees * self.config.burn_rate_bps / 10_000;
        self.state.total_burned += burn; self.state.total_supply -= burn;
        self.state.height = block.header.height; self.state.last_block_hash = block.hash();
        self.state.epoch = block.header.height / self.config.epoch_length;
        self.blocks.push(block);
        Ok(())
    }

    fn apply_transaction(&mut self, tx: &Transaction) -> Result<(), ChainError> {
        if tx.from == [0u8;32] { *self.state.balances.entry(tx.to).or_insert(0) += tx.amount; self.state.total_supply += tx.amount; }
        else { let bal = self.state.balances.get(&tx.from).copied().unwrap_or(0); if bal < tx.amount + tx.fee { return Err(ChainError::InternalError("Solde insuffisant".into())); } *self.state.balances.entry(tx.from).or_insert(0) -= tx.amount + tx.fee; *self.state.balances.entry(tx.to).or_insert(0) += tx.amount; }
        Ok(())
    }

    pub fn register_validator(&mut self, address: [u8;32], stake: u64) -> Result<(), ChainError> {
        if stake < self.config.min_stake { return Err(ChainError::InternalError(format!("Stake min: {}", self.config.min_stake))); }
        if self.validators.len() >= self.config.max_validators { return Err(ChainError::InternalError("Max validateurs".into())); }
        let bal = self.get_balance(&address);
        if bal < stake { return Err(ChainError::InternalError("Solde insuffisant".into())); }
        *self.state.balances.entry(address).or_insert(0) -= stake;
        self.state.total_staked += stake;
        self.validators.insert(address, ValidatorInfo { address, staked_amount: stake, blocks_validated: 0, rewards_earned: 0, is_active: true, joined_at: chrono::Utc::now().timestamp() });
        Ok(())
    }

    pub fn deactivate_validator(&mut self, address: &[u8;32]) -> Result<(), ChainError> {
        if let Some(info) = self.validators.get_mut(address) { info.is_active = false; *self.state.balances.entry(*address).or_insert(0) += info.staked_amount; self.state.total_staked -= info.staked_amount; Ok(()) }
        else { Err(ChainError::InternalError("Validateur inconnu".into())) }
    }

    pub fn calculate_difficulty(&self) -> u32 {
        if self.blocks.len() < 2 { return 1; }
        let last = &self.blocks[self.blocks.len()-1];
        let prev = &self.blocks[self.blocks.len()-2];
        let interval = last.header.timestamp - prev.header.timestamp;
        if interval < self.config.block_interval_seconds / 2 { last.header.difficulty + 1 }
        else if interval > self.config.block_interval_seconds * 2 { last.header.difficulty.saturating_sub(1).max(1) }
        else { last.header.difficulty }
    }

    pub fn get_balance(&self, address: &[u8;32]) -> u64 { self.state.balances.get(address).copied().unwrap_or(0) }
    pub fn get_nonce(&self, address: &[u8;32]) -> u64 { self.state.nonces.get(address).copied().unwrap_or(0) }

    pub fn verify_full_chain(&self) -> Result<(), ChainError> {
        for i in 1..self.blocks.len() {
            if self.blocks[i].header.previous_hash != self.blocks[i-1].hash() { return Err(ChainError::InternalError(format!("Hash cassé à {}", self.blocks[i].header.height))); }
            self.blocks[i].verify()?;
        }
        Ok(())
    }

    pub fn get_stats(&self) -> ChainStats { ChainStats { height: self.state.height, total_supply: self.state.total_supply, total_burned: self.state.total_burned, total_staked: self.state.total_staked, pending_tx_count: self.pending_transactions.len() as u64, validator_count: self.validators.len() as u64, active_validator_count: self.validators.values().filter(|v|v.is_active).count() as u64, epoch: self.state.epoch } }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainStats { pub height: u64, pub total_supply: u64, pub total_burned: u64, pub total_staked: u64, pub pending_tx_count: u64, pub validator_count: u64, pub active_validator_count: u64, pub epoch: u64 }

#[cfg(test)]
mod tests {
    use super::*;
    fn make_chain() -> UqChain { let mut c = UqChain::new().unwrap_or_default(); c.state.balances.insert([1u8;32],1_000_000_000_000); c.register_validator([1u8;32],200_000_000_000).unwrap_or_default(); c }
    fn tx(from:[u8;32],to:[u8;32],amount:u64,nonce:u64) -> Transaction { Transaction{id:[0u8;32],from,to,amount,fee:1000,nonce,timestamp:chrono::Utc::now().timestamp(),signature:vec![1,2,3]} }

    #[test] fn test_create() { let c = UqChain::new().unwrap_or_default(); assert_eq!(c.state.height,0); assert_eq!(c.blocks.len(),1); }
    #[test] fn test_add_tx() { let mut c = make_chain(); assert!(c.add_transaction(tx([1u8;32],[2u8;32],1000,0)).is_ok()); assert_eq!(c.pending_transactions.len(),1); }
    #[test] fn test_bad_nonce() { let mut c = make_chain(); assert!(c.add_transaction(tx([1u8;32],[2u8;32],1000,99)).is_err()); }
    #[test] fn test_register() { let mut c = make_chain(); c.state.balances.insert([3u8;32],500_000_000_000); assert!(c.register_validator([3u8;32],150_000_000_000).is_ok()); assert_eq!(c.validators.len(),2); }
    #[test] fn test_min_stake() { let mut c = make_chain(); assert!(c.register_validator([3u8;32],1000).is_err()); }
    #[test] fn test_balance() { let c = make_chain(); let platform = c.get_balance(&[0u8;32]); let founder = c.get_balance(&[1u8;32]); println!("========================================"); println!("💰 PLATEFORME : {} BORB ({} milliards)", platform, platform/1_000_000_000_000); println!("👑 FONDATEUR  : {} BORB ({} milliards)", founder, founder/1_000_000_000_000); println!("========================================"); assert_eq!(platform, 900_000_000_000_000); }
    #[test] fn test_stats() { let c = make_chain(); let s = c.get_stats(); assert_eq!(s.height,0); assert!(s.total_supply>0); assert_eq!(s.validator_count,1); }
    #[test] fn test_block() { let mut c = make_chain(); for i in 0..3 { c.add_transaction(tx([1u8;32],[2u8;32],1000+i*100,i)).unwrap_or_default(); } let b = c.create_block([1u8;32],[0u8;32]).unwrap_or_default(); assert!(c.add_block(b).is_ok()); assert_eq!(c.state.height,1); }
    #[test] fn test_diff() { let c = make_chain(); assert!(c.calculate_difficulty()>=1); }
}



