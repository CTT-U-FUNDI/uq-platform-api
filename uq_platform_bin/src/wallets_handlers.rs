//! Handlers API pour la gestion multi-wallets.
//! Routes : /api/wallets/*
//! Max 10 wallets : 6 online + 4 offline par compte UQ.
//! Auth : signature Ed25519 du wallet (comme 2FA) — pas de JWT.

use actix_web::{web, HttpRequest, HttpResponse};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde_json::json;
use std::sync::OnceLock;

use crate::wallets_store::{WalletType, WalletsStore};

static STORE: OnceLock<WalletsStore> = OnceLock::new();
fn store() -> &'static WalletsStore {
    STORE.get_or_init(|| WalletsStore::new("data/wallets"))
}

fn ok(v: serde_json::Value) -> HttpResponse {
    HttpResponse::Ok().json(v)
}
fn err(code: u16, msg: &str) -> HttpResponse {
    HttpResponse::build(actix_web::http::StatusCode::from_u16(code).unwrap())
        .json(json!({"status":"error","message":msg}))
}

/// Vérifie la signature Ed25519 du wallet pour l'authentification.
/// Message : UQ_WALLET_V1|{uq}|{vk}|{ts}
fn signature_wallet_valide(uq: &str, vk_hex: &str, ts: u64, signature_hex: &str) -> bool {
    if ts.abs_diff(now_ts()) > 300 {
        return false;
    }
    let vk_bytes = match hex::decode(vk_hex) {
        Ok(b) if b.len() == 32 => b,
        _ => return false,
    };
    let sig_bytes = match hex::decode(signature_hex) {
        Ok(b) if b.len() == 64 => b,
        _ => return false,
    };
    let vk_arr: [u8; 32] = vk_bytes.try_into().unwrap();
    let vk = match VerifyingKey::from_bytes(&vk_arr) {
        Ok(k) => k,
        Err(_) => return false,
    };
    let msg = format!("UQ_WALLET_V1|{}|{}|{}", uq, vk_hex, ts);
    let sig = Signature::from_slice(&sig_bytes).unwrap();
    vk.verify(msg.as_bytes(), &sig).is_ok()
}

fn now_ts() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// GET /api/wallets/{uq} — liste tous les wallets d'un compte
pub async fn lister_wallets(path: web::Path<String>) -> HttpResponse {
    let uq = path.into_inner();
    ok(store().lister(&uq))
}

/// POST /api/wallets/{uq}/ajouter — ajoute un wallet (auth par signature)
pub async fn ajouter_wallet(
    body: web::Json<serde_json::Value>,
    path: web::Path<String>,
) -> HttpResponse {
    let uq = path.into_inner();
    let vk_hex = body.get("vk").and_then(|v| v.as_str()).unwrap_or("");
    let nom = body.get("nom").and_then(|v| v.as_str()).unwrap_or("Wallet");
    let r#type_str = body
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("online");
    let ts = body.get("ts").and_then(|v| v.as_u64()).unwrap_or(0);
    let signature = body.get("signature").and_then(|v| v.as_str()).unwrap_or("");

    // Auth par signature Ed25519
    if !signature_wallet_valide(&uq, vk_hex, ts, signature) {
        return err(403, "Signature invalide — authentification requise");
    }

    let r#type = match r#type_str {
        "offline" => WalletType::Offline,
        _ => WalletType::Online,
    };

    // Validation clé publique (64 hex chars)
    if vk_hex.len() != 64 || !vk_hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return err(400, "Clé publique invalide (64 hex chars requis)");
    }

    match store().ajouter(&uq, vk_hex, nom, r#type) {
        Ok(wallet) => ok(json!({
            "status": "success",
            "wallet": wallet,
            "message": "Wallet ajouté avec succès"
        })),
        Err(e) => err(400, &e),
    }
}

