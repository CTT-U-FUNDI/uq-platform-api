/// Le séquestre — bloque, rembourse, paie
pub struct Escrow {
    pub deposited_amount: u64,
}

impl Escrow {
    pub fn new() -> Self {
        Self { deposited_amount: 0 }
    }
    
    pub fn deposit(&mut self, amount: u64) -> Result<(), String> {
        self.deposited_amount += amount;
        Ok(())
    }
    
    pub fn refund(&mut self, amount: u64) -> Result<(), String> {
        self.deposited_amount = self.deposited_amount.saturating_sub(amount);
        Ok(())
    }
    
    pub fn pay(&mut self, recipient: [u8; 32], amount: u64) -> Result<(), String> {
        self.deposited_amount = self.deposited_amount.saturating_sub(amount);
        Ok(())
    }
    
    pub fn pay_to_six_cases(&mut self, amount: u64) -> Result<(), String> {
        self.deposited_amount = self.deposited_amount.saturating_sub(amount);
        Ok(())
    }
}
