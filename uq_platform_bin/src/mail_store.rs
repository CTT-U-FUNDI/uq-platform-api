//! UQ Mail — boîtes mail par compte UQ.
//! Une seule SQLite (data/mail/mailbox.db), table mail_messages indexée
//! par uq_number. Dossiers : inbox, sent, drafts, spam.
//! Le serveur mail (Postfix) dépose via /api/mail/in ; l'app lit via
//! /api/mail/*. L'adresse réelle = localpart(uq_email)@UQ_MAIL_DOMAIN.

use rusqlite::{params, Connection};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, Serialize)]
pub struct MailMessage {
    pub id: i64,
    pub folder: String,
    pub from_addr: String,
    pub from_name: String,
    pub to_addr: String,
    pub subject: String,
    pub body_text: String,
    pub date: i64,
    pub is_read: bool,
    pub otp_code: Option<String>,
}

pub struct MailStore {
    conn: Mutex<Connection>,
}

impl MailStore {
    pub fn new(db_path: impl AsRef<std::path::Path>) -> Result<Self, String> {
        if let Some(p) = db_path.as_ref().parent() {
            let _ = std::fs::create_dir_all(p);
        }
        let conn = Connection::open(db_path).map_err(|e| e.to_string())?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS mail_messages (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               uq_number TEXT NOT NULL,
               folder TEXT NOT NULL DEFAULT 'inbox',
               from_addr TEXT NOT NULL DEFAULT '',
               from_name TEXT NOT NULL DEFAULT '',
               to_addr TEXT NOT NULL DEFAULT '',
               subject TEXT NOT NULL DEFAULT '',
               body_text TEXT NOT NULL DEFAULT '',
               body_html TEXT NOT NULL DEFAULT '',
               date INTEGER NOT NULL,
               is_read INTEGER NOT NULL DEFAULT 0,
               otp_code TEXT,
               in_reply_to INTEGER
             );
             CREATE INDEX IF NOT EXISTS idx_mail_uq ON mail_messages(uq_number, folder, date DESC);
             CREATE INDEX IF NOT EXISTS idx_mail_to ON mail_messages(to_addr);",
        )
        .map_err(|e| e.to_string())?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    /// Dépose un message entrant (depuis Postfix via /api/mail/in).
    #[allow(clippy::too_many_arguments)]
    pub fn deposer(
        &self,
        uq: &str,
        from_addr: &str,
        from_name: &str,
        to_addr: &str,
        subject: &str,
        body_text: &str,
        body_html: &str,
        otp_code: Option<&str>,
        folder: &str,
    ) -> Result<i64, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT INTO mail_messages
             (uq_number, folder, from_addr, from_name, to_addr, subject, body_text, body_html, date, is_read, otp_code)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,0,?10)",
            params![uq, folder, from_addr, from_name, to_addr, subject, body_text, body_html, now, otp_code],
        )
        .map_err(|e| e.to_string())?;
        Ok(conn.last_insert_rowid())
    }

    /// Enregistre un message envoyé (dossier 'sent').
    #[allow(clippy::too_many_arguments)]
    pub fn enregistrer_envoi(
        &self,
        uq: &str,
        from_addr: &str,
        to_addr: &str,
        subject: &str,
        body_text: &str,
        body_html: &str,
    ) -> Result<i64, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT INTO mail_messages
             (uq_number, folder, from_addr, to_addr, subject, body_text, body_html, date, is_read)
             VALUES (?1,'sent',?2,?3,?4,?5,?6,?7,1)",
            params![uq, from_addr, to_addr, subject, body_text, body_html, now],
        )
        .map_err(|e| e.to_string())?;
        Ok(conn.last_insert_rowid())
    }

    /// Liste les messages d'un dossier (récent d'abord).
    pub fn lister(&self, uq: &str, folder: &str, limite: i64) -> Vec<MailMessage> {
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        let mut stmt = match conn.prepare(
            "SELECT id, folder, from_addr, from_name, to_addr, subject, body_text, date, is_read, otp_code
             FROM mail_messages WHERE uq_number=?1 AND folder=?2
             ORDER BY date DESC LIMIT ?3",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        stmt.query_map(params![uq, folder, limite], |r| {
            Ok(MailMessage {
                id: r.get(0)?,
                folder: r.get(1)?,
                from_addr: r.get(2)?,
                from_name: r.get(3)?,
                to_addr: r.get(4)?,
                subject: r.get(5)?,
                body_text: r.get(6)?,
                date: r.get(7)?,
                is_read: r.get::<_, i64>(8)? == 1,
                otp_code: r.get(9)?,
            })
        })
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
    }

    /// Un message précis (pour lecture).
    pub fn lire(&self, uq: &str, id: i64) -> Option<MailMessage> {
        let conn = self.conn.lock().ok()?;
        conn.query_row(
            "SELECT id, folder, from_addr, from_name, to_addr, subject, body_text, date, is_read, otp_code
             FROM mail_messages WHERE id=?1 AND uq_number=?2",
            params![id, uq],
            |r| {
                Ok(MailMessage {
                    id: r.get(0)?,
                    folder: r.get(1)?,
                    from_addr: r.get(2)?,
                    from_name: r.get(3)?,
                    to_addr: r.get(4)?,
                    subject: r.get(5)?,
                    body_text: r.get(6)?,
                    date: r.get(7)?,
                    is_read: r.get::<_, i64>(8)? == 1,
                    otp_code: r.get(9)?,
                })
            },
        )
        .ok()
    }

    /// Marque comme lu.
    pub fn marquer_lu(&self, uq: &str, id: i64) -> bool {
        let Ok(conn) = self.conn.lock() else { return false };
        conn.execute(
            "UPDATE mail_messages SET is_read=1 WHERE id=?1 AND uq_number=?2",
            params![id, uq],
        )
        .map(|n| n > 0)
        .unwrap_or(false)
    }

    /// Nombre de non-lus (badge dans le chat).
    pub fn non_lus(&self, uq: &str) -> i64 {
        let Ok(conn) = self.conn.lock() else { return 0 };
        conn.query_row(
            "SELECT COUNT(*) FROM mail_messages WHERE uq_number=?1 AND folder='inbox' AND is_read=0",
            params![uq],
            |r| r.get(0),
        )
        .unwrap_or(0)
    }

    /// Trouve le uq_number propriétaire d'une adresse réelle
    /// (prenom@domaine → uq). Utilisé par /api/mail/in.
    pub fn uq_par_adresse(&self, _adresse: &str) -> Option<String> {
        // La résolution se fait via uq_index (normalized_email), pas ici.
        // Cette méthode existe pour une éventuelle table d'alias mail.
        None
    }
}

