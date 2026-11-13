use serde::{Serialize, Deserialize};

/// Une offre de données mobiles
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataOffer {
    pub id: [u8; 32],
    pub seller: [u8; 32],
    pub price_per_mb_micro: u64,
    pub max_data_mb: u64,
    pub signal_strength_dbm: i8,
    pub network_type: String,
    pub location: Option<GeoPoint>,
    pub created_at: i64,
    pub expires_at: i64,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeoPoint {
    pub lat: f64,
    pub lon: f64,
}

/// Une session d'\''achat active
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataSession {
    pub id: [u8; 32],
    pub offer_id: [u8; 32],
    pub buyer: [u8; 32],
    pub seller: [u8; 32],
    pub started_at: i64,
    pub bytes_used: u64,
    pub max_bytes: u64,
    pub paid_amount_micro: u64,
    pub tunnel_key: [u8; 32],
    pub active: bool,
}

/// Configuration du tunnel
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelConfig {
    pub server_public_key: [u8; 32],
    pub client_private_key: [u8; 32],
    pub server_endpoint: String,
    pub allowed_ips: Vec<String>,
    pub mtu: u16,
}

/// Statistiques du marché
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketStats {
    pub active_sellers: u64,
    pub active_buyers: u64,
    pub total_data_sold_mb: u64,
    pub total_volume_micro: u64,
    pub average_price_per_mb: u64,
}
