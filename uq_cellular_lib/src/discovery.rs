use uq_blockchain::error::ChainError;

// ═══════════════════════════════════════════════════════════════
// TYPES DE PAIRS
// ═══════════════════════════════════════════════════════════════

/// Informations sur un pair découvert
#[derive(Debug, Clone)]
pub struct DiscoveredPeer {
    /// Adresse unique
    pub address: [u8; 32],
    /// Nom lisible
    pub name: String,
    /// Force du signal (dBm)
    pub signal_dbm: i8,
    /// Mode de transport disponible
    pub transport_mode: String,
    /// Horodatage de découverte
    pub discovered_at: u64,
    /// Dernière fois vu
    pub last_seen: u64,
    /// Nombre de fois découvert
    pub discovery_count: u32,
    /// Pair vérifié (handshake réussi)
    pub verified: bool,
    /// Distance estimée (mètres)
    pub estimated_distance_m: f64,
}

impl DiscoveredPeer {
    /// Crée un nouveau pair
    pub fn new(address: [u8; 32], name: &str, signal_dbm: i8, transport: &str) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        DiscoveredPeer {
            address,
            name: name.to_string(),
            signal_dbm,
            transport_mode: transport.to_string(),
            discovered_at: now,
            last_seen: now,
            discovery_count: 1,
            verified: false,
            estimated_distance_m: DiscoveredPeer::estimate_distance(signal_dbm),
        }
    }

    /// Estime la distance basée sur le RSSI (formule simple)
    fn estimate_distance(rssi_dbm: i8) -> f64 {
        let tx_power = -40.0; // Puissance d'émission supposée
        10.0_f64.powf((tx_power - rssi_dbm as f64) / 20.0)
    }

    /// Met à jour le timestamp last_seen
    pub fn update_last_seen(&mut self) {
        self.last_seen = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        self.discovery_count += 1;
    }

    /// Vérifie si le pair est récent (vu dans les X secondes)
    pub fn is_recent(&self, max_age_secs: u64) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        now - self.last_seen < max_age_secs
    }
}

// ═══════════════════════════════════════════════════════════════
// MODE DE DÉCOUVERTE
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq)]
pub enum DiscoveryMode {
    /// Scan passif (écoute uniquement)
    Passive,
    /// Scan actif (envoie des probes)
    Active,
    /// Mode hybride
    Hybrid,
    /// Désactivé
    Off,
}

// ═══════════════════════════════════════════════════════════════
// GESTIONNAIRE DE DÉCOUVERTE
// ═══════════════════════════════════════════════════════════════

pub struct DiscoveryManager {
    /// Pairs découverts
    peers: Vec<DiscoveredPeer>,
    /// Mode actuel
    mode: DiscoveryMode,
    /// Nombre de scans effectués
    scan_count: u64,
    /// Durée totale de scan (secondes)
    total_scan_time_s: u64,
    /// Intervalle entre scans (secondes)
    scan_interval_s: u64,
    /// Limite de pairs stockés
    max_peers: usize,
}

impl DiscoveryManager {
    /// Crée un nouveau gestionnaire
    pub fn new(max_peers: usize, scan_interval_s: u64) -> Self {
        DiscoveryManager {
            peers: Vec::new(),
            mode: DiscoveryMode::Passive,
            scan_count: 0,
            total_scan_time_s: 0,
            scan_interval_s,
            max_peers,
        }
    }

    /// Ajoute un pair découvert
    pub fn add_peer(&mut self, address: [u8; 32], name: &str, signal_dbm: i8, transport: &str) {
        // Vérifier si le pair existe déjà
        if let Some(existing) = self.peers.iter_mut().find(|p| p.address == address) {
            existing.update_last_seen();
            existing.signal_dbm = signal_dbm;
            return;
        }

        // Supprimer les plus anciens si limite atteinte
        if self.peers.len() >= self.max_peers {
            self.cleanup_oldest();
        }

        let peer = DiscoveredPeer::new(address, name, signal_dbm, transport);
        self.peers.push(peer);
    }

    /// Retire un pair
    pub fn remove_peer(&mut self, address: &[u8; 32]) -> bool {
        let len_before = self.peers.len();
        self.peers.retain(|p| &p.address != address);
        self.peers.len() < len_before
    }

    /// Supprime les pairs trop anciens
    pub fn cleanup_stale(&mut self, max_age_secs: u64) -> usize {
        let before = self.peers.len();
        self.peers.retain(|p| p.is_recent(max_age_secs));
        before - self.peers.len()
    }

    /// Supprime le plus ancien pair
    fn cleanup_oldest(&mut self) {
        if let Some(oldest_idx) = self.peers.iter()
            .enumerate()
            .min_by_key(|(_, p)| p.last_seen)
            .map(|(i, _)| i)
        {
            self.peers.remove(oldest_idx);
        }
    }

