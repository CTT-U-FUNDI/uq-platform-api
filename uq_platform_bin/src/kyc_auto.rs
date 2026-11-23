// ====================
// KYC AUTOMATIQUE — Orchestration Tesseract + YuNet + SFace
// ====================

use actix_web::{web, HttpRequest, HttpResponse};
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

// ====================
// SÉCURITÉ KYC — whitelist chemins + rate-limit + auth JWT
// ====================

/// Racine autorisée pour les fichiers KYC (uploads uniquement).
fn kyc_root() -> PathBuf {
    PathBuf::from("data").join("kyc_uploads")
}

/// Extensions acceptées pour les fichiers analyzés.
const EXT_OK: &[&str] = &["jpg", "jpeg", "png", "webp", "bmp", "mp4", "webm", "mov"];

/// Valide qu'un chemin pointe vers un fichier existant SOUS data/kyc_uploads.
/// Bloque tout path traversal (../../etc/passwd).
fn chemin_valide(raw: &str) -> Result<PathBuf, String> {
    if raw.is_empty() {
        return Err("chemin vide".to_string());
    }
    let p = Path::new(raw);
    let ext_ok = p
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| EXT_OK.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false);
    if !ext_ok {
        return Err("extension non autorisée".to_string());
    }
    // Le chemin doit être sous la racine KYC (relatif ou absolu canonisé)
    let root = kyc_root();
    let candidat = if p.is_absolute() {
        // Chemins absolus : on exige qu'ils soient sous la racine canonisée
        match (p.canonicalize(), root.canonicalize()) {
            (Ok(c), Ok(r)) if c.starts_with(&r) => c,
            _ => return Err("hors zone KYC".to_string()),
        }
    } else {
        // Relatifs : on les préfixe avec la racine, puis canonise
        let joint = root.join(p);
        match joint.canonicalize() {
            Ok(c) => c,
            Err(_) => return Err("fichier introuvable".to_string()),
        }
    };
    if !candidat.starts_with(&root.canonicalize().unwrap_or_else(|_| root.clone())) {
        return Err("path traversal bloqué".to_string());
    }
    if !candidat.is_file() {
        return Err("pas un fichier".to_string());
    }
    Ok(candidat)
}

/// Rate-limit simple : max N requêtes KYC par IP par fenêtre de 60s.
static KYC_RATE: Mutex<Option<(u64, HashMap<String, u32>)>> = Mutex::new(None);
const KYC_MAX_PAR_MINUTE: u32 = 20;

fn rate_limit_ok(ip: &str) -> bool {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        / 60;
    let mut guard = match KYC_RATE.lock() {
        Ok(g) => g,
        Err(_) => return true,
    };
    let slot = guard.get_or_insert_with(|| (now, HashMap::new()));
    if slot.0 != now {
        slot.0 = now;
        slot.1.clear();
    }
    let count = slot.1.entry(ip.to_string()).or_insert(0);
    if *count >= KYC_MAX_PAR_MINUTE {
        return false;
    }
    *count += 1;
    true
}

/// Garde commune : auth JWT + rate-limit. Retourne Err(HttpResponse) si refusé.
fn garde_kyc(req: &HttpRequest) -> Result<(), HttpResponse> {
    // 1. Authentification JWT obligatoire
    if let Err(r) = crate::auth_guard::exiger_auth(req) {
        return Err(r);
    }
    // 2. Rate-limit par IP
    let ip = req
        .connection_info()
        .peer_addr()
        .unwrap_or("inconnu")
        .to_string();
    if !rate_limit_ok(&ip) {
        return Err(HttpResponse::TooManyRequests().json(json!({
            "status": "error",
            "message": "Trop de requêtes KYC — réessayez dans une minute"
        })));
    }
    Ok(())
}

// ====================
// HELPER — Appeler un script Python
// ====================

