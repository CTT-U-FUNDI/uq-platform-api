
#[derive(Debug, Clone)]
pub struct CellularConfig {
    pub scan_interval_secs: u64,
    pub internet_relay_price_micro: u64,
    pub max_relay_clients: u8,
    pub relay_bandwidth_limit_kbps: u32,
    pub auto_pilot_enabled: bool,
    pub battery_saver_threshold: u8,
}

impl Default for CellularConfig {
    fn default() -> Self {
        CellularConfig {
            scan_interval_secs: 30,
            internet_relay_price_micro: 10,
            max_relay_clients: 5,
            relay_bandwidth_limit_kbps: 1000,
            auto_pilot_enabled: true,
            battery_saver_threshold: 20,
        }
    }
}

impl CellularConfig {
    pub fn new() -> Self { Self::default() }
    pub fn with_prices(price_micro: u64) -> Self {
        CellularConfig { internet_relay_price_micro: price_micro, ..Self::default() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn test_default() { let c = CellularConfig::default(); assert_eq!(c.scan_interval_secs, 30); }
    #[test] fn test_price() { let c = CellularConfig::with_prices(20); assert_eq!(c.internet_relay_price_micro, 20); }
}
