/// Les statistiques du lecteur UQ
pub struct PlayerStats {
    pub total_sessions: u64,
    pub total_seconds: u64,
    pub total_paid: u64,
}

impl PlayerStats {
    pub fn new() -> Self {
        Self {
            total_sessions: 0,
            total_seconds: 0,
            total_paid: 0,
        }
    }
}
