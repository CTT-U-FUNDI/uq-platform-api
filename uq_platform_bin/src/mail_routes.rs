//! UQ Mail — routes /api/mail/*.
//! - Lecture/envoi authentifiés par signature Ed25519 (domaine UQ_MAIL_V1).
//! - Entrée : POST /api/mail/in appelé par le script inbound du VPS
//!   (Postfix pipe → HTTPS → ici), protégé par X-UQ-Mail-Key.
//! - Sortie : POST /api/mail/send → relais HTTP du VPS (UQ_MAIL_RELAY_URL)
//!   → sendmail local. Aucun port SMTP exposé à UQ Platform.

use actix_web::{web, HttpRequest, HttpResponse};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde_json::json;

use crate::mail_store::{shared as mail_store, detecter_otp};
use crate::uq_index::UqIndex;

fn index() -> std::sync::Arc<UqIndex> {
    UqIndex::shared("uq_index.db")
}

fn ok(v: serde_json::Value) -> HttpResponse { HttpResponse::Ok().json(v) }
fn err(code: u16, msg: &str) -> HttpResponse {
    HttpResponse::build(actix_web::http::StatusCode::from_u16(code).unwrap())
        .json(json!({"status":"error","message":msg}))
}
fn now_ts() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Domaine réel de la messagerie (ex: mail.uq-platform.com).
fn mail_domain() -> String {
    std::env::var("UQ_MAIL_DOMAIN").unwrap_or_else(|_| "localhost".into())
}

/// Auth Ed25519 : UQ_MAIL_V1|{uq}|{vk}|{ts}
fn signature_mail_valide(uq: &str, vk_hex: &str, ts: u64, signature_hex: &str) -> bool {
    if ts.abs_diff(now_ts()) > 300 { return false; }
    let vk_bytes = match hex::decode(vk_hex) { Ok(b) if b.len() == 32 => b, _ => return false };
    let sig_bytes = match hex::decode(signature_hex) { Ok(b) if b.len() == 64 => b, _ => return false };
    let vk_arr: [u8; 32] = vk_bytes.try_into().unwrap();
    let vk = match VerifyingKey::from_bytes(&vk_arr) { Ok(k) => k, Err(_) => return false };
    let msg = format!("UQ_MAIL_V1|{}|{}|{}", uq, vk_hex, ts);
    let sig = match Signature::from_slice(&sig_bytes) { Ok(s) => s, Err(_) => return false };
    vk.verify(msg.as_bytes(), &sig).is_ok()
}

/// Extrait vk/ts/signature du body et vérifie l'auth mail.
fn auth_ok(body: &serde_json::Value, uq: &str) -> bool {
    let vk = body.get("vk").and_then(|v| v.as_str()).unwrap_or("");
    let ts = body.get("ts").and_then(|v| v.as_u64()).unwrap_or(0);
    let sig = body.get("signature").and_then(|v| v.as_str()).unwrap_or("");
    signature_mail_valide(uq, vk, ts, sig)
}

/// Adresse réelle d'un compte : localpart(uq_email)@UQ_MAIL_DOMAIN.
pub fn adresse_reelle(uq: &str) -> Option<String> {
    let compte = index().get_account(uq)?;
    let email_uq = compte.get("uq_email")?.as_str()?;
    let local = email_uq.split('@').next()?;
    Some(format!("{}@{}", local, mail_domain()))
}

// ── GET /api/mail/adresse/{uq} — adresse réelle du compte ──
pub async fn mail_adresse(path: web::Path<String>) -> HttpResponse {
    let uq = path.into_inner();
    match adresse_reelle(&uq) {
        Some(a) => ok(json!({"status":"ok","adresse":a,"domaine":mail_domain()})),
        None => err(404, "Compte introuvable"),
    }
}

// ── GET /api/mail/boite/{uq}/{dossier} — liste inbox/sent/drafts ──
pub async fn mail_lister(path: web::Path<(String, String)>) -> HttpResponse {
    let (uq, dossier) = path.into_inner();
    let dossier = match dossier.as_str() {
        "sent" | "drafts" | "spam" => dossier,
        _ => "inbox".to_string(),
    };
    let msgs = mail_store().lister(&uq, &dossier, 100);
    // Version liste : sans le corps complet pour rester léger
    let liste: Vec<serde_json::Value> = msgs.iter().map(|m| json!({
        "id": m.id, "from_addr": m.from_addr, "from_name": m.from_name,
        "to_addr": m.to_addr, "subject": m.subject, "date": m.date,
        "is_read": m.is_read, "otp_code": m.otp_code,
        "apercu": m.body_text.chars().take(140).collect::<String>(),
    })).collect();
    ok(json!({"status":"ok","dossier":dossier,"messages":liste,"non_lus":mail_store().non_lus(&uq)}))
}

// ── GET /api/mail/message/{uq}/{id} — lit un message (marque lu) ──
pub async fn mail_lire(path: web::Path<(String, i64)>) -> HttpResponse {
    let (uq, id) = path.into_inner();
    match mail_store().lire(&uq, id) {
        Some(m) => {
            mail_store().marquer_lu(&uq, id);
            ok(json!({"status":"ok","message":m}))
        }
        None => err(404, "Message introuvable"),
    }
}

// ── GET /api/mail/non-lus/{uq} — badge notification ──
pub async fn mail_non_lus(path: web::Path<String>) -> HttpResponse {
    let uq = path.into_inner();
    ok(json!({"status":"ok","non_lus":mail_store().non_lus(&uq)}))
}

