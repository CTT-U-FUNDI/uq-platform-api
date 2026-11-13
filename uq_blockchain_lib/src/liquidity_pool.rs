// ═══════════════════════════════════════════════════════════════
// UQ CHAIN — PISCINE DE LIQUIDITÉ (LIQUIDITY POOL)
// ═══════════════════════════════════════════════════════════════

use crate::error::ChainError;
use std::collections::HashMap;

/// Une paire de trading dans la piscine
#[derive(Debug, Clone)]
pub struct LiquidityPair {
    pub base_currency: String,      // BORB
    pub quote_currency: String,     // USD, EUR, CDF, etc.
    pub base_reserve: u64,          // Quantité BORB dans la piscine
    pub quote_reserve: u64,         // Quantité quote dans la piscine
    pub fee_pct: u8,                // Frais de swap (0.3%)
    pub total_lp_tokens: u64,       // Total des tokens LP émis
    pub providers: HashMap<[u8; 32], u64>, // Adresse → LP tokens
}

impl LiquidityPair {
    pub fn new(base: &str, quote: &str, initial_base: u64, initial_quote: u64) -> Self {
        LiquidityPair {
            base_currency: base.to_string(),
            quote_currency: quote.to_string(),
            base_reserve: initial_base,
            quote_reserve: initial_quote,
            fee_pct: 3, // 0.3%
            total_lp_tokens: initial_base, // 1:1 au départ
            providers: HashMap::new(),
        }
    }

    /// Calcule le prix : combien de quote pour 1 base
    pub fn get_price(&self) -> f64 {
        if self.base_reserve == 0 { return 0.0; }
        self.quote_reserve as f64 / self.base_reserve as f64
    }

    /// Ajoute de la liquidité
    pub fn add_liquidity(&mut self, provider: [u8; 32], base_amount: u64, quote_amount: u64) -> u64 {
        let lp_tokens = if self.total_lp_tokens == 0 {
            base_amount
        } else {
            (base_amount as u128 * self.total_lp_tokens as u128 / self.base_reserve as u128) as u64
        };
        self.base_reserve += base_amount;
        self.quote_reserve += quote_amount;
        self.total_lp_tokens += lp_tokens;
        *self.providers.entry(provider).or_insert(0) += lp_tokens;
        lp_tokens
    }

    /// Échange BORB → quote (ex: BORB → USD)
    pub fn swap_base_to_quote(&mut self, amount_in: u64) -> Result<u64, ChainError> {
        if amount_in == 0 || self.base_reserve == 0 || self.quote_reserve == 0 {
            return Err(ChainError::InternalError("Liquidité insuffisante".into()));
        }
        let fee = amount_in * self.fee_pct as u64 / 1000;
        let amount_in_after_fee = amount_in - fee;
        let amount_out = (amount_in_after_fee as u128 * self.quote_reserve as u128 
            / (self.base_reserve as u128 + amount_in_after_fee as u128)) as u64;
        self.base_reserve += amount_in;
        self.quote_reserve -= amount_out;
        Ok(amount_out)
    }

    /// Échange quote → BORB (ex: USD → BORB)
    pub fn swap_quote_to_base(&mut self, amount_in: u64) -> Result<u64, ChainError> {
        if amount_in == 0 || self.base_reserve == 0 || self.quote_reserve == 0 {
            return Err(ChainError::InternalError("Liquidité insuffisante".into()));
        }
        let fee = amount_in * self.fee_pct as u64 / 1000;
        let amount_in_after_fee = amount_in - fee;
        let amount_out = (amount_in_after_fee as u128 * self.base_reserve as u128 
            / (self.quote_reserve as u128 + amount_in_after_fee as u128)) as u64;
        self.quote_reserve += amount_in;
        self.base_reserve -= amount_out;
        Ok(amount_out)
    }
}

/// Gestionnaire de toutes les piscines de liquidité
pub struct LiquidityPoolManager {
    pub pairs: HashMap<String, LiquidityPair>,
}

impl LiquidityPoolManager {
    pub fn new() -> Self {
        let mut manager = LiquidityPoolManager { pairs: HashMap::new() };
        // Piscines par défaut
        manager.create_pair("BORB", "USD", 1_000_000, 10); // 1M BORB = 10 USD (prix initial)
        manager.create_pair("BORB", "CDF", 1_000_000, 28_000_000); // 1M BORB = 28M CDF
        manager.create_pair("BORB", "EUR", 1_000_000, 9); // 1M BORB = 9 EUR
        manager.create_pair("BORB", "GOLD", 1_000_000, 1); // 1M BORB = 1g Or
        manager
    }

    pub fn create_pair(&mut self, base: &str, quote: &str, base_amount: u64, quote_amount: u64) {
        let key = format!("{}/{}", base, quote);
        self.pairs.insert(key, LiquidityPair::new(base, quote, base_amount, quote_amount));
    }

    pub fn get_price(&self, base: &str, quote: &str) -> Option<f64> {
        let key = format!("{}/{}", base, quote);
        self.pairs.get(&key).map(|p| p.get_price())
    }

    pub fn swap(&mut self, from: &str, to: &str, amount: u64) -> Result<u64, ChainError> {
        let key = format!("{}/{}", from, to);
        if let Some(pair) = self.pairs.get_mut(&key) {
            return pair.swap_base_to_quote(amount);
        }
        // Essayer l'inverse
        let key_inv = format!("{}/{}", to, from);
        if let Some(pair) = self.pairs.get_mut(&key_inv) {
            return pair.swap_quote_to_base(amount);
        }
        Err(ChainError::InternalError("Paire non trouvée".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_pair() {
        let pair = LiquidityPair::new("BORB", "USD", 1_000_000, 10);
        assert_eq!(pair.base_reserve, 1_000_000);
        assert_eq!(pair.quote_reserve, 10);
        assert_eq!(pair.get_price(), 0.00001); // 10 USD / 1M BORB
    }

    #[test]
    fn test_swap() {
        let mut pair = LiquidityPair::new("BORB", "USD", 1_000_000, 10);
        let usd_out = pair.swap_base_to_quote(1000).unwrap_or_default();
        assert!(usd_out > 0);
        assert!(pair.base_reserve > 1_000_000); // BORB ajoutés
        assert!(pair.quote_reserve < 10); // USD retirés
    }

    #[test]
    fn test_add_liquidity() {
        let mut pair = LiquidityPair::new("BORB", "USD", 1_000_000, 10);
        let lp = pair.add_liquidity([1u8; 32], 100_000, 1);
        assert!(lp > 0);
        assert_eq!(pair.base_reserve, 1_100_000);
        assert_eq!(pair.quote_reserve, 11);
    }
}

