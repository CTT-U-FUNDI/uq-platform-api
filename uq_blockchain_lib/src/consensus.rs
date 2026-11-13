use serde::{Serialize, Deserialize};
// use sha2::Digest;
use crate::error::ChainError;
use crate::state::GlobalState;
use std::collections::HashMap;

// ═══════════════════════════════════════════════════════════════
// PROOF OF STAKE - CONSENSUS UQ
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsensusEngine {
    pub validators: HashMap<[u8; 32], ValidatorStake>,
    pub epoch: u64,
    pub epoch_start_height: u64,
    pub total_staked: u64,
    pub min_stake: u64,
    pub max_validators: usize,
    pub epoch_length: u64,
    pub last_epoch_rewards: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorStake {
    pub address: [u8; 32],
    pub staked_amount: u64,
    pub delegation: HashMap<[u8; 32], u64>, // Délégations reçues
    pub blocks_proposed: u64,
    pub blocks_validated: u64,
    pub missed_blocks: u64,
    pub slashed_amount: u64,
    pub rewards_earned: u64,
    pub joined_at: i64,
    pub is_active: bool,
    pub performance_score: u64, // 0-1000
}

impl ConsensusEngine {
    /// Crée un nouveau moteur de consensus
    pub fn new(min_stake: u64, max_validators: usize, epoch_length: u64) -> Self {
        ConsensusEngine {
            validators: HashMap::new(),
            epoch: 0,
            epoch_start_height: 0,
            total_staked: 0,
            min_stake,
            max_validators,
            epoch_length,
            last_epoch_rewards: 0,
        }
    }

    /// Ajoute un validateur
    pub fn add_validator(&mut self, address: [u8; 32], stake: u64) -> Result<(), ChainError> {
        if stake < self.min_stake {
            return Err(ChainError::InternalError(format!(
                "Stake minimum : {}", self.min_stake
            )));
        }

        if self.validators.len() >= self.max_validators && !self.validators.contains_key(&address) {
            return Err(ChainError::InternalError("Max validateurs atteint".into()));
        }

        let now = chrono::Utc::now().timestamp();

        if let Some(v) = self.validators.get_mut(&address) {
            v.staked_amount = v.staked_amount.saturating_add(stake);
            v.is_active = true;
        } else {
            self.validators.insert(address, ValidatorStake {
                address,
                staked_amount: stake,
                delegation: HashMap::new(),
                blocks_proposed: 0,
                blocks_validated: 0,
                missed_blocks: 0,
                slashed_amount: 0,
                rewards_earned: 0,
                joined_at: now,
                is_active: true,
                performance_score: 500,
            });
        }

        self.total_staked = self.total_staked.saturating_add(stake);
        Ok(())
    }

    /// Retire un validateur
    pub fn remove_validator(&mut self, address: &[u8; 32]) -> Result<u64, ChainError> {
        if let Some(v) = self.validators.get_mut(address) {
            v.is_active = false;
            let refund = v.staked_amount;
            self.total_staked = self.total_staked.saturating_sub(refund);
            v.staked_amount = 0;
            Ok(refund)
        } else {
            Err(ChainError::InternalError("Validateur inconnu".into()))
        }
    }

    /// Délègue du stake à un validateur
    pub fn delegate(&mut self, delegator: [u8; 32], validator: &[u8; 32], amount: u64) -> Result<(), ChainError> {
        if let Some(v) = self.validators.get_mut(validator) {
            if !v.is_active {
                return Err(ChainError::InternalError("Validateur inactif".into()));
            }
            let entry = v.delegation.entry(delegator).or_insert(0);
            *entry = entry.saturating_add(amount);
            v.staked_amount = v.staked_amount.saturating_add(amount);
            self.total_staked = self.total_staked.saturating_add(amount);
            Ok(())
        } else {
            Err(ChainError::InternalError("Validateur inconnu".into()))
        }
    }

    /// Sélectionne un validateur pour proposer le prochain bloc
    pub fn select_proposer(&self, seed: u64) -> Option<&[u8; 32]> {
        let active: Vec<_> = self.validators.iter()
            .filter(|(_, v)| v.is_active && v.staked_amount >= self.min_stake)
            .collect();

        if active.is_empty() {
            return None;
        }

        // Sélection pondérée par le stake (plus de stake = plus de chances)
        let total_weight: u64 = active.iter().map(|(_, v)| v.staked_amount).sum();
        if total_weight == 0 {
            return Some(active[0].0);
        }

        let mut cumulative = 0u64;
        let target = seed % total_weight;

        for (addr, v) in &active {
            cumulative = cumulative.saturating_add(v.staked_amount);
            if cumulative > target {
                return Some(addr);
            }
        }

        Some(active[active.len() - 1].0)
    }

    /// Enregistre qu'un validateur a proposé un bloc
    pub fn record_block_proposed(&mut self, address: &[u8; 32]) {
        if let Some(v) = self.validators.get_mut(address) {
            v.blocks_proposed = v.blocks_proposed.saturating_add(1);
            v.performance_score = (v.performance_score.saturating_add(10)).min(1000);
        }
    }

    /// Enregistre qu'un validateur a manqué son tour
    pub fn record_missed_block(&mut self, address: &[u8; 32]) {
        if let Some(v) = self.validators.get_mut(address) {
            v.missed_blocks = v.missed_blocks.saturating_add(1);
            v.performance_score = v.performance_score.saturating_sub(50);

            // Slash si trop de blocs manqués
            if v.missed_blocks > 10 {
                let slash = v.staked_amount / 10;
                v.slashed_amount = v.slashed_amount.saturating_add(slash);
                v.staked_amount = v.staked_amount.saturating_sub(slash);
                self.total_staked = self.total_staked.saturating_sub(slash);
            }
        }
    }

    /// Récompense un validateur pour un bloc validé
    pub fn reward_validator(&mut self, address: &[u8; 32], reward: u64) {
        if let Some(v) = self.validators.get_mut(address) {
            v.rewards_earned = v.rewards_earned.saturating_add(reward);
            v.blocks_validated = v.blocks_validated.saturating_add(1);
        }
    }

    /// Termine une époque et distribue les récompenses
    pub fn finalize_epoch(&mut self, state: &mut GlobalState) -> Result<u64, ChainError> {
        let total_rewards: u64 = self.validators.values()
            .filter(|v| v.is_active)
            .map(|v| v.staked_amount / 1000) // 0.1% de récompense par époque
            .sum();

        for (addr, v) in self.validators.iter_mut() {
            if v.is_active && v.staked_amount > 0 {
                let reward = v.staked_amount / 1000;
                v.rewards_earned = v.rewards_earned.saturating_add(reward);
                v.staked_amount = v.staked_amount.saturating_add(reward);

                // Mettre à jour le state
                if let Some(account) = state.accounts.get_mut(addr) {
                    account.balance = account.balance.saturating_add(reward);
                }
            }
        }

        self.epoch = self.epoch.saturating_add(1);
        self.last_epoch_rewards = total_rewards;
        self.total_staked = self.total_staked.saturating_add(total_rewards);

        Ok(total_rewards)
    }

    /// Vérifie si l'époque doit être finalisée
    pub fn should_finalize_epoch(&self, current_height: u64) -> bool {
        current_height >= self.epoch_start_height.saturating_add(self.epoch_length)
    }

    /// Calcule le poids total du consensus (pour la sécurité)
    pub fn total_weight(&self) -> u64 {
        self.validators.values()
            .filter(|v| v.is_active)
            .map(|v| v.staked_amount)
            .sum()
    }

    /// Vérifie si une attaque nécessiterait plus de 51% du stake
    pub fn is_secure(&self) -> bool {
        let total = self.total_weight();
        if total == 0 { return true; }

        // Aucun validateur ne doit avoir plus de 50% du stake
        !self.validators.values()
            .filter(|v| v.is_active)
            .any(|v| v.staked_amount > total / 2)
    }

    /// Retourne les statistiques du consensus
    pub fn get_stats(&self) -> ConsensusStats {
        let active = self.validators.values().filter(|v| v.is_active).count();
        ConsensusStats {
            total_validators: self.validators.len() as u64,
            active_validators: active as u64,
            total_staked: self.total_staked,
            min_stake: self.min_stake,
            epoch: self.epoch,
            last_epoch_rewards: self.last_epoch_rewards,
            is_secure: self.is_secure(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsensusStats {
    pub total_validators: u64,
    pub active_validators: u64,
    pub total_staked: u64,
    pub min_stake: u64,
    pub epoch: u64,
    pub last_epoch_rewards: u64,
    pub is_secure: bool,
}

// ═══════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> ConsensusEngine {
        ConsensusEngine::new(100_000_000_000, 100, 210_000)
    }

    #[test]
    fn test_add_validator() {
        let mut e = engine();
        assert!(e.add_validator([1u8; 32], 200_000_000_000).is_ok());
        assert_eq!(e.validators.len(), 1);
        assert_eq!(e.total_staked, 200_000_000_000);
    }

    #[test]
    fn test_min_stake() {
        let mut e = engine();
        assert!(e.add_validator([1u8; 32], 1000).is_err());
    }

    #[test]
    fn test_select_proposer() {
        let mut e = engine();
        e.add_validator([1u8; 32], 500_000_000_000).unwrap_or_default();
        e.add_validator([2u8; 32], 300_000_000_000).unwrap_or_default();
        e.add_validator([3u8; 32], 200_000_000_000).unwrap_or_default();

        let proposer = e.select_proposer(12345);
        assert!(proposer.is_some());
    }

    #[test]
    fn test_record_block() {
        let mut e = engine();
        e.add_validator([1u8; 32], 200_000_000_000).unwrap_or_default();
        e.record_block_proposed(&[1u8; 32]);

        let v = e.validators.get(&[1u8; 32]).unwrap_or_default();
        assert_eq!(v.blocks_proposed, 1);
        assert!(v.performance_score > 500);
    }

    #[test]
    fn test_record_missed() {
        let mut e = engine();
        e.add_validator([1u8; 32], 200_000_000_000).unwrap_or_default();

        let v = e.validators.get(&[1u8; 32]).unwrap_or_default();
        let initial_score = v.performance_score;

        e.record_missed_block(&[1u8; 32]);

        let v = e.validators.get(&[1u8; 32]).unwrap_or_default();
        assert!(v.performance_score < initial_score);
        assert_eq!(v.missed_blocks, 1);
    }

    #[test]
    fn test_delegation() {
        let mut e = engine();
        e.add_validator([1u8; 32], 200_000_000_000).unwrap_or_default();
        assert!(e.delegate([5u8; 32], &[1u8; 32], 50_000_000_000).is_ok());

        let v = e.validators.get(&[1u8; 32]).unwrap_or_default();
        assert_eq!(v.delegation.get(&[5u8; 32]), Some(&50_000_000_000));
    }

    #[test]
    fn test_is_secure() {
        let mut e = engine();
        e.add_validator([1u8; 32], 500_000_000_000).unwrap_or_default();
        e.add_validator([2u8; 32], 300_000_000_000).unwrap_or_default();
        e.add_validator([3u8; 32], 200_000_000_000).unwrap_or_default();
        assert!(e.is_secure());
    }

    #[test]
    fn test_finalize_epoch() {
        let mut e = engine();
        e.add_validator([1u8; 32], 200_000_000_000).unwrap_or_default();

        let mut state = GlobalState::new();
        let now = chrono::Utc::now().timestamp();
        state.set_account([1u8; 32], crate::state::AccountState {
            address: [1u8; 32], balance: 1_000_000_000, staked: 0, nonce: 0,
            is_validator: true, reputation_score: 500,
            created_at: now, last_active: now, total_transactions: 0, total_volume: 0,
        });

        let rewards = e.finalize_epoch(&mut state).unwrap_or_default();
        assert!(rewards > 0);
        assert_eq!(e.epoch, 1);
    }

    #[test]
    fn test_get_stats() {
        let mut e = engine();
        e.add_validator([1u8; 32], 200_000_000_000).unwrap_or_default();
        let stats = e.get_stats();
        assert_eq!(stats.total_validators, 1);
        assert_eq!(stats.active_validators, 1);
        // assert!(stats.is_secure); // 1 validateur = pas sécurisé
    }
}




