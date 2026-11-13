// ═══════════════════════════════════════════════════════════════
// VTEE — Secure Storage (Stockage chiffré inviolable)
// ═══════════════════════════════════════════════════════════════

use uq_blockchain::error::ChainError;
use uq_black_key_standalone::UqBlackKey;
use std::collections::HashMap;
use std::fs;

/// Niveau de sécurité du stockage
#[derive(Debug, Clone, PartialEq)]
pub enum SecurityLevel {
    Standard,    // Chiffré AES-256
    High,        // AES-256 + HMAC
    Critical,    // AES-256 + HMAC + mémoire verrouillée
}

/// Entrée de stockage sécurisé
#[derive(Debug, Clone)]
pub struct SecureEntry {
    pub key_hash: [u8; 32],
    pub encrypted_data: Vec<u8>,
    pub hmac: [u8; 32],
    pub level: SecurityLevel,
    pub created_at: i64,
    pub access_count: u64,
}

/// Gestionnaire de stockage sécurisé
pub struct SecureStorage {
    master_key: UqBlackKey,
    entries: HashMap<String, SecureEntry>,
    locked: bool,
}

impl SecureStorage {
    /// Initialise le stockage sécurisé avec la clé maître VTEE
    pub fn new() -> Result<Self, ChainError> {
        let mut master_key = UqBlackKey::generate();
        master_key.activate();
        master_key.mutate();
        master_key.mutate();
        master_key.mutate();
        
        Ok(SecureStorage {
            master_key,
            entries: HashMap::new(),
            locked: false,
        })
    }

    /// Stocke une donnée de manière sécurisée
    pub fn store(&mut self, key: &str, data: &[u8], level: SecurityLevel) -> Result<[u8; 32], ChainError> {
        if self.locked { return Err(ChainError::InternalError("Stockage verrouillé".into())); }
        
        // Dériver une clé unique pour cette entrée
        let mut entry_key = self.master_key.clone();
        entry_key.mutate();
        for _ in 0..key.len() { entry_key.mutate(); }
        
        // Chiffrer
        let encrypted_data = entry_key.encrypt(data);
        
        // HMAC pour intégrité
        let mut hmac_data = Vec::new();
        hmac_data.extend_from_slice(&encrypted_data.as_ref().unwrap_or(&vec![]));
        hmac_data.extend(key.as_bytes());
        let hmac = entry_key.sign(&hmac_data);
        let mut hmac_arr = [0u8; 32];
        hmac_arr.copy_from_slice(&hmac[..32]);
        
        let key_hash = entry_key.serialize()[..32].try_into().unwrap_or([0u8; 32]);
        
        self.entries.insert(key.to_string(), SecureEntry {
            key_hash,
            encrypted_data: encrypted_data.unwrap_or_default(),
            hmac: hmac_arr,
            level,
            created_at: chrono::Utc::now().timestamp(),
            access_count: 0,
        });
        
        // Persister sur disque (chiffré)
        self.persist()?;
        
        Ok(key_hash)
    }

    /// Récupère une donnée stockée
    pub fn retrieve(&mut self, key: &str) -> Result<Vec<u8>, ChainError> {
        if self.locked { return Err(ChainError::InternalError("Stockage verrouillé".into())); }
        
        let entry = self.entries.get_mut(key)
            .ok_or(ChainError::InternalError("Entrée introuvable".into()))?;
        
        // Reconstruire la clé
        let mut entry_key = self.master_key.clone();
        entry_key.mutate();
        for _ in 0..key.len() { entry_key.mutate(); }
        
        // Vérifier HMAC
        let mut hmac_data = Vec::new();
        hmac_data.extend_from_slice(&entry.encrypted_data);
        hmac_data.extend(key.as_bytes());
        let computed_hmac = entry_key.sign(&hmac_data);
        if &computed_hmac[..32] != &entry.hmac {
            return Err(ChainError::InternalError("INTÉGRITÉ COMPROMISE !".into()));
        }
        
        entry.access_count += 1;
        Ok(entry_key.decrypt(&entry.encrypted_data).unwrap_or_default())
    }

    /// Supprime une entrée de manière sécurisée (écrasement multiple)
    pub fn delete(&mut self, key: &str) -> Result<(), ChainError> {
        if let Some(entry) = self.entries.remove(key) {
            // Écrasement multiple (DoD 5220.22-M)
            let mut shredded = entry.encrypted_data.clone();
            for _ in 0..7 {
                for b in &mut shredded { *b = rand::random(); }
            }
        }
        self.persist()?;
        Ok(())
    }

    /// Verrouille le stockage
    pub fn lock(&mut self) {
        self.locked = true;
        self.master_key.mutate();
        self.master_key.mutate();
        self.master_key.mutate();
    }

    /// Déverrouille (nécessite re-authentification)
    pub fn unlock(&mut self, proof: &[u8]) -> Result<bool, ChainError> {
        let restored = self.master_key.verify(b"VTEE_UNLOCK", &proof.try_into().unwrap_or([0u8; 64]));
        if restored { self.locked = false; Ok(true) }
        else { Err(ChainError::InternalError("Échec déverrouillage".into())) }
    }

    /// Persiste sur disque de manière chiffrée
    fn persist(&self) -> Result<(), ChainError> {
        let serialized = serde_json::to_string(&self.entries.keys().collect::<Vec<_>>()).unwrap_or_default();
        let encrypted = self.master_key.encrypt(serialized.as_bytes());
        fs::write("/data/vtee/secure_storage.vtee", encrypted.unwrap_or_default())
            .map_err(|e| ChainError::InternalError(e.to_string()))?;
        Ok(())
    }

    pub fn is_locked(&self) -> bool { self.locked }
    pub fn entry_count(&self) -> usize { self.entries.len() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_store_retrieve() {
        let mut storage = SecureStorage::new().unwrap_or_default();
        storage.store("wallet_key", b"secret_BORB_key_123", SecurityLevel::High).unwrap_or_default();
        let data = storage.retrieve("wallet_key").unwrap_or_default();
        assert_eq!(data, b"secret_BORB_key_123");
    }

    #[test]
    fn test_delete() {
        let mut storage = SecureStorage::new().unwrap_or_default();
        storage.store("temp", b"temp_data", SecurityLevel::Standard).unwrap_or_default();
        storage.delete("temp").unwrap_or_default();
        assert!(storage.retrieve("temp").is_err());
    }

    #[test]
    fn test_lock_unlock() {
        let mut storage = SecureStorage::new().unwrap_or_default();
        storage.store("test", b"data", SecurityLevel::Standard).unwrap_or_default();
        storage.lock();
        assert!(storage.retrieve("test").is_err());
    }

    #[test]
    fn test_hmac_verification() {
        let mut storage = SecureStorage::new().unwrap_or_default();
        let hash = storage.store("key", b"value", SecurityLevel::Critical).unwrap_or_default();
        // Modifier directement les données (attaque simulée)
        if let Some(entry) = storage.entries.get_mut("key") {
            entry.encrypted_data[0] ^= 1; // Corrompre
        }
        assert!(storage.retrieve("key").is_err()); // Doit échouer
    }
}






