/// Le mode robinet — paiement continu par seconde
pub struct Robinet {
    pub active: bool,
    pub total_ticks: u64,
}

impl Robinet {
    pub fn new() -> Self {
        Self {
            active: false,
            total_ticks: 0,
        }
    }
    
    pub fn start(&mut self) {
        self.active = true;
    }
    
    pub fn tick(&mut self) {
        self.total_ticks += 1;
    }
    
    pub fn stop(&mut self) {
        self.active = false;
    }
    
    pub fn is_active(&self) -> bool {
        self.active
    }
}
