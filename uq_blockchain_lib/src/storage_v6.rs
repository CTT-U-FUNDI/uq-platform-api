use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use crate::error::ChainError;
use crate::state::PERSISTENCE_MAGIC;

// ═══════════════════════════════════════════════════════════════
// STORAGE ENGINE — Abstraction pour RocksDB (et mock pour tests)
// ═══════════════════════════════════════════════════════════════

pub trait StorageBackend: Send + Sync {
    fn put(&mut self, key: &[u8], value: &[u8]) -> Result<(), ChainError>;
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, ChainError>;
    fn delete(&mut self, key: &[u8]) -> Result<(), ChainError>;
    fn exists(&self, key: &[u8]) -> Result<bool, ChainError>;
    fn flush(&mut self) -> Result<(), ChainError>;
    fn snapshot(&self) -> Result<HashMap<Vec<u8>, Vec<u8>>, ChainError>;
}

/// Implémentation en mémoire pour les tests
pub struct MemoryStorage {
    data: HashMap<Vec<u8>, Vec<u8>>,
}

impl MemoryStorage {
    pub fn new() -> Self { MemoryStorage { data: HashMap::new() } }
}

impl StorageBackend for MemoryStorage {
    fn put(&mut self, key: &[u8], value: &[u8]) -> Result<(), ChainError> {
        self.data.insert(key.to_vec(), value.to_vec()); Ok(())
    }
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, ChainError> {
        Ok(self.data.get(key).cloned())
    }
    fn delete(&mut self, key: &[u8]) -> Result<(), ChainError> {
        self.data.remove(key); Ok(())
    }
    fn exists(&self, key: &[u8]) -> Result<bool, ChainError> {
        Ok(self.data.contains_key(key))
    }
    fn flush(&mut self) -> Result<(), ChainError> { Ok(()) }
    fn snapshot(&self) -> Result<HashMap<Vec<u8>, Vec<u8>>, ChainError> {
        Ok(self.data.clone())
    }
}

// ═══════════════════════════════════════════════════════════════
// PERSISTENCE MANAGER
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistenceHeader {
    pub magic: u32,
    pub version: u32,
    pub created_at: u64,
    pub last_compaction: u64,
    pub total_writes: u64,
    pub total_reads: u64,
    pub data_size_bytes: u64,
}

pub struct PersistenceManager<B: StorageBackend> {
    backend: B,
    header: PersistenceHeader,
    prefix: String,
}

impl<B: StorageBackend> PersistenceManager<B> {
    pub fn new(mut backend: B, prefix: &str) -> Result<Self, ChainError> {
        let now = utc_now();
        let header_key = format!("{}:header", prefix);
        
        let header = if backend.exists(header_key.as_bytes())? {
            // Charger le header existant
            if let Some(data) = backend.get(header_key.as_bytes())? {
                bincode::deserialize(&data).map_err(|e| ChainError::InternalError(e.to_string()))?
            } else {
                PersistenceHeader {
                    magic: PERSISTENCE_MAGIC, version: 6, created_at: now,
                    last_compaction: 0, total_writes: 0, total_reads: 0, data_size_bytes: 0,
                }
            }
        } else {
            let header = PersistenceHeader {
                magic: PERSISTENCE_MAGIC, version: 6, created_at: now,
                last_compaction: 0, total_writes: 0, total_reads: 0, data_size_bytes: 0,
            };
            let data = bincode::serialize(&header).map_err(|e| ChainError::InternalError(e.to_string()))?;
            backend.put(header_key.as_bytes(), &data)?;
            header
        };
        
        if header.magic != PERSISTENCE_MAGIC {
            return Err(ChainError::InternalError("Magic invalide — fichier corrompu".into()));
        }
        
        Ok(PersistenceManager {
            backend, header,
            prefix: prefix.to_string(),
        })
    }

    /// Sauvegarde une valeur
    pub fn save(&mut self, key: &str, value: &[u8]) -> Result<(), ChainError> {
        let full_key = format!("{}:{}", self.prefix, key);
        self.backend.put(full_key.as_bytes(), value)?;
        self.header.total_writes += 1;
        self.header.data_size_bytes += value.len() as u64;
        self.save_header()?;
        Ok(())
    }

    /// Sauvegarde un objet sérialisable
    pub fn save_object<T: Serialize>(&mut self, key: &str, obj: &T) -> Result<(), ChainError> {
        let data = bincode::serialize(obj).map_err(|e| ChainError::InternalError(e.to_string()))?;
        self.save(key, &data)
    }

