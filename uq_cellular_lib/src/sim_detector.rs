#[derive(Debug, Clone)]
pub struct SimStatus {
    pub has_sim: bool,
    pub has_data: bool,
    pub battery_pct: u8,
    pub total_bandwidth_kbps: u32,
}

pub struct SimDetector;

impl SimDetector {
    pub fn new() -> Self { SimDetector }
    pub fn detect(&self) -> SimStatus {
        SimStatus {
            has_sim: cfg!(target_os = "android"),
            has_data: cfg!(target_os = "android"),
            battery_pct: 85,
            total_bandwidth_kbps: 10000,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn test_detect() { let sd = SimDetector::new(); let s = sd.detect(); assert!(s.battery_pct > 0); }
}
