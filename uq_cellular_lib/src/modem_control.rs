use uq_blockchain::error::ChainError;

pub trait ModemHardware {
    fn send_at_command(&self, cmd: &str) -> Result<String, ChainError>;
    fn get_signal_strength(&self) -> Result<i8, ChainError>;
    fn is_radio_on(&self) -> bool;
}

#[derive(Debug, Clone, PartialEq)]
pub enum ModemState { Off, Normal, TestMode, FlightMode, Emergency }

pub struct ModemController<H: ModemHardware> {
    hardware: H, state: ModemState, current_frequency_mhz: u32, current_power_dbm: i8, transmitting: bool,
}

impl<H: ModemHardware> ModemController<H> {
    pub fn new(hardware: H) -> Self {
        ModemController { hardware, state: ModemState::Off, current_frequency_mhz: 2442, current_power_dbm: -40, transmitting: false }
    }
    pub fn detect_state(&self) -> Result<ModemState, ChainError> {
        let r = self.hardware.send_at_command("AT+CFUN?")?;
        if r.contains("+CFUN: 0") { Ok(ModemState::FlightMode) }
        else if r.contains("+CFUN: 4") { Ok(ModemState::TestMode) }
        else if r.contains("+CFUN: 1") { Ok(ModemState::Normal) }
        else { Ok(ModemState::Off) }
    }
    pub fn enter_normal_mode(&mut self) -> Result<(), ChainError> { self.hardware.send_at_command("AT+CFUN=1")?; self.state = ModemState::Normal; Ok(()) }
    pub fn enter_test_mode(&mut self) -> Result<(), ChainError> { self.hardware.send_at_command("AT+CFUN=4")?; self.state = ModemState::TestMode; Ok(()) }
    pub fn reset(&mut self) -> Result<(), ChainError> { self.hardware.send_at_command("AT+CFUN=0")?; std::thread::sleep(std::time::Duration::from_secs(1)); self.hardware.send_at_command("AT+CFUN=1")?; self.state = ModemState::Normal; Ok(()) }
    pub fn set_frequency(&mut self, freq_mhz: u32) -> Result<(), ChainError> {
        if !matches!(freq_mhz, 2400..=2483 | 5150..=5875 | 863..=870 | 902..=928) { return Err(ChainError::InternalError("Fréquence non supportée".into())); }
        self.hardware.send_at_command("AT*BAND=ISM")?; self.current_frequency_mhz = freq_mhz; Ok(())
    }
    pub fn set_power(&mut self, dbm: i8) -> Result<(), ChainError> {
        if dbm < -80 || dbm > 0 { return Err(ChainError::InternalError("Puissance [-80,0] dBm".into())); }
        self.hardware.send_at_command(&format!("AT*TXGAIN={}", dbm))?; self.current_power_dbm = dbm; Ok(())
    }
    pub fn start_tx(&mut self, _data: &[u8], _interval_ms: u64) -> Result<(), ChainError> { self.transmitting = true; Ok(()) }
    pub fn stop_tx(&mut self) -> Result<(), ChainError> { self.transmitting = false; Ok(()) }
    pub fn activate_pdp(&self, cid: u8, apn: &str) -> Result<(), ChainError> { self.hardware.send_at_command(&format!("AT+CGDCONT={},\"IP\",\"{}\"", cid, apn))?; self.hardware.send_at_command(&format!("AT+CGACT={},1", cid))?; Ok(()) }
    pub fn get_signal(&self) -> Result<i8, ChainError> { self.hardware.get_signal_strength() }
    pub fn is_ready(&self) -> bool { self.state != ModemState::Off && self.hardware.is_radio_on() }
    pub fn get_state(&self) -> ModemState { self.state.clone() }
    pub fn get_metrics(&self) -> ModemMetrics { ModemMetrics { state: format!("{:?}", self.state), frequency_mhz: self.current_frequency_mhz, power_dbm: self.current_power_dbm, transmitting: self.transmitting, signal_dbm: self.get_signal().unwrap_or(-113), ready: self.is_ready() } }
}

#[derive(Debug, Clone)]
pub struct ModemMetrics { pub state: String, pub frequency_mhz: u32, pub power_dbm: i8, pub transmitting: bool, pub signal_dbm: i8, pub ready: bool }

#[cfg(test)]
mod tests {
    use super::*;
    struct MockModem;
    impl ModemHardware for MockModem {
        fn send_at_command(&self, cmd: &str) -> Result<String, ChainError> {
            if cmd == "AT+CFUN?" { return Ok("+CFUN: 1".into()); }
            Ok("OK".into())
        }
        fn get_signal_strength(&self) -> Result<i8, ChainError> { Ok(-65) }
        fn is_radio_on(&self) -> bool { true }
    }
    #[test] fn test_detect() { let m = ModemController::new(MockModem); assert_eq!(m.detect_state().unwrap(), ModemState::Normal); }
    #[test] fn test_test_mode() { let mut m = ModemController::new(MockModem); m.enter_test_mode().unwrap(); assert_eq!(m.state, ModemState::TestMode); }
    #[test] fn test_reset() { let mut m = ModemController::new(MockModem); m.reset().unwrap(); assert_eq!(m.state, ModemState::Normal); }
    #[test] fn test_frequency() { let mut m = ModemController::new(MockModem); assert!(m.set_frequency(2442).is_ok()); assert!(m.set_frequency(9999).is_err()); }
    #[test] fn test_power() { let mut m = ModemController::new(MockModem); assert!(m.set_power(-40).is_ok()); assert!(m.set_power(-100).is_err()); }
    #[test] fn test_pdp() { let m = ModemController::new(MockModem); assert!(m.activate_pdp(99, "uq").is_ok()); }
    #[test] fn test_metrics_off() { let m = ModemController::new(MockModem); assert!(!m.get_metrics().ready); }
}

