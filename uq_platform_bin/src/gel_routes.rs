// ============================================================================
// gel_routes.rs — Routes admin/compte du système de gel gradué
// ============================================================================

use actix_web::{web, HttpRequest, HttpResponse};
use serde_json::json;

use crate::auth_guard::exiger_auth;
use crate::gel;

fn ok(value: serde_json::Value) -> HttpResponse {
    HttpResponse::Ok().json(json!({ "status": "success", "data": value }))
}

fn err(message: &str) -> HttpResponse {
    HttpResponse::BadRequest().json(json!({ "status": "error", "message": message }))
}

/// POST /api/gel/appliquer — body { uq, niveau (2-7), motif, duree_secs? }
/// Réservé au propriétaire (owner_guard posé au niveau du scope de route).
pub async fn appliquer(req: HttpRequest, body: web::Json<serde_json::Value>) -> HttpResponse {
    let _admin = match exiger_auth(&req) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let uq = body.get("uq").and_then(|v| v.as_str()).unwrap_or("");
    let niveau = body.get("niveau").and_then(|v| v.as_i64()).unwrap_or(0);
    let motif = body.get("motif").and_then(|v| v.as_str()).unwrap_or("");
    let duree = body.get("duree_secs").and_then(|v| v.as_i64()).unwrap_or(0);
    if uq.is_empty() {
        return err("uq obligatoire");
    }
    match gel::appliquer_gel(uq, niveau, motif, duree) {
        Ok(()) => ok(json!({ "applique": true, "uq": uq, "niveau": niveau })),
        Err(e) => err(&e),
    }
}

/// POST /api/gel/degeler — body { uq } : réactive complètement un compte.
pub async fn degeler(req: HttpRequest, body: web::Json<serde_json::Value>) -> HttpResponse {
    let _admin = match exiger_auth(&req) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let uq = body.get("uq").and_then(|v| v.as_str()).unwrap_or("");
    if uq.is_empty() {
        return err("uq obligatoire");
    }
    match gel::degeler(uq) {
        Ok(()) => ok(json!({ "degele": true, "uq": uq })),
        Err(e) => err(&e),
    }
}

/// POST /api/gel/escalader — body { uq, motif? } : passe au niveau supérieur
/// après récidive (2→3→…→7).
pub async fn escalader(req: HttpRequest, body: web::Json<serde_json::Value>) -> HttpResponse {
    let _admin = match exiger_auth(&req) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let uq = body.get("uq").and_then(|v| v.as_str()).unwrap_or("");
    let motif = body
        .get("motif")
        .and_then(|v| v.as_str())
        .unwrap_or("recidive");
    if uq.is_empty() {
        return err("uq obligatoire");
    }
    match gel::escalader(uq, motif) {
        Ok(niveau) => ok(json!({ "uq": uq, "nouveau_niveau": niveau })),
        Err(e) => err(&e),
    }
}

/// GET /api/gel/{uq} — état de gel d'un compte (admin ou soi-même).
pub async fn voir(req: HttpRequest, path: web::Path<String>) -> HttpResponse {
    let claims = match exiger_auth(&req) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let uq = path.into_inner();
    // Un non-admin ne peut consulter que son propre état.
    if claims.sub.trim().to_uppercase() != uq.to_uppercase()
        && claims.sub.trim().to_uppercase() != "UQ-00000001"
    {
        return HttpResponse::Forbidden().json(json!({
            "status": "error",
            "message": "Tu ne peux consulter que ton propre gel"
        }));
    }
    match gel::etat(&uq) {
        Ok(v) => ok(v),
        Err(e) => err(&e),
    }
}

/// GET /api/gel/peut/{uq}?action=... — test d'autorisation (admin/debug).
pub async fn peut(
    req: HttpRequest,
    path: web::Path<String>,
    query: web::Query<std::collections::HashMap<String, String>>,
) -> HttpResponse {
    let claims = match exiger_auth(&req) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let uq = path.into_inner();
    let action = query.get("action").map(String::as_str).unwrap_or("");
    if claims.sub.trim().to_uppercase() != uq.to_uppercase()
        && claims.sub.trim().to_uppercase() != "UQ-00000001"
    {
        return HttpResponse::Forbidden().json(json!({
            "status": "error",
            "message": "Action réservée"
        }));
    }
    match gel::peut_faire(&uq, action) {
        Ok(autorise) => ok(json!({ "uq": uq, "action": action, "autorise": autorise })),
        Err(e) => err(&e),
    }
}
