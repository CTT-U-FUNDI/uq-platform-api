use uq_blockchain::error::ChainError;

// ═══════════════════════════════════════════════════════════════
// TRAIT TRANSPORT (abstraction pour tests + production)
// ═══════════════════════════════════════════════════════════════

pub trait PhysicalTransport {
    fn send_bytes(&self, data: &[u8]) -> Result<(), ChainError>;
    fn receive_bytes(&self, buffer: &mut [u8]) -> Result<usize, ChainError>;
    fn is_connected(&self) -> bool;
    fn get_mtu(&self) -> usize;
    
    /// 🆕 Broadcast par défaut : non supporté
    fn broadcast_bytes(&self, _data: &[u8]) -> Result<(), ChainError> {
        Err(ChainError::InternalError("Broadcast non supporté".into()))
    }
}

// ═══════════════════════════════════════════════════════════════
// TYPES DE TRANSPORT
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq)]
pub enum TransportType {
    Serial,
    SPI,
    I2C,
    USB,
    Virtual,
}

/// Modes de transport UQ Cellular (6 variantes)
#[derive(Debug, Clone, PartialEq)]
pub enum TransportMode {
    /// Mode 1 : DATA normal (forfait opérateur)
    Normal,
    /// Mode 2 : APN "uq" (SIM reconfigurée, gratuit)
    ApnUq,
    /// Mode 3 : Canaux fantômes (SMS/USSD/Control Plane)
    Phantom,
    /// Mode 4 : WiFi Direct (P2P sans Internet)
    WiFiDirect,
    /// Mode 5 : Bluetooth Low Energy
    Bluetooth,
    /// Mode 6 : Ultrasons
    Ultrasonic,
    /// Mode 7 : Tor (anti-censure)
    Tor,
    /// Aucun transport disponible
    None,
}

impl TransportMode {
    /// Priorité (1 = meilleur, 255 = inutilisable)
    pub fn priority(&self) -> u8 {
        match self {
            TransportMode::Normal => 1,
            TransportMode::ApnUq => 2,
            TransportMode::WiFiDirect => 3,
            TransportMode::Bluetooth => 4,
            TransportMode::Ultrasonic => 5,
            TransportMode::Tor => 6,
            TransportMode::Phantom => 7,
            TransportMode::None => 255,
        }
    }

    /// Bande passante estimée (kbps)
    pub fn estimated_bandwidth_kbps(&self) -> u64 {
        match self {
            TransportMode::Normal => 100_000,
            TransportMode::ApnUq => 100,
            TransportMode::WiFiDirect => 50_000,
            TransportMode::Bluetooth => 2_000,
            TransportMode::Ultrasonic => 10,
            TransportMode::Tor => 500,
            TransportMode::Phantom => 1,
            TransportMode::None => 0,
        }
    }

    /// Nom lisible
    pub fn name(&self) -> &str {
        match self {
            TransportMode::Normal => "DATA Normal",
            TransportMode::ApnUq => "APN UQ",
            TransportMode::Phantom => "Canal Fantôme",
            TransportMode::WiFiDirect => "WiFi Direct",
            TransportMode::Bluetooth => "Bluetooth LE",
            TransportMode::Ultrasonic => "Ultrasons",
            TransportMode::Tor => "Tor",
            TransportMode::None => "Aucun",
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// TRAME UQ
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone)]
pub struct TransportFrame {
    pub sync: u8,
    pub length: u16,
    pub sequence: u8,
    pub payload: Vec<u8>,
    pub checksum: u8,
}

impl TransportFrame {
    pub fn new(payload: &[u8], sequence: u8) -> Self {
        let mut frame = TransportFrame {
            sync: 0x55,
            length: payload.len() as u16,
            sequence,
            payload: payload.to_vec(),
            checksum: 0,
        };
        frame.checksum = frame.compute_checksum();
        frame
    }

