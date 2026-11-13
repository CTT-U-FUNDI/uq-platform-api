use uq_blockchain::error::ChainError;

// ═══════════════════════════════════════════════════════════════
// TYPES DE CANAUX FANTÔMES
// ═══════════════════════════════════════════════════════════════

/// Types de canaux fantômes supportés
#[derive(Debug, Clone, PartialEq)]
pub enum PhantomChannelType {
    /// SMS (0 octets facturés si illimité)
    Sms,
    /// USSD (*100#)
    Ussd,
    /// Control Plane (signaling LTE)
    ControlPlane,
    /// Cell Broadcast
    CellBroadcast,
    /// Paging Channel
    PagingChannel,
    /// Random Access Channel
    Rach,
}

impl PhantomChannelType {
    /// Bande passante estimée (bits/s)
    pub fn bandwidth_bps(&self) -> u64 {
        match self {
            PhantomChannelType::Sms => 1120,           // 140 octets * 8 bits
            PhantomChannelType::Ussd => 1456,          // 182 caractères * 8 bits
            PhantomChannelType::ControlPlane => 8000,     // ~8 kbps (meilleur que SMS)
            PhantomChannelType::CellBroadcast => 9600,    // ~9.6 kbps
            PhantomChannelType::PagingChannel => 800,      // ~800 bps
            PhantomChannelType::Rach => 4000,              // ~4 kbps
        }
    }

