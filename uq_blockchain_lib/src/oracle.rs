use serde::{Serialize, Deserialize};
// use sha2::Digest;
use crate::error::ChainError;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OracleRegistry {
    pub oracles: HashMap<[u8; 32], OracleNode>,
    pub price_feeds: HashMap<String, PriceFeed>,
    pub min_confirmations: u32,
    pub max_deviation_bps: u64,
    pub heartbeat_interval: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OracleNode {
    pub address: [u8; 32],
    pub reputation: u64,
    pub total_reports: u64,
    pub successful_reports: u64,
    pub last_heartbeat: i64,
    pub is_active: bool,
    pub staked_amount: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceFeed {
    pub pair: String,
    pub current_price: u64,
    pub last_updated: i64,
    pub reports: Vec<PriceReport>,
    pub median_price: u64,
    pub volatility_24h: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceReport {
    pub oracle: [u8; 32],
    pub price: u64,
    pub timestamp: i64,
    pub signature: Vec<u8>,
}

impl OracleRegistry {
    pub fn new(min_confirmations: u32, max_deviation_bps: u64) -> Self {
        OracleRegistry {
            oracles: HashMap::new(),
            price_feeds: HashMap::new(),
            min_confirmations,
            max_deviation_bps,
            heartbeat_interval: 3600,
        }
    }

    pub fn register_oracle(&mut self, address: [u8; 32], stake: u64) -> Result<(), ChainError> {
        if stake < 1_000_000_000_000 {
            return Err(ChainError::InternalError("Stake minimum: 1000 BORB".into()));
        }
        if self.oracles.contains_key(&address) {
            return Err(ChainError::InternalError("Oracle déjà enregistré".into()));
        }
        self.oracles.insert(address, OracleNode {
            address, reputation: 500, total_reports: 0, successful_reports: 0,
            last_heartbeat: chrono::Utc::now().timestamp(), is_active: true, staked_amount: stake,
        });
        Ok(())
    }

    pub fn report_price(&mut self, oracle: &[u8; 32], pair: &str, price: u64, signature: Vec<u8>) -> Result<(), ChainError> {
        let node = self.oracles.get_mut(oracle).ok_or_else(|| ChainError::InternalError("Oracle inconnu".into()))?;
        if !node.is_active { return Err(ChainError::InternalError("Oracle inactif".into())); }
        if price == 0 { return Err(ChainError::InternalError("Prix nul".into())); }

        let now = chrono::Utc::now().timestamp();
        let report = PriceReport { oracle: *oracle, price, timestamp: now, signature };
        let feed = self.price_feeds.entry(pair.to_string()).or_insert(PriceFeed {
            pair: pair.to_string(), current_price: price, last_updated: now,
            reports: Vec::new(), median_price: price, volatility_24h: 0,
        });

        feed.reports.retain(|r| now - r.timestamp < 86400);
        feed.reports.push(report);
        feed.last_updated = now;

        if feed.reports.len() >= self.min_confirmations as usize {
            let mut prices: Vec<u64> = feed.reports.iter().map(|r| r.price).collect();
            prices.sort();
            let median = prices[prices.len() / 2];
            feed.current_price = median;
            feed.median_price = median;

            if prices.len() >= 2 {
                let avg = prices.iter().sum::<u64>() / prices.len() as u64;
                feed.volatility_24h = if avg > 0 { (prices[prices.len()-1].saturating_sub(prices[0])) * 10000 / avg } else { 0 };
            }
        }

        node.total_reports = node.total_reports.saturating_add(1);
        node.successful_reports = node.successful_reports.saturating_add(1);
        node.last_heartbeat = now;
        Ok(())
    }

    pub fn get_price(&self, pair: &str) -> Option<u64> {
        self.price_feeds.get(pair).map(|f| f.current_price)
    }

    pub fn get_median_price(&self, pair: &str) -> Option<u64> {
        self.price_feeds.get(pair).map(|f| f.median_price)
    }

    pub fn get_volatility(&self, pair: &str) -> Option<u64> {
        self.price_feeds.get(pair).map(|f| f.volatility_24h)
    }

    pub fn is_price_valid(&self, pair: &str, price: u64) -> bool {
        if let Some(feed) = self.price_feeds.get(pair) {
            if feed.current_price == 0 { return true; }
            let deviation = if price > feed.current_price {
                (price - feed.current_price) * 10000 / feed.current_price
            } else {
                (feed.current_price - price) * 10000 / feed.current_price
            };
            deviation <= self.max_deviation_bps
        } else { true }
    }

    pub fn slash_inactive_oracles(&mut self) -> u64 {
        let now = chrono::Utc::now().timestamp();
        let mut total_slashed = 0u64;
        for (_, node) in self.oracles.iter_mut() {
            if node.is_active && now - node.last_heartbeat > self.heartbeat_interval * 3 {
                let slash = node.staked_amount / 10;
                node.staked_amount = node.staked_amount.saturating_sub(slash);
                node.reputation = node.reputation.saturating_sub(100);
                total_slashed = total_slashed.saturating_add(slash);
                if node.reputation < 200 { node.is_active = false; }
            }
        }
        total_slashed
    }

    pub fn get_stats(&self) -> OracleStats {
        OracleStats {
            total_oracles: self.oracles.len() as u64,
            active_oracles: self.oracles.values().filter(|o| o.is_active).count() as u64,
            price_feeds_count: self.price_feeds.len() as u64,
            min_confirmations: self.min_confirmations,
            total_reports: self.oracles.values().map(|o| o.total_reports).sum(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OracleStats {
    pub total_oracles: u64,
    pub active_oracles: u64,
    pub price_feeds_count: u64,
    pub min_confirmations: u32,
    pub total_reports: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oracle() -> OracleRegistry {
        let mut o = OracleRegistry::new(2, 500);
        o.register_oracle([1u8;32], 2_000_000_000_000).unwrap_or_default();
        o.register_oracle([2u8;32], 2_000_000_000_000).unwrap_or_default();
        o.register_oracle([3u8;32], 2_000_000_000_000).unwrap_or_default();
        o
    }

    #[test] fn test_register() { let o = oracle(); assert_eq!(o.oracles.len(), 3); }
    #[test] fn test_min_stake() { let mut o = OracleRegistry::new(2,500); assert!(o.register_oracle([5u8;32], 100).is_err()); }
    #[test] fn test_report_price() { let mut o = oracle(); assert!(o.report_price(&[1u8;32], "BORB/USD", 100_000, vec![1,2,3]).is_ok()); }
    #[test] fn test_get_price() { let mut o = oracle(); o.report_price(&[1u8;32], "BORB/USD", 100_000, vec![1,2,3]).unwrap_or_default(); o.report_price(&[2u8;32], "BORB/USD", 100_000, vec![1,2,3]).unwrap_or_default(); assert!(o.get_price("BORB/USD").is_some()); }
    #[test] fn test_median() { let mut o = oracle(); o.report_price(&[1u8;32], "BORB/USD", 90_000, vec![1,2,3]).unwrap_or_default(); o.report_price(&[2u8;32], "BORB/USD", 100_000, vec![1,2,3]).unwrap_or_default(); o.report_price(&[3u8;32], "BORB/USD", 110_000, vec![1,2,3]).unwrap_or_default(); assert_eq!(o.get_median_price("BORB/USD"), Some(100_000)); }
    #[test] fn test_price_valid() { let mut o = oracle(); o.report_price(&[1u8;32], "BORB/USD", 100_000, vec![1,2,3]).unwrap_or_default(); o.report_price(&[2u8;32], "BORB/USD", 100_000, vec![1,2,3]).unwrap_or_default(); assert!(o.is_price_valid("BORB/USD", 102_000)); assert!(!o.is_price_valid("BORB/USD", 200_000)); }
    #[test] fn test_slash() { let mut o = oracle(); o.report_price(&[1u8;32], "BORB/USD", 100_000, vec![1,2,3]).unwrap_or_default(); let slashed = o.slash_inactive_oracles(); assert_eq!(slashed, 0); }
    #[test] fn test_stats() { let mut o = oracle(); o.report_price(&[1u8;32], "BORB/USD", 100_000, vec![1,2,3]).unwrap_or_default(); assert_eq!(o.get_stats().total_oracles, 3); }
}


