// ═══════════════════════════════════════════════════════════════
// UQ PLATFORM — CONSTANTES GRAVÉES DANS LE BINAIRE
// ═══════════════════════════════════════════════════════════════

// ═══════════════════════════════════════════════════════════════
// ANCIENNES CONSTANTES (ESSENTIELLES — NE PAS SUPPRIMER)
// ═══════════════════════════════════════════════════════════════

/// Nombre TOTAL de bulles dans l'écosystème UQ
pub const TOTAL_BUBBLES: u64 = 40_000_000_000;

/// Bulles par BORB
pub const BULLES_PER_BORB: u64 = 10_000;

/// BORB par once d'or
pub const BORB_PER_OUNCE: u64 = 2_000;

/// Frais de transaction (basis points)
pub const TX_FEE_BPS: u64 = 10;

/// Femto par BORB (la plus petite unité)
pub const FEMTO_PER_BORB: u128 = 1_000_000_000_000_000;

/// Prix BORB en USD (indexé or)
pub const BORB_PRICE_USD: u64 = 1;

/// Micro par BORB
pub const MICRO_PER_BORB: u64 = 1_000_000;

/// Points par bulle
pub const POINTS_PER_BUBBLE: u64 = 100;

/// Masse monétaire totale
pub const TOTAL_BORB: u64 = 40_000_000_000;

// ═══════════════════════════════════════════════════════════════
// NOUVELLES CONSTANTES (AJOUTÉES)
// ═══════════════════════════════════════════════════════════════

// COMPTE PROPRIÉTAIRE (GRAVÉ)
pub const OWNER_NUMBER: &str = "UQ-00000001";
pub const OWNER_EMAIL: &str = "delis@ufundi.uq";
pub const OWNER_PSEUDO: &str = "delis-dev";
pub const OWNER_PUBLIC_NUMBER: &str = "UQ-00000123";

// Sensitive runtime secrets are loaded from environment variables.
// They must NOT be hard-coded. Defaults are provided for development only.
use std::env;
use lazy_static::lazy_static;

lazy_static! {
    // These sensitive values MUST be provided via environment variables in production.
    // For development, populate a local .env file and use dotenv to load it at startup.
    /// Unlock code for the guardian terminal. Required: OWNER_UNLOCK_CODE
    pub static ref OWNER_UNLOCK_CODE: String = env::var("OWNER_UNLOCK_CODE").expect(
        "OWNER_UNLOCK_CODE must be set in the environment (or in a .env for development)");

    /// Initial owner password. Required: OWNER_PASSWORD_INITIAL
    pub static ref OWNER_PASSWORD_INITIAL: String = env::var("OWNER_PASSWORD_INITIAL").expect(
        "OWNER_PASSWORD_INITIAL must be set in the environment (or in a .env for development)");

    /// Terminal authentication key. Required: TERMINAL_AUTH_KEY
    pub static ref TERMINAL_AUTH_KEY: String = env::var("TERMINAL_AUTH_KEY").expect(
        "TERMINAL_AUTH_KEY must be set in the environment (or in a .env for development)");
}

// NOMS SYSTÈME POUR PUBLICATIONS (GRAVÉS)
pub const SYSTEM_PUBLISH_GUIDE: &str = "UQ Info";
pub const SYSTEM_PUBLISH_ALERT: &str = "UQ Security";
pub const SYSTEM_PUBLISH_INFO: &str = "UQ Community";
pub const SYSTEM_PUBLISH_ANNONCE: &str = "UQ Treasury";
pub const SYSTEM_PUBLISH_NEWS: &str = "UQ News";
pub const SYSTEM_PUBLISH_URGENT: &str = "UQ Urgent";

// ANNONCE TREASURY (GRAVÉE)
pub const TREASURY_TOTAL: u64 = 1_000_000;
pub const TREASURY_PRICE_USD: u64 = 1;
pub const TREASURY_RAVITAILLEMENT_SECS: u64 = 5;
pub const LIMIT_NORMAL_PCT: u64 = 50;
pub const LIMIT_MARKETER_PCT: u64 = 40;
pub const LIMIT_OWNER_PCT: u64 = 30;

