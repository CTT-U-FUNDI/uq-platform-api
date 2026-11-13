//! UQ Acoustic Module - Communication par Ultrasons
//! Dernier recours quand Internet est coupé.
//! Utilise les hauts-parleurs et micros pour transmettre des données.

use crate::crypto::sha256;
use crate::error::UqError;

/// Fréquence porteuse pour les ultrasons (18-20 kHz, inaudible)
pub const CARRIER_FREQ: u32 = 19000;

/// Taille maximale d'un message acoustique en octets
pub const MAX_ACOUSTIC_PAYLOAD: usize = 256;

/// Préambule acoustique pour synchronisation
pub const ACOUSTIC_PREAMBLE: [u8; 4] = [0xAA, 0x55, 0xAA, 0x55];

/// Structure représentant le module acoustique
#[derive(Debug)]
pub struct UqAcoustic {
    /// Indique si le module est en écoute
    is_listening: bool,
    /// Dernier message reçu (simulé)
    last_received: Option<Vec<u8>>,
    /// Compteur de messages envoyés
    sent_count: u64,
    /// Compteur de messages reçus
    received_count: u64,
}

impl UqAcoustic {
    /// Crée un nouveau module acoustique.
    pub fn new() -> Self {
        UqAcoustic {
            is_listening: false,
            last_received: None,
            sent_count: 0,
            received_count: 0,
        }
    }

    /// Démarre l'écoute ultrasonique.
    pub fn start_listening(&mut self) -> Result<(), UqError> {
        if self.is_listening {
            return Err(UqError::AlreadyInitialized);
        }
        self.is_listening = true;
        Ok(())
    }

    /// Arrête l'écoute ultrasonique.
    pub fn stop_listening(&mut self) {
        self.is_listening = false;
    }

    /// Encode des données en signal acoustique.
    ///
    /// # Format du signal
    /// ```text
    /// [PREAMBLE: 4][CHECKSUM: 4][LENGTH: 2][DATA: variable]
    /// ```
    pub fn encode_signal(&self, data: &[u8]) -> Result<Vec<u8>, UqError> {
        if data.len() > MAX_ACOUSTIC_PAYLOAD {
            return Err(UqError::InternalError(format!(
                "Payload trop grand: {} octets (max: {})",
                data.len(),
                MAX_ACOUSTIC_PAYLOAD
            )));
        }

        let mut signal = Vec::with_capacity(10 + data.len());

        // Préambule (4 octets)
        signal.extend_from_slice(&ACOUSTIC_PREAMBLE);

        // Checksum (4 octets) = SHA256(data)[0..4]
        let checksum = &sha256(data)[0..4];
        signal.extend_from_slice(checksum);

        // Longueur (2 octets) en big-endian
        let len_bytes = (data.len() as u16).to_be_bytes();
        signal.extend_from_slice(&len_bytes);

        // Données
        signal.extend_from_slice(data);

        Ok(signal)
    }

    /// Décode un signal acoustique reçu.
    pub fn decode_signal(&self, signal: &[u8]) -> Result<Vec<u8>, UqError> {
        if signal.len() < 10 {
            return Err(UqError::InternalError("Signal trop court".to_string()));
        }

        // Vérifier le préambule
        if signal[0..4] != ACOUSTIC_PREAMBLE {
            return Err(UqError::InternalError("Préambule invalide".to_string()));
        }

        // Lire la longueur
        let length = u16::from_be_bytes([signal[8], signal[9]]) as usize;

        if signal.len() < 10 + length {
            return Err(UqError::InternalError("Signal tronqué".to_string()));
        }

        // Extraire les données
        let data = &signal[10..10 + length];

        // Vérifier le checksum
        let received_checksum = &signal[4..8];
        let computed_checksum = &sha256(data)[0..4];

        if received_checksum != computed_checksum {
            return Err(UqError::InternalError("Checksum invalide".to_string()));
        }

        Ok(data.to_vec())
    }

    /// Simule l'envoi de données par ultrasons.
    pub fn send_data(&mut self, data: &[u8]) -> Result<(), UqError> {
        let _signal = self.encode_signal(data)?;
        self.sent_count += 1;
        Ok(())
    }

    /// Simule la réception de données par ultrasons.
    pub fn receive_data(&mut self, signal: &[u8]) -> Result<Vec<u8>, UqError> {
        let data = self.decode_signal(signal)?;
        self.received_count += 1;
        self.last_received = Some(data.clone());
        Ok(data)
    }

    /// Vérifie si le module est en écoute.
    pub fn is_available(&self) -> bool {
        self.is_listening
    }

    /// Retourne le nombre de messages envoyés.
    pub fn get_sent_count(&self) -> u64 {
        self.sent_count
    }

    /// Retourne le nombre de messages reçus.
    pub fn get_received_count(&self) -> u64 {
        self.received_count
    }
}

/// ─── TESTS ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_start_stop_listening() {
        let mut acoustic = UqAcoustic::new();
        assert!(!acoustic.is_available());

        acoustic.start_listening().unwrap_or_default();
        assert!(acoustic.is_available());

        acoustic.stop_listening();
        assert!(!acoustic.is_available());
    }

    #[test]
    fn test_encode_decode() {
        let acoustic = UqAcoustic::new();
        let data = b"Hello acoustic world!";
        
        let signal = acoustic.encode_signal(data).unwrap_or_default();
        let decoded = acoustic.decode_signal(&signal).unwrap_or_default();
        
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_encode_too_large() {
        let acoustic = UqAcoustic::new();
        let data = vec![0u8; MAX_ACOUSTIC_PAYLOAD + 1];
        assert!(acoustic.encode_signal(&data).is_err());
    }

    #[test]
    fn test_decode_invalid_preamble() {
        let acoustic = UqAcoustic::new();
        let signal = vec![0xFF; 20];
        assert!(acoustic.decode_signal(&signal).is_err());
    }

    #[test]
    fn test_decode_too_short() {
        let acoustic = UqAcoustic::new();
        assert!(acoustic.decode_signal(&[1, 2, 3]).is_err());
    }

    #[test]
    fn test_send_and_receive() {
        let mut acoustic = UqAcoustic::new();
        let data = b"Secret acoustic message";

        acoustic.send_data(data).unwrap_or_default();
        assert_eq!(acoustic.get_sent_count(), 1);

        // Simuler la réception
        let signal = acoustic.encode_signal(data).unwrap_or_default();
        let received = acoustic.receive_data(&signal).unwrap_or_default();
        assert_eq!(received, data);
        assert_eq!(acoustic.get_received_count(), 1);
    }

    #[test]
    fn test_checksum_detects_corruption() {
        let acoustic = UqAcoustic::new();
        let data = b"test";
        let mut signal = acoustic.encode_signal(data).unwrap_or_default();

        // Corrompre les données
        signal[12] ^= 0xFF;

        assert!(acoustic.decode_signal(&signal).is_err());
    }
}
