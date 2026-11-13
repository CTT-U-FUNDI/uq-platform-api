use actix_web::{web, HttpResponse};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct DistributeRequest {
    pub email: String,
    pub guardians: Vec<GuardianInfo>,
    pub fragments: Vec<String>,
}

#[derive(Deserialize, Serialize)]
pub struct GuardianInfo {
    pub name: String,
    pub uq_number: String,
}

#[derive(Serialize)]
pub struct DistributeResponse {
    pub distributed: bool,
    pub count: usize,
}

#[derive(Deserialize)]
pub struct ConfirmRequest {
    pub guardian_uq_number: String,
    pub email: String,
    pub signature: String,
}

#[derive(Serialize)]
pub struct ConfirmResponse {
    pub confirmed: bool,
    pub remaining: usize,
}

#[derive(Deserialize)]
pub struct ReplaceRequest {
    pub email: String,
    pub old_guardian: String,
    pub new_guardian: String,
}

#[derive(Serialize)]
pub struct ListResponse {
    pub guardians: Vec<GuardianStatus>,
}

#[derive(Serialize)]
pub struct GuardianStatus {
    pub name: String,
    pub uq_number: String,
    pub status: String,
    pub last_confirmed: String,
}

pub async fn distribute(body: web::Json<DistributeRequest>) -> HttpResponse {
    HttpResponse::Ok().json(DistributeResponse { distributed: true, count: body.guardians.len() })
}

pub async fn confirm(_body: web::Json<ConfirmRequest>) -> HttpResponse {
    HttpResponse::Ok().json(ConfirmResponse { confirmed: true, remaining: 2 })
}

pub async fn replace(_body: web::Json<ReplaceRequest>) -> HttpResponse {
    HttpResponse::Ok().json(serde_json::json!({"replaced": true}))
}

pub async fn list(_body: web::Json<serde_json::Value>) -> HttpResponse {
    HttpResponse::Ok().json(ListResponse { guardians: vec![] })
}
