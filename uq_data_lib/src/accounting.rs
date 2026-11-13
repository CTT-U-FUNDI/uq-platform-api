// Accounting module for data market
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionRecord {
    pub buyer: String,
    pub seller: String,
    pub amount: u64,
    pub timestamp: u64,
}

pub fn record_transaction(buyer: &str, seller: &str, amount: u64) -> TransactionRecord {
    TransactionRecord {
        buyer: buyer.to_string(),
        seller: seller.to_string(),
        amount,
        timestamp: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs(),
    }
}