// ── POST /api/mail/mark-read — {uq, id, vk, ts, signature} ──
pub async fn mail_mark_read(body: web::Json<serde_json::Value>) -> HttpResponse {
    let uq = body.get("uq").and_then(|v| v.as_str()).unwrap_or("");
    let id = body.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
    if uq.is_empty() || id == 0 { return err(400, "uq et id requis"); }
    if !auth_ok(&body, uq) { return err(403, "Signature invalide"); }
    if mail_store().marquer_lu(uq, id) {
        ok(json!({"status":"ok"}))
    } else {
        err(404, "Message introuvable")
    }
}

/// Envoie un mail via le relais du VPS (HTTPS → sendmail).
/// UQ_MAIL_RELAY_URL + UQ_MAIL_RELAY_KEY dans .env.
pub(crate) async fn envoyer_via_relais(from: &str, to: &str, subject: &str, body_text: &str) -> Result<(), String> {
    let url = std::env::var("UQ_MAIL_RELAY_URL")
        .map_err(|_| "UQ_MAIL_RELAY_URL non configuré".to_string())?;
    let key = std::env::var("UQ_MAIL_RELAY_KEY").unwrap_or_default();
    let payload = json!({
        "from": from, "to": to, "subject": subject, "body": body_text,
    });
    let client = reqwest::Client::new();
    let mut req = client.post(&url).json(&payload);
    if !key.is_empty() { req = req.bearer_auth(&key); }
    let resp = req.send().await.map_err(|e| format!("relais injoignable: {}", e))?;
    if resp.status().is_success() { Ok(()) } else {
        Err(format!("relais HTTP {}", resp.status()))
    }
}

// ── POST /api/mail/send — {uq, to, subject, body, vk, ts, signature} ──
pub async fn mail_send(body: web::Json<serde_json::Value>) -> HttpResponse {
    let uq = body.get("uq").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let to = body.get("to").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let subject = body.get("subject").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let body_text = body.get("body").and_then(|v| v.as_str()).unwrap_or("").to_string();
    if uq.is_empty() || to.is_empty() { return err(400, "uq et to requis"); }
    if !auth_ok(&body, &uq) { return err(403, "Signature invalide"); }

    // Validation basique anti-injection d'en-têtes
    if to.contains('\n') || to.contains('\r') || subject.contains('\n') || subject.contains('\r') {
        return err(400, "Destinataire/sujet invalide");
    }

    let Some(from) = adresse_reelle(&uq) else { return err(404, "Compte introuvable"); };

    // Envoi via relais VPS
    match envoyer_via_relais(&from, &to, &subject, &body_text).await {
        Ok(()) => {
            let _ = mail_store().enregistrer_envoi(&uq, &from, &to, &subject, &body_text, "");
            ok(json!({"status":"ok","from":from}))
        }
        Err(e) => err(502, &format!("Envoi échoué: {}", e)),
    }
}

// ── POST /api/mail/in — entrée depuis Postfix (script inbound VPS) ──
// Headers : X-UQ-Mail-Key: <UQ_MAIL_INBOUND_KEY>
// Body JSON : {to, from, from_name, subject, body_text, body_html}
// 'to' = adresse réelle (prenom@domaine). On résout le localpart → compte UQ.
pub async fn mail_in(req: HttpRequest, body: web::Json<serde_json::Value>) -> HttpResponse {
    let key_attendue = std::env::var("UQ_MAIL_INBOUND_KEY").unwrap_or_default();
    let key_recue = req.headers().get("X-UQ-Mail-Key")
        .and_then(|v| v.to_str().ok()).unwrap_or("");
    if key_attendue.is_empty() || key_recue != key_attendue {
        return err(403, "Clé inbound invalide");
    }

    let to = body.get("to").and_then(|v| v.as_str()).unwrap_or("");
    let from_addr = body.get("from").and_then(|v| v.as_str()).unwrap_or("");
    let from_name = body.get("from_name").and_then(|v| v.as_str()).unwrap_or("");
    let subject = body.get("subject").and_then(|v| v.as_str()).unwrap_or("");
    let body_text = body.get("body_text").and_then(|v| v.as_str()).unwrap_or("");
    let body_html = body.get("body_html").and_then(|v| v.as_str()).unwrap_or("");

    let localpart = match to.split('@').next() {
        Some(l) if !l.is_empty() => l.to_lowercase(),
        _ => return err(400, "Destinataire invalide"),
    };

    // Résout localpart → uq_email (prenom@zando.uq) → uq_number
    let uq = match index().get_uq_by_email_localpart(&localpart) {
        Some(u) => u,
        None => {
            // Adresse inconnue : on log et on accepte (Postfix attend 200)
            eprintln!("[mail] destinataire inconnu: {}", to);
            return ok(json!({"status":"ok","delivre":false,"raison":"destinataire_inconnu"}));
        }
    };

    let otp = detecter_otp(subject, body_text);
    let dossier = if body.get("spam").and_then(|v| v.as_bool()).unwrap_or(false) { "spam" } else { "inbox" };

    match mail_store().deposer(&uq, from_addr, from_name, to, subject, body_text, body_html, otp.as_deref(), dossier) {
        Ok(id) => ok(json!({"status":"ok","delivre":true,"id":id,"uq":uq,"otp":otp.is_some()})),
        Err(e) => err(500, &format!("Stockage échoué: {}", e)),
    }
}
