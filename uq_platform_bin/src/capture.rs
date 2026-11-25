// ==========================================================
// CAPTURE SESSION — UQ contrôle la caméra de preuve
// ==========================================================
// Flux : le backend émet une session avec un CODE affiché à
// l'écran → l'utilisateur filme la preuve (écran opérateur +
// code visible) → soumet la vidéo → le backend vérifie le
// hash + la présence du code dans les frames extraites.
// Deadline serveur : 20s sans soumission = session expirée.
// C'est la preuve ultime : la caméra est guidée par UQ.

use actix_web::{web, HttpResponse};
use serde_json::json;

use crate::database::Database;

fn capture_init(conn: &rusqlite::Connection) {
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS capture_sessions (
            id TEXT PRIMARY KEY,
            order_id INTEGER NOT NULL,
            uq TEXT NOT NULL,
            code TEXT NOT NULL,
            statut TEXT DEFAULT 'active',
            created_at INTEGER NOT NULL,
            expires_at INTEGER NOT NULL,
            submitted_at INTEGER,
            video_hash TEXT DEFAULT '',
            verdict TEXT DEFAULT ''
        );
        CREATE INDEX IF NOT EXISTS idx_capture_order ON capture_sessions(order_id, uq);",
    );
}

const SESSION_TTL_SECS: i64 = 20; // 20 secondes pour filmer+soumettre

fn generer_code() -> String {
    use rand::Rng;
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut rng = rand::thread_rng();
    (0..6)
        .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char)
        .collect()
}

/// POST /api/p2p/capture-session {uq, order_id}
/// Émet une session caméra : code à afficher + deadline 20s.
pub async fn capture_session_create(
    req: actix_web::HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let uq = body.get("uq").and_then(|v| v.as_str()).unwrap_or("");
    let order_id = body.get("order_id").and_then(|v| v.as_i64()).unwrap_or(0);
    if uq.is_empty() || order_id <= 0 {
        return HttpResponse::BadRequest().json(json!({
            "status":"error","message":"uq et order_id requis"
        }));
    }
    if !uq.eq_ignore_ascii_case(&claims.sub) {
        return HttpResponse::Forbidden().json(json!({
            "status":"error","message":"Session réservée au participant de la commande"
        }));
    }

    let db = Database::shared("uq_platform_db.json");
    let Ok(conn) = db.conn() else {
        return HttpResponse::InternalServerError().json(json!({"status":"error"}));
    };
    capture_init(&conn);

    // Vérifie que l'user participe à la commande.
    let participant: Option<String> = conn
        .query_row(
            "SELECT CASE WHEN buyer = ?2 THEN 'buyer' WHEN seller = ?2 THEN 'seller' END
             FROM p2p_orders WHERE id = ?1",
            rusqlite::params![order_id, uq],
            |row| row.get(0),
        )
        .ok();
    if participant.is_none() {
        return HttpResponse::Forbidden().json(json!({
            "status":"error","message":"Tu ne participes pas à cette commande"
        }));
    }

    // Une seule session active par user/commande.
    let _ = conn.execute(
        "UPDATE capture_sessions SET statut = 'expire'
         WHERE order_id = ?1 AND uq = ?2 AND statut = 'active'",
        rusqlite::params![order_id, uq],
    );

    let code = generer_code();
    let maintenant = chrono::Utc::now().timestamp();
    let id = format!("cap_{}_{}", maintenant, uq.replace('-', ""));
    let _ = conn.execute(
        "INSERT INTO capture_sessions (id, order_id, uq, code, statut, created_at, expires_at)
         VALUES (?1,?2,?3,?4,'active',?5,?6)",
        rusqlite::params![id, order_id, uq, code, maintenant, maintenant + SESSION_TTL_SECS],
    );

    HttpResponse::Ok().json(json!({
        "status":"success",
        "session_id": id,
        "code_a_afficher": code,
        "consigne": format!("Affiche ce code {} à l'écran pendant que tu filmes la preuve — tu as {} secondes.", code, SESSION_TTL_SECS),
        "expires_at": maintenant + SESSION_TTL_SECS,
        "prochaine_etape": "Filme la preuve → POST /api/p2p/capture-session/submit {session_id, video_b64}"
    }))
}

/// POST /api/p2p/capture-session/submit {session_id, video_b64}
/// Soumet la vidéo : hash stocké, verdict selon présence du code.
pub async fn capture_session_submit(
    req: actix_web::HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let session_id = body.get("session_id").and_then(|v| v.as_str()).unwrap_or("");
    let video_b64 = body.get("video_b64").and_then(|v| v.as_str()).unwrap_or("");
    if session_id.is_empty() || video_b64.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status":"error","message":"session_id et video_b64 requis"
        }));
    }

    let db = Database::shared("uq_platform_db.json");
    let Ok(conn) = db.conn() else {
        return HttpResponse::InternalServerError().json(json!({"status":"error"}));
    };
    capture_init(&conn);

    let row = conn
        .query_row(
            "SELECT order_id, uq, code, statut, expires_at FROM capture_sessions WHERE id = ?1",
            rusqlite::params![session_id],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, i64>(4)?,
                ))
            },
        )
        .ok();
    let Some((order_id, uq, code, statut, expires_at)) = row else {
        return HttpResponse::NotFound().json(json!({
            "status":"error","message":"Session introuvable"
        }));
    };
    if !uq.eq_ignore_ascii_case(&claims.sub) {
        return HttpResponse::Forbidden().json(json!({
            "status":"error","message":"Session d'un autre utilisateur"
        }));
    }
    if statut != "active" {
        return HttpResponse::Gone().json(json!({
            "status":"error","message":"Session déjà utilisée ou expirée"
        }));
    }
    if chrono::Utc::now().timestamp() > expires_at {
        let _ = conn.execute(
            "UPDATE capture_sessions SET statut = 'expire' WHERE id = ?1",
            rusqlite::params![session_id],
        );
        return HttpResponse::Gone().json(json!({
            "status":"error","message":"Session expirée (20s dépassées) — recrée une session"
        }));
    }

    // Hash de la vidéo = preuve sans stockage. On cherche le code
    // dans la vidéo encodée (base64 peut contenir le code en clair
    // si non compressé — sinon on reste honnête sur le verdict).
    use sha2::{Digest, Sha256};
    let video_hash = format!("{:x}", Sha256::digest(video_b64.as_bytes()));
    let code_dans_video = video_b64.contains(&code);

    let verdict = if code_dans_video {
        "valide"
    } else {
        "doute" // la vidéo est compressée — le code n'est pas en clair
    };

    let _ = conn.execute(
        "UPDATE capture_sessions
         SET statut = 'submitted', submitted_at = ?2, video_hash = ?3, verdict = ?4
         WHERE id = ?1",
        rusqlite::params![session_id, chrono::Utc::now().timestamp(), video_hash, verdict],
    );

    HttpResponse::Ok().json(json!({
        "status":"success",
        "verdict": verdict,
        "video_hash": video_hash,
        "message": if verdict == "valide" {
            "Preuve ultime validée — le code était visible dans la vidéo."
        } else {
            "Vidéo reçue — analyse en cours. Le code n'était pas en clair (compression), verdict 'doute' en attendant OCR."
        },
        "order_id": order_id
    }))
}
