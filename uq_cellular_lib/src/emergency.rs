use uq_blockchain::error::ChainError;

// ═══════════════════════════════════════════════════════════════
// TYPES D'URGENCE
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq)]
pub enum EmergencyType {
    /// Appel d'urgence (112, 911)
    EmergencyCall,
    /// Batterie critique (< 5%)
    LowBattery,
    /// Perte de signal
    SignalLost,
    /// Température excessive
    Overheating,
    /// Mode survie activé manuellement
    SurvivalMode,
    /// Alerte catastrophe
    DisasterAlert,
    /// Détresse personnelle
    PersonalDistress,
}

impl EmergencyType {
    pub fn name(&self) -> &str {
        match self {
            EmergencyType::EmergencyCall => "Appel urgence",
            EmergencyType::LowBattery => "Batterie critique",
            EmergencyType::SignalLost => "Perte signal",
            EmergencyType::Overheating => "Surchauffe",
            EmergencyType::SurvivalMode => "Mode survie",
            EmergencyType::DisasterAlert => "Alerte catastrophe",
            EmergencyType::PersonalDistress => "Détresse",
        }
    }

    pub fn priority(&self) -> u8 {
        match self {
            EmergencyType::EmergencyCall => 1,
            EmergencyType::PersonalDistress => 1,
            EmergencyType::DisasterAlert => 2,
            EmergencyType::SurvivalMode => 3,
            EmergencyType::LowBattery => 4,
            EmergencyType::SignalLost => 4,
            EmergencyType::Overheating => 5,
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// ÉTAT D'URGENCE
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq)]
pub enum EmergencyState {
    Normal,
    Warning,
    Active,
    Resolved,
}

// ═══════════════════════════════════════════════════════════════
// MESSAGE D'URGENCE
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone)]
pub struct EmergencyBeacon {
    pub emergency_type: EmergencyType,
    pub sender: [u8; 32],
    pub location: Option<(f64, f64)>,
    pub battery_pct: u8,
    pub message: String,
    pub timestamp: u64,
    pub ttl: u8,
}

impl EmergencyBeacon {
    pub fn new(
        emergency_type: EmergencyType,
        sender: [u8; 32],
        battery_pct: u8,
        message: &str,
    ) -> Self {
        EmergencyBeacon {
            emergency_type,
            sender,
            location: None,
            battery_pct,
            message: message.to_string(),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            ttl: 10,
        }
    }

    pub fn with_location(mut self, lat: f64, lon: f64) -> Self {
        self.location = Some((lat, lon));
        self
    }

    /// Encode en binaire compact
    pub fn encode(&self) -> Vec<u8> {
        let mut data = vec![
            self.emergency_type.priority(),
            self.battery_pct,
            self.ttl,
        ];
        data.extend_from_slice(&self.sender);
        data.extend_from_slice(&self.timestamp.to_le_bytes());
        let msg_bytes = self.message.as_bytes();
        data.push(msg_bytes.len() as u8);
        data.extend_from_slice(msg_bytes);
        data
    }

    /// Décrémente le TTL
    pub fn decrement_ttl(&mut self) -> bool {
        if self.ttl > 0 {
            self.ttl -= 1;
            true
        } else {
            false
        }
    }

    /// Vérifie si le beacon est encore valide
    pub fn is_valid(&self) -> bool {
        self.ttl > 0
    }
}

// ═══════════════════════════════════════════════════════════════
// GESTIONNAIRE D'URGENCE
// ═══════════════════════════════════════════════════════════════

pub struct EmergencyManager {
    state: EmergencyState,
    active_emergency: Option<EmergencyType>,
    our_address: [u8; 32],
    beacons_sent: u64,
    beacons_received: u64,
    beacons_relayed: u64,
    min_battery_for_relay: u8,
    relay_enabled: bool,
}

impl EmergencyManager {
    pub fn new(our_address: [u8; 32]) -> Self {
        EmergencyManager {
            state: EmergencyState::Normal,
            active_emergency: None,
            our_address,
            beacons_sent: 0,
            beacons_received: 0,
            beacons_relayed: 0,
            min_battery_for_relay: 5,
            relay_enabled: true,
        }
    }

