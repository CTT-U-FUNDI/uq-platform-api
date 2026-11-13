use crate::gene::UqGene;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Capability { SendMessage, ReceiveMessage, RelayMessage, StoreData, ValidateBlock, CreateChannel }

#[derive(Debug)]
pub struct UqPermissions {
    gene: UqGene,
    modules: HashMap<String, Vec<Capability>>,
    default_caps: Vec<Capability>,
}

impl UqPermissions {
    pub fn new(gene: UqGene) -> Self {
        UqPermissions {
            gene,
            modules: HashMap::new(),
            default_caps: vec![Capability::SendMessage, Capability::ReceiveMessage],
        }
    }

    pub fn register_module(&mut self, module_id: &str, capabilities: Vec<Capability>) {
        self.modules.insert(module_id.to_string(), capabilities);
    }

    pub fn has_permission(&self, module_id: &str, capability: &Capability) -> bool {
        let caps = self.modules.get(module_id).unwrap_or(&self.default_caps);
        caps.contains(capability) && self.gene.get_trust_level() >= self.min_trust(capability)
    }

    fn min_trust(&self, c: &Capability) -> f64 {
        match c {
            Capability::SendMessage | Capability::ReceiveMessage => 0.0,
            Capability::RelayMessage => 0.5,
            Capability::StoreData => 0.6,
            Capability::CreateChannel => 0.7,
            Capability::ValidateBlock => 0.9,
        }
    }

    pub fn get_gene(&self) -> &UqGene { &self.gene }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::generate_keypair;

    fn test_perm(trust: f64) -> UqPermissions {
        let kp = generate_keypair();
        let mut gene = UqGene::new("uq:test", &kp.public_key).unwrap_or_default();
        // Forcer le trust_level manuellement pour le test
        gene.update_metrics(trust, trust, trust, trust, trust);
        gene.trust_level = trust;
        let mut p = UqPermissions::new(gene);
        p.register_module("chain", vec![Capability::ValidateBlock]);
        p
    }

    #[test]
    fn test_register() { 
        let p = test_perm(0.5); 
        assert!(p.has_permission("chat", &Capability::SendMessage)); 
    }

    #[test]
    fn test_low_trust_cant_validate() {
        let p = test_perm(0.3);
        assert!(!p.has_permission("chain", &Capability::ValidateBlock));
    }

    #[test]
    fn test_high_trust_can_validate() {
        let p = test_perm(0.95);
        assert!(p.has_permission("chain", &Capability::ValidateBlock));
    }

    #[test]
    fn test_send_always_allowed() {
        let p = test_perm(0.1);
        assert!(p.has_permission("chat", &Capability::SendMessage));
    }
}
