use uq_blockchain::error::ChainError;

pub trait RadioHardware {
    fn set_frequency(&mut self, freq_mhz: u32) -> Result<(), ChainError>;
    fn set_power(&mut self, dbm: i8) -> Result<(), ChainError>;
    fn start_tx(&mut self, data: &[u8], interval_ms: u64) -> Result<(), ChainError>;
    fn stop_tx(&mut self) -> Result<(), ChainError>;
    fn is_available(&self) -> bool;
}

pub trait NodeIdentity {
    fn get_address(&self) -> [u8; 32];
    fn sign(&self, data: &[u8]) -> Result<Vec<u8>, ChainError>;
}

#[derive(Debug, Clone, Default)]
pub struct PeerCapabilities {
    pub gpu_available: bool,
    pub storage_mb: u32,
    pub bandwidth_kbps: u32,
    pub battery_pct: u8,
}

pub struct BeaconEngine<H: RadioHardware, I: NodeIdentity> {
    hardware: H,
    identity: I,
    active: bool,
    beacons_sent: u64,
    power: i8,
    frequency_mhz: u32,
}

impl<H: RadioHardware, I: NodeIdentity> BeaconEngine<H, I> {
    pub fn new(hardware: H, identity: I) -> Self {
        BeaconEngine { hardware, identity, active: false, beacons_sent: 0, power: -40, frequency_mhz: 2442 }
    }

    pub fn start(&mut self, power: i8, freq: u32) -> Result<(), ChainError> {
        if !self.hardware.is_available() { return Err(ChainError::InternalError("Radio indisponible".into())); }
        self.hardware.set_frequency(freq)?;
        self.hardware.set_power(power)?;
        let payload = self.build_payload();
        self.hardware.start_tx(&payload, 1000)?;
        self.active = true; self.beacons_sent += 1; self.power = power; self.frequency_mhz = freq;
        Ok(())
    }

    pub fn stop(&mut self) -> Result<(), ChainError> { self.hardware.stop_tx()?; self.active = false; Ok(()) }
    pub fn is_active(&self) -> bool { self.active }

    fn build_payload(&self) -> Vec<u8> {
        let mut data = vec![0x90, 0x01]; // MCC=901
        data.push(0x01); // MNC=01
        data.extend_from_slice(&self.identity.get_address());
        data.extend_from_slice(&chrono::Utc::now().timestamp().to_le_bytes());
        data
    }

    pub fn get_beacons_sent(&self) -> u64 { self.beacons_sent }
    pub fn get_metrics(&self) -> BeaconMetrics {
        BeaconMetrics { active: self.active, beacons_sent: self.beacons_sent, power: self.power, frequency_mhz: self.frequency_mhz }
    }
}

#[derive(Debug)]
pub struct BeaconMetrics { pub active: bool, pub beacons_sent: u64, pub power: i8, pub frequency_mhz: u32 }

#[cfg(test)]
mod tests {
    use super::*;
    struct MockRadio { available: bool }
    impl MockRadio { fn new() -> Self { MockRadio { available: true } } }
    impl RadioHardware for MockRadio {
        fn set_frequency(&mut self, _f: u32) -> Result<(), ChainError> { Ok(()) }
        fn set_power(&mut self, _p: i8) -> Result<(), ChainError> { Ok(()) }
        fn start_tx(&mut self, _data: &[u8], _i: u64) -> Result<(), ChainError> { Ok(()) }
        fn stop_tx(&mut self) -> Result<(), ChainError> { Ok(()) }
        fn is_available(&self) -> bool { self.available }
    }
    struct MockId;
    impl NodeIdentity for MockId {
        fn get_address(&self) -> [u8; 32] { [1u8; 32] }
        fn sign(&self, _d: &[u8]) -> Result<Vec<u8>, ChainError> { Ok(vec![9u8; 64]) }
    }

    #[test] fn test_new() { let e = BeaconEngine::new(MockRadio::new(), MockId); assert!(!e.is_active()); }
    #[test] fn test_start_stop() { let mut e = BeaconEngine::new(MockRadio::new(), MockId); e.start(-40, 2442).unwrap(); assert!(e.is_active()); e.stop().unwrap(); assert!(!e.is_active()); }
    #[test] fn test_radio_unavailable() { let r = MockRadio { available: false }; let mut e = BeaconEngine::new(r, MockId); assert!(e.start(-40, 2442).is_err()); }
    #[test] fn test_count() { let mut e = BeaconEngine::new(MockRadio::new(), MockId); e.start(-40, 2442).unwrap(); e.stop().unwrap(); e.start(-40, 2442).unwrap(); assert_eq!(e.get_beacons_sent(), 2); }
    #[test] fn test_metrics() { let e = BeaconEngine::new(MockRadio::new(), MockId); let m = e.get_metrics(); assert!(!m.active); assert_eq!(m.power, -40); }
}

