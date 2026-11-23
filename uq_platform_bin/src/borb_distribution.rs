use rusqlite::Connection;
use serde_json::json;

// ============================================================
// BORB DISTRIBUTION - VERSION DB OWNER
// Les profils 1 et 2 sont dans uq_00000001.db
//   Profil 1 -> table treasury_system
//   Profil 2 -> table dev_profile
// ============================================================

const OWNER_DB: &str = "data/users/uq_00000001.db";

pub struct BorbDistribution {
    pub owner_id: String,
}

impl BorbDistribution {
    pub fn new(owner_id: &str) -> Self {
        Self {
            owner_id: owner_id.to_string(),
        }
    }

    fn conn(&self) -> Result<Connection, String> {
        Connection::open(OWNER_DB).map_err(|e| e.to_string())
    }

    // ============================================================
    // REPARTITION UNIQUE 70/30
    // ============================================================
    pub fn repartir_initial(&self, solde_total_borb: u64) -> Result<(u64, u64), String> {
        let conn = self.conn()?;

        // Verifier si deja reparti
        let p1_exist: i64 = conn
            .query_row("SELECT COUNT(*) FROM treasury_system", [], |row| row.get(0))
            .unwrap_or(0);
        let p2_exist: i64 = conn
            .query_row("SELECT COUNT(*) FROM dev_profile", [], |row| row.get(0))
            .unwrap_or(0);

        if p1_exist > 0 || p2_exist > 0 {
            return Err("Deja reparti".to_string());
        }

        if solde_total_borb == 0 {
            return Err("Solde total nul".to_string());
        }

        let profil1 = solde_total_borb * 70 / 100;
        let profil2 = solde_total_borb * 30 / 100;
        let now = chrono::Utc::now().timestamp();

        // Inserer Profil 1
        conn.execute(
            "INSERT INTO treasury_system (owner_id, balance_borb, balance_usd, percentage, auto_sell_enabled, created_at) VALUES (1, ?1, ?1, 70.0, 1, ?2)",
            rusqlite::params![profil1 as f64, now],
        ).map_err(|e| e.to_string())?;

        // Inserer Profil 2
        conn.execute(
            "INSERT INTO dev_profile (owner_id, balance_borb, balance_usd, percentage, can_manipulate_70, can_buy_from_system, created_at) VALUES (1, ?1, ?1, 30.0, 0, 0, ?2)",
            rusqlite::params![profil2 as f64, now],
        ).map_err(|e| e.to_string())?;

        Ok((profil1, profil2))
    }

    pub fn est_deja_reparti(&self) -> bool {
        let Ok(conn) = self.conn() else {
            return false;
        };
        let p1_exist: i64 = conn
            .query_row("SELECT COUNT(*) FROM treasury_system", [], |row| row.get(0))
            .unwrap_or(0);
        p1_exist > 0
    }

    /// Recuperer les soldes Profil 1 + Profil 2
    fn get_soldes(&self) -> (u64, u64) {
        let Ok(conn) = self.conn() else {
            return (0, 0);
        };
        let p1: f64 = conn
            .query_row(
                "SELECT COALESCE(balance_borb, 0) FROM treasury_system ORDER BY id DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0.0);
        let p2: f64 = conn
            .query_row(
                "SELECT COALESCE(balance_borb, 0) FROM dev_profile ORDER BY id DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0.0);
        (p1 as u64, p2 as u64)
    }

    /// Mettre a jour les soldes
    fn set_soldes(&self, p1: u64, p2: u64) -> Result<(), String> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE treasury_system SET balance_borb = ?1, balance_usd = ?1 WHERE id = (SELECT MAX(id) FROM treasury_system)",
            rusqlite::params![p1 as f64],
        ).map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE dev_profile SET balance_borb = ?1, balance_usd = ?1 WHERE id = (SELECT MAX(id) FROM dev_profile)",
            rusqlite::params![p2 as f64],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    // ============================================================
    // PROFIL 1 : Ventes crypto
    // ============================================================
    pub fn profile1_sell_crypto(
        &self,
        amount_borb: u64,
        crypto_received: bool,
    ) -> Result<u64, String> {
        if !crypto_received {
            return Err("Crypto non recu".to_string());
        }
        let (mut p1, p2) = self.get_soldes();
        if amount_borb > p1 {
            return Err(format!("Stock insuffisant : {}", p1));
        }
        p1 -= amount_borb;
        self.set_soldes(p1, p2)?;
        Ok(p1)
    }

    pub fn profile1_auto_recharge(&self) -> bool {
        let (p1, p2) = self.get_soldes();
        let threshold = 700_000 * 50 / 100;
        if p1 < threshold && p1 > 0 {
            let _ = self.set_soldes(700_000, p2);
            return true;
        }
        false
    }

    // ============================================================
    // PROFIL 2 : Ventes P2P
    // ============================================================
    pub fn profile2_sell_p2p(&self, amount_borb: u64, mm_received: bool) -> Result<u64, String> {
        if !mm_received {
            return Err("Mobile Money non recu".to_string());
        }
        let (p1, mut p2) = self.get_soldes();
        if amount_borb > p2 {
            return Err(format!("Argent de poche insuffisant : {}", p2));
        }
        p2 -= amount_borb;
        self.set_soldes(p1, p2)?;
        Ok(p2)
    }

    pub fn profile2_needs_claim(&self) -> bool {
        let (_, p2) = self.get_soldes();
        p2 < 5_000
    }

    pub fn profile2_claim_from_profile1(&self) -> Result<u64, String> {
        if !self.profile2_needs_claim() {
            return Err("Pas besoin de reclamer".to_string());
        }
        let (mut p1, mut p2) = self.get_soldes();
        let claim_amount = p1 * 30 / 100;
        if claim_amount == 0 {
            return Err("Stock du Profil 1 vide".to_string());
        }
        p1 -= claim_amount;
        p2 += claim_amount;
        self.set_soldes(p1, p2)?;
        Ok(p2)
    }

    pub fn distribute_ubi(&self, total_users: u64, amount_per_user: u64) -> Result<u64, String> {
        Ok(total_users * amount_per_user)
    }

    // ============================================================
    // STATS
    // ============================================================
    pub fn get_stats(&self) -> serde_json::Value {
        let (p1, p2) = self.get_soldes();
        let deja = self.est_deja_reparti();

        json!({
            "profile1_treasury": {
                "stock_borb": p1,
                "seuil_recharge_pct": 50,
                "auto_recharge_active": true,
                "usage": "Vente crypto externe UNIQUEMENT",
                "personne_ne_touche": true
            },
            "profile2_delis": {
                "balance_borb": p2,
                "seuil_reclamation": 5000,
                "claim_pct": 30,
                "usage": "Argent de poche - P2P Mobile Money"
            },
            "ubi_rewards": {
                "total_distribue": 0,
                "pool_quotidien": 0,
                "source": "Case Recompenses - PAS le proprietaire"
            },
            "deja_reparti": deja
        })
    }
}