static MAIL_STORE: OnceLock<MailStore> = OnceLock::new();

/// Instance globale du store mail.
pub fn shared() -> &'static MailStore {
    MAIL_STORE.get_or_init(|| {
        MailStore::new("data/mail/mailbox.db").unwrap_or_else(|e| {
            eprintln!("❌ [mail] Impossible d'ouvrir mailbox.db : {}", e);
            // Fallback en mémoire impossible — on panique au premier usage
            panic!("mailbox.db requis: {}", e);
        })
    })
}

/// Extrait un code OTP probable d'un corps de mail (4-8 chiffres).
/// Cherche les patterns typiques : "code", "OTP", "vérification", "123456".
pub fn detecter_otp(sujet: &str, corps: &str) -> Option<String> {
    let texte = format!("{}\n{}", sujet, corps);
    // Patterns : "code: 123456", "OTP 123456", "123456 est votre code"
    let keywords = ["code", "otp", "vérification", "verification", "mot de passe", "password", "pin"];
    let bas = texte.to_lowercase();
    let a_un_mot_cle = keywords.iter().any(|k| bas.contains(k));

    // Cherche des séquences de 4-8 chiffres
    let mut candidats: Vec<String> = Vec::new();
    let bytes = texte.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() { i += 1; }
            let len = i - start;
            if (4..=8).contains(&len) {
                let s = &texte[start..i];
                // Évite les années seules (1900-2099) et les longs IDs
                if !(len == 4 && s.starts_with("19") || s.starts_with("20")) {
                    candidats.push(s.to_string());
                }
            }
        } else { i += 1; }
    }

    if candidats.is_empty() { return None; }
    // Si un mot-clé OTP est présent, plus confiant
    if a_un_mot_cle {
        return candidats.first().cloned();
    }
    // Sinon seulement si un seul candidat (peu de faux positifs)
    if candidats.len() == 1 { return candidats.pop(); }
    None
}
