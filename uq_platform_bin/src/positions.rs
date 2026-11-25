// ==========================================================
// POSITIONS — sync hors-ligne + position courante
// ==========================================================
// Le client (wallet/app) enregistre les positions en local quand la
// connexion tombe, puis rejoue la file via POST /api/positions/sync
// dès le retour réseau. Le backend garde la DERNIÈRE position par UQ
// et un historique borné — l'itinéraire/le flux peut alors proposer
// des contenus proches même si l'utilisateur était hors-ligne.

use actix_web::{web, HttpResponse};
use serde_json::json;

use crate::database::Database;

fn positions_init(conn: &rusqlite::Connection) {
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS positions_actuelles (
            uq TEXT PRIMARY KEY,
            latitude REAL NOT NULL,
            longitude REAL NOT NULL,
            precision_m REAL DEFAULT 0,
            source TEXT DEFAULT 'gps',
            updated_at INTEGER NOT NULL,
            offline_synced INTEGER DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS positions_historique (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            uq TEXT NOT NULL,
            latitude REAL NOT NULL,
            longitude REAL NOT NULL,
            ts INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_pos_hist_uq ON positions_historique(uq, ts);",
    );
}

/// POST /api/positions/sync {points: [{uq, latitude, longitude, ts?, precision_m?}]}
/// JWT requis — chaque point doit appartenir au compte authentifié.
/// Accepte une FILE : le client rejoue ses positions hors-ligne ;
/// on ne garde que la plus récente par UQ + trace l'historique.
pub async fn positions_sync(
    req: actix_web::HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let points = match body.get("points").and_then(|v| v.as_array()) {
        Some(p) if !p.is_empty() => p,
        _ => {
            return HttpResponse::BadRequest().json(json!({
                "status": "error", "message": "points[] requis"
            }))
        }
    };
    if points.len() > 500 {
        return HttpResponse::BadRequest().json(json!({
            "status": "error", "message": "Maximum 500 points par sync"
        }));
    }

    let db = Database::shared("uq_platform_db.json");
    let Ok(conn) = db.conn() else {
        return HttpResponse::InternalServerError().json(json!({"status":"error"}));
    };
    positions_init(&conn);

    let mut appliquees = 0u32;
    let mut derniere: Option<(f64, f64, i64)> = None;
    for p in points {
        let uq = p.get("uq").and_then(|v| v.as_str()).unwrap_or("");
        // Sécurité : on n'accepte que la position du compte authentifié.
        if !uq.eq_ignore_ascii_case(&claims.sub) {
            continue;
        }
        let (Some(lat), Some(lon)) = (
            p.get("latitude").and_then(|v| v.as_f64()),
            p.get("longitude").and_then(|v| v.as_f64()),
        ) else {
            continue;
        };
        if !lat.is_finite() || !lon.is_finite() || lat.abs() > 90.0 || lon.abs() > 180.0 {
            continue;
        }
        let ts = p
            .get("ts")
            .and_then(|v| v.as_i64())
            .unwrap_or_else(|| chrono::Utc::now().timestamp());
        let precision = p.get("precision_m").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let offline = if ts < chrono::Utc::now().timestamp() - 300 { 1 } else { 0 };

        // Upsert : la position la plus RÉCENTE gagne (ts client fait foi
        // pour l'ordre de la file ; updated_at = réception serveur).
        let _ = conn.execute(
            "INSERT INTO positions_actuelles (uq, latitude, longitude, precision_m, source, updated_at, offline_synced)
             VALUES (?1,?2,?3,?4,'sync',?5,?6)
             ON CONFLICT(uq) DO UPDATE SET
               latitude = excluded.latitude,
               longitude = excluded.longitude,
               precision_m = excluded.precision_m,
               updated_at = excluded.updated_at,
               offline_synced = excluded.offline_synced
             WHERE excluded.updated_at >= positions_actuelles.updated_at",
            rusqlite::params![claims.sub, lat, lon, precision, ts, offline],
        );
        // Historique borné : on garde les 200 derniers points par user.
        let _ = conn.execute(
            "INSERT INTO positions_historique (uq, latitude, longitude, ts) VALUES (?1,?2,?3,?4)",
            rusqlite::params![claims.sub, lat, lon, ts],
        );
        let _ = conn.execute(
            "DELETE FROM positions_historique WHERE uq = ?1 AND id NOT IN
             (SELECT id FROM positions_historique WHERE uq = ?1 ORDER BY ts DESC LIMIT 200)",
            rusqlite::params![claims.sub],
        );
        appliquees += 1;
        derniere = Some((lat, lon, ts));
    }

    HttpResponse::Ok().json(json!({
        "status": "success",
        "appliquees": appliquees,
        "derniere": derniere.map(|(la, lo, ts)| json!({"lat": la, "lon": lo, "ts": ts})),
        "message": "Positions synchronisées — le flux peut proposer du contenu proche."
    }))
}

/// GET /api/positions/ma-position — ma dernière position connue.
pub async fn position_moi(req: actix_web::HttpRequest) -> HttpResponse {
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let db = Database::shared("uq_platform_db.json");
    let Ok(conn) = db.conn() else {
        return HttpResponse::Ok().json(json!({"status":"success","position":null}));
    };
    positions_init(&conn);
    let row = conn
        .query_row(
            "SELECT latitude, longitude, precision_m, updated_at, offline_synced
             FROM positions_actuelles WHERE uq = ?1",
            rusqlite::params![claims.sub],
            |r| {
                Ok(json!({
                    "lat": r.get::<_, f64>(0)?,
                    "lon": r.get::<_, f64>(1)?,
                    "precision_m": r.get::<_, f64>(2).unwrap_or(0.0),
                    "updated_at": r.get::<_, i64>(3)?,
                    "offline_synced": r.get::<_, i64>(4)? == 1,
                }))
            },
        )
        .ok();
    HttpResponse::Ok().json(json!({"status":"success","position":row}))
}
