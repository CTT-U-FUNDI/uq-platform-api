// ====================
// GUARDIAN ROUTES — Exposer les commandes au terminal
// ====================

use actix_web::{web, HttpResponse, HttpRequest};
use serde_json::json;
use crate::command_registry::init_registry;
use uq_blockchain::constants::{TERMINAL_AUTH_KEY, OWNER_NUMBER, OWNER_EMAIL, OWNER_PASSWORD_INITIAL, OWNER_UNLOCK_CODE};
use std::sync::Arc;
use std::sync::Mutex;

/// État du Gardien (verrouillage, sessions)
pub struct GuardianState {
    pub failed_attempts: u8,
    pub locked: bool,
    pub session_active: bool,
}

lazy_static::lazy_static! {
    pub static ref GUARDIAN_STATE: Mutex<GuardianState> = Mutex::new(GuardianState {
        failed_attempts: 0,
        locked: false,
        session_active: false,
    });
    
    pub static ref COMMAND_REGISTRY: Arc<CommandRegistry> = Arc::new(init_registry());
}

use crate::command_registry::CommandRegistry;

/// Vérifier la clé terminal
pub fn verify_terminal_key(req: &HttpRequest) -> bool {
    match req.headers().get("X-UQ-Terminal-Key") {
        Some(key) => key.to_str().unwrap_or("") == TERMINAL_AUTH_KEY.as_str(),
        None => false,
    }
}

/// GET /api/guardian/login — Connexion du Gardien
pub async fn guardian_login(body: web::Json<serde_json::Value>) -> HttpResponse {
    let number = body.get("number").and_then(|v| v.as_str()).unwrap_or("");
    let email = body.get("email").and_then(|v| v.as_str()).unwrap_or("");
    let password = body.get("password").and_then(|v| v.as_str()).unwrap_or("");
    
    let mut state = GUARDIAN_STATE.lock().unwrap();
    
    if state.locked {
        return HttpResponse::Forbidden().json(json!({
            "status": "error",
            "message": "Terminal verrouillé. Utilisez le code de déblocage."
        }));
    }
    
    // CAS 1 : Proprietaire (hardcode)
    let is_owner = number == OWNER_NUMBER
        && email == OWNER_EMAIL
        && password == OWNER_PASSWORD_INITIAL.as_str();

    if is_owner {
        state.failed_attempts = 0;
        state.session_active = true;
        drop(state);
        // Email + mot de passe verifies : le serveur horodate ce controle complet
        // et peut des lors accepter la naissance d'une cle sur cet appareil.
        crate::secure_handlers::horoder_controle_complet(
            OWNER_NUMBER,
            body.get("credential_id").and_then(|v| v.as_str()).unwrap_or(""),
        );
        // Issue a JWT token for the guardian session (24h)
        let auth = crate::auth::AuthManager::new();
        match auth.create_jwt(OWNER_NUMBER, "guardian", 24) {
            Ok(token) => HttpResponse::Ok().json(json!({
                "status": "success",
                "message": "Connexion réussie. Session JWT fournie.",
                "token": token,
            })),
            Err(_) => HttpResponse::InternalServerError().json(json!({"status":"error","message":"Failed to create token"})),
        }
    } else {
        // CAS 2 : Utilisateur normal -> verifier dans uq_index.db
        let idx = crate::uq_index::UqIndex::shared("uq_index.db");
        if let Some(account) = idx.verify_credentials(number, email, password) {
            state.failed_attempts = 0;
            state.session_active = true;
            drop(state);
            crate::secure_handlers::horoder_controle_complet(
                number,
                body.get("credential_id").and_then(|v| v.as_str()).unwrap_or(""),
            );
            let auth = crate::auth::AuthManager::new();
            match auth.create_jwt(number, "user", 24) {
                Ok(token) => return HttpResponse::Ok().json(json!({
                    "status": "success",
                    "message": "Connexion reussie",
                    "token": token,
                    "account": account
                })),
                Err(_) => return HttpResponse::InternalServerError().json(json!({
                    "status": "error",
                    "message": "Failed to create token"
                })),
            }
        }

        state.failed_attempts += 1;
        if state.failed_attempts >= 3 {
            state.locked = true;
            drop(state);
            HttpResponse::Forbidden().json(json!({
                "status": "error",
                "message": "TERMINAL VERROUILLÉ après 3 échecs. Code de déblocage requis."
            }))
        } else {
            let remaining = 3 - state.failed_attempts;
            drop(state);
            HttpResponse::Unauthorized().json(json!({
                "status": "error",
                "message": format!("Échec. Tentatives restantes : {}", remaining)
            }))
        }
    }
}

