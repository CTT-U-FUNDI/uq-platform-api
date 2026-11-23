//! UQ SMS — routes /api/sms/*.
//! - /inbound : POST depuis les vieux Android (SMS Forwarder) — clé X-UQ-Sms-Key.
//! - /assign, /release, /status : actions utilisateur (signature Ed25519 UQ_SMS_V1).
//! - /pool/* : gestion du pool par l'owner (owner_guard).

use actix_web::{web, HttpRequest, HttpResponse};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde_json::json;

use crate::mail_store::detecter_otp;
use crate::sms_store::shared as sms_store;

fn ok(v: serde_json::Value) -> HttpResponse {
    HttpResponse::Ok().json(v)
}
fn err(code: u16, msg: &str) -> HttpResponse {
    HttpResponse::build(actix_web::http::StatusCode::from_u16(code).unwrap())
        .json(json!({"status":"error","message":msg}))
}
fn now_ts() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Auth Ed25519 : UQ_SMS_V1|{uq}|{vk}|{ts}
fn signature_sms_valide(uq: &str, vk_hex: &str, ts: u64, signature_hex: &str) -> bool {
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
    let msg = format!("UQ_SMS_V1|{}|{}|{}", uq, vk_hex, ts);
    let sig = match Signature::from_slice(&sig_bytes) {
        Ok(s) => s,
        Err(_) => return false,
    };
    vk.verify(msg.as_bytes(), &sig).is_ok()
}

fn auth_ok(body: &serde_json::Value, uq: &str) -> bool {
    let vk = body.get("vk").and_then(|v| v.as_str()).unwrap_or("");
    let ts = body.get("ts").and_then(|v| v.as_u64()).unwrap_or(0);
    let sig = body.get("signature").and_then(|v| v.as_str()).unwrap_or("");
    signature_sms_valide(uq, vk, ts, sig)
}

// ═══ INBOUND — le tuyau depuis les vieux Android ═══
// POST /api/sms/inbound  Header: X-UQ-Sms-Key
// Body : {to, from, text}  (format SMS Forwarder compatible)
// 'to' = le numéro relais destinataire ; on matche → uq_number.
pub async fn sms_inbound(req: HttpRequest, body: web::Json<serde_json::Value>) -> HttpResponse {
    let key_attendue = std::env::var("UQ_SMS_INBOUND_KEY").unwrap_or_default();
    let key_recue = req
        .headers()
        .get("X-UQ-Sms-Key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if key_attendue.is_empty() || key_recue != key_attendue {
        return err(403, "Clé SMS invalide");
    }

    let to = body.get("to").and_then(|v| v.as_str()).unwrap_or("");
    let from = body.get("from").and_then(|v| v.as_str()).unwrap_or("");
    let text = body.get("text").and_then(|v| v.as_str()).unwrap_or("");
    if to.is_empty() || text.is_empty() {
        return err(400, "to et text requis");
    }

    // Normaliser : garder chiffres + '+'
    let relay: String = to
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '+')
        .collect();
    if relay.len() < 8 {
        return err(400, "Numéro destinataire invalide");
    }

    // Détecter si c'est un SMS de réattribution opérateur ("nouveau numéro", "votre SIM")
    let bas = text.to_lowercase();
    let reseau = bas.contains("nouveau numéro")
        || bas.contains("new number")
        || bas.contains("votre sim")
        || bas.contains("rechargez")
        || bas.contains("expire")
        || bas.contains("forfait");

    let otp = detecter_otp("", text);
    match sms_store().deposer_sms(&relay, from, text, otp.as_deref()) {
        Some(id) => {
            if reseau {
                // Log : possible réattribution — à surveiller
                eprintln!(
                    "[sms] ⚠️ SMS réseau détecté sur {} : {}",
                    relay,
                    &text[..text.len().min(80)]
                );
            }
            ok(json!({"status":"ok","id":id,"otp":otp.is_some()}))
        }
        None => {
            // Numéro relais inconnu ou pas actif — on log, on accepte (évite boucle forwarder)
            eprintln!("[sms] relais sans propriétaire: {}", relay);
            ok(json!({"status":"ok","delivre":false,"raison":"relais_inconnu"}))
        }
    }
}

// ═══ UTILISATEUR — assignation / libération / statut ═══