    pub fn from_bytes(data: &[u8]) -> Result<Self, ChainError> {
        if data.len() < 5 { return Err(ChainError::InternalError("Trame trop courte".into())); }
        if data[0] != 0x55 { return Err(ChainError::InternalError("Sync invalide".into())); }

        let length = u16::from_le_bytes([data[1], data[2]]) as usize;
        if data.len() < 4 + length { return Err(ChainError::InternalError("Longueur invalide".into())); }

        let sequence = data[3];
        let payload = data[4..4 + length].to_vec();
        let checksum = data[4 + length];

        let frame = TransportFrame { sync: 0x55, length: length as u16, sequence, payload, checksum };
        if !frame.verify_checksum() {
            return Err(ChainError::InternalError("Checksum invalide".into()));
        }
        Ok(frame)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = vec![self.sync];
        bytes.extend_from_slice(&self.length.to_le_bytes());
        bytes.push(self.sequence);
        bytes.extend_from_slice(&self.payload);
        bytes.push(self.checksum);
        bytes
    }

    fn compute_checksum(&self) -> u8 {
        let mut sum: u8 = self.sync;
        sum = sum.wrapping_add(self.length as u8);
        sum = sum.wrapping_add((self.length >> 8) as u8);
        sum = sum.wrapping_add(self.sequence);
        for &b in &self.payload { sum = sum.wrapping_add(b); }
        sum
    }

    fn verify_checksum(&self) -> bool {
        self.checksum == self.compute_checksum()
    }
}

// ═══════════════════════════════════════════════════════════════
// GESTIONNAIRE DE TRANSPORT (AVEC MODES UQ CELLULAR)
// ═══════════════════════════════════════════════════════════════

pub struct TransportLayer<T: PhysicalTransport> {
    transport: T,
    transport_type: TransportType,
    current_mode: TransportMode,
    mtu: usize,
    sequence: u8,
    packets_sent: u64,
    packets_received: u64,
    errors: u64,
}

impl<T: PhysicalTransport> TransportLayer<T> {
    pub fn new(transport: T, transport_type: TransportType) -> Self {
        let mtu = transport.get_mtu();
        TransportLayer {
            transport,
            transport_type,
            current_mode: TransportMode::None,
            mtu,
            sequence: 0,
            packets_sent: 0,
            packets_received: 0,
            errors: 0,
        }
    }

    // ─── TRAMAGE ──────────────────────────────────

    pub fn send(&mut self, data: &[u8]) -> Result<(), ChainError> {
        if !self.transport.is_connected() {
            return Err(ChainError::InternalError("Transport non connecté".into()));
        }
        if data.len() > self.mtu - 5 {
            return Err(ChainError::InternalError("Données trop volumineuses".into()));
        }
        let frame = TransportFrame::new(data, self.sequence);
        self.transport.send_bytes(&frame.to_bytes())?;
        self.sequence = self.sequence.wrapping_add(1);
        self.packets_sent += 1;
        Ok(())
    }

    /// 🆕 Diffuse un message en broadcast UDP
    pub fn broadcast(&mut self, data: &[u8]) -> Result<(), ChainError> {
        if !self.transport.is_connected() {
            return Err(ChainError::InternalError("Transport non connecté".into()));
        }
        let frame = TransportFrame::new(data, self.sequence);
        self.transport.broadcast_bytes(&frame.to_bytes())?;
        self.sequence = self.sequence.wrapping_add(1);
        self.packets_sent += 1;
        Ok(())
    }

    pub fn receive(&mut self, buffer: &mut [u8; 2048]) -> Result<Vec<u8>, ChainError> {
        let n = self.transport.receive_bytes(buffer)?;
        if n < 5 { return Err(ChainError::InternalError("Trame reçue trop courte".into())); }
        let frame = TransportFrame::from_bytes(&buffer[..n])?;
        self.packets_received += 1;
        Ok(frame.payload)
    }

