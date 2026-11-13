use crate::constants::*;
use crate::bubble::BubbleRegistry;
use crate::error::ChainError;

#[derive(Debug, Clone, PartialEq)]
pub enum ProfileType {
    Personal,
    Professional,
    Merchant,
    Artist,
    Developer,
    Spiritual,
}

impl ProfileType {
    pub fn activation_fee(&self) -> u64 {
        match self {
            ProfileType::Personal => 0,
            ProfileType::Professional => PROFILE_PRO_FEE,
            ProfileType::Merchant => PROFILE_MERCHANT_FEE,
            ProfileType::Artist => PROFILE_ARTIST_FEE,
            ProfileType::Developer => PROFILE_DEV_FEE,
            ProfileType::Spiritual => PROFILE_SPIRITUAL_FEE,
        }
    }
}

/// Active un profil — verifie le solde via BubbleRegistry
pub fn activate_profile(
    user: [u8; 32],
    profile_type: ProfileType,
    registry: &mut BubbleRegistry,
) -> Result<(), ChainError> {
    let fee = profile_type.activation_fee();
    
    if fee > 0 {
        let balance = registry.get_balance(&user);
        let fee_micro = fee * MICRO_PER_BORB;
        
        if balance < fee_micro {
            return Err(ChainError::InternalError("Fonds insuffisants".into()));
        }
        
        // Le debit reel est fait par le handler API
    }
    
    Ok(())
}
