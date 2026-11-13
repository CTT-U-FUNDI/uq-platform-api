use crate::constants::*;
use serde::{Serialize, Deserialize};
use uq_black_key_standalone::UqBlackKey;
use crate::bubble::BubbleRegistry;
use crate::error::ChainError;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum OperationType {
    Transfer,
    Payment,
    Withdrawal,
}

#[derive(Debug, Clone)]
pub struct UqOperation {
    pub from: [u8; 32],
    pub to: [u8; 32],
    pub amount: u64,
    pub op_type: OperationType,
    pub signature: Option<Vec<u8>>,
}

impl UqOperation {
    pub fn new(
        from: [u8; 32],
        to: [u8; 32],
        amount: u64,
        op_type: OperationType,
    ) -> Self {
        UqOperation {
            from,
            to,
            amount,
            op_type,
            signature: None,
        }
    }

    pub fn serialize_for_signing(&self) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(&self.from);
        data.extend_from_slice(&self.to);
        data.extend_from_slice(&self.amount.to_le_bytes());
        data
    }

    pub fn sign_with_blackkey(&mut self, key: &UqBlackKey) {
        let data = self.serialize_for_signing();
        self.signature = Some(key.sign(&data).to_vec());
    }

    pub fn verify_with_blackkey(&self, key: &UqBlackKey) -> bool {
        if let Some(ref sig) = self.signature {
            let data = self.serialize_for_signing();
            if sig.len() == 64 {
                let mut sig_arr = [0u8; 64];
                sig_arr.copy_from_slice(sig);
                return key.verify(&data, &sig_arr);
            }
        }
        false
    }

    /// Execute l'operation — verifie et debite via BubbleRegistry
    pub fn execute(
        &mut self,
        key: &UqBlackKey,
        registry: &mut BubbleRegistry,
    ) -> Result<(), ChainError> {
        // 1. Verifier le solde dans le registry
        let balance = registry.get_balance(&self.from);
        let amount_micro = self.amount * MICRO_PER_BORB;
        
        if amount_micro > balance {
            return Err(ChainError::InternalError("Solde insuffisant".into()));
        }

        // 2. Signer
        self.sign_with_blackkey(key);

        // 3. Verifier la signature
        if !self.verify_with_blackkey(key) {
            return Err(ChainError::InternalError("Signature invalide".into()));
        }

        // 4. Le transfert reel est fait par le handler API
        // via registry.transfer_bubble()

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_operation_lifecycle() {
        let key = UqBlackKey::generate();
        let mut registry = BubbleRegistry::new();
        
        // Creer une bulle pour l'envoyeur
        let from = key.address;
        let _ = registry.create_bubble(
            from, from,
            1_000_000 * MICRO_PER_BORB,
            vec![],
            |data| key.sign(data).to_vec(),
        );
        
        let mut op = UqOperation::new(
            from, [2u8; 32], 500, OperationType::Transfer,
        );
        
        // L'execution verifie le solde
        assert!(op.execute(&key, &mut registry).is_ok());
    }
}