    /// Active un mode d'urgence
    pub fn activate_emergency(&mut self, emergency_type: EmergencyType, battery_pct: u8, message: &str) -> EmergencyBeacon {
        self.state = EmergencyState::Active;
        self.active_emergency = Some(emergency_type.clone());
        let beacon = EmergencyBeacon::new(emergency_type, self.our_address, battery_pct, message);
        self.beacons_sent += 1;
        beacon
    }

    /// Désactive le mode urgence
    pub fn deactivate_emergency(&mut self) {
        self.state = EmergencyState::Resolved;
        self.active_emergency = None;
    }

    /// Traite un beacon d'urgence reçu
    pub fn process_beacon(&mut self, beacon: &mut EmergencyBeacon) -> Result<bool, ChainError> {
        self.beacons_received += 1;

        // Vérifier si on doit relayer
        if self.relay_enabled && beacon.is_valid() {
            beacon.decrement_ttl();
            self.beacons_relayed += 1;
            return Ok(true); // Relayer
        }

        Ok(false)
    }

    /// Vérifie si la batterie est suffisante pour relayer
    pub fn can_relay(&self, battery_pct: u8) -> bool {
        battery_pct > self.min_battery_for_relay
    }

    /// Active/désactive le relais
    pub fn set_relay(&mut self, enabled: bool) {
        self.relay_enabled = enabled;
    }

    /// Définit le seuil minimum de batterie pour relayer
    pub fn set_min_battery(&mut self, pct: u8) {
        self.min_battery_for_relay = pct.min(100);
    }

    /// Vérifie si on est en mode urgence
    pub fn is_in_emergency(&self) -> bool {
        self.state == EmergencyState::Active
    }

    /// Retourne le type d'urgence actif
    pub fn get_active_emergency(&self) -> Option<&EmergencyType> {
        self.active_emergency.as_ref()
    }

    /// Retourne l'état actuel
    pub fn get_state(&self) -> &EmergencyState {
        &self.state
    }

    /// Active l'alerte batterie faible
    pub fn check_battery(&mut self, battery_pct: u8) -> Option<EmergencyBeacon> {
        if battery_pct <= 5 && self.state == EmergencyState::Normal {
            self.state = EmergencyState::Warning;
            Some(self.activate_emergency(
                EmergencyType::LowBattery,
                battery_pct,
                "Batterie critique - Mode survie activé"
            ))
        } else {
            None
        }
    }

    /// Active l'alerte surchauffe
    pub fn check_temperature(&mut self, temp_c: f32, battery_pct: u8) -> Option<EmergencyBeacon> {
        if temp_c > 85.0 && self.state == EmergencyState::Normal {
            self.state = EmergencyState::Warning;
            Some(self.activate_emergency(
                EmergencyType::Overheating,
                battery_pct,
                &format!("Surchauffe détectée: {:.0}°C", temp_c)
            ))
        } else {
            None
        }
    }

    /// Retourne les statistiques
    pub fn get_stats(&self) -> EmergencyStats {
        EmergencyStats {
            state: format!("{:?}", self.state),
            active_emergency: self.active_emergency.as_ref().map(|e| e.name().to_string()).unwrap(),
            beacons_sent: self.beacons_sent,
            beacons_received: self.beacons_received,
            beacons_relayed: self.beacons_relayed,
            relay_enabled: self.relay_enabled,
        }
    }
}

#[derive(Debug, Clone)]
pub struct EmergencyStats {
    pub state: String,
    pub active_emergency: String,
    pub beacons_sent: u64,
    pub beacons_received: u64,
    pub beacons_relayed: u64,
    pub relay_enabled: bool,
}

