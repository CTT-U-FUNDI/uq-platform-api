// Moteur de vision UQ — portage Rust de scripts/kyc_python/vision_server.py.
// Protocole : une commande JSON en entree -> une reponse JSON en sortie.
// Commandes : ping | face {img,crop?} | doc {img,warp?} | check {frames,...}
//
// Detection visage : YuNet ONNX (models/face/face_detection_yunet_2023mar.onnx)
// Recognition : SFace ONNX (models/face/face_recognition_sface_2021dec.onnx)
// Metriques image : lumiere, nettete, reflets — pures fonctions Rust.

pub mod face;
pub mod metrics;
pub mod proto;

use actix_web::web;
use std::sync::OnceLock;

/// Moteur unique : detecteur YuNet charge une fois.
pub struct VisionEngine {
    pub detecteur: face::YuNet,
}

static ENGINE: OnceLock<VisionEngine> = OnceLock::new();

pub fn engine() -> Result<&'static VisionEngine, String> {
    if let Some(e) = ENGINE.get() { return Ok(e); }
    let modele = std::env::var("UQ_YUNET_MODEL")
        .unwrap_or_else(|_| "models/face/face_detection_yunet_2023mar.onnx".to_string());
    let detecteur = face::YuNet::charger(&modele)?;
    let e = ENGINE.get_or_init(|| VisionEngine { detecteur });
    Ok(e)
}

/// Point d'entree HTTP : POST /api/ai/vision {cmd, ...} -> JSON.
pub fn executer(cmd: &str, args: &serde_json::Value) -> serde_json::Value {
    let res = match cmd {
        "ping" => proto::cmd_ping(),
        "face" => engine().and_then(|e| proto::cmd_face(e, args)),
        "doc" => proto::cmd_doc(args),
        "check" => proto::cmd_check(args),
        _ => Err(format!("commande inconnue: {}", cmd)),
    };
    match res {
        Ok(v) => v,
        Err(e) => serde_json::json!({"error": e}),
    }
}

/// POST /api/ai/vision {cmd, ...} -> JSON (actix handler).
pub async fn route_vision(body: web::Json<serde_json::Value>) -> actix_web::HttpResponse {
    let cmd = body.get("cmd").and_then(|v| v.as_str()).unwrap_or("");
    let args = body.get("args").cloned().unwrap_or(serde_json::json!({}));
    let out = executer(cmd, &args);
    actix_web::HttpResponse::Ok().json(out)
}
