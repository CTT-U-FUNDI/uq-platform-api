use crate::models::{DataOffer, DataSession};

use crate::error::ChainError;
use sha2::{Sha256, Digest};

pub fn find_offers(
    market_offers: &[DataOffer],
    min_signal_dbm: i8,
    max_price_per_mb_micro: u64,
) -> Vec<&DataOffer> {
    market_offers
        .iter()
        .filter(|o| o.signal_strength_dbm >= min_signal_dbm && o.price_per_mb_micro <= max_price_per_mb_micro)
        .collect()
}

pub fn request_session(
    offer: &DataOffer,
    buyer_address: &[u8; 32],
    max_bytes: u64,
) -> Result<DataSession, ChainError> {
    if max_bytes == 0 {
        return Err(ChainError::InternalError("Taille invalide".into()));
    }
    let id = Sha256::digest(
        format!("{}{}{}", hex::encode(buyer_address), hex::encode(&offer.seller), max_bytes).as_bytes()
    ).into();
    Ok(DataSession {
        id,
        offer_id: offer.id,
        buyer: *buyer_address,
        seller: offer.seller,
        started_at: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64,
        bytes_used: 0,
        max_bytes,
        paid_amount_micro: 0,
        tunnel_key: [0u8; 32],
        active: true,
    })
}

pub fn connect_proxy(_host: String, port: u16) -> Result<(), ChainError> {
    if port == 0 { return Err(ChainError::InternalError("Port invalide".into())); }
    Ok(())
}



