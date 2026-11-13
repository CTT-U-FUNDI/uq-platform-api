use crate::database::Database;


// ====================
// RECYCLAGE DES COMPTES INACTIFS — SYSTÈME HYBRIDE
// ====================

pub struct RecyclingSystem {
    db: Database,
    last_global_check: u64,
}

impl RecyclingSystem {
    /// Crée le système de recyclage
    pub fn new(db: Database) -> Self {
        Self {
            db,
            last_global_check: 0,
        }
    }
    
    /// VÉRIFICATION À CHAQUE TRANSACTION (séquentiel)
    pub fn on_transaction(&mut self, account_id: &str) {
        let now = chrono::Utc::now().timestamp() as u64;
        let last_activity = self.db.get_last_activity(account_id);
        let years_inactive = (now - last_activity) / (365 * 24 * 3600);
        
        if years_inactive > INACTIVITY_GRACE_YEARS {
            let recycled = self.db.recycle_inactive(account_id, years_inactive).unwrap_or(0);
            if recycled > 0 {
                log::info!("♻️ Recyclage : {} → {} au Trésor", account_id, recycled);
            }
        }
        
        self.db.update_last_activity(account_id, now);
    }
    
    /// VÉRIFICATION MENSUELLE (lazy scan global)
    pub fn on_monthly_check(&mut self) {
        let now = chrono::Utc::now().timestamp() as u64;
        
        if now - self.last_global_check > 30 * 24 * 3600 {
            // Scan paresseux : seulement les comptes inactifs > 1 an
            let inactive_accounts = self.db.get_inactive_accounts(1);
            
            for account_id in inactive_accounts {
                let last_activity = self.db.get_last_activity(&account_id);
                let years_inactive = (now - last_activity) / (365 * 24 * 3600);
                let recycled = self.db.recycle_inactive(&account_id, years_inactive).unwrap_or(0);
                
                if recycled > 0 {
                    log::info!("♻️ Recyclage mensuel : {} → {} au Trésor", account_id, recycled);
                }
            }
            
            self.last_global_check = now;
        }
    }
    
    /// Statistiques du recyclage
    pub fn get_stats(&self) -> serde_json::Value {
        serde_json::json!({
            "grace_years": INACTIVITY_GRACE_YEARS,
            "fee_rate_pct": INACTIVITY_FEE_RATE,
            "recycle_50_years": RECYCLE_50_YEARS,
            "recycle_100_years": RECYCLE_100_YEARS,
            "last_global_check": self.last_global_check
        })
    }
}

