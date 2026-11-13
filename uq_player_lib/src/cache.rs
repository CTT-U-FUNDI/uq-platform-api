use std::collections::HashSet;
use sha2::{Sha256, Digest};

/// Le cache UQ — les contenus déjà payés
pub struct ContentCache {
    pub paid_hashes: HashSet<[u8; 32]>,
}

impl ContentCache {
    pub fn new() -> Self {
        Self {
            paid_hashes: HashSet::new(),
        }
    }
    
    /// Calcule le hash d'un contenu
    pub fn hash_content(content_url: &str) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(content_url.as_bytes());
        hasher.update(b"UQ-CONTENT-CACHE");
        let result = hasher.finalize();
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&result);
        hash
    }
    
    /// Vérifie si un contenu a déjà été payé
    pub fn is_paid(&self, content_url: &str) -> bool {
        let hash = Self::hash_content(content_url);
        self.paid_hashes.contains(&hash)
    }
    
    /// Ajoute un contenu au cache après paiement
    pub fn mark_paid(&mut self, content_url: &str) {
        let hash = Self::hash_content(content_url);
        self.paid_hashes.insert(hash);
    }
    
    /// Nombre de contenus en cache
    pub fn cache_size(&self) -> usize {
        self.paid_hashes.len()
    }
}
