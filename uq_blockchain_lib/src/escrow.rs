use serde::{Serialize, Deserialize};
use crate::error::ChainError;
use sha2::{Sha256, Digest};

/// Contrat Smart Escrow (séquestre intelligent)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscrowContract {
    pub id: [u8; 32],
    pub trade_id: [u8; 32],
    pub seller: [u8; 32],
    pub buyer: [u8; 32],
    pub amount: u64,                // Montant bloqué
    pub collateral_seller: u64,     // Caution vendeur
    pub collateral_buyer: u64,      // Caution acheteur
    pub created_at: i64,
    pub timeout: i64,               // Timeout avant libération automatique
    pub status: EscrowStatus,
    pub release_conditions: ReleaseConditions,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EscrowStatus {
    Active,         // Fonds bloqués
    Released,       // Fonds libérés à l'acheteur
    Refunded,       // Fonds retournés au vendeur
    Disputed,       // En litige
    Expired,        // Timeout dépassé
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseConditions {
    pub require_seller_confirm: bool,
    pub require_buyer_confirm: bool,
    pub require_proof: bool,
    pub auto_release_after: Option<i64>,  // Secondes avant libération auto
    pub min_arbitrators: u32,             // Nombre minimum d'arbitres
}

impl EscrowContract {
    pub fn new(
        trade_id: [u8; 32],
        seller: [u8; 32],
        buyer: [u8; 32],
        amount: u64,
    ) -> Self {
        let mut contract = EscrowContract {
            id: [0u8; 32],
            trade_id,
            seller,
            buyer,
            amount,
            collateral_seller: 0,
            collateral_buyer: 0,
            created_at: chrono::Utc::now().timestamp(),
            timeout: 86400, // 24h
            status: EscrowStatus::Active,
            release_conditions: ReleaseConditions {
                require_seller_confirm: true,
                require_buyer_confirm: true,
                require_proof: true,
                auto_release_after: Some(172800), // 48h
                min_arbitrators: 5,
            },
        };
        contract.id = contract.compute_id();
        contract
    }
    
    fn compute_id(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(&self.trade_id);
        hasher.update(&self.seller);
        hasher.update(&self.buyer);
        hasher.update(&self.amount.to_le_bytes());
        let result = hasher.finalize();
        let mut id = [0u8; 32];
        id.copy_from_slice(&result);
        id
    }
    
    pub fn can_release(&self, seller_confirmed: bool, buyer_confirmed: bool, has_proof: bool) -> bool {
        if seller_confirmed && buyer_confirmed && has_proof {
            return true;
        }
        if self.is_expired() {
            return true;
        }
        false
    }
    
    pub fn is_expired(&self) -> bool {
        chrono::Utc::now().timestamp() > self.created_at + self.timeout
    }
    
    pub fn release(&mut self) -> Result<(), ChainError> {
        if self.status != EscrowStatus::Active {
            return Err(ChainError::EscrowFailed("Contrat non actif".into()));
        }
        self.status = EscrowStatus::Released;
        Ok(())
    }
    
    pub fn refund(&mut self) -> Result<(), ChainError> {
        if self.status != EscrowStatus::Active {
            return Err(ChainError::EscrowFailed("Contrat non actif".into()));
        }
        self.status = EscrowStatus::Refunded;
        Ok(())
    }
}
