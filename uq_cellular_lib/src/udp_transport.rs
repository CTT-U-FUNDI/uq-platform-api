use std::net::UdpSocket;
use std::sync::Arc;
use uq_blockchain::error::ChainError;
use crate::transport::PhysicalTransport;

/// VRAI transport UDP pour le réseau P2P D2D
pub struct UdpTransport {
    socket: Arc<UdpSocket>,
}

impl UdpTransport {
    /// Crée un vrai transport UDP qui écoute sur 0.0.0.0:8080
    pub fn new(bind_addr: &str) -> Self {
        let socket = Arc::new(UdpSocket::bind(bind_addr).expect("UDP bind failed"));
        socket.set_nonblocking(true).ok();
        // 🆕 Activer le broadcast UDP
        socket.set_broadcast(true).ok();
        Self { socket }
    }

    /// Envoie un VRAI paquet UDP vers un destinataire
    pub fn send_to(&self, data: &[u8], dest: &str) -> Result<usize, std::io::Error> {
        self.socket.send_to(data, dest)
    }

    /// 🆕 Diffuse un paquet UDP en broadcast
    pub fn broadcast_to(&self, data: &[u8], port: u16) -> Result<usize, std::io::Error> {
        let broadcast_addr = format!("255.255.255.255:{}", port);
        self.socket.send_to(data, broadcast_addr)
    }

    /// Reçoit un VRAI paquet UDP
    pub fn recv_from(&self, buf: &mut [u8]) -> Result<(usize, std::net::SocketAddr), std::io::Error> {
        self.socket.recv_from(buf)
    }

    /// Adresse locale du socket
    pub fn local_addr(&self) -> std::io::Result<std::net::SocketAddr> {
        self.socket.local_addr()
    }
}

/// Implémentation du trait PhysicalTransport pour UdpTransport
impl PhysicalTransport for UdpTransport {
    fn send_bytes(&self, data: &[u8]) -> Result<(), ChainError> {
        self.socket.send(data)
            .map_err(|e| ChainError::InternalError(e.to_string()))?;
        Ok(())
    }

    fn receive_bytes(&self, buffer: &mut [u8]) -> Result<usize, ChainError> {
        self.socket.recv(buffer)
            .map_err(|e| ChainError::InternalError(e.to_string()))
    }

    fn is_connected(&self) -> bool {
        self.socket.local_addr().is_ok()
    }

    fn get_mtu(&self) -> usize {
        1500
    }

    /// 🆕 Broadcast UDP
    fn broadcast_bytes(&self, data: &[u8]) -> Result<(), ChainError> {
        let port = self.local_addr().map(|a| a.port()).unwrap_or(8080);
        self.broadcast_to(data, port)
            .map_err(|e| ChainError::InternalError(e.to_string()))?;
        Ok(())
    }
}



