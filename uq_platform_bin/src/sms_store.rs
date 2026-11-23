//! UQ SMS — relais de numéros et réception OTP.
//! Chaque compte UQ peut avoir UN numéro relais (vrai numéro du pool SIM).
//! Les SMS entrants (forwardés par les vieux Android) sont matchés sur
//! relay_number → uq_number, l'OTP est extrait et notifié dans le chat.

use rusqlite::{params, Connection};
use serde::Serialize;
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, Serialize)]
pub struct SmsRecu {
    pub id: i64,
    pub relay_number: String,
    pub from_addr: String,
    pub text: String,
    pub otp_code: Option<String>,
    pub date: i64,
    pub is_read: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct RelayNumber {
    pub id: i64,
    pub relay_number: String,
    pub uq_number: Option<String>,
    pub status: String, // libre | actif | maintenance | bloque
    pub sim_id: String,
    pub last_activity: i64,
    pub created_at: i64,
}

pub struct SmsStore {
    conn: Mutex<Connection>,
}

impl SmsStore {
    pub fn new(db_path: impl AsRef<std::path::Path>) -> Result<Self, String> {
        if let Some(p) = db_path.as_ref().parent() {
            let _ = std::fs::create_dir_all(p);
        }
        let conn = Connection::open(db_path).map_err(|e| e.to_string())?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS uq_relay_numbers (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               relay_number TEXT UNIQUE NOT NULL,
               uq_number TEXT,
               status TEXT NOT NULL DEFAULT 'libre',
               sim_id TEXT NOT NULL DEFAULT '',
               last_activity INTEGER NOT NULL DEFAULT 0,
               created_at INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_relay_uq ON uq_relay_numbers(uq_number);
             CREATE TABLE IF NOT EXISTS sms_recus (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               uq_number TEXT NOT NULL,
               relay_number TEXT NOT NULL,
               from_addr TEXT NOT NULL DEFAULT '',
               text TEXT NOT NULL DEFAULT '',
               otp_code TEXT,
               date INTEGER NOT NULL,
               is_read INTEGER NOT NULL DEFAULT 0
             );
             CREATE INDEX IF NOT EXISTS idx_sms_uq ON sms_recus(uq_number, date DESC);
             CREATE INDEX IF NOT EXISTS idx_sms_relay ON sms_recus(relay_number);",
        )
        .map_err(|e| e.to_string())?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    // ── Pool de numéros relais ──

    /// Enregistre un numéro relais dans le pool (sim_id = identifiant du modem/téléphone).
    pub fn ajouter_relais(&self, relay_number: &str, sim_id: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT OR IGNORE INTO uq_relay_numbers (relay_number, sim_id, status, last_activity, created_at)
             VALUES (?1,?2,'libre',?3,?3)",
            params![relay_number, sim_id, now],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Liste les numéros libres du pool.
    pub fn numeros_libres(&self) -> Vec<RelayNumber> {
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        let mut stmt = match conn.prepare(
            "SELECT id, relay_number, uq_number, status, sim_id, last_activity, created_at
             FROM uq_relay_numbers WHERE status='libre' ORDER BY id LIMIT 50",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        stmt.query_map([], |r| {
            Ok(RelayNumber {
                id: r.get(0)?,
                relay_number: r.get(1)?,
                uq_number: r.get(2)?,
                status: r.get(3)?,
                sim_id: r.get(4)?,
                last_activity: r.get(5)?,
                created_at: r.get(6)?,
            })
        })
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
    }

    /// Attribue un numéro libre à un compte. Retourne le numéro ou None si pool vide.
    pub fn attribuer(&self, uq: &str) -> Option<String> {
        let conn = self.conn.lock().ok()?;
        let now = chrono::Utc::now().timestamp();
        // Un seul relais actif par compte : libérer l'ancien d'abord
        let _ = conn.execute(
            "UPDATE uq_relay_numbers SET status='libre', uq_number=NULL, last_activity=?1
             WHERE uq_number=?2 AND status='actif'",
            params![now, uq],
        );
        let numero = conn.query_row(
            "SELECT relay_number FROM uq_relay_numbers WHERE status='libre' ORDER BY id LIMIT 1",
            [], |r| r.get::<_, String>(0),
        ).ok()?;
        conn.execute(
            "UPDATE uq_relay_numbers SET uq_number=?1, status='actif', last_activity=?2
             WHERE relay_number=?3",
            params![uq, now, numero],
        )
        .ok()?;
        Some(numero)
    }

    /// Numéro relais actif d'un compte.
    pub fn numero_de(&self, uq: &str) -> Option<String> {
        let conn = self.conn.lock().ok()?;
        conn.query_row(
            "SELECT relay_number FROM uq_relay_numbers WHERE uq_number=?1 AND status='actif'",
            params![uq],
            |r| r.get::<_, String>(0),
        )
        .ok()
    }

    /// Libère le numéro d'un compte (retour au pool).
    pub fn liberer(&self, uq: &str) -> bool {
        let Ok(conn) = self.conn.lock() else {
            return false;
        };
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "UPDATE uq_relay_numbers SET status='libre', uq_number=NULL, last_activity=?1
             WHERE uq_number=?2 AND status='actif'",
            params![now, uq],
        )
        .map(|n| n > 0)
        .unwrap_or(false)
    }

