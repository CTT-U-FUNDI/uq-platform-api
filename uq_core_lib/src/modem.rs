//! UQ Modem Module - Émetteur/Récepteur unifié

use crate::hole_puncher::UqHolePuncher;
use crate::mimicry::UqQuicMimicry;
use crate::preamble::UqPreamble;
use crate::error::UqError;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct PeerInfo {
    pub address: String,
    pub public_key: [u8; 32],
    pub connected: bool,
}

#[derive(Debug)]
pub struct UqModem {
    hole_puncher: UqHolePuncher,
    mimicry: Option<UqQuicMimicry>,
    preamble: Option<UqPreamble>,
    peers: HashMap<String, PeerInfo>,
    is_running: bool,
}

impl UqModem {
    pub fn new() -> Self {
        UqModem {
            hole_puncher: UqHolePuncher::new(),
            mimicry: None,
            preamble: None,
            peers: HashMap::new(),
            is_running: false,
        }
    }

    pub fn set_mimicry(&mut self, mimicry: UqQuicMimicry) { self.mimicry = Some(mimicry); }
    pub fn set_preamble(&mut self, preamble: UqPreamble) { self.preamble = Some(preamble); }

    pub fn start(&mut self) -> Result<(), UqError> {
        if self.is_running { return Err(UqError::AlreadyInitialized); }
        self.hole_puncher.start_listening()?;
        self.is_running = true;
        Ok(())
    }

    pub fn stop(&mut self) {
        self.hole_puncher.stop_listening();
        self.is_running = false;
    }

    pub fn register_peer(&mut self, address: &str, public_key: &[u8; 32]) {
        self.peers.insert(address.to_string(), PeerInfo {
            address: address.to_string(), public_key: *public_key, connected: false,
        });
    }

    pub fn connect_to_peer(&mut self, address: &str) -> Result<(), UqError> {
        if let Some(peer) = self.peers.get(address) {
            let pk = peer.public_key;
            self.hole_puncher.request_connection(address, &pk)?;
            if let Some(p) = self.peers.get_mut(address) { p.connected = true; }
        }
        Ok(())
    }

    pub fn send_direct(&mut self, address: &str, data: &[u8]) -> Result<(), UqError> {
        if !self.is_running { return Err(UqError::NotInitialized("Modem arrêté".into())); }
        if !self.peers.contains_key(address) { return Err(UqError::HostNotFound(address.into())); }
        let _disguised = if let Some(ref mut m) = self.mimicry { m.disguise_packet(data) } else { data.to_vec() };
        Ok(())
    }

    pub fn get_connected_peers(&self) -> Vec<&PeerInfo> {
        self.peers.values().filter(|p| p.connected).collect()
    }

    pub fn get_peer_count(&self) -> usize { self.peers.len() }
    pub fn is_running(&self) -> bool { self.is_running }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::generate_keypair;

    #[test]
    fn test_start_stop() {
        let mut m = UqModem::new();
        assert!(!m.is_running());
        m.start().unwrap_or_default();
        assert!(m.is_running());
        m.stop();
        assert!(!m.is_running());
    }

    #[test]
    fn test_register_peer() {
        let mut m = UqModem::new();
        let kp = generate_keypair();
        m.register_peer("uq:peer1", &kp.public_key);
        assert_eq!(m.get_peer_count(), 1);
    }

    #[test]
    fn test_connect_peer() {
        let mut m = UqModem::new();
        m.start().unwrap_or_default();
        let kp = generate_keypair();
        m.register_peer("uq:peer2", &kp.public_key);
        m.connect_to_peer("uq:peer2").unwrap_or_default();
        assert_eq!(m.get_connected_peers().len(), 1);
    }
}
