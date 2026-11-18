// ====================
// SECURE HANDLERS — Owner 3 tentatives + 3543 + 2FA
// ====================

use actix_web::{web, HttpRequest, HttpResponse};
use serde_json::json;
use crate::security_login;

// ====================
// CHEMIN ABSOLU — cherche data/users/uq_996697110.db
// à partir du répertoire du binaire
// ====================
fn get_owner_db_path() -> String {
    // 1. Essayer le chemin relatif standard
    let relative = "data/users/uq_996697110.db";
    if std::path::Path::new(relative).exists() {
        return relative.to_string();
    }
    
    // 2. Essayer depuis le répertoire du binaire
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            // Remonter de target/release/ vers le projet
            let candidates = vec![
                exe_dir.join("data/users/uq_996697110.db"),
                exe_dir.join("../data/users/uq_996697110.db"),
                exe_dir.join("../../data/users/uq_996697110.db"),
                exe_dir.join("../../../data/users/uq_996697110.db"),
                exe_dir.join("../../../../data/users/uq_996697110.db"),
            ];
            
            for candidate in candidates {
                if candidate.exists() {
                    if let Some(path_str) = candidate.to_str() {
                        return path_str.to_string();
                    }
                }
            }
        }
    }
    
    // 3. Fallback : chemin absolu connu
    let fallback = r"C:\Users\destin\Desktop\UQ_Platform\uq_platform_bin\data\users\uq_996697110.db";
    if std::path::Path::new(fallback).exists() {
        return fallback.to_string();
    }
    
    // 4. Vraiment rien trouvé → retourner le relatif
    relative.to_string()
}

fn get_ip(req: &HttpRequest) -> String {
    req.headers().get("X-Forwarded-For")
        .and_then(|h| h.to_str().ok())
        .map(|s| s.split(',').next().unwrap_or(s).trim().to_string())
        .or_else(|| req.peer_addr().map(|a| a.ip().to_string()))
        .unwrap_or_else(|| "unknown".to_string())
}

fn bad_data() -> HttpResponse {
    HttpResponse::NotFound().json(json!({
        "status": "error",
        "message": "Mauvaise donnée"
    }))
}

fn iphone_mode_response(cooldown_until: i64, level: i64, attempts: i64) -> HttpResponse {
    let now = chrono::Utc::now().timestamp();
    let remaining = (cooldown_until - now).max(0);
    HttpResponse::TooManyRequests().json(json!({
        "status": "iphone_mode",
        "cooldown_until": cooldown_until,
        "cooldown_remaining": remaining,
        "cooldown_level": level,
        "attempts": attempts,
        "message": "Compte temporairement bloqué"
    }))
}

/// Un controle complet (email + mot de passe, ou PIN proprietaire) vient de
/// reussir : le serveur l'horodate. Deux usages :
///  * l'enrolement d'un passkey dans la foulée en hérite (sinon l'appareil
///    neuf repartirait au palier complet a chaque connexion) ;
///  * le plafond de 30 jours repart de ce moment.
///
/// `credential_id` vient du client mais ne fait que designe une cle deja
/// enregistree pour ce compte : elle ne peut pas creer un appareil.
pub fn horoder_controle_complet(uq: &str, credential_id: &str) {
    let conn = match crate::device_auth::ouvrir(uq) {
        Ok(conn) => conn,
        Err(_) => return,
    };
    let maintenant = chrono::Utc::now().timestamp();
    let _ = crate::device_auth::tracer_controle_complet(&conn, maintenant);
    if credential_id.trim().is_empty() {
        return;
    }
    let deja_known = matches!(
        crate::webauthn_manager::lire_credential(&conn, credential_id.trim()),
        Ok(Some(_))
    );
    if deja_known {
        let _ = crate::device_auth::tracer_connexion(&conn, credential_id.trim(), true, maintenant);
    }
}

