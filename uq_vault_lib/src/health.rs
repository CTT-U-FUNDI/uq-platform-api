use actix_web::HttpResponse;
use serde::Serialize;

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub uptime: u64,
    pub connections: ConnectionStatus,
}

#[derive(Serialize)]
pub struct ConnectionStatus {
    pub chain: bool,
    pub p2p: bool,
}

pub async fn check() -> HttpResponse {
    HttpResponse::Ok().json(HealthResponse {
        status: "ok".into(),
        uptime: 3600,
        connections: ConnectionStatus { chain: true, p2p: true },
    })
}
