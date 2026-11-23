//! Handlers 2FA — comptes UQ, wallets liés par clé publique Ed25519.
//! Secret jamais exposé : seuls les sous-secrets wallet (dérivés) circulent.

use actix_web::{web, HttpRequest, HttpResponse};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::Deserialize;
use serde_json::json;

use crate::auth_guard::exiger_soi_ou_owner;
use crate::twofa_store::TwoFaStore;

fn store() -> &'static TwoFaStore {
    crate::twofa_store::shared()
}

fn ok(v: serde_json::Value) -> HttpResponse {
    HttpResponse::Ok().json(v)
}
fn err(code: u16, msg: &str) -> HttpResponse {
    HttpResponse::build(actix_web::http::StatusCode::from_u16(code).unwrap())
        .json(json!({"status":"error","message":msg}))
}

#[derive(Deserialize)]
pub struct UqBody {
    pub uq: String,
}
#[derive(Deserialize)]
pub struct VkBody {
    pub uq: String,
    pub vk: String,
}
#[derive(Deserialize)]
pub struct CodeBody {
    pub uq: String,
    pub vk: String,
    pub code: String,
}
#[derive(Deserialize)]
pub struct VerifyBody {
    pub uq: String,
    pub vk: String,
    pub code: String,
    pub ts: u64,
    pub signature: String,
}

/// Message canonique signé par le wallet pour les opérations 2FA.
fn message_2fa(uq: &str, vk: &str, ts: u64) -> String {
    format!("UQ_2FA_V1|{}|{}|{}", uq, vk, ts)
}

