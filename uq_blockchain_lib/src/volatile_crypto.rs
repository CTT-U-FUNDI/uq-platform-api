use crate::error::ChainError;
use uq_black_key_standalone::UqBlackKey;

/// Chiffrement volatile utilisant UQ_BLACK_KEY 512 bits
pub fn encrypt_volatile(data: &[u8], key: &mut UqBlackKey) -> Result<Vec<u8>, ChainError> {
    let encrypted = key.encrypt(data);
    match encrypted {
        Ok(result) => Ok(result),
        Err(e) => Err(ChainError::InternalError(e)),
    }
}

/// Déchiffrement volatile utilisant UQ_BLACK_KEY 512 bits
pub fn decrypt_volatile(data: &[u8], key: &mut UqBlackKey) -> Result<Vec<u8>, ChainError> {
    let decrypted = key.decrypt(data);
    match decrypted {
        Ok(result) => Ok(result),
        Err(e) => Err(ChainError::InternalError(e)),
    }
}