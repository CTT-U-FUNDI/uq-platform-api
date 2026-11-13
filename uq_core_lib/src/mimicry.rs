use crate::crypto::sha256;
use crate::preamble::UqPreamble;

pub const QUIC_MIN_PACKET_SIZE: usize = 64;
pub const QUIC_MAX_PACKET_SIZE: usize = 1500;

#[derive(Debug)]
pub struct UqQuicMimicry {
    preamble: UqPreamble,
    packet_counter: u64,
}

impl UqQuicMimicry {
    pub fn new(preamble: UqPreamble) -> Self {
        UqQuicMimicry { preamble, packet_counter: 0 }
    }

    pub fn disguise_packet(&mut self, uq_payload: &[u8]) -> Vec<u8> {
        let preamble = self.preamble.generate_current();
        self.packet_counter += 1;
        
        // Taille du payload original (4 octets)
        let payload_len = (uq_payload.len() as u32).to_le_bytes();
        
        // Taille totale (min 64, max 1500)
        let header_size = 8 + 8 + 4 + 8; // quic_header + preamble + payload_len + packet_id
        let total_size = (header_size + uq_payload.len()).max(QUIC_MIN_PACKET_SIZE).min(QUIC_MAX_PACKET_SIZE);
        
        let mut packet = Vec::with_capacity(total_size);
        
        // 1. En-tete QUIC (8 octets)
        let mut quic_header = [0u8; 8];
        quic_header[0] = 0x40 | ((self.packet_counter % 32) as u8);
        let conn_id = sha256(&self.packet_counter.to_le_bytes());
        quic_header[1..5].copy_from_slice(&conn_id[0..4]);
        quic_header[5..8].copy_from_slice(&(self.packet_counter as u32).to_le_bytes()[0..3]);
        packet.extend_from_slice(&quic_header);
        
        // 2. Preambule TOTP (8 octets)
        packet.extend_from_slice(&preamble);
        
        // 3. Taille du payload (4 octets)
        packet.extend_from_slice(&payload_len);
        
        // 4. Packet ID (8 octets)
        packet.extend_from_slice(&self.packet_counter.to_le_bytes());
        
        // 5. Donnees UQ
        packet.extend_from_slice(uq_payload);
        
        // 6. Padding aleatoire
        while packet.len() < total_size {
            packet.push(rand::random::<u8>());
        }
        
        packet
    }

    pub fn extract_payload(&self, quic_packet: &[u8]) -> Option<Vec<u8>> {
        if quic_packet.len() < 28 { return None; } // 8+8+4+8 minimum
        
        // Verifier le preambule
        let received_preamble: [u8; 8] = quic_packet[8..16].try_into().ok()?;
        if !self.preamble.validate(&received_preamble) { return None; }
        
        // Lire la taille du payload (octets 16..20)
        let payload_len = u32::from_le_bytes([quic_packet[16], quic_packet[17], quic_packet[18], quic_packet[19]]) as usize;
        
        // Extraire le payload (apres les 28 octets d'en-tete)
        let start = 28;
        let end = (start + payload_len).min(quic_packet.len());
        
        if end <= start { return Some(Vec::new()); }
        
        Some(quic_packet[start..end].to_vec())
    }

    pub fn get_packet_count(&self) -> u64 { self.packet_counter }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gene::UqGene;
    use crate::crypto::generate_keypair;

    fn test_mimicry() -> UqQuicMimicry {
        let kp = generate_keypair();
        let gene = UqGene::new("uq:test", &kp.public_key).unwrap_or_default();
        let preamble = UqPreamble::new(&gene.get_entropy());
        UqQuicMimicry::new(preamble)
    }

    #[test]
    fn test_disguise_packet() {
        let mut m = test_mimicry();
        let p = m.disguise_packet(b"Hello");
        assert!(p.len() >= QUIC_MIN_PACKET_SIZE);
    }

    #[test]
    fn test_extract_payload() {
        let mut m = test_mimicry();
        let orig = b"Secret message";
        let packet = m.disguise_packet(orig);
        assert_eq!(m.extract_payload(&packet).unwrap_or_default(), orig);
    }

    #[test]
    fn test_extract_empty() {
        let mut m = test_mimicry();
        let packet = m.disguise_packet(b"");
        assert_eq!(m.extract_payload(&packet).unwrap_or_default(), b"");
    }

    #[test]
    fn test_extract_invalid() {
        let mut m = test_mimicry();
        let mut packet = m.disguise_packet(b"test");
        packet[10] ^= 0xFF;
        assert!(m.extract_payload(&packet).is_none());
    }

    #[test]
    fn test_counter() {
        let mut m = test_mimicry();
        let c = m.get_packet_count();
        m.disguise_packet(b"x");
        assert_eq!(m.get_packet_count(), c + 1);
    }
}
