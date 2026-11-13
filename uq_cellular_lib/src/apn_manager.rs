use uq_blockchain::error::ChainError;

const APN_LIST: &[&str] = &[
    "internet", "uq", "uq1", "uq2", "uq3", "wap", "mms", "ims",
];

#[derive(Debug, Clone, PartialEq)]
pub enum ApnTestResult { Success, FailOpen, Timeout, Refused, Unknown }

#[derive(Debug, Clone, PartialEq)]
pub enum ApnState { Disconnected, Connecting, Connected, Failed, FailOpen }

pub struct ApnManager {
    current_apn_index: usize,
    fail_open_detected: bool,
    tests_performed: u32,
    state: ApnState,
    total_data_bytes: u64,
    last_test_result: Option<ApnTestResult>,
}

impl ApnManager {
    pub fn new() -> Self {
        ApnManager { current_apn_index: 0, fail_open_detected: false, tests_performed: 0, state: ApnState::Disconnected, total_data_bytes: 0, last_test_result: None }
    }

    pub fn is_apn_working(&self) -> bool { self.state == ApnState::Connected || self.state == ApnState::FailOpen }
    pub fn is_fail_open(&self) -> bool { self.fail_open_detected }
    pub fn is_connected(&self) -> bool { self.state == ApnState::Connected }

    pub fn rotate_apn(&mut self) -> &str {
        self.current_apn_index = (self.current_apn_index + 1) % APN_LIST.len();
        self.state = ApnState::Disconnected;
        self.get_current_apn()
    }

    pub fn rotate_apn_prev(&mut self) -> &str {
        if self.current_apn_index == 0 { self.current_apn_index = APN_LIST.len() - 1; }
        else { self.current_apn_index -= 1; }
        self.state = ApnState::Disconnected;
        self.get_current_apn()
    }

    pub fn get_current_apn(&self) -> &str { APN_LIST[self.current_apn_index] }
    pub fn get_current_index(&self) -> usize { self.current_apn_index }
    pub fn get_all_apns(&self) -> &[&str] { APN_LIST }
    pub fn get_apn_count(&self) -> usize { APN_LIST.len() }

    pub fn test_fail_open(&mut self) -> ApnTestResult {
        self.tests_performed += 1;
        let result = if self.fail_open_detected { ApnTestResult::FailOpen } else { ApnTestResult::Refused };
        self.last_test_result = Some(result.clone());
        if result == ApnTestResult::FailOpen { self.fail_open_detected = true; self.state = ApnState::FailOpen; }
        result
    }

    pub fn set_fail_open(&mut self, detected: bool) { self.fail_open_detected = detected; if detected { self.state = ApnState::FailOpen; } }

    pub fn force_apn(&mut self, apn: &str) -> Result<(), ChainError> {
        if let Some(pos) = APN_LIST.iter().position(|&a| a == apn) { self.current_apn_index = pos; self.state = ApnState::Disconnected; Ok(()) }
        else { Err(ChainError::InternalError(format!("APN inconnu: {}", apn))) }
    }

    pub fn force_apn_by_index(&mut self, index: usize) -> Result<(), ChainError> {
        if index < APN_LIST.len() { self.current_apn_index = index; self.state = ApnState::Disconnected; Ok(()) }
        else { Err(ChainError::InternalError(format!("Index APN invalide: {}", index))) }
    }

    pub fn connect(&mut self) -> Result<(), ChainError> { self.state = ApnState::Connected; Ok(()) }
    pub fn disconnect(&mut self) -> Result<(), ChainError> { self.state = ApnState::Disconnected; Ok(()) }
    pub fn add_data_usage(&mut self, bytes: u64) { self.total_data_bytes += bytes; }

    pub fn auto_discover(&mut self) -> Option<String> {
        let original = self.current_apn_index;
        for i in 0..APN_LIST.len() {
            self.current_apn_index = i;
            if self.is_apn_working() || self.test_fail_open() == ApnTestResult::FailOpen { return Some(self.get_current_apn().to_string()); }
        }
        self.current_apn_index = original;
        None
    }

    pub fn get_stats(&self) -> ApnStats {
        ApnStats { current_apn: self.get_current_apn().to_string(), current_index: self.current_apn_index, total_apns: APN_LIST.len(), state: format!("{:?}", self.state), fail_open: self.fail_open_detected, tests_performed: self.tests_performed, connected: self.is_connected(), total_data_mb: self.total_data_bytes as f64 / 1_048_576.0, last_test_result: format!("{:?}", self.last_test_result) }
    }

    pub fn get_state(&self) -> &ApnState { &self.state }
    pub fn is_uq_apn_available(&self) -> bool { let apn = self.get_current_apn(); (apn.starts_with("uq") || apn == "internet") && self.is_connected() }
}

#[derive(Debug, Clone)]
pub struct ApnStats { pub current_apn: String, pub current_index: usize, pub total_apns: usize, pub state: String, pub fail_open: bool, pub tests_performed: u32, pub connected: bool, pub total_data_mb: f64, pub last_test_result: String }

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn test_new() { let m = ApnManager::new(); assert_eq!(m.get_current_apn(), "internet"); assert!(!m.is_fail_open()); }
    #[test] fn test_rotate() { let mut m = ApnManager::new(); m.rotate_apn(); assert_eq!(m.get_current_apn(), "uq"); }
    #[test] fn test_rotate_wrap() { let mut m = ApnManager::new(); for _ in 0..8 { m.rotate_apn(); } assert_eq!(m.get_current_apn(), "internet"); }
    #[test] fn test_rotate_prev() { let mut m = ApnManager::new(); m.rotate_apn_prev(); assert_eq!(m.get_current_apn(), "ims"); }
    #[test] fn test_force_valid() { let mut m = ApnManager::new(); assert!(m.force_apn("uq2").is_ok()); }
    #[test] fn test_force_invalid() { let mut m = ApnManager::new(); assert!(m.force_apn("bad").is_err()); }
    #[test] fn test_force_index() { let mut m = ApnManager::new(); assert!(m.force_apn_by_index(3).is_ok()); assert!(m.force_apn_by_index(99).is_err()); }
    #[test] fn test_fail_open() { let mut m = ApnManager::new(); assert_eq!(m.test_fail_open(), ApnTestResult::Refused); }
    #[test] fn test_set_fail_open() { let mut m = ApnManager::new(); m.set_fail_open(true); assert!(m.is_fail_open()); }
    #[test] fn test_connect() { let mut m = ApnManager::new(); m.connect().unwrap(); assert!(m.is_connected()); }
    #[test] fn test_disconnect() { let mut m = ApnManager::new(); m.connect().unwrap(); m.disconnect().unwrap(); assert!(!m.is_connected()); }
    #[test] fn test_data() { let mut m = ApnManager::new(); m.add_data_usage(1048576); let s = m.get_stats(); assert!(s.total_data_mb > 0.9); }
    #[test] fn test_stats() { let m = ApnManager::new(); let s = m.get_stats(); assert_eq!(s.current_apn, "internet"); assert_eq!(s.total_apns, 8); }
    #[test] fn test_uq_available() { let mut m = ApnManager::new(); m.force_apn("uq").unwrap(); m.connect().unwrap(); assert!(m.is_uq_apn_available()); }
    #[test] fn test_auto_discover() { let mut m = ApnManager::new(); m.set_fail_open(true); assert!(m.auto_discover().is_some()); }
    #[test] fn test_index() { let mut m = ApnManager::new(); assert_eq!(m.get_current_index(), 0); m.rotate_apn(); assert_eq!(m.get_current_index(), 1); }
}

