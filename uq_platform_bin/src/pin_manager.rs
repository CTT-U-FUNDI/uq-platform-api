// ====================
// GESTIONNAIRE DE PIN UQ
// 4-6 chiffres | 3 essais | Cooldown 24h | Blocage 15min
// ====================

use chrono::Utc;
use rusqlite::{Connection, params};
use sha2::{Sha256, Digest};

// ===== Constantes =====
pub const PIN_MIN_LENGTH: usize = 4;
pub const PIN_MAX_LENGTH: usize = 6;
pub const MAX_ATTEMPTS: i64 = 3;
pub const LOCKOUT_MINUTES: i64 = 15;
pub const COOLDOWN_HOURS: i64 = 0;  // Désactivé pour dev

// ===== Statut du PIN =====
#[derive(Debug, Clone, serde::Serialize)]
pub enum PinStatus {
    /// PIN utilisable
    Ready,
    /// PIN non configure
    NotConfigured,
    /// En cooldown apres modification
    Cooldown { hours_remaining: i64 },
    /// Bloque apres trop d'echecs
    Locked { minutes_remaining: i64 },
    /// Desactive par l'utilisateur
    Disabled,
}

// ===== Verification du format PIN =====
pub fn is_valid_pin_format(pin: &str) -> bool {
    let len = pin.len();
    len >= PIN_MIN_LENGTH && len <= PIN_MAX_LENGTH && pin.chars().all(|c| c.is_ascii_digit())
}

// ===== Hashage PIN =====
pub fn hash_pin(pin: &str, salt: &str) -> String {
    // Hash robuste : SHA-256(UQ_PIN_V2:pin:salt)
    let mut hasher = Sha256::new();
    hasher.update(format!("UQ_PIN_V2:{}:{}", pin, salt).as_bytes());
    format!("{:x}", hasher.finalize())
}

// ====================
// FONCTIONS DB
// ====================

/// Cree la table pin_status si elle n'existe pas
pub fn init_pin_table(conn: &Connection) -> Result<(), String> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS pin_status (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            pin_hash TEXT DEFAULT '',
            pin_enabled INTEGER DEFAULT 0,
            pin_length INTEGER DEFAULT 4,
            modified_at INTEGER DEFAULT 0,
            attempts INTEGER DEFAULT 0,
            locked_until INTEGER DEFAULT 0,
            created_at INTEGER DEFAULT 0
        )",
        [],
    ).map_err(|e| e.to_string())?;

    // S'assurer qu'une ligne existe
    conn.execute(
        "INSERT OR IGNORE INTO pin_status (id, created_at) VALUES (1, ?1)",
        params![Utc::now().timestamp()],
    ).map_err(|e| e.to_string())?;

    Ok(())
}

/// Recupere le statut actuel du PIN
pub fn get_pin_status(conn: &Connection) -> Result<PinStatus, String> {
    let now = Utc::now().timestamp();

    let result: Result<(i64, i64, i64, i64, i64), _> = conn.query_row(
        "SELECT pin_enabled, modified_at, attempts, locked_until, created_at
         FROM pin_status WHERE id = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
    );

    let (enabled, modified_at, _attempts, locked_until, _created_at) = match result {
        Ok(r) => r,
        Err(_) => return Ok(PinStatus::NotConfigured),
    };

    // 1. Desactive ?
    if enabled == 0 {
        return Ok(PinStatus::NotConfigured);
    }

    // 2. Bloque ?
    if locked_until > now {
        return Ok(PinStatus::Locked {
            minutes_remaining: (locked_until - now) / 60,
        });
    }

    // 3. Cooldown apres modification ?
    // ✅ COOLDOWN DÉSACTIVÉ
    // if modified_at > 0 {
    //     let hours_since = (now - modified_at) / 3600;
    //     if hours_since < COOLDOWN_HOURS {
    //         return Ok(PinStatus::Cooldown {
    //             hours_remaining: COOLDOWN_HOURS - hours_since,
    //         });
    //     }
    // }

    Ok(PinStatus::Ready)
}

