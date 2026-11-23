// ============================================================================
// network_routes.rs — Systeme de reseau (demandes d'amitie, 1 USD)
// ============================================================================

use actix_web::{web, HttpRequest, HttpResponse};
use rusqlite::params;
use serde_json::json;
use uuid::Uuid;

use crate::auth_guard::exiger_auth;
use crate::database::Database;

// Montant en points (1 USD = 100 points, si 1 BORB = 100 points)
const MONTANT_POINTS: i64 = 100;
const DUREE_DEMANDE_SECS: i64 = 12 * 3600; // 12h

/// Ouvre la base d'un compte
fn ouvrir_base(uq: &str) -> Result<rusqlite::Connection, String> {
    let chemin = format!("data/users/{}.db", uq.replace("-", "_").to_lowercase());
    rusqlite::Connection::open(&chemin).map_err(|e| format!("Base {} : {}", chemin, e))
}

/// Envoyer une demande d'amitie (PIN du demandeur)
pub async fn request(req: HttpRequest, body: web::Json<serde_json::Value>) -> HttpResponse {
    let claims = match exiger_auth(&req) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let demandeur = claims.sub;
    let destinataire = body
        .get("destinataire")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let pin = body.get("pin").and_then(|v| v.as_str()).unwrap_or("");

    if destinataire.is_empty() || pin.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error", "message": "destinataire et pin obligatoires"
        }));
    }

    if demandeur == destinataire {
        return HttpResponse::BadRequest().json(json!({
            "status": "error", "message": "Vous ne pouvez pas vous ajouter vous-meme"
        }));
    }

    let db = Database::shared("uq_platform_db.json");
    let conn = match db.conn() {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error", "message": e.to_string()
            }))
        }
    };

    // 1. Verifier le PIN du demandeur
    if let Err(e) = verifier_pin(&demandeur, pin) {
        return HttpResponse::Unauthorized().json(json!({
            "status": "error", "message": e
        }));
    }

    // 2. Verifier qu'il n'y a pas deja une demande en attente
    let deja: i64 = conn.query_row(
        "SELECT COUNT(*) FROM followers WHERE uq_follower = ?1 AND uq_followed = ?2 AND status = 'pending'",
        params![demandeur, destinataire],
        |row| row.get(0),
    ).unwrap_or(0);

    if deja > 0 {
        return HttpResponse::BadRequest().json(json!({
            "status": "error", "message": "Demande deja en attente"
        }));
    }

    // 3. Verifier que l'amitie n'existe pas deja
    let deja_ami: i64 = conn.query_row(
        "SELECT COUNT(*) FROM followers WHERE uq_follower = ?1 AND uq_followed = ?2 AND status = 'accepted'",
        params![demandeur, destinataire],
        |row| row.get(0),
    ).unwrap_or(0);

    if deja_ami > 0 {
        return HttpResponse::BadRequest().json(json!({
            "status": "error", "message": "Deja ami"
        }));
    }

    // 4. Geler 1 USD dans la base du demandeur
    let base_demandeur = match ouvrir_base(&demandeur) {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error", "message": e
            }))
        }
    };

    let solde: i64 = base_demandeur
        .query_row("SELECT points FROM balance WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap_or(0);

    if solde < MONTANT_POINTS {
        return HttpResponse::BadRequest().json(json!({
            "status": "error", "message": "Solde insuffisant (1 USD = 100 points)"
        }));
    }

    if let Err(e) = base_demandeur.execute(
        "UPDATE balance SET points = points - ?1, frozen_points = frozen_points + ?1 WHERE id = 1",
        params![MONTANT_POINTS],
    ) {
        return HttpResponse::InternalServerError().json(json!({
            "status": "error", "message": format!("Erreur gel: {}", e)
        }));
    }

    // 5. Creer la demande
    let now = chrono::Utc::now().timestamp();
    let expires = now + DUREE_DEMANDE_SECS;
    let id = Uuid::new_v4().to_string();

    let _ = conn.execute(
        "INSERT INTO followers (uq_follower, uq_followed, created_at, status, amount_usd, expires_at, paid_by)
         VALUES (?1, ?2, ?3, 'pending', 1.0, ?4, ?1)",
        params![demandeur, destinataire, now, expires],
    );

    HttpResponse::Ok().json(json!({
        "status": "success",
        "demande_id": id,
        "message": "Demande envoyee. 1 USD gele. Expire dans 12h."
    }))
}

