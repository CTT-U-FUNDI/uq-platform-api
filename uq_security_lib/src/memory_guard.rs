// ═══════════════════════════════════════════════════════════════
// VTEE — Memory Guard (RAM chiffrée)
// ═══════════════════════════════════════════════════════════════

use uq_blockchain::error::ChainError;
use uq_black_key_standalone::UqBlackKey;
use std::collections::HashMap;

/// Page mémoire protégée
struct GuardedPage {
    data: Vec<u8>,
    tag: [u8; 16],
    integrity: [u8; 32],
    encrypted: bool,
}

/// Gardien de mémoire
pub struct MemoryGuard {
    pages: HashMap<u64, GuardedPage>,
    session_key: UqBlackKey,
    total_guarded_bytes: u64,
    integrity_failures: u64,
}

impl MemoryGuard {
    pub fn new() -> Result<Self, ChainError> {
        let mut session_key = UqBlackKey::generate();
        session_key.activate();
        session_key.mutate();
        
        Ok(MemoryGuard {
            pages: HashMap::new(),
            session_key,
            total_guarded_bytes: 0,
            integrity_failures: 0,
        })
    }

    /// Alloue une page mémoire protégée
    pub fn allocate(&mut self, size: usize) -> Result<u64, ChainError> {
        let page_id = rand::random::<u64>();
        let data = vec![0u8; size];
        
        let page = GuardedPage {
            data: data.clone(),
            tag: self.generate_tag(&data),
            integrity: self.compute_integrity(&data, &[]),
            encrypted: false,
        };
        
        self.total_guarded_bytes += size as u64;
        self.pages.insert(page_id, page);
        Ok(page_id)
    }

    /// Écrit dans une page protégée (chiffrement automatique)
    pub fn write(&mut self, page_id: u64, offset: usize, data: &[u8]) -> Result<(), ChainError> {
    let encrypted = self.session_key.encrypt(data).unwrap_or_default();
    
    // Étape 1 : Mettre à jour les données
    {
        let page = self.pages.get_mut(&page_id)
            .ok_or(ChainError::InternalError("Page introuvable".into()))?;
        
        if offset + data.len() > page.data.len() {
            return Err(ChainError::InternalError("Débordement mémoire".into()));
        }
        
        page.data[offset..offset + data.len()].copy_from_slice(&encrypted[..data.len()]);
        page.encrypted = true;
    }
    
    // Étape 2 : Calculer tag et intégrité (après le borrow mutable)
    let (tag, integrity) = {
        let page = self.pages.get(&page_id).unwrap();
        let tag = self.generate_tag(&page.data);
        let integrity = self.compute_integrity(&page.data, &tag);
        (tag, integrity)
    };
    
    // Étape 3 : Mettre à jour tag et intégrité
    {
        let page = self.pages.get_mut(&page_id).unwrap();
        page.tag = tag;
        page.integrity = integrity;
    }
    
    Ok(())
}

    /// Lit depuis une page protégée (déchiffrement automatique)
    pub fn read(&self, page_id: u64, offset: usize, length: usize) -> Result<Vec<u8>, ChainError> {
        let page = self.pages.get(&page_id)
            .ok_or(ChainError::InternalError("Page introuvable".into()))?;
        
        // Vérifier l'intégrité avant de lire
        let computed = self.compute_integrity(&page.data, &page.tag);
        if computed != page.integrity {
            return Err(ChainError::InternalError("INTÉGRITÉ MÉMOIRE COMPROMISE !".into()));
        }
        
        if page.encrypted {
            Ok(self.session_key.decrypt(&page.data[offset..offset + length]).unwrap_or_default())
        } else {
            Ok(page.data[offset..offset + length].to_vec())
        }
    }

    /// Libère une page (écrasement sécurisé)
    pub fn free(&mut self, page_id: u64) -> Result<(), ChainError> {
        if let Some(mut page) = self.pages.remove(&page_id) {
            // Écrasement sécurisé avant libération
            for b in &mut page.data { *b = 0; }
            self.total_guarded_bytes -= page.data.len() as u64;
        }
        Ok(())
    }

    /// Vérifie l'intégrité de toutes les pages
    pub fn verify_all(&mut self) -> Result<u64, ChainError> {
        let mut failures = 0u64;
        for (_id, page) in &self.pages {
            let computed = self.compute_integrity(&page.data, &page.tag);
            if computed != page.integrity {
                failures += 1;
            }
        }
        self.integrity_failures += failures;
        Ok(failures)
    }

    /// Détruit la session (efface tout)
    pub fn destroy(&mut self) {
        for (_, mut page) in self.pages.drain() {
            for b in &mut page.data { *b = 0; }
        }
        self.session_key.mutate();
        self.session_key.mutate();
        self.total_guarded_bytes = 0;
    }

    fn generate_tag(&self, data: &[u8]) -> [u8; 16] {
        let sig = self.session_key.sign(data);
        let mut tag = [0u8; 16];
        tag.copy_from_slice(&sig[..16]);
        tag
    }

    fn compute_integrity(&self, data: &[u8], tag: &[u8]) -> [u8; 32] {
        let mut combined = Vec::new();
        combined.extend_from_slice(data);
        combined.extend_from_slice(tag);
        let sig = self.session_key.sign(&combined);
        let mut integrity = [0u8; 32];
        integrity.copy_from_slice(&sig[..32]);
        integrity
    }

    pub fn stats(&self) -> MemoryGuardStats {
        MemoryGuardStats {
            page_count: self.pages.len() as u64,
            total_bytes: self.total_guarded_bytes,
            integrity_failures: self.integrity_failures,
        }
    }
}

#[derive(Debug)]
pub struct MemoryGuardStats {
    pub page_count: u64,
    pub total_bytes: u64,
    pub integrity_failures: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allocate_write_read() {
        let mut mg = MemoryGuard::new().unwrap_or_default();
        let pid = mg.allocate(256).unwrap_or_default();
        mg.write(pid, 0, b"secret_key").unwrap_or_default();
        let data = mg.read(pid, 0, 9).unwrap_or_default();
        assert_eq!(data, b"secret_key");
    }

    #[test]
    fn test_overflow_protection() {
        let mut mg = MemoryGuard::new().unwrap_or_default();
        let pid = mg.allocate(10).unwrap_or_default();
        assert!(mg.write(pid, 9, b"overflow").is_err());
    }

    #[test]
    fn test_free() {
        let mut mg = MemoryGuard::new().unwrap_or_default();
        let pid = mg.allocate(100).unwrap_or_default();
        mg.free(pid).unwrap_or_default();
        assert!(mg.read(pid, 0, 10).is_err());
    }
}





