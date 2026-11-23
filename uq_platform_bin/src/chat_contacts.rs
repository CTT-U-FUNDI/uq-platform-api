//! Contacts et reglages de conversation, persistes par compte dans
//! `data/users/{uq}.db`.
//!
//! Toute conversation ajoute automatiquement son interlocuteur a la liste de
//! contacts. Une suppression pose un marqueur (`deleted = 1`) : la ligne reste
//! en base pour que la conversation suivante ne ressuscite pas un contact
//! supprime volontairement.

use actix_web::{web, HttpRequest, HttpResponse};
use rusqlite::{params, Connection};
use serde_json::{json, Value};

const TABLE_SQL: &str = "
CREATE TABLE IF NOT EXISTS chat_contacts (
    peer_id TEXT PRIMARY KEY,
    added_at INTEGER NOT NULL,
    deleted INTEGER NOT NULL DEFAULT 0,
    blocked INTEGER NOT NULL DEFAULT 0,
    muted INTEGER NOT NULL DEFAULT 0,
    locked INTEGER NOT NULL DEFAULT 0,
    zando_item_id TEXT,
    zando_item_title TEXT
);
";

/// Mode ephemere ajoute apres coup : une base deja creee ne recoit pas la
/// colonne via `CREATE TABLE IF NOT EXISTS`, donc migration separee et toleree.
const MIGRATION_SQL: &str =
    "ALTER TABLE chat_contacts ADD COLUMN ephemere INTEGER NOT NULL DEFAULT 0;";

/// Un UQ valide reste `UQ-` suivi de chiffres : rien d'autre ne doit servir a
/// construire un chemin de fichier.
pub fn uq_valide(uq: &str) -> bool {
    let t = uq.trim();
    match t.get(..3) {
        Some(prefix) if prefix.eq_ignore_ascii_case("UQ-") => {}
        _ => return false,
    }
    let reste = &t[3..];
    !reste.is_empty() && reste.chars().all(|c| c.is_ascii_digit())
}

fn chemin_db(uq: &str) -> Option<String> {
    if !uq_valide(uq) {
        return None;
    }
    Some(format!(
        "data/users/{}.db",
        uq.trim().replace('-', "_").to_lowercase()
    ))
}

fn ouvrir(uq: &str) -> Result<Connection, String> {
    let path = chemin_db(uq).ok_or_else(|| "identifiant UQ invalide".to_string())?;
    if !std::path::Path::new(&path).exists() {
        return Err(format!("compte introuvable : {}", uq));
    }
    let conn = Connection::open(&path).map_err(|e| e.to_string())?;
    conn.execute_batch(TABLE_SQL).map_err(|e| e.to_string())?;
    // Colonne deja presente sur les bases recentes : l'erreur est attendue et
    // ignoree, la migration doit etre rejouable a chaque ouverture.
    crate::database::Database::ajouter_colonne_si_absente(
        &conn,
        "chat_contacts",
        "ephemere",
        "INTEGER NOT NULL DEFAULT 0",
    );
    Ok(conn)
}

fn ligne_vers_json(row: &rusqlite::Row) -> rusqlite::Result<Value> {
    let peer_id: String = row.get(0)?;
    let added_at: i64 = row.get(1)?;
    let deleted: i64 = row.get(2)?;
    let blocked: i64 = row.get(3)?;
    let muted: i64 = row.get(4)?;
    let locked: i64 = row.get(5)?;
    let zando_item_id: Option<String> = row.get(6)?;
    let zando_item_title: Option<String> = row.get(7)?;
    let ephemere: i64 = row.get(8)?;
    Ok(json!({
        "peer_id": peer_id,
        "added_at": added_at,
        "deleted": deleted != 0,
        "blocked": blocked != 0,
        "muted": muted != 0,
        "locked": locked != 0,
        "zando_item_id": zando_item_id,
        "zando_item_title": zando_item_title,
        "ephemere": ephemere != 0,
    }))
}

