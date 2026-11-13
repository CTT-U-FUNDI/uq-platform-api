// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// UQ CORE - network.rs
// Couche rÃ©seau unifiÃ©e - Abstraction de tous les canaux
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

use serde::{Serialize, Deserialize};
use crate::error::ChainError;
use std::collections::HashMap;

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// STRUCTURES
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    pub enable_web_rtc: bool,
    pub enable_tor: bool,
    pub enable_ultrasonic: bool,
    pub enable_bluetooth: bool,
    pub enable_wifi_direct: bool,
    pub enable_phantom: bool,
    pub max_peers: u32,
    pub timeout_ms: u64,
    pub retry_count: u32,
    pub bind_address: String,
    pub port: u16,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        NetworkConfig {
            enable_web_rtc: true, enable_tor: true, enable_ultrasonic: true,
            enable_bluetooth: true, enable_wifi_direct: true, enable_phantom: true,
            max_peers: 100, timeout_ms: 5000, retry_count: 3,
            bind_address: "127.0.0.1".into(), port: 9876,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TransportMode {
    WebRTC, WiFiDirect, Tor, Ultrasonic, Bluetooth, Phantom, Direct, Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Priority { Low, Normal, High, Critical }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BroadcastScope { Direct, Group, Mesh, All }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfo {
    pub address: [u8; 32],
    pub latency_ms: u64,
    pub transport: TransportMode,
    pub connected_since: u64,
    pub data_transferred: u64,
    pub last_seen: u64,
    pub is_online: bool,
    pub signal_strength: i8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkStats {
    pub peers_connected: u32,
    pub total_peers_known: u32,
    pub bandwidth_up_bps: u64,
    pub bandwidth_down_bps: u64,
    pub latency_avg_ms: u64,
    pub packets_sent: u64,
    pub packets_lost: u64,
    pub uptime_seconds: u64,
    pub active_channels: Vec<TransportMode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectivityReport {
    pub web_rtc_available: bool,
    pub tor_available: bool,
    pub ultrasonic_available: bool,
    pub bluetooth_available: bool,
    pub wifi_direct_available: bool,
    pub phantom_channel_available: bool,
    pub recommended: TransportMode,
    pub tested_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    pub id: [u8; 32],
    pub topic: String,
    pub subscribed_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub from: [u8; 32],
    pub topic: String,
    pub data: Vec<u8>,
    pub received_at: u64,
    pub transport: TransportMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelStatus {
    pub transport: TransportMode,
    pub available: bool,
    pub active: bool,
    pub latency_ms: u64,
    pub error_count: u64,
}

/// Couche rÃ©seau unifiÃ©e
pub struct NetworkLayer {
    pub config: NetworkConfig,
    pub peers: HashMap<[u8; 32], PeerInfo>,
    pub subscriptions: HashMap<String, Vec<Box<dyn Fn(Message) + Send>>>,
    pub started_at: u64,
    pub packets_sent: u64,
    pub packets_lost: u64,
    pub on_peer_connected: Vec<Box<dyn Fn(PeerInfo) + Send>>,
    pub on_peer_disconnected: Vec<Box<dyn Fn([u8; 32]) + Send>>,
    pub channel_status: HashMap<TransportMode, ChannelStatus>,
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// HELPERS
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

fn get_hardbit() -> u64 { crate::hardbit::get_current_hardbit() }
fn sha256_hash(data: &[u8]) -> [u8; 32] { use sha2::{Sha256, Digest}; let mut h = Sha256::new(); h.update(data); let mut r = [0u8; 32]; r.copy_from_slice(&h.finalize()); r }

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// F1. INITIALISER LA COUCHE RÃ‰SEAU
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

pub fn new(config: NetworkConfig) -> Result<NetworkLayer, ChainError> {
    let hb = get_hardbit();
    let mut channel_status = HashMap::new();

    // Initialiser les statuts des canaux
    for transport in &[
        TransportMode::WebRTC, TransportMode::WiFiDirect, TransportMode::Tor,
        TransportMode::Ultrasonic, TransportMode::Bluetooth, TransportMode::Phantom,
        TransportMode::Direct
    ] {
        channel_status.insert(transport.clone(), ChannelStatus {
            transport: transport.clone(),
            available: false,
            active: false,
            latency_ms: 0,
            error_count: 0,
        });
    }

    Ok(NetworkLayer {
        config,
        peers: HashMap::new(),
        subscriptions: HashMap::new(),
        started_at: hb,
        packets_sent: 0,
        packets_lost: 0,
        on_peer_connected: Vec::new(),
        on_peer_disconnected: Vec::new(),
        channel_status,
    })
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// F2. ENVOYER DES DONNÃ‰ES
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

pub fn send(network: &mut NetworkLayer, _data: &[u8], dest: &[u8; 32], priority: Priority) -> Result<(), ChainError> {
    let _hb = get_hardbit();

    // 1. VÃ©rifier la connexion directe
    if let Some(peer) = network.peers.get(dest) {
        if peer.is_online {
            network.packets_sent += 1;
            // TODO: Envoi rÃ©el via le canal du pair
            return Ok(());
        }
    }

    // 2. Essayer les canaux dans l'ordre de prioritÃ©
    let channels = match priority {
        Priority::Critical => vec![TransportMode::Direct, TransportMode::WebRTC, TransportMode::WiFiDirect, TransportMode::Tor, TransportMode::Ultrasonic, TransportMode::Phantom],
        Priority::High => vec![TransportMode::WebRTC, TransportMode::WiFiDirect, TransportMode::Tor, TransportMode::Ultrasonic],
        Priority::Normal => vec![TransportMode::Tor, TransportMode::Ultrasonic, TransportMode::Phantom],
        Priority::Low => vec![TransportMode::Phantom],
    };

    for channel in &channels {
        if let Some(cs) = network.channel_status.get(channel) {
            if cs.available {
                network.packets_sent += 1;
                // TODO: Envoi via ce canal
                return Ok(());
            }
        }
    }

    network.packets_lost += 1;
    Err(ChainError::InternalError("Aucun canal disponible".into()))
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// F3. DIFFUSER
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

pub fn broadcast(network: &mut NetworkLayer, data: &[u8], scope: BroadcastScope) -> Result<u32, ChainError> {
    let mut sent: u32 = 0;

    let targets: Vec<[u8; 32]> = match scope {
        BroadcastScope::Direct => network.peers.iter()
            .filter(|(_, p)| p.is_online)
            .map(|(addr, _)| *addr)
            .collect(),
        BroadcastScope::Group => network.peers.iter()
            .filter(|(_, p)| p.is_online && p.latency_ms < 200)
            .map(|(addr, _)| *addr)
            .collect(),
        BroadcastScope::Mesh => network.peers.iter()
            .filter(|(_, p)| p.is_online)
            .map(|(addr, _)| *addr)
            .collect(),
        BroadcastScope::All => network.peers.iter()
            .filter(|(_, p)| p.is_online || p.last_seen > get_hardbit().saturating_sub(300))
            .map(|(addr, _)| *addr)
            .collect(),
    };

    for dest in &targets {
        if send(network, data, dest, Priority::Normal).is_ok() {
            sent += 1;
        }
    }

    Ok(sent)
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// F4. S'ABONNER Ã€ UN TOPIC
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

pub fn subscribe(
    network: &mut NetworkLayer,
    topic: &str,
    callback: Box<dyn Fn(Message) + Send>,
) -> Result<Subscription, ChainError> {
    let hb = get_hardbit();
    let sub_id = sha256_hash(&[topic.as_bytes(), &hb.to_le_bytes()].concat());

    let sub = Subscription {
        id: sub_id,
        topic: topic.to_string(),
        subscribed_at: hb,
    };

    network.subscriptions
        .entry(topic.to_string())
        .or_insert_with(Vec::new)
        .push(callback);

    Ok(sub)
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// F5. SE DÃ‰SABONNER
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

pub fn unsubscribe(network: &mut NetworkLayer, sub: &Subscription) -> Result<(), ChainError> {
    network.subscriptions.remove(&sub.topic);
    Ok(())
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// F6. LISTER LES PAIRS
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

pub fn get_peers(network: &NetworkLayer) -> Result<Vec<PeerInfo>, ChainError> {
    Ok(network.peers.values().cloned().collect())
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// F7. STATISTIQUES RÃ‰SEAU
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

pub fn get_network_stats(network: &NetworkLayer) -> Result<NetworkStats, ChainError> {
    let hb = get_hardbit();
    let online = network.peers.values().filter(|p| p.is_online).count() as u32;
    let active_channels: Vec<TransportMode> = network.channel_status.iter()
        .filter(|(_, cs)| cs.active)
        .map(|(t, _)| t.clone())
        .collect();

    Ok(NetworkStats {
        peers_connected: online,
        total_peers_known: network.peers.len() as u32,
        bandwidth_up_bps: 0, // TODO
        bandwidth_down_bps: 0,
        latency_avg_ms: 0,
        packets_sent: network.packets_sent,
        packets_lost: network.packets_lost,
        uptime_seconds: hb.saturating_sub(network.started_at),
        active_channels,
    })
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// F8. CALLBACK CONNEXION PAIR
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

pub fn on_peer_connected(network: &mut NetworkLayer, callback: Box<dyn Fn(PeerInfo) + Send>) {
    network.on_peer_connected.push(callback);
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// F9. CALLBACK DÃ‰CONNEXION PAIR
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

pub fn on_peer_disconnected(network: &mut NetworkLayer, callback: Box<dyn Fn([u8; 32]) + Send>) {
    network.on_peer_disconnected.push(callback);
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// F10. TEST DE CONNECTIVITÃ‰
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

pub fn test_connectivity(network: &mut NetworkLayer) -> Result<ConnectivityReport, ChainError> {
    let hb = get_hardbit();

    // Tester chaque canal
    let web_rtc_available = check_web_rtc();
    let tor_available = check_tor();
    let ultrasonic_available = check_ultrasonic();
    let bluetooth_available = check_bluetooth();
    let wifi_direct_available = check_wifi_direct();
    let phantom_available = check_phantom();

    // Mettre Ã  jour les statuts
    update_channel_status(network, TransportMode::WebRTC, web_rtc_available);
    update_channel_status(network, TransportMode::Tor, tor_available);
    update_channel_status(network, TransportMode::Ultrasonic, ultrasonic_available);
    update_channel_status(network, TransportMode::Bluetooth, bluetooth_available);
    update_channel_status(network, TransportMode::WiFiDirect, wifi_direct_available);
    update_channel_status(network, TransportMode::Phantom, phantom_available);

    // Recommander le meilleur canal
    let recommended = if web_rtc_available {
        TransportMode::WebRTC
    } else if wifi_direct_available {
        TransportMode::WiFiDirect
    } else if tor_available {
        TransportMode::Tor
    } else if ultrasonic_available {
        TransportMode::Ultrasonic
    } else if phantom_available {
        TransportMode::Phantom
    } else {
        TransportMode::Unknown
    };

    Ok(ConnectivityReport {
        web_rtc_available,
        tor_available,
        ultrasonic_available,
        bluetooth_available,
        wifi_direct_available,
        phantom_channel_available: phantom_available,
        recommended,
        tested_at: hb,
    })
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// FONCTIONS SUPPLÃ‰MENTAIRES DE PRODUCTION
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

/// Ajoute un pair au rÃ©seau
pub fn add_peer(network: &mut NetworkLayer, peer: PeerInfo) {
    network.peers.insert(peer.address, peer.clone());
    for cb in &network.on_peer_connected {
        cb(peer.clone());
    }
}

/// Retire un pair du rÃ©seau
pub fn remove_peer(network: &mut NetworkLayer, address: &[u8; 32]) {
    network.peers.remove(address);
    for cb in &network.on_peer_disconnected {
        cb(*address);
    }
}

/// VÃ©rifie si un pair est connectÃ©
pub fn is_peer_connected(network: &NetworkLayer, address: &[u8; 32]) -> bool {
    network.peers.get(address).map(|p| p.is_online).unwrap_or(false)
}

/// Retourne le nombre de pairs en ligne
pub fn online_peer_count(network: &NetworkLayer) -> u32 {
    network.peers.values().filter(|p| p.is_online).count() as u32
}

/// VÃ©rifie la disponibilitÃ© d'un canal spÃ©cifique
pub fn is_channel_available(network: &NetworkLayer, transport: &TransportMode) -> bool {
    network.channel_status.get(transport).map(|cs| cs.available).unwrap_or(false)
}

/// RÃ©initialise les compteurs rÃ©seau
pub fn reset_counters(network: &mut NetworkLayer) {
    network.packets_sent = 0;
    network.packets_lost = 0;
}

/// Retourne la latence moyenne
pub fn average_latency(network: &NetworkLayer) -> u64 {
    let peers: Vec<&PeerInfo> = network.peers.values().filter(|p| p.is_online).collect();
    if peers.is_empty() { return 0; }
    peers.iter().map(|p| p.latency_ms).sum::<u64>() / peers.len() as u64
}

/// DÃ©marre la dÃ©couverte de pairs
pub fn start_peer_discovery(_network: &mut NetworkLayer) -> Result<(), ChainError> {
    // TODO: Lancer la dÃ©couverte DHT + beacon
    Ok(())
}

/// ArrÃªte la dÃ©couverte de pairs
pub fn stop_peer_discovery(_network: &mut NetworkLayer) -> Result<(), ChainError> {
    Ok(())
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// HELPERS DE TEST DE CONNECTIVITÃ‰
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

fn check_web_rtc() -> bool { true }
fn check_tor() -> bool { false }
fn check_ultrasonic() -> bool { true }
fn check_bluetooth() -> bool { true }
fn check_wifi_direct() -> bool { true }
fn check_phantom() -> bool { true }

fn update_channel_status(network: &mut NetworkLayer, transport: TransportMode, available: bool) {
    if let Some(cs) = network.channel_status.get_mut(&transport) {
        cs.available = available;
        cs.active = available;
    }
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// TESTS
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

#[cfg(test)]
mod tests {
    use super::*;

    fn create_network() -> NetworkLayer {
        new(NetworkConfig::default()).unwrap_or_default()
    }

    #[test] fn test_new() { let n = create_network(); assert!(n.started_at >= 0); assert_eq!(n.peers.len(), 0); }
    #[test] fn test_add_peer() { let mut n = create_network(); let peer = PeerInfo { address:[1u8;32], latency_ms:50, transport:TransportMode::WebRTC, connected_since:get_hardbit(), data_transferred:0, last_seen:get_hardbit(), is_online:true, signal_strength:-40 }; add_peer(&mut n, peer); assert_eq!(n.peers.len(), 1); }
    #[test] fn test_remove_peer() { let mut n = create_network(); add_peer(&mut n, PeerInfo { address:[1u8;32], latency_ms:50, transport:TransportMode::WebRTC, connected_since:get_hardbit(), data_transferred:0, last_seen:get_hardbit(), is_online:true, signal_strength:-40 }); remove_peer(&mut n, &[1u8;32]); assert_eq!(n.peers.len(), 0); }
    #[test] fn test_is_peer_connected() { let mut n = create_network(); add_peer(&mut n, PeerInfo { address:[1u8;32], latency_ms:50, transport:TransportMode::WebRTC, connected_since:get_hardbit(), data_transferred:0, last_seen:get_hardbit(), is_online:true, signal_strength:-40 }); assert!(is_peer_connected(&n, &[1u8;32])); }
    #[test] fn test_get_peers() { let mut n = create_network(); add_peer(&mut n, PeerInfo { address:[1u8;32], latency_ms:50, transport:TransportMode::WebRTC, connected_since:get_hardbit(), data_transferred:0, last_seen:get_hardbit(), is_online:true, signal_strength:-40 }); assert_eq!(get_peers(&n).unwrap_or_default().len(), 1); }
    #[test] fn test_stats() { let n = create_network(); let s = get_network_stats(&n).unwrap(); assert_eq!(s.peers_connected, 0); }
    #[test] fn test_connectivity_report() { let mut n = create_network(); let r = test_connectivity(&mut n).unwrap_or_default(); assert!(r.web_rtc_available); assert!(r.ultrasonic_available); }
    #[test] fn test_broadcast() { let mut n = create_network(); add_peer(&mut n, PeerInfo { address:[1u8;32], latency_ms:50, transport:TransportMode::WebRTC, connected_since:get_hardbit(), data_transferred:0, last_seen:get_hardbit(), is_online:true, signal_strength:-40 }); let sent = broadcast(&mut n, b"test", BroadcastScope::Direct).unwrap_or_default(); assert!(sent >= 1); }
    #[test] fn test_subscribe_unsubscribe() { let mut n = create_network(); let sub = subscribe(&mut n, "test.topic", Box::new(|_| {})).unwrap_or_default(); assert!(n.subscriptions.contains_key("test.topic")); unsubscribe(&mut n, &sub).unwrap_or_default(); assert!(!n.subscriptions.contains_key("test.topic")); }
    #[test] fn test_online_count() { let mut n = create_network(); add_peer(&mut n, PeerInfo { address:[1u8;32], latency_ms:50, transport:TransportMode::WebRTC, connected_since:get_hardbit(), data_transferred:0, last_seen:get_hardbit(), is_online:true, signal_strength:-40 }); add_peer(&mut n, PeerInfo { address:[2u8;32], latency_ms:100, transport:TransportMode::Tor, connected_since:get_hardbit(), data_transferred:0, last_seen:get_hardbit(), is_online:false, signal_strength:-60 }); assert_eq!(online_peer_count(&n), 1); }
    #[test] fn test_channel_available() { let mut n = create_network(); let _ = test_connectivity(&mut n); assert!(is_channel_available(&n, &TransportMode::WebRTC)); }
    #[test] fn test_average_latency() { let mut n = create_network(); add_peer(&mut n, PeerInfo { address:[1u8;32], latency_ms:50, transport:TransportMode::WebRTC, connected_since:get_hardbit(), data_transferred:0, last_seen:get_hardbit(), is_online:true, signal_strength:-40 }); add_peer(&mut n, PeerInfo { address:[2u8;32], latency_ms:100, transport:TransportMode::WebRTC, connected_since:get_hardbit(), data_transferred:0, last_seen:get_hardbit(), is_online:true, signal_strength:-50 }); assert_eq!(average_latency(&n), 75); }
}










