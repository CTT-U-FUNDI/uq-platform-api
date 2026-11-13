pub struct BootManager {
    pub started: bool,
}

impl BootManager {
    pub fn new() -> Self { BootManager { started: false } }
    pub fn start(&mut self) -> Result<(), String> { self.started = true; Ok(()) }
    pub fn stop(&mut self) -> Result<(), String> { self.started = false; Ok(()) }
    pub fn is_running(&self) -> bool { self.started }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test] fn test_new() { let b = BootManager::new(); assert!(!b.is_running()); }
    #[test] fn test_start_stop() { let mut b = BootManager::new(); b.start().unwrap_or_default(); assert!(b.is_running()); b.stop().unwrap_or_default(); assert!(!b.is_running()); }
}
