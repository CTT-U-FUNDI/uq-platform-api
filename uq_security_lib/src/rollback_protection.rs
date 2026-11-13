// ═══════════════════════════════════════════════════════════════
// VTEE — Rollback Protection (Anti-retour version)
// ═══════════════════════════════════════════════════════════════

use uq_blockchain::error::ChainError;
use std::fs;
use std::path::Path;

/// Protection anti-retour version
pub struct RollbackProtection {
    current_version: u64,
    min_allowed_version: u64,
    storage_path: String,
    counter: u64,
}

impl RollbackProtection {
    /// Initialise la protection avec la version actuelle
    pub fn new(current_version: u64, storage_path: &str) -> Result<Self, ChainError> {
        let path = Path::new(storage_path);
        
        // Lire la version stockée précédemment
        let stored_version = if path.exists() {
            let data = fs::read_to_string(path)
                .map_err(|e| ChainError::InternalError(e.to_string()))?;
            data.trim().parse::<u64>().unwrap_or(0)
        } else {
            0
        };
        
        // Si la version actuelle est plus petite → rollback détecté !
        if current_version < stored_version {
            return Err(ChainError::InternalError(
                format!("🛑 ROLLBACK DÉTECTÉ ! v{} < v{}", current_version, stored_version)
            ));
        }
        
        // Persister la nouvelle version
        fs::write(path, current_version.to_string())
            .map_err(|e| ChainError::InternalError(e.to_string()))?;
        
        Ok(RollbackProtection {
            current_version,
            min_allowed_version: stored_version.max(current_version),
            storage_path: storage_path.to_string(),
            counter: 0,
        })
    }

    /// Vérifie que la version n'a pas reculé
    pub fn verify(&self, version: u64) -> Result<bool, ChainError> {
        if version < self.min_allowed_version {
            return Err(ChainError::InternalError(
                format!("🛑 TENTATIVE ROLLBACK: v{} < v{}", version, self.min_allowed_version)
            ));
        }
        Ok(true)
    }

    /// Incrémente le compteur anti-retour
    pub fn increment(&mut self) -> Result<u64, ChainError> {
        self.counter += 1;
        
        // Persister le compteur
        let counter_path = format!("{}.counter", self.storage_path);
        fs::write(&counter_path, self.counter.to_string())
            .map_err(|e| ChainError::InternalError(e.to_string()))?;
        
        Ok(self.counter)
    }

    /// Obtient le compteur actuel
    pub fn get_counter(&self) -> u64 { self.counter }

    /// Vérifie que le compteur n'a pas reculé
    pub fn verify_counter(&self, counter: u64) -> Result<bool, ChainError> {
        if counter < self.counter {
            return Err(ChainError::InternalError(
                format!("🛑 COMPTEUR ROLLBACK: {} < {}", counter, self.counter)
            ));
        }
        Ok(true)
    }

    /// Met à jour la version minimale
    pub fn update_min_version(&mut self, new_version: u64) -> Result<(), ChainError> {
        if new_version < self.min_allowed_version {
            return Err(ChainError::InternalError("Rollback interdit".into()));
        }
        self.min_allowed_version = new_version;
        fs::write(&self.storage_path, new_version.to_string())
            .map_err(|e| ChainError::InternalError(e.to_string()))?;
        Ok(())
    }

    pub fn stats(&self) -> RollbackStats {
        RollbackStats {
            current_version: self.current_version,
            min_allowed_version: self.min_allowed_version,
            counter: self.counter,
        }
    }
}

#[derive(Debug)]
pub struct RollbackStats {
    pub current_version: u64,
    pub min_allowed_version: u64,
    pub counter: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_version() {
        let rb = RollbackProtection::new(3, "/tmp/vtee_test_version.txt").unwrap_or_default();
        assert!(rb.verify(3).unwrap_or_default());
        assert!(rb.verify(5).unwrap_or_default());
        assert!(rb.verify(1).is_err()); // Rollback !
    }

    #[test]
    fn test_counter() {
        let mut rb = RollbackProtection::new(1, "/tmp/vtee_test_counter.txt").unwrap_or_default();
        assert_eq!(rb.increment().unwrap_or_default(), 1);
        assert_eq!(rb.increment().unwrap_or_default(), 2);
        assert_eq!(rb.get_counter(), 2);
    }

    #[test]
    fn test_counter_rollback() {
        let mut rb = RollbackProtection::new(1, "/tmp/vtee_test_counter2.txt").unwrap_or_default();
        rb.increment().unwrap_or_default();
        rb.increment().unwrap_or_default();
        assert!(rb.verify_counter(0).is_err());
    }
}