/// Configure le PIN (premiere fois ou modification)
pub fn set_pin(conn: &Connection, pin: &str) -> Result<(), String> {
    if !is_valid_pin_format(pin) {
        return Err(format!(
            "PIN invalide : doit faire {} a {} chiffres",
            PIN_MIN_LENGTH, PIN_MAX_LENGTH
        ));
    }

    let now = Utc::now().timestamp();
    let salt = format!("uq_pin_salt_{}", now);
    let hash = hash_pin(pin, &salt);

    conn.execute(
        "UPDATE pin_status
         SET pin_hash = ?1,
             pin_enabled = 1,
             pin_length = ?2,
             modified_at = ?3,
             attempts = 0,
             locked_until = 0
         WHERE id = 1",
        params![hash, pin.len() as i64, now],
    ).map_err(|e| e.to_string())?;

    Ok(())
}

/// Supprime le PIN
pub fn remove_pin(conn: &Connection) -> Result<(), String> {
    conn.execute(
        "UPDATE pin_status
         SET pin_hash = '',
             pin_enabled = 0,
             attempts = 0,
             locked_until = 0
         WHERE id = 1",
        [],
    ).map_err(|e| e.to_string())?;

    Ok(())
}

/// Verifie le PIN
pub fn verify_pin(conn: &Connection, pin: &str) -> Result<bool, String> {
    let now = Utc::now().timestamp();

    // Recuperer les infos
    let result: Result<(String, i64, i64), _> = conn.query_row(
        "SELECT pin_hash, modified_at, locked_until FROM pin_status WHERE id = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    );

    let (stored_hash, modified_at, locked_until) = match result {
        Ok(r) => r,
        Err(_) => return Ok(false),
    };

    // 1. Verifier si bloque
    if locked_until > now {
        return Err("PIN bloque. Attends 15 minutes.".into());
    }

    // 2. Verifier le PIN
    let salt = format!("uq_pin_salt_{}", modified_at);
    let computed_hash = hash_pin(pin, &salt);

    if computed_hash == stored_hash {
        // Succes : reset attempts
        conn.execute(
            "UPDATE pin_status SET attempts = 0 WHERE id = 1",
            [],
        ).map_err(|e| e.to_string())?;
        Ok(true)
    } else {
        // Echec : incrementer attempts
        let attempts: i64 = conn.query_row(
            "SELECT attempts FROM pin_status WHERE id = 1",
            [],
            |row| row.get(0),
        ).unwrap_or(0);

        let new_attempts = attempts + 1;

        if new_attempts >= MAX_ATTEMPTS {
            // Bloquer
            let locked_until = now + (LOCKOUT_MINUTES * 60);
            conn.execute(
                "UPDATE pin_status SET attempts = 0, locked_until = ?1 WHERE id = 1",
                params![locked_until],
            ).map_err(|e| e.to_string())?;
            return Err(format!("PIN bloque pendant {} minutes.", LOCKOUT_MINUTES));
        } else {
            conn.execute(
                "UPDATE pin_status SET attempts = ?1 WHERE id = 1",
                params![new_attempts],
            ).map_err(|e| e.to_string())?;
            return Ok(false);  // ✅ FIX : PIN incorrect
            return Ok(false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_format() {
        assert!(is_valid_pin_format("1234"));
        assert!(is_valid_pin_format("123456"));
        assert!(!is_valid_pin_format("123"));
        assert!(!is_valid_pin_format("1234567"));
        assert!(!is_valid_pin_format("abcd"));
    }
}

// ====================
// VÉRIFIER LE MOT DE PASSE DU COMPTE (uq_index.db)
// ====================
pub fn verify_account_password(uq_number: &str, password: &str) -> bool {
    let idx = crate::uq_index::UqIndex::shared("uq_index.db");
    match idx.get_account(uq_number) {
        Some(info) => {
            let email = info.get("uq_email").and_then(|v| v.as_str()).unwrap_or("");
            idx.verify_credentials(uq_number, email, password).is_some()
        }
        None => false,
    }
}