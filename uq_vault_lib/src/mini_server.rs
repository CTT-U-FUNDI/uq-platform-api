use uq_black_key_standalone::UqBlackKey;
use crate::db_p2p::DbP2p;

/// Le Mini Serveur UQ — chaque téléphone devient un serveur
pub struct MiniServer {
    pub node_address: [u8; 32],
    pub db: DbP2p,
    pub bandwidth_limit_kbps: u32,
    pub battery_limit_pct: u8,
    pub always_active: bool,
}

impl MiniServer {
    pub fn new(black_key: &UqBlackKey) -> Self {
        Self {
            node_address: black_key.address,
            db: DbP2p::new(),
            bandwidth_limit_kbps: 1024,  // 1 Mo/s
            battery_limit_pct: 5,        // 5% par jour
            always_active: true,         // pas de veille
        }
    }
    
    /// Récupère l'identité du nœud
    pub fn identity(&self) -> String {
        hex::encode(self.node_address)
    }
    
    /// Vérifie l'état de santé
    pub fn health(&self) -> (u64, u64, bool) {
        (self.db.storage_used, self.db.storage_max, self.always_active)
    }
    
    /// Stocke un fragment chiffré
    pub fn store(&mut self, fragment: Vec<u8>) -> Result<(), String> {
        self.db.store(fragment)
    }
    
    /// Récupère un fragment
    pub fn retrieve(&self, id: &[u8; 32]) -> Option<&Vec<u8>> {
        self.db.retrieve(id)
    }
    
    /// Liste les pairs D2D proches
    pub fn peers(&self) -> Vec<[u8; 32]> {
        // CONNECTÉ à discovery.rs existant
        // Pour l'instant : retourne vide (à connecter)
        Vec::new()
    }
}
