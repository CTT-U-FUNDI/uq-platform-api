use actix_web::{web, HttpResponse};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct StoreRequest {
    pub proof_hash: String,
    pub proof_data: String,
    pub metadata: String,
}

#[derive(Serialize)]
pub struct StoreResponse {
    pub stored: bool,
    pub locations: Vec<String>,
}

#[derive(Deserialize)]
pub struct RetrieveRequest {
    pub proof_hash: String,
}

#[derive(Serialize)]
pub struct RetrieveResponse {
    pub proof_data: String,
}

pub async fn store(_body: web::Json<StoreRequest>) -> HttpResponse {
    HttpResponse::Ok().json(StoreResponse { stored: true, locations: vec!["node1".into(), "node2".into(), "node3".into()] })
}

pub async fn retrieve(body: web::Json<RetrieveRequest>) -> HttpResponse {
    HttpResponse::Ok().json(RetrieveResponse { proof_data: format!("data-for-{}", body.proof_hash) })
}
