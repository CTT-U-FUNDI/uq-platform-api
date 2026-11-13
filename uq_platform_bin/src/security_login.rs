// ====================
// SECURITY LOGIN — Owner vs User + Mode iPhone
// ====================

use chrono::Utc;
use rusqlite::{Connection, params};
use sha2::{Sha256, Digest};

pub const OWNER_UQ: &str = "UQ-00000001";
pub const OWNER_PUBLIC_UQ: &str = "UQ-732337351";
pub const OWNER_DISPLAY: &str = "ELIEZER DESTIN";
pub const OWNER_MAX_ATTEMPTS: i64 = 3;    // Owner : 3 échecs → iPhone
pub const USER_MAX_ATTEMPTS: i64 = 5;     // User  : 5 échecs → iPhone
pub const SESSION_TIMEOUT_SECS: i64 = 300;

pub const DEFAULT_LOGIN_PIN_SALT: &str = "uq_login_salt_UQ-00000001";
/// SHA-256("3543:uq_login_salt_UQ-00000001") — dérive de hash_pin avec le sel par défaut.
pub const FALLBACK_OWNER_PIN_HASH: &str =
    "b48aebda138510a338db788128235bd76b85cf554b3d084e16da90009323ea47";

/// Comparaison en temps constant de deux chaînes (échoue vite si longueurs différentes,
/// mais ne révèle pas la position du premier octet divergent).
pub fn constant_time_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.bytes().zip(b.bytes()) {
        diff |= x ^ y;
    }
    diff == 0
}

pub fn is_owner(uq: &str) -> bool {
    uq == OWNER_UQ
}

pub fn cooldown_seconds(level: u32) -> i64 {
    match level {
        0 => 0,
        1 => 60,
        2 => 300,
        3 => 900,
        4 => 3600,
        5 => 86400,
        _ => 604800,
    }
}

pub fn generate_tfa_token() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let bytes: [u8; 32] = rng.gen();
    hex::encode(bytes)
}

