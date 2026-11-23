use crate::udp_network::UdpNetwork;
use std::sync::Arc;
use std::sync::Mutex;
use uq_cellular::d2d::D2dManager;
use uq_cellular::discovery::{DiscoveryManager, DiscoveryMode};
use uq_cellular::emergency::{EmergencyManager, EmergencyType};
use uq_cellular::mesh_transit::{MeshMode, MeshNetwork};
use uq_cellular::relay_internet::RelayInternet;
use uq_cellular::transport::{TransportLayer, TransportType};
use uq_cellular::udp_transport::UdpTransport;

pub struct P2pState {
    pub d2d: Mutex<D2dManager>,
    pub mesh: Mutex<MeshNetwork>,
    pub discovery: Mutex<DiscoveryManager>,
    pub emergency: Mutex<EmergencyManager>,
    pub relay_internet: Mutex<RelayInternet>,
    // 🆕 AJOUT — Transport physique UDP
    pub udp_transport: Mutex<TransportLayer<UdpTransport>>,
    // 🆕 AJOUT — Vrai transport UDP
    pub udp_network: Arc<UdpNetwork>,
}

impl P2pState {
    pub fn new(our_address: [u8; 32]) -> Self {
        Self {
            d2d: Mutex::new(D2dManager::new()),
            mesh: Mutex::new(MeshNetwork::new()),
            discovery: Mutex::new(DiscoveryManager::new(100, 30)),
            emergency: Mutex::new(EmergencyManager::new(our_address)),
            relay_internet: Mutex::new(RelayInternet::new(1000)),
            // 🆕 AJOUT — Instanciation du transport UDP
            udp_transport: Mutex::new(TransportLayer::new(
                UdpTransport::new(&format!(
                    "0.0.0.0:{}",
                    std::env::var("D2D_UDP_PORT").unwrap_or_else(|_| "8082".to_string())
                )),
                TransportType::Virtual,
            )),
            // 🆕 AJOUT — Démarrer le vrai réseau UDP
            udp_network: Arc::new(UdpNetwork::start(
                std::env::var("UDP_PORT")
                    .unwrap_or_else(|_| "8080".to_string())
                    .parse()
                    .unwrap_or(8080),
            )),
        }
    }

    // ====================
    // D2D — Device-to-Device
    // ====================

    pub fn d2d_enable(&self) -> Result<String, String> {
        let mut d2d = self.d2d.lock().unwrap();
        let mode = d2d.enable().map_err(|e| format!("{:?}", e))?;
        Ok(format!("{:?}", mode))
    }

    pub fn d2d_disable(&self) {
        let mut d2d = self.d2d.lock().unwrap();
        d2d.disable();
    }

    pub fn d2d_is_active(&self) -> bool {
        let d2d = self.d2d.lock().unwrap();
        d2d.is_active()
    }

    pub fn d2d_get_stats(&self) -> serde_json::Value {
        let d2d = self.d2d.lock().unwrap();
        let stats = d2d.get_stats();
        serde_json::json!({
            "is_active": stats.is_active,
            "mode": format!("{:?}", d2d.get_mode()),
            "max_range_m": d2d.get_max_range(),
            "max_bandwidth_kbps": d2d.get_max_bandwidth()
        })
    }

    // ====================
    // MESH — Maillage multi-sauts
    // ====================

    pub fn mesh_register_node(
        &self,
        address_hex: &str,
        mode: &str,
        bandwidth_kbps: u32,
        battery_pct: u8,
        price_per_day: u64,
    ) -> Result<(), String> {
        let mut mesh = self.mesh.lock().unwrap();
        let addr_bytes = hex::decode(address_hex).map_err(|_| "Adresse invalide")?;
        let mut addr = [0u8; 32];
        addr.copy_from_slice(&addr_bytes[..32]);

        let mesh_mode = match mode {
            "seller" => MeshMode::Seller,
            "buyer" => MeshMode::Buyer,
            "relay" => MeshMode::Relay,
            _ => MeshMode::Relay,
        };

        mesh.register_node(addr, mesh_mode, bandwidth_kbps, battery_pct, price_per_day);
        Ok(())
    }

    pub fn mesh_become_seller(
        &self,
        address_hex: &str,
        price_per_day: u64,
        bandwidth_kbps: u32,
    ) -> Result<(), String> {
        let mut mesh = self.mesh.lock().unwrap();
        let addr_bytes = hex::decode(address_hex).map_err(|_| "Adresse invalide")?;
        let mut addr = [0u8; 32];
        addr.copy_from_slice(&addr_bytes[..32]);
        mesh.become_seller(addr, price_per_day, bandwidth_kbps);
        Ok(())
    }

    pub fn mesh_become_relay(&self, address_hex: &str, bandwidth_kbps: u32) -> Result<(), String> {
        let mut mesh = self.mesh.lock().unwrap();
        let addr_bytes = hex::decode(address_hex).map_err(|_| "Adresse invalide")?;
        let mut addr = [0u8; 32];
        addr.copy_from_slice(&addr_bytes[..32]);
        mesh.become_relay(addr, bandwidth_kbps);
        Ok(())
    }

    pub fn mesh_get_stats(&self) -> serde_json::Value {
        let mesh = self.mesh.lock().unwrap();
        let stats = mesh.get_stats();
        serde_json::json!({
            "active_nodes": stats.active_nodes,
            "sellers": stats.sellers,
            "relays": stats.relays,
            "buyers": stats.buyers,
            "total_packets_routed": stats.total_packets_routed,
            "avg_hops": stats.avg_hops,
            "uptime_seconds": stats.uptime_seconds
        })
    }

