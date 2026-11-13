use std::net::UdpSocket;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;

// ====================
// UDP NETWORK — VRAI SOCKET P2P SANS SERVEUR
// ====================

pub struct UdpNetwork {
    socket: Arc<UdpSocket>,
    peers: Arc<Mutex<Vec<String>>>,
    // Les messages entrants sortent ici ; sans consommateur, ils restent
    // simplement dans le buffer du canal (les pairs se découvrent quand même).
    reception: Arc<Mutex<Option<mpsc::Receiver<(String, String)>>>>,
}

impl UdpNetwork {
    /// Démarre le serveur UDP P2P sur le port spécifié
    pub fn start(port: u16) -> Self {
        let bind_addr = format!("0.0.0.0:{}", port);
        let socket = Arc::new(UdpSocket::bind(&bind_addr).expect("UDP bind"));

        let (tx, rx) = mpsc::channel();
        let peers = Arc::new(Mutex::new(Vec::<String>::new()));
        let network = Self {
            socket: Arc::clone(&socket),
            peers: Arc::clone(&peers),
            reception: Arc::new(Mutex::new(Some(rx))),
        };
        let peers_recv = Arc::clone(&peers);

        // Thread de réception
        let socket_recv = Arc::clone(&socket);

        thread::spawn(move || {
            let mut buf = [0u8; 65535];
            loop {
                match socket_recv.recv_from(&mut buf) {
                    Ok((amt, src_addr)) => {
                        let message = String::from_utf8_lossy(&buf[..amt]).to_string();
                        log::info!("📥 P2P de {} : {}", src_addr, message);

                        // Ajouter le pair s'il n'est pas connu
                        let peer_str = src_addr.to_string();
                        {
                            let mut peers = peers_recv.lock().unwrap();
                            if !peers.contains(&peer_str) {
                                peers.push(peer_str.clone());
                            }
                        }
                        let _ = tx.send((peer_str, message));
                    }
                    Err(e) => {
                        log::error!("Erreur UDP : {}", e);
                    }
                }
            }
        });

        log::info!("✅ UDP P2P démarré sur 0.0.0.0:{}", port);
        network
    }

    /// Récupère le canal des messages entrants (une seule fois).
    pub fn prendre_reception(&self) -> Option<mpsc::Receiver<(String, String)>> {
        self.reception.lock().ok()?.take()
    }
    
    /// Envoie un message à un pair
    pub fn send(&self, peer_addr: &str, message: &str) -> Result<(), String> {
        self.socket
            .send_to(message.as_bytes(), peer_addr)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    /// Ajoute manuellement un pair à la liste
    pub fn add_peer(&self, peer_addr: &str) {
        let mut peers = self.peers.lock().unwrap();
        if !peers.contains(&peer_addr.to_string()) {
            peers.push(peer_addr.to_string());
        }
    }
    
    /// Diffuse à TOUS les pairs connus
    pub fn broadcast(&self, message: &str) -> usize {
        let peers = self.peers.lock().unwrap();
        let mut sent = 0;
        
        for peer in peers.iter() {
            if self.send(peer, message).is_ok() {
                sent += 1;
            }
        }
        
        sent
    }
    
    /// Liste des pairs connus
    pub fn get_peers(&self) -> Vec<String> {
        self.peers.lock().unwrap().clone()
    }
    
    /// Nombre de pairs
    pub fn peer_count(&self) -> usize {
        self.peers.lock().unwrap().len()
    }
}

