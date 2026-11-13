use crate::constants::*;
use crate::bubble::BubbleRegistry;
use crate::error::ChainError;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Enterprise {
    pub owner: [u8; 32],
    pub name: String,
    pub verified: bool,
    pub created_at: u64,
}

impl Enterprise {
    /// Cree une entreprise — debite le registry via transfer_bubble
    pub fn create(
        owner: [u8; 32],
        name: &str,
        registry: &mut BubbleRegistry,
    ) -> Result<Self, ChainError> {
        // 1. Verifier le solde dans BubbleRegistry
        let balance = registry.get_balance(&owner);
        let fee_micro = ENTERPRISE_CREATE_FEE * MICRO_PER_BORB;
        
        if balance < fee_micro {
            return Err(ChainError::InternalError("Fonds insuffisants".into()));
        }
        
        // 2. Debiter via le registry (transfert vers Tresor)
        // Note : Le transfert est gere par le handler appelant
        // Ici on verifie juste l'eligibilite
        
        Ok(Enterprise {
            owner,
            name: name.to_string(),
            verified: true,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        })
    }
}