    // ─── SÉLECTION DE MODE UQ CELLULAR ────────────

    /// F1. Sélectionne le meilleur mode de transport disponible
    pub fn select_best_mode(
        &self,
        sim_ok: bool,
        wifi_ok: bool,
        bt_ok: bool,
        data_available: bool,
    ) -> TransportMode {
        let mut candidates: Vec<TransportMode> = Vec::new();

        if data_available && sim_ok {
            candidates.push(TransportMode::Normal);
        }
        if sim_ok && self.apn_uq_available() {
            candidates.push(TransportMode::ApnUq);
        }
        if wifi_ok && self.direct_available() {
            candidates.push(TransportMode::WiFiDirect);
        }
        if bt_ok {
            candidates.push(TransportMode::Bluetooth);
        }
        if self.phantom_available() {
            candidates.push(TransportMode::Phantom);
        }

        candidates.sort_by_key(|m| m.priority());
        candidates.first().cloned().unwrap_or(TransportMode::None)
    }

    /// F2. Vérifie si WiFi Direct est disponible
    pub fn direct_available(&self) -> bool {
        self.transport.is_connected()
    }

    /// F3. Vérifie si l'APN "uq" fonctionne
    pub fn apn_uq_available(&self) -> bool {
        // TODO: Appeler apn_manager::test_fail_open()
        false
    }

    /// F4. Vérifie si les données normales sont disponibles
    pub fn data_normal_available(&self) -> bool {
        // TODO: Vérifier via TelephonyManager.isDataEnabled()
        false
    }

    /// F5. Vérifie si le canal fantôme est disponible
    pub fn phantom_available(&self) -> bool {
        // TODO: Vérifier si SMS/USSD/Control Plane accessibles
        false
    }

    /// F6. Bascule vers un nouveau mode de transport
    pub fn switch_mode(&mut self, mode: TransportMode) -> Result<(), ChainError> {
        match mode {
            TransportMode::Normal => { /* Rétablir APN "internet" */ }
            TransportMode::ApnUq => { /* Configurer APN "uq" sur la SIM */ }
            TransportMode::Phantom => { /* Activer canaux fantômes */ }
            TransportMode::WiFiDirect => { /* Activer WiFi P2P */ }
            TransportMode::Bluetooth => { /* Activer Bluetooth LE */ }
            TransportMode::Ultrasonic => { /* Activer émission/réception ultrasons */ }
            TransportMode::Tor => { /* Activer routage Tor */ }
            TransportMode::None => {
                return Err(ChainError::InternalError("Aucun transport disponible".into()));
            }
        }
        self.current_mode = mode;
        Ok(())
    }

    // ─── UTILITAIRES ──────────────────────────────

    pub fn is_ready(&self) -> bool { self.transport.is_connected() }
    pub fn reset_sequence(&mut self) { self.sequence = 0; }
    pub fn get_current_mode(&self) -> &TransportMode { &self.current_mode }

    pub fn get_metrics(&self) -> TransportMetrics {
        TransportMetrics {
            transport_type: format!("{:?}", self.transport_type),
            current_mode: self.current_mode.name().to_string(),
            mtu: self.mtu,
            connected: self.is_ready(),
            packets_sent: self.packets_sent,
            packets_received: self.packets_received,
            errors: self.errors,
            sequence: self.sequence,
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// MÉTRIQUES
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone)]
pub struct TransportMetrics {
    pub transport_type: String,
    pub current_mode: String,
    pub mtu: usize,
    pub connected: bool,
    pub packets_sent: u64,
    pub packets_received: u64,
    pub errors: u64,
    pub sequence: u8,
}

// ═══════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct MockTransport {
        connected: bool,
        mtu: usize,
        last_sent: RefCell<Vec<u8>>,
        recv_buffer: RefCell<Vec<u8>>,
    }

