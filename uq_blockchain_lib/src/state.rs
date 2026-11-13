use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const PERSISTENCE_MAGIC: u32 = 0x55513031;
pub const PERSISTENCE_VERSION: u32 = 6;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountState {
    pub balance: u64,
    pub nonce: u64,
    pub staked: u64,
    pub is_validator: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalState {
    pub migration_version: u32,
    pub accounts: HashMap<[u8; 32], AccountState>,
    pub total_accounts: u64,
}

impl GlobalState {
    pub fn new() -> Self {
        GlobalState { migration_version: PERSISTENCE_VERSION, accounts: HashMap::new(), total_accounts: 0 }
    }
    pub fn set_account(&mut self, address: [u8; 32], account: AccountState) {
        if !self.accounts.contains_key(&address) { self.total_accounts += 1; }
        self.accounts.insert(address, account);
    }
    pub fn get_account(&self, address: &[u8; 32]) -> Option<&AccountState> {
        self.accounts.get(address)
    }
    pub fn get_balance(&self, address: &[u8; 32]) -> u64 {
        self.accounts.get(address).map(|a| a.balance).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn test_new() { let s = GlobalState::new(); assert_eq!(s.migration_version, 6); assert_eq!(s.total_accounts, 0); }
    #[test] fn test_set_get() { let mut s = GlobalState::new(); s.set_account([1u8;32], AccountState { balance: 100, nonce: 0, staked: 0, is_validator: false }); assert_eq!(s.get_balance(&[1u8;32]), 100); }
    #[test] fn test_magic() { assert_eq!(PERSISTENCE_MAGIC, 0x55513031); }
}
