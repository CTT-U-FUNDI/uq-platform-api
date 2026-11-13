use uq_blockchain::error::ChainError;

pub struct RelayInternet {
    connected: bool,
    peer_address: Option<[u8; 32]>,
    daily_cost: u64,
    price_per_24h: u64,
}

impl RelayInternet {
    pub fn new(price_per_24h: u64) -> Self {
        RelayInternet { connected: false, peer_address: None, daily_cost: 0, price_per_24h }
    }
    pub fn connect(&mut self, addr: &[u8; 32], _hours: u64) -> Result<(), ChainError> {
        self.peer_address = Some(*addr);
        self.connected = true;
        self.daily_cost = self.price_per_24h;
        Ok(())
    }
    pub fn disconnect(&mut self) { self.connected = false; self.peer_address = None; }
    pub fn is_connected(&self) -> bool { self.connected }
    pub fn get_daily_cost(&self) -> u64 { self.daily_cost }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn test_connect() { let mut r = RelayInternet::new(10); assert!(r.connect(&[1u8;32], 24).is_ok()); assert!(r.is_connected()); }
    #[test] fn test_disconnect() { let mut r = RelayInternet::new(10); r.connect(&[1u8;32], 24).unwrap(); r.disconnect(); assert!(!r.is_connected()); }
}

