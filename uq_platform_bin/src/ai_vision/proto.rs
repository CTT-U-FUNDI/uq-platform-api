// Commandes du protocole vision — port de vision_server.py.
use crate::ai_vision::{metrics, VisionEngine};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use image::ImageReader;
use std::io::Cursor;

fn decode_img(b64: &str) -> Result<image::RgbImage, String> {
    let bytes = B64.decode(b64).map_err(|_| "image illisible (base64)")?;
    let img = ImageReader::new(Cursor::new(bytes)).with_guessed_format()
        .map_err(|e| e.to_string())?
        .decode().map_err(|e| e.to_string())?;
    Ok(img.to_rgb8())
}

fn r4(v: f32) -> f32 { (v * 10000.0).round() / 10000.0 }
fn r1(v: f32) -> f32 { (v * 10.0).round() / 10.0 }

pub fn cmd_ping() -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({
        "ready": true, "engine": "rust", "yunet": "face_detection_yunet_2023mar.onnx"
    }))
}

/// face {img} -> metrics du plus grand visage (meme format que le Python).
pub fn cmd_face(e: &VisionEngine, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let img = decode_img(args.get("img").and_then(|v| v.as_str()).unwrap_or(""))?;
    let (w, h) = (img.width() as usize, img.height() as usize);
    let rgb = img.into_raw();
    let light = metrics::lumiere(&rgb, w, h);
    let blur = metrics::nettete(&rgb, w, h);
    let glare = metrics::reflets(&rgb, w, h);
    let frame = serde_json::json!({"w": w, "h": h});

    let visages = e.detecteur.detecter(&rgb, w as u32, h as u32)?;
    let Some(v) = visages.into_iter().next() else {
        return Ok(serde_json::json!({
            "detected": false, "frame": frame,
            "light": r1(light), "blur": r1(blur), "glare": r4(glare)
        }));
    };

    let (w_f, h_f) = (w as f32, h as f32);
    let cx = (v.x + v.w / 2.0) / w_f;
    let cy = (v.y + v.h / 2.0) / h_f;
    let bouche = ((v.bouche_r.0 + v.bouche_l.0) / 2.0, (v.bouche_r.1 + v.bouche_l.1) / 2.0);
    let ecart_yeux = ((v.oeil_droit.0 - v.oeil_gauche.0).powi(2)
        + (v.oeil_droit.1 - v.oeil_gauche.1).powi(2)).sqrt().max(1.0);
    let ligne_yeux_y = (v.oeil_droit.1 + v.oeil_gauche.1) / 2.0;
    let roulis = (v.oeil_gauche.1 - v.oeil_droit.1)
        .atan2(v.oeil_gauche.0 - v.oeil_droit.0).to_degrees();
    let lacet = (v.nez.0 - (v.oeil_droit.0 + v.oeil_gauche.0) / 2.0) / ecart_yeux;
    let bande = bouche.1 - ligne_yeux_y;
    let tangage = if bande.abs() > 1.0 { (v.nez.1 - ligne_yeux_y) / bande } else { 0.0 };

    Ok(serde_json::json!({
        "detected": true,
        "frame": frame,
        "box": {"x": r4(v.x / w_f), "y": r4(v.y / h_f), "w": r4(v.w / w_f), "h": r4(v.h / h_f)},
        "center": {"cx": r4(cx), "cy": r4(cy)},
        "size": r4(v.w / w_f),
        "offset": {"dx": r4(cx - 0.5), "dy": r4(cy - 0.5)},
        "angles": {"roulis": r1(roulis), "lacet": r4(lacet), "tangage": r4(tangage)},
        "frontal": lacet.abs() < 0.22 && tangage > 0.55 && tangage < 0.95,
        "yeux_et_bouche": r4(bande / v.h.max(1.0)),
        "score": r4(v.score),
        "light": r1(light), "blur": r1(blur), "glare": r4(glare),
        "points": {
            "oeil_droit": [r4(v.oeil_droit.0 / w_f), r4(v.oeil_droit.1 / h_f)],
            "oeil_gauche": [r4(v.oeil_gauche.0 / w_f), r4(v.oeil_gauche.1 / h_f)],
            "nez": [r4(v.nez.0 / w_f), r4(v.nez.1 / h_f)],
            "bouche": [r4(bouche.0 / w_f), r4(bouche.1 / h_f)],
        }
    }))
}

/// doc {img, warp?} -> detection du quad document (meme format).
pub fn cmd_doc(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let img = decode_img(args.get("img").and_then(|v| v.as_str()).unwrap_or(""))?;
    let gray = image::imageops::grayscale(&img);
    let (w, h) = (gray.width() as usize, gray.height() as usize);
    let raw: Vec<u8> = gray.into_raw();
    let light = metrics::lumiere_rgb(&raw, w, h);
    let blur = metrics::nettete_gray(&raw, w, h);
    let glare = metrics::reflets_gray(&raw, w, h);
    Ok(serde_json::json!({
        "found": false,
        "frame": {"w": w, "h": h},
        "light": r1(light), "blur": r1(blur), "glare": r4(glare),
        "note": "detecteur de contours a porter (imageproc)"
    }))
}

/// check {frames} -> verification de sequence de vivacite.
pub fn cmd_check(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let frames = args.get("frames").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    // Port minimal : chaque frame doit avoir detected=true et angles dans les seuils.
    let mut ok = 0usize;
    for f in &frames {
        if f.get("detected").and_then(|v| v.as_bool()).unwrap_or(false) {
            let frontal = f.get("frontal").and_then(|v| v.as_bool()).unwrap_or(false);
            if frontal { ok += 1; }
        }
    }
    Ok(serde_json::json!({
        "verified": frames.len() > 0 && ok == frames.len(),
        "frames": frames.len(), "frontal_frames": ok
    }))
}
