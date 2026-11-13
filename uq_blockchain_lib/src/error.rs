use thiserror::Error;

#[derive(Error, Debug)]
pub enum ChainError {
    #[error("Transaction invalide: {0}")]
    InvalidTransaction(String),
    
    #[error("Bloc invalide: {0}")]
    InvalidBlock(String),
    
    #[error("Escrow échoué: {0}")]
    EscrowFailed(String),
    
    #[error("Preuve invalide: {0}")]
    InvalidProof(String),
    
    #[error("Arbitrage refusé: {0}")]
    ArbitrationDenied(String),
    
    #[error("Fonds insuffisants: {0}")]
    InsufficientFunds(String),
    
    #[error("Score de confiance trop bas: {0}")]
    TrustTooLow(String),
    
    #[error("Fraude détectée: {0}")]
    FraudDetected(String),
    
    #[error("Erreur interne: {0}")]
    InternalError(String),

    #[error("Limite d'assurance atteinte: {0}")]
    InsuranceLimitReached(String),

    #[error("Réclamation invalide: {0}")]
    InvalidClaim(String),
}
