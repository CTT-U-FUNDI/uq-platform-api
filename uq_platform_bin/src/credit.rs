use crate::database::Database;

// ====================
// CRÉDIT — BORB NÉGATIF, DETTES, PRÉLÈVEMENT AUTOMATIQUE
// ====================

pub struct CreditSystem {
    db: Database,
}

impl CreditSystem {
    /// Crée le système de crédit
    pub fn new(db: Database) -> Self {
        Self { db }
    }
    
    /// ACCORDE UNE AVANCE (crée une dette)
    pub fn lend(&self, borrower: &str, amount: u128, collateral: u128) -> Result<(), String> {
        // Vérifier la garantie
        let borrower_savings = self.db.get_balance(&format!("savings:{}", borrower));
        if borrower_savings < collateral {
            return Err("Garantie insuffisante".to_string());
        }
        
        // NANTIR les épargnes (garantie collatérale)
        self.db.nantir_savings(borrower, collateral);
        
        // Créer la dette : solde peut devenir NÉGATIF
        let current_balance = self.db.get_balance(&format!("account:{}:balance", borrower));
        self.db.set_balance(
            &format!("account:{}:balance", borrower),
            current_balance + amount
        );
        
        // Enregistrer la dette
        self.db.set_debt(borrower, amount);
        
        log::info!("🤝 Avance accordée : {} → {} BORB", borrower, amount);
        Ok(())
    }
    
    /// PRÉLÈVE AUTOMATIQUEMENT LA DETTE SUR LES REVENUS
    pub fn auto_collect_on_income(&self, account_id: &str, income_amount: u128) -> u128 {
        let debt = self.db.get_debt(account_id);
        
        if debt > 0 {
            let collected = income_amount.min(debt);
            let new_debt = debt - collected;
            self.db.set_debt(account_id, new_debt);
            
            log::info!("💰 Prélèvement dette : {} → {} BORB restants", collected, new_debt);
            return income_amount - collected;
        }
        
        income_amount
    }
    
    /// SAISIE DES GARANTIES EN CAS DE DÉFAUT
    pub fn seize_collateral(&self, borrower: &str, debt_amount: u128) -> u128 {
        let nanties = self.db.get_nanties(borrower);
        let seized = nanties.min(debt_amount);
        
        self.db.transfer_to_credit(borrower, seized);
        self.db.set_debt(borrower, debt_amount - seized);
        
        log::info!("⚖️ Garantie saisie : {} → {} BORB", borrower, seized);
        seized
    }
    
    /// Statistiques du crédit
    pub fn get_stats(&self) -> serde_json::Value {
        serde_json::json!({
            "total_credit_pool": self.db.get_balance("case:credit:balance"),
            "active_debts": self.db.get_total_debt(),
            "active_lenders": self.db.get_lender_count()
        })
    }
}
