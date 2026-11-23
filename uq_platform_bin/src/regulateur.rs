// ====================
// 🧮 RÉGULATEUR — Calcul du prix (BigDecimal 60 décimales)
// ====================
//
// Ce module gère la RÉGULATION :
// - Calcul du prix en 60 décimales
// - Ajustement automatique
// - Facteurs de demande/rareté/vitesse
//
// ====================

use bigdecimal::BigDecimal;
use bigdecimal::ToPrimitive;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// Prix initial (1 BORB = 1 USD) — 60 décimales
pub fn prix_initial() -> BigDecimal {
    BigDecimal::from_str("1.000000000000000000000000000000000000000000000000000000000000").unwrap()
}

/// Parité structurelle de la plateforme : 1 BORB vaut 1/2000 d'once d'or, soit
/// 0,0005 once.
///
/// Ce nombre n'est PAS un taux de marché : c'est la définition du BORB, comme
/// `MASSE_BORB`. Le seul taux qui bouge est le prix de l'once, capté par le
/// moteur. Il vivait en trois littéraux `2000.0` dans `api_handlers` (or, usd,
/// all) et dans leurs tests — donc trois endroits à faire bouger le jour où la
/// parité change, et aucun pour la lire.
pub const BORB_PAR_ONCE: f64 = 2000.0;

/// Valeur USD d'un BORB dérivée du prix de l'once : `once / 2000`.
/// `None` si le prix de l'once n'est pas une observation exploitable — une
/// capture à 0 ou `NaN` ne doit jamais devenir un prix.
pub fn borb_usd_depuis_or(gold_usd_per_ounce: f64) -> Option<f64> {
    (gold_usd_per_ounce.is_finite() && gold_usd_per_ounce > 0.0)
        .then_some(gold_usd_per_ounce / BORB_PAR_ONCE)
}

pub struct Regulateur;

impl Regulateur {
    /// Masse totale BORB (40 milliards — FIXE)
    pub const MASSE_BORB: u128 = 40_000_000_000;

    /// Calcule le prix BORB en USD
    /// Formule : prix = max(1, demande_usd / masse_borb)
    /// - Demande ≤ 40 Md → 1 BORB = 1 USD
    /// - Demande > 40 Md → 1 BORB = demande / 40 Md USD
    pub fn calculer_prix(demande_usd: u128) -> BigDecimal {
        if demande_usd <= Self::MASSE_BORB {
            BigDecimal::from(1)
        } else {
            let demande_bd = BigDecimal::from(demande_usd);
            let masse_bd = BigDecimal::from(Self::MASSE_BORB);
            demande_bd / masse_bd
        }
    }

    /// Choisit l'unité d'affichage selon le prix
    /// - Prix < 1000 USD → BORB
    /// - Prix < 10 000 USD → borb
    /// - etc.
    pub fn choisir_unite(prix_usd: &BigDecimal) -> (&'static str, &'static str, u128) {
        let prix: f64 = prix_usd.to_f64().unwrap_or(1.0);

        if prix < 1_000.0 {
            ("BORB", "BORB", 1)
        } else if prix < 10_000.0 {
            ("borb", "FIXO", 100)
        } else if prix < 1_000_000.0 {
            ("mborb", "MICRO", 10_000)
        } else if prix < 100_000_000.0 {
            ("nborb", "NANO", 1_000_000)
        } else if prix < 10_000_000_000.0 {
            ("pborb", "PICCO", 100_000_000)
        } else {
            ("fborb", "FEMTO", 10_000_000_000)
        }
    }

    /// Facteur de demande
    pub fn facteur_demande(demande_actuelle: u128, demande_precedente: u128) -> BigDecimal {
        if demande_precedente == 0 {
            return BigDecimal::from(1);
        }

        let actuelle = BigDecimal::from(demande_actuelle);
        let precedente = BigDecimal::from(demande_precedente);

        actuelle / precedente
    }

    /// Facteur de rareté
    pub fn facteur_rarete(circulation: u128, masse: u128) -> BigDecimal {
        if masse == 0 {
            return BigDecimal::from(1);
        }

        let circulation_bd = BigDecimal::from(circulation);
        let masse_bd = BigDecimal::from(masse);

        BigDecimal::from(1) - (circulation_bd / masse_bd)
    }

    /// Facteur de vitesse
    pub fn facteur_vitesse(tx_actuelles: u128, tx_precedentes: u128) -> BigDecimal {
        if tx_precedentes == 0 {
            return BigDecimal::from(1);
        }

        let actuelles = BigDecimal::from(tx_actuelles);
        let precedentes = BigDecimal::from(tx_precedentes);

        actuelles / precedentes
    }

    /// Ajuste si nécessaire (auto-régulation)
    pub fn ajuster_si_necessaire(prix_actuel: &BigDecimal) -> Option<BigDecimal> {
        let seuil_haut = BigDecimal::from_str("10000.00").unwrap();
        let seuil_bas = BigDecimal::from_str("0.001").unwrap();

        if prix_actuel > &seuil_haut {
            // Prix trop haut → augmenter l'offre (baisser le prix)
            return Some(prix_actuel * BigDecimal::from_str("0.95").unwrap());
        }

        if prix_actuel < &seuil_bas {
            // Prix trop bas → réduire l'offre (monter le prix)
            return Some(prix_actuel * BigDecimal::from_str("1.05").unwrap());
        }

        None
    }

    /// Vérifie l'invariant
    pub fn verifier_invariant(total: &BigDecimal, capacite: &BigDecimal) -> bool {
        let diff = (total - capacite).abs();
        let tolerance =
            BigDecimal::from_str("0.000000000000000000000000000000000000000000000000000000000001")
                .unwrap();

        diff < tolerance
    }
}

/// État du régulateur
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegulateurState {
    pub prix_actuel: String, // BigDecimal sérialisé en string
    pub demande: u128,
    pub rarete: String,
    pub vitesse: String,
    pub dernier_ajustement: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prix_initial() {
        let prix = prix_initial();
        assert_eq!(
            prix.to_string(),
            "1.000000000000000000000000000000000000000000000000000000000000"
        );
    }

    #[test]
    fn test_facteur_demande() {
        let f = Regulateur::facteur_demande(1000, 500);
        // 1000 / 500 = 2
        assert_eq!(f.to_string(), "2");
    }

    #[test]
    fn test_facteur_rarete() {
        let f = Regulateur::facteur_rarete(10, 100);
        // 1 - 10/100 = 0.9
        assert_eq!(f.to_string(), "0.9");
    }
}
