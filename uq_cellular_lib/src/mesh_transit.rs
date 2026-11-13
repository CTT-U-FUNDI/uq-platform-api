use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::collections::HashMap;
use uq_blockchain::error::ChainError;

// ═══════════════════════════════════════════════════════════════
// CONSTANTES
// ═══════════════════════════════════════════════════════════════

/// Nombre maximum de sauts (relais intermédiaires)
pub const MAX_HOPS: u8 = 10;
/// Timeout par saut (secondes)
pub const HOP_TIMEOUT_SECS: u64 = 5;
/// Taille maximale d'un paquet en transit (octets)
pub const MAX_PACKET_SIZE: usize = 1500;
/// Frais de transit par saut (micro-BORB) — 0 = gratuit
pub const TRANSIT_FEE_PER_HOP: u64 = 0;

// ═══════════════════════════════════════════════════════════════
// STRUCTURES
// ═══════════════════════════════════════════════════════════════

/// Un paquet en transit dans le réseau maillé
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshPacket {
    /// ID unique du paquet
    pub id: [u8; 32],
    /// Adresse source (vendeur Starlink)
    pub source: [u8; 32],
    /// Adresse destination (acheteur)
    pub destination: [u8; 32],
    /// Chemin emprunté (chaque saut)
    pub path: Vec<MeshHop>,
    /// Données utiles
    pub payload: Vec<u8>,
    /// Timestamp de création
    pub created_at: u64,
    /// TTL (Time To Live) — décrémenté à chaque saut
    pub ttl: u8,
    /// Signature de la source
    pub source_signature: Vec<u8>,
}

/// Un saut dans le chemin
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshHop {
    /// Adresse du relayeur
    pub relay_address: [u8; 32],
    /// Timestamp de réception
    pub received_at: u64,
    /// Timestamp d'envoi
    pub forwarded_at: u64,
    /// Signature du relayeur (preuve de transit)
    pub relay_signature: Vec<u8>,
}

/// Un nœud du réseau maillé
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshNode {
    /// Adresse du nœud
    pub address: [u8; 32],
    /// Mode : Vendeur, Relais, Acheteur
    pub mode: MeshMode,
    /// Prix si vendeur (BORB/jour)
    pub price_per_day: u64,
    /// Bande passante disponible (kbps)
    pub available_bandwidth_kbps: u32,
    /// Batterie restante (%)
    pub battery_pct: u8,
    /// Dernière activité (timestamp)
    pub last_seen: u64,
    /// Nombre de paquets relayés
    pub packets_relayed: u64,
    /// Est actif
    pub is_active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MeshMode {
    /// Vendeur de connexion (Starlink, 4G, etc.)
    Seller,
    /// Relais gratuit (contribue au réseau)
    Relay,
    /// Acheteur de connexion
    Buyer,
    /// Hybride (relais + acheteur)
    Hybrid,
}

/// Table de routage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshRoutingTable {
    /// Prochains sauts connus : destination → (prochain_saut, nombre_sauts)
    pub routes: HashMap<[u8; 32], ([u8; 32], u8)>,
    /// Dernière mise à jour
    pub last_updated: u64,
}

/// Statistiques du réseau maillé
#[derive(Debug, Clone)]
pub struct MeshStats {
    pub active_nodes: usize,
    pub sellers: usize,
    pub relays: usize,
    pub buyers: usize,
    pub total_packets_routed: u64,
    pub avg_hops: f64,
    pub uptime_seconds: u64,
}

// ═══════════════════════════════════════════════════════════════
// RÉSEAU MAILLÉ
// ═══════════════════════════════════════════════════════════════

pub struct MeshNetwork {
    /// Nœuds connus
    pub nodes: HashMap<[u8; 32], MeshNode>,
    /// Table de routage
    pub routing: MeshRoutingTable,
    /// Paquets en transit
    pub pending_packets: Vec<MeshPacket>,
    /// Historique des paquets relayés
    pub relayed_packets: Vec<[u8; 32]>,
    /// Total des paquets routés
    pub total_packets_routed: u64,
    /// Timestamp de démarrage
    pub started_at: u64,
}