    /// Nom lisible
    pub fn name(&self) -> &str {
        match self {
            PhantomChannelType::Sms => "SMS",
            PhantomChannelType::Ussd => "USSD",
            PhantomChannelType::ControlPlane => "Control Plane",
            PhantomChannelType::CellBroadcast => "Cell Broadcast",
            PhantomChannelType::PagingChannel => "Paging",
            PhantomChannelType::Rach => "RACH",
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// MESSAGE FANTÔME
// ═══════════════════════════════════════════════════════════════

/// Message encodé pour canal fantôme
#[derive(Debug, Clone)]
pub struct PhantomMessage {
    /// Type de canal utilisé
    pub channel: PhantomChannelType,
    /// Données utiles (max 140 octets pour SMS)
    pub payload: Vec<u8>,
    /// Numéro de séquence
    pub sequence: u8,
    /// Horodatage
    pub timestamp: u64,
    /// Priorité (0 = normale, 1 = urgente)
    pub priority: u8,
}

impl PhantomMessage {
    /// Crée un nouveau message fantôme
    pub fn new(channel: PhantomChannelType, payload: &[u8], sequence: u8, priority: u8) -> Self {
        PhantomMessage {
            channel,
            payload: payload.to_vec(),
            sequence,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            priority,
        }
    }

    /// Encode le message en binaire pour transmission
    pub fn encode(&self) -> Vec<u8> {
        let mut data = vec![
            self.channel.to_byte(),
            self.priority,
            self.sequence,
        ];
        data.extend_from_slice(&self.timestamp.to_le_bytes());
        data.push(self.payload.len() as u8);
        data.extend_from_slice(&self.payload);
        data
    }

    /// Décode un message binaire
    pub fn decode(data: &[u8]) -> Result<Self, ChainError> {
        if data.len() < 12 {
            return Err(ChainError::InternalError("Message fantôme trop court".into()));
        }
        
        let channel = PhantomChannelType::from_byte(data[0])?;
        let priority = data[1];
        let sequence = data[2];
        let timestamp = u64::from_le_bytes([
            data[3], data[4], data[5], data[6],
            data[7], data[8], data[9], data[10],
        ]);
        let payload_len = data[11] as usize;
        
        if data.len() < 12 + payload_len {
            return Err(ChainError::InternalError("Longueur payload invalide".into()));
        }
        
        let payload = data[12..12 + payload_len].to_vec();
        
        Ok(PhantomMessage {
            channel,
            payload,
            sequence,
            timestamp,
            priority,
        })
    }

    /// Vérifie si le message est urgent
    pub fn is_urgent(&self) -> bool {
        self.priority == 1
    }

    /// Taille totale en octets
    pub fn size(&self) -> usize {
        12 + self.payload.len()
    }
}

impl PhantomChannelType {
    fn to_byte(&self) -> u8 {
        match self {
            PhantomChannelType::Sms => 0x01,
            PhantomChannelType::Ussd => 0x02,
            PhantomChannelType::ControlPlane => 0x03,
            PhantomChannelType::CellBroadcast => 0x04,
            PhantomChannelType::PagingChannel => 0x05,
            PhantomChannelType::Rach => 0x06,
        }
    }

    fn from_byte(byte: u8) -> Result<Self, ChainError> {
        match byte {
            0x01 => Ok(PhantomChannelType::Sms),
            0x02 => Ok(PhantomChannelType::Ussd),
            0x03 => Ok(PhantomChannelType::ControlPlane),
            0x04 => Ok(PhantomChannelType::CellBroadcast),
            0x05 => Ok(PhantomChannelType::PagingChannel),
            0x06 => Ok(PhantomChannelType::Rach),
            _ => Err(ChainError::InternalError(format!("Type canal fantôme inconnu: 0x{:02x}", byte))),
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// GESTIONNAIRE DE CANAUX FANTÔMES
// ═══════════════════════════════════════════════════════════════

pub struct PhantomChannelManager {
    /// Canaux disponibles
    available_channels: Vec<PhantomChannelType>,
    /// Canal actif
    active_channel: Option<PhantomChannelType>,
    /// Séquence actuelle
    sequence: u8,
    /// Messages envoyés
    messages_sent: u64,
    /// Messages reçus
    messages_received: u64,
    /// Total octets envoyés
    bytes_sent: u64,
    /// Total octets reçus
    bytes_received: u64,
    /// Mode découverte actif
    discovery_mode: bool,
}

impl PhantomChannelManager {
    /// Crée un nouveau gestionnaire
    pub fn new() -> Self {
        PhantomChannelManager {
            available_channels: vec![
                PhantomChannelType::Sms,
                PhantomChannelType::Ussd,
                PhantomChannelType::ControlPlane,
            ],
            active_channel: None,
            sequence: 0,
            messages_sent: 0,
            messages_received: 0,
            bytes_sent: 0,
            bytes_received: 0,
            discovery_mode: true,
        }
    }

    /// Vérifie si un canal spécifique est disponible
    pub fn is_channel_available(&self, channel: &PhantomChannelType) -> bool {
        self.available_channels.contains(channel)
    }

    /// Retourne les canaux disponibles
    pub fn get_available_channels(&self) -> &[PhantomChannelType] {
        &self.available_channels
    }

    /// Active un canal spécifique
    pub fn activate_channel(&mut self, channel: PhantomChannelType) -> Result<(), ChainError> {
        if !self.is_channel_available(&channel) {
            return Err(ChainError::InternalError(format!("Canal {} non disponible", channel.name())));
        }
        self.active_channel = Some(channel);
        Ok(())
    }

    /// Désactive le canal actif
    pub fn deactivate(&mut self) {
        self.active_channel = None;
    }

    /// Vérifie si un canal est actif
    pub fn is_active(&self) -> bool {
        self.active_channel.is_some()
    }

    /// Retourne le canal actif
    pub fn get_active_channel(&self) -> Option<&PhantomChannelType> {
        self.active_channel.as_ref()
    }

    /// Envoie un message via le canal actif
    pub fn send_message(&mut self, payload: &[u8], priority: u8) -> Result<(), ChainError> {
        let channel = self.active_channel.clone()
            .ok_or_else(|| ChainError::InternalError("Aucun canal fantôme actif".into()))?;

        let max_payload = self.get_max_payload(&channel);
        if payload.len() > max_payload {
            return Err(ChainError::InternalError(
                format!("Payload trop grand ({} > {} octets)", payload.len(), max_payload)
            ));
        }

        let msg = PhantomMessage::new(channel, payload, self.sequence, priority);
        let encoded = msg.encode();

        // TODO: Envoyer via le canal approprié
        // match channel { ... }

        self.sequence = self.sequence.wrapping_add(1);
        self.messages_sent += 1;
        self.bytes_sent += encoded.len() as u64;

        Ok(())
    }

    /// Reçoit un message fantôme
    pub fn receive_message(&mut self, data: &[u8]) -> Result<PhantomMessage, ChainError> {
        let msg = PhantomMessage::decode(data)?;
        self.messages_received += 1;
        self.bytes_received += data.len() as u64;
        Ok(msg)
    }

    /// Active/désactive le mode découverte
    pub fn set_discovery_mode(&mut self, active: bool) {
        self.discovery_mode = active;
    }

    /// Vérifie si le mode découverte est actif
    pub fn is_discovering(&self) -> bool {
        self.discovery_mode
    }

    /// Retourne la taille maximale du payload par canal
    fn get_max_payload(&self, channel: &PhantomChannelType) -> usize {
        match channel {
            PhantomChannelType::Sms => 140,
            PhantomChannelType::Ussd => 182,
            PhantomChannelType::ControlPlane => 256,
            PhantomChannelType::CellBroadcast => 9600,    // ~9.6 kbps
            PhantomChannelType::PagingChannel => 800,      // ~800 bps
            PhantomChannelType::Rach => 4000,              // ~4 kbps
        }
    }

    /// Ajoute un canal disponible
    pub fn add_channel(&mut self, channel: PhantomChannelType) {
        if !self.available_channels.contains(&channel) {
            self.available_channels.push(channel);
        }
    }

    /// Retire un canal disponible
    pub fn remove_channel(&mut self, channel: &PhantomChannelType) {
        self.available_channels.retain(|c| c != channel);
        if self.active_channel.as_ref() == Some(channel) {
            self.active_channel = None;
        }
    }

    /// Retourne les statistiques
    pub fn get_stats(&self) -> PhantomStats {
        PhantomStats {
            available_channels: self.available_channels.len(),
            active_channel: self.active_channel.as_ref().map(|c| c.name().to_string()).unwrap(),
            is_active: self.is_active(),
            discovering: self.discovery_mode,
            messages_sent: self.messages_sent,
            messages_received: self.messages_received,
            bytes_sent: self.bytes_sent,
            bytes_received: self.bytes_received,
            sequence: self.sequence,
        }
    }
}

/// Statistiques des canaux fantômes
#[derive(Debug, Clone)]
pub struct PhantomStats {
    pub available_channels: usize,
    pub active_channel: String,
    pub is_active: bool,
    pub discovering: bool,
    pub messages_sent: u64,
    pub messages_received: u64,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub sequence: u8,
}

// ═══════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    // ─── Tests PhantomChannelType ──────────────

    #[test] fn test_channel_names() {
        assert_eq!(PhantomChannelType::Sms.name(), "SMS");
        assert_eq!(PhantomChannelType::Ussd.name(), "USSD");
        assert_eq!(PhantomChannelType::ControlPlane.name(), "Control Plane");
    }

    #[test] fn test_channel_bandwidth() {
        assert!(PhantomChannelType::Sms.bandwidth_bps() > 0);
        assert!(PhantomChannelType::ControlPlane.bandwidth_bps() > PhantomChannelType::Sms.bandwidth_bps());
    }

    #[test] fn test_channel_byte_roundtrip() {
        let types = vec![
            PhantomChannelType::Sms,
            PhantomChannelType::Ussd,
            PhantomChannelType::ControlPlane,
            PhantomChannelType::CellBroadcast,
            PhantomChannelType::PagingChannel,
            PhantomChannelType::Rach,
        ];
        for t in types {
            let byte = t.to_byte();
            let decoded = PhantomChannelType::from_byte(byte).unwrap();
            assert_eq!(t, decoded);
        }
    }

    #[test] fn test_channel_invalid_byte() {
        assert!(PhantomChannelType::from_byte(0xFF).is_err());
    }

    // ─── Tests PhantomMessage ──────────────────

    #[test] fn test_message_create() {
        let msg = PhantomMessage::new(PhantomChannelType::Sms, b"Hello", 0, 0);
        assert_eq!(msg.channel, PhantomChannelType::Sms);
        assert_eq!(msg.payload, b"Hello");
        assert!(!msg.is_urgent());
    }

