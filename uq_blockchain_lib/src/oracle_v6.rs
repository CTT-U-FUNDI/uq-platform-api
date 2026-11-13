use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uq_black_key_standalone::UqBlackKey;
use crate::error::ChainError;

// ═══════════════════════════════════════════════════════════════
// CONSTANTES
// ═══════════════════════════════════════════════════════════════

pub const ORACLE_VALIDATOR_COUNT: usize = 7;
pub const ORACLE_CONSENSUS_REQUIRED: usize = 5;
pub const ORACLE_ROTATION_INTERVAL: u64 = 24 * 3600; // 24h
pub const MAX_PRICE_DEVIATION_BPS: u64 = 500; // 5% d'écart max

// ═══════════════════════════════════════════════════════════════
// STRUCTURES
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OracleValidator {
    pub address: [u8; 32],
    pub stake: u64,
    pub reputation: u64,
    pub joined_at: u64,
    pub last_report_at: u64,
    pub total_reports: u64,
    pub accurate_reports: u64,
    pub is_active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceReport {
    pub validator: [u8; 32],
    pub pair: String,
    pub price_micro: u64,
    pub timestamp: u64,
    pub signature: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OraclePrice {
    pub pair: String,
    pub median_price_micro: u64,
    pub num_reports: usize,
    pub last_updated: u64,
    pub volatility_bps: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OracleRegistry {
    pub validators: Vec<OracleValidator>,
    pub prices: HashMap<String, OraclePrice>,
    pub pending_reports: Vec<PriceReport>,
    pub last_rotation: u64,
    pub total_validators_registered: u64,
}

#[derive(Debug, Clone)]
pub struct OracleStats {
    pub active_validators: usize,
    pub total_reports: u64,
    pub prices_tracked: usize,
    pub consensus_rate: f64,
}

// ═══════════════════════════════════════════════════════════════
// IMPLÉMENTATION
// ═══════════════════════════════════════════════════════════════

impl OracleRegistry {
    pub fn new() -> Self {
        OracleRegistry {
            validators: Vec::new(),
            prices: HashMap::new(),
            pending_reports: Vec::new(),
            last_rotation: utc_now(),
            total_validators_registered: 0,
        }
    }

    /// Enregistre un nouveau validateur oracle
    pub fn register_validator(&mut self, address: [u8; 32], stake: u64, key: &UqBlackKey) -> Result<(), ChainError> {
        if !key.can_operate() {
            return Err(ChainError::InternalError("Clé non opérationnelle".into()));
        }
        if self.validators.iter().any(|v| v.address == address) {
            return Err(ChainError::InternalError("Déjà enregistré".into()));
        }
        if self.validators.len() >= ORACLE_VALIDATOR_COUNT {
            // Remplacer le moins performant
            self.validators.sort_by_key(|v| v.reputation);
            self.validators.remove(0);
        }
        let now = utc_now();
        self.validators.push(OracleValidator {
            address, stake, reputation: 100, joined_at: now,
            last_report_at: 0, total_reports: 0, accurate_reports: 0, is_active: true,
        });
        self.total_validators_registered += 1;
        Ok(())
    }

    /// Soumet un rapport de prix signé
    pub fn report_price(&mut self, validator: &[u8; 32], pair: &str,
        price_micro: u64, key: &UqBlackKey) -> Result<(), ChainError> {
        let v = self.validators.iter_mut().find(|v| v.address == *validator)
            .ok_or(ChainError::InternalError("Validateur inconnu".into()))?;
        if !v.is_active { return Err(ChainError::InternalError("Validateur inactif".into())); }
        
        let payload = format!("{}:{}:{}", pair, price_micro, utc_now());
        let signature = key.sign(payload.as_bytes());
        
        self.pending_reports.push(PriceReport {
            validator: *validator, pair: pair.to_string(),
            price_micro, timestamp: utc_now(), signature: signature.to_vec(),
        });
        v.last_report_at = utc_now();
        v.total_reports += 1;
        Ok(())
    }

    /// Calcule le prix médian et met à jour le registre
    pub fn finalize_prices(&mut self) -> Result<u64, ChainError> {
        let mut pairs: HashMap<String, Vec<u64>> = HashMap::new();
        for report in &self.pending_reports {
            pairs.entry(report.pair.clone()).or_default().push(report.price_micro);
        }
        
        let mut updated = 0u64;
        for (pair, prices) in &pairs {
            if prices.len() < ORACLE_CONSENSUS_REQUIRED { continue; }
            let mut sorted = prices.clone();
            sorted.sort();
            let median = sorted[sorted.len() / 2];
            
            let avg = prices.iter().sum::<u64>() / prices.len() as u64;
            let deviation = if avg > 0 { (median.max(avg) - median.min(avg)) * 10000 / avg } else { 0 };
            
            self.prices.insert(pair.clone(), OraclePrice {
                pair: pair.clone(), median_price_micro: median,
                num_reports: prices.len(), last_updated: utc_now(),
                volatility_bps: deviation,
            });
            updated += 1;
        }
        
        // Récompenser les validateurs précis
        for report in &self.pending_reports {
            if let Some(price) = self.prices.get(&report.pair) {
                let deviation = if price.median_price_micro > 0 {
                    (report.price_micro.max(price.median_price_micro) - report.price_micro.min(price.median_price_micro)) * 10000 / price.median_price_micro
                } else { 0 };
                if let Some(v) = self.validators.iter_mut().find(|v| v.address == report.validator) {
                    if deviation <= MAX_PRICE_DEVIATION_BPS {
                        v.accurate_reports += 1;
                        v.reputation = v.reputation.saturating_add(1);
                    } else {
                        v.reputation = v.reputation.saturating_sub(5);
                    }
                }
            }
        }
        
        self.pending_reports.clear();
        Ok(updated)
    }

    /// Rotation périodique des validateurs
    pub fn rotate_validators(&mut self) -> Result<(), ChainError> {
        let now = utc_now();
        if now - self.last_rotation < ORACLE_ROTATION_INTERVAL { return Ok(()); }
        
        // Désactiver les inactifs (>7 jours sans rapport)
        for v in &mut self.validators {
            if now - v.last_report_at > 7 * 24 * 3600 && v.last_report_at > 0 {
                v.is_active = false;
            }
        }
        
        // Trier par réputation
        self.validators.sort_by_key(|v| -(v.reputation as i64));
        self.last_rotation = now;
        Ok(())
    }

    /// Récupère le prix médian d'une paire
    pub fn get_price(&self, pair: &str) -> Option<u64> {
        self.prices.get(pair).map(|p| p.median_price_micro)
    }

    /// Vérifie si un prix est valide (dans la déviation acceptable)
    pub fn is_price_valid(&self, pair: &str, price_micro: u64) -> bool {
        if let Some(p) = self.prices.get(pair) {
            if p.median_price_micro == 0 { return true; }
            let deviation = (price_micro.max(p.median_price_micro) - price_micro.min(p.median_price_micro)) * 10000 / p.median_price_micro;
            deviation <= MAX_PRICE_DEVIATION_BPS
        } else { true }
    }

    /// Retourne les statistiques
    pub fn get_stats(&self) -> OracleStats {
        let active = self.validators.iter().filter(|v| v.is_active).count();
        let total_reports: u64 = self.validators.iter().map(|v| v.total_reports).sum();
        let accurate: u64 = self.validators.iter().map(|v| v.accurate_reports).sum();
        OracleStats {
            active_validators: active,
            total_reports,
            prices_tracked: self.prices.len(),
            consensus_rate: if total_reports > 0 { accurate as f64 / total_reports as f64 } else { 0.0 },
        }
    }
}

fn utc_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()
}

// ═══════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use super::*;

    fn test_key() -> UqBlackKey { let mut k = UqBlackKey::generate(); k.activate(); k }
    fn addr(i: u8) -> [u8; 32] { let mut a = [0u8; 32]; a[0] = i; a }

    #[test] fn test_new() { let r = OracleRegistry::new(); assert_eq!(r.validators.len(), 0); }
    
    #[test] fn test_register() {
        let mut r = OracleRegistry::new();
        r.register_validator(addr(1), 1000, &test_key()).unwrap_or_default();
        assert_eq!(r.validators.len(), 1);
    }

    #[test] fn test_register_7() {
        let mut r = OracleRegistry::new();
        for i in 1..=7 { r.register_validator(addr(i), 1000, &test_key()).unwrap_or_default(); }
        assert_eq!(r.validators.len(), 7);
    }

    #[test] fn test_report_and_finalize() {
        let mut r = OracleRegistry::new();
        let key = test_key();
        for i in 1..=7 { r.register_validator(addr(i), 1000, &test_key()).unwrap_or_default(); }
        for i in 1..=7 { r.report_price(&addr(i), "BORB/USD", 1000 + i as u64, &key).unwrap_or_default(); }
        let updated = r.finalize_prices().unwrap_or_default();
        assert!(updated >= 1);
        assert!(r.get_price("BORB/USD").is_some());
    }

    #[test] fn test_insufficient_reports() {
        let mut r = OracleRegistry::new();
        let key = test_key();
        for i in 1..=7 { r.register_validator(addr(i), 1000, &test_key()).unwrap_or_default(); }
        // Seulement 3 rapports (< 5 requis)
        for i in 1..=3 { r.report_price(&addr(i), "BORB/USD", 1000, &key).unwrap_or_default(); }
        let updated = r.finalize_prices().unwrap_or_default();
        assert_eq!(updated, 0); // Pas assez de consensus
    }

    #[test] fn test_price_validation() {
        let mut r = OracleRegistry::new();
        let key = test_key();
        for i in 1..=7 { r.register_validator(addr(i), 1000, &test_key()).unwrap_or_default(); }
        for i in 1..=7 { r.report_price(&addr(i), "BORB/USD", 100000, &key).unwrap_or_default(); }
        r.finalize_prices().unwrap_or_default();
        assert!(r.is_price_valid("BORB/USD", 100000));
        assert!(!r.is_price_valid("BORB/USD", 200000)); // Trop loin
    }

    #[test] fn test_stats() {
        let mut r = OracleRegistry::new();
        let key = test_key();
        for i in 1..=5 { r.register_validator(addr(i), 1000, &test_key()).unwrap_or_default(); }
        let s = r.get_stats();
        assert_eq!(s.active_validators, 5);
    }

    #[test] fn test_rotation() {
        let mut r = OracleRegistry::new();
        let key = test_key();
        for i in 1..=7 { r.register_validator(addr(i), 1000, &test_key()).unwrap_or_default(); }
        // Forcer la rotation en modifiant last_rotation
        r.last_rotation = 0;
        r.rotate_validators().unwrap_or_default();
        assert!(r.last_rotation > 0);
    }
}