impl MeshNetwork {
    pub fn new() -> Self {
        MeshNetwork {
            nodes: HashMap::new(),
            routing: MeshRoutingTable { routes: HashMap::new(), last_updated: utc_now() },
            pending_packets: Vec::new(),
            relayed_packets: Vec::new(),
            total_packets_routed: 0,
            started_at: utc_now(),
        }
    }

    /// Enregistre un nœud dans le réseau
    pub fn register_node(&mut self, address: [u8; 32], mode: MeshMode, bandwidth_kbps: u32, battery_pct: u8, price_per_day: u64) {
        self.nodes.insert(address, MeshNode {
            address, mode, price_per_day,
            available_bandwidth_kbps: bandwidth_kbps,
            battery_pct, last_seen: utc_now(),
            packets_relayed: 0, is_active: true,
        });
    }

    /// Stéphane appuie sur "VENDRE MA CONNEXION STARLINK"
    pub fn become_seller(&mut self, address: [u8; 32], price_per_day: u64, bandwidth_kbps: u32) {
        if let Some(node) = self.nodes.get_mut(&address) {
            node.mode = MeshMode::Seller;
            node.price_per_day = price_per_day;
            node.available_bandwidth_kbps = bandwidth_kbps;
            node.is_active = true;
        } else {
            self.register_node(address, MeshMode::Seller, bandwidth_kbps, 100, price_per_day);
        }
    }

    /// Acheteur appuie sur "ACHETER CONNEXION STARLINK"
    pub fn become_buyer(&mut self, address: [u8; 32]) {
        if let Some(node) = self.nodes.get_mut(&address) {
            node.mode = MeshMode::Buyer;
            node.is_active = true;
        } else {
            self.register_node(address, MeshMode::Buyer, 0, 100, 0);
        }
    }

    /// Active le mode relais gratuit (téléphone intermédiaire)
    pub fn become_relay(&mut self, address: [u8; 32], bandwidth_kbps: u32) {
        if let Some(node) = self.nodes.get_mut(&address) {
            node.mode = MeshMode::Relay;
            node.available_bandwidth_kbps = bandwidth_kbps;
            node.is_active = true;
        } else {
            self.register_node(address, MeshMode::Relay, bandwidth_kbps, 100, 0);
        }
    }

    /// Trouve les vendeurs disponibles (Starlink, 4G, etc.)
    pub fn find_sellers(&self) -> Vec<&MeshNode> {
        self.nodes.values()
            .filter(|n| n.mode == MeshMode::Seller && n.is_active && n.battery_pct > 10)
            .collect()
    }

    /// Trouve le chemin vers une destination via les relais
    pub fn find_route(&mut self, destination: &[u8; 32]) -> Option<Vec<[u8; 32]>> {
        // Vérifier si la destination est directement accessible
        if self.nodes.contains_key(destination) {
            return Some(vec![*destination]);
        }

        // Recherche de chemin via les relais actifs
        let relays: Vec<&MeshNode> = self.nodes.values()
            .filter(|n| n.mode == MeshMode::Relay && n.is_active && n.battery_pct > 20)
            .collect();

        if relays.is_empty() {
            return None;
        }

        // Construire un chemin simple : passer par les relais disponibles
        let mut path = Vec::new();
        for relay in relays.iter().take(MAX_HOPS as usize) {
            path.push(relay.address);
        }
        path.push(*destination);

        // Mettre à jour la table de routage
        if path.len() > 1 {
            self.routing.routes.insert(*destination, (path[0], path.len() as u8));
            self.routing.last_updated = utc_now();
        }

        Some(path)
    }

