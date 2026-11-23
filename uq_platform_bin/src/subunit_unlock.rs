// ====================
// SUBUNIT UNLOCK — Déblocage progressif des sous-unités BORB
// ====================
//
// Règle : quand la dernière sous-unité active vaut 1 USD,
//         on débloque les 3 suivantes.
//
// ====================

use bigdecimal::BigDecimal;
use bigdecimal::ToPrimitive;
use std::str::FromStr;

/// Structure d'une sous-unité
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Subunit {
    pub unit: String,
    pub name: String,
    pub rate: u128,
    pub exponent: u32,
    pub position: usize,
    pub description: String,
}

/// Les 6 sous-unités de BASE (toujours actives)
/// Format : (unit, name, rate, exponent)
pub const BASE_UNITS: &[(&str, &str, u128, u32)] = &[
    ("BORB", "BORB", 1, 0),
    ("borb", "FIXO", 100, 2),
    ("mborb", "MICRO", 10_000, 4),
    ("nborb", "NANO", 1_000_000, 6),
    ("pborb", "PICCO", 100_000_000, 8),
    ("fborb", "FEMTO", 10_000_000_000, 10),
];

/// Les sous-unités ÉTENDUES (déblocables, ×1000)
/// Format : (unit, name, exponent)
pub const EXTENDED_UNITS: &[(&str, &str, u32)] = &[
    ("aborb", "ATTO", 15),
    ("zborb", "ZEPTO", 18),
    ("yborb", "YOCTO", 21),
    ("rborb", "RONNA", 24),
    ("qborb", "QUETTA", 27),
    ("cborb", "CENTI", 30),
    ("dborb", "DECI", 33),
    ("hborb", "HECTO", 36),
];

/// Retourne TOUTES les sous-unités débloquées (base + débloquées)
pub fn get_all_unlocked_units(unlocked_count: usize) -> Vec<Subunit> {
    let mut result = Vec::new();
    let mut position = 1;

    // 1. Unités de base
    for (unit, name, rate, exponent) in BASE_UNITS {
        let exp = *exponent;
        let rate_val = 10u128.pow(exp);
        result.push(Subunit {
            unit: unit.to_string(),
            name: name.to_string(),
            rate: rate_val,
            exponent: exp,
            position,
            description: format!("1 BORB = 10^{} {}", exp, unit),
        });
        position += 1;
    }

    // 2. Unités étendues débloquées
    let extended_to_add = unlocked_count.saturating_sub(BASE_UNITS.len());
    for i in 0..extended_to_add.min(EXTENDED_UNITS.len()) {
        let (unit, name, exponent) = EXTENDED_UNITS[i];
        let rate_val = 10u128.pow(exponent);
        result.push(Subunit {
            unit: unit.to_string(),
            name: name.to_string(),
            rate: rate_val,
            exponent,
            position,
            description: format!("1 BORB = 10^{} {}", exponent, unit),
        });
        position += 1;
    }

    result
}

/// Vérifie si on doit débloquer les 3 sous-unités suivantes
/// Retourne le nombre de nouvelles unités à débloquer (0, 3, 6...)
pub fn check_and_unlock(prix_borb_usd: &BigDecimal, current_unlocked: usize) -> usize {
    // Si toutes les unités sont déjà débloquées → rien à faire
    if current_unlocked >= BASE_UNITS.len() + EXTENDED_UNITS.len() {
        return 0;
    }

    // Position de la dernière unité active
    let last_pos = current_unlocked;

    // Calculer l'exposant de la dernière unité
    let last_exponent = if last_pos <= BASE_UNITS.len() {
        BASE_UNITS[last_pos - 1].3
    } else {
        let ext_idx = last_pos - BASE_UNITS.len() - 1;
        if ext_idx >= EXTENDED_UNITS.len() {
            return 0;
        }
        EXTENDED_UNITS[ext_idx].2
    };

    // Valeur en USD d'1 unité
    let divisor = BigDecimal::from(10u128.pow(last_exponent));
    let last_value = prix_borb_usd / divisor;

    // Si ≥ 1 USD → débloquer les 3 suivantes
    if last_value >= BigDecimal::from(1) {
        let remaining = (BASE_UNITS.len() + EXTENDED_UNITS.len()) - current_unlocked;
        return remaining.min(3);
    }

    0
}

/// Retourne le statut actuel des sous-unités
pub fn get_unlock_status(prix_borb_usd: &BigDecimal, unlocked_count: usize) -> serde_json::Value {
    let all_units = get_all_unlocked_units(unlocked_count);
    let last_unit = all_units.last();

    let last_unit_value_usd = if let Some(lu) = last_unit {
        let divisor = BigDecimal::from(10u128.pow(lu.exponent));
        (prix_borb_usd / divisor).to_f64().unwrap_or(0.0)
    } else {
        0.0
    };

    let should_unlock = check_and_unlock(prix_borb_usd, unlocked_count);

    serde_json::json!({
        "unlocked_count": unlocked_count,
        "total_available": BASE_UNITS.len() + EXTENDED_UNITS.len(),
        "all_unlocked": unlocked_count >= BASE_UNITS.len() + EXTENDED_UNITS.len(),
        "units": all_units,
        "last_unit_value_usd": last_unit_value_usd,
        "should_unlock_next_3": should_unlock > 0,
        "next_unlock_count": should_unlock,
    })
}
