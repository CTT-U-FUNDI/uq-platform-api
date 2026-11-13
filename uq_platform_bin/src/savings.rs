use crate::database::Database;

// ====================
// ÉPARGNE — RENDEMENT ADOSSÉ AU CRÉDIT RÉEL
// ====================

pub struct SavingsSystem {
    db: Database,
}

impl SavingsSystem {
    /// Crée le système d'épargne
    pub fn new(db: Database) -> Self {
        Self { db }
    }
    
    /// CRÉE UN CONTRAT D'ÉPARGNE (gel des BORB)
    pub fn create_contract(&self, account_id: &str, amount: u128, term_months: u64) -> Result<String, String> {
        // Vérifier le solde
        let balance = self.db.get_balance(&format!("account:{}:balance", account_id));
        if balance < amount {
            return Err("Solde insuffisant".to_string());
        }
        
        // Geler les BORB : transférer de l'utilisateur vers la case Épargne
        self.db.atomic_write(
            &format!("account:{}:balance", account_id),
            "case:savings:balance",
            amount
        )?;
        
        // Créer le contrat
        let contract_id = format!("contract_{}_{}", account_id, chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0));
        let start_time = chrono::Utc::now().timestamp();
        let end_time = start_time + (term_months as i64 * 30 * 24 * 3600);
        
        self.db.save_contract(&contract_id, account_id, amount, start_time, end_time);
        
        Ok(contract_id)
    }
    
    /// CALCULE LE RENDEMENT RÉEL
    /// Rendement = intérêts du crédit / total épargne
    pub fn calculate_yield(&self) -> f64 {
        let credit_interests = self.db.get_credit_interests_last_year();
        let total_savings = self.db.get_balance("case:savings:balance");
        
        if total_savings == 0 {
            return 0.0;
        }
        
        (credit_interests as f64 / total_savings as f64) * 100.0
    }
    
    /// DISTRIBUE LES RENDEMENTS AUX ÉPARGNANTS
    pub fn distribute_yield(&self) -> u128 {
        let yield_rate = self.calculate_yield();
        let total_savings = self.db.get_balance("case:savings:balance");
        let total_yield = (total_savings as f64 * yield_rate / 100.0) as u128;
        
        // Distribue proportionnellement à chaque épargnant
        self.db.distribute_savings_yield(total_yield);
        
        total_yield
    }
    
    /// Statistiques de l'épargne
    pub fn get_stats(&self) -> serde_json::Value {
        serde_json::json!({
            "total_savings": self.db.get_balance("case:savings:balance"),
            "yield_rate_pct": self.calculate_yield(),
            "contracts_active": self.db.get_contract_count()
        })
    }
}
