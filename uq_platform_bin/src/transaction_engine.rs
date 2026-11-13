// ====================
// 💸 TRANSACTION ENGINE — Transactions (Decimal 28-30)
// ====================
//
// Ce module gère les CALCULS NORMAUX :
// - Conversion USD → BORB
// - Calcul des frais
// - Distribution aux 6 Cases
//
// Précision : 28-30 décimales (rust_decimal)
//
// ====================

use rust_decimal::Decimal;
use rust_decimal::prelude::*;
use serde::{Serialize, Deserialize};
use crate::montant::MontantBorb;

/// Résultat d'une transaction
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionResult {
    pub montant_usd: Decimal,
    pub montant_borb: MontantBorb,
    pub prix_usd: Decimal,
    pub frais_usd: Decimal,
    pub frais_borb: u128,
    pub distribution: DistributionDecimal,
    pub precision: String,
}

/// Distribution des frais (Decimal)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistributionDecimal {
    pub ocean: Decimal,
    pub nuages: Decimal,
    pub rivieres: Decimal,
    pub glaciers: Decimal,
    pub nappes: Decimal,
    pub vapeur: Decimal,
}

pub struct TransactionEngine;

impl TransactionEngine {
    /// Calcule une transaction en USD
    pub fn calculer(
        montant_usd: Decimal,
        prix_usd: Decimal,
        tx_type: TxType,
    ) -> TransactionResult {
        // 1. Convertir en BORB
        let montant_borb = MontantBorb::from_usd_decimal(montant_usd, prix_usd);
        
        // 2. Calculer les frais selon le type
        let taux_frais = Self::taux_frais(tx_type);
        let frais_usd = montant_usd * taux_frais;
        let frais_borb = MontantBorb::from_usd_decimal(frais_usd, prix_usd).0;
        
        // 3. Distribution
        let distribution = Self::distribuer(frais_usd);
        
        TransactionResult {
            montant_usd,
            montant_borb,
            prix_usd,
            frais_usd,
            frais_borb,
            distribution,
            precision: "28-30 décimales".to_string(),
        }
    }
    
    /// Taux de frais selon le type
    fn taux_frais(tx_type: TxType) -> Decimal {
        match tx_type {
            TxType::Send | TxType::Receive => Decimal::new(1, 3),      // 0.1%
            TxType::ZandoBuy | TxType::ZandoSell => Decimal::new(1, 2), // 1%
            TxType::P2PTrade => Decimal::new(1, 3),                     // 0.1%
            TxType::Ubi | TxType::Bubble => Decimal::ZERO,              // 0%
            TxType::Saving => Decimal::ZERO,                            // 0%
            TxType::Credit => Decimal::new(2, 2),                       // 2%
            TxType::Shield => Decimal::ZERO,                            // 0%
            TxType::Reader => Decimal::new(10, 2),                      // 10%
            TxType::Starlink => Decimal::new(10, 2),                    // 10%
            TxType::Delivery => Decimal::new(5, 3),                     // 0.5%
            TxType::Treasury => Decimal::ZERO,                          // 0%
        }
    }
    
    /// Distribution des frais (30/40/15/10/5/0)
    fn distribuer(frais: Decimal) -> DistributionDecimal {
        DistributionDecimal {
            ocean: frais * Decimal::new(30, 2),
            nuages: frais * Decimal::new(40, 2),
            rivieres: frais * Decimal::new(15, 2),
            glaciers: frais * Decimal::new(10, 2),
            nappes: frais * Decimal::new(5, 2),
            vapeur: Decimal::ZERO,
        }
    }
}

/// Type de transaction
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TxType {
    Send,
    Receive,
    ZandoBuy,
    ZandoSell,
    P2PTrade,
    Ubi,
    Bubble,
    Saving,
    Credit,
    Shield,
    Reader,
    Starlink,
    Delivery,
    Treasury,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    
    #[test]
    fn test_calcul_transaction() {
        let montant = Decimal::from_str("22.00").unwrap();
        let prix = Decimal::from_str("1000.00").unwrap();
        
        let result = TransactionEngine::calculer(montant, prix, TxType::Send);
        
        // 22 USD → 0.022 BORB
        assert_eq!(result.montant_borb.to_borb_decimal(), Decimal::from_str("0.022").unwrap());
        
        // Frais 0.1% de 22 USD = 0.022 USD
        assert_eq!(result.frais_usd, Decimal::from_str("0.022").unwrap());
    }
    
    #[test]
    fn test_distribution() {
        let frais = Decimal::from_str("100").unwrap();
        let dist = TransactionEngine::distribuer(frais);
        
        assert_eq!(dist.ocean, Decimal::from_str("30").unwrap());
        assert_eq!(dist.nuages, Decimal::from_str("40").unwrap());
        assert_eq!(dist.rivieres, Decimal::from_str("15").unwrap());
        assert_eq!(dist.glaciers, Decimal::from_str("10").unwrap());
        assert_eq!(dist.nappes, Decimal::from_str("5").unwrap());
    }
}
