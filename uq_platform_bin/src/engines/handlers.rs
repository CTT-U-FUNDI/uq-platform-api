// ====================
// 🌐 HANDLERS — Routes du bus de moteurs
// ====================
//
// GET  /api/engines/etat                 santé, quota, compteurs
// GET  /api/engines/pairs                pairs UDP + appareils appairés
// POST /api/engines/gossip               ingest d'une Diffusion (pair ou
//                                        appareil connecté ; UDP a un
//                                        pump dédié, HTTP est le fallback)
// POST /api/engines/appareil             appaire un appareil à un UQ
// POST /api/engines/evenement            publie des events métier (synchro)
// GET  /api/engines/oracle               médianes collectives multi-pairs
// GET  /api/engines/reputation/{uq}      agrégat d'attestations
// GET  /api/engines/synchro/{uq}         events reçus des autres appareils

use std::sync::Arc;

use actix_web::{web, HttpRequest, HttpResponse};

use super::bus::{BusMoteurs, MOTEUR_SYNCHRO};
use super::gossip::Diffusion;

type Bus = web::Data<Arc<BusMoteurs>>;

pub async fn engines_etat(bus: Bus) -> HttpResponse {
    HttpResponse::Ok().json(bus.etat())
}

pub async fn engines_pairs(bus: Bus) -> HttpResponse {
    HttpResponse::Ok().json(bus.pairs())
}

pub async fn engines_gossip(bus: Bus, corps: web::Json<Diffusion>) -> HttpResponse {
    match bus.recevoir(corps.into_inner(), None) {
        Ok(rep) => HttpResponse::Ok().json(rep),
        Err(e) => HttpResponse::BadRequest().json(serde_json::json!({"status": "error", "message": e})),
    }
}

pub async fn engines_appareil(bus: Bus, req: HttpRequest, corps: web::Json<serde_json::Value>) -> HttpResponse {
    if let Err(r) = crate::auth_guard::exiger_auth(&req) {
        return r;
    }
    let uq = corps.get("uq").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let appareil = corps.get("appareil").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let addr = corps.get("addr").and_then(|v| v.as_str()).unwrap_or("").to_string();
    if uq.is_empty() || appareil.is_empty() || addr.is_empty() {
        return HttpResponse::BadRequest().json(serde_json::json!({"status":"error","message":"uq, appareil et addr requis"}));
    }
    match bus.appairer(&uq, &appareil, &addr) {
        Ok(()) => {
            bus.notifier_pair(&addr);
            HttpResponse::Ok().json(serde_json::json!({"status":"success","uq":uq,"appareil":appareil,"addr":addr}))
        }
        Err(e) => HttpResponse::InternalServerError().json(serde_json::json!({"status":"error","message":e})),
    }
}

/// POST {uq, evenements: [{id, type, ...}]} — chaque event est diffusé
/// aux appareils appairés du compte, jamais au mesh ouvert.
pub async fn engines_evenement(bus: Bus, req: HttpRequest, corps: web::Json<serde_json::Value>) -> HttpResponse {
    if let Err(r) = crate::auth_guard::exiger_auth(&req) {
        return r;
    }
    let uq = corps.get("uq").and_then(|v| v.as_str()).unwrap_or("").to_string();
    if uq.is_empty() {
        return HttpResponse::BadRequest().json(serde_json::json!({"status":"error","message":"uq requis"}));
    }
    let Some(events) = corps.get("evenements").and_then(|v| v.as_array()) else {
        return HttpResponse::BadRequest().json(serde_json::json!({"status":"error","message":"evenements[] requis"}));
    };
    let mut publies = 0usize;
    for event in events {
        let id = event
            .get("id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| event.get("id").and_then(|v| v.as_i64()).map(|n| n.to_string()))
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let cle = format!("event:{}:{}", uq, id);
        if bus.publier(MOTEUR_SYNCHRO, &cle, event.clone(), Some(&uq)).is_ok() {
            publies += 1;
        }
    }
    HttpResponse::Ok().json(serde_json::json!({"status":"success","publies":publies,"uq":uq}))
}

pub async fn engines_oracle(bus: Bus) -> HttpResponse {
    HttpResponse::Ok().json(bus.agregat_oracle())
}

pub async fn engines_reputation(bus: Bus, path: web::Path<String>) -> HttpResponse {
    let uq = path.into_inner();
    HttpResponse::Ok().json(bus.agregat_reputation(&uq))
}

pub async fn engines_synchro(bus: Bus, req: HttpRequest, path: web::Path<String>) -> HttpResponse {
    if let Err(r) = crate::auth_guard::exiger_auth(&req) {
        return r;
    }
    let uq = path.into_inner();
    HttpResponse::Ok().json(bus.agregat_synchro(&uq))
}