pub fn init_tables(conn: &Connection) -> Result<(), String> {
    conn.execute_batch("
        CREATE TABLE IF NOT EXISTS security_login_sessions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id TEXT UNIQUE NOT NULL,
            ip TEXT DEFAULT '',
            user_agent TEXT DEFAULT '',
            uq_number TEXT DEFAULT '',
            is_owner INTEGER DEFAULT 0,
            current_step INTEGER DEFAULT 1,
            step1_ok INTEGER DEFAULT 0,
            step2_ok INTEGER DEFAULT 0,
            step3_ok INTEGER DEFAULT 0,
            tfa_token TEXT DEFAULT '',
            tfa_generated_at INTEGER DEFAULT 0,
            tfa_verified INTEGER DEFAULT 0,
            status TEXT DEFAULT 'in_progress',
            final_token TEXT DEFAULT '',
            created_at INTEGER DEFAULT 0,
            last_activity INTEGER DEFAULT 0,
            expires_at INTEGER DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS security_login_cooldown_uq (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            uq_number TEXT UNIQUE NOT NULL,
            is_owner INTEGER DEFAULT 0,
            failure_count INTEGER DEFAULT 0,
            cooldown_level INTEGER DEFAULT 0,
            cooldown_until INTEGER DEFAULT 0,
            last_attempt INTEGER DEFAULT 0,
            first_failure_at INTEGER DEFAULT 0,
            attempts_ok INTEGER DEFAULT 0,
            is_locked INTEGER DEFAULT 0,
            locked_until INTEGER DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS security_owner_lock (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            session_id TEXT DEFAULT '',
            ip TEXT DEFAULT '',
            token TEXT DEFAULT '',
            connected_at INTEGER DEFAULT 0,
            last_activity INTEGER DEFAULT 0
        );
        INSERT OR IGNORE INTO security_owner_lock (id, connected_at) VALUES (1, 0);
    ").map_err(|e| e.to_string())?;
    
    // ===== MIGRATION : Ajouter les colonnes manquantes (si DB existante) =====
    // SQLite ne supporte pas ADD COLUMN IF NOT EXISTS, donc on essaie et on ignore les erreurs
    let _ = conn.execute("ALTER TABLE security_login_cooldown_uq ADD COLUMN attempts_ok INTEGER DEFAULT 0", []);
    let _ = conn.execute("ALTER TABLE security_login_cooldown_uq ADD COLUMN is_locked INTEGER DEFAULT 0", []);
    let _ = conn.execute("ALTER TABLE security_login_cooldown_uq ADD COLUMN locked_until INTEGER DEFAULT 0", []);
    
    Ok(())
}

pub fn is_iphone_mode_active(conn: &Connection, uq: &str) -> Option<(i64, i64, i64)> {
    let result: Result<(i64, i64, i64), _> = conn.query_row(
        "SELECT cooldown_until, cooldown_level, failure_count FROM security_login_cooldown_uq WHERE uq_number = ?1",
        params![uq],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    );
    if let Ok((until, level, count)) = result {
        let now = Utc::now().timestamp();
        if until > now {
            return Some((until, level, count));
        }
    }
    None
}

pub fn record_failure_uq(conn: &Connection, uq: &str) -> (i64, i64, i64, i64) {
    // Retourne (cooldown_until, level, new_count, attempts_until_iphone)
    let now = Utc::now().timestamp();
    let owner = is_owner(uq);
    let max_attempts = if owner { OWNER_MAX_ATTEMPTS } else { USER_MAX_ATTEMPTS };

    let result: Result<(i64, i64, i64), _> = conn.query_row(
        "SELECT failure_count, cooldown_level, first_failure_at FROM security_login_cooldown_uq WHERE uq_number = ?1",
        params![uq],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    );

    let (current_count, _level, first_failure) = result.unwrap_or((0, 0, now));
    let first = if first_failure == 0 || now - first_failure > 86400 { now } else { first_failure };
    let new_count = current_count + 1;

    let level = if new_count < max_attempts {
        0
    } else {
        let over = new_count - max_attempts;
        let step = if owner { 3 } else { 5 };
        ((over / step) + 1).min(5) as i64
    };

    let cooldown_until = if level > 0 {
        now + cooldown_seconds(level as u32)
    } else {
        0
    };

    let attempts_until_iphone = if level == 0 { max_attempts - new_count } else { 0 };

    let _ = conn.execute(
        "INSERT INTO security_login_cooldown_uq (uq_number, is_owner, failure_count, cooldown_level, cooldown_until, last_attempt, first_failure_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(uq_number) DO UPDATE SET
            failure_count = ?3,
            cooldown_level = ?4,
            cooldown_until = ?5,
            last_attempt = ?6,
            first_failure_at = ?7",
        params![uq, if owner { 1 } else { 0 }, new_count, level, cooldown_until, now, first]
    );

    eprintln!("⚠️ Échec #{} pour {} (niveau {}, jusqu'à iPhone: {})", new_count, uq, level, attempts_until_iphone);

    (cooldown_until, level, new_count, attempts_until_iphone)
}

pub fn reset_cooldown_uq(conn: &Connection, uq: &str) {
    let _ = conn.execute("DELETE FROM security_login_cooldown_uq WHERE uq_number = ?1", params![uq]);
}

pub fn create_session(conn: &Connection, ip: &str, ua: &str, uq: &str) -> Result<(String, String), String> {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let bytes: [u8; 16] = rng.gen();
    let session_id = hex::encode(bytes);
    let tfa_token = generate_tfa_token();
    let now = Utc::now().timestamp();
    let expires = now + SESSION_TIMEOUT_SECS;
    let owner = is_owner(uq);

    conn.execute(
        "INSERT INTO security_login_sessions (session_id, ip, user_agent, uq_number, is_owner, current_step, tfa_token, tfa_generated_at, created_at, last_activity, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?7, ?7, ?7, ?8)",
        params![session_id, ip, ua, uq, if owner { 1 } else { 0 }, tfa_token, now, expires]
    ).map_err(|e| e.to_string())?;

    Ok((session_id, tfa_token))
}

pub fn advance_session(conn: &Connection, session_id: &str, expected_step: i64) -> Result<(i64, String), String> {
    let now = Utc::now().timestamp();
    let result: Result<(i64, i64, i64), _> = conn.query_row(
        "SELECT current_step, expires_at, is_owner FROM security_login_sessions WHERE session_id = ?1 AND status = 'in_progress'",
        params![session_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    );
    let (current, expires, _owner) = result.map_err(|_| "Session introuvable".to_string())?;
    if expires < now { return Err("Session expirée".to_string()); }
    if current != expected_step { return Err("Étape invalide".to_string()); }
    let new_tfa = generate_tfa_token();
    let next_step = current + 1;
    let update_sql = match current {
        1 => "UPDATE security_login_sessions SET current_step = 2, step1_ok = 1, tfa_token = ?1, tfa_generated_at = ?2, last_activity = ?2 WHERE session_id = ?3",
        2 => "UPDATE security_login_sessions SET current_step = 3, step2_ok = 1, tfa_token = ?1, tfa_generated_at = ?2, last_activity = ?2 WHERE session_id = ?3",
        _ => return Err("Étape invalide".to_string()),
    };
    conn.execute(update_sql, params![new_tfa, now, session_id]).map_err(|e| e.to_string())?;
    Ok((next_step, new_tfa))
}

pub fn finalize_session(conn: &Connection, session_id: &str, ip: &str, token: &str) -> Result<(), String> {
    let now = Utc::now().timestamp();
    conn.execute(
        "UPDATE security_login_sessions SET status = 'connected', step3_ok = 1, final_token = ?1, last_activity = ?2 WHERE session_id = ?3",
        params![token, now, session_id]
    ).map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE security_owner_lock SET session_id = ?1, ip = ?2, token = ?3, connected_at = ?4, last_activity = ?4 WHERE id = 1",
        params![session_id, ip, token, now]
    ).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn is_owner_connected_elsewhere(conn: &Connection, current_ip: &str) -> Option<(String, String)> {
    let result: Result<(String, String, i64), _> = conn.query_row(
        "SELECT session_id, ip, last_activity FROM security_owner_lock WHERE id = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    );
    if let Ok((session_id, ip, last_activity)) = result {
        if !session_id.is_empty() && ip != current_ip {
            let now = Utc::now().timestamp();
            if now - last_activity < SESSION_TIMEOUT_SECS {
                return Some((session_id, ip));
            }
        }
    }
    None
}

pub fn disconnect_old_session(conn: &Connection) -> Result<(), String> {
    conn.execute(
        "UPDATE security_owner_lock SET session_id = '', ip = '', token = '', last_activity = 0 WHERE id = 1",
        []
    ).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn hash_pin(pin: &str, salt: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(format!("{}:{}", pin, salt).as_bytes());
    format!("{:x}", hasher.finalize())
}

// ====================
// VÉRIFIER SI UN COMPTE EXISTE (user normal)
// ====================
pub fn check_user_exists(uq: &str, email: &str, password: &str) -> bool {
    let idx = crate::uq_index::UqIndex::shared("uq_index.db");
    idx.verify_credentials(uq, email, password).is_some()
}

// ====================
// VÉRIFIER LE PIN DE CONNEXION (owner uniquement)
// ====================
pub fn verify_login_pin(conn: &Connection, pin: &str) -> bool {
    if pin.is_empty() {
        return false;
    }

    let salt: String = conn.query_row(
        "SELECT value FROM settings WHERE key = 'login_pin_salt'",
        [], |row| row.get(0)
    ).unwrap_or_else(|_| DEFAULT_LOGIN_PIN_SALT.to_string());

    let stored_hash: String = conn.query_row(
        "SELECT value FROM settings WHERE key = 'login_pin_hash'",
        [], |row| row.get(0)
    ).unwrap_or_default();

    if stored_hash.is_empty() {
        // Aucune amorce en DB : PIN owner de secours, comparé sous forme de hash.
        return constant_time_eq(&hash_pin(pin, DEFAULT_LOGIN_PIN_SALT), FALLBACK_OWNER_PIN_HASH);
    }

    constant_time_eq(&hash_pin(pin, &salt), &stored_hash)
}

// ====================
// COMPTEUR DE TENTATIVES PERSISTANT (par UQ, survit aux sessions)
// ====================

/// Lit le compteur persistant pour un UQ
pub fn get_persistent_attempts(conn: &Connection, uq: &str) -> i64 {
    let result: Result<i64, _> = conn.query_row(
        "SELECT attempts_ok FROM security_login_cooldown_uq WHERE uq_number = ?1",
        params![uq],
        |row| row.get(0),
    );
    result.unwrap_or(0)
}

/// Incrémente le compteur persistant et retourne la nouvelle valeur
pub fn increment_persistent_attempts(conn: &Connection, uq: &str) -> i64 {
    let current = get_persistent_attempts(conn, uq);
    let new_value = current + 1;
    let now = Utc::now().timestamp();
    let owner = is_owner(uq);
    
    // Upsert
    let _ = conn.execute(
        "INSERT INTO security_login_cooldown_uq (uq_number, is_owner, attempts_ok, last_attempt)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(uq_number) DO UPDATE SET
            attempts_ok = ?3,
            last_attempt = ?4",
        params![uq, if owner { 1 } else { 0 }, new_value, now],
    );
    
    new_value
}

/// Reset le compteur persistant (après connexion réussie)
pub fn reset_persistent_attempts(conn: &Connection, uq: &str) {
    let _ = conn.execute(
        "UPDATE security_login_cooldown_uq SET attempts_ok = 0 WHERE uq_number = ?1",
        params![uq],
    );
}

// ====================
// NETTOYAGE AUTOMATIQUE — Appelé au démarrage du backend
// ====================
pub fn cleanup_stale_sessions(conn: &Connection) -> Result<usize, String> {
    let now = Utc::now().timestamp();
    
    // 1. Abandonner les sessions expirées
    let count1 = conn.execute(
        "UPDATE security_login_sessions SET status = 'abandoned' WHERE status = 'in_progress' AND expires_at < ?1",
        params![now]
    ).unwrap_or(0);
    
    // 2. Abandonner les sessions inactives (> 5 min)
    let count2 = conn.execute(
        "UPDATE security_login_sessions SET status = 'abandoned' WHERE status = 'in_progress' AND (?1 - last_activity) > 300",
        params![now]
    ).unwrap_or(0);
    
    // 3. Supprimer les cooldowns expirés
    let count3 = conn.execute(
        "DELETE FROM security_login_cooldown_uq WHERE cooldown_until < ?1 AND cooldown_until > 0",
        params![now]
    ).unwrap_or(0);
    
    // 4. Libérer l'owner lock si inactif > 5 min
    let count4 = conn.execute(
        "UPDATE security_owner_lock SET session_id = '', ip = '', token = '', last_activity = 0 WHERE id = 1 AND (?1 - last_activity) > 300",
        params![now]
    ).unwrap_or(0);
    
    let total = count1 + count2 + count3 + count4;
    if total > 0 {
        eprintln!("🧹 Nettoyage: {} session(s)/cooldown(s) supprimé(s)", total);
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn_with_settings(rows: &[(&str, &str)]) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT)", []).unwrap();
        for (k, v) in rows {
            conn.execute("INSERT INTO settings VALUES (?1, ?2)", params![k, v]).unwrap();
        }
        conn
    }

    #[test]
    fn hash_pin_correspond_au_format_attendu() {
        assert_eq!(
            hash_pin("3543", DEFAULT_LOGIN_PIN_SALT),
            FALLBACK_OWNER_PIN_HASH,
            "le hash en dur doit correspondre à SHA256(\"3543:sel_defaut\")"
        );
    }

    #[test]
    fn sans_amorce_le_pin_de_secours_passe_en_hash() {
        let conn = conn_with_settings(&[]);
        assert!(verify_login_pin(&conn, "3543"));
        assert!(!verify_login_pin(&conn, "0000"));
        assert!(!verify_login_pin(&conn, ""));
    }

    #[test]
    fn sans_amorce_un_sel_custom_ne_delivre_pas_le_pin_vide() {
        // fail-closed : même avec login_pin_hash absent, le PIN vide est rejeté
        let conn = conn_with_settings(&[("login_pin_salt", "sel_perso")]);
        assert!(!verify_login_pin(&conn, ""));
    }

    #[test]
    fn avec_amorce_le_hash_en_db_fait_foi() {
        let conn = conn_with_settings(&[
            ("login_pin_salt", "sel_perso"),
            ("login_pin_hash", hash_pin("1234", "sel_perso").as_str()),
        ]);
        assert!(verify_login_pin(&conn, "1234"));
        assert!(!verify_login_pin(&conn, "3543"));
    }

    #[test]
    fn constant_time_eq_valide() {
        assert!(constant_time_eq("abc123", "abc123"));
        assert!(!constant_time_eq("abc123", "abc124"));
        assert!(!constant_time_eq("abc", "abcd"));
    }
}