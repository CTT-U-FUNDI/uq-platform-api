//! Signalements de conversation, globallement cote plateforme.
//!
//! Le blocage reste local (voir `chat_contacts`) : il protege un seul compte.
//! Le signalement, lui, est enregistre dans le registre central `uq_index.db`,
//! a cote des avis et des abonnes, parce que c'est la moderation qui le traite
//! et que sa suite — la baisse du score de confiance — concerne tout le monde.
//!
//! Regles appliquees au score (voir `profile_routes::score`) :
//! - un signalement CONFIRME par la moderation retire 10 points a la cible,
//!   dans la limite de 30 ;
//! - un compte qui emet 3 signalements REJETES perd 5 points par signalement
//!   abusif, dans la limite de 15 : une erreur isolee n'est pas sanctionnee.

use actix_web::{web, HttpRequest, HttpResponse};
use rusqlite::{params, Connection};
use serde_json::json;

const ANCRE: &str = "uq_index.db";
const CHEMIN_ANCRE_ABSOLU: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/uq_index.db");

/// Le relatif d'abord, comme partout dans ce projet : un bac de test isole peut
/// lire sa propre copie du registre sans jamais toucher a celui de la prod.
const CHEMINS: [&str; 2] = [ANCRE, CHEMIN_ANCRE_ABSOLU];

const TABLE_SQL: &str = "
CREATE TABLE IF NOT EXISTS chat_reports (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    reporter_uq TEXT NOT NULL,
    target_uq TEXT NOT NULL,
    motif TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    preuve TEXT,
    statut TEXT NOT NULL DEFAULT 'en_attente',
    created_at INTEGER NOT NULL,
    reviewed_at INTEGER
);
CREATE INDEX IF NOT EXISTS idx_chat_reports_cible ON chat_reports(target_uq, statut);
CREATE INDEX IF NOT EXISTS idx_chat_reports_emetteur ON chat_reports(reporter_uq, statut);
";

pub const MOTIFS: [&str; 4] = ["spam", "frauduleux", "illegal", "hors_sujet"];
pub const STATUTS: [&str; 3] = ["en_attente", "confirmee", "rejetee"];

const DESCRIPTION_MAX_CAR: usize = 1000;
const PREUVE_MAX_CHARS: usize = 900_000;

fn ouvrir() -> Result<Connection, HttpResponse> {
    for chemin in CHEMINS {
        if !std::path::Path::new(chemin).exists() {
            continue;
        }
        match Connection::open(chemin) {
            Ok(conn) => {
                let _ = conn.execute_batch(TABLE_SQL);
                return Ok(conn);
            }
            Err(erreur) => {
                eprintln!("[SIGNAL] registre {} inutilisable : {}", chemin, erreur);
            }
        }
    }
    Err(HttpResponse::ServiceUnavailable().json(json!({
        "status": "error",
        "message": "registre indisponible",
    })))
}

fn normaliser(uq: &str) -> String {
    uq.trim().to_uppercase()
}

/// Signalement deja ouvert entre les deux memes comptes : la moderation n'a pas
/// besoin de dix lignes identiques, et un compte ne peut pas empiler des motifs
/// pour peser plus lourd.
pub fn signalement_deja_ouvert(conn: &Connection, reporter: &str, cible: &str) -> Option<i64> {
    conn.query_row(
        "SELECT id FROM chat_reports
         WHERE reporter_uq = ?1 AND target_uq = ?2 AND statut = 'en_attente'
         ORDER BY id LIMIT 1",
        params![normaliser(reporter), normaliser(cible)],
        |row| row.get::<_, i64>(0),
    )
    .ok()
}

/// (signalements confirmés contre ce compte, signalements abusifs émis par lui).
/// Le registre des signalements est global (`uq_index.db`) : la base du compte ne
/// les connait pas, c'est ici que le score de confiance doit venir les lire.
pub fn compteurs_signalements(uq: &str) -> (i64, i64) {
    let Ok(conn) = ouvrir() else {
        return (0, 0);
    };
    let cible = normaliser(uq);
    let compter = |sql: &str| -> i64 {
        conn.query_row(sql, params![cible], |row| row.get::<_, i64>(0))
            .unwrap_or(0)
    };
    (
        compter("SELECT COUNT(*) FROM chat_reports WHERE target_uq = ?1 AND statut = 'confirmee'"),
        compter("SELECT COUNT(*) FROM chat_reports WHERE reporter_uq = ?1 AND statut = 'rejetee'"),
    )
}

