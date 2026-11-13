use actix_web::{web, HttpResponse};
use serde_json::json;
use uq_vault::mini_server::MiniServer;
use uq_vault::authenticity::AuthenticityVerifier;

pub struct MiniServerState {
    pub mini: std::sync::Mutex<MiniServer>,
    pub verifier: std::sync::Mutex<AuthenticityVerifier>,
}

impl MiniServerState {
    pub fn new() -> Self {
        let black_key = uq_black_key_standalone::UqBlackKey::generate();
        Self {
            mini: std::sync::Mutex::new(MiniServer::new(&black_key)),
            verifier: std::sync::Mutex::new(AuthenticityVerifier::new()),
        }
    }
}

pub async fn mini_identity(state: web::Data<std::sync::Arc<MiniServerState>>) -> HttpResponse {
    let mini = state.mini.lock().unwrap();
    HttpResponse::Ok().json(json!({
        "node_address": mini.identity(),
        "type": "mini_server_uq"
    }))
}

pub async fn mini_health(state: web::Data<std::sync::Arc<MiniServerState>>) -> HttpResponse {
    let mini = state.mini.lock().unwrap();
    let (used, max, active) = mini.health();
    HttpResponse::Ok().json(json!({
        "storage_used": used,
        "storage_max": max,
        "always_active": active
    }))
}

pub async fn mini_store(state: web::Data<std::sync::Arc<MiniServerState>>, body: web::Json<serde_json::Value>) -> HttpResponse {
    let fragment = body.get("fragment").and_then(|v| v.as_str()).unwrap_or("");
    let bytes = fragment.as_bytes().to_vec();
    let mut mini = state.mini.lock().unwrap();
    match mini.store(bytes) {
        Ok(()) => HttpResponse::Ok().json(json!({"status": "success", "stored": true})),
        Err(e) => HttpResponse::BadRequest().json(json!({"status": "error", "message": e}))
    }
}