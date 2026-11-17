//! Upload média -> Pixeldrain (stockage public gratuit).
//! POST /api/media/upload  body=octets bruts  ?name=fichier.jpg
//! Retourne { "url": "https://pixeldrain.com/u/<id>" }
use actix_web::{web, HttpRequest, HttpResponse};
use serde_json::json;

pub async fn media_upload(req: HttpRequest, body: web::Bytes) -> HttpResponse {
    let _claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(c) => c,
        Err(r) => return r,
    };
    if body.is_empty() || body.len() > 100 * 1024 * 1024 {
        return HttpResponse::BadRequest().json(json!({"status":"error","message":"Fichier vide ou >100Mo"}));
    }
    let name = req
        .query_string()
        .split('&')
        .find_map(|p| p.strip_prefix("name="))
        .unwrap_or("upload.bin")
        .to_string();

    // Multipart manuel vers Pixeldrain
    let boundary = "----uqzboundary42";
    let mut payload = Vec::new();
    payload.extend_from_slice(format!("--{}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{}\"\r\nContent-Type: application/octet-stream\r\n\r\n", boundary, name).as_bytes());
    payload.extend_from_slice(&body);
    payload.extend_from_slice(format!("\r\n--{}--\r\n", boundary).as_bytes());

    let client = reqwest::Client::new();
    let resp = client
        .post("https://pixeldrain.com/api/file")
        .header("Content-Type", format!("multipart/form-data; boundary={}", boundary))
        .body(payload)
        .send().await;
    let Ok(resp) = resp else { return HttpResponse::BadGateway().json(json!({"status":"error","message":"Pixeldrain injoignable"})); };
    let Ok(v): Result<serde_json::Value,_> = resp.json().await else {
        return HttpResponse::BadGateway().json(json!({"status":"error","message":"Reponse Pixeldrain invalide"}));
    };
    if let Some(id) = v.get("id").and_then(|x| x.as_str()) {
        return HttpResponse::Ok().json(json!({
            "status":"success",
            "url": format!("https://pixeldrain.com/u/{}", id),
            "id": id
        }));
    }
    HttpResponse::BadGateway().json(json!({"status":"error","message": v.to_string()}))
}