/// Vérifie la signature Ed25519 du wallet (vk = clé publique enregistrée).
fn signature_valide(uq: &str, vk_hex: &str, ts: u64, signature_hex: &str) -> bool {
    if ts.abs_diff(now_ts()) > 300 {
        return false; // anti-rejeu : 5 min max
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
    let msg = message_2fa(uq, vk_hex, ts);
    let sig = Signature::from_slice(&sig_bytes).unwrap();
    vk.verify(msg.as_bytes(), &sig).is_ok()
}

fn now_ts() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Vrai si la requête porte un pass humain valide (header X-Human-Pass).
/// Les routes 2FA mutantes l'exigent : un bot ne peut ni enrôler, ni
/// pivoter, ni lier un appareil sans avoir résolu la scène/Turnstile.
fn pass_humain_ok(req: &HttpRequest) -> bool {
    let token = req
        .headers()
        .get("X-Human-Pass")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    crate::verify_routes::preuve_humaine_valide(token)
}

fn err_pass() -> HttpResponse {
    err(403, "PREUVE_HUMAINE_REQUISE: resolvez /api/human/begin puis /api/human/verify, header X-Human-Pass")
}

// ---------- POST /api/2fa/enroll ----------
pub async fn twofa_enroll(req: HttpRequest, body: web::Json<UqBody>) -> HttpResponse {
    if !pass_humain_ok(&req) {
        return err_pass();
    }
    let claims = match exiger_soi_ou_owner(&req, &body.uq) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let _ = claims;
    let r = store().enroll(&body.uq);
    crate::audit_log::record_security_event("TWOFA_ENROLLED", &body.uq, "secret_compte_genere");
    ok(r)
}

// ---------- POST /api/2fa/rotate ----------
pub async fn twofa_rotate(req: HttpRequest, body: web::Json<UqBody>) -> HttpResponse {
    if !pass_humain_ok(&req) {
        return err_pass();
    }
    if exiger_soi_ou_owner(&req, &body.uq).is_err() {
        return err(403, "Accès refusé");
    }
    let r = store().rotate(&body.uq);
    crate::audit_log::record_security_event("TWOFA_ROTATED", &body.uq, "secret_rotation");
    ok(r)
}

// ---------- GET /api/2fa/status?uq= ----------
pub async fn twofa_status(req: HttpRequest) -> HttpResponse {
    let uq = req
        .query_string()
        .split('&')
        .find_map(|p| p.strip_prefix("uq=").map(|s| s.to_string()))
        .unwrap_or_default();
    if uq.is_empty() {
        return err(400, "uq requis");
    }
    // public mais minimal : juste actif + nombre de wallets
    let s = store().statut(&uq);
    ok(json!({"uq": uq, "statut": s}))
}

// ---------- POST /api/2fa/link-wallet ----------
pub async fn twofa_link_wallet(req: HttpRequest, body: web::Json<VkBody>) -> HttpResponse {
    if !pass_humain_ok(&req) {
        return err_pass();
    }
    if exiger_soi_ou_owner(&req, &body.uq).is_err() {
        return err(403, "Accès refusé");
    }
    let r = store().lier_wallet(&body.uq, &body.vk);
    // vk = clé publique (non sensible) — tronquée dans le détail
    let vk_short: String = body.vk.chars().take(16).collect();
    crate::audit_log::record_security_event("DEVICE_LINKED", &body.uq, &format!("vk:{}", vk_short));
    ok(r)
}

// ---------- POST /api/2fa/code ----------
// L'app UQ affiche le code du wallet (JWT requis).
pub async fn twofa_code(req: HttpRequest, body: web::Json<VkBody>) -> HttpResponse {
    if exiger_soi_ou_owner(&req, &body.uq).is_err() {
        return err(403, "Accès refusé");
    }
    match store().code_pour_wallet(&body.uq, &body.vk) {
        Some(code) => ok(json!({"status":"success","code":code,"expires_in":30})),
        None => err(404, "Wallet non lié ou 2FA inactive"),
    }
}

// ---------- POST /api/2fa/verify ----------
// Wallet : prouve son vk par signature Ed25519 → vérifie TOTP/backup.
pub async fn twofa_verify(body: web::Json<VerifyBody>) -> HttpResponse {
    if !signature_valide(&body.uq, &body.vk, body.ts, &body.signature) {
        return err(401, "Signature wallet invalide");
    }
    if store().verifier(&body.uq, &body.vk, &body.code) {
        ok(json!({"status":"success","verified":true}))
    } else {
        err(403, "Code 2FA incorrect")
    }
}

// ---------- GET /api/2fa/secret-wallet?uq=&vk=&ts=&sig= ----------
// Le wallet récupère SON sous-secret pour vérif offline.
// Le sous-secret seul ne compromet pas le secret compte.
pub async fn twofa_secret_wallet(req: HttpRequest) -> HttpResponse {
    let q = req.query_string();
    let get = |k: &str| -> String {
        q.split('&')
            .find_map(|p| p.strip_prefix(&format!("{}=", k)).map(|s| s.to_string()))
            .unwrap_or_default()
    };
    let (uq, vk, ts_s, sig) = (get("uq"), get("vk"), get("ts"), get("sig"));
    let ts: u64 = ts_s.parse().unwrap_or(0);
    if uq.is_empty() || vk.is_empty() || sig.is_empty() {
        return err(400, "Paramètres manquants");
    }
    if !signature_valide(&uq, &vk, ts, &sig) {
        return err(401, "Signature wallet invalide");
    }
    match store().secret_pour_wallet(&uq, &vk) {
        Some(secret) => ok(json!({"status":"success","secret":secret,"algo":"TOTP-SHA1-30s-6d"})),
        None => err(404, "Wallet non lié ou 2FA inactive"),
    }
}

// Récupère la clé publique du compte BORB auprès du nœud (canal admin).
// GET {BORB_NODE_URL}/compte/{uq}/cle avec X-Admin-Token.
async fn cle_compte_borb(uq: &str) -> Option<VerifyingKey> {
    let base =
        std::env::var("BORB_NODE_URL").unwrap_or_else(|_| "http://127.0.0.1:8787".to_string());
    let token = std::env::var("BORB_ADMIN_TOKEN").unwrap_or_default();
    if token.is_empty() {
        return None;
    }
    let url = format!("{}/compte/{}/cle", base.trim_end_matches('/'), uq);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .ok()?;
    let v: serde_json::Value = client
        .get(&url)
        .header("X-Admin-Token", token)
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    let hexs = v.get("cle_publique")?.as_str()?;
    let bytes = hex::decode(hexs).ok()?;
    let arr: [u8; 32] = bytes.as_slice().try_into().ok()?;
    VerifyingKey::from_bytes(&arr).ok()
}

// ---------- POST /api/2fa/device/pair ----------
// Appairage appareil hors-ligne : double preuve exigée.
//   1) signature du DEVICE (vk+ts+signature, domaine UQ_2FA_V1) → prouve la clé
//   2) signature du COMPTE (acct_vk+acct_ts+acct_sig, domaine UQ_PAIR_V1)
//      vérifiée contre public_key de uq_index → autorise le pairage.
// Body : {uq, vk, ts, signature, acct_ts, acct_sig}
//   (acct_vk implicite = clé du compte connecté — on la prend en base)
pub async fn twofa_device_pair(
    req: HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    if !pass_humain_ok(&req) {
        return err_pass();
    }
    let uq = body.get("uq").and_then(|v| v.as_str()).unwrap_or("");
    let vk = body.get("vk").and_then(|v| v.as_str()).unwrap_or("");
    let ts = body.get("ts").and_then(|v| v.as_u64()).unwrap_or(0);
    let sig = body.get("signature").and_then(|v| v.as_str()).unwrap_or("");
    let acct_ts = body.get("acct_ts").and_then(|v| v.as_u64()).unwrap_or(0);
    let acct_sig = body.get("acct_sig").and_then(|v| v.as_str()).unwrap_or("");
    if uq.is_empty() || vk.is_empty() || sig.is_empty() || acct_sig.is_empty() {
        return err(
            400,
            "Paramètres manquants (uq, vk, ts, signature, acct_ts, acct_sig)",
        );
    }
    // 1) Le device prouve sa clé
    if !signature_valide(uq, vk, ts, sig) {
        return err(401, "Signature device invalide");
    }
    // 2) Le compte autorise : signature UQ_PAIR_V1|uq|vk|ts par la clé compte.
    //    La clé compte réelle vit sur le nœud BORB (cles_annuaire, posée à
    //    /compte/ouvrir) — uq_index ne stocke qu'un placeholder à zéro.
    let Some(vk_compte) = cle_compte_borb(uq).await else {
        return err(404, "Cle compte introuvable — le wallet BORB doit d'abord ouvrir le compte (/compte/ouvrir)");
    };
    if acct_ts.abs_diff(now_ts()) > 300 {
        return err(401, "Signature compte expirée");
    }
    let msg_pair = format!("UQ_PAIR_V1|{}|{}|{}", uq, vk, ts);
    let Ok(sig_compte) = hex::decode(acct_sig).map(|b| Signature::from_slice(&b)) else {
        return err(400, "Signature compte illisible");
    };
    let Ok(sig_compte) = sig_compte else {
        return err(400, "Signature compte invalide");
    };
    if vk_compte.verify(msg_pair.as_bytes(), &sig_compte).is_err() {
        return err(401, "Signature compte invalide — autorisation refusée");
    }
    // 3) Lier le device + audit
    let r = store().lier_wallet(uq, vk);
    let vk_short: String = vk.chars().take(16).collect();
    crate::audit_log::record_security_event("DEVICE_PAIRED", uq, &format!("vk:{}", vk_short));
    ok(r)
}

// ---------- GET /api/audit/status ----------
// Statut public minimal des deux chaînes (comptes + événements sécurité).
pub async fn audit_status() -> HttpResponse {
    let (ok_c, n_c, _) = crate::audit_log::shared().verifier();
    let (ok_s, n_s, _) = crate::audit_log::security().verifier();
    ok(json!({
        "status":"ok",
        "chaines": {
            "comptes": {"ok": ok_c, "entrees": n_c},
            "securite": {"ok": ok_s, "entrees": n_s},
        }
    }))
}

// ---------- GET /api/audit/events/{uq} ----------
// Historique des événements sécurité d'un compte (auth requis).
pub async fn audit_events(req: HttpRequest, path: web::Path<String>) -> HttpResponse {
    let uq = path.into_inner();
    if exiger_soi_ou_owner(&req, &uq).is_err() {
        return err(403, "Accès refusé");
    }
    let events = crate::audit_log::security().evenements_de(&uq);
    // On n'expose que kind/ts/detail — jamais prev/hash (inutile côté client)
    let liste: Vec<serde_json::Value> = events
        .iter()
        .map(|e| {
            json!({
                "seq": e.seq, "ts": e.ts, "kind": e.kind, "detail": e.detail,
            })
        })
        .collect();
    ok(json!({"status":"ok","uq":uq,"evenements":liste,"total":liste.len()}))
}