const COLONNES: &str =
    "peer_id, added_at, deleted, blocked, muted, locked, zando_item_id, zando_item_title, ephemere";

/// Ajoute l'interlocuteur s'il n'existe pas encore, sans jamais toucher au
/// marqueur de suppression. Un element Zando fourni met a jour le lien de la
/// conversation (le dernier element contacte gagne).
pub fn ajouter_contact(uq: &str, peer: &str, item_id: Option<&str>, item_titre: Option<&str>) {
    if !uq_valide(uq) || !uq_valide(peer) || uq.trim().eq_ignore_ascii_case(peer.trim()) {
        return;
    }
    let Ok(conn) = ouvrir(uq) else { return };
    let result = conn.execute(
        "INSERT INTO chat_contacts (peer_id, added_at, zando_item_id, zando_item_title)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(peer_id) DO UPDATE SET
            zando_item_id = COALESCE(excluded.zando_item_id, chat_contacts.zando_item_id),
            zando_item_title = COALESCE(excluded.zando_item_title, chat_contacts.zando_item_title)",
        params![
            peer.trim().to_uppercase(),
            chrono::Utc::now().timestamp(),
            item_id,
            item_titre
        ],
    );
    if let Err(error) = result {
        eprintln!("[CONTACTS] ajout {} -> {} impossible : {}", uq, peer, error);
    }
}

pub fn liste_contacts(uq: &str) -> Result<Vec<Value>, String> {
    let conn = ouvrir(uq)?;
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {} FROM chat_contacts WHERE deleted = 0 ORDER BY added_at DESC",
            COLONNES
        ))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| ligne_vers_json(row))
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

pub fn contact(uq: &str, peer: &str) -> Option<Value> {
    let conn = ouvrir(uq).ok()?;
    conn.query_row(
        &format!("SELECT {} FROM chat_contacts WHERE peer_id = ?1", COLONNES),
        params![peer.trim().to_uppercase()],
        |row| ligne_vers_json(row),
    )
    .ok()
}

/// Vrai si `owner` a bloque `interlocuteur` : ses messages ne sont plus acceptes.
pub fn bloquant(owner: &str, interlocuteur: &str) -> bool {
    contact(owner, interlocuteur)
        .and_then(|c| c.get("blocked").and_then(|v| v.as_bool()))
        .unwrap_or(false)
}