    impl MockTransport {
        fn new() -> Self {
            MockTransport { connected: true, mtu: 256, last_sent: RefCell::new(vec![]), recv_buffer: RefCell::new(vec![]) }
        }
    }

    impl PhysicalTransport for MockTransport {
        fn send_bytes(&self, data: &[u8]) -> Result<(), ChainError> {
            self.last_sent.replace(data.to_vec());
            Ok(())
        }
        fn receive_bytes(&self, buffer: &mut [u8]) -> Result<usize, ChainError> {
            let data = self.recv_buffer.borrow();
            let len = data.len().min(buffer.len());
            buffer[..len].copy_from_slice(&data[..len]);
            Ok(len)
        }
        fn is_connected(&self) -> bool { self.connected }
        fn get_mtu(&self) -> usize { self.mtu }
    }

    // ─── Tests de trame ──────────────────────────

    #[test] fn test_frame_create() { let f = TransportFrame::new(b"Hello", 0); assert_eq!(f.sync, 0x55); assert_eq!(f.payload, b"Hello"); }
    #[test] fn test_frame_roundtrip() { let f = TransportFrame::new(b"Test", 5); let bytes = f.to_bytes(); let f2 = TransportFrame::from_bytes(&bytes).unwrap(); assert_eq!(f.payload, f2.payload); }
    #[test] fn test_frame_bad_sync() { assert!(TransportFrame::from_bytes(&[0x00]).is_err()); }
    #[test] fn test_frame_checksum_error() { let mut f = TransportFrame::new(b"Data", 1); f.checksum = 0xFF; let bytes = f.to_bytes(); assert!(TransportFrame::from_bytes(&bytes).is_err()); }

    // ─── Tests de transport ──────────────────────

    #[test] fn test_transport_new() { let t = TransportLayer::new(MockTransport::new(), TransportType::SPI); assert!(t.is_ready()); }
    #[test] fn test_send() { let mut t = TransportLayer::new(MockTransport::new(), TransportType::SPI); assert!(t.send(b"Test").is_ok()); }
    #[test] fn test_sequence_rollover() { let mut t = TransportLayer::new(MockTransport::new(), TransportType::SPI); t.sequence = 254; t.send(b"a").unwrap(); assert_eq!(t.sequence, 255); t.send(b"b").unwrap(); assert_eq!(t.sequence, 0); }
    #[test] fn test_metrics() { let t = TransportLayer::new(MockTransport::new(), TransportType::SPI); let m = t.get_metrics(); assert!(m.connected); }

    // ─── Tests UQ Cellular ───────────────────────

    #[test] fn test_transport_mode_priority() { assert!(TransportMode::Normal.priority() < TransportMode::ApnUq.priority()); assert!(TransportMode::ApnUq.priority() < TransportMode::Phantom.priority()); assert_eq!(TransportMode::None.priority(), 255); }
    #[test] fn test_bandwidth_estimation() { assert!(TransportMode::Normal.estimated_bandwidth_kbps() > 1000); assert!(TransportMode::Phantom.estimated_bandwidth_kbps() < 100); }
    #[test] fn test_select_best_mode_normal() { let t = TransportLayer::new(MockTransport::new(), TransportType::USB); let mode = t.select_best_mode(true, true, true, true); assert_eq!(mode, TransportMode::Normal); }
    #[test] fn test_select_best_mode_no_data() { let t = TransportLayer::new(MockTransport::new(), TransportType::SPI); let mode = t.select_best_mode(true, true, true, false); assert_eq!(mode, TransportMode::WiFiDirect); }
    #[test] fn test_select_best_mode_nothing() { let t = TransportLayer::new(MockTransport::new(), TransportType::Virtual); let mode = t.select_best_mode(false, false, false, false); assert_eq!(mode, TransportMode::None); }
    #[test] fn test_mode_names() { assert_eq!(TransportMode::ApnUq.name(), "APN UQ"); assert_eq!(TransportMode::Phantom.name(), "Canal Fantôme"); }
}




