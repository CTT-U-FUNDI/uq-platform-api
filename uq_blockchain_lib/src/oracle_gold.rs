
/// Prix BORB indexé sur le cours de l'or
pub struct GoldOracle {
    pub gold_price_usd_per_kg: u64,
    pub price: u64,
    pub last_update: u64,
}

impl GoldOracle {
    pub fn new() -> Self {
        GoldOracle {
            gold_price_usd_per_kg: 100_000,
            price: 100_000,
            last_update: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        }
    }

    pub fn update_price(&mut self, new_gold_price_usd_per_kg: u64) {
        self.gold_price_usd_per_kg = new_gold_price_usd_per_kg;
        self.price = new_gold_price_usd_per_kg;
        self.last_update = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
    }

    pub fn usd_to_BORB(&self, usd_amount: u64) -> u64 {
        if self.price == 0 { return 0; }
        usd_amount * 1_000_000 / self.price
    }

    pub fn BORB_to_usd(&self, BORB_micro: u64) -> u64 {
        BORB_micro * self.price / 1_000_000
    }

    pub fn get_price(&self) -> u64 { self.price }

    /// Calcule la valeur actuelle d'une bulle en USD
    pub fn bubble_value_usd(&self) -> f64 {
        self.price as f64 / crate::constants::BORB_PER_OUNCE as f64
    }

    /// Calcule la valeur d'un point en USD
    pub fn point_value_usd(&self) -> f64 {
        self.bubble_value_usd() / crate::constants::BULLES_PER_BORB as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] fn test_initial_price() { let o = GoldOracle::new(); assert_eq!(o.price, 100_000); }
    #[test] fn test_update_price() { let mut o = GoldOracle::new(); o.update_price(120_000); assert_eq!(o.price, 120_000); }
    #[test] fn test_usd_to_BORB() { let o = GoldOracle::new(); let BORB = o.usd_to_BORB(100_000); assert_eq!(BORB, 1_000_000); }
    #[test] fn test_BORB_to_usd() { let o = GoldOracle::new(); let usd = o.BORB_to_usd(1_000_000); assert_eq!(usd, 100_000); }
    #[test] fn test_bubble_value() { let o = GoldOracle::new(); let v = o.bubble_value_usd(); assert!(v > 0.0); }
    #[test] fn test_point_value() { let o = GoldOracle::new(); let p = o.point_value_usd(); assert!(p > 0.0); }
}