fn ligne(row: &rusqlite::Row) -> rusqlite::Result<serde_json::Value> {
    Ok(json!({
        "id": row.get::<_, i64>(0)?,
        "reporter_uq": row.get::<_, String>(1)?,
        "target_uq": row.get::<_, String>(2)?,
        "motif": row.get::<_, String>(3)?,
        "description": row.get::<_, String>(4)?,
        "a_une_preuve": row.get::<_, Option<String>>(5)?.is_some(),
        "statut": row.get::<_, String>(6)?,
        "created_at": row.get::<_, i64>(7)?,
        "reviewed_at": row.get::<_, Option<i64>>(8)?,
    }))
}

const COLONNES: &str =
    "id, reporter_uq, target_uq, motif, description, preuve, statut, created_at, reviewed_at";

/// POST /api/chat/report — { cible_uq, motif, description?, preuve? }
pub async fn chat_signaler(req: HttpRequest, body: web::Json<serde_json::Value>) -> HttpResponse {
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(claims) => claims,
        Err(response) => return response,
    };

    let cible = body
        .get("cible_uq")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if !crate::chat_contacts::uq_valide(&cible) {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "cible_uq invalide",
        }));
    }
    if normaliser(&cible) == normaliser(&claims.sub) {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "tu ne peux pas te signaler toi-meme",
        }));
    }

    let motif = body.get("motif").and_then(|v| v.as_str()).unwrap_or("");
    if !MOTIFS.contains(&motif) {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": format!("motif inconnu (attendu: {})", MOTIFS.join(", ")),
        }));
    }

    let description = body
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .chars()
        .take(DESCRIPTION_MAX_CAR)
        .collect::<String>();

    // La preuve est facultative et doit rester une image locale, jamais une URL
    // vers un service tiers.
    let preuve = match body.get("preuve").and_then(|v| v.as_str()) {
        Some(value) => {
            let value = value.trim();
            if value.is_empty() {
                None
            } else if !value.starts_with("data:image/") || value.len() > PREUVE_MAX_CHARS {
                return HttpResponse::BadRequest().json(json!({
                    "status": "error",
                    "message": "preuve invalide (image locale, max 650 Ko)",
                }));
            } else {
                Some(value.to_string())
            }
        }
        None => None,
    };

    let conn = match ouvrir() {
        Ok(conn) => conn,
        Err(response) => return response,
    };

    if let Some(id) = signalement_deja_ouvert(&conn, &claims.sub, &cible) {
        return HttpResponse::Conflict().json(json!({
            "status": "duplique",
            "id": id,
            "message": "un signalement est deja en cours pour ce compte",
        }));
    }

    let resultat = conn.execute(
        "INSERT INTO chat_reports (reporter_uq, target_uq, motif, description, preuve, statut, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'en_attente', ?6)",
        params![
            normaliser(&claims.sub),
            normaliser(&cible),
            motif,
            description,
            preuve,
            chrono::Utc::now().timestamp()
        ],
    );
    match resultat {
        Ok(_) => {
            let id = conn.last_insert_rowid();
            HttpResponse::Ok().json(json!({
                "status": "success",
                "id": id,
                "statut": "en_attente",
                "message": "signalement enregistre, la moderation le traite",
            }))
        }
        Err(erreur) => {
            eprintln!("[SIGNAL] ecriture impossible : {}", erreur);
            HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": "signalement non enregistre",
            }))
        }
    }
}

