use uq_blockchain::error::ChainError;
use super::d2d::D2dMode;

/// Un pair D2D découvert
#[derive(Debug, Clone)]
pub struct D2dPeer {
    pub address: [u8; 32],
    pub name: String,
    pub mode: D2dMode,
    pub signal_dbm: i8,
    pub distance_estimated_m: f64,
    pub discovered_at: u64,
    pub last_seen: u64,
    pub is_verified: bool,
    pub capabilities: PeerCapabilities,
}

#[derive(Debug, Clone)]
pub struct PeerCapabilities {
    pub can_relay_internet: bool,
    pub has_sim_data: bool,
    pub battery_pct: u8,
    pub max_bandwidth_kbps: u32,
}

impl D2dPeer {
    pub fn new(address: [u8; 32], name: &str, mode: D2dMode, signal_dbm: i8) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        
        D2dPeer {
            address,
            name: name.to_string(),
            mode,
            signal_dbm,
            distance_estimated_m: D2dPeer::estimate_distance(signal_dbm),
            discovered_at: now,
            last_seen: now,
            is_verified: false,
            capabilities: PeerCapabilities {
                can_relay_internet: false,
                has_sim_data: false,
                battery_pct: 100,
                max_bandwidth_kbps: 0,
            },
        }
    }

    fn estimate_distance(rssi_dbm: i8) -> f64 {
        let tx_power = -40.0;
        10.0_f64.powf((tx_power - rssi_dbm as f64) / 20.0)
    }

    pub fn is_recent(&self, max_age_secs: u64) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        now - self.last_seen < max_age_secs
    }

    pub fn update_seen(&mut self) {
        self.last_seen = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
    }
}

/// Gestionnaire de découverte D2D
pub struct D2dDiscovery {
    peers: Vec<D2dPeer>,
    scan_active: bool,
    scan_interval_secs: u64,
    max_peers: usize,
    total_scans: u64,
    total_discoveries: u64,
}

impl D2dDiscovery {
    pub fn new(max_peers: usize, scan_interval_secs: u64) -> Self {
        D2dDiscovery {
            peers: Vec::new(),
            scan_active: false,
            scan_interval_secs,
            max_peers,
            total_scans: 0,
            total_discoveries: 0,
        }
    }

    /// Démarre le scan D2D
    pub fn start_scan(&mut self) -> Result<(), ChainError> {
        self.scan_active = true;
        self.total_scans += 1;
        Ok(())
    }

    /// Arrête le scan D2D
    pub fn stop_scan(&mut self) {
        self.scan_active = false;
    }

    /// Ajoute ou met à jour un pair découvert
    pub fn add_peer(&mut self, address: [u8; 32], name: &str, mode: D2dMode, signal_dbm: i8) {
        if let Some(existing) = self.peers.iter_mut().find(|p| p.address == address) {
            existing.signal_dbm = signal_dbm;
            existing.distance_estimated_m = D2dPeer::estimate_distance(signal_dbm);
            existing.update_seen();
            return;
        }

        if self.peers.len() >= self.max_peers {
            self.cleanup_oldest();
        }

        let peer = D2dPeer::new(address, name, mode, signal_dbm);
        self.peers.push(peer);
        self.total_discoveries += 1;
    }

    /// Retire un pair
    pub fn remove_peer(&mut self, address: &[u8; 32]) -> bool {
        let before = self.peers.len();
        self.peers.retain(|p| &p.address != address);
        self.peers.len() < before
    }

    /// Nettoie les pairs trop anciens
    pub fn cleanup_stale(&mut self, max_age_secs: u64) -> usize {
        let before = self.peers.len();
        self.peers.retain(|p| p.is_recent(max_age_secs));
        before - self.peers.len()
    }

    /// Supprime le plus ancien
    fn cleanup_oldest(&mut self) {
        if let Some(idx) = self.peers.iter()
            .enumerate()
            .min_by_key(|(_, p)| p.last_seen)
            .map(|(i, _)| i)
        {
            self.peers.remove(idx);
        }
    }

    /// Retourne les pairs triés par force de signal
    pub fn get_peers_by_signal(&self) -> Vec<&D2dPeer> {
        let mut sorted: Vec<&D2dPeer> = self.peers.iter().collect();
        sorted.sort_by(|a, b| b.signal_dbm.cmp(&a.signal_dbm));
        sorted
    }

