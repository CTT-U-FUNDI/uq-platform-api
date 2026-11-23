// ============================================================================
// contestation_routes.rs — Routes de la contestation publique des gels
// ============================================================================

use actix_web::{web, HttpRequest, HttpResponse};
use serde_json::json;

use crate::auth_guard::exiger_auth;
use crate::contestation;

fn ok(value: serde_json::Value) -> HttpResponse {
    HttpResponse::Ok().json(json!({ "status": "success", "data": value }))
}

fn err(message: &str) -> HttpResponse {
    HttpResponse::BadRequest().json(json!({ "status": "error", "message": message }))
}

/// POST /api/contestation/ouvrir — body { points } : le compte gelé dépose des
/// points pour ouvrir sa contestation publique.
pub async fn ouvrir(req: HttpRequest, body: web::Json<serde_json::Value>) -> HttpResponse {
    let claims = match exiger_auth(&req) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let uq = claims.sub.trim().to_uppercase();
    if let Err(e) = crate::gel::exiger(&uq, "contester") {
        return HttpResponse::Forbidden().json(json!({ "status": "error", "message": e }));
    }
    let points = body.get("points").and_then(|v| v.as_i64()).unwrap_or(0);
    match contestation::ouvrir(&uq, points) {
        Ok(id) => ok(json!({ "contestation_id": id })),
        Err(e) => err(&e),
    }
}

/// GET /api/contestation/mes — ma contestation active (ou null).
pub async fn mes(req: HttpRequest) -> HttpResponse {
    let claims = match exiger_auth(&req) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let uq = claims.sub.trim().to_uppercase();
    match contestation::ma_contestation(&uq) {
        Ok(Some(id)) => match contestation::detail(id) {
            Ok(d) => ok(d),
            Err(e) => err(&e),
        },
        Ok(None) => ok(serde_json::Value::Null),
        Err(e) => err(&e),
    }
}

/// GET /api/contestation — liste des contestations actives (public).
pub async fn liste() -> HttpResponse {
    match contestation::liste() {
        Ok(v) => ok(serde_json::json!({ "contestations": v })),
        Err(e) => err(&e),
    }
}

/// GET /api/contestation/{id} — détail public (commentaires, votes, ratio).
pub async fn voir(path: web::Path<i64>) -> HttpResponse {
    match contestation::detail(path.into_inner()) {
        Ok(d) => ok(d),
        Err(e) => err(&e),
    }
}

/// POST /api/contestation/{id}/commenter — body { texte }.
/// Coûte COUT_COMMENTAIRE_POINTS prélevés sur le fonds du fautif.
pub async fn commenter(
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let claims = match exiger_auth(&req) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let auteur = claims.sub.trim().to_uppercase();
    if let Err(e) = crate::gel::exiger(&auteur, "commenter_contestation") {
        return HttpResponse::Forbidden().json(json!({ "status": "error", "message": e }));
    }
    let texte = body.get("texte").and_then(|v| v.as_str()).unwrap_or("");
    let c = match contestation::par_id(path.into_inner()) {
        Ok(c) => c,
        Err(e) => return err(&e),
    };
    match contestation::commenter(&c, &auteur, texte) {
        Ok(()) => ok(json!({ "commente": true })),
        Err(e) => err(&e),
    }
}

/// POST /api/contestation/{id}/voter — body { vote: "pour"|"contre" }.
pub async fn voter(
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let claims = match exiger_auth(&req) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let auteur = claims.sub.trim().to_uppercase();
    if let Err(e) = crate::gel::exiger(&auteur, "voter_contestation") {
        return HttpResponse::Forbidden().json(json!({ "status": "error", "message": e }));
    }
    let vote = body.get("vote").and_then(|v| v.as_str()).unwrap_or("");
    let c = match contestation::par_id(path.into_inner()) {
        Ok(c) => c,
        Err(e) => return err(&e),
    };
    match contestation::voter(&c, &auteur, vote) {
        Ok(()) => ok(json!({ "vote_enregistre": true })),
        Err(e) => err(&e),
    }
}