fn call_python(script: &str, args: &[&str]) -> Result<serde_json::Value, String> {
    let script_path = format!("scripts/kyc_python/{}", script);

    let mut cmd = Command::new("python");
    cmd.arg(&script_path);

    for arg in args {
        cmd.arg(arg);
    }

    let output = cmd
        .output()
        .map_err(|e| format!("Erreur execution Python: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Python erreur: {}", stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    serde_json::from_str(&stdout).map_err(|e| format!("Erreur parsing JSON: {}", e))
}

// ====================
// POST /api/kyc/doc/analyze
// Analyse un document (image) avec OCR
// ====================

pub async fn kyc_doc_analyze(req: HttpRequest, body: web::Json<serde_json::Value>) -> HttpResponse {
    if let Err(r) = garde_kyc(&req) {
        return r;
    }
    let image_path = body
        .get("image_path")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let country = body
        .get("country")
        .and_then(|v| v.as_str())
        .unwrap_or("RDC");
    let lang = body.get("lang").and_then(|v| v.as_str()).unwrap_or("fra");

    if image_path.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "image_path requis"
        }));
    }

    let image_path = match chemin_valide(image_path) {
        Ok(p) => p.to_string_lossy().to_string(),
        Err(e) => return HttpResponse::BadRequest().json(json!({"status":"error","message":e})),
    };

    // Appeler Python
    let country_arg = format!("--country={}", country);
    let lang_arg = format!("--lang={}", lang);

    match call_python("ocr_tesseract.py", &[&image_path, &country_arg, &lang_arg]) {
        Ok(result) => {
            let status = result
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("error");

            if status == "success" {
                let confidence = result
                    .get("ocr")
                    .and_then(|o| o.get("confidence"))
                    .and_then(|c| c.as_f64())
                    .unwrap_or(0.0);

                // Calculer score final
                let final_score = if confidence > 0.8 {
                    "VALIDATED"
                } else if confidence > 0.5 {
                    "PARTIAL"
                } else {
                    "REJECTED"
                };

                HttpResponse::Ok().json(json!({
                    "status": "success",
                    "score": confidence,
                    "verdict": final_score,
                    "data": result
                }))
            } else {
                HttpResponse::BadRequest().json(result)
            }
        }
        Err(e) => HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": e
        })),
    }
}

// ====================
// POST /api/kyc/face/compare
// Compare deux visages
// ====================

pub async fn kyc_face_compare(
    req: HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    if let Err(r) = garde_kyc(&req) {
        return r;
    }
    let img1_raw = body.get("image1").and_then(|v| v.as_str()).unwrap_or("");
    let img2_raw = body.get("image2").and_then(|v| v.as_str()).unwrap_or("");

    if img1_raw.is_empty() || img2_raw.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "image1 et image2 requis"
        }));
    }

    let (img1, img2) = match (chemin_valide(img1_raw), chemin_valide(img2_raw)) {
        (Ok(a), Ok(b)) => (
            a.to_string_lossy().to_string(),
            b.to_string_lossy().to_string(),
        ),
        (Err(e), _) | (_, Err(e)) => {
            return HttpResponse::BadRequest().json(json!({"status":"error","message":e}));
        }
    };

    match call_python("face_compare.py", &[&img1, &img2]) {
        Ok(result) => HttpResponse::Ok().json(result),
        Err(e) => HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": e
        })),
    }
}

// ====================
// POST /api/kyc/liveness
// Vérifie la vivacité d'une vidéo
// ====================

pub async fn kyc_liveness(req: HttpRequest, body: web::Json<serde_json::Value>) -> HttpResponse {
    if let Err(r) = garde_kyc(&req) {
        return r;
    }
    let video_path = body
        .get("video_path")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if video_path.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "video_path requis"
        }));
    }

    let video_path = match chemin_valide(video_path) {
        Ok(p) => p.to_string_lossy().to_string(),
        Err(e) => return HttpResponse::BadRequest().json(json!({"status":"error","message":e})),
    };

    match call_python("liveness_check.py", &[&video_path]) {
        Ok(result) => HttpResponse::Ok().json(result),
        Err(e) => HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": e
        })),
    }
}

// ====================
// POST /api/kyc/auto/validate
// Validation automatique complète (doc + face + liveness)
// ====================

