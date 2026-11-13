use uq_blockchain::error::ChainError;

/// Modes D2D supportés
#[derive(Debug, Clone, PartialEq)]
pub enum D2dMode {
    /// LTE Direct (3GPP Release 12, 2015)
    LteDirect,
    /// 5G NR Sidelink (3GPP Release 16, 2020)
    NrSidelink,
    /// WiFi Direct (fallback)
    WiFiDirect,
    /// Bluetooth 5.0 Long Range (fallback)
    BluetoothLR,
    /// Ultrasons (dernier recours)
    Ultrasonic,
    /// Désactivé
    Off,
}

impl D2dMode {
    pub fn name(&self) -> &str {
        match self {
            D2dMode::LteDirect => "LTE Direct",
            D2dMode::NrSidelink => "5G NR Sidelink",
            D2dMode::WiFiDirect => "WiFi Direct",
            D2dMode::BluetoothLR => "Bluetooth 5.0 LR",
            D2dMode::Ultrasonic => "Ultrasons",
            D2dMode::Off => "Désactivé",
        }
    }

    pub fn max_range_m(&self) -> u32 {
        match self {
            D2dMode::LteDirect => 500,
            D2dMode::NrSidelink => 1000,
            D2dMode::WiFiDirect => 100,
            D2dMode::BluetoothLR => 400,
            D2dMode::Ultrasonic => 10,
            D2dMode::Off => 0,
        }
    }

    pub fn max_bandwidth_kbps(&self) -> u32 {
        match self {
            D2dMode::LteDirect => 50000,
            D2dMode::NrSidelink => 100000,
            D2dMode::WiFiDirect => 250000,
            D2dMode::BluetoothLR => 2000,
            D2dMode::Ultrasonic => 10,
            D2dMode::Off => 0,
        }
    }

    pub fn priority(&self) -> u8 {
        match self {
            D2dMode::NrSidelink => 1,
            D2dMode::LteDirect => 2,
            D2dMode::WiFiDirect => 3,
            D2dMode::BluetoothLR => 4,
            D2dMode::Ultrasonic => 5,
            D2dMode::Off => 255,
        }
    }
}

/// Statistiques de lien D2D
#[derive(Debug, Clone)]
pub struct D2dLinkStats {
    pub mode: D2dMode,
    pub signal_strength_dbm: i8,
    pub bandwidth_kbps: u32,
    pub latency_ms: u16,
    pub packet_loss_pct: f32,
    pub is_active: bool,
    pub peer_count: u32,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub uptime_seconds: u64,
}

/// Gestionnaire D2D principal
pub struct D2dManager {
    active_mode: D2dMode,
    is_enabled: bool,
    stats: D2dLinkStats,
    start_time: Option<std::time::Instant>,
}

impl D2dManager {
    pub fn new() -> Self {
        D2dManager {
            active_mode: D2dMode::Off,
            is_enabled: false,
            stats: D2dLinkStats {
                mode: D2dMode::Off,
                signal_strength_dbm: -120,
                bandwidth_kbps: 0,
                latency_ms: 999,
                packet_loss_pct: 100.0,
                is_active: false,
                peer_count: 0,
                bytes_sent: 0,
                bytes_received: 0,
                uptime_seconds: 0,
            },
            start_time: None,
        }
    }

    /// Active le D2D avec le meilleur mode disponible
    pub fn enable(&mut self) -> Result<D2dMode, ChainError> {
        // Essayer les modes dans l'ordre de priorité
        let modes = [
            D2dMode::NrSidelink,
            D2dMode::LteDirect,
            D2dMode::WiFiDirect,
            D2dMode::BluetoothLR,
        ];

        for mode in &modes {
            if self.try_enable_mode(mode.clone()) {
                self.active_mode = mode.clone();
                self.is_enabled = true;
                self.stats.mode = mode.clone();
                self.stats.is_active = true;
                self.start_time = Some(std::time::Instant::now());
                return Ok(mode.clone());
            }
        }

        Err(ChainError::InternalError("Aucun mode D2D disponible".into()))
    }

    /// Désactive le D2D
    pub fn disable(&mut self) {
        self.is_enabled = false;
        self.active_mode = D2dMode::Off;
        self.stats.is_active = false;
        self.start_time = None;
    }