    /// Statut d'un numéro relais (pour détecter réattribution opérateur).
    pub fn statut_numero(&self, relay_number: &str) -> Option<String> {
        let conn = self.conn.lock().ok()?;
        conn.query_row(
            "SELECT status FROM uq_relay_numbers WHERE relay_number=?1",
            params![relay_number],
            |r| r.get::<_, String>(0),
        )
        .ok()
    }

    /// Marque un numéro en maintenance (SIM en keep-alive, panne modem).
    pub fn maintenance(&self, relay_number: &str, actif: bool) -> bool {
        let Ok(conn) = self.conn.lock() else {
            return false;
        };
        conn.execute(
            "UPDATE uq_relay_numbers SET status=?2 WHERE relay_number=?1",
            params![relay_number, if actif { "maintenance" } else { "libre" }],
        )
        .map(|n| n > 0)
        .unwrap_or(false)
    }

    // ── SMS reçus ──

    /// Dépose un SMS reçu (forwardé par le téléphone). Matche relay → uq.
    pub fn deposer_sms(
        &self,
        relay_number: &str,
        from_addr: &str,
        text: &str,
        otp: Option<&str>,
    ) -> Option<i64> {
        let conn = self.conn.lock().ok()?;
        // Retrouve le propriétaire du numéro relais
        let uq: String = conn
            .query_row(
                "SELECT uq_number FROM uq_relay_numbers WHERE relay_number=?1 AND status='actif'",
                params![relay_number],
                |r| r.get(0),
            )
            .ok()?;
        let now = chrono::Utc::now().timestamp();
        // Activity keep-alive
        let _ = conn.execute(
            "UPDATE uq_relay_numbers SET last_activity=?1 WHERE relay_number=?2",
            params![now, relay_number],
        );
        conn.execute(
            "INSERT INTO sms_recus (uq_number, relay_number, from_addr, text, otp_code, date, is_read)
             VALUES (?1,?2,?3,?4,?5,?6,0)",
            params![uq, relay_number, from_addr, text, otp, now],
        ).ok()?;
        Some(conn.last_insert_rowid())
    }

    /// Liste les SMS reçus d'un compte (récent d'abord).
    pub fn sms_reçus(&self, uq: &str, limite: i64) -> Vec<SmsRecu> {
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        let mut stmt = match conn.prepare(
            "SELECT id, relay_number, from_addr, text, otp_code, date, is_read
             FROM sms_recus WHERE uq_number=?1 ORDER BY date DESC LIMIT ?2",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        stmt.query_map(params![uq, limite], |r| {
            Ok(SmsRecu {
                id: r.get(0)?,
                relay_number: r.get(1)?,
                from_addr: r.get(2)?,
                text: r.get(3)?,
                otp_code: r.get(4)?,
                date: r.get(5)?,
                is_read: r.get::<_, i64>(6)? == 1,
            })
        })
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
    }

    pub fn marquer_sms_lu(&self, uq: &str, id: i64) -> bool {
        let Ok(conn) = self.conn.lock() else {
            return false;
        };
        conn.execute(
            "UPDATE sms_recus SET is_read=1 WHERE id=?1 AND uq_number=?2",
            params![id, uq],
        )
        .map(|n| n > 0)
        .unwrap_or(false)
    }

    /// SMS non lus (badge OTP dans le chat).
    pub fn sms_non_lus(&self, uq: &str) -> i64 {
        let Ok(conn) = self.conn.lock() else { return 0 };
        conn.query_row(
            "SELECT COUNT(*) FROM sms_recus WHERE uq_number=?1 AND is_read=0 AND otp_code IS NOT NULL",
            params![uq], |r| r.get(0),
        ).unwrap_or(0)
    }

    /// Numéros dont la SIM n'a pas eu d'activité depuis `secondes` (keep-alive).
    pub fn numéros_sans_activité(&self, secondes: i64) -> Vec<String> {
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        let seuil = chrono::Utc::now().timestamp() - secondes;
        let mut stmt = match conn.prepare(
            "SELECT relay_number FROM uq_relay_numbers
             WHERE status='actif' AND last_activity < ?1",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        stmt.query_map(params![seuil], |r| r.get(0))
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default()
    }
}

static SMS_STORE: OnceLock<SmsStore> = OnceLock::new();

pub fn shared() -> &'static SmsStore {
    SMS_STORE.get_or_init(|| {
        SmsStore::new("data/sms/sms.db").unwrap_or_else(|e| {
            eprintln!("❌ [sms] Impossible d'ouvrir sms.db : {}", e);
            panic!("sms.db requis: {}", e);
        })
    })
}
