use crate::database::Database;

// ====================
// CONTENT PAYMENT — LE ROBINET 90/10 EN FEMTO
// ====================

pub struct ContentPayment {
    db: Database,
}

impl ContentPayment {
    /// Crée le système de paiement du contenu
    pub fn new(db: Database) -> Self {
        Self { db }
    }
    
    /// LE CRÉATEUR FIXE SON PRIX PAR SECONDE
    pub fn set_creator_price(&self, creator_id: &str, price_per_second: u128) {
        self.db.set_creator_price(creator_id, price_per_second);
    }
    
    /// LE ROBINET DISTRIBUE LE PAIEMENT (90/10)
    pub fn stream(
        &self,
        creator_id: &str,
        price_per_second: u128,
        seconds: u64,
    ) -> Result<u128, String> {
        let total = price_per_second * seconds as u128;
        
        // 90% au créateur
        let creator = total * 90 / 100;
        let cases = total * 10 / 100;
        
        // Les 6 cases reçoivent leurs parts du 10%
        let treasury = cases * 35 / 100;    // 3.5%
        let fees = cases * 15 / 100;         // 1.5%
        let rewards = cases * 15 / 100;      // 1.5%
        let savings = cases * 10 / 100;      // 1%
        let credit = cases * 15 / 100;       // 1.5%
        let shield = cases * 10 / 100;       // 1%
        
        // ÉCRITURE ATOMIQUE — 7 transferts dans le MÊME batch
        self.db.atomic_content_payment(
            creator_id, creator,
            treasury, fees, rewards, savings, credit, shield
        )?;
        
        log::info!(
            "🎵 Contenu streamé : {} BORB → Créateur 90%, 6 cases 10%",
            total
        );
        
        Ok(creator)
    }
    
    /// Statistiques du contenu
    pub fn get_stats(&self) -> serde_json::Value {
        serde_json::json!({
            "creator_pct": 90,
            "cases_pct": 10,
            "total_streamed": self.db.get_total_streamed()
        })
    }
}
