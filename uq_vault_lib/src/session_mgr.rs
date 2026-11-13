use actix_web::{web, HttpResponse};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Deserialize)]
pub struct CreateSessionRequest {
    pub email: String,
    pub device_id: String,
    pub biometric_hash: String,
}

#[derive(Serialize)]
pub struct CreateSessionResponse {
    pub session_token: String,
    pub expires_at: u64,
}

#[derive(Deserialize)]
pub struct VerifySessionRequest {
    pub session_token: String,
    pub device_id: String,
}

#[derive(Serialize)]
pub struct VerifySessionResponse {
    pub valid: bool,
}

#[derive(Deserialize)]
pub struct RevokeSessionRequest {
    pub session_token: String,
    pub email: String,
}

#[derive(Serialize)]
pub struct RevokeSessionResponse {
    pub revoked: bool,
}

pub async fn create(_body: web::Json<CreateSessionRequest>) -> HttpResponse {
    let token = Uuid::new_v4().to_string();
    let expires = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() + 86400;
    HttpResponse::Ok().json(CreateSessionResponse { session_token: token, expires_at: expires })
}

pub async fn verify(body: web::Json<VerifySessionRequest>) -> HttpResponse {
    HttpResponse::Ok().json(VerifySessionResponse { valid: !body.session_token.is_empty() })
}

pub async fn revoke(_body: web::Json<RevokeSessionRequest>) -> HttpResponse {
    HttpResponse::Ok().json(RevokeSessionResponse { revoked: true })
}
