use crate::models::{DataOffer, GeoPoint};

use crate::error::ChainError;
use sha2::{Sha256, Digest};

pub fn create_offer(
    seller_address: &[u8; 32],
    price_per_mb_micro: u64,
    max_data_mb: u64,
    signal_strength_dbm: i8,
    network_type: String,
    location: Option<GeoPoint>,
) -> Result<DataOffer, ChainError> {
    if price_per_mb_micro == 0 { return Err(ChainError::InternalError("Prix invalide".into())); }
    if signal_strength_dbm > -30 || signal_strength_dbm < -113 { return Err(ChainError::InternalError("Signal invalide".into())); }
    let id = Sha256::digest(format!("{}{}{}", hex::encode(seller_address), price_per_mb_micro, max_data_mb).as_bytes()).into();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
    Ok(DataOffer {
        id, seller: *seller_address, price_per_mb_micro, max_data_mb,
        signal_strength_dbm, network_type, location, created_at: now,
        expires_at: now + 86400, active: true,
    })
}



