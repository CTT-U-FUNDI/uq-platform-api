use uq_blockchain::error::ChainError;
use super::d2d::D2dManager;
use super::d2d_discovery::D2dDiscovery;
use super::relay_invisible::RelayInvisible;
use super::relay_internet::RelayInternet;
use super::sim_detector::SimDetector;
use super::cellular_config::CellularConfig;

/// États du pilote automatique
#[derive(Debug, Clone, PartialEq)]
pub enum AutoPilotState {
    /// En attente
    Idle,
    /// Scan en cours
    Scanning,
    /// Connecté au réseau UQ natif (gratuit)
    ConnectedUqMesh,
    /// Connecté via relais Internet (payant)
    ConnectedViaRelay,
    /// Mode économie d'énergie
    BatterySaver,
    /// Erreur
    Error,
}

/// Rapport d'état du pilote automatique
#[derive(Debug, Clone)]
pub struct AutoPilotReport {
    pub state: AutoPilotState,
    pub d2d_active: bool,
    pub peers_visible: usize,
    pub relay_capable_peers: usize,
    pub connected_to_internet: bool,
    pub is_providing_relay: bool,
    pub daily_earnings_micro: u64,
    pub daily_cost_micro: u64,
    pub battery_pct: u8,
    pub recommendation: String,
}

/// Pilote automatique principal
pub struct AutoPilot {
    config: CellularConfig,
    state: AutoPilotState,
    d2d: D2dManager,
    discovery: D2dDiscovery,
    relay_invisible: RelayInvisible,
    relay_internet: RelayInternet,
    sim_detector: SimDetector,
    total_uptime_seconds: u64,
    last_decision_time: Option<std::time::Instant>,
}

impl AutoPilot {
    pub fn new(config: CellularConfig) -> Self {
        AutoPilot {
            config: config.clone(),
            state: AutoPilotState::Idle,
            d2d: D2dManager::new(),
            discovery: D2dDiscovery::new(100, config.scan_interval_secs),
            relay_invisible: RelayInvisible::new(),
            relay_internet: RelayInternet::new(config.internet_relay_price_micro),
            sim_detector: SimDetector::new(),
            total_uptime_seconds: 0,
            last_decision_time: None,
        }
    }

    /// Démarre le pilote automatique
    pub fn start(&mut self) -> Result<(), ChainError> {
        // 1. Activer le D2D
        self.d2d.enable()?;
        
        // 2. Détecter la SIM
        let sim_status = self.sim_detector.detect();
        
        // 3. Si SIM + DATA → activer le relais invisible
        if sim_status.has_sim && sim_status.has_data && sim_status.battery_pct > 30 {
            self.relay_invisible.enable(
                sim_status.total_bandwidth_kbps,
                5, // max 5 clients
                sim_status.battery_pct
            );
        }
        
        // 4. Démarrer le scan de découverte
        self.discovery.start_scan()?;
        self.state = AutoPilotState::Scanning;
        
        Ok(())
    }

    /// Arrête le pilote automatique
    pub fn stop(&mut self) {
        self.d2d.disable();
        self.relay_invisible.disable();
        self.relay_internet.disconnect();
        self.discovery.stop_scan();
        self.state = AutoPilotState::Idle;
    }

    /// Évalue l'environnement et prend une décision
    pub fn evaluate(&mut self) -> AutoPilotReport {
        let sim_status = self.sim_detector.detect();
        let d2d_stats = self.d2d.get_stats();
        let discovery_stats = self.discovery.get_stats();
        
        // Décision : état actuel
        if self.relay_internet.is_connected() {
            self.state = AutoPilotState::ConnectedViaRelay;
        } else if d2d_stats.is_active && discovery_stats.peer_count > 0 {
            self.state = AutoPilotState::ConnectedUqMesh;
        } else if sim_status.battery_pct < 20 {
            self.state = AutoPilotState::BatterySaver;
        }
        
        // Recommandation
        let recommendation = if sim_status.has_data && !self.relay_invisible.is_active() {
            "Activer le relais invisible pour gagner des BORB".to_string()
        } else if !self.relay_internet.is_connected() && discovery_stats.relay_capable_count > 0 {
            "Des relais Internet sont disponibles. Se connecter ?".to_string()
        } else {
            "Fonctionnement normal".to_string()
        };
        
        AutoPilotReport {
            state: self.state.clone(),
            d2d_active: d2d_stats.is_active,
            peers_visible: discovery_stats.peer_count,
            relay_capable_peers: discovery_stats.relay_capable_count,
            connected_to_internet: self.relay_internet.is_connected(),
            is_providing_relay: self.relay_invisible.is_active(),
            daily_earnings_micro: self.relay_invisible.get_daily_earnings(),
            daily_cost_micro: self.relay_internet.get_daily_cost(),
            battery_pct: sim_status.battery_pct,
            recommendation,
        }
    }

    /// Connecte via un relais Internet
    pub fn connect_via_relay(&mut self, peer_address: &[u8; 32]) -> Result<(), ChainError> {
        self.relay_internet.connect(peer_address, 24)?; // 24h
        self.state = AutoPilotState::ConnectedViaRelay;
        Ok(())
    }

    /// Déconnecte du relais Internet
    pub fn disconnect_relay(&mut self) {
        self.relay_internet.disconnect();
        self.state = AutoPilotState::ConnectedUqMesh;
    }

    /// Retourne l'état actuel
    pub fn get_state(&self) -> &AutoPilotState {
        &self.state
    }

    /// Retourne un rapport complet
    pub fn get_report(&self) -> AutoPilotReport {
        AutoPilotReport {
            state: self.state.clone(),
            d2d_active: self.d2d.is_active(),
            peers_visible: self.discovery.peer_count(),
            relay_capable_peers: self.discovery.get_relay_capable_peers().len(),
            connected_to_internet: self.relay_internet.is_connected(),
            is_providing_relay: self.relay_invisible.is_active(),
            daily_earnings_micro: self.relay_invisible.get_daily_earnings(),
            daily_cost_micro: self.relay_internet.get_daily_cost(),
            battery_pct: self.sim_detector.detect().battery_pct,
            recommendation: "Rapport manuel".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config() -> CellularConfig {
        CellularConfig {
            scan_interval_secs: 30,
            internet_relay_price_micro: 10,
            ..Default::default()
        }
    }

    #[test] fn test_new() {
        let ap = AutoPilot::new(make_config());
        assert_eq!(ap.get_state(), &AutoPilotState::Idle);
    }

    #[test] fn test_start_stop() {
        let mut ap = AutoPilot::new(make_config());
        assert!(ap.start().is_ok());
        assert_eq!(ap.get_state(), &AutoPilotState::Scanning);
        ap.stop();
        assert_eq!(ap.get_state(), &AutoPilotState::Idle);
    }

    #[test] fn test_evaluate() {
        let mut ap = AutoPilot::new(make_config());
        ap.start().unwrap();
        let report = ap.evaluate();
        assert!(!report.recommendation.is_empty());
    }

    #[test] fn test_report() {
        let ap = AutoPilot::new(make_config());
        let report = ap.get_report();
        assert_eq!(report.state, AutoPilotState::Idle);
        assert!(!report.is_providing_relay);
    }

    #[test] fn test_connect_disconnect_relay() {
        let mut ap = AutoPilot::new(make_config());
        let addr = [1u8; 32];
        assert!(ap.connect_via_relay(&addr).is_ok());
        assert_eq!(ap.get_state(), &AutoPilotState::ConnectedViaRelay);
        ap.disconnect_relay();
        assert_eq!(ap.get_state(), &AutoPilotState::ConnectedUqMesh);
    }
}