// ====================
// ÉTAPE 1 — Vérifier UQ
// ====================
pub async fn secure_step1(
    req: HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    // Accepte uq_number OU uq_email : si on reçoit un email, on résout
    // le compte en interne — l'utilisateur n'a plus besoin de taper son UQ.
    let input = body.get("uq_number")
        .or_else(|| body.get("uq_email"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let ip = get_ip(&req);

    if input.is_empty() {
        return bad_data();
    }

    // Résolution : si l'input ressemble à un email (contient '@' et ne
    // commence pas par "UQ-"), on cherche le compte par email.
    let uq = if input.contains('@') && !input.starts_with("UQ-") {
        let idx = crate::uq_index::UqIndex::shared(&crate::uq_index::uq_index_path());
        match idx.get_uq_by_email(&input) {
            Some(uq) => uq,
            None => return bad_data(),
        }
    } else {
        input
    };

    // ===== BASCULE OWNER : Lire X-Switch-From =====
    // Si le header est present ET vaut UQ-996697110 (owner) => autoriser la connexion
    // d'un membre de cercle (bascule). Sinon => bloquer si membre.
    let allow_circle_member = req
        .headers()
        .get("X-Switch-From")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim() == "UQ-996697110")
        .unwrap_or(false);

    if allow_circle_member {
        eprintln!("[SWITCH] Header X-Switch-From detecte pour {} -> bascule owner", uq);
    }

    // 1. Vérifier que le compte existe + charger la config
    let config = match crate::application::login_service::validate_uq_step1(&uq, allow_circle_member) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[SWITCH] validate_uq_step1 refuse pour {} : {}", uq, e);
            return bad_data();
        }
    };

    // 2. Vérifier cooldown
    if let Some(_remaining) = crate::application::login_service::is_in_cooldown(&uq) {
        return bad_data();
    }

    // 3. Créer la session
    let conn = match rusqlite::Connection::open(&get_owner_db_path()) {
        Ok(c) => c,
        Err(_) => return bad_data(),
    };
    let _ = security_login::init_tables(&conn);

    match security_login::create_session(&conn, &ip, "web", &uq) {
        Ok((session_id, tfa_token)) => {
            let now = chrono::Utc::now().timestamp();
            let _ = conn.execute(
                "UPDATE security_login_sessions SET current_step = 2, step1_ok = 1, last_activity = ?1 WHERE session_id = ?2",
                rusqlite::params![now, session_id],
            );

            eprintln!(
                "🔓 Étape 1 OK pour {} ({}) — step=2, owner={}",
                uq,
                if config.is_owner { "OWNER" } else { "USER" },
                config.is_owner
            );

            HttpResponse::Ok().json(json!({
                "status": "step_required",
                "step": 2,
                "session_id": session_id,
                "tfa_token": tfa_token,
                "is_owner": config.is_owner,
                "requires_triple_attempt": config.requires_triple_attempt,
                "requires_unlock_code": config.requires_unlock_code,
                "max_attempts": config.max_fails_before_iphone,
            }))
        }
        Err(_) => bad_data(),
    }
}