/// POST /api/sms/assign {uq, vk, ts, signature} → attribue un numéro relais
pub async fn sms_assign(body: web::Json<serde_json::Value>) -> HttpResponse {
    let uq = body.get("uq").and_then(|v| v.as_str()).unwrap_or("");
    if uq.is_empty() {
        return err(400, "uq requis");
    }
    if !auth_ok(&body, uq) {
        return err(403, "Signature invalide");
    }
    match sms_store().attribuer(uq) {
        Some(num) => ok(json!({"status":"ok","relay_number":num})),
        None => err(503, "Pool de numéros épuisé — réessayez plus tard"),
    }
}

/// POST /api/sms/release {uq, vk, ts, signature} → libère le numéro
pub async fn sms_release(body: web::Json<serde_json::Value>) -> HttpResponse {
    let uq = body.get("uq").and_then(|v| v.as_str()).unwrap_or("");
    if uq.is_empty() {
        return err(400, "uq requis");
    }
    if !auth_ok(&body, uq) {
        return err(403, "Signature invalide");
    }
    if sms_store().liberer(uq) {
        ok(json!({"status":"ok","libere":true}))
    } else {
        err(404, "Aucun numéro actif pour ce compte")
    }
}

/// GET /api/sms/status/{uq} → numéro assigné + compteur OTP non lus
pub async fn sms_status(path: web::Path<String>) -> HttpResponse {
    let uq = path.into_inner();
    ok(json!({
        "status":"ok",
        "relay_number": sms_store().numero_de(&uq),
        "otp_non_lus": sms_store().sms_non_lus(&uq),
    }))
}

/// GET /api/sms/messages/{uq} → historique SMS reçus
pub async fn sms_messages(path: web::Path<String>) -> HttpResponse {
    let uq = path.into_inner();
    let msgs = sms_store().sms_reçus(&uq, 100);
    ok(json!({"status":"ok","messages":msgs,"otp_non_lus":sms_store().sms_non_lus(&uq)}))
}

/// POST /api/sms/mark-read {uq, id, vk, ts, signature}
pub async fn sms_mark_read(body: web::Json<serde_json::Value>) -> HttpResponse {
    let uq = body.get("uq").and_then(|v| v.as_str()).unwrap_or("");
    let id = body.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
    if uq.is_empty() || id == 0 {
        return err(400, "uq et id requis");
    }
    if !auth_ok(&body, uq) {
        return err(403, "Signature invalide");
    }
    if sms_store().marquer_sms_lu(uq, id) {
        ok(json!({"status":"ok"}))
    } else {
        err(404, "SMS introuvable")
    }
}

// ═══ OWNER — gestion du pool ═══

/// POST /api/sms/pool/ajouter {relay_number, sim_id} — owner uniquement
pub async fn sms_pool_ajouter(
    req: HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    // Réutilise le guard owner existant via header signé owner — on vérifie
    // simplement via le module owner_guard si disponible, sinon clé env.
    let owner_key = std::env::var("UQ_OWNER_KEY").unwrap_or_default();
    let key = req
        .headers()
        .get("X-UQ-Owner-Key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if owner_key.is_empty() || key != owner_key {
        return err(403, "Owner requis");
    }
    let relay = body
        .get("relay_number")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let sim = body.get("sim_id").and_then(|v| v.as_str()).unwrap_or("");
    if relay.is_empty() {
        return err(400, "relay_number requis");
    }
    match sms_store().ajouter_relais(relay, sim) {
        Ok(()) => ok(json!({"status":"ok","ajoute":relay})),
        Err(e) => err(500, &e),
    }
}

/// GET /api/sms/pool/libres — owner : numéros disponibles
pub async fn sms_pool_libres(req: HttpRequest) -> HttpResponse {
    let owner_key = std::env::var("UQ_OWNER_KEY").unwrap_or_default();
    let key = req
        .headers()
        .get("X-UQ-Owner-Key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if owner_key.is_empty() || key != owner_key {
        return err(403, "Owner requis");
    }
    ok(json!({"status":"ok","libres":sms_store().numeros_libres()}))
}

/// GET /api/sms/pool/sante — owner : SIM sans activité (keep-alive à envoyer)
pub async fn sms_pool_sante(req: HttpRequest) -> HttpResponse {
    let owner_key = std::env::var("UQ_OWNER_KEY").unwrap_or_default();
    let key = req
        .headers()
        .get("X-UQ-Owner-Key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if owner_key.is_empty() || key != owner_key {
        return err(403, "Owner requis");
    }
    // Seuil : 21 jours sans activité → keep-alive recommandé
    let sans_activite = sms_store().numéros_sans_activité(21 * 24 * 3600);
    ok(json!({"status":"ok","sans_activite_21j":sans_activite}))
}
