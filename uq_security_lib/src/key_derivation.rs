// ═══════════════════════════════════════════════════════════════
// VTEE — Key Derivation (HKDF hiérarchique)
// ═══════════════════════════════════════════════════════════════

use uq_blockchain::error::ChainError;
use uq_black_key_standalone::UqBlackKey;
use sha2::Digest;

/// Contexte de dérivation (pour isolation)
#[derive(Debug, Clone)]
pub struct DerivationContext {
    pub purpose: String,
    pub index: u32,
    pub salt: [u8; 32],
}

/// Gestionnaire de dérivation de clés
pub struct KeyDerivation {
    master_key: UqBlackKey,
    derived_keys: Vec<[u8; 64]>,
    contexts: Vec<DerivationContext>,
}

impl KeyDerivation {
    /// Initialise avec la clé maître VTEE
    pub fn new() -> Result<Self, ChainError> {
        let mut master_key = UqBlackKey::generate();
        master_key.activate();
        master_key.mutate();
        
        Ok(KeyDerivation {
            master_key,
            derived_keys: Vec::new(),
            contexts: Vec::new(),
        })
    }

    /// Dérive une sous-clé pour un usage spécifique (HKDF simplifié)
    pub fn derive_key(&mut self, purpose: &str, index: u32) -> Result<[u8; 64], ChainError> {
        // Étape 1 : Extract (PRK)
        let mut extract_data = Vec::new();
        extract_data.extend_from_slice(purpose.as_bytes());
        extract_data.extend_from_slice(&index.to_le_bytes());
        let prk = self.master_key.sign(&extract_data);
        
        // Étape 2 : Expand (OKM)
        let mut expand_data = Vec::new();
        expand_data.extend_from_slice(&prk);
        expand_data.extend_from_slice(b"UQ_VTEE_KEY_EXPAND");
        expand_data.extend_from_slice(&index.to_le_bytes());
        
        let okm = self.master_key.sign(&expand_data);
        
        let mut derived = [0u8; 64];
        derived.copy_from_slice(&okm[..64]);
        
        // Enregistrer le contexte
        self.contexts.push(DerivationContext {
            purpose: purpose.to_string(),
            index,
            salt: prk[..32].try_into().unwrap_or([0u8; 32]),
        });
        
        self.derived_keys.push(derived);
        Ok(derived)
    }

    /// Dérive une clé enfant à partir d'une clé parent
    pub fn derive_child(&self, parent: &[u8; 64], child_index: u32) -> Result<[u8; 64], ChainError> {
        let mut data = Vec::new();
        data.extend_from_slice(parent);
        data.extend_from_slice(&child_index.to_le_bytes());
        data.extend_from_slice(b"UQ_VTEE_CHILD");
        
        let signature = self.master_key.sign(&data);
        let mut child = [0u8; 64];
        child.copy_from_slice(&signature[..64]);
        Ok(child)
    }

    /// Dérive une clé de session temporaire
    pub fn derive_session_key(&self, session_id: &[u8; 32]) -> Result<[u8; 64], ChainError> {
        let mut data = Vec::new();
        data.extend_from_slice(session_id);
        data.extend_from_slice(&chrono::Utc::now().timestamp().to_le_bytes());
        
        let signature = self.master_key.sign(&data);
        let mut session_key = [0u8; 64];
        session_key.copy_from_slice(&signature[..64]);
        Ok(session_key)
    }

    /// Efface toutes les clés dérivées
    pub fn purge(&mut self) {
        for key in &mut self.derived_keys {
            for b in key.iter_mut() { *b = 0; }
        }
        self.derived_keys.clear();
        self.contexts.clear();
    }

    pub fn key_count(&self) -> usize { self.derived_keys.len() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derive_key() {
        let mut kd = KeyDerivation::new().unwrap_or_default();
        let key1 = kd.derive_key("wallet", 0).unwrap_or_default();
        let key2 = kd.derive_key("wallet", 0).unwrap_or_default();
        assert_eq!(key1, key2); // Même entrée = même clé (déterministe)
    }

    #[test]
    fn test_different_purposes() {
        let mut kd = KeyDerivation::new().unwrap_or_default();
        let wallet_key = kd.derive_key("wallet", 0).unwrap_or_default();
        let chat_key = kd.derive_key("chat", 0).unwrap_or_default();
        assert_ne!(wallet_key, chat_key); // Différents = isolation
    }

    #[test]
    fn test_child_key() {
        let mut kd = KeyDerivation::new().unwrap_or_default();
        let parent = kd.derive_key("master", 0).unwrap_or_default();
        let child1 = kd.derive_child(&parent, 1).unwrap_or_default();
        let child2 = kd.derive_child(&parent, 2).unwrap_or_default();
        assert_ne!(child1, child2);
    }

    #[test]
    fn test_purge() {
        let mut kd = KeyDerivation::new().unwrap_or_default();
        kd.derive_key("test", 0).unwrap_or_default();
        kd.purge();
        assert_eq!(kd.key_count(), 0);
    }
}

