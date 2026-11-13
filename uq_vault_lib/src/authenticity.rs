use uq_black_key_standalone::UqBlackKey;

/// Vérification continue de l'authenticité des pairs
pub struct AuthenticityVerifier {
    pub trusted_nodes: Vec<[u8; 32]>,
    pub compromised_nodes: Vec<[u8; 32]>,
    pub last_verification: u64,
}

impl AuthenticityVerifier {
    pub fn new() -> Self {
        Self {
            trusted_nodes: Vec::new(),
            compromised_nodes: Vec::new(),
            last_verification: 0,
        }
    }
    
    /// Vérifie l'authenticité d'un pair
    pub fn verify(&mut self, black_key: &UqBlackKey) -> bool {
        self.last_verification = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        
        let is_valid = black_key.verify_integrity();
        
        if is_valid {
            if !self.trusted_nodes.contains(&black_key.address) {
                self.trusted_nodes.push(black_key.address);
            }
        } else {
            if !self.compromised_nodes.contains(&black_key.address) {
                self.compromised_nodes.push(black_key.address);
            }
        }
        
        is_valid
    }
    
    /// Vérifie si un nœud est de confiance
    pub fn is_trusted(&self, address: &[u8; 32]) -> bool {
        self.trusted_nodes.contains(address)
    }
    
    /// Vérifie si un nœud est compromis
    pub fn is_compromised(&self, address: &[u8; 32]) -> bool {
        self.compromised_nodes.contains(address)
    }
    
    /// Dernière vérification
    pub fn last_check(&self) -> u64 {
        self.last_verification
    }
    
    /// Nombre de nœuds de confiance
    pub fn trusted_count(&self) -> usize {
        self.trusted_nodes.len()
    }
    
    /// Nombre de nœuds compromis
    pub fn compromised_count(&self) -> usize {
        self.compromised_nodes.len()
    }
}