    // ====================
    // DISCOVERY — Découverte des pairs
    // ====================

    pub fn discovery_scan(&self) -> Result<usize, String> {
        let mut disc = self.discovery.lock().unwrap();
        disc.set_mode(DiscoveryMode::Active);
        disc.scan().map_err(|e| format!("{:?}", e))
    }

    pub fn discovery_get_peers(&self) -> serde_json::Value {
        let disc = self.discovery.lock().unwrap();
        let peers = disc.get_peers_by_signal();
        serde_json::json!({
            "peer_count": disc.peer_count(),
            "peers": peers.iter().map(|p| {
                serde_json::json!({
                    "address": hex::encode(p.address),
                    "signal_dbm": p.signal_dbm
                })
            }).collect::<Vec<_>>()
        })
    }

    // ====================
    // EMERGENCY — Mode urgence
    // ====================

    pub fn emergency_activate(
        &self,
        emergency_type: &str,
        battery_pct: u8,
        message: &str,
    ) -> serde_json::Value {
        let mut em = self.emergency.lock().unwrap();

        let etype = match emergency_type {
            "emergency_call" => EmergencyType::EmergencyCall,
            "low_battery" => EmergencyType::LowBattery,
            "signal_lost" => EmergencyType::SignalLost,
            "overheating" => EmergencyType::Overheating,
            _ => EmergencyType::EmergencyCall,
        };

        let beacon = em.activate_emergency(etype, battery_pct, message);
        serde_json::json!({
            "emergency_type": format!("{:?}", beacon.emergency_type),
            "battery_pct": beacon.battery_pct,
            "message": beacon.message,
            "timestamp": beacon.timestamp,
            "ttl": beacon.ttl
        })
    }

    pub fn emergency_is_active(&self) -> bool {
        let em = self.emergency.lock().unwrap();
        em.is_in_emergency()
    }

    // ====================
    // RELAY INTERNET — Vente de connexion
    // ====================

    pub fn relay_connect(&self, address_hex: &str, hours: u64) -> Result<(), String> {
        let mut relay = self.relay_internet.lock().unwrap();
        let addr_bytes = hex::decode(address_hex).map_err(|_| "Adresse invalide")?;
        let mut addr = [0u8; 32];
        addr.copy_from_slice(&addr_bytes[..32]);
        relay.connect(&addr, hours).map_err(|e| format!("{:?}", e))
    }

    pub fn relay_disconnect(&self) {
        let mut relay = self.relay_internet.lock().unwrap();
        relay.disconnect();
    }

    pub fn relay_is_connected(&self) -> bool {
        let relay = self.relay_internet.lock().unwrap();
        relay.is_connected()
    }

    // ====================
    // TRANSPORT UDP — Envoi et réception de données
    // ====================

    /// Envoie une donnée via le transport UDP
    pub fn send(&self, data: &[u8]) -> Result<(), String> {
        let mut transport = self
            .udp_transport
            .lock()
            .map_err(|_| "Transport lock failed")?;

        transport
            .send(data)
            .map_err(|e| format!("Send failed: {:?}", e))
    }

    /// Diffuse une donnée à tous les pairs
    /// (même méthode que send pour l'instant, c'est UDP)
    pub fn broadcast(&self, data: &[u8]) -> Result<(), String> {
        self.send(data)
    }

    /// Vérifie que le transport est prêt
    pub fn is_transport_ready(&self) -> bool {
        self.udp_transport
            .lock()
            .map(|t| t.is_ready())
            .unwrap_or(false)
    }

    /// Récupère les métriques du transport
    pub fn transport_metrics(&self) -> serde_json::Value {
        let transport = self.udp_transport.lock().unwrap();
        let metrics = transport.get_metrics();
        serde_json::json!({
            "connected": metrics.connected,
        })
    }
    // ====================
    // RÉSEAU UDP — VRAI TRANSPORT P2P
    // ====================

    /// Envoie un message à un pair spécifique
    pub fn send_to_peer(&self, peer_addr: &str, data: &[u8]) -> Result<(), String> {
        let msg = String::from_utf8_lossy(data);
        self.udp_network.send(peer_addr, &msg)
    }

    /// Diffuse un message à TOUS les pairs connus
    pub fn broadcast_to_peers(&self, data: &[u8]) -> Result<usize, String> {
        let msg = String::from_utf8_lossy(data);
        Ok(self.udp_network.broadcast(&msg))
    }

    /// Liste des pairs connus (par UDP)
    pub fn get_udp_peers(&self) -> Vec<String> {
        self.udp_network.get_peers()
    }

    /// Nombre de pairs UDP
    pub fn udp_peer_count(&self) -> usize {
        self.udp_network.peer_count()
    }

    /// Ajoute manuellement un pair UDP
    pub fn add_udp_peer(&self, addr: &str) {
        self.udp_network.add_peer(addr);
        let _ = self.udp_network.send(addr, "UQ_HANDSHAKE");
    }

    /// Synchronise les pairs UDP dans DiscoveryManager
    pub fn sync_discovery(&self) -> usize {
        let udp_peers = self.udp_network.get_peers();
        let mut discovery = self.discovery.lock().unwrap();
        let count = udp_peers.len();

        for addr in udp_peers {
            let mut address = [0u8; 32];
            let bytes = addr.as_bytes();
            for (i, b) in bytes.iter().enumerate() {
                if i < 32 {
                    address[i] = *b;
                }
            }
            discovery.add_peer(address, &addr, -50, "UDP");
        }
        count
    }
}