    /// Crée un paquet pour la source
    pub fn create_packet(&self, source: &[u8; 32], destination: &[u8; 32], payload: &[u8]) -> Result<MeshPacket, ChainError> {
        if payload.len() > MAX_PACKET_SIZE {
            return Err(ChainError::InternalError("Paquet trop volumineux".into()));
        }
        let now = utc_now();
        Ok(MeshPacket {
            id: hash_packet(source, destination, payload, now),
            source: *source,
            destination: *destination,
            path: Vec::new(),
            payload: payload.to_vec(),
            created_at: now,
            ttl: MAX_HOPS,
            source_signature: Vec::new(),
        })
    }

    /// Un relayeur fait passer le paquet (ajoute un saut)
    pub fn relay_packet(&mut self, packet: &mut MeshPacket, relay_address: &[u8; 32]) -> Result<(), ChainError> {
        if packet.ttl == 0 {
            return Err(ChainError::InternalError("TTL expiré".into()));
        }

        // Vérifier que le relayeur est actif
        if let Some(node) = self.nodes.get_mut(relay_address) {
            if !node.is_active {
                return Err(ChainError::InternalError("Relayeur inactif".into()));
            }
            if node.battery_pct < 10 {
                return Err(ChainError::InternalError("Batterie trop faible".into()));
            }
            node.packets_relayed += 1;
            node.last_seen = utc_now();
        }

        let now = utc_now();
        packet.path.push(MeshHop {
            relay_address: *relay_address,
            received_at: now,
            forwarded_at: now,
            relay_signature: Vec::new(), // Signé par le relayeur
        });
        packet.ttl -= 1;
        self.total_packets_routed += 1;

        Ok(())
    }

    /// Vérifie si un paquet est arrivé à destination
    pub fn is_delivered(&self, packet: &MeshPacket) -> bool {
        if let Some(last_hop) = packet.path.last() {
            last_hop.relay_address == packet.destination
        } else {
            false
        }
    }

    /// Retourne les statistiques du réseau
    pub fn get_stats(&self) -> MeshStats {
        let active: Vec<&MeshNode> = self.nodes.values().filter(|n| n.is_active).collect();
        let total_hops: u64 = self.relayed_packets.iter()
            .filter_map(|_id| {
                // Compter les sauts (simplifié)
                Some(1u64)
            })
            .sum();

        MeshStats {
            active_nodes: active.len(),
            sellers: active.iter().filter(|n| n.mode == MeshMode::Seller).count(),
            relays: active.iter().filter(|n| n.mode == MeshMode::Relay).count(),
            buyers: active.iter().filter(|n| n.mode == MeshMode::Buyer).count(),
            total_packets_routed: self.total_packets_routed,
            avg_hops: if self.relayed_packets.len() > 0 { total_hops as f64 / self.relayed_packets.len() as f64 } else { 0.0 },
            uptime_seconds: utc_now() - self.started_at,
        }
    }
}

