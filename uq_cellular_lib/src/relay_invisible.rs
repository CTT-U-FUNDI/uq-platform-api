pub struct RelayInvisible {
    active: bool,
    daily_earnings: u64,
}

impl RelayInvisible {
    pub fn new() -> Self { RelayInvisible { active: false, daily_earnings: 0 } }
    pub fn enable(&mut self, _bw: u32, _max: u8, _bat: u8) { self.active = true; }
    pub fn disable(&mut self) { self.active = false; }
    pub fn is_active(&self) -> bool { self.active }
    pub fn get_daily_earnings(&self) -> u64 { self.daily_earnings }
    pub fn add_earnings(&mut self, amount: u64) { self.daily_earnings += amount; }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn test_enable() { let mut r = RelayInvisible::new(); r.enable(1000,5,80); assert!(r.is_active()); }
    #[test] fn test_disable() { let mut r = RelayInvisible::new(); r.enable(1000,5,80); r.disable(); assert!(!r.is_active()); }
}
