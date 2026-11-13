//! UQ DHT Module - Table de Hachage Distribuée (Kademlia)
//! Permet de trouver les pairs sans serveur central.
//! Chaque nœud stocke les informations des K=20 nœuds les plus proches.

use crate::crypto::sha256;
use crate::error::UqError;
use std::collections::HashMap;

/// Nombre de buckets dans la DHT (basé sur SHA256 = 256 bits)
pub const DHT_BUCKETS: usize = 256;

/// Nombre de nœuds par bucket (Kademlia standard)
pub const K: usize = 20;

/// Taille d'une clé DHT en octets (SHA256)
pub const DHT_KEY_SIZE: usize = 32;

/// TTL par défaut en secondes (1 heure)
pub const DEFAULT_TTL: u64 = 3600;

/// Structure représentant un pair dans la DHT
#[derive(Debug, Clone)]
pub struct Peer {
    /// Adresse UQ du pair
    pub address: String,
    /// Clé publique du pair
    pub public_key: [u8; 32],
    /// Signaux de connexion (WebRTC, IP, Tor, etc.)
    pub signals: Vec<u8>,
    /// Timestamp de la dernière mise à jour
    pub last_seen: u64,
    /// TTL en secondes
    pub ttl: u64,
}

/// Structure représentant la DHT locale
#[derive(Debug)]
pub struct UqDHT {
    /// Buckets (256 buckets de K=20 pairs maximum)
    buckets: Vec<Vec<Peer>>,
    /// Clés de session avec les pairs
    session_keys: HashMap<String, [u8; 32]>,
    /// Nombre total de pairs connus
    total_peers: usize,
}

impl UqDHT {
    /// Crée une nouvelle DHT vide avec 256 buckets.
    pub fn new() -> Self {
        UqDHT {
            buckets: vec![Vec::new(); DHT_BUCKETS],
            session_keys: HashMap::new(),
            total_peers: 0,
        }
    }

    /// Calcule l'index du bucket pour une clé donnée.
    /// Utilise le premier octet du hash SHA256.
    fn get_bucket_index(key: &[u8]) -> usize {
        if key.is_empty() {
            return 0;
        }
        (key[0] as usize) % DHT_BUCKETS
    }

    /// Calcule la distance XOR entre deux clés (sur 32 octets).
    fn xor_distance(a: &[u8], b: &[u8]) -> [u8; DHT_KEY_SIZE] {
        let mut distance = [0u8; DHT_KEY_SIZE];
        let len = a.len().min(b.len()).min(DHT_KEY_SIZE);
        for i in 0..len {
            distance[i] = a[i] ^ b[i];
        }
        distance
    }

    /// Compare deux distances XOR (retourne true si a < b).
    fn is_closer(a: &[u8; DHT_KEY_SIZE], b: &[u8; DHT_KEY_SIZE]) -> bool {
        for i in 0..DHT_KEY_SIZE {
            if a[i] != b[i] {
                return a[i] < b[i];
            }
        }
        false
    }

