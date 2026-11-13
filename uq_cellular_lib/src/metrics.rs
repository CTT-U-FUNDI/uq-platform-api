
// ═══════════════════════════════════════════════════════════════
// MÉTRIQUES GLOBALES UQ CELLULAR
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone)]
pub struct CellularMetrics {
    // Beacon
    pub beacons_sent: u64,
    pub beacons_received: u64,
    // SIM
    pub sim_configured: bool,
    pub fail_open_detected: bool,
    // Modem
    pub modem_state: String,
    pub signal_dbm: i8,
    pub frequency_mhz: u32,
    pub power_dbm: i8,
    // Transport
    pub transport_mode: String,
    pub packets_sent: u64,
    pub packets_received: u64,
    pub transport_errors: u64,
    // APN
    pub current_apn: String,
    pub apn_connected: bool,
    // Phantom
    pub phantom_active: bool,
    pub phantom_messages_sent: u64,
    pub phantom_messages_received: u64,
    // Power
    pub battery_pct: u8,
    pub temperature_c: f32,
    pub eco_mode: bool,
    pub estimated_hours_remaining: f64,
    // Discovery
    pub peers_discovered: usize,
    pub verified_peers: usize,
    // Handshake
    pub handshakes_completed: u64,
    pub handshakes_failed: u64,
    // Emergency
    pub emergency_state: String,
    pub emergency_beacons_relayed: u64,
    // Number Pool
    pub numbers_assigned: usize,
    pub numbers_available: usize,
    // Global
    pub uptime_seconds: u64,
    pub total_data_mb: f64,
}

impl CellularMetrics {
    pub fn new() -> Self {
        CellularMetrics {
            beacons_sent: 0, beacons_received: 0,
            sim_configured: false, fail_open_detected: false,
            modem_state: "Off".into(), signal_dbm: -120, frequency_mhz: 0, power_dbm: -80,
            transport_mode: "None".into(), packets_sent: 0, packets_received: 0, transport_errors: 0,
            current_apn: "none".into(), apn_connected: false,
            phantom_active: false, phantom_messages_sent: 0, phantom_messages_received: 0,
            battery_pct: 100, temperature_c: 25.0, eco_mode: false, estimated_hours_remaining: 0.0,
            peers_discovered: 0, verified_peers: 0,
            handshakes_completed: 0, handshakes_failed: 0,
            emergency_state: "Normal".into(), emergency_beacons_relayed: 0,
            numbers_assigned: 0, numbers_available: 0,
            uptime_seconds: 0, total_data_mb: 0.0,
        }
    }

    /// Résumé texte
    pub fn summary(&self) -> String {
        format!(
            "UQ Cellular | Signal: {} dBm | Freq: {} MHz | Power: {} dBm | Battery: {}% | APN: {} | Peers: {} | Uptime: {}s",
            self.signal_dbm, self.frequency_mhz, self.power_dbm,
            self.battery_pct, self.current_apn, self.peers_discovered, self.uptime_seconds
        )
    }

    /// Vérifie si tout est optimal
    pub fn is_healthy(&self) -> bool {
        self.signal_dbm > -100
            && self.battery_pct > 10
            && self.temperature_c < 80.0
            && (self.apn_connected || self.phantom_active)
    }

    /// Score de santé (0-100)
    pub fn health_score(&self) -> u8 {
        let mut score: u8 = 100;

        if self.signal_dbm < -90 { score = score.saturating_sub(30); }
        else if self.signal_dbm < -70 { score = score.saturating_sub(10); }

        if self.battery_pct < 10 { score = score.saturating_sub(40); }
        else if self.battery_pct < 30 { score = score.saturating_sub(15); }

        if self.temperature_c > 80.0 { score = score.saturating_sub(30); }
        else if self.temperature_c > 60.0 { score = score.saturating_sub(10); }

        if !self.apn_connected && !self.phantom_active { score = score.saturating_sub(20); }

        score
    }
}

// ═══════════════════════════════════════════════════════════════
// COLLECTEUR DE MÉTRIQUES
// ═══════════════════════════════════════════════════════════════

pub struct MetricsCollector {
    metrics: CellularMetrics,
    start_time: u64,
    snapshots: Vec<CellularMetrics>,
    max_snapshots: usize,
}

impl MetricsCollector {
    pub fn new(max_snapshots: usize) -> Self {
        MetricsCollector {
            metrics: CellularMetrics::new(),
            start_time: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            snapshots: Vec::new(),
            max_snapshots,
        }
    }

    /// Met à jour les métriques
    pub fn update(&mut self, new_metrics: CellularMetrics) {
        self.metrics = new_metrics;
        self.metrics.uptime_seconds = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() - self.start_time;
    }

    /// Prend un snapshot
    pub fn snapshot(&mut self) {
        if self.snapshots.len() >= self.max_snapshots {
            self.snapshots.remove(0);
        }
        self.snapshots.push(self.metrics.clone());
    }

    /// Retourne les métriques actuelles
    pub fn get_metrics(&self) -> &CellularMetrics {
        &self.metrics
    }

    /// Retourne les snapshots
    pub fn get_snapshots(&self) -> &[CellularMetrics] {
        &self.snapshots
    }

    /// Retourne le nombre de snapshots
    pub fn snapshot_count(&self) -> usize {
        self.snapshots.len()
    }

    /// Calcule la moyenne des signaux sur les snapshots
    pub fn average_signal(&self) -> f64 {
        if self.snapshots.is_empty() { return self.metrics.signal_dbm as f64; }
        let sum: i32 = self.snapshots.iter().map(|s| s.signal_dbm as i32).sum();
        sum as f64 / self.snapshots.len() as f64
    }

