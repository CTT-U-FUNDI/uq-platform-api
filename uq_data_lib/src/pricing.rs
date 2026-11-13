// Pricing module for data market
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceQuote {
    pub price_per_mb: u64,
    pub currency: String,
}

pub fn calculate_price(data_size_bytes: u64, price_per_mb: u64) -> u64 {
    let mb = data_size_bytes / 1_000_000;
    if mb == 0 { price_per_mb / 1000 } else { mb * price_per_mb }
}

