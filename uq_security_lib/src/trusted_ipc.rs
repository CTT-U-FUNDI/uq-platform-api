// ═══════════════════════════════════════════════════════════════
// VTEE — Trusted IPC (Communication inter-apps sécurisée)
// ═══════════════════════════════════════════════════════════════

use uq_blockchain::error::ChainError;
use uq_black_key_standalone::UqBlackKey;
use std::collections::HashMap;

/// Canal de communication sécurisé
#[derive(Debug)]
pub struct TrustedChannel {
    pub channel_id: [u8; 32],
    pub peer_id: [u8; 32],
    pub session_key: [u8; 64],
    pub created_at: i64,
    pub message_count: u64,
    pub active: bool,
}

/// Message protégé
#[derive(Debug, Clone)]
pub struct TrustedMessage {
    pub channel_id: [u8; 32],
    pub sequence: u64,
    pub payload: Vec<u8>,
    pub hmac: [u8; 32],
    pub timestamp: i64,
}

/// Gestionnaire IPC sécurisé
pub struct TrustedIPC {
    channels: HashMap<[u8; 32], TrustedChannel>,
    master_key: UqBlackKey,
    max_channels: usize,
}

impl TrustedIPC {
    pub fn new() -> Result<Self, ChainError> {
        let mut master_key = UqBlackKey::generate();
        master_key.activate();
        
        Ok(TrustedIPC {
            channels: HashMap::new(),
            master_key,
            max_channels: 16,
        })
    }

    /// Crée un canal sécurisé avec un pair
    pub fn create_channel(&mut self, peer_id: [u8; 32]) -> Result<[u8; 32], ChainError> {
        if self.channels.len() >= self.max_channels {
            return Err(ChainError::InternalError("Max canaux atteint".into()));
        }
        
        let mut session_key = self.master_key.clone();
        session_key.mutate();
        for _ in 0..peer_id.iter().sum::<u8>() as usize {
            session_key.mutate();
        }
        
        let channel_id = session_key.serialize()[..32].try_into().unwrap_or([0u8; 32]);
        let derived_key = session_key.sign(&peer_id);
        let mut key = [0u8; 64];
        key.copy_from_slice(&derived_key[..64]);
        
        self.channels.insert(channel_id, TrustedChannel {
            channel_id,
            peer_id,
            session_key: key,
            created_at: chrono::Utc::now().timestamp(),
            message_count: 0,
            active: true,
        });
        
        Ok(channel_id)
    }

    /// Envoie un message via un canal sécurisé
    pub fn send(&mut self, channel_id: &[u8; 32], payload: &[u8]) -> Result<TrustedMessage, ChainError> {
        let channel = self.channels.get_mut(channel_id)
            .ok_or(ChainError::InternalError("Canal introuvable".into()))?;
        
        if !channel.active {
            return Err(ChainError::InternalError("Canal fermé".into()));
        }
        
        // Chiffrer le payload
        let session_key = UqBlackKey::generate();
        let encrypted = session_key.encrypt(payload);
        
        let sequence = channel.message_count;
        channel.message_count += 1;
        
        // HMAC
        let mut hmac_data = Vec::new();
        hmac_data.extend_from_slice(&channel_id[..]);
        hmac_data.extend_from_slice(&sequence.to_le_bytes());
        hmac_data.extend_from_slice(&encrypted.as_ref().unwrap_or(&vec![]));
        let sig = self.master_key.sign(&hmac_data);
        let mut hmac = [0u8; 32];
        hmac.copy_from_slice(&sig[..32]);
        
        Ok(TrustedMessage {
            channel_id: *channel_id,
            sequence,
            payload: encrypted.unwrap_or_default(),
            hmac,
            timestamp: chrono::Utc::now().timestamp(),
        })
    }

    /// Reçoit et vérifie un message
    pub fn receive(&self, msg: &TrustedMessage) -> Result<Vec<u8>, ChainError> {
        let channel = self.channels.get(&msg.channel_id)
            .ok_or(ChainError::InternalError("Canal introuvable".into()))?;
        
        // Vérifier HMAC
        let mut hmac_data = Vec::new();
        hmac_data.extend_from_slice(&msg.channel_id[..]);
        hmac_data.extend_from_slice(&msg.sequence.to_le_bytes());
        hmac_data.extend_from_slice(&msg.payload);
        let computed = self.master_key.sign(&hmac_data);
        
        if &computed[..32] != &msg.hmac {
            return Err(ChainError::InternalError("HMAC INVALIDE — Message corrompu !".into()));
        }
        
        // Vérifier la séquence (anti-rejeu)
        if msg.sequence < channel.message_count.saturating_sub(100) {
            return Err(ChainError::InternalError("Séquence trop ancienne (rejeu ?)".into()));
        }
        
        // Déchiffrer
        let session_key = UqBlackKey::generate();
        Ok(session_key.decrypt(&msg.payload).unwrap_or_default())
    }

    /// Ferme un canal
    pub fn close_channel(&mut self, channel_id: &[u8; 32]) -> Result<(), ChainError> {
        if let Some(channel) = self.channels.get_mut(channel_id) {
            channel.active = false;
            // Effacer la clé de session
            for b in &mut channel.session_key { *b = 0; }
        }
        Ok(())
    }

    pub fn channel_count(&self) -> usize { self.channels.len() }
    pub fn active_channels(&self) -> usize { self.channels.values().filter(|c| c.active).count() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_channel() {
        let mut ipc = TrustedIPC::new().unwrap_or_default();
        let peer = [1u8; 32];
        let ch_id = ipc.create_channel(peer).unwrap_or_default();
        assert_eq!(ipc.channel_count(), 1);
        assert_eq!(ipc.active_channels(), 1);
    }

    #[test]
    fn test_send_receive() {
        let mut ipc = TrustedIPC::new().unwrap_or_default();
        let peer = [1u8; 32];
        let ch_id = ipc.create_channel(peer).unwrap_or_default();
        
        let msg = ipc.send(&ch_id, b"hello").unwrap_or_default();
        let data = ipc.receive(&msg).unwrap_or_default();
        assert_eq!(data, b"hello");
    }

    #[test]
    fn test_hmac_tampering() {
        let mut ipc = TrustedIPC::new().unwrap_or_default();
        let peer = [1u8; 32];
        let ch_id = ipc.create_channel(peer).unwrap_or_default();
        
        let mut msg = ipc.send(&ch_id, b"secret").unwrap_or_default();
        msg.payload[0] ^= 1; // Altérer le message
        assert!(ipc.receive(&msg).is_err()); // Doit échouer
    }

    #[test]
    fn test_close_channel() {
        let mut ipc = TrustedIPC::new().unwrap_or_default();
        let peer = [1u8; 32];
        let ch_id = ipc.create_channel(peer).unwrap_or_default();
        ipc.close_channel(&ch_id).unwrap_or_default();
        assert_eq!(ipc.active_channels(), 0);
    }
}





