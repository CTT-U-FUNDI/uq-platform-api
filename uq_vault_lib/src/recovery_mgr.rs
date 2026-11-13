use actix_web::{web, HttpResponse};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Deserialize)]
pub struct InitRecoveryRequest {
    pub email: String,
    pub reason: String,
}

#[derive(Serialize)]
pub struct InitRecoveryResponse {
    pub recovery_id: String,
    pub expires_at: u64,
}

#[derive(Serialize)]
pub struct StatusResponse {
    pub confirmations: usize,
    pub remaining_time: String,
}

#[derive(Deserialize)]
pub struct CancelRequest {
    pub recovery_id: String,
    pub email: String,
    pub signature: String,
}

#[derive(Deserialize)]
pub struct EmergencyRequest {
    pub email: String,
    pub reason: String,
}

#[derive(Serialize)]
pub struct EmergencyResponse {
    pub locked: bool,
    pub emergency_id: String,
}

pub async fn init(_body: web::Json<InitRecoveryRequest>) -> HttpResponse {
    let expires = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() + 259200;
    HttpResponse::Ok().json(InitRecoveryResponse { recovery_id: Uuid::new_v4().to_string(), expires_at: expires })
}

pub async fn status(_body: web::Json<serde_json::Value>) -> HttpResponse {
    HttpResponse::Ok().json(StatusResponse { confirmations: 2, remaining_time: "48h".into() })
}

pub async fn cancel(_body: web::Json<CancelRequest>) -> HttpResponse {
    HttpResponse::Ok().json(serde_json::json!({"cancelled": true}))
}

pub async fn emergency(_body: web::Json<EmergencyRequest>) -> HttpResponse {
    HttpResponse::Ok().json(EmergencyResponse { locked: true, emergency_id: Uuid::new_v4().to_string() })
}