// RÉSERVE STABILITÉ (GRAVÉE)
pub const RESERVE_STABILITE_PCT: u64 = 50;
pub const RESERVE_DEV_PCT: u64 = 20;
pub const RESERVE_EXPANSION_PCT: u64 = 15;
pub const RESERVE_CONFORMITE_PCT: u64 = 10;
pub const RESERVE_REWARDS_PCT: u64 = 5;

// RÉCLAMATION DU PROFIL PUBLIC (GRAVÉE)
pub const OWNER_RECLAIM_THRESHOLD_PCT: u64 = 20;
pub const OWNER_RECLAIM_AMOUNT_PCT: u64 = 30;
pub const OWNER_MAX_RECLAIMS_PER_DAY: u64 = 3;

// ALIMENTATION AUTOMATIQUE DES 6 CASES (GRAVÉE)
pub const FEES_TREASURY_PCT: u64 = 30;
pub const FEES_VALIDATORS_PCT: u64 = 40;
pub const FEES_REWARDS_PCT: u64 = 20;
pub const FEES_SAVINGS_PCT: u64 = 10;
pub const FEES_CREDIT_PCT: u64 = 10;
pub const FEES_SHIELD_PCT: u64 = 5;

// SEUILS DE SÉCURITÉ (GRAVÉS)
pub const MIN_TREASURY: u64 = 20_000_000_000;
pub const MIN_FEES: u64 = 1_000_000_000;
pub const MIN_REWARDS: u64 = 1_000_000_000;
pub const MIN_SAVINGS: u64 = 500_000_000;
pub const MIN_CREDIT: u64 = 500_000_000;
pub const MIN_SHIELD: u64 = 300_000_000;

// ═══════════════════════════════════════════════════════════════
// CONSTANTES MANQUANTES (AJOUTÉES)
// ═══════════════════════════════════════════════════════════════

/// Frais critique (seuil d'alerte)
pub const FEES_CRITICAL: u64 = 500_000_000;

/// Frais cible (montant idéal)
pub const FEES_TARGET: u64 = 1_000_000_000;

/// Plancher du Trésor (ne jamais descendre en dessous)
pub const TREASURY_FLOOR: u64 = 20_000_000_000;

/// Femto par Bulle (la plus petite unité d'une bulle)
pub const FEMTO_PER_BULLE: u128 = 1_000_000_000_000_000;

/// Facteur de sous-unité (combien de sous-unités par unité)
pub const SUB_UNIT_FACTOR: u128 = 1_000_000;

pub const EXTENSION_THRESHOLD_USD : u64 = 1_000_000;
pub const ENTERPRISE_CREATE_FEE : u64 = 1_000_000;
pub const MARKETPLACE_PUBLISH_FEE : u64 = 1_000_000;
pub const PROFILE_PRO_FEE : u64 = 1_000_000;
pub const PROFILE_MERCHANT_FEE : u64 = 1_000_000;
pub const RELATION_PRO_CONTRACT_FEE : u64 = 1_000_000;
pub const IA_ADVANCED_REQUEST_FEE : u64 = 1_000_000;
pub const CONTENT_STREAM_FEE_PCT : u64 = 1_000_000;
pub const CONNECTIVITY_SALE_FEE_PCT : u64 = 1_000_000;
// REWARD_VALIDATOR_PCT — SUPPRIMEE (les 6 Cases gerent tout)
// REWARD_RELAY_PCT — SUPPRIMEE (les 6 Cases gerent tout)
// REWARD_GPU_PCT — SUPPRIMEE (les 6 Cases gerent tout)
pub const MARKETPLACE_COMMISSION_PCT : u64 = 1_000_000;
pub const MICRO_CREDIT_MAX_BORB : u64 = 1_000_000;
pub const PROFILE_ARTIST_FEE : u64 = 1_000_000;
pub const PROFILE_DEV_FEE : u64 = 1_000_000;
pub const PROFILE_SPIRITUAL_FEE : u64 = 1_000_000;
// REWARD_DEV_PCT — SUPPRIMEE (les 6 Cases gerent tout)
// REWARD_EMERGENCY_PCT — SUPPRIMEE (les 6 Cases gerent tout)
pub fn calculate_micro_burn(amount: u64) -> u64 {
    (amount * TX_FEE_BPS) / 10_000
}