pub async fn kyc_auto_validate(
    req: HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    if let Err(r) = garde_kyc(&req) {
        return r;
    }
    let doc_raw = body.get("doc_image").and_then(|v| v.as_str()).unwrap_or("");
    let selfie_raw = body
        .get("selfie_image")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let video_raw = body
        .get("video_path")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let country = body
        .get("country")
        .and_then(|v| v.as_str())
        .unwrap_or("RDC");

    if doc_raw.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "doc_image requis"
        }));
    }

    // Validation des chemins — whitelist data/kyc_uploads uniquement
    let doc_image = match chemin_valide(doc_raw) {
        Ok(p) => p.to_string_lossy().to_string(),
        Err(e) => return HttpResponse::BadRequest().json(json!({"status":"error","message":e})),
    };
    let selfie_image = if selfie_raw.is_empty() {
        String::new()
    } else {
        match chemin_valide(selfie_raw) {
            Ok(p) => p.to_string_lossy().to_string(),
            Err(e) => {
                return HttpResponse::BadRequest().json(json!({"status":"error","message":e}))
            }
        }
    };
    let video_path = if video_raw.is_empty() {
        String::new()
    } else {
        match chemin_valide(video_raw) {
            Ok(p) => p.to_string_lossy().to_string(),
            Err(e) => {
                return HttpResponse::BadRequest().json(json!({"status":"error","message":e}))
            }
        }
    };

    // 1. OCR du document
    let country_arg = format!("--country={}", country);
    let ocr_result = call_python("ocr_tesseract.py", &[&doc_image, &country_arg])
        .unwrap_or(json!({"status": "error"}));

    let ocr_score = ocr_result
        .get("ocr")
        .and_then(|o| o.get("confidence"))
        .and_then(|c| c.as_f64())
        .unwrap_or(0.0);

    // 2. Face compare (si selfie fourni)
    let face_score = if !selfie_image.is_empty() && Path::new(&selfie_image).exists() {
        let face_result = call_python("face_compare.py", &[&doc_image, &selfie_image])
            .unwrap_or(json!({"similarity": 0.0}));
        face_result
            .get("similarity")
            .and_then(|s| s.as_f64())
            .unwrap_or(0.0)
    } else {
        0.0
    };

    // 3. Liveness (si vidéo fournie)
    let liveness_score = if !video_path.is_empty() && Path::new(&video_path).exists() {
        let live_result = call_python("liveness_check.py", &[&video_path])
            .unwrap_or(json!({"liveness_score": 0.0}));
        live_result
            .get("liveness_score")
            .and_then(|s| s.as_f64())
            .unwrap_or(0.0)
    } else {
        0.0
    };

    // 4. Score final pondéré
    // OCR: 20%, Face: 50%, Liveness: 30%
    let final_score = (ocr_score * 0.2 + face_score * 0.5 + liveness_score * 0.3) * 100.0;

    let verdict = if final_score > 80.0 {
        "VALIDATED"
    } else if final_score > 50.0 {
        "PARTIAL"
    } else {
        "REJECTED"
    };

    HttpResponse::Ok().json(json!({
        "status": "success",
        "final_score": round(final_score, 1),
        "verdict": verdict,
        "details": {
            "ocr_score": round(ocr_score * 100.0, 1),
            "face_score": round(face_score * 100.0, 1),
            "liveness_score": round(liveness_score * 100.0, 1)
        },
        "ocr_data": ocr_result
    }))
}

fn round(val: f64, decimals: u32) -> f64 {
    let factor = 10_f64.powi(decimals as i32);
    (val * factor).round() / factor
}

// ====================
// GET /api/kyc/status
// Statut du système KYC
// ====================

pub async fn kyc_status() -> HttpResponse {
    let tesseract_ok = Path::new("C:\\Program Files\\Tesseract-OCR\\tesseract.exe").exists()
        || Path::new("/usr/bin/tesseract").exists()
        || Path::new("/usr/local/bin/tesseract").exists()
        || Command::new("tesseract").arg("--version").output().is_ok();
    let ocr_script_ok = Path::new("scripts/kyc_python/ocr_tesseract.py").exists();
    let face_script_ok = Path::new("scripts/kyc_python/face_compare.py").exists();
    let liveness_script_ok = Path::new("scripts/kyc_python/liveness_check.py").exists();

    HttpResponse::Ok().json(json!({
        "status": "success",
        "components": {
            "tesseract": tesseract_ok,
            "ocr_script": ocr_script_ok,
            "face_script": face_script_ok,
            "liveness_script": liveness_script_ok
        },
        "ready": tesseract_ok && ocr_script_ok && face_script_ok && liveness_script_ok
    }))
}