    /// Calcule la tendance du signal (amélioration ou dégradation)
    pub fn signal_trend(&self) -> &str {
        if self.snapshots.len() < 2 { return "stable"; }
        let first = self.snapshots.first().unwrap().signal_dbm;
        let last = self.snapshots.last().unwrap().signal_dbm;
        if last > first + 5 { "improving" }
        else if last < first - 5 { "degrading" }
        else { "stable" }
    }

    /// Retourne le temps écoulé depuis le démarrage
    pub fn get_uptime(&self) -> u64 {
        self.metrics.uptime_seconds
    }

    /// Vérifie si les métriques sont saines
    pub fn is_healthy(&self) -> bool {
        self.metrics.is_healthy()
    }

    /// Retourne le score de santé
    pub fn health_score(&self) -> u8 {
        self.metrics.health_score()
    }

    /// Retourne le résumé
    pub fn summary(&self) -> String {
        self.metrics.summary()
    }
}

// ═══════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    // ─── Tests CellularMetrics ──────────────────

    #[test] fn test_metrics_new() {
        let m = CellularMetrics::new();
        assert_eq!(m.battery_pct, 100);
        assert_eq!(m.signal_dbm, -120);
    }

    #[test] fn test_metrics_summary() {
        let m = CellularMetrics::new();
        let summary = m.summary();
        assert!(summary.contains("UQ Cellular"));
        assert!(summary.contains("120 dBm"));
    }

    #[test] fn test_metrics_is_healthy() {
        let mut m = CellularMetrics::new();
        m.signal_dbm = -120;
        assert!(!m.is_healthy());
        m.signal_dbm = -50;
        m.apn_connected = true;
        assert!(m.is_healthy());
    }

    #[test] fn test_metrics_health_score_perfect() {
        let mut m = CellularMetrics::new();
        m.signal_dbm = -40;
        m.battery_pct = 100;
        m.temperature_c = 25.0;
        m.apn_connected = true;
        assert_eq!(m.health_score(), 100);
    }

    #[test] fn test_metrics_health_score_bad() {
        let mut m = CellularMetrics::new();
        m.signal_dbm = -110;
        m.battery_pct = 5;
        m.temperature_c = 90.0;
        m.apn_connected = false;
        m.phantom_active = false;
        assert!(m.health_score() < 30);
    }

    // ─── Tests MetricsCollector ─────────────────

    #[test] fn test_collector_new() {
        let mc = MetricsCollector::new(10);
        assert_eq!(mc.snapshot_count(), 0);
        assert!(mc.get_uptime() < 5);
    }

    #[test] fn test_collector_update() {
        let mut mc = MetricsCollector::new(10);
        let mut m = CellularMetrics::new();
        m.signal_dbm = -50;
        mc.update(m);
        assert_eq!(mc.get_metrics().signal_dbm, -50);
    }

    #[test] fn test_collector_snapshot() {
        let mut mc = MetricsCollector::new(10);
        mc.snapshot();
        assert_eq!(mc.snapshot_count(), 1);
    }

    #[test] fn test_collector_max_snapshots() {
        let mut mc = MetricsCollector::new(3);
        mc.snapshot(); mc.snapshot(); mc.snapshot(); mc.snapshot();
        assert_eq!(mc.snapshot_count(), 3); // Max 3
    }

    #[test] fn test_collector_average_signal() {
        let mut mc = MetricsCollector::new(10);
        let mut m1 = CellularMetrics::new(); m1.signal_dbm = -40;
        let mut m2 = CellularMetrics::new(); m2.signal_dbm = -60;
        mc.update(m1); mc.snapshot();
        mc.update(m2); mc.snapshot();
        assert_eq!(mc.average_signal(), -50.0);
    }

    #[test] fn test_collector_signal_trend_improving() {
        let mut mc = MetricsCollector::new(10);
        let mut m1 = CellularMetrics::new(); m1.signal_dbm = -80;
        let mut m2 = CellularMetrics::new(); m2.signal_dbm = -40;
        mc.update(m1); mc.snapshot();
        mc.update(m2); mc.snapshot();
        assert_eq!(mc.signal_trend(), "improving");
    }

    #[test] fn test_collector_signal_trend_degrading() {
        let mut mc = MetricsCollector::new(10);
        let mut m1 = CellularMetrics::new(); m1.signal_dbm = -40;
        let mut m2 = CellularMetrics::new(); m2.signal_dbm = -80;
        mc.update(m1); mc.snapshot();
        mc.update(m2); mc.snapshot();
        assert_eq!(mc.signal_trend(), "degrading");
    }

    #[test] fn test_collector_signal_trend_stable() {
        let mut mc = MetricsCollector::new(10);
        let mut m1 = CellularMetrics::new(); m1.signal_dbm = -50;
        let mut m2 = CellularMetrics::new(); m2.signal_dbm = -52;
        mc.update(m1); mc.snapshot();
        mc.update(m2); mc.snapshot();
        assert_eq!(mc.signal_trend(), "stable");
    }

    #[test] fn test_collector_health_score() {
        let mut mc = MetricsCollector::new(10);
        let mut m = CellularMetrics::new();
        m.signal_dbm = -40; m.battery_pct = 100; m.apn_connected = true;
        mc.update(m);
        assert_eq!(mc.health_score(), 100);
    }

    #[test] fn test_collector_summary() {
        let mc = MetricsCollector::new(10);
        assert!(mc.summary().contains("UQ Cellular"));
    }
}
