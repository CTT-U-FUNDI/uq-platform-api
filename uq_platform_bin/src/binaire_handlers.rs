//! Handlers API pour la protection des binaires propriétaires.
//! Routes : /api/binaire/*
//! Auth : signature Ed25519 du owner UQ-00000001.

use actix_web::{web, HttpRequest, HttpResponse};
use serde_json::json;
use std::sync::OnceLock;

use crate::binaire_protector::{BinaireStore, DemandeDecrypt, signature_owner_valide};

static STORE: OnceLock<BinaireStore> = OnceLock::new();
fn store() -> &'static BinaireStore {
    STORE.get_or_init(|| BinaireStore::new("data/binaires"))
}

fn ok(v: serde_json::Value) -> HttpResponse {
    HttpResponse::Ok().json(v)
}
fn err(code: u16, msg: &str) -> HttpResponse {
    HttpResponse::build(actix_web::http::StatusCode::from_u16(code).unwrap())
        .json(json!({"status":"error","message":msg}))
}

/// GET /api/binaire/liste — liste les binaires protégés (owner only)
pub async fn lister_binaires(req: HttpRequest) -> HttpResponse {
    // Vérifier owner via JWT
    let claims = match crate::auth_guard::exiger_owner(&req) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let _ = claims;
    ok(json!({"status":"success","binaires":store().lister()}))
}

/// POST /api/binaire/definir — définit une clé de chiffrement (owner only)
pub async fn definir_cle(
    req: HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let _claims = match crate::auth_guard::exiger_owner(&req) {
        Ok(c) => c,
        Err(r) => return r,
    };

    let id = body.get("id").and_then(|v| v.as_str()).unwrap_or("");
    let nom = body.get("nom").and_then(|v| v.as_str()).unwrap_or("Binaire");
    let cle = body.get("cle").and_then(|v| v.as_str()).unwrap_or("");

    if id.is_empty() {
        return err(400, "ID binaire requis");
    }

    match store().definir_cle(id, nom, cle) {
        Ok(()) => ok(json!({"status":"success","message":"Clé définie"})),
        Err(e) => err(400, &e),
    }
}

/// POST /api/binaire/decrypt — demande la clé de déchiffrement
/// Le loader envoie : {binaire_id, uq_owner, ts, signature}
/// UQ vérifie la signature owner → retourne la clé AES-256
pub async fn decrypt_binaire(body: web::Json<DemandeDecrypt>) -> HttpResponse {
    // Vérifier la signature owner
    // Note: on utilise la clé publique owner stockée dans la config
    let owner_vk = std::env::var("OWNER_PUBLIC_KEY").unwrap_or_default();

    if !signature_owner_valide(
        &body.binaire_id,
        &body.uq_owner,
        body.ts,
        &body.signature,
        &owner_vk,
    ) {
        return err(403, "Signature owner invalide");
    }

    // Vérifier que le binaire existe et a une clé
    if !store().existe(&body.binaire_id) {
        return err(404, "Binaire non protégé ou clé révoquée");
    }

    // Récupérer la clé
    match store().obtenir_cle(&body.binaire_id) {
        Some(cle) => {
            // Logger l'accès
            eprintln!("🔓 [binaire] Clé servie pour {} à {} (ts={})", body.binaire_id, body.uq_owner, body.ts);
            ok(json!({
                "status":"success",
                "cle":cle,
                "algorithme":"AES-256-GCM",
                "expire_dans":300, // 5 minutes pour utiliser la clé
            }))
        }
        None => err(404, "Clé non trouvée ou révoquée"),
    }
}

/// POST /api/binaire/revoquer — révoque une clé (owner only)
pub async fn revoquer_cle(
    req: HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let _claims = match crate::auth_guard::exiger_owner(&req) {
        Ok(c) => c,
        Err(r) => return r,
    };

    let id = body.get("id").and_then(|v| v.as_str()).unwrap_or("");
    match store().revoquer(id) {
        Ok(()) => ok(json!({"status":"success","message":"Clé révoquée"})),
        Err(e) => err(400, &e),
    }
}
