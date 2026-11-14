//! Contact public du site vitrine — POST /api/contact.
//! Endpoint non authentifié (visiteurs) : validation stricte + rate-limit IP,
//! puis envoi via le relais mail du VPS (même canal que UQ Mail sortant).

use actix_web::{web, HttpRequest, HttpResponse};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Mutex;

fn ok(v: serde_json::Value) -> HttpResponse { HttpResponse::Ok().json(v) }
fn err(code: u16, msg: &str) -> HttpResponse {
    HttpResponse::build(actix_web::http::StatusCode::from_u16(code).unwrap())
        .json(json!({"status":"error","message":msg}))
}

fn now_ts() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Rate-limit : 3 messages / IP / 10 minutes.
static CONTACT_RATE: Mutex<Option<(u64, HashMap<String, u32>)>> = Mutex::new(None);
const FENETRE_S: u64 = 600;
const MAX_PAR_FENETRE: u32 = 3;

fn rate_limit_ok(ip: &str) -> bool {
    let now = now_ts();
    let mut guard = match CONTACT_RATE.lock() { Ok(g) => g, Err(_) => return false };
    let slot = guard.get_or_insert_with(|| (now, HashMap::new()));
    if now.saturating_sub(slot.0) > FENETRE_S {
        slot.0 = now;
        slot.1.clear();
    }
    let n = slot.1.entry(ip.to_string()).or_insert(0);
    if *n >= MAX_PAR_FENETRE { return false; }
    *n += 1;
    true
}

fn propre(s: &str, max: usize) -> String {
    s.chars()
        .filter(|c| !c.is_control() || *c == ' ' || *c == '\n' || *c == '\t')
        .take(max)
        .collect::<String>()
        .trim()
        .to_string()
}

fn email_valide(e: &str) -> bool {
    let e = e.trim();
    e.len() <= 254
        && e.contains('@')
        && !e.contains(' ')
        && e.split('@').count() == 2
        && e.split('@').all(|p| !p.is_empty())
}

// ── POST /api/contact — {nom, email, message} ──
pub async fn contact_envoyer(req: HttpRequest, body: web::Json<serde_json::Value>) -> HttpResponse {
    let ip = req.peer_addr().map(|a| a.ip().to_string()).unwrap_or_else(|| "inconnu".into());
    if !rate_limit_ok(&ip) {
        return err(429, "Trop de messages. Réessayez dans quelques minutes.");
    }

    let nom = propre(body.get("nom").and_then(|v| v.as_str()).unwrap_or(""), 80);
    let email = propre(body.get("email").and_then(|v| v.as_str()).unwrap_or(""), 254);
    let message = propre(body.get("message").and_then(|v| v.as_str()).unwrap_or(""), 2000);

    if nom.len() < 2 {
        return err(400, "Veuillez indiquer votre nom.");
    }
    if !email_valide(&email) {
        return err(400, "Adresse email invalide.");
    }
    if message.len() < 10 {
        return err(400, "Votre message est trop court.");
    }

    // Anti-injection d'en-têtes : pas de retour chariot dans nom/email.
    if nom.contains('\n') || nom.contains('\r') || email.contains('\n') || email.contains('\r') {
        return err(400, "Champ invalide.");
    }

    let dest = std::env::var("UQ_CONTACT_TO").unwrap_or_else(|_| "contact@uqplatform.com".into());
    let from = std::env::var("UQ_CONTACT_FROM").unwrap_or_else(|_| "noreply@uqplatform.com".into());
    let sujet = format!("[Site] Contact — {}", nom);
    let corps = format!(
        "Message du site vitrine uqplatform.com\n\nNom : {}\nEmail : {}\nIP : {}\n\n---\n{}\n",
        nom, email, ip, message
    );

    match crate::mail_routes::envoyer_via_relais(&from, &dest, &sujet, &corps).await {
        Ok(()) => ok(json!({"status":"ok"})),
        Err(e) => {
            eprintln!("[contact] relais échoué: {}", e);
            err(503, "Service d'envoi indisponible. Écrivez-nous directement à contact@uqplatform.com.")
        }
    }
}
