//! Publications ephemeres (annonces, conférences, posts, statuts) du carnet public.
//!
//! Une publication classique disparait apres 7 jours, dans le flux comme dans
//! le canal Tendances du chat. Un statut (`kind: "status"`, publie depuis un
//! element du flux Zando) disparait apres 24 heures. Dans les deux cas les
//! likes et les vues deja recus restent dans les tables persistantes et
//! comptent pour le profil de l'auteur. L'auteur est identifie par son pseudo ;
//! le numero UQ reste interne et n'est jamais renvoye.

use actix_web::{web, HttpRequest, HttpResponse};
use serde_json::json;

use crate::database::Database;

const KINDS: [&str; 4] = ["annonce", "conference", "publication", "status"];

fn db() -> std::sync::Arc<Database> {
    Database::shared("uq_platform_db.json")
}

fn uq_du_lecteur(req: &HttpRequest, body: Option<&web::Json<serde_json::Value>>) -> Option<String> {
    let claims = crate::auth_guard::exiger_auth(req).ok()?;
    let demande = body
        .and_then(|payload| payload.get("uq").and_then(|value| value.as_str()))
        .unwrap_or("");
    if !demande.trim().is_empty()
        && !demande.trim().eq_ignore_ascii_case(claims.sub.trim())
        && !crate::auth_guard::est_owner(&claims)
    {
        return None;
    }
    Some(claims.sub)
}

/// GET /api/publications — liste publique (7 derniers jours).
pub async fn publication_list(req: HttpRequest) -> HttpResponse {
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(claims) => claims,
        Err(response) => return response,
    };
    let publications = db().lister_publications(&claims.sub);
    HttpResponse::Ok().json(json!({
        "status": "success",
        "publications": publications,
        "total": publications.len(),
        "ttl_seconds": crate::database::PUBLICATION_TTL_SECS,
    }))
}

/// POST /api/publications — { uq, kind, title, body?, price?, media?, zando_item_id? }
pub async fn publication_create(req: HttpRequest, body: web::Json<serde_json::Value>) -> HttpResponse {
    let lecteur = match uq_du_lecteur(&req, Some(&body)) {
        Some(lecteur) => lecteur,
        None => {
            return HttpResponse::Forbidden().json(json!({
                "status": "error",
                "message": "Acces refuse"
            }))
        }
    };
    let kind = body.get("kind").and_then(|value| value.as_str()).unwrap_or("publication");
    if !KINDS.contains(&kind) {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Type de publication invalide"
        }));
    }
    let title = body.get("title").and_then(|value| value.as_str()).unwrap_or("").trim();
    if title.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Titre requis"
        }));
    }
    let texte = body.get("body").and_then(|value| value.as_str()).unwrap_or("");
    let price = body.get("price").and_then(|value| value.as_f64());
    let media = body.get("media").and_then(|value| value.as_str()).unwrap_or("");
    if media.len() > crate::database::PUBLICATION_MEDIA_MAX_CHARS {
        return HttpResponse::PayloadTooLarge().json(json!({
            "status": "error",
            "message": "Media trop volumineux"
        }));
    }
    let lien_zando = body
        .get("zando_item_id")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    // Un statut ne fait que pointer vers un element du flux : le lien doit
    // exister, sinon la fiche serait inatteignable depuis la bulle.
    if !lien_zando.trim().is_empty() && db().get_shop_item_by_id(lien_zando.trim()).is_none() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Element Zando introuvable"
        }));
    }

    match db().creer_publication(&lecteur, kind, title, texte, price, media, lien_zando) {
        Ok(publication) => HttpResponse::Ok().json(json!({
            "status": "success",
            "publication": publication
        })),
        Err(error) => HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": error
        })),
    }
}

/// POST /api/publications/{id}/comment — { uq, content }
pub async fn publication_comment(req: HttpRequest, path: web::Path<String>, body: web::Json<serde_json::Value>) -> HttpResponse {
    let lecteur = match uq_du_lecteur(&req, Some(&body)) {
        Some(lecteur) => lecteur,
        None => {
            return HttpResponse::Forbidden().json(json!({
                "status": "error",
                "message": "Acces refuse"
            }))
        }
    };
    let texte = body.get("content").and_then(|value| value.as_str()).unwrap_or("").trim();
    if texte.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Commentaire vide"
        }));
    }
    match db().commenter_publication(&path.into_inner(), &lecteur, texte) {
        Ok(publication) => HttpResponse::Ok().json(json!({
            "status": "success",
            "publication": publication
        })),
        Err(error) => HttpResponse::NotFound().json(json!({
            "status": "error",
            "message": error
        })),
    }
}