    #[test] fn test_message_urgent() {
        let msg = PhantomMessage::new(PhantomChannelType::Sms, b"URGENT", 0, 1);
        assert!(msg.is_urgent());
    }

    #[test] fn test_message_encode_decode() {
        let msg = PhantomMessage::new(PhantomChannelType::Ussd, b"Test123", 5, 0);
        let encoded = msg.encode();
        let decoded = PhantomMessage::decode(&encoded).unwrap();
        assert_eq!(msg.channel, decoded.channel);
        assert_eq!(msg.payload, decoded.payload);
        assert_eq!(msg.sequence, decoded.sequence);
    }

    #[test] fn test_message_too_short() {
        assert!(PhantomMessage::decode(&[0x01]).is_err());
    }

    #[test] fn test_message_size() {
        let msg = PhantomMessage::new(PhantomChannelType::Sms, b"Hello", 0, 0);
        assert_eq!(msg.size(), 12 + 5); // 12 header + 5 payload
    }

    // ─── Tests PhantomChannelManager ───────────

    #[test] fn test_manager_new() {
        let m = PhantomChannelManager::new();
        assert!(m.is_discovering());
        assert!(!m.is_active());
        assert_eq!(m.get_available_channels().len(), 3);
    }

    #[test] fn test_activate_channel() {
        let mut m = PhantomChannelManager::new();
        assert!(m.activate_channel(PhantomChannelType::Sms).is_ok());
        assert!(m.is_active());
        assert_eq!(m.get_active_channel().unwrap(), &PhantomChannelType::Sms);
    }

    #[test] fn test_activate_unavailable() {
        let mut m = PhantomChannelManager::new();
        m.remove_channel(&PhantomChannelType::Sms);
        assert!(m.activate_channel(PhantomChannelType::Sms).is_err());
    }

    #[test] fn test_deactivate() {
        let mut m = PhantomChannelManager::new();
        m.activate_channel(PhantomChannelType::Sms).unwrap();
        m.deactivate();
        assert!(!m.is_active());
    }

    #[test] fn test_send_message() {
        let mut m = PhantomChannelManager::new();
        m.activate_channel(PhantomChannelType::Sms).unwrap();
        assert!(m.send_message(b"Hello UQ", 0).is_ok());
        assert_eq!(m.messages_sent, 1);
    }

    #[test] fn test_send_no_active_channel() {
        let mut m = PhantomChannelManager::new();
        assert!(m.send_message(b"Test", 0).is_err());
    }

    #[test] fn test_send_too_large() {
        let mut m = PhantomChannelManager::new();
        m.activate_channel(PhantomChannelType::Sms).unwrap();
        let large_payload = vec![0u8; 200]; // SMS max 140
        assert!(m.send_message(&large_payload, 0).is_err());
    }

    #[test] fn test_add_remove_channel() {
        let mut m = PhantomChannelManager::new();
        m.add_channel(PhantomChannelType::CellBroadcast);
        assert_eq!(m.get_available_channels().len(), 4);
        m.remove_channel(&PhantomChannelType::Sms);
        assert_eq!(m.get_available_channels().len(), 3);
    }

    #[test] fn test_discovery_mode() {
        let mut m = PhantomChannelManager::new();
        assert!(m.is_discovering());
        m.set_discovery_mode(false);
        assert!(!m.is_discovering());
    }

    #[test] fn test_stats() {
        let mut m = PhantomChannelManager::new();
        m.activate_channel(PhantomChannelType::Sms).unwrap();
        m.send_message(b"Test", 0).unwrap();
        let stats = m.get_stats();
        assert!(stats.is_active);
        assert_eq!(stats.messages_sent, 1);
        assert_eq!(stats.available_channels, 3);
    }
}


