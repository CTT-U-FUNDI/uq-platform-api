/// La session de lecture
pub struct PlayerSession {
    pub creator: [u8; 32],
    pub price_per_second: u64,
    pub seconds_consumed: u64,
    pub started_at: i64,
}

impl PlayerSession {
    pub fn new(creator: [u8; 32], price_per_second: u64, started_at: i64) -> Self {
        Self {
            creator,
            price_per_second,
            seconds_consumed: 0,
            started_at,
        }
    }
}
