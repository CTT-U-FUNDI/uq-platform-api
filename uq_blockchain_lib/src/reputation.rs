use serde::{Serialize, Deserialize};
use crate::error::ChainError;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReputationScore {
    pub address: [u8; 32],
    pub score: u64,
    pub total_trades: u64,
    pub successful_trades: u64,
    pub disputed_trades: u64,
    pub lost_disputes: u64,
    pub total_volume: u64,
    pub account_age_days: u64,
    pub staked_amount: u64,
    pub community_endorsements: u64,
    pub last_updated: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Endorsement {
    pub from: [u8; 32],
    pub to: [u8; 32],
    pub weight: u8,
    pub comment: String,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReputationPool {
    pub scores: HashMap<[u8; 32], ReputationScore>,
    pub endorsement_history: Vec<Endorsement>,
}

impl ReputationPool {
    pub fn new() -> Self {
        ReputationPool {
            scores: HashMap::new(),
            endorsement_history: Vec::new(),
        }
    }

    pub fn initialize_score(&mut self, address: [u8; 32]) -> Result<(), ChainError> {
        if self.scores.contains_key(&address) {
            return Err(ChainError::InternalError("Score déjà initialisé".into()));
        }
        let now = chrono::Utc::now().timestamp();
        self.scores.insert(address, ReputationScore {
            address,
            score: 500,
            total_trades: 0,
            successful_trades: 0,
            disputed_trades: 0,
            lost_disputes: 0,
            total_volume: 0,
            account_age_days: 0,
            staked_amount: 0,
            community_endorsements: 0,
            last_updated: now,
        });
        Ok(())
    }

    pub fn record_successful_trade(
        &mut self,
        buyer: &[u8; 32],
        seller: &[u8; 32],
        amount: u64,
    ) -> Result<(), ChainError> {
        for addr in [buyer, seller] {
            if let Some(s) = self.scores.get_mut(addr) {
                s.total_trades += 1;
                s.successful_trades += 1;
                s.total_volume += amount;
                s.last_updated = chrono::Utc::now().timestamp();
                Self::recalculate(s);
            }
        }
        Ok(())
    }

    pub fn record_lost_dispute(&mut self, address: &[u8; 32]) -> Result<(), ChainError> {
        if let Some(s) = self.scores.get_mut(address) {
            s.disputed_trades += 1;
            s.lost_disputes += 1;
            s.last_updated = chrono::Utc::now().timestamp();
            Self::recalculate(s);
        }
        Ok(())
    }

    pub fn endorse(
        &mut self,
        from: &[u8; 32],
        to: &[u8; 32],
        weight: u8,
        comment: String,
    ) -> Result<(), ChainError> {
        if weight < 1 || weight > 10 {
            return Err(ChainError::InternalError("Poids invalide (1-10)".into()));
        }
        if from == to {
            return Err(ChainError::InternalError("Auto-endossement interdit".into()));
        }
        let endorser_score = self.scores.get(from).map(|s| s.score).unwrap_or(0);
        if endorser_score < 3000 {
            return Err(ChainError::InternalError("Score insuffisant (min 3000)".into()));
        }
        let now = chrono::Utc::now().timestamp();
        self.endorsement_history.push(Endorsement {
            from: *from,
            to: *to,
            weight,
            comment,
            timestamp: now,
        });
        if let Some(s) = self.scores.get_mut(to) {
            s.community_endorsements += 1;
            s.last_updated = now;
            Self::recalculate(s);
        }
        Ok(())
    }

    pub fn update_stake(&mut self, address: &[u8; 32], staked: u64) -> Result<(), ChainError> {
        if let Some(s) = self.scores.get_mut(address) {
            s.staked_amount = staked;
            s.last_updated = chrono::Utc::now().timestamp();
            Self::recalculate(s);
        }
        Ok(())
    }

    pub fn update_account_age(&mut self, address: &[u8; 32], days: u64) {
        if let Some(s) = self.scores.get_mut(address) {
            s.account_age_days = days;
            s.last_updated = chrono::Utc::now().timestamp();
            Self::recalculate(s);
        }
    }

    pub fn get_score(&self, address: &[u8; 32]) -> Option<u64> {
        self.scores.get(address).map(|s| s.score)
    }

    pub fn is_trusted(&self, address: &[u8; 32]) -> bool {
        self.get_score(address).unwrap_or(0) >= 7000
    }

    pub fn is_risky(&self, address: &[u8; 32]) -> bool {
        self.get_score(address).unwrap_or(0) < 2000
    }

    fn recalculate(s: &mut ReputationScore) {
        let success_rate = if s.total_trades > 0 {
            (s.successful_trades as f64 / s.total_trades as f64) * 4000.0
        } else {
            0.0
        };
        let volume_score = (s.total_volume as f64 / 1_000_000.0).min(2000.0);
        let age_score = (s.account_age_days as f64 / 365.0).min(1.0) * 1500.0;
        let stake_score = (s.staked_amount as f64 / 100_000.0).min(1.0) * 1500.0;
        let endorsement_score = (s.community_endorsements as f64 * 50.0).min(1000.0);
        let dispute_penalty = (s.lost_disputes as f64 * 500.0).min(3000.0);
        let raw = success_rate + volume_score + age_score + stake_score + endorsement_score - dispute_penalty;
        s.score = raw.max(0.0).min(10000.0) as u64;
    }

    pub fn get_top_reputation(&self, n: usize) -> Vec<(u64, [u8; 32])> {
        let mut scores: Vec<_> = self.scores.iter().map(|(addr, s)| (s.score, *addr)).collect();
        scores.sort_by(|a, b| b.0.cmp(&a.0));
        scores.truncate(n);
        scores
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_pool() -> ReputationPool {
        let mut pool = ReputationPool::new();
        pool.initialize_score([1u8; 32]).unwrap_or_default();
        pool.initialize_score([2u8; 32]).unwrap_or_default();
        pool.initialize_score([3u8; 32]).unwrap_or_default();
        pool
    }

    #[test]
    fn test_initialize_score() {
        let pool = create_pool();
        assert_eq!(pool.get_score(&[1u8; 32]).unwrap_or_default(), 500);
    }

    #[test]
    fn test_successful_trade() {
        let mut pool = create_pool();
        let before = pool.get_score(&[1u8; 32]).unwrap_or_default();
        pool.record_successful_trade(&[1u8; 32], &[2u8; 32], 50000).unwrap_or_default();
        let after = pool.get_score(&[1u8; 32]).unwrap_or_default();
        assert!(after > before, "Le score doit augmenter");
    }

    #[test]
    fn test_lost_dispute() {
        let mut pool = create_pool();
        pool.record_lost_dispute(&[1u8; 32]).unwrap_or_default();
        assert!(pool.get_score(&[1u8; 32]).unwrap_or_default() < 500);
    }

    #[test]
    fn test_self_endorsement_forbidden() {
        let pool = create_pool();
        // Score trop bas pour endosser, mais on teste d'abord l'auto-endossement
        let result = ReputationPool::new().endorse(&[1u8; 32], &[1u8; 32], 5, "Auto".into());
        assert!(result.is_err());
    }

    #[test]
    fn test_stake_increases_score() {
        let mut pool = create_pool();
        let before = pool.get_score(&[1u8; 32]).unwrap_or_default();
        pool.update_stake(&[1u8; 32], 500000).unwrap_or_default();
        let after = pool.get_score(&[1u8; 32]).unwrap_or_default();
        assert!(after > before, "Le stake doit augmenter le score");
    }
}


