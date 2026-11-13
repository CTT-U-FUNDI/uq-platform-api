// ====================
// 💰 MONTANT — Stockage en u128 (18 décimales)
// ====================
//
// « L'eau ne se crée pas. L'eau ne se détruit pas. L'eau circule. »
//
// Ce module gère les QUANTITÉS de BORB en entiers u128.
// 1 BORB = 10^18 femto-BORB
//
// ====================

use bigdecimal::BigDecimal;
use rust_decimal::Decimal;
use rust_decimal::prelude::*;
use serde::{Serialize, Deserialize};
use std::str::FromStr;

/// 1 BORB = 10^18 femto-BORB (u128)
pub const FEMTO_PAR_BORB: u128 = 1_000_000_000_000_000_000;

/// Capacité totale : 40 Md BORB en femto
pub const CAPACITE_TOTALE: u128 = 40_000_000_000_000_000_000_000_000_000;

/// Représente un montant en femto-BORB (u128)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct MontantBorb(pub u128);

impl MontantBorb {
    /// Crée depuis un montant BORB entier
    pub fn from_borb(borb: u64) -> Self {
        MontantBorb((borb as u128) * FEMTO_PAR_BORB)
    }
    
    /// Crée depuis un montant USD avec Decimal (28-30 décimales)
    pub fn from_usd_decimal(montant_usd: Decimal, prix_usd: Decimal) -> Self {
        if prix_usd.is_zero() {
            return MontantBorb(0);
        }
        
        let montant_borb = montant_usd / prix_usd;
        let dix_puissance_18 = Decimal::from_i128_with_scale(1_000_000_000_000_000_000, 0);
        let montant_femto = montant_borb * dix_puissance_18;
        
        let montant_u128 = montant_femto
            .to_string()
            .split('.')
            .next()
            .unwrap_or("0")
            .parse::<u128>()
            .unwrap_or(0);
        
        MontantBorb(montant_u128)
    }
    
    /// Crée depuis un montant USD avec BigDecimal (60 décimales)
    pub fn from_usd_bigdecimal(montant_usd: &BigDecimal, prix_usd: &BigDecimal) -> Self {
        if prix_usd == &BigDecimal::from(0) {
            return MontantBorb(0);
        }
        
        let montant_borb = montant_usd / prix_usd;
        let dix_puissance_18 = BigDecimal::from_str("1000000000000000000").unwrap();
        let montant_femto = montant_borb * dix_puissance_18;
        
        let montant_u128 = montant_femto
            .to_string()
            .split('.')
            .next()
            .unwrap_or("0")
            .parse::<u128>()
            .unwrap_or(0);
        
        MontantBorb(montant_u128)
    }
    
    /// Convertit en BORB décimal (BigDecimal 60)
    pub fn to_borb_bigdecimal(&self) -> BigDecimal {
        BigDecimal::from(self.0) / BigDecimal::from(FEMTO_PAR_BORB)
    }
    
    /// Convertit en BORB décimal (Decimal 28-30)
    pub fn to_borb_decimal(&self) -> Decimal {
        Decimal::from_i128_with_scale(self.0 as i128, 18)
    }
    
    /// Convertit en USD (Decimal 28-30)
    pub fn to_usd_decimal(&self, prix_usd: Decimal) -> Decimal {
        self.to_borb_decimal() * prix_usd
    }
    
    /// Convertit en USD (BigDecimal 60)
    pub fn to_usd_bigdecimal(&self, prix_usd: &BigDecimal) -> BigDecimal {
        &self.to_borb_bigdecimal() * prix_usd
    }
    
    /// Addition sécurisée
    pub fn add(&self, other: MontantBorb) -> Result<MontantBorb, String> {
        self.0.checked_add(other.0)
            .map(MontantBorb)
            .ok_or_else(|| "Dépassement u128".to_string())
    }
    
    /// Soustraction sécurisée
    pub fn sub(&self, other: MontantBorb) -> Result<MontantBorb, String> {
        self.0.checked_sub(other.0)
            .map(MontantBorb)
            .ok_or_else(|| "Solde insuffisant".to_string())
    }
}

// ====================
// TESTS
// ====================

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_from_borb() {
        let m = MontantBorb::from_borb(1);
        assert_eq!(m.0, 1_000_000_000_000_000_000);
    }
    
    #[test]
    fn test_from_usd_decimal() {
        let prix = Decimal::from_str("1000.00").unwrap();
        let montant = Decimal::from_str("22.00").unwrap();
        let m = MontantBorb::from_usd_decimal(montant, prix);
        
        // 22 / 1000 = 0.022 BORB = 22 000 000 000 000 000 femto
        assert_eq!(m.0, 22_000_000_000_000_000);
    }
    
    #[test]
    fn test_capacite_totale() {
        assert_eq!(CAPACITE_TOTALE, 40_000_000_000 * FEMTO_PAR_BORB);
    }
}