/// POST /api/guardian/unlock — Débloquer avec le code de secours (comparé en SHA-256)
pub async fn guardian_unlock(body: web::Json<serde_json::Value>) -> HttpResponse {
    let code = body.get("code").and_then(|v| v.as_str()).unwrap_or("");

    let mut state = GUARDIAN_STATE.lock().unwrap();

    let code_ok = if code.is_empty() {
        false
    } else {
        use sha2::{Digest, Sha256};
        let provided = format!("{:x}", Sha256::digest(code.as_bytes()));
        let expected = format!("{:x}", Sha256::digest(OWNER_UNLOCK_CODE.as_bytes()));
        crate::security_login::constant_time_eq(&provided, &expected)
    };

    if code_ok {
        state.locked = false;
        state.failed_attempts = 0;
        drop(state);
        HttpResponse::Ok().json(json!({
            "status": "success",
            "message": "Terminal débloqué. Vous pouvez réessayer."
        }))
    } else {
        drop(state);
        HttpResponse::Forbidden().json(json!({
            "status": "error",
            "message": "Code incorrect."
        }))
    }
}

/// GET /api/guardian/commands — Lister TOUTES les commandes
pub async fn guardian_commands(req: HttpRequest) -> HttpResponse {
    if !verify_terminal_key(&req) {
        return HttpResponse::Forbidden().json(json!({"error": "Clé terminal invalide"}));
    }
    
    let registry = COMMAND_REGISTRY.clone();
    let commands = registry.list_all();
    HttpResponse::Ok().json(commands)
}

/// POST /api/guardian/execute — Exécuter une commande
pub async fn guardian_execute(req: HttpRequest, body: web::Json<serde_json::Value>) -> HttpResponse {
    if !verify_terminal_key(&req) {
        return HttpResponse::Forbidden().json(json!({"error": "Clé terminal invalide"}));
    }
    
    let command = body.get("command").and_then(|v| v.as_str()).unwrap_or("");
    let args = body.get("args").and_then(|v| v.as_str()).unwrap_or("");
    
    let registry = COMMAND_REGISTRY.clone();
    
    match registry.execute(command, args) {
        Some(result) => HttpResponse::Ok().json(json!({
            "status": "success",
            "result": result
        })),
        None => HttpResponse::NotFound().json(json!({
            "status": "error",
            "message": format!("Commande inconnue : {}", command)
        })),
    }
}

/// GET /api/guardian/status — Voir l'état du Gardien
pub async fn guardian_status(req: HttpRequest) -> HttpResponse {
    if !verify_terminal_key(&req) {
        return HttpResponse::Forbidden().json(json!({"error": "Clé terminal invalide"}));
    }
    
    let state = GUARDIAN_STATE.lock().unwrap();
    HttpResponse::Ok().json(json!({
        "locked": state.locked,
        "failed_attempts": state.failed_attempts,
        "session_active": state.session_active,
    }))
}

/// POST /api/guardian/logout — Déconnexion
pub async fn guardian_logout(req: HttpRequest) -> HttpResponse {
    if !verify_terminal_key(&req) {
        return HttpResponse::Forbidden().json(json!({"error": "Clé terminal invalide"}));
    }
    
    let mut state = GUARDIAN_STATE.lock().unwrap();
    state.session_active = false;
    HttpResponse::Ok().json(json!({"status": "success", "message": "Déconnecté."}))
}

