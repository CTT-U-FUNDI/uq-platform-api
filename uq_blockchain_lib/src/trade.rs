use serde::{Serialize, Deserialize};
use crate::error::ChainError;

/// Statut d'un trade P2P
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TradeStatus {
    PendingSellerDeposit,    // Vendeur doit déposer les BORB
    SellerDeposited,         // BORB bloqués dans l'escrow
    PendingBuyerPayment,     // Acheteur doit payer en fiat
    BuyerPaymentProof,       // Preuve de paiement soumise
    PendingSellerConfirm,    // Vendeur doit confirmer réception
    Completed,               // Trade terminé ✅
    Disputed,                // Litige en cours
    Cancelled,               // Annulé
    Refunded,                // Remboursé
}

/// Transaction P2P Trade (acheter/vendre BORB contre fiat)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct P2PTrade {
    pub id: [u8; 32],
    pub seller: [u8; 32],           // Vendeur BORB
    pub buyer: [u8; 32],            // Acheteur BORB
    pub BORB_amount: u64,          // Montant en BORB
    pub fiat_amount: u64,           // Montant en monnaie fiat
    pub fiat_currency: String,      // "CDF", "USD", "EUR", etc.
    pub payment_method: String,     // "MOBILE_MONEY", "BANK", "PAYPAL"
    pub status: TradeStatus,
    pub escrow_id: [u8; 32],        // ID du contrat escrow
    pub proof_hash: Option<[u8; 32]>, // Hash de la preuve de paiement
    pub created_at: i64,
    pub expires_at: i64,            // Timeout
    pub seller_confirmed: bool,
    pub buyer_confirmed: bool,
}

impl P2PTrade {
    pub fn new(
        seller: [u8; 32],
        buyer: [u8; 32],
        BORB_amount: u64,
        fiat_amount: u64,
        fiat_currency: String,
        payment_method: String,
    ) -> Result<Self, ChainError> {
        if BORB_amount == 0 {
            return Err(ChainError::InvalidTransaction("Montant BORB nul".into()));
        }
        if fiat_amount == 0 {
            return Err(ChainError::InvalidTransaction("Montant fiat nul".into()));
        }
        
        let now = chrono::Utc::now().timestamp();
        
        Ok(P2PTrade {
            id: [0u8; 32],
            seller,
            buyer,
            BORB_amount,
            fiat_amount,
            fiat_currency,
            payment_method,
            status: TradeStatus::PendingSellerDeposit,
            escrow_id: [0u8; 32],
            proof_hash: None,
            created_at: now,
            expires_at: now + 86400, // 24h
            seller_confirmed: false,
            buyer_confirmed: false,
        })
    }
    
    pub fn is_expired(&self) -> bool {
        chrono::Utc::now().timestamp() > self.expires_at
    }
    
    pub fn can_auto_complete(&self) -> bool {
        self.status == TradeStatus::PendingSellerConfirm && self.is_expired()
    }
}

