use std::collections::HashMap;

/// Database P2P légère — 10 Mo par nœud
pub struct DbP2p {
    pub fragments: HashMap<[u8; 32], Vec<u8>>,
    pub storage_used: u64,
    pub storage_max: u64,
}

impl DbP2p {
    pub fn new() -> Self {
        Self {
            fragments: HashMap::new(),
            storage_used: 0,
            storage_max: 10 * 1024 * 1024, // 10 Mo
        }
    }
    
    /// Stocke un fragment (vérifie la taille)
    pub fn store(&mut self, fragment: Vec<u8>) -> Result<(), String> {
        if self.storage_used + fragment.len() as u64 > self.storage_max {
            return Err("Stockage plein (10 Mo atteint)".to_string());
        }
        
        let id = Self::hash_fragment(&fragment);
        self.storage_used += fragment.len() as u64;
        self.fragments.insert(id, fragment);
        Ok(())
    }
    
    /// Récupère un fragment par son ID
    pub fn retrieve(&self, id: &[u8; 32]) -> Option<&Vec<u8>> {
        self.fragments.get(id)
    }
    
    /// Supprime un fragment
    pub fn delete(&mut self, id: &[u8; 32]) -> Result<(), String> {
        if let Some(fragment) = self.fragments.remove(id) {
            self.storage_used = self.storage_used.saturating_sub(fragment.len() as u64);
        }
        Ok(())
    }
    
    /// Pourcentage de stockage utilisé
    pub fn usage_pct(&self) -> f64 {
        (self.storage_used as f64 / self.storage_max as f64) * 100.0
    }
    
    /// Vérifie si le stockage est plein
    pub fn is_full(&self) -> bool {
        self.storage_used >= self.storage_max
    }
    
    /// Hash d'un fragment
    fn hash_fragment(fragment: &[u8]) -> [u8; 32] {
        let mut hash = [0u8; 32];
        for (i, byte) in fragment.iter().take(32).enumerate() {
            hash[i] = *byte;
        }
        hash
    }
}