/// Accepter une demande (PIN du destinataire)
pub async fn accept(req: HttpRequest, body: web::Json<serde_json::Value>) -> HttpResponse {
    let claims = match exiger_auth(&req) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let destinataire = claims.sub;
    let demandeur = body.get("demandeur").and_then(|v| v.as_str()).unwrap_or("");
    let pin = body.get("pin").and_then(|v| v.as_str()).unwrap_or("");

    if demandeur.is_empty() || pin.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error", "message": "demandeur et pin obligatoires"
        }));
    }

    let db = Database::shared("uq_platform_db.json");
    let conn = match db.conn() {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error", "message": e.to_string()
            }))
        }
    };

    // 1. Verifier PIN du destinataire
    if let Err(e) = verifier_pin(&destinataire, pin) {
        return HttpResponse::Unauthorized().json(json!({
            "status": "error", "message": e
        }));
    }

    // 2. Verifier que la demande existe et n'est pas expiree
    let (expires_at,): (i64,) = match conn.query_row(
        "SELECT expires_at FROM followers WHERE uq_follower = ?1 AND uq_followed = ?2 AND status = 'pending'",
        params![demandeur, destinataire],
        |row| Ok((row.get(0)?,)),
    ) {
        Ok(r) => r,
        Err(_) => return HttpResponse::NotFound().json(json!({
            "status": "error", "message": "Demande introuvable"
        })),
    };

    if chrono::Utc::now().timestamp() > expires_at {
        // Rembourser
        let _ = rembourser(&demandeur);
        let _ = conn.execute(
            "UPDATE followers SET status = 'expired' WHERE uq_follower = ?1 AND uq_followed = ?2 AND status = 'pending'",
            params![demandeur, destinataire],
        );
        return HttpResponse::BadRequest().json(json!({
            "status": "error", "message": "Demande expiree (1 USD rembourse)"
        }));
    }

    // 3. Transferer le 1 USD : degeler chez A, crediter chez B
    if let Err(e) = transferer_montant(&demandeur, &destinataire) {
        return HttpResponse::InternalServerError().json(json!({
            "status": "error", "message": format!("Erreur transfert: {}", e)
        }));
    }

    // 4. Marquer comme acceptee
    let _ = conn.execute(
        "UPDATE followers SET status = 'accepted' WHERE uq_follower = ?1 AND uq_followed = ?2 AND status = 'pending'",
        params![demandeur, destinataire],
    );

    HttpResponse::Ok().json(json!({
        "status": "success",
        "message": "Amitie acceptee. 1 USD transfere."
    }))
}

/// Refuser une demande (PIN du destinataire)
pub async fn reject(req: HttpRequest, body: web::Json<serde_json::Value>) -> HttpResponse {
    let claims = match exiger_auth(&req) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let destinataire = claims.sub;
    let demandeur = body.get("demandeur").and_then(|v| v.as_str()).unwrap_or("");
    let pin = body.get("pin").and_then(|v| v.as_str()).unwrap_or("");

    if demandeur.is_empty() || pin.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error", "message": "demandeur et pin obligatoires"
        }));
    }

    if let Err(e) = verifier_pin(&destinataire, pin) {
        return HttpResponse::Unauthorized().json(json!({
            "status": "error", "message": e
        }));
    }

    // Rembourser le 1 USD
    let _ = rembourser(&demandeur);

    let db = Database::shared("uq_platform_db.json");
    let conn = match db.conn() {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error", "message": e.to_string()
            }))
        }
    };

    let _ = conn.execute(
        "UPDATE followers SET status = 'rejected' WHERE uq_follower = ?1 AND uq_followed = ?2 AND status = 'pending'",
        params![demandeur, destinataire],
    );

    HttpResponse::Ok().json(json!({
        "status": "success",
        "message": "Demande refusee. 1 USD rembourse."
    }))
}

/// Quitter une amitie (PIN de celui qui quitte)
pub async fn leave(req: HttpRequest, body: web::Json<serde_json::Value>) -> HttpResponse {
    let claims = match exiger_auth(&req) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let quittant = claims.sub;
    let autre = body.get("autre").and_then(|v| v.as_str()).unwrap_or("");
    let pin = body.get("pin").and_then(|v| v.as_str()).unwrap_or("");

    if autre.is_empty() || pin.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error", "message": "autre et pin obligatoires"
        }));
    }

    if let Err(e) = verifier_pin(&quittant, pin) {
        return HttpResponse::Unauthorized().json(json!({
            "status": "error", "message": e
        }));
    }

    let db = Database::shared("uq_platform_db.json");
    let conn = match db.conn() {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error", "message": e.to_string()
            }))
        }
    };

    // Verifier que l'amitie existe
    let existe: i64 = conn.query_row(
        "SELECT COUNT(*) FROM followers WHERE uq_follower = ?1 AND uq_followed = ?2 AND status = 'accepted'",
        params![quittant, autre],
        |row| row.get(0),
    ).unwrap_or(0);

    if existe == 0 {
        return HttpResponse::NotFound().json(json!({
            "status": "error", "message": "Amitie introuvable"
        }));
    }

    // Rembourser 1 USD
    if let Err(e) = transferer_montant(&quittant, &autre) {
        return HttpResponse::InternalServerError().json(json!({
            "status": "error", "message": format!("Erreur remboursement: {}", e)
        }));
    }

    // Supprimer l'amitie
    let _ = conn.execute(
        "DELETE FROM followers WHERE uq_follower = ?1 AND uq_followed = ?2 AND status = 'accepted'",
        params![quittant, autre],
    );

    HttpResponse::Ok().json(json!({
        "status": "success",
        "message": "Amitie quittee. 1 USD rembourse."
    }))
}