    /// Définit le mode de découverte
    pub fn set_mode(&mut self, mode: DiscoveryMode) {
        self.mode = mode;
    }

    /// Retourne le mode actuel
    pub fn get_mode(&self) -> &DiscoveryMode {
        &self.mode
    }

    /// Exécute un scan (simulé)
    pub fn scan(&mut self) -> Result<usize, ChainError> {
        if self.mode == DiscoveryMode::Off {
            return Err(ChainError::InternalError("Découverte désactivée".into()));
        }

        self.scan_count += 1;
        self.total_scan_time_s += self.scan_interval_s;

        // En mode passif, moins de découvertes
        // En mode actif, plus de découvertes mais plus d'énergie

        Ok(self.peers.len())
    }

    /// Retourne les pairs triés par force de signal
    pub fn get_peers_by_signal(&self) -> Vec<&DiscoveredPeer> {
        let mut sorted: Vec<&DiscoveredPeer> = self.peers.iter().collect();
        sorted.sort_by(|a, b| b.signal_dbm.cmp(&a.signal_dbm));
        sorted
    }

    /// Retourne les pairs récents
    pub fn get_recent_peers(&self, max_age_secs: u64) -> Vec<&DiscoveredPeer> {
        self.peers.iter().filter(|p| p.is_recent(max_age_secs)).collect()
    }

    /// Retourne les pairs vérifiés
    pub fn get_verified_peers(&self) -> Vec<&DiscoveredPeer> {
        self.peers.iter().filter(|p| p.verified).collect()
    }

    /// Marque un pair comme vérifié
    pub fn verify_peer(&mut self, address: &[u8; 32]) -> bool {
        if let Some(peer) = self.peers.iter_mut().find(|p| &p.address == address) {
            peer.verified = true;
            return true;
        }
        false
    }

    /// Retourne le nombre de pairs
    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }

    /// Cherche un pair par adresse
    pub fn find_peer(&self, address: &[u8; 32]) -> Option<&DiscoveredPeer> {
        self.peers.iter().find(|p| &p.address == address)
    }

    /// Retourne le meilleur pair (signal le plus fort)
    pub fn get_best_peer(&self) -> Option<&DiscoveredPeer> {
        self.peers.iter().max_by_key(|p| p.signal_dbm)
    }

    /// Retourne les statistiques
    pub fn get_stats(&self) -> DiscoveryStats {
        DiscoveryStats {
            mode: format!("{:?}", self.mode),
            peer_count: self.peers.len(),
            max_peers: self.max_peers,
            scan_count: self.scan_count,
            total_scan_time_s: self.total_scan_time_s,
            verified_count: self.get_verified_peers().len(),
            recent_count: self.get_recent_peers(60).len(),
            best_signal_dbm: self.get_best_peer().map(|p| p.signal_dbm).unwrap_or(-120),
        }
    }
}

/// Statistiques de découverte
#[derive(Debug, Clone)]
pub struct DiscoveryStats {
    pub mode: String,
    pub peer_count: usize,
    pub max_peers: usize,
    pub scan_count: u64,
    pub total_scan_time_s: u64,
    pub verified_count: usize,
    pub recent_count: usize,
    pub best_signal_dbm: i8,
}

