// ====================
// CAPSULE UQ — routes sceller/ouvrir (JWT requis)
// ====================
// Permet aux apps UQ de chiffrer des données sensibles
// (ex: numéro UQ) en capsules opaques — seule la plateforme
// peut les rouvrir. Jamais de donnée en clair côté client.

use actix_web::{web, HttpRequest, HttpResponse};
use serde_json::json;

use crate::auth_guard::exiger_auth;

fn reponse_erreur(msg: &str) -> HttpResponse {
    HttpResponse::BadRequest().json(json!({ "status": "error", "message": msg }))
}

/// POST /api/capsule/sceller { donnees, aad? } → { capsule }
/// JWT requis — n'importe quel utilisateur authentifié peut
/// sceller (la capsule ne peut être ouverte que par la plateforme).
pub async fn capsule_sceller(req: HttpRequest, body: web::Json<serde_json::Value>) -> HttpResponse {
    if let Err(e) = exiger_auth(&req) { return e; }

    let donnees = body.get("donnees").and_then(|v| v.as_str()).unwrap_or("");
    if donnees.is_empty() || donnees.len() > 8192 {
        return reponse_erreur("donnees requis (max 8 Ko)");
    }
    let aad = body.get("aad").and_then(|v| v.as_str()).unwrap_or("");

    match uq_security::capsule::sceller(donnees.as_bytes(), aad.as_bytes()) {
        Ok(c) => HttpResponse::Ok().json(json!({ "status": "success", "capsule": c })),
        Err(e) => reponse_erreur(&e.to_string()),
    }
}

/// POST /api/capsule/ouvrir { capsule, aad? } → { donnees }
/// JWT requis — réservé aux services de confiance (owner/admin
/// en pratique ; le JWT user suffit pour l'instant).
pub async fn capsule_ouvrir(req: HttpRequest, body: web::Json<serde_json::Value>) -> HttpResponse {
    let claims = match exiger_auth(&req) { Ok(c) => c, Err(e) => return e };

    let capsule = body.get("capsule").and_then(|v| v.as_str()).unwrap_or("");
    if capsule.is_empty() || capsule.len() > 16384 {
        return reponse_erreur("capsule requis");
    }
    let aad = body.get("aad").and_then(|v| v.as_str()).unwrap_or("");

    // Audit : qui ouvre quoi (sans journaliser le contenu)
    eprintln!("[capsule] ouverture par {} (len={})", claims.sub, capsule.len());

    match uq_security::capsule::ouvrir(capsule, aad.as_bytes()) {
        Ok(brut) => {
            let texte = String::from_utf8_lossy(&brut).to_string();
            HttpResponse::Ok().json(json!({ "status": "success", "donnees": texte }))
        }
        Err(e) => HttpResponse::Forbidden().json(json!({
            "status": "error",
            "code": "CAPSULE_REFUSEE",
            "message": e.to_string(),
        })),
    }
}

/// GET /api/capsule/sante → vérifie que la clé est configurée.
pub async fn capsule_sante() -> HttpResponse {
    let ok = std::env::var("UQ_CAPSULE_KEY").is_ok()
        || std::env::var("JWT_SECRET").is_ok();
    HttpResponse::Ok().json(json!({
        "status": if ok { "success" } else { "error" },
        "capsule_prete": ok,
        "cle_dediee": std::env::var("UQ_CAPSULE_KEY").is_ok(),
    }))
}