pub fn supprimer_contact(uq: &str, peer: &str) -> Result<(), String> {
    let conn = ouvrir(uq)?;
    conn.execute(
        "INSERT INTO chat_contacts (peer_id, added_at, deleted) VALUES (?1, ?2, 1)
         ON CONFLICT(peer_id) DO UPDATE SET deleted = 1",
        params![peer.trim().to_uppercase(), chrono::Utc::now().timestamp()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn restaurer_contact(uq: &str, peer: &str) -> Result<(), String> {
    let conn = ouvrir(uq)?;
    conn.execute(
        "INSERT INTO chat_contacts (peer_id, added_at) VALUES (?1, ?2)
         ON CONFLICT(peer_id) DO UPDATE SET deleted = 0",
        params![peer.trim().to_uppercase(), chrono::Utc::now().timestamp()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn regler(
    uq: &str,
    peer: &str,
    blocked: Option<bool>,
    muted: Option<bool>,
    locked: Option<bool>,
    ephemere: Option<bool>,
) -> Result<Value, String> {
    let conn = ouvrir(uq)?;
    let peer_id = peer.trim().to_uppercase();
    conn.execute(
        "INSERT INTO chat_contacts (peer_id, added_at) VALUES (?1, ?2)
         ON CONFLICT(peer_id) DO NOTHING",
        params![peer_id, chrono::Utc::now().timestamp()],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE chat_contacts SET
            blocked = COALESCE(?2, blocked),
            muted = COALESCE(?3, muted),
            locked = COALESCE(?4, locked),
            ephemere = COALESCE(?5, ephemere)
         WHERE peer_id = ?1",
        params![
            peer_id,
            blocked.map(|value| value as i64),
            muted.map(|value| value as i64),
            locked.map(|value| value as i64),
            ephemere.map(|value| value as i64),
        ],
    )
    .map_err(|e| e.to_string())?;
    contact(uq, &peer_id).ok_or_else(|| "contact introuvable".to_string())
}

/// Vrai si l'expediteur a mis la conversation en auto-destruction : seuls SES
/// messages portent une date de fin, le choix du pair ne lui est pas impose.
pub fn conversation_ephemere(uq: &str, peer: &str) -> bool {
    contact(uq, peer)
        .and_then(|c| c.get("ephemere").and_then(|v| v.as_bool()))
        .unwrap_or(false)
}

// ==================== Routes ====================

/// GET /api/chat/contacts/{user_id}
pub async fn chat_contacts_list(req: HttpRequest, path: web::Path<String>) -> HttpResponse {
    let uq = path.into_inner();
    if let Err(response) = crate::auth_guard::exiger_soi_ou_owner(&req, &uq) {
        return response;
    }
    match liste_contacts(&uq) {
        Ok(contacts) => {
            let total = contacts.len();
            HttpResponse::Ok().json(json!({
                "status": "success",
                "contacts": contacts,
                "total": total,
            }))
        }
        Err(error) => HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": error,
        })),
    }
}

/// POST /api/chat/contacts — { uq, peer_id, action: add|delete|restore,
/// zando_item_id?, zando_item_title? }
pub async fn chat_contact_update(req: HttpRequest, body: web::Json<Value>) -> HttpResponse {
    let uq = body
        .get("uq")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_string();
    let peer = body
        .get("peer_id")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_string();
    let action = body
        .get("action")
        .and_then(|value| value.as_str())
        .unwrap_or("add");
    let item_id = body.get("zando_item_id").and_then(|value| value.as_str());
    let item_titre = body
        .get("zando_item_title")
        .and_then(|value| value.as_str());

    if let Err(response) = crate::auth_guard::exiger_soi_ou_owner(&req, &uq) {
        return response;
    }
    if !uq_valide(&peer) {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "peer_id invalide",
        }));
    }

    let result = match action {
        "delete" => supprimer_contact(&uq, &peer),
        "restore" => restaurer_contact(&uq, &peer),
        _ => {
            ajouter_contact(&uq, &peer, item_id, item_titre);
            Ok(())
        }
    };

    match result {
        Ok(()) => HttpResponse::Ok().json(json!({
            "status": "success",
            "contact": contact(&uq, &peer),
        })),
        Err(error) => HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": error,
        })),
    }
}

/// POST /api/chat/settings — { uq, peer_id, blocked?, muted?, locked? }
pub async fn chat_settings_update(req: HttpRequest, body: web::Json<Value>) -> HttpResponse {
    let uq = body
        .get("uq")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_string();
    let peer = body
        .get("peer_id")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_string();
    if let Err(response) = crate::auth_guard::exiger_soi_ou_owner(&req, &uq) {
        return response;
    }
    if !uq_valide(&peer) {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "peer_id invalide",
        }));
    }

    let flag = |key: &str| body.get(key).and_then(|value| value.as_bool());
    let blocked = flag("blocked");
    let muted = flag("muted");
    let locked = flag("locked");
    let ephemere = flag("ephemere");
    if blocked.is_none() && muted.is_none() && locked.is_none() && ephemere.is_none() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "aucun reglage fourni",
        }));
    }

    match regler(&uq, &peer, blocked, muted, locked, ephemere) {
        Ok(contact) => HttpResponse::Ok().json(json!({
            "status": "success",
            "contact": contact,
        })),
        Err(error) => HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": error,
        })),
    }
}