    /// Retourne les pairs avec capacité de relais Internet
    pub fn get_relay_capable_peers(&self) -> Vec<&D2dPeer> {
        self.peers.iter()
            .filter(|p| p.capabilities.can_relay_internet && p.capabilities.has_sim_data)
            .collect()
    }

    /// Retourne le meilleur pair (signal le plus fort)
    pub fn get_best_peer(&self) -> Option<&D2dPeer> {
        self.peers.iter().max_by_key(|p| p.signal_dbm)
    }

    /// Vérifie si le scan est actif
    pub fn is_scanning(&self) -> bool {
        self.scan_active
    }

    /// Retourne le nombre de pairs
    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }

    /// Retourne les statistiques
    pub fn get_stats(&self) -> DiscoveryStats {
        DiscoveryStats {
            scan_active: self.scan_active,
            peer_count: self.peers.len(),
            max_peers: self.max_peers,
            total_scans: self.total_scans,
            total_discoveries: self.total_discoveries,
            relay_capable_count: self.get_relay_capable_peers().len(),
            best_signal_dbm: self.get_best_peer().map(|p| p.signal_dbm).unwrap_or(-120),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DiscoveryStats {
    pub scan_active: bool,
    pub peer_count: usize,
    pub max_peers: usize,
    pub total_scans: u64,
    pub total_discoveries: u64,
    pub relay_capable_count: usize,
    pub best_signal_dbm: i8,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_addr(n: u8) -> [u8; 32] { let mut a = [0u8; 32]; a[0] = n; a }

    #[test] fn test_peer_new() {
        let peer = D2dPeer::new(make_addr(1), "Test", D2dMode::WiFiDirect, -40);
        assert_eq!(peer.name, "Test");
        assert!(!peer.is_verified);
    }

    #[test] fn test_peer_distance() {
        let near = D2dPeer::new(make_addr(1), "Near", D2dMode::WiFiDirect, -20);
        let far = D2dPeer::new(make_addr(2), "Far", D2dMode::WiFiDirect, -80);
        assert!(near.distance_estimated_m < far.distance_estimated_m);
    }

    #[test] fn test_peer_is_recent() {
        let peer = D2dPeer::new(make_addr(1), "Test", D2dMode::WiFiDirect, -40);
        assert!(peer.is_recent(3600));
    }

    #[test] fn test_discovery_new() {
        let dd = D2dDiscovery::new(50, 30);
        assert!(!dd.is_scanning());
        assert_eq!(dd.peer_count(), 0);
    }

    #[test] fn test_add_peer() {
        let mut dd = D2dDiscovery::new(50, 30);
        dd.add_peer(make_addr(1), "A", D2dMode::WiFiDirect, -40);
        assert_eq!(dd.peer_count(), 1);
    }

    #[test] fn test_add_duplicate_updates() {
        let mut dd = D2dDiscovery::new(50, 30);
        dd.add_peer(make_addr(1), "A", D2dMode::WiFiDirect, -40);
        dd.add_peer(make_addr(1), "A", D2dMode::WiFiDirect, -30);
        assert_eq!(dd.peer_count(), 1);
        assert_eq!(dd.get_best_peer().unwrap().signal_dbm, -30);
    }

    #[test] fn test_remove_peer() {
        let mut dd = D2dDiscovery::new(50, 30);
        dd.add_peer(make_addr(1), "A", D2dMode::WiFiDirect, -40);
        assert!(dd.remove_peer(&make_addr(1)));
        assert_eq!(dd.peer_count(), 0);
    }

    #[test] fn test_start_stop_scan() {
        let mut dd = D2dDiscovery::new(50, 30);
        assert!(dd.start_scan().is_ok());
        assert!(dd.is_scanning());
        dd.stop_scan();
        assert!(!dd.is_scanning());
    }

    #[test] fn test_stats() {
        let mut dd = D2dDiscovery::new(50, 30);
        dd.add_peer(make_addr(1), "A", D2dMode::WiFiDirect, -40);
        dd.start_scan().unwrap();
        let stats = dd.get_stats();
        assert!(stats.scan_active);
        assert_eq!(stats.peer_count, 1);
        assert_eq!(stats.total_scans, 1);
    }
}