pub async fn secure_step2(
    req: HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let session_id = body.get("session_id").and_then(|v| v.as_str()).unwrap_or("");
    let tfa_token = body.get("tfa_token").and_then(|v| v.as_str()).unwrap_or("");
    let email = body.get("email").and_then(|v| v.as_str()).unwrap_or("");
    let password = body.get("password").and_then(|v| v.as_str()).unwrap_or("");
    let ip = get_ip(&req);

    // Login « UQ + mot de passe » : l'email est OPTIONNEL. S'il est absent,
    // le serveur le resout depuis le uq_number de la session — l'utilisateur
    // ne saisit jamais son email dans l'app UQZ.
    if session_id.is_empty() || tfa_token.is_empty() || password.is_empty() {
        return bad_data();
    }

    // 1. Vérifier la session
    let conn = match rusqlite::Connection::open(&get_owner_db_path()) {
        Ok(c) => c,
        Err(_) => return bad_data(),
    };

    let result: Result<(String, i64, String), _> = conn.query_row(
        "SELECT tfa_token, current_step, uq_number FROM security_login_sessions WHERE session_id = ?1 AND status = 'in_progress'",
        rusqlite::params![session_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    );

    let (stored_tfa, current_step, uq) = match result {
        Ok(v) => v,
        Err(_) => return bad_data(),
    };

    if stored_tfa != tfa_token || current_step != 2 {
        let _ = crate::application::login_service::increment_failure(&uq);
        return bad_data();
    }

    // Email absent -> resolution interne par numero UQ (login mobile).
    let email_resolu;
    let email = if email.trim().is_empty() {
        let idx = crate::uq_index::UqIndex::shared(&crate::uq_index::uq_index_path());
        match idx.get_email(&uq) {
            Some(e) if !e.trim().is_empty() => { email_resolu = e; &email_resolu }
            _ => return bad_data(),
        }
    } else { email };

    // 2. Valider email + password via le service
    match crate::application::login_service::validate_step2(&uq, email, password) {
        Ok(result) => {
            let new_tfa = crate::application::login_service::generate_tfa_token();
            let now = chrono::Utc::now().timestamp();

            if result.status == "attempt_required" {
                // Owner : tentative intermédiaire → reset session
                let _ = conn.execute(
                    "UPDATE security_login_sessions SET status = 'abandoned' WHERE session_id = ?1",
                    rusqlite::params![session_id],
                );

                eprintln!(
                    "🔄 Tentative {}/{} réussie (owner)",
                    result.attempts_done, result.attempts_total
                );

                return HttpResponse::Ok().json(json!({
                    "status": "attempt_required",
                    "attempts_ok": result.attempts_done,
                    "attempts_total": result.attempts_total,
                    "next_attempt": result.attempts_done + 1,
                    "message": result.message,
                }));
            }

            // step_required (owner ou user)
            let next_step = result.next_step;
            let _ = conn.execute(
                "UPDATE security_login_sessions SET current_step = ?1, step2_ok = 1, tfa_token = ?2, last_activity = ?3 WHERE session_id = ?4",
                rusqlite::params![next_step, new_tfa, now, session_id],
            );

            eprintln!(
                "🔐 Étape 2 OK pour {} → step {} ({})",
                uq, next_step,
                if result.is_owner { "OWNER" } else { "USER" }
            );

            HttpResponse::Ok().json(json!({
                "status": "step_required",
                "step": next_step,
                "tfa_token": new_tfa,
                "session_id": session_id,
                "is_owner": result.is_owner,
            }))
        }
        Err(e) => {
            eprintln!("⚠️  Échec step2 pour {} : {}", uq, e);
            let _ = crate::application::login_service::increment_failure(&uq);
            bad_data()
        }
    }
}
pub async fn secure_step_login_pin(
    req: HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let session_id = body.get("session_id").and_then(|v| v.as_str()).unwrap_or("");
    let tfa_token = body.get("tfa_token").and_then(|v| v.as_str()).unwrap_or("");
    let login_pin = body.get("login_pin").and_then(|v| v.as_str()).unwrap_or("");

    if session_id.is_empty() || tfa_token.is_empty() || login_pin.is_empty() {
        return bad_data();
    }

    let conn = match rusqlite::Connection::open(&get_owner_db_path()) {
        Ok(c) => c,
        Err(_) => return bad_data(),
    };

    let result: Result<(String, i64, String), _> = conn.query_row(
        "SELECT tfa_token, current_step, uq_number FROM security_login_sessions WHERE session_id = ?1 AND status = 'in_progress'",
        rusqlite::params![session_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    );

    let (stored_tfa, current_step, _uq) = match result {
        Ok(v) => v,
        Err(_) => {
            eprintln!("🔍 step-login-pin: session introuvable {}", session_id);
            return HttpResponse::Ok().json(json!({"status":"error","message":"SESSION_INTROUVABLE","debug":"session"}));
        }
    };

    if stored_tfa != tfa_token || current_step != 3 {
        eprintln!("🔍 step-login-pin: gate fail tfa={} step={} (attendu step=3)", stored_tfa==tfa_token, current_step);
        return HttpResponse::Ok().json(json!({"status":"error","message":"GATE_FAIL","debug":format!("step={}",current_step)}));
    }

    let salt: String = conn.query_row(
        "SELECT value FROM settings WHERE key = 'login_pin_salt'",
        [], |row| row.get(0)
    ).unwrap_or_else(|_| security_login::DEFAULT_LOGIN_PIN_SALT.to_string());

    let stored_hash: String = conn.query_row(
        "SELECT value FROM settings WHERE key = 'login_pin_hash'",
        [], |row| row.get(0)
    ).unwrap_or_default();

    // Hash fail-closed : sans amorce en DB, seul le PIN de secours (comparé en hash) passe.
    let computed = {
        use sha2::{Sha256, Digest};
        let mut hasher = Sha256::new();
        hasher.update(format!("{}:{}", login_pin, salt).as_bytes());
        format!("{:x}", hasher.finalize())
    };
    // Le code owner env (OWNER_UNLOCK_CODE) est accepte en plus du hash DB :
    // la DB Render peut contenir une autre valeur — l'env fait foi.
    let env_ok = std::env::var("OWNER_UNLOCK_CODE")
        .map(|c| security_login::constant_time_eq(login_pin, c.trim()))
        .unwrap_or(false);
    let pin_ok = env_ok || if stored_hash.is_empty() {
        security_login::constant_time_eq(
            &security_login::hash_pin(login_pin, security_login::DEFAULT_LOGIN_PIN_SALT),
            security_login::FALLBACK_OWNER_PIN_HASH,
        )
    } else {
        security_login::constant_time_eq(&computed, &stored_hash)
    };

    if !pin_ok {
        eprintln!("🔍 step-login-pin: PIN incorrect (env_ok={})", env_ok);
        return HttpResponse::Ok().json(json!({"status":"error","message":"PIN_INCORRECT","debug":format!("env_ok={}",env_ok)}));
    }

    let new_tfa = security_login::generate_tfa_token();
    let now = chrono::Utc::now().timestamp();
    
    let _ = conn.execute(
        "UPDATE security_login_sessions SET current_step = 4, step3_ok = 1, tfa_token = ?1, last_activity = ?2 WHERE session_id = ?3",
        rusqlite::params![new_tfa, now, session_id],
    );

    eprintln!("✅ Étape 3 OK (3543) — Step 4");

    HttpResponse::Ok().json(json!({
        "status": "step_required",
        "step": 4,
        "tfa_token": new_tfa,
        "session_id": session_id,
        "is_owner": true
    }))
}

// ====================
// ÉTAPE 4/5 — FINALISER OWNER (après 2FA + Biométrie)
// ====================
pub async fn secure_finalize_owner(
    req: HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let session_id = body.get("session_id").and_then(|v| v.as_str()).unwrap_or("");
    let tfa_token = body.get("tfa_token").and_then(|v| v.as_str()).unwrap_or("");
    let ip = get_ip(&req);

    if session_id.is_empty() || tfa_token.is_empty() {
        return bad_data();
    }

    let conn = match rusqlite::Connection::open(&get_owner_db_path()) {
        Ok(c) => c,
        Err(_) => return bad_data(),
    };

    let result: Result<(String, String, i64), _> = conn.query_row(
        "SELECT tfa_token, uq_number, is_owner FROM security_login_sessions WHERE session_id = ?1 AND status = 'in_progress'",
        rusqlite::params![session_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    );

    let (stored_tfa, uq, is_owner) = match result {
        Ok(v) => v,
        Err(_) => return bad_data(),
    };

    if is_owner != 1 || stored_tfa != tfa_token {
        return bad_data();
    }

    let auth = crate::auth::AuthManager::new();
    let token = match auth.create_jwt(&uq, "guardian", 24) {
        Ok(t) => t,
        Err(_) => return bad_data(),
    };

    if let Err(_) = security_login::finalize_session(&conn, session_id, &ip, &token) {
        return bad_data();
    }
    security_login::reset_cooldown_uq(&conn, &uq);
    horoder_controle_complet(
        &uq,
        body.get("credential_id").and_then(|v| v.as_str()).unwrap_or(""),
    );

    // 🧹 Auto-cleanup du compte owner (par transaction / par compte / 1x 24h)
    if let Some(rapport) = crate::auto_cleanup::purger_compte_au_login(&uq) {
        if rapport.lignes_supprimees > 0 {
            eprintln!(
                "🧹 Historique de {} : {} ligne(s) de plus d'un an supprimee(s)",
                uq, rapport.lignes_supprimees
            );
        }
    }

    eprintln!("✅ CONNEXION OWNER RÉUSSIE (IP: {})", ip);

    HttpResponse::Ok().json(json!({
        "status": "success",
        "token": token,
        "uq_number": security_login::OWNER_UQ,
        "uq_email": "delis@ufundi.uq",
        "uq_pseudo": "uq@dev",
        "display_name": security_login::OWNER_DISPLAY,
        "public_uq": security_login::OWNER_PUBLIC_UQ,
        "is_owner": true
    }))
}

// ====================
// FINALISER USER
// ====================
pub async fn secure_finalize_user(
    req: HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let session_id = body.get("session_id").and_then(|v| v.as_str()).unwrap_or("");
    let tfa_token = body.get("tfa_token").and_then(|v| v.as_str()).unwrap_or("");
    let ip = get_ip(&req);

    if session_id.is_empty() || tfa_token.is_empty() {
        return bad_data();
    }

    let conn = match rusqlite::Connection::open(&get_owner_db_path()) {
        Ok(c) => c,
        Err(_) => return bad_data(),
    };

    let result: Result<(String, String, i64), _> = conn.query_row(
        "SELECT tfa_token, uq_number, is_owner FROM security_login_sessions WHERE session_id = ?1 AND status = 'in_progress'",
        rusqlite::params![session_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    );

    let (stored_tfa, uq, is_owner) = match result {
        Ok(v) => v,
        Err(_) => return bad_data(),
    };

    if is_owner == 1 || stored_tfa != tfa_token {
        return bad_data();
    }

    let auth = crate::auth::AuthManager::new();
    let token = match auth.create_jwt(&uq, "user", 24) {
        Ok(t) => t,
        Err(_) => return bad_data(),
    };

    if let Err(_) = security_login::finalize_session(&conn, session_id, &ip, &token) {
        return bad_data();
    }
    security_login::reset_cooldown_uq(&conn, &uq);
    horoder_controle_complet(
        &uq,
        body.get("credential_id").and_then(|v| v.as_str()).unwrap_or(""),
    );

    let idx = crate::uq_index::UqIndex::shared(&crate::uq_index::uq_index_path());
    let account_info = idx.get_account(&uq);

    // ====================
    // 🧹 AUTO-CLEANUP DU COMPTE (règles 1/2/3)
    // Chaque compte gère SON cycle au démarrage de SON app :
    //   - une transaction à la fois (LIMIT 1)
    //   - uniquement CE compte
    //   - au maximum une passe par 24 h
    // ====================
    if let Some(rapport) = crate::auto_cleanup::purger_compte_au_login(&uq) {
        if rapport.lignes_supprimees > 0 {
            eprintln!(
                "🧹 Historique de {} : {} ligne(s) de plus d'un an supprimee(s) (reste {})",
                uq, rapport.lignes_supprimees, rapport.reste_a_purger
            );
        }
    }

    eprintln!("✅ CONNEXION USER RÉUSSIE pour {}", uq);

    HttpResponse::Ok().json(json!({
        "status": "success",
        "token": token,
        "uq_number": uq,
        "account": account_info,
        "is_owner": false
    }))
}

// ====================
// FORGOT PASSWORD
// ====================
pub async fn secure_forgot_password(body: web::Json<serde_json::Value>) -> HttpResponse {
    let uq = body.get("uq_number").and_then(|v| v.as_str()).unwrap_or("");
    eprintln!("🔑 Réinitialisation demandée pour {}", uq);
    HttpResponse::Ok().json(json!({
        "status": "success",
        "message": "Si ce compte existe, vous recevrez un email."
    }))
}








// DEBUG BETA — verifie la resolution d'un UQ sans connexion.
pub async fn debug_account_check(body: web::Json<serde_json::Value>) -> HttpResponse {
    let uq = body.get("uq_number").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    if uq.is_empty() { return bad_data(); }

    let idx = crate::uq_index::UqIndex::shared(&crate::uq_index::uq_index_path());
    let account = idx.get_account(&uq);
    let exists = account.is_some();
    let is_owner = account.as_ref()
        .and_then(|a| a.get("is_owner").and_then(|v| v.as_bool()))
        .unwrap_or(false);

    let config = crate::application::login_service::get_login_config(&uq).ok();

    HttpResponse::Ok().json(json!({
        "uq": uq,
        "exists_in_index": exists,
        "is_owner": is_owner,
        "login_config_ok": config.is_some(),
        "is_owner_config": config.as_ref().map(|c| c.is_owner),
        "firestore_env": std::env::var("FIREBASE_SA_CLIENT_EMAIL").is_ok(),
    }))
}
