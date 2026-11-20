//! Relais de signalement P2P ephemere — UQ Chat telephone-a-telephone.
//!
//! Regle absolue : RIEN n'est persiste. Les messages de signalement
//! (offres SDP / ICE WebRTC) vivent en memoire vive seulement, sont
//! transferes au premier passage du destinataire puis expires.
//! Le contenu des conversations ne passe JAMAIS par ce serveur.

use actix_web::{web, HttpResponse};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

lazy_static::lazy_static! {
    static ref RELAY: Mutex<HashMap<String, Vec<(Instant, serde_json::Value)>>> = Mutex::new(HashMap::new());
}

const TTL_MS: u64 = 60_000;      // 60 s : une offre SDP ne sert qu'une fois
const MAX_PAR_BOITE: usize = 20; // anti-flood
const TAILLE_MAX: usize = 16_384; // 16 Ko par signal — SDP tient largement

fn nettoyer() {
    if let Ok(mut map) = RELAY.lock() {
        let now = Instant::now();
        for msgs in map.values_mut() {
            msgs.retain(|(t, _)| now.duration_since(*t) < Duration::from_millis(TTL_MS));
        }
        map.retain(|_, msgs| !msgs.is_empty());
    }
}

/// POST /api/p2p/signal — { to, kind, payload } -> stocke pour `to`.
/// `kind` : "offer" | "answer" | "ice" | "bye" | "pair".
pub async fn signal_post(body: web::Json<serde_json::Value>) -> HttpResponse {
    let to = body.get("to").and_then(|v| v.as_str()).unwrap_or("").trim();
    let kind = body.get("kind").and_then(|v| v.as_str()).unwrap_or("").trim();
    let payload = body.get("payload").cloned().unwrap_or(json!(null));

    if to.is_empty() || to.len() > 128 || kind.is_empty() || kind.len() > 16 {
        return HttpResponse::BadRequest().json(json!({ "status": "bad_request" }));
    }
    let payload_str = payload.to_string();
    if payload_str.len() > TAILLE_MAX {
        return HttpResponse::PayloadTooLarge().json(json!({ "status": "too_large" }));
    }

    nettoyer();
    let now = Instant::now();
    if let Ok(mut map) = RELAY.lock() {
        let box_ = map.entry(to.to_string()).or_default();
        if box_.len() >= MAX_PAR_BOITE {
            // le plus vieux saute — jamais de blocage
            box_.remove(0);
        }
        box_.push((now, json!({ "kind": kind, "payload": payload, "at": now.elapsed().as_millis() as u64 })));
    }
    HttpResponse::Ok().json(json!({ "status": "queued" }))
}

/// GET /api/p2p/signal/{me} — draine les signaux destines a `me`.
/// Lecture unique : une fois remis, ils sont effaces (le serveur ne garde rien).
pub async fn signal_get(path: web::Path<String>) -> HttpResponse {
    let me = path.into_inner();
    if me.is_empty() || me.len() > 128 {
        return HttpResponse::BadRequest().json(json!({ "status": "bad_request" }));
    }
    nettoyer();
    let msgs = RELAY.lock().ok().and_then(|mut map| map.remove(&me));
    HttpResponse::Ok().json(json!({ "status": "ok", "messages": msgs.unwrap_or_default().into_iter().map(|(_, v)| v).collect::<Vec<_>>() }))
}

/// GET /api/p2p/health — le relais est vivant (pour UptimeRobot).
pub async fn health() -> HttpResponse {
    HttpResponse::Ok().json(json!({ "status": "ok", "mode": "ephemeral", "storage": "none" }))
}
