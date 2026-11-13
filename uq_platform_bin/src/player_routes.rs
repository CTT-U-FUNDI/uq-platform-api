use actix_web::{web, HttpResponse};
use serde_json::json;
use uq_player::{player::UqPlayer, cache::ContentCache, verification_link::VerificationBridge};

pub struct PlayerState {
    pub player: std::sync::Mutex<UqPlayer>,
    pub cache: std::sync::Mutex<ContentCache>,
}

impl PlayerState {
    pub fn new() -> Self {
        Self {
            player: std::sync::Mutex::new(UqPlayer::new()),
            cache: std::sync::Mutex::new(ContentCache::new()),
        }
    }
}

pub async fn player_start(
    state: web::Data<std::sync::Arc<PlayerState>>,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let creator_hex = body.get("creator").and_then(|v| v.as_str()).unwrap_or("");
    let price_per_second = body.get("price_per_second").and_then(|v| v.as_u64()).unwrap_or(1);
    let max_duration = body.get("max_duration_s").and_then(|v| v.as_u64()).unwrap_or(600);
    let content_url = body.get("content_url").and_then(|v| v.as_str()).unwrap_or("");
    let uq_link = body.get("uq_link").and_then(|v| v.as_str()).unwrap_or("");
    
    let cache = state.cache.lock().unwrap();
    if cache.is_paid(content_url) {
        return HttpResponse::Ok().json(json!({
            "status": "success",
            "cached": true,
            "message": "Contenu déjà payé"
        }));
    }
    drop(cache);
    
    let mut player = state.player.lock().unwrap();
    let creator_bytes = hex::decode(creator_hex).unwrap_or_default();
    let mut creator = [0u8; 32];
    if creator_bytes.len() >= 32 {
        creator.copy_from_slice(&creator_bytes[..32]);
    }
    
    match player.start(creator, price_per_second, max_duration) {
        Ok(()) => HttpResponse::Ok().json(json!({
            "status": "success",
            "message": "Lecture démarrée",
            "price_per_second": price_per_second
        })),
        Err(e) => HttpResponse::BadRequest().json(json!({"status": "error", "message": e}))
    }
}

pub async fn player_tick(state: web::Data<std::sync::Arc<PlayerState>>) -> HttpResponse {
    let mut player = state.player.lock().unwrap();
    match player.tick() {
        Ok(()) => HttpResponse::Ok().json(json!({"status": "success"})),
        Err(e) => HttpResponse::BadRequest().json(json!({"status": "error", "message": e}))
    }
}

pub async fn player_stop(
    state: web::Data<std::sync::Arc<PlayerState>>,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let content_url = body.get("content_url").and_then(|v| v.as_str()).unwrap_or("");
    let mut player = state.player.lock().unwrap();
    match player.stop() {
        Ok(total) => {
            let mut cache = state.cache.lock().unwrap();
            cache.mark_paid(content_url);
            HttpResponse::Ok().json(json!({
                "status": "success",
                "total_points_paid": total
            }))
        },
        Err(e) => HttpResponse::BadRequest().json(json!({"status": "error", "message": e}))
    }
}

pub async fn player_stats(state: web::Data<std::sync::Arc<PlayerState>>) -> HttpResponse {
    let player = state.player.lock().unwrap();
    let cache = state.cache.lock().unwrap();
    HttpResponse::Ok().json(json!({
        "total_sessions": player.stats.total_sessions,
        "total_seconds": player.stats.total_seconds,
        "total_paid": player.stats.total_paid,
        "cached_contents": cache.cache_size()
    }))
}