/// POST /api/wallets/{uq}/principal — définit le wallet principal
pub async fn definir_principal(
    body: web::Json<serde_json::Value>,
    path: web::Path<String>,
) -> HttpResponse {
    let uq = path.into_inner();
    let vk_hex = body.get("vk").and_then(|v| v.as_str()).unwrap_or("");
    let ts = body.get("ts").and_then(|v| v.as_u64()).unwrap_or(0);
    let signature = body.get("signature").and_then(|v| v.as_str()).unwrap_or("");

    if !signature_wallet_valide(&uq, vk_hex, ts, signature) {
        return err(403, "Signature invalide");
    }

    match store().definir_principal(&uq, vk_hex) {
        Ok(()) => ok(json!({"status": "success", "message": "Wallet principal défini"})),
        Err(e) => err(400, &e),
    }
}

/// POST /api/wallets/{uq}/statut — active/désactive un wallet
pub async fn maj_statut(
    body: web::Json<serde_json::Value>,
    path: web::Path<String>,
) -> HttpResponse {
    let uq = path.into_inner();
    let vk_hex = body.get("vk").and_then(|v| v.as_str()).unwrap_or("");
    let actif = body.get("actif").and_then(|v| v.as_bool()).unwrap_or(true);
    let ts = body.get("ts").and_then(|v| v.as_u64()).unwrap_or(0);
    let signature = body.get("signature").and_then(|v| v.as_str()).unwrap_or("");

    if !signature_wallet_valide(&uq, vk_hex, ts, signature) {
        return err(403, "Signature invalide");
    }

    match store().maj_statut(&uq, vk_hex, actif) {
        Ok(()) => ok(json!({"status": "success"})),
        Err(e) => err(400, &e),
    }
}

/// POST /api/wallets/{uq}/usb — lie une clé USB à un wallet offline
pub async fn lier_usb(body: web::Json<serde_json::Value>, path: web::Path<String>) -> HttpResponse {
    let uq = path.into_inner();
    let vk_hex = body.get("vk").and_then(|v| v.as_str()).unwrap_or("");
    let usb_hash = body.get("usb_hash").and_then(|v| v.as_str()).unwrap_or("");
    let ts = body.get("ts").and_then(|v| v.as_u64()).unwrap_or(0);
    let signature = body.get("signature").and_then(|v| v.as_str()).unwrap_or("");

    if !signature_wallet_valide(&uq, vk_hex, ts, signature) {
        return err(403, "Signature invalide");
    }

    match store().lier_usb(&uq, vk_hex, usb_hash) {
        Ok(()) => ok(json!({"status": "success", "message": "Clé USB liée"})),
        Err(e) => err(400, &e),
    }
}

/// GET /api/wallets/{uq}/code/{vk} — génère le code 2FA pour un wallet
pub async fn code_2fa_wallet(
    path: web::Path<(String, String)>,
    query: web::Query<std::collections::HashMap<String, String>>,
) -> HttpResponse {
    let (uq, vk) = path.into_inner();
    let ts = query
        .get("ts")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);
    let signature = query.get("signature").map(|s| s.as_str()).unwrap_or("");

    if !signature_wallet_valide(&uq, &vk, ts, signature) {
        return err(403, "Signature invalide");
    }

    match crate::twofa_store::shared().code_pour_wallet(&uq, &vk) {
        Some(code) => ok(json!({
            "status": "success",
            "code": code,
            "expire_dans": 30,
        })),
        None => err(404, "Wallet non lié au 2FA ou 2FA non activée"),
    }
}

/// POST /api/wallets/{uq}/verifier-2fa — vérifie le code 2FA d'un wallet
pub async fn verifier_2fa(body: web::Json<serde_json::Value>) -> HttpResponse {
    let uq = body.get("uq").and_then(|v| v.as_str()).unwrap_or("");
    let vk = body.get("vk").and_then(|v| v.as_str()).unwrap_or("");
    let code = body.get("code").and_then(|v| v.as_str()).unwrap_or("");
    let ts = body.get("ts").and_then(|v| v.as_u64()).unwrap_or(0);
    let signature = body.get("signature").and_then(|v| v.as_str()).unwrap_or("");

    if !signature_wallet_valide(uq, vk, ts, signature) {
        return err(403, "Signature invalide");
    }

    let valide = crate::twofa_store::shared().verifier(uq, vk, code);

    if valide {
        ok(json!({"status": "success", "valide": true}))
    } else {
        err(403, "Code 2FA incorrect")
    }
}
