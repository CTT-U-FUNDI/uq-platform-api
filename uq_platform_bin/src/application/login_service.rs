// ====================
// LOGIN SERVICE — Login piloté par user_login_config
// Un SEUL flux, adapté automatiquement selon le compte
// ====================

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

// ====================
// STRUCTURES
// ====================

#[derive(Debug, Clone, Serialize)]
pub struct LoginConfig {
    pub uq_number: String,
    pub is_owner: bool,
    pub requires_unlock_code: bool,
    pub requires_triple_attempt: bool,
    pub pin_required_step: i64,
    pub max_fails_before_iphone: i64,
}

impl LoginConfig {
    /// Config par défaut pour user normal
    pub fn default_user(uq: &str) -> Self {
        LoginConfig {
            uq_number: uq.to_string(),
            is_owner: false,
            requires_unlock_code: false,
            requires_triple_attempt: false,
            pin_required_step: 1,
            max_fails_before_iphone: 5,
        }
    }

    /// Config par défaut pour owner
    pub fn default_owner(uq: &str) -> Self {
        LoginConfig {
            uq_number: uq.to_string(),
            is_owner: true,
            requires_unlock_code: true,
            requires_triple_attempt: true,
            pin_required_step: 3,
            max_fails_before_iphone: 3,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct LoginStep1Result {
    pub uq_number: String,
    pub is_owner: bool,
    pub session_id: String,
    pub tfa_token: String,
    pub next_step: i64,
    pub requires_triple_attempt: bool,
    pub requires_unlock_code: bool,
    pub max_attempts: i64,
}

#[derive(Debug, Serialize)]
pub struct LoginStep2Result {
    pub status: String,        // "attempt_required" | "step_required" | "error"
    pub is_owner: bool,
    pub next_step: i64,
    pub tfa_token: String,
    pub attempts_done: i64,
    pub attempts_total: i64,
    pub message: Option<String>,
}

// ====================
// CHARGER LA CONFIG
// ====================

pub fn get_login_config(uq_number: &str) -> Result<LoginConfig, String> {
    let db_path = format!("data/users/{}.db", uq_number.replace("-", "_").to_lowercase());

    if !std::path::Path::new(&db_path).exists() {
        // Comptes servis par Firestore (uq_index) n'ont pas de DB locale :
        // la config login est DERIVEE de l'index — le serveur ne garde
        // que les preuves, pas de fichier par compte.
        let idx = crate::uq_index::UqIndex::shared("uq_index.db");
        let is_owner = idx.get_account(uq_number)
            .map(|a| a.get("is_owner").and_then(|v| v.as_bool()).unwrap_or(false))
            .unwrap_or(uq_number == crate::auth_guard::OWNER_UQ);
        return Ok(if is_owner {
            LoginConfig::default_owner(uq_number)
        } else {
            LoginConfig::default_user(uq_number)
        });
    }

    let conn = Connection::open(&db_path).map_err(|e| e.to_string())?;

    // Essayer de lire depuis user_login_config
    let result: Result<(i64, i64, i64, i64, i64), _> = conn.query_row(
        "SELECT is_owner, requires_unlock_code, requires_triple_attempt, pin_required_step, max_fails_before_iphone
         FROM user_login_config WHERE uq_number = ?1",
        params![uq_number],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
    );

    match result {
        Ok((is_owner, unlock, triple, pin_step, max_fails)) => Ok(LoginConfig {
            uq_number: uq_number.to_string(),
            is_owner: is_owner == 1,
            requires_unlock_code: unlock == 1,
            requires_triple_attempt: triple == 1,
            pin_required_step: pin_step,
            max_fails_before_iphone: max_fails,
        }),
        Err(_) => {
            // Pas de config → utiliser les défauts
            let is_owner = uq_number == "UQ-996697110";
            if is_owner {
                Ok(LoginConfig::default_owner(uq_number))
            } else {
                Ok(LoginConfig::default_user(uq_number))
            }
        }
    }
}

// ====================
// VÉRIFIER SI LE UQ EXISTE
// ====================

pub fn account_exists(uq_number: &str) -> bool {
    let idx = crate::uq_index::UqIndex::shared("uq_index.db");
    idx.get_account(uq_number).is_some()
}

// ====================
// CRÉER LA CONFIG SI MANQUANTE (auto-guérison)
// ====================

pub fn ensure_config_exists(uq_number: &str) -> Result<(), String> {
    let db_path = format!("data/users/{}.db", uq_number.replace("-", "_").to_lowercase());

    if !std::path::Path::new(&db_path).exists() {
        return Err("Compte introuvable".to_string());
    }

    let conn = Connection::open(&db_path).map_err(|e| e.to_string())?;

    // Vérifier si la config existe
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM user_login_config WHERE uq_number = ?1",
        params![uq_number],
        |row| row.get(0),
    ).unwrap_or(0);

    if count == 0 {
        // Créer la config par défaut
        let is_owner = uq_number == "UQ-996697110";
        let (unlock, triple, pin_step, max_fails) = if is_owner { (1, 1, 3, 3) } else { (0, 0, 1, 5) };
        let now = chrono::Utc::now().timestamp();

        conn.execute(
            "INSERT INTO user_login_config 
             (uq_number, is_owner, requires_unlock_code, requires_triple_attempt, pin_required_step, max_fails_before_iphone, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![uq_number, is_owner as i64, unlock, triple, pin_step, max_fails, now],
        ).map_err(|e| e.to_string())?;

        eprintln!("✅ user_login_config créé automatiquement pour {}", uq_number);
    }

    Ok(())
}

// ====================
// VALIDER LE UQ (step1)
// ====================

pub fn validate_uq_step1(uq_number: &str, allow_circle_member: bool) -> Result<LoginConfig, String> {
    // 1. Verifier que le compte existe
    if !account_exists(uq_number) {
        return Err("Compte introuvable".to_string());
    }

    // 1.5 BLOQUER si le compte est membre d'un cercle (sauf bascule owner)
    // Un membre de cercle ne peut PAS se connecter seul depuis l'exterieur.
    // Il doit passer par le compte owner (UQ-996697110) via X-Switch-From.
    if !allow_circle_member {
        if let Ok(conn) = Connection::open("uq_index.db") {
            let is_member: i64 = conn.query_row(
                "SELECT COUNT(*) FROM circle_members WHERE member_uq = ?1 AND is_active = 1",
                rusqlite::params![uq_number],
                |row| row.get(0),
            ).unwrap_or(0);

            if is_member > 0 {
                eprintln!("[CIRCLE] Connexion externe bloquee pour {} (membre d'un cercle)", uq_number);
                return Err("Ce compte est lie a un proprietaire. Utilisez le compte proprietaire pour y acceder.".to_string());
            }
        }
    } else {
        eprintln!("[CIRCLE] Bascule autorisee pour {} (X-Switch-From owner)", uq_number);
    }

    // 2. Verifier la config
    let config = get_login_config(uq_number)?;

    // 3. Si pas de config  auto-creer
    if config.max_fails_before_iphone == 0 {
        ensure_config_exists(uq_number)?;
        return get_login_config(uq_number);
    }

    Ok(config)
}

// ====================
// COMPTER LES TENTATIVES PERSISTANTES (owner)
// ====================

pub fn get_attempts_ok(uq_number: &str) -> i64 {
    let db_path = format!("data/users/{}.db", uq_number.replace("-", "_").to_lowercase());
    let owner_db = "data/users/uq_996697110.db";

    if let Ok(conn) = Connection::open(owner_db) {
        return conn.query_row(
            "SELECT COALESCE(attempts_ok, 0) FROM security_login_cooldown_uq WHERE uq_number = ?1",
            params![uq_number],
            |row| row.get(0),
        ).unwrap_or(0);
    }
    0
}

pub fn increment_attempts_ok(uq_number: &str) -> i64 {
    let owner_db = "data/users/uq_996697110.db";
    let now = chrono::Utc::now().timestamp();

    if let Ok(conn) = Connection::open(owner_db) {
        let current = get_attempts_ok(uq_number);
        let new_value = current + 1;

        let _ = conn.execute(
            "INSERT INTO security_login_cooldown_uq (uq_number, attempts_ok, last_attempt)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(uq_number) DO UPDATE SET attempts_ok = ?2, last_attempt = ?3",
            params![uq_number, new_value, now],
        );

        return new_value;
    }
    -1
}

pub fn reset_attempts_ok(uq_number: &str) {
    let owner_db = "data/users/uq_996697110.db";
    if let Ok(conn) = Connection::open(owner_db) {
        let _ = conn.execute(
            "UPDATE security_login_cooldown_uq SET attempts_ok = 0 WHERE uq_number = ?1",
            params![uq_number],
        );
    }
}

// ====================
// CALCULER LE NEXT STEP
// ====================

pub fn calculate_next_step(config: &LoginConfig, attempts_done: i64) -> i64 {
    if config.is_owner {
        // Owner : 3 tentatives avant le step 3543
        if attempts_done < 3 {
            2  // Redemander email+password
        } else {
            3  // Passer au code 3543
        }
    } else {
        // User : direct au 2FA
        3
    }
}

// ====================
// VALIDER L'ÉTAPE 2 (password)
// ====================

pub fn validate_step2(
    uq_number: &str,
    email_saisi: &str,
    password: &str,
) -> Result<LoginStep2Result, String> {
    let config = get_login_config(uq_number)?;

    // 1. Vérifier le couple email + password saisis
    let idx = crate::uq_index::UqIndex::shared("uq_index.db");
    let account = idx.get_account(uq_number).ok_or("Compte introuvable")?;
    let email_compte = account.get("uq_email").and_then(|v| v.as_str()).unwrap_or("");

    let email_saisi = email_saisi.trim();
    let correspond = !email_compte.is_empty()
        && email_saisi.to_lowercase() == email_compte.to_lowercase();

    if !correspond || idx.verify_credentials(uq_number, email_compte, password).is_none() {
        // Échec : incrémenter le cooldown. Message unique : ne pas dire lequel
        // des deux champs est faux réduit l'oracle offert à qui devine un UQ.
        let _ = increment_failure(uq_number);
        return Err("Identifiants incorrects".to_string());
    }

    // 2. Owner : compter les tentatives
    if config.requires_triple_attempt {
        let attempts = increment_attempts_ok(uq_number);
        let next_step = calculate_next_step(&config, attempts);

        if next_step == 2 {
            // Redemander email + password
            return Ok(LoginStep2Result {
                status: "attempt_required".to_string(),
                is_owner: true,
                next_step: 2,
                tfa_token: String::new(),
                attempts_done: attempts,
                attempts_total: 3,
                message: Some(format!("Tentative {}/3", attempts)),
            });
        } else {
            // Passer au 3543
            reset_attempts_ok(uq_number);
            return Ok(LoginStep2Result {
                status: "step_required".to_string(),
                is_owner: true,
                next_step: 3,
                tfa_token: generate_tfa_token(),
                attempts_done: 3,
                attempts_total: 3,
                message: Some("Code 3543 requis".to_string()),
            });
        }
    }

    // 3. User : passer directement au 2FA
    Ok(LoginStep2Result {
        status: "step_required".to_string(),
        is_owner: false,
        next_step: 3,
        tfa_token: generate_tfa_token(),
        attempts_done: 0,
        attempts_total: 0,
        message: None,
    })
}

// ====================
// INCRÉMENTER LES ÉCHECS (cooldown)
// ====================

pub fn increment_failure(uq_number: &str) -> i64 {
    let owner_db = "data/users/uq_996697110.db";
    let now = chrono::Utc::now().timestamp();

    if let Ok(conn) = Connection::open(owner_db) {
        // Lire le compteur actuel
        let current: i64 = conn.query_row(
            "SELECT COALESCE(failure_count, 0) FROM security_login_cooldown_uq WHERE uq_number = ?1",
            params![uq_number],
            |row| row.get(0),
        ).unwrap_or(0);

        let new_count = current + 1;

        // Calculer le niveau de cooldown
        let config = get_login_config(uq_number).unwrap_or(LoginConfig::default_user(uq_number));
        let max_attempts = config.max_fails_before_iphone;

        let level = if new_count < max_attempts {
            0
        } else {
            let over = new_count - max_attempts;
            let step = if config.is_owner { 3 } else { 5 };
            ((over / step) + 1).min(5)
        };

        let cooldown_seconds: i64 = match level {
            0 => 0,
            1 => 60,
            2 => 300,
            3 => 900,
            4 => 3600,
            5 => 86400,
            _ => 604800,
        };

        let cooldown_until = if level > 0 { now + cooldown_seconds } else { 0 };

        // Upsert
        let _ = conn.execute(
            "INSERT INTO security_login_cooldown_uq 
             (uq_number, is_owner, failure_count, cooldown_level, cooldown_until, last_attempt, first_failure_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
             ON CONFLICT(uq_number) DO UPDATE SET
                failure_count = ?3,
                cooldown_level = ?4,
                cooldown_until = ?5,
                last_attempt = ?6",
            params![
                uq_number,
                config.is_owner as i64,
                new_count,
                level,
                cooldown_until,
                now
            ],
        );

        return new_count;
    }
    0
}

// ====================
// VÉRIFIER SI EN COOLDOWN
// ====================

pub fn is_in_cooldown(uq_number: &str) -> Option<i64> {
    let owner_db = "data/users/uq_996697110.db";
    let now = chrono::Utc::now().timestamp();

    if let Ok(conn) = Connection::open(owner_db) {
        let result: Result<i64, _> = conn.query_row(
            "SELECT cooldown_until FROM security_login_cooldown_uq WHERE uq_number = ?1",
            params![uq_number],
            |row| row.get(0),
        );

        if let Ok(until) = result {
            if until > now {
                return Some(until - now);
            }
        }
    }
    None
}

// ====================
// RESET COOLDOWN
// ====================

pub fn reset_cooldown(uq_number: &str) {
    let owner_db = "data/users/uq_996697110.db";
    if let Ok(conn) = Connection::open(owner_db) {
        let _ = conn.execute(
            "UPDATE security_login_cooldown_uq SET failure_count = 0, cooldown_level = 0, cooldown_until = 0 WHERE uq_number = ?1",
            params![uq_number],
        );
    }
}

// ====================
// GÉNÉRER UN TFA TOKEN
// ====================

pub fn generate_tfa_token() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let bytes: [u8; 32] = rng.gen();
    hex::encode(bytes)
}

// ====================
// TESTS
// ====================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_user_config() {
        let cfg = LoginConfig::default_user("UQ-732688531");
        assert!(!cfg.is_owner);
        assert!(!cfg.requires_unlock_code);
        assert!(!cfg.requires_triple_attempt);
        assert_eq!(cfg.pin_required_step, 1);
        assert_eq!(cfg.max_fails_before_iphone, 5);
    }

    #[test]
    fn test_default_owner_config() {
        let cfg = LoginConfig::default_owner("UQ-996697110");
        assert!(cfg.is_owner);
        assert!(cfg.requires_unlock_code);
        assert!(cfg.requires_triple_attempt);
        assert_eq!(cfg.pin_required_step, 3);
        assert_eq!(cfg.max_fails_before_iphone, 3);
    }

    #[test]
    fn test_next_step_user() {
        let cfg = LoginConfig::default_user("UQ-TEST");
        assert_eq!(calculate_next_step(&cfg, 0), 3);
        assert_eq!(calculate_next_step(&cfg, 1), 3);
    }

    #[test]
    fn test_next_step_owner() {
        let cfg = LoginConfig::default_owner("UQ-996697110");
        assert_eq!(calculate_next_step(&cfg, 0), 2);
        assert_eq!(calculate_next_step(&cfg, 1), 2);
        assert_eq!(calculate_next_step(&cfg, 2), 2);
        assert_eq!(calculate_next_step(&cfg, 3), 3);
    }
}