    /// Vérifie si le D2D est actif
    pub fn is_active(&self) -> bool {
        self.is_enabled
    }

    /// Retourne le mode actif
    pub fn get_mode(&self) -> &D2dMode {
        &self.active_mode
    }

    /// Retourne la portée maximale actuelle
    pub fn get_max_range(&self) -> u32 {
        self.active_mode.max_range_m()
    }

    /// Retourne la bande passante maximale
    pub fn get_max_bandwidth(&self) -> u32 {
        self.active_mode.max_bandwidth_kbps()
    }

    /// Met à jour les statistiques
    pub fn update_stats(&mut self, signal_dbm: i8, peers: u32, bytes_tx: u64, bytes_rx: u64) {
        self.stats.signal_strength_dbm = signal_dbm;
        self.stats.peer_count = peers;
        self.stats.bytes_sent += bytes_tx;
        self.stats.bytes_received += bytes_rx;
        if let Some(start) = self.start_time {
            self.stats.uptime_seconds = start.elapsed().as_secs();
        }
    }

    /// Retourne les statistiques
    pub fn get_stats(&self) -> &D2dLinkStats {
        &self.stats
    }

    /// Tente d'activer un mode spécifique
    fn try_enable_mode(&self, mode: D2dMode) -> bool {
        match mode {
            D2dMode::NrSidelink => {
                // Vérifier si le chipset supporte 5G NR Sidelink
                // TODO: Vérifier via Android TelephonyManager
                cfg!(target_os = "android") // Simplifié pour l'instant
            }
            D2dMode::LteDirect => {
                // Vérifier si le chipset supporte LTE Direct
                cfg!(target_os = "android")
            }
            D2dMode::WiFiDirect => true, // Toujours disponible
            D2dMode::BluetoothLR => true, // Presque toujours disponible
            D2dMode::Ultrasonic => true, // Toujours disponible
            D2dMode::Off => false,
        }
    }

    /// Vérifie si un mode spécifique est disponible
    pub fn is_mode_available(&self, mode: &D2dMode) -> bool {
        self.try_enable_mode(mode.clone())
    }

    /// Retourne les modes disponibles
    pub fn get_available_modes(&self) -> Vec<D2dMode> {
        let all_modes = [
            D2dMode::NrSidelink,
            D2dMode::LteDirect,
            D2dMode::WiFiDirect,
            D2dMode::BluetoothLR,
            D2dMode::Ultrasonic,
        ];
        
        all_modes.iter()
            .filter(|m| self.try_enable_mode((*m).clone()))
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] fn test_new() {
        let dm = D2dManager::new();
        assert!(!dm.is_active());
        assert_eq!(dm.get_mode(), &D2dMode::Off);
    }

    #[test] fn test_enable_disable() {
        let mut dm = D2dManager::new();
        assert!(dm.enable().is_ok());
        assert!(dm.is_active());
        dm.disable();
        assert!(!dm.is_active());
    }

    #[test] fn test_mode_names() {
        assert_eq!(D2dMode::LteDirect.name(), "LTE Direct");
        assert_eq!(D2dMode::NrSidelink.name(), "5G NR Sidelink");
    }

    #[test] fn test_mode_range() {
        assert!(D2dMode::NrSidelink.max_range_m() > D2dMode::LteDirect.max_range_m());
        assert!(D2dMode::LteDirect.max_range_m() > D2dMode::WiFiDirect.max_range_m());
    }

    #[test] fn test_mode_priority() {
        assert!(D2dMode::NrSidelink.priority() < D2dMode::LteDirect.priority());
        assert!(D2dMode::LteDirect.priority() < D2dMode::WiFiDirect.priority());
    }

    #[test] fn test_stats() {
        let mut dm = D2dManager::new();
        dm.enable().unwrap();
        dm.update_stats(-50, 3, 1024, 2048);
        let stats = dm.get_stats();
        assert_eq!(stats.signal_strength_dbm, -50);
        assert_eq!(stats.peer_count, 3);
        assert!(stats.bytes_sent > 0);
    }

    #[test] fn test_available_modes() {
        let dm = D2dManager::new();
        let modes = dm.get_available_modes();
        assert!(modes.contains(&D2dMode::WiFiDirect));
        assert!(modes.contains(&D2dMode::BluetoothLR));
        assert!(modes.contains(&D2dMode::Ultrasonic));
    }
}