// ═══════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    fn make_addr(n: u8) -> [u8; 32] { let mut a = [0u8; 32]; a[0] = n; a }

    // ─── Tests EmergencyType ────────────────────

    #[test] fn test_emergency_type_names() {
        assert_eq!(EmergencyType::LowBattery.name(), "Batterie critique");
        assert_eq!(EmergencyType::EmergencyCall.name(), "Appel urgence");
    }

    #[test] fn test_emergency_priority() {
        assert!(EmergencyType::EmergencyCall.priority() < EmergencyType::LowBattery.priority());
    }

    // ─── Tests EmergencyBeacon ──────────────────

    #[test] fn test_beacon_new() {
        let beacon = EmergencyBeacon::new(
            EmergencyType::LowBattery,
            make_addr(1),
            3,
            "Test"
        );
        assert_eq!(beacon.ttl, 10);
        assert!(beacon.is_valid());
    }

    #[test] fn test_beacon_decrement_ttl() {
        let mut beacon = EmergencyBeacon::new(EmergencyType::LowBattery, make_addr(1), 3, "Test");
        assert!(beacon.decrement_ttl());
        assert_eq!(beacon.ttl, 9);
    }

    #[test] fn test_beacon_ttl_expires() {
        let mut beacon = EmergencyBeacon::new(EmergencyType::LowBattery, make_addr(1), 3, "Test");
        beacon.ttl = 1;
        beacon.decrement_ttl();
        assert!(!beacon.is_valid());
    }

    #[test] fn test_beacon_encode() {
        let beacon = EmergencyBeacon::new(EmergencyType::LowBattery, make_addr(1), 3, "SOS");
        let encoded = beacon.encode();
        assert!(encoded.len() > 32);
    }

    // ─── Tests EmergencyManager ─────────────────

    #[test] fn test_manager_new() {
        let em = EmergencyManager::new(make_addr(1));
        assert!(!em.is_in_emergency());
        assert_eq!(em.get_state(), &EmergencyState::Normal);
    }

    #[test] fn test_activate_emergency() {
        let mut em = EmergencyManager::new(make_addr(1));
        let beacon = em.activate_emergency(EmergencyType::LowBattery, 3, "Test");
        assert!(em.is_in_emergency());
        assert_eq!(beacon.battery_pct, 3);
    }

    #[test] fn test_deactivate_emergency() {
        let mut em = EmergencyManager::new(make_addr(1));
        em.activate_emergency(EmergencyType::LowBattery, 3, "Test");
        em.deactivate_emergency();
        assert!(!em.is_in_emergency());
    }

    #[test] fn test_process_beacon() {
        let mut em = EmergencyManager::new(make_addr(1));
        let mut beacon = EmergencyBeacon::new(EmergencyType::LowBattery, make_addr(2), 5, "Help");
        let relay = em.process_beacon(&mut beacon).unwrap();
        assert!(relay); // Doit relayer
        assert_eq!(beacon.ttl, 9);
    }

    #[test] fn test_process_beacon_expired() {
        let mut em = EmergencyManager::new(make_addr(1));
        let mut beacon = EmergencyBeacon::new(EmergencyType::LowBattery, make_addr(2), 5, "Help");
        beacon.ttl = 0;
        let relay = em.process_beacon(&mut beacon).unwrap();
        assert!(!relay);
    }

    #[test] fn test_can_relay() {
        let em = EmergencyManager::new(make_addr(1));
        assert!(em.can_relay(50));
        assert!(!em.can_relay(3));
    }

    #[test] fn test_relay_disabled() {
        let mut em = EmergencyManager::new(make_addr(1));
        em.set_relay(false);
        let mut beacon = EmergencyBeacon::new(EmergencyType::LowBattery, make_addr(2), 50, "Help");
        let relay = em.process_beacon(&mut beacon).unwrap();
        assert!(!relay);
    }

    #[test] fn test_check_battery() {
        let mut em = EmergencyManager::new(make_addr(1));
        let beacon = em.check_battery(3);
        assert!(beacon.is_some());
        assert!(em.is_in_emergency());
    }

    #[test] fn test_check_battery_ok() {
        let mut em = EmergencyManager::new(make_addr(1));
        let beacon = em.check_battery(80);
        assert!(beacon.is_none());
    }

    #[test] fn test_check_temperature() {
        let mut em = EmergencyManager::new(make_addr(1));
        let beacon = em.check_temperature(90.0, 50);
        assert!(beacon.is_some());
    }

    #[test] fn test_stats() {
        let mut em = EmergencyManager::new(make_addr(1));
        em.activate_emergency(EmergencyType::LowBattery, 3, "Test");
        let stats = em.get_stats();
        assert_eq!(stats.beacons_sent, 1);
        assert!(stats.relay_enabled);
    }
}