/// GET /api/chat/reports — file de moderation, reservee aux roles reserves.
/// Filtre facultatif `?statut=confirmee|rejetee|en_attente`.
pub async fn chat_signalements_list(
    req: HttpRequest,
    query: web::Query<std::collections::HashMap<String, String>>,
) -> HttpResponse {
    if let Err(response) = crate::auth_guard::exiger_owner(&req) {
        return response;
    }
    let statut = query.get("statut").map(|v| v.as_str()).unwrap_or("");
    if !statut.is_empty() && !STATUTS.contains(&statut) {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": format!("statut inconnu (attendu: {})", STATUTS.join(", ")),
        }));
    }

    let conn = match ouvrir() {
        Ok(conn) => conn,
        Err(response) => return response,
    };
    let sql = if statut.is_empty() {
        format!(
            "SELECT {} FROM chat_reports ORDER BY created_at DESC LIMIT 200",
            COLONNES
        )
    } else {
        format!(
            "SELECT {} FROM chat_reports WHERE statut = ?1 ORDER BY created_at DESC LIMIT 200",
            COLONNES
        )
    };
    let sortie = if statut.is_empty() {
        conn.prepare(&sql).and_then(|mut stmt| {
            stmt.query_map([], |row| ligne(row))?
                .collect::<Result<Vec<_>, _>>()
        })
    } else {
        conn.prepare(&sql).and_then(|mut stmt| {
            stmt.query_map(params![statut], |row| ligne(row))?
                .collect::<Result<Vec<_>, _>>()
        })
    };
    match sortie {
        Ok(signalements) => {
            let total = signalements.len();
            HttpResponse::Ok().json(json!({
                "status": "success",
                "signalements": signalements,
                "total": total,
            }))
        }
        Err(erreur) => HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": erreur.to_string(),
        })),
    }
}

/// POST /api/chat/reports/{id}/review — { statut: confirmee | rejetee }.
/// C'est cette decision qui bouge le score, jamais le signalement brut.
pub async fn chat_signalement_review(
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let claims = match crate::auth_guard::exiger_owner(&req) {
        Ok(claims) => claims,
        Err(response) => return response,
    };
    let id = path.into_inner();
    let statut = body.get("statut").and_then(|v| v.as_str()).unwrap_or("");
    if statut != "confirmee" && statut != "rejetee" {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "statut attendu: confirmee ou rejetee",
        }));
    }

    let conn = match ouvrir() {
        Ok(conn) => conn,
        Err(response) => return response,
    };
    match conn.execute(
        "UPDATE chat_reports SET statut = ?1, reviewed_at = ?2 WHERE id = ?3",
        params![statut, chrono::Utc::now().timestamp(), id],
    ) {
        Ok(1..) => HttpResponse::Ok().json(json!({
            "status": "success",
            "id": id,
            "statut": statut,
            "traite_par": claims.sub,
        })),
        Ok(_) => HttpResponse::NotFound().json(json!({
            "status": "error",
            "message": "signalement introuvable",
        })),
        Err(erreur) => HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": erreur.to_string(),
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bac() -> Connection {
        let conn = Connection::open_in_memory().expect("base memoire");
        conn.execute_batch(TABLE_SQL).expect("schema");
        conn
    }

    fn inserer(conn: &Connection, reporter: &str, cible: &str, motif: &str, statut: &str) -> i64 {
        conn.execute(
            "INSERT INTO chat_reports (reporter_uq, target_uq, motif, description, statut, created_at)
             VALUES (?1, ?2, ?3, '', ?4, 1)",
            params![reporter, cible, motif, statut],
        )
        .expect("insert");
        conn.last_insert_rowid()
    }

    #[test]
    fn un_signalement_ouvert_est_retrouve() {
        let conn = bac();
        let id = inserer(&conn, "UQ-111111111", "UQ-222222222", "spam", "en_attente");
        assert_eq!(
            signalement_deja_ouvert(&conn, "uq-111111111", "UQ-222222222"),
            Some(id),
            "la casse de la saisie ne doit pas dupliquer le dossier"
        );
        assert_eq!(
            signalement_deja_ouvert(&conn, "UQ-333333333", "UQ-222222222"),
            None
        );
    }

    #[test]
    fn un_dossier_ferme_ne_bloque_plus_un_nouveau_signalement() {
        let conn = bac();
        inserer(&conn, "UQ-111111111", "UQ-222222222", "spam", "confirmee");
        assert_eq!(
            signalement_deja_ouvert(&conn, "UQ-111111111", "UQ-222222222"),
            None,
            "un dossier ferme doit permettre de signaler a nouveau"
        );
    }

    #[test]
    fn les_motifs_sont_ceux_du_cahier_des_charges() {
        assert_eq!(MOTIFS, ["spam", "frauduleux", "illegal", "hors_sujet"]);
    }
}