// ═══════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    fn make_addr(n: u8) -> [u8; 32] { let mut a = [0u8; 32]; a[0] = n; a }

    // ─── Tests DiscoveredPeer ──────────────────

    #[test] fn test_peer_new() {
        let peer = DiscoveredPeer::new(make_addr(1), "Test", -40, "APN UQ");
        assert_eq!(peer.name, "Test");
        assert_eq!(peer.signal_dbm, -40);
        assert!(!peer.verified);
    }

    #[test] fn test_peer_update_last_seen() {
        let mut peer = DiscoveredPeer::new(make_addr(1), "Test", -40, "APN");
        let old_count = peer.discovery_count;
        peer.update_last_seen();
        // Vérifie juste que le compteur augmente
        assert_eq!(peer.discovery_count, old_count + 1);
    }

    #[test] fn test_peer_is_recent() {
        let peer = DiscoveredPeer::new(make_addr(1), "Test", -40, "APN");
        assert!(peer.is_recent(3600)); // 1 heure
    }

    #[test] fn test_peer_distance() {
        let near = DiscoveredPeer::new(make_addr(1), "Near", -20, "WiFi");
        let far = DiscoveredPeer::new(make_addr(2), "Far", -80, "WiFi");
        assert!(near.estimated_distance_m < far.estimated_distance_m);
    }

    // ─── Tests DiscoveryManager ────────────────

    #[test] fn test_manager_new() {
        let dm = DiscoveryManager::new(100, 30);
        assert_eq!(dm.peer_count(), 0);
        assert_eq!(dm.get_mode(), &DiscoveryMode::Passive);
    }

    #[test] fn test_add_peer() {
        let mut dm = DiscoveryManager::new(10, 30);
        dm.add_peer(make_addr(1), "Peer1", -40, "APN");
        assert_eq!(dm.peer_count(), 1);
    }

    #[test] fn test_add_duplicate_updates() {
        let mut dm = DiscoveryManager::new(10, 30);
        dm.add_peer(make_addr(1), "Peer1", -40, "APN");
        dm.add_peer(make_addr(1), "Peer1", -30, "APN");
        assert_eq!(dm.peer_count(), 1); // Toujours 1
        let peer = dm.find_peer(&make_addr(1)).unwrap();
        assert_eq!(peer.signal_dbm, -30); // Signal mis à jour
        assert_eq!(peer.discovery_count, 2);
    }

    #[test] fn test_remove_peer() {
        let mut dm = DiscoveryManager::new(10, 30);
        dm.add_peer(make_addr(1), "Peer1", -40, "APN");
        assert!(dm.remove_peer(&make_addr(1)));
        assert_eq!(dm.peer_count(), 0);
    }

    #[test] fn test_remove_nonexistent() {
        let mut dm = DiscoveryManager::new(10, 30);
        assert!(!dm.remove_peer(&make_addr(99)));
    }

    #[test] fn test_max_peers() {
        let mut dm = DiscoveryManager::new(3, 30);
        dm.add_peer(make_addr(1), "A", -40, "APN");
        dm.add_peer(make_addr(2), "B", -50, "APN");
        dm.add_peer(make_addr(3), "C", -60, "APN");
        dm.add_peer(make_addr(4), "D", -70, "APN"); // Devrait supprimer le plus ancien
        assert_eq!(dm.peer_count(), 3);
    }

    #[test] fn test_scan_off() {
        let mut dm = DiscoveryManager::new(10, 30);
        dm.set_mode(DiscoveryMode::Off);
        assert!(dm.scan().is_err());
    }

    #[test] fn test_scan_active() {
        let mut dm = DiscoveryManager::new(10, 30);
        dm.set_mode(DiscoveryMode::Active);
        dm.add_peer(make_addr(1), "A", -40, "APN");
        assert!(dm.scan().is_ok());
        assert_eq!(dm.scan_count, 1);
    }

    #[test] fn test_get_peers_by_signal() {
        let mut dm = DiscoveryManager::new(10, 30);
        dm.add_peer(make_addr(1), "Weak", -80, "APN");
        dm.add_peer(make_addr(2), "Strong", -20, "WiFi");
        dm.add_peer(make_addr(3), "Medium", -50, "BT");
        let sorted = dm.get_peers_by_signal();
        assert_eq!(sorted[0].name, "Strong");
        assert_eq!(sorted[2].name, "Weak");
    }

    #[test] fn test_verify_peer() {
        let mut dm = DiscoveryManager::new(10, 30);
        dm.add_peer(make_addr(1), "Peer", -40, "APN");
        assert!(dm.verify_peer(&make_addr(1)));
        assert!(dm.find_peer(&make_addr(1)).unwrap().verified);
        assert_eq!(dm.get_verified_peers().len(), 1);
    }

    #[test] fn test_verify_nonexistent() {
        let mut dm = DiscoveryManager::new(10, 30);
        assert!(!dm.verify_peer(&make_addr(99)));
    }

    #[test] fn test_best_peer() {
        let mut dm = DiscoveryManager::new(10, 30);
        dm.add_peer(make_addr(1), "Weak", -80, "APN");
        dm.add_peer(make_addr(2), "Best", -10, "WiFi");
        assert_eq!(dm.get_best_peer().unwrap().name, "Best");
    }

    #[test] fn test_cleanup_stale() {
        let mut dm = DiscoveryManager::new(10, 30);
        dm.add_peer(make_addr(1), "Old", -40, "APN");
        // Force le pair à être ancien
        dm.peers[0].last_seen = 0; // 1970
        let removed = dm.cleanup_stale(60);
        assert_eq!(removed, 1);
        assert_eq!(dm.peer_count(), 0);
    }

    #[test] fn test_stats() {
        let mut dm = DiscoveryManager::new(10, 30);
        dm.add_peer(make_addr(1), "A", -40, "APN");
        dm.set_mode(DiscoveryMode::Active);
        dm.scan().unwrap();
        let stats = dm.get_stats();
        assert_eq!(stats.peer_count, 1);
        assert_eq!(stats.scan_count, 1);
        assert!(stats.mode.contains("Active"));
    }
}




