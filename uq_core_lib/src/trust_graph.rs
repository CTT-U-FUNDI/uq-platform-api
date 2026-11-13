//! UQ Trust Graph Module - Graphe de Confiance Distribué

use crate::gene::UqGene;
use std::collections::HashMap;

#[derive(Debug)]
pub struct UqTrustGraph {
    gene: UqGene,
    peers: HashMap<String, UqGene>,
}

impl UqTrustGraph {
    pub fn new(gene: UqGene) -> Self {
        UqTrustGraph { gene, peers: HashMap::new() }
    }

    pub fn update_peer(&mut self, address: &str, gene: UqGene) {
        self.peers.insert(address.to_string(), gene);
    }

    pub fn remove_peer(&mut self, address: &str) {
        self.peers.remove(address);
    }

    pub fn get_trust_level(&self, target: &str) -> f64 {
        if let Some(peer) = self.peers.get(target) {
            return peer.get_trust_level();
        }
        let mut max_trust: f64 = 0.0;
        for (_, peer) in &self.peers {
            if peer.get_trust_level() > 0.5 {
                max_trust = max_trust.max(peer.get_trust_level() * 0.8);
            }
        }
        max_trust.clamp(0.0, 1.0)
    }

    pub fn get_own_gene(&self) -> &UqGene { &self.gene }
    pub fn get_own_gene_mut(&mut self) -> &mut UqGene { &mut self.gene }
    pub fn get_peer_count(&self) -> usize { self.peers.len() }

    pub fn get_high_trust_peers(&self) -> Vec<&String> {
        self.peers.iter().filter(|(_, g)| g.get_trust_level() > 0.8).map(|(a, _)| a).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::generate_keypair;

    fn test_gene(address: &str, trust: f64) -> UqGene {
        let kp = generate_keypair();
        let mut gene = UqGene::new(address, &kp.public_key).unwrap_or_default();
        gene.update_metrics(trust, trust, trust, trust, trust);
        gene.evolve();
        gene
    }

    #[test]
    fn test_new() {
        let g = UqTrustGraph::new(test_gene("uq:me", 0.7));
        assert_eq!(g.get_peer_count(), 0);
    }

    #[test]
    fn test_add_peer() {
        let mut g = UqTrustGraph::new(test_gene("uq:me", 0.7));
        g.update_peer("uq:alice", test_gene("uq:alice", 0.9));
        assert_eq!(g.get_peer_count(), 1);
        assert!(g.get_trust_level("uq:alice") > 0.8);
    }

    #[test]
    fn test_transitive() {
        let mut g = UqTrustGraph::new(test_gene("uq:me", 0.7));
        g.update_peer("uq:alice", test_gene("uq:alice", 0.9));
        let t = g.get_trust_level("uq:bob");
        assert!(t > 0.0 && t < 0.9);
    }

    #[test]
    fn test_high_trust() {
        let mut g = UqTrustGraph::new(test_gene("uq:me", 0.7));
        g.update_peer("uq:hi", test_gene("uq:hi", 0.95));
        g.update_peer("uq:lo", test_gene("uq:lo", 0.2));
        let high: Vec<String> = g.get_high_trust_peers().iter().map(|s| s.to_string()).collect();
        assert!(high.contains(&"uq:hi".to_string()));
        assert!(!high.contains(&"uq:lo".to_string()));
    }

    #[test]
    fn test_remove() {
        let mut g = UqTrustGraph::new(test_gene("uq:me", 0.7));
        g.update_peer("uq:x", test_gene("uq:x", 0.5));
        assert_eq!(g.get_peer_count(), 1);
        g.remove_peer("uq:x");
        assert_eq!(g.get_peer_count(), 0);
    }
}
