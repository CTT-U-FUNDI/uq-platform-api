use crate::error::UqError;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct P2PConnection {
    pub address: String,
    pub public_key: [u8; 32],
    pub established: bool,
    pub latency_ms: u64,
}

#[derive(Debug)]
pub struct UqHolePuncher {
    connections: HashMap<String, P2PConnection>,
    is_listening: bool,
}

impl UqHolePuncher {
    pub fn new() -> Self {
        UqHolePuncher { connections: HashMap::new(), is_listening: false }
    }

    pub fn start_listening(&mut self) -> Result<(), UqError> {
        if self.is_listening { return Err(UqError::AlreadyInitialized); }
        self.is_listening = true;
        Ok(())
    }

    pub fn stop_listening(&mut self) { self.is_listening = false; }

    pub fn request_connection(&mut self, address: &str, public_key: &[u8; 32]) -> Result<(), UqError> {
        if !self.is_listening { return Err(UqError::NotInitialized("Not listening".into())); }
        if self.connections.contains_key(address) { return Ok(()); }
        self.connections.insert(address.to_string(), P2PConnection {
            address: address.to_string(), public_key: *public_key, established: true, latency_ms: 50,
        });
        Ok(())
    }

    pub fn get_connection(&self, address: &str) -> Option<&P2PConnection> { self.connections.get(address) }
    pub fn close_connection(&mut self, address: &str) { self.connections.remove(address); }
    pub fn get_connection_count(&self) -> usize { self.connections.len() }
    pub fn is_available(&self) -> bool { self.is_listening }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::generate_keypair;

    #[test]
    fn test_new() {
        let hp = UqHolePuncher::new();
        assert!(!hp.is_available());
        assert_eq!(hp.get_connection_count(), 0);
    }

    #[test]
    fn test_start_stop() {
        let mut hp = UqHolePuncher::new();
        hp.start_listening().unwrap_or_default();
        assert!(hp.is_available());
        hp.stop_listening();
        assert!(!hp.is_available());
    }

    #[test]
    fn test_connection() {
        let mut hp = UqHolePuncher::new();
        hp.start_listening().unwrap_or_default();
        let kp = generate_keypair();
        hp.request_connection("uq:peer1", &kp.public_key).unwrap_or_default();
        assert_eq!(hp.get_connection_count(), 1);
        assert!(hp.get_connection("uq:peer1").is_some());
    }

    #[test]
    fn test_close() {
        let mut hp = UqHolePuncher::new();
        hp.start_listening().unwrap_or_default();
        let kp = generate_keypair();
        hp.request_connection("uq:peer2", &kp.public_key).unwrap_or_default();
        hp.close_connection("uq:peer2");
        assert_eq!(hp.get_connection_count(), 0);
    }

    #[test]
    fn test_requires_listening() {
        let mut hp = UqHolePuncher::new();
        let kp = generate_keypair();
        assert!(hp.request_connection("uq:peer3", &kp.public_key).is_err());
    }
}