fn hash_packet(source: &[u8; 32], dest: &[u8; 32], payload: &[u8], ts: u64) -> [u8; 32] {
    let mut h = sha2::Sha256::new();
    h.update(source); h.update(dest); h.update(payload); h.update(&ts.to_le_bytes());
    let r = h.finalize(); let mut id = [0u8; 32]; id.copy_from_slice(&r); id
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

    fn addr(i: u8) -> [u8; 32] { let mut a = [0u8; 32]; a[0] = i; a }

    #[test] fn test_new() { let m = MeshNetwork::new(); assert_eq!(m.nodes.len(), 0); }

    #[test] fn test_become_seller() {
        let mut m = MeshNetwork::new();
        let stephane = addr(1);
        m.become_seller(stephane, 10, 100_000); // 10 BORB/jour, 100 Mbps
        let sellers = m.find_sellers();
        assert_eq!(sellers.len(), 1);
        assert_eq!(sellers[0].price_per_day, 10);
    }

    #[test] fn test_become_buyer() {
        let mut m = MeshNetwork::new();
        let acheteur = addr(2);
        m.become_buyer(acheteur);
        assert!(m.nodes.contains_key(&acheteur));
    }

    #[test] fn test_become_relay() {
        let mut m = MeshNetwork::new();
        let tel1 = addr(3);
        m.become_relay(tel1, 50_000);
        assert_eq!(m.nodes.get(&tel1).unwrap().mode, MeshMode::Relay);
    }

    #[test] fn test_create_packet() {
        let m = MeshNetwork::new();
        let packet = m.create_packet(&addr(1), &addr(2), b"hello").unwrap();
        assert_eq!(packet.ttl, MAX_HOPS);
        assert_eq!(packet.payload, b"hello");
    }

    #[test] fn test_relay_packet() {
        let mut m = MeshNetwork::new();
        m.register_node(addr(3), MeshMode::Relay, 1000, 80, 0);
        let mut packet = m.create_packet(&addr(1), &addr(2), b"data").unwrap();
        assert!(m.relay_packet(&mut packet, &addr(3)).is_ok());
        assert_eq!(packet.path.len(), 1);
        assert_eq!(packet.ttl, MAX_HOPS - 1);
    }

    #[test] fn test_ttl_expired() {
        let mut m = MeshNetwork::new();
        m.register_node(addr(3), MeshMode::Relay, 1000, 80, 0);
        let mut packet = m.create_packet(&addr(1), &addr(2), b"data").unwrap();
        packet.ttl = 0;
        assert!(m.relay_packet(&mut packet, &addr(3)).is_err());
    }

    #[test] fn test_find_sellers() {
        let mut m = MeshNetwork::new();
        m.become_seller(addr(1), 5, 100_000);   // Stéphane Starlink
        m.become_seller(addr(4), 3, 50_000);    // Marie 4G
        m.become_relay(addr(3), 10_000);        // Tel relais
        let sellers = m.find_sellers();
        assert_eq!(sellers.len(), 2);
    }

    #[test] fn test_full_scenario() {
        // Stéphane a Starlink, il vend sa connexion
        let mut mesh = MeshNetwork::new();
        let stephane = addr(1);
        mesh.become_seller(stephane, 10, 100_000);

        // 3 téléphones intermédiaires = relais gratuits
        let tel1 = addr(10); mesh.become_relay(tel1, 50_000);
        let tel2 = addr(11); mesh.become_relay(tel2, 50_000);
        let tel3 = addr(12); mesh.become_relay(tel3, 30_000);

        // Acheteur veut se connecter
        let acheteur = addr(20);
        mesh.become_buyer(acheteur);

        // Créer un paquet de l'acheteur vers Stéphane
        let mut packet = mesh.create_packet(&acheteur, &stephane, b"BUY_STARLINK").unwrap();

        // Les 3 relais font passer le paquet
        assert!(mesh.relay_packet(&mut packet, &tel1).is_ok());
        assert!(mesh.relay_packet(&mut packet, &tel2).is_ok());
        assert!(mesh.relay_packet(&mut packet, &tel3).is_ok());

        // Vérifier le chemin
        assert_eq!(packet.path.len(), 3);
        assert_eq!(packet.ttl, MAX_HOPS - 3);

        // Statistiques
        let stats = mesh.get_stats();
        assert_eq!(stats.sellers, 1);
        assert_eq!(stats.relays, 3);
        assert_eq!(stats.buyers, 1);
        assert!(stats.total_packets_routed > 0);
    }

    #[test] fn test_stats() {
        let mut m = MeshNetwork::new();
        m.become_seller(addr(1), 10, 100_000);
        m.become_relay(addr(2), 50_000);
        m.become_buyer(addr(3));
        let stats = m.get_stats();
        assert_eq!(stats.active_nodes, 3);
        assert_eq!(stats.sellers, 1);
        assert_eq!(stats.relays, 1);
        assert_eq!(stats.buyers, 1);
    }
}


