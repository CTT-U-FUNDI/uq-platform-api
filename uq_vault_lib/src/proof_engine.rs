use actix_web::{web, HttpResponse};
use serde::{Deserialize, Serialize};
use sha2::{Sha256, Digest};
use uuid::Uuid;

#[derive(Deserialize)]
pub struct CreateProofRequest {
    pub email: String,
    pub uq_number: String,
    pub public_key: String,
    pub signature: String,
}

#[derive(Serialize)]
pub struct CreateProofResponse {
    pub proof_id: String,
    pub proof_hash: String,
}

#[derive(Deserialize)]
pub struct VerifyProofRequest {
    pub proof_id: String,
    pub proof_hash: String,
    pub uq_number: String,
}

#[derive(Serialize)]
pub struct VerifyProofResponse {
    pub valid: bool,
}

pub async fn create_proof(body: web::Json<CreateProofRequest>) -> HttpResponse {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    
    let proof_id = Uuid::new_v4().to_string();
    let raw = format!("{}|{}|{}|{}|{}", body.email, body.uq_number, body.public_key, timestamp, body.signature);
    let proof_hash = hex::encode(Sha256::digest(raw.as_bytes()));
    
    HttpResponse::Ok().json(CreateProofResponse { proof_id, proof_hash })
}

pub async fn verify_proof(body: web::Json<VerifyProofRequest>) -> HttpResponse {
    let valid = !body.proof_hash.is_empty() && !body.proof_id.is_empty();
    HttpResponse::Ok().json(VerifyProofResponse { valid })
}
