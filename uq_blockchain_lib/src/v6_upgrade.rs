use crate::error::ChainError;
use crate::bubble::BubbleRegistry;

// ═══════════════════════════════════════════════════════════════
// CONSTANTES V6
// ═══════════════════════════════════════════════════════════════

pub const PERSISTENCE_MAGIC: u32 = 0x55513031;
pub const PERSISTENCE_VERSION: u32 = 6;
pub const MAX_TREASURY_DURATION: u64 = 10 * 365 * 24 * 3600;
pub const TOTAL_SUPPLY_BORB: u64 = 900_000_000_000;
pub const MICRO_PER_BORB: u64 = 1_000_000;
pub const TX_FEE_RATE: u64 = 10;
pub const MAX_CONCENTRATION_BASIS_POINTS: u64 = 1000;

// ═══════════════════════════════════════════════════════════════
// P0-1 : MIGRATION
// ═══════════════════════════════════════════════════════════════

pub fn check_migration(current_version: u32) -> Result<(), ChainError> {
    if current_version < PERSISTENCE_VERSION {
        return Err(ChainError::InternalError(
            format!("Migration requise : v{} → v{}", current_version, PERSISTENCE_VERSION)
        ));
    }
    Ok(())
}

// ═══════════════════════════════════════════════════════════════
// P0-2 : INVARIANT TOTAL
// ═══════════════════════════════════════════════════════════════

pub fn verify_total_invariant(
    treasury: u64, fees: u64, rewards: u64,
    savings: u64, credit: u64, shield: u64,
    all_users_balance: u64,
) -> Result<bool, ChainError> {
    let cases_total = treasury + fees + rewards + savings + credit + shield;
    let grand_total = cases_total + all_users_balance;
    if grand_total != TOTAL_SUPPLY_BORB {
        return Err(ChainError::InternalError(
            format!("INVARIANT VIOLÉ : cases={} + users={} = {} (attendu {})",
                cases_total, all_users_balance, grand_total, TOTAL_SUPPLY_BORB)
        ));
    }
    Ok(true)
}

// ═══════════════════════════════════════════════════════════════
// P0-3 : ANTI-CONCENTRATION
// ═══════════════════════════════════════════════════════════════

pub fn is_concentration_warning(registry: &BubbleRegistry, address: &[u8; 32]) -> bool {
    let balance = registry.get_balance(address);
    let balance_BORB = balance / MICRO_PER_BORB;
    balance_BORB > (TOTAL_SUPPLY_BORB * MAX_CONCENTRATION_BASIS_POINTS / 10000)
}

pub fn get_effective_fee_rate(registry: &BubbleRegistry, sender: &[u8; 32]) -> u64 {
    if is_concentration_warning(registry, sender) { TX_FEE_RATE * 2 } else { TX_FEE_RATE }
}

// ═══════════════════════════════════════════════════════════════
// P0-4 : TRÉSORERIE PÉRISSABLE
// ═══════════════════════════════════════════════════════════════

pub fn is_treasury_expired(created_at: u64) -> bool {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    now > created_at + MAX_TREASURY_DURATION
}

// ═══════════════════════════════════════════════════════════════
// P0-5 : AccountState V6
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub enum AccountStateV6 {
    Active,
    LossDeclared { declared_at: u64, reclaim_deadline: u64, debt_amount_micro: u64 },
    Frozen { reason: String, frozen_at: u64 },
}

// ═══════════════════════════════════════════════════════════════
// P0-6 : DebtEntry V6
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DebtEntryV6 {
    pub original_owner: [u8; 32],
    pub amount_micro: u64,
    pub declared_at: u64,
    pub reclaim_deadline: u64,
    pub status: DebtStatusV6,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub enum DebtStatusV6 {
    Pending,
    Reclaimed { reclaimed_at: u64 },
    Expired,
}

// ═══════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test] fn test_migration_ok() { assert!(check_migration(6).is_ok()); }
    #[test] fn test_migration_old() { assert!(check_migration(3).is_err()); }

    #[test] fn test_invariant_ok() {
        assert!(verify_total_invariant(700_000_000_000,50_000_000_000,50_000_000_000,50_000_000_000,30_000_000_000,20_000_000_000,0).unwrap_or_default());
    }
    #[test] fn test_invariant_fail() {
        assert!(verify_total_invariant(700_000_000_000,50_000_000_000,50_000_000_000,50_000_000_000,30_000_000_000,20_000_000_000,1).is_err());
    }
    #[test] fn test_treasury_ok() {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
        assert!(!is_treasury_expired(now));
    }
    #[test] fn test_treasury_expired() { assert!(is_treasury_expired(0)); }
    #[test] fn test_constants() {
        assert_eq!(TOTAL_SUPPLY_BORB, 900_000_000_000);
        assert_eq!(PERSISTENCE_MAGIC, 0x55513031);
    }
}