    /// Charge une valeur
    pub fn load(&mut self, key: &str) -> Result<Option<Vec<u8>>, ChainError> {
        let full_key = format!("{}:{}", self.prefix, key);
        self.header.total_reads += 1;
        self.backend.get(full_key.as_bytes())
    }

    /// Charge un objet désérialisable
    pub fn load_object<T: for<'a> Deserialize<'a>>(&mut self, key: &str) -> Result<Option<T>, ChainError> {
        if let Some(data) = self.load(key)? {
            let obj = bincode::deserialize(&data).map_err(|e| ChainError::InternalError(e.to_string()))?;
            Ok(Some(obj))
        } else {
            Ok(None)
        }
    }

    /// Supprime une clé
    pub fn delete(&mut self, key: &str) -> Result<(), ChainError> {
        let full_key = format!("{}:{}", self.prefix, key);
        self.backend.delete(full_key.as_bytes())
    }

    /// Vérifie si une clé existe
    pub fn exists(&self, key: &str) -> Result<bool, ChainError> {
        let full_key = format!("{}:{}", self.prefix, key);
        self.backend.exists(full_key.as_bytes())
    }

    /// Force l'écriture sur disque
    pub fn flush(&mut self) -> Result<(), ChainError> {
        self.save_header()?;
        self.backend.flush()
    }

    /// Retourne les statistiques
    pub fn get_stats(&self) -> &PersistenceHeader { &self.header }

    fn save_header(&mut self) -> Result<(), ChainError> {
        let key = format!("{}:header", self.prefix);
        let data = bincode::serialize(&self.header).map_err(|e| ChainError::InternalError(e.to_string()))?;
        self.backend.put(key.as_bytes(), &data)
    }
}

fn utc_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
}

// ═══════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    fn create_manager() -> PersistenceManager<MemoryStorage> {
        PersistenceManager::new(MemoryStorage::new(), "test").unwrap()
    }

    #[test] fn test_new() {
        let m = create_manager();
        assert_eq!(m.header.magic, PERSISTENCE_MAGIC);
        assert_eq!(m.header.version, 6);
    }

    #[test] fn test_save_load() {
        let mut m = create_manager();
        m.save("key1", b"hello").unwrap();
        let val = m.load("key1").unwrap();
        assert_eq!(val, Some(b"hello".to_vec()));
    }

    #[test] fn test_save_load_object() {
        #[derive(Debug, Serialize, Deserialize, PartialEq)]
        struct TestData { name: String, value: u64 }
        let mut m = create_manager();
        let obj = TestData { name: "test".into(), value: 42 };
        m.save_object("obj1", &obj).unwrap();
        let loaded: TestData = m.load_object("obj1").unwrap().unwrap();
        assert_eq!(loaded, obj);
    }

    #[test] fn test_delete() {
        let mut m = create_manager();
        m.save("key1", b"data").unwrap();
        assert!(m.exists("key1").unwrap());
        m.delete("key1").unwrap();
        assert!(!m.exists("key1").unwrap());
    }

    #[test] fn test_overwrite() {
        let mut m = create_manager();
        m.save("key1", b"v1").unwrap();
        m.save("key1", b"v2").unwrap();
        assert_eq!(m.load("key1").unwrap(), Some(b"v2".to_vec()));
    }

    #[test] fn test_stats() {
        let mut m = create_manager();
        let before = m.get_stats().total_writes;
        m.save("k1", b"12345").unwrap();
        m.load("k1").unwrap();
        let stats = m.get_stats();
        assert!(stats.total_writes > before);
        assert_eq!(stats.total_reads, 1);
    }

    #[test] fn test_flush() {
        let mut m = create_manager();
        m.save("k1", b"data").unwrap();
        assert!(m.flush().is_ok());
    }

    #[test] fn test_bad_magic_rejected() {
        let mut backend = MemoryStorage::new();
        // Écrire un mauvais magic
        let bad_header = PersistenceHeader {
            magic: 0xDEAD, version: 6, created_at: 0,
            last_compaction: 0, total_writes: 0, total_reads: 0, data_size_bytes: 0,
        };
        let data = bincode::serialize(&bad_header).unwrap();
        backend.put(b"test:header", &data).unwrap();
        assert!(PersistenceManager::new(backend, "test").is_err());
    }
}
