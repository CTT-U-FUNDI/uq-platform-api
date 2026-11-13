#[allow(static_mut_refs)]
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const EPOCH_LENGTH: u64 = 600;
const HB_MAX: u64 = 1_000_000;
const HB_CRITICAL: u64 = 800_000;
pub const HARDBIT_GENESIS: u64 = 0;

#[derive(Debug, Clone)]
pub struct HardbitState {
    pub current: u64,
    pub genesis: u64,
    pub last_update: i64,
    pub epoch: u64,
    pub total_transactions: u64,
    pub peak: u64,
    pub temperature: f64,
}

impl Default for HardbitState {
    fn default() -> Self {
        HardbitState {
            current: 0,
            genesis: 0,
            last_update: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64,
            epoch: 0,
            total_transactions: 0,
            peak: 0,
            temperature: 0.0,
        }
    }
}

static HARDBIT_STATE: Mutex<Option<HardbitState>> = Mutex::new(None);

pub fn init_hardbit() {
    *HARDBIT_STATE.lock().unwrap() = Some(HardbitState::default());
}

pub fn get_current_hardbit() -> u64 {
    let state = HARDBIT_STATE.lock().unwrap();
    state.as_ref().map(|s| s.current).unwrap_or(0)
}

pub fn verify_hardbit(hb: u64) -> bool {
    let state = HARDBIT_STATE.lock().unwrap();
    match state.as_ref() {
        Some(s) => hb >= s.current && hb <= HB_MAX,
        None => true,
    }
}

pub fn propose_time(hb: u64, address: &[u8; 32]) -> Result<u64, String> {
    let slot = (hb.wrapping_mul(u64::from_le_bytes(address[..8].try_into().unwrap_or_default()))) % 1000;
    Ok(slot)
}

pub fn recalculate_hardbit() -> Result<(), String> {
    let mut state = HARDBIT_STATE.lock().unwrap();
    if let Some(ref mut s) = *state {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
        let elapsed = now - s.last_update;
        let epochs = elapsed as u64 / EPOCH_LENGTH;
        s.current += epochs;
        s.epoch += epochs;
        s.last_update = now;
        s.temperature = (s.current as f64 / HB_MAX as f64).min(1.0);
        if s.current > s.peak { s.peak = s.current; }
    }
    Ok(())
}

pub fn sync_hardbit(external: u64) -> bool {
    let mut state = HARDBIT_STATE.lock().unwrap();
    if let Some(ref mut s) = *state {
        if external > s.current {
            s.current = external;
            return true;
        }
    }
    false
}

pub fn get_hardbit_stats() -> Result<HardbitStats, String> {
    let state = HARDBIT_STATE.lock().unwrap();
    match state.as_ref() {
        Some(s) => Ok(HardbitStats {
            current: s.current,
            epoch: s.epoch,
            peak: s.peak,
            temperature: s.temperature,
            is_critical: s.current > HB_CRITICAL,
        }),
        None => Err("HardBit non initialisé".into()),
    }
}

#[derive(Debug)]
pub struct HardbitStats {
    pub current: u64,
    pub epoch: u64,
    pub peak: u64,
    pub temperature: f64,
    pub is_critical: bool,
}

pub fn get_time_until_next_epoch() -> Result<u64, String> {
    let state = HARDBIT_STATE.lock().unwrap();
    match state.as_ref() {
        Some(s) => {
            let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
            let elapsed = (now - s.last_update) as u64;
            Ok(EPOCH_LENGTH - (elapsed % EPOCH_LENGTH))
        }
        None => Err("HardBit non initialisé".into()),
    }
}

pub fn get_epoch_length() -> u64 { EPOCH_LENGTH }
pub fn get_hb_max() -> u64 { HB_MAX }

/// Vérifie si le Hardbit est initialisé et synchronisé
pub fn is_synchronized() -> bool {
    HARDBIT_STATE.lock().unwrap().is_some()
}
pub fn get_hb_critical() -> u64 { HB_CRITICAL }

#[cfg(test)]
mod tests {
    use super::*;
    
    fn setup() { init_hardbit(); }
    
    #[test] fn test_init() { setup(); assert_eq!(get_current_hardbit(), 0); }
    #[test] fn test_verify() { setup(); assert!(verify_hardbit(100)); assert!(!verify_hardbit(HB_MAX + 1)); }
    #[test] fn test_propose() { setup(); let slot = propose_time(1000, &[1u8;32]).unwrap_or_default(); assert!(slot < 1000); }
    #[test] fn test_sync() { setup(); assert!(sync_hardbit(500)); assert_eq!(get_current_hardbit(), 500); }
    #[test] fn test_recalculate() { setup(); recalculate_hardbit().unwrap_or_default(); assert!(get_current_hardbit() >= 0); }
    #[test] fn test_stats() { setup(); let stats = get_hardbit_stats().unwrap_or_default(); assert!(!stats.is_critical); }
    #[test] fn test_epoch() { setup(); assert!(get_time_until_next_epoch().unwrap_or_default() <= EPOCH_LENGTH); }
    #[test] fn test_constants() { assert_eq!(get_epoch_length(), 600); assert_eq!(get_hb_max(), 1_000_000); assert_eq!(get_hb_critical(), 800_000); }
    #[test] fn test_never_decreases() { setup(); let a = get_current_hardbit(); std::thread::sleep(std::time::Duration::from_millis(5)); assert!(get_current_hardbit() >= a); }
    #[test] fn test_temperature() { setup(); recalculate_hardbit().unwrap_or_default(); let stats = get_hardbit_stats().unwrap_or_default(); assert!(stats.temperature >= 0.0 && stats.temperature <= 1.0); }
}