    /// Stocke une valeur dans la DHT.
    ///
    /// # Arguments
    /// * `key` - Clé de 32 octets (hash de l'adresse)
    /// * `address` - Adresse UQ du pair
    /// * `public_key` - Clé publique du pair
    /// * `signals` - Signaux de connexion
    /// * `ttl` - Durée de vie en secondes
    pub fn put(
        &mut self,
        key: &[u8],
        address: &str,
        public_key: &[u8; 32],
        signals: &[u8],
        ttl: u64,
    ) -> Result<(), UqError> {
        if address.is_empty() {
            return Err(UqError::DhtError("Adresse vide".to_string()));
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let peer = Peer {
            address: address.to_string(),
            public_key: *public_key,
            signals: signals.to_vec(),
            last_seen: now,
            ttl,
        };

        let bucket_idx = Self::get_bucket_index(key);
        let bucket = &mut self.buckets[bucket_idx];

        // Mettre à jour si le pair existe déjà
        for existing in bucket.iter_mut() {
            if existing.address == address {
                *existing = peer;
                return Ok(());
            }
        }

        // Ajouter le pair
        if bucket.len() < K {
            bucket.push(peer);
        } else {
            // Remplacer le pair le plus ancien
            bucket.sort_by_key(|p| p.last_seen);
            bucket[0] = peer;
        }

        self.total_peers += 1;
        Ok(())
    }

    /// Récupère les signaux d'un pair depuis la DHT.
    ///
    /// # Arguments
    /// * `key` - Clé de 32 octets (hash de l'adresse)
    /// * `target_address` - Adresse UQ du pair recherché
    pub fn get(&self, key: &[u8], target_address: &str) -> Option<&Peer> {
        let bucket_idx = Self::get_bucket_index(key);
        let bucket = &self.buckets[bucket_idx];

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        bucket.iter().find(|p| {
            p.address == target_address && (now - p.last_seen) < p.ttl
        })
    }

    /// Trouve les K nœuds les plus proches d'une clé cible.
    ///
    /// # Arguments
    /// * `target_key` - Clé cible de 32 octets
    /// * `k` - Nombre de nœuds à retourner (défaut: K=20)
    pub fn get_nearest_neighbors(&self, target_key: &[u8], k: usize) -> Vec<&Peer> {
        let k = k.min(K);
        let mut all_peers: Vec<&Peer> = Vec::new();

        for bucket in &self.buckets {
            for peer in bucket {
                all_peers.push(peer);
            }
        }

        // Trier par distance XOR
        let target_hash = sha256(target_key);
        all_peers.sort_by(|a, b| {
            let dist_a = Self::xor_distance(&sha256(a.address.as_bytes()), &target_hash);
            let dist_b = Self::xor_distance(&sha256(b.address.as_bytes()), &target_hash);
            if Self::is_closer(&dist_a, &dist_b) {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Greater
            }
        });

        all_peers.truncate(k);
        all_peers
    }

    /// Enregistre un nouveau pair dans la DHT.
    pub fn register_peer(
        &mut self,
        address: &str,
        public_key: &[u8; 32],
    ) -> Result<(), UqError> {
        let key = sha256(address.as_bytes());
        self.put(&key, address, public_key, &[], DEFAULT_TTL)
    }

    /// Récupère ou crée une clé de session avec un pair.
    pub fn get_session_key(&self, peer_address: &str) -> Option<&[u8; 32]> {
        self.session_keys.get(peer_address)
    }

    /// Définit une clé de session pour un pair.
    pub fn set_session_key(&mut self, peer_address: &str, key: [u8; 32]) {
        self.session_keys.insert(peer_address.to_string(), key);
    }

    /// Retourne le nombre de pairs actuellement stockés.
    pub fn get_total_peers(&self) -> usize {
        self.total_peers
    }

    /// Retourne le nombre de buckets non vides.
    pub fn get_active_buckets(&self) -> usize {
        self.buckets.iter().filter(|b| !b.is_empty()).count()
    }

    /// Nettoie les pairs expirés (TTL dépassé).
    pub fn cleanup(&mut self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        for bucket in self.buckets.iter_mut() {
            bucket.retain(|p| (now - p.last_seen) < p.ttl);
        }
    }
}

/// ─── TESTS ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::generate_keypair;

    #[test]
    fn test_new_dht() {
        let dht = UqDHT::new();
        assert_eq!(dht.get_total_peers(), 0);
        assert_eq!(dht.get_active_buckets(), 0);
    }

    #[test]
    fn test_put_and_get() {
        let mut dht = UqDHT::new();
        let kp = generate_keypair();
        let key = sha256(b"test-peer");
        let address = "uq:test123";

        dht.put(&key, address, &kp.public_key, &[1, 2, 3], 3600).unwrap_or_default();
        dht.register_peer(address, &kp.public_key).unwrap_or_default();

        let found = dht.get(&key, address);
        assert!(found.is_some());
        assert_eq!(found.unwrap_or_default().address, address);
    }

    #[test]
    fn test_get_nonexistent() {
        let dht = UqDHT::new();
        let key = sha256(b"nonexistent");
        assert!(dht.get(&key, "uq:ghost").is_none());
    }

    #[test]
    fn test_nearest_neighbors() {
        let mut dht = UqDHT::new();

        for i in 0..25 {
            let kp = generate_keypair();
            let addr = format!("uq:peer{}", i);
            let key = sha256(addr.as_bytes());
            dht.put(&key, &addr, &kp.public_key, &[], 3600).unwrap_or_default();
        }

        let target = sha256(b"target");
        let neighbors = dht.get_nearest_neighbors(&target, K);
        assert_eq!(neighbors.len(), K);
    }

    #[test]
    fn test_session_key() {
        let mut dht = UqDHT::new();
        let peer = "uq:peer123";
        let key = sha256(b"session-secret");

        dht.set_session_key(peer, key);
        let retrieved = dht.get_session_key(peer);
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap_or_default(), &key);
    }

    #[test]
    fn test_cleanup() {
        let mut dht = UqDHT::new();
        let kp = generate_keypair();
        let key = sha256(b"expired-peer");

        // Ajouter avec TTL = 0 (expire immédiatement)
        dht.put(&key, "uq:expired", &kp.public_key, &[], 0).unwrap_or_default();
        dht.cleanup();

        assert!(dht.get(&key, "uq:expired").is_none());
    }

    #[test]
    fn test_bucket_index() {
        let key1 = sha256(b"peer1");
        let key2 = sha256(b"peer2");
        let idx1 = UqDHT::get_bucket_index(&key1);
        let idx2 = UqDHT::get_bucket_index(&key2);
        assert!(idx1 < DHT_BUCKETS);
        assert!(idx2 < DHT_BUCKETS);
    }
}