/// Lister les demandes recues et envoyees
pub async fn list(req: HttpRequest) -> HttpResponse {
    let claims = match exiger_auth(&req) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let uq = claims.sub;

    let db = Database::shared("uq_platform_db.json");
    let conn = match db.conn() {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error", "message": e.to_string()
            }))
        }
    };

    // Recues
    let mut stmt = match conn.prepare(
        "SELECT uq_follower, created_at, expires_at FROM followers
         WHERE uq_followed = ?1 AND status = 'pending' ORDER BY created_at DESC",
    ) {
        Ok(s) => s,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error", "message": e.to_string()
            }))
        }
    };
    let recues: Vec<serde_json::Value> = stmt
        .query_map(params![uq], |row| {
            Ok(json!({
                "de": row.get::<_, String>(0)?,
                "created_at": row.get::<_, i64>(1)?,
                "expires_at": row.get::<_, i64>(2)?,
            }))
        })
        .map(|r| r.filter_map(|x| x.ok()).collect::<Vec<_>>())
        .unwrap_or_default();

    // Envoyees
    let mut stmt = match conn.prepare(
        "SELECT uq_followed, created_at, expires_at FROM followers
         WHERE uq_follower = ?1 AND status = 'pending' ORDER BY created_at DESC",
    ) {
        Ok(s) => s,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error", "message": e.to_string()
            }))
        }
    };
    let envoyees: Vec<serde_json::Value> = stmt
        .query_map(params![uq], |row| {
            Ok(json!({
                "vers": row.get::<_, String>(0)?,
                "created_at": row.get::<_, i64>(1)?,
                "expires_at": row.get::<_, i64>(2)?,
            }))
        })
        .map(|r| r.filter_map(|x| x.ok()).collect::<Vec<_>>())
        .unwrap_or_default();

    HttpResponse::Ok().json(json!({
        "status": "success",
        "recues": recues,
        "envoyees": envoyees,
    }))
}

// === Fonctions internes ===

fn verifier_pin(uq: &str, pin: &str) -> Result<(), String> {
    // Utiliser la fonction existante de verification de PIN
    let base = ouvrir_base(uq)?;
    let hash_stocke: String = base
        .query_row(
            "SELECT login_pin_hash FROM settings WHERE key = 'login_pin_hash'",
            [],
            |row| row.get(0),
        )
        .map_err(|_| "PIN non configure".to_string())?;

    if hash_stocke.is_empty() {
        return Err("PIN non configure".to_string());
    }

    // Calculer le hash du PIN fourni
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(format!("UQ_PIN_V2:{}:{}", pin, uq).as_bytes());
    let hash_fourni = format!("{:x}", hasher.finalize());

    if hash_fourni != hash_stocke {
        return Err("PIN incorrect".to_string());
    }

    Ok(())
}

fn rembourser(uq: &str) -> Result<(), String> {
    let base = ouvrir_base(uq)?;
    base.execute(
        "UPDATE balance SET points = points + ?1, frozen_points = frozen_points - ?1 WHERE id = 1",
        params![MONTANT_POINTS],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn transferer_montant(de: &str, vers: &str) -> Result<(), String> {
    let base_de = ouvrir_base(de)?;
    let base_vers = ouvrir_base(vers)?;

    // Degeler chez de
    base_de
        .execute(
            "UPDATE balance SET frozen_points = frozen_points - ?1 WHERE id = 1",
            params![MONTANT_POINTS],
        )
        .map_err(|e| e.to_string())?;

    // Crediter chez vers
    base_vers
        .execute(
            "UPDATE balance SET points = points + ?1 WHERE id = 1",
            params![MONTANT_POINTS],
        )
        .map_err(|e| e.to_string())?;

    Ok(())
}

// ============================================================================
// JOB : EXPIRATION DES DEMANDES (12h)
// ============================================================================

/// Expire les demandes de plus de 12h et rembourse le 1 USD.
pub fn expirer_demandes() -> Result<Vec<String>, String> {
    let db = Database::shared("uq_platform_db.json");
    let conn = db.conn().map_err(|e| e.to_string())?;

    let now = chrono::Utc::now().timestamp();

    let mut stmt = conn
        .prepare(
            "SELECT id, uq_follower, uq_followed FROM followers
         WHERE status = 'pending' AND expires_at < ?1",
        )
        .map_err(|e| e.to_string())?;

    let demandes: Vec<(i64, String, String)> = stmt
        .query_map(params![now], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    let mut expirees = Vec::new();

    for (id, demandeur, _destinataire) in demandes {
        if let Ok(base) = ouvrir_base(&demandeur) {
            let _ = base.execute(
                "UPDATE balance SET points = points + ?1, frozen_points = frozen_points - ?1 WHERE id = 1",
                params![MONTANT_POINTS],
            );
        }

        let _ = conn.execute(
            "UPDATE followers SET status = 'expired' WHERE id = ?1",
            params![id],
        );

        expirees.push(id.to_string());
    }

    Ok(expirees)
}
