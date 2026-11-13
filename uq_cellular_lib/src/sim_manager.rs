use uq_blockchain::error::ChainError;

pub trait SimHardware {
    fn send_apdu(&self, command: &[u8]) -> Result<ApduResponse, ChainError>;
    fn is_sim_present(&self) -> bool;
    fn get_imsi(&self) -> Result<String, ChainError>;
    fn get_iccid(&self) -> Result<String, ChainError>;
}

pub trait ModemAtCommands {
    fn send_at(&self, command: &str) -> Result<String, ChainError>;
}

#[derive(Debug, Clone)]
pub struct ApduResponse { pub data: Vec<u8>, pub sw1: u8, pub sw2: u8 }
impl ApduResponse {
    pub fn is_success(&self) -> bool { self.sw1 == 0x90 && self.sw2 == 0x00 }
}

pub struct SimManager<H: SimHardware, M: ModemAtCommands> {
    sim: H, modem: M, configured: bool, uq_service_activated: bool, current_apn: String,
}

impl<H: SimHardware, M: ModemAtCommands> SimManager<H, M> {
    pub fn new(sim: H, modem: M) -> Self { SimManager { sim, modem, configured: false, uq_service_activated: false, current_apn: "uq".into() } }
    pub fn is_ready(&self) -> bool { self.sim.is_sim_present() && self.configured }

    pub fn configure(&mut self) -> Result<bool, ChainError> {
        if !self.sim.is_sim_present() { return Ok(false); }
        self.sim.send_apdu(&[0x00, 0xA4, 0x00, 0x00, 0x02, 0x3F, 0x00])?;
        let ust = self.read_binary(&[0x6F, 0x38])?;
        if self.is_service_active(&ust, 99) { self.uq_service_activated = true; self.configured = true; return Ok(true); }
        let new_ust = self.activate_service(&ust, 99);
        self.update_binary(&[0x6F, 0x38], &new_ust)?;
        self.modem.send_at("AT+CGDCONT=99,\"IP\",\"uq\"")?;
        self.modem.send_at("AT+CGACT=99,1")?;
        self.uq_service_activated = true; self.configured = true; Ok(true)
    }

    fn read_binary(&self, _path: &[u8]) -> Result<Vec<u8>, ChainError> {
        let r = self.sim.send_apdu(&[0x00, 0xB0, 0x00, 0x00, 0x00])?;
        if r.is_success() { Ok(r.data) } else { Err(ChainError::InternalError("Lecture SIM".into())) }
    }
    fn update_binary(&self, _path: &[u8], data: &[u8]) -> Result<(), ChainError> {
        let mut cmd = vec![0x00, 0xD6, 0x00, 0x00]; cmd.push(data.len() as u8); cmd.extend_from_slice(data);
        let r = self.sim.send_apdu(&cmd)?; if r.is_success() { Ok(()) } else { Err(ChainError::InternalError("Écriture SIM".into())) }
    }
    fn is_service_active(&self, ust: &[u8], s: u8) -> bool {
        let b = (s/8) as usize; b < ust.len() && (ust[b] & (1 << (s%8))) != 0
    }
    fn activate_service(&self, ust: &[u8], s: u8) -> Vec<u8> {
        let mut u = ust.to_vec(); let b = (s/8) as usize; if b >= u.len() { u.resize(b+1, 0); } u[b] |= 1 << (s%8); u
    }

    pub fn deconfigure(&mut self) -> Result<(), ChainError> {
        self.modem.send_at("AT+CGACT=99,0")?;
        let ust = self.read_binary(&[0x6F, 0x38])?;
        let mut u = ust.clone(); let b = (99/8) as usize; if b < u.len() { u[b] &= !(1 << (99%8)); }
        self.update_binary(&[0x6F, 0x38], &u)?; self.configured = false; Ok(())
    }
    pub fn get_sim_info(&self) -> Result<SimInfo, ChainError> {
        Ok(SimInfo { present: self.sim.is_sim_present(), configured: self.configured, uq_ready: self.is_ready(), imsi: self.sim.get_imsi().unwrap(), iccid: self.sim.get_iccid().unwrap(), apn: self.current_apn.clone() })
    }
}

#[derive(Debug, Clone)]
pub struct SimInfo { pub present: bool, pub configured: bool, pub uq_ready: bool, pub imsi: String, pub iccid: String, pub apn: String }

#[cfg(test)]
mod tests {
    use super::*;
    struct MockSim { present: bool, ust: RefCell<Vec<u8>>, imsi: String }
    impl MockSim { fn new() -> Self { let mut u = vec![0u8;13]; u[0]=0b00000111; MockSim{present:true,ust:RefCell::new(u),imsi:"901010000000001".into()} } }
    impl SimHardware for MockSim {
        fn send_apdu(&self, cmd: &[u8]) -> Result<ApduResponse, ChainError> {
            if cmd.len()>=5 && cmd[1]==0xA4 { return Ok(ApduResponse{data:vec![],sw1:0x90,sw2:0x00}); }
            if cmd[1]==0xB0 { return Ok(ApduResponse{data:self.ust.borrow().clone(),sw1:0x90,sw2:0x00}); }
            if cmd[1]==0xD6 { let l=cmd[4] as usize; self.ust.replace(cmd[5..5+l].to_vec()); return Ok(ApduResponse{data:vec![],sw1:0x90,sw2:0x00}); }
            Ok(ApduResponse{data:vec![],sw1:0x90,sw2:0x00})
        }
        fn is_sim_present(&self) -> bool { self.present }
        fn get_imsi(&self) -> Result<String, ChainError> { Ok(self.imsi.clone()) }
        fn get_iccid(&self) -> Result<String, ChainError> { Ok("8901000000000000001".into()) }
    }
    struct MockModem;
    impl ModemAtCommands for MockModem { fn send_at(&self, _c: &str) -> Result<String, ChainError> { Ok("OK".into()) } }
    fn mgr() -> SimManager<MockSim, MockModem> { SimManager::new(MockSim::new(), MockModem) }

    #[test] fn test_new() { let m = mgr(); assert!(!m.configured); }
    #[test] fn test_configure() { let mut m = mgr(); assert!(m.configure().unwrap()); assert!(m.configured); }
    #[test] fn test_double() { let mut m = mgr(); m.configure().unwrap(); assert!(m.configure().unwrap()); }
    #[test] fn test_deconfigure() { let mut m = mgr(); m.configure().unwrap(); m.deconfigure().unwrap(); assert!(!m.configured); }
    #[test] fn test_is_ready() { let mut m = mgr(); assert!(!m.is_ready()); m.configure().unwrap(); assert!(m.is_ready()); }
    #[test] fn test_info() { let mut m = mgr(); m.configure().unwrap(); let i = m.get_sim_info().unwrap(); assert!(i.uq_ready); }
    #[test] fn test_no_sim() { let s = MockSim { present: false, ust: RefCell::new(vec![]), imsi: String::new() }; let mut m = SimManager::new(s, MockModem); assert!(!m.configure().unwrap()); }
}

