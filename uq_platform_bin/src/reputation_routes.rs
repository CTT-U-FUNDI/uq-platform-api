// ============================================================================
// reputation_routes.rs — Systeme de reputation a 5 couleurs
// ============================================================================

use actix_web::{web, HttpRequest, HttpResponse};
use rusqlite::params;
use serde_json::json;

use crate::auth_guard::exiger_auth;
use crate::database::Database;
use uuid::Uuid;

/// GET /api/reputation/{uq} — retourne les 5 couleurs
pub async fn get_reputation(path: web::Path<String>) -> HttpResponse {
    let uq = path.into_inner();
    let db = Database::shared("uq_platform_db.json");
    let conn = match db.conn() {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": e.to_string()
            }))
        }
    };

    match conn.query_row(
        "SELECT vert, blanc, orange, rouge, noir, vert_depuis_rouge FROM reputation WHERE uq = ?1",
        params![uq],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
            ))
        },
    ) {
        Ok((vert, blanc, orange, rouge, noir, vert_depuis_rouge)) => {
            HttpResponse::Ok().json(json!({
                "status": "success",
                "vert": vert,
                "blanc": blanc,
                "orange": orange,
                "rouge": rouge,
                "noir": noir,
                "vert_depuis_rouge": vert_depuis_rouge,
            }))
        }
        Err(_) => HttpResponse::Ok().json(json!({
            "status": "success",
            "vert": 0, "blanc": 0, "orange": 0, "rouge": 0, "noir": 0,
            "vert_depuis_rouge": 0,
        })),
    }
}

/// POST /api/reputation/{uq}/action — ajoute +1 Vert, +1 Blanc, +1 vert_depuis_rouge
pub async fn add_action(req: HttpRequest, path: web::Path<String>) -> HttpResponse {
    let claims = match exiger_auth(&req) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let uq = path.into_inner();
    // 🔐 Anti-auto-attribution : personne ne peut se donner des Verts
    // à soi-même — sinon un compte farmerait sa réputation en boucle.
    if claims.sub == uq {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Impossible de s'attribuer une action de reputation."
        }));
    }
    let db = Database::shared("uq_platform_db.json");
    let conn = match db.conn() {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": e.to_string()
            }))
        }
    };

    // 1. +1 Vert, +1 Blanc, +1 vert_depuis_rouge
    let _ = conn.execute(
        "INSERT INTO reputation (uq, vert, blanc, vert_depuis_rouge)
         VALUES (?1, 1, 1, 1)
         ON CONFLICT(uq) DO UPDATE SET
           vert = vert + 1,
           blanc = blanc + 1,
           vert_depuis_rouge = vert_depuis_rouge + 1,
           updated_at = datetime('now')",
        params![uq],
    );

    // 2. Si vert_depuis_rouge >= 10 et rouge > 0, effacer 1 Rouge
    let _ = conn.execute(
        "UPDATE reputation SET
           rouge = CASE WHEN vert_depuis_rouge >= 10 AND rouge > 0 THEN rouge - 1 ELSE rouge END,
           vert_depuis_rouge = CASE WHEN vert_depuis_rouge >= 10 THEN 0 ELSE vert_depuis_rouge END
         WHERE uq = ?1",
        params![uq],
    );

    HttpResponse::Ok().json(json!({"status": "success"}))
}

/// POST /api/reputation/{uq}/report — ajoute +1 Orange

/// POST /api/reputation/{uq}/verify — Orange -> disparait (faux) ou devient Rouge + Noir (vrai)
pub async fn verify_report(
    req: HttpRequest,
    path: web::Path<String>,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    // 🔐 OWNER UNIQUEMENT : `claims` était récupéré puis ignoré —
    // n'importe quel utilisateur pouvait transformer un Orange en
    // Rouge+Noir sur N'IMPORTE QUEL compte (sanction arbitraire).
    let _claims = match crate::auth_guard::exiger_owner(&req) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let uq = path.into_inner();
    let coupable = body
        .get("coupable")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let db = Database::shared("uq_platform_db.json");
    let conn = match db.conn() {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": e.to_string()
            }))
        }
    };

    if coupable {
        // -1 Orange, +1 Rouge, +1 Noir, -10 Vert
        let _ = conn.execute(
            "UPDATE reputation SET
               orange = CASE WHEN orange > 0 THEN orange - 1 ELSE 0 END,
               rouge = rouge + 1,
               noir = noir + 1,
               vert = CASE WHEN vert >= 10 THEN vert - 10 ELSE 0 END,
               vert_depuis_rouge = 0,
               updated_at = datetime('now')
             WHERE uq = ?1",
            params![uq],
        );
    } else {
        // -1 Orange
        let _ = conn.execute(
            "UPDATE reputation SET
               orange = CASE WHEN orange > 0 THEN orange - 1 ELSE 0 END,
               updated_at = datetime('now')
             WHERE uq = ?1",
            params![uq],
        );
    }

    HttpResponse::Ok().json(json!({"status": "success"}))
}

// ============================================================================
// ANALYSE DE L'HISTORIQUE
// ============================================================================

pub enum AnalyseResult {
    Refuse(&'static str),
    AccusateurSuspect(usize),
    AccusationNonFondee,
    Credible {
        total: usize,
        echecs: usize,
        en_cours: usize,
    },
}

/// Analyse l'historique recu du client (7 derniers jours).
pub fn analyser_historique(historique: &serde_json::Value) -> AnalyseResult {
    let transactions = historique
        .get("transactions")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let p2p = historique
        .get("p2p")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let achats = historique
        .get("achats")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let messages = historique
        .get("messages")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let total = transactions.len() + p2p.len() + achats.len() + messages.len();

    if total == 0 {
        return AnalyseResult::Refuse("Aucun historique avec ce profil sur 7 jours.");
    }

    let mut succes = 0;
    let mut echecs = 0;
    let mut en_cours = 0;

    let mut compter = |tx: &serde_json::Value| match tx.get("statut").and_then(|v| v.as_str()) {
        Some("succes") => succes += 1,
        Some("echec") => echecs += 1,
        _ => en_cours += 1,
    };

    for tx in &transactions {
        compter(tx);
    }
    for tx in &p2p {
        compter(tx);
    }
    for tx in &achats {
        compter(tx);
    }

    if succes == total {
        return AnalyseResult::AccusateurSuspect(total);
    }

    AnalyseResult::Credible {
        total,
        echecs,
        en_cours,
    }
}

/// Verifie si un utilisateur a un historique reel avec l'un des deux.
pub fn verifier_historique_reel(
    conn: &rusqlite::Connection,
    uq: &str,
    accusateur: &str,
    accuse: &str,
) -> Result<bool, String> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transactions
         WHERE (sender = ?1 AND recipient = ?2) OR (sender = ?2 AND recipient = ?1)
           AND timestamp >= ?3",
            rusqlite::params![uq, accusateur, chrono::Utc::now().timestamp() - 7 * 86400],
            |row| row.get(0),
        )
        .unwrap_or(0);

    if count > 0 {
        return Ok(true);
    }

    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transactions
         WHERE (sender = ?1 AND recipient = ?2) OR (sender = ?2 AND recipient = ?1)
           AND timestamp >= ?3",
            rusqlite::params![uq, accuse, chrono::Utc::now().timestamp() - 7 * 86400],
            |row| row.get(0),
        )
        .unwrap_or(0);

    Ok(count > 0)
}

/// Interroge le nœud BORB local : GET {BORB_NODE_URL}/historique/{uq}.
/// Retourne None si le nœud est injoignable (mode dégradé : la
/// cohérence repose alors sur la SQLite seule — jamais bloquant).
async fn historique_noeud(uq: &str) -> Option<serde_json::Value> {
    let base = std::env::var("BORB_NODE_URL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "https://uq-borb-node.onrender.com".to_string());
    let url = format!("{}/historique/{}", base.trim_end_matches('/'), uq);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(4))
        .build()
        .ok()?;
    let rep = client.get(&url).send().await.ok()?;
    if !rep.status().is_success() {
        return None;
    }
    rep.json::<serde_json::Value>().await.ok()
}

/// POST /api/reputation/report -- signalement avec historique
pub async fn add_report(
    req: actix_web::HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let claims = match exiger_auth(&req) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let accusateur = claims.sub;
    let accuse = body.get("accuse").and_then(|v| v.as_str()).unwrap_or("");
    let texte = body.get("texte").and_then(|v| v.as_str()).unwrap_or("");
    let historique = body
        .get("historique")
        .cloned()
        .unwrap_or(serde_json::json!({}));
    let type_litige = body
        .get("type_litige")
        .and_then(|v| v.as_str())
        .unwrap_or("general");

    if accuse.is_empty() || texte.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Accuse et texte obligatoires."
        }));
    }

    let db = Database::shared("uq_platform_db.json");
    let conn = match db.conn() {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": e.to_string()
            }))
        }
    };

    let historique_recent =
        verifier_historique_reel(&conn, &accuse, &accusateur, &accuse).unwrap_or(false);

    // ===== COHÉRENCE CROSS-NŒUD =====
    // L'historique envoyé par le client est falsifiable : on le compare
    // aux données RÉELLES — SQLite locale (transactions < 7 j entre les
    // deux UQ) + historique local du nœud BORB. Un client qui gonfle
    // massivement ses chiffres est refusé et passe à l'orange.
    // Tolérance large (×3 + 10) : le client peut légitimement agréger
    // des sources que le serveur ne voit pas toutes.
    let total_client: usize = ["transactions", "p2p", "achats", "messages"]
        .iter()
        .map(|k| {
            historique
                .get(*k)
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0)
        })
        .sum();
    if total_client > 0 {
        // Source 1 : SQLite locale — transactions réelles < 7 j.
        let transactions_reelles: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM transactions
                 WHERE ((sender = ?1 AND recipient = ?2)
                     OR (sender = ?2 AND recipient = ?1))
                   AND timestamp >= ?3",
                rusqlite::params![
                    accusateur,
                    accuse,
                    chrono::Utc::now().timestamp() - 7 * 86400
                ],
                |row| row.get(0),
            )
            .unwrap_or(0);
        // Source 2 : nœud BORB local — historique propre au nœud.
        let noeud_total = historique_noeud(&accuse)
            .await
            .map(|j| {
                j.get("total_local")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as i64
            })
            .unwrap_or(0);
        let total_reel = transactions_reelles + noeud_total;
        let plafond = (total_reel * 3 + 10) as usize;
        if total_client > plafond {
            let _ = conn.execute(
                "INSERT INTO reputation (uq, orange) VALUES (?1, 1)
                 ON CONFLICT(uq) DO UPDATE SET orange = orange + 1, updated_at = datetime('now')",
                params![accusateur],
            );
            return HttpResponse::BadRequest().json(json!({
                "status": "accusation_non_fondée",
                "message": "Historique declare incoherent avec les donnees des noeuds. Signalement refuse. Un orange a ete ajoute a votre compte."
            }));
        }
    }

    if !historique_recent {
        let _ = conn.execute(
            "INSERT INTO reputation (uq, orange) VALUES (?1, 1)
             ON CONFLICT(uq) DO UPDATE SET orange = orange + 1, updated_at = datetime('now')",
            params![accusateur],
        );
        return HttpResponse::BadRequest().json(json!({
            "status": "accusation_non_fondée",
            "message": "Aucune operation recente (< 7 jours) avec ce membre. Signalement refuse. Un orange a ete ajoute a votre compte."
        }));
    }

    match analyser_historique(&historique) {
        AnalyseResult::Refuse(msg) => HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": msg
        })),
        AnalyseResult::AccusationNonFondee => HttpResponse::BadRequest().json(json!({
            "status": "accusation_non_fondée",
            "message": "Signalement non fonde."
        })),
        AnalyseResult::AccusateurSuspect(total) => {
            let _ = conn.execute(
                "INSERT INTO reputation (uq, orange) VALUES (?1, 1)
                 ON CONFLICT(uq) DO UPDATE SET orange = orange + 1, updated_at = datetime('now')",
                params![accusateur],
            );
            HttpResponse::Ok().json(json!({
                "status": "suspect",
                "message": "Signalement suspect : toutes les actions sont des succes. L'accusateur est place en Orange.",
                "total": total
            }))
        }
        AnalyseResult::Credible {
            total,
            echecs,
            en_cours,
        } => {
            let dossier_id = format!("dossier_{}", Uuid::new_v4());
            let _ = conn.execute(
                "INSERT INTO dossiers (id, accusateur, accuse, texte_accusation, historique, nb_evenements, echecs, en_cours, type_litige)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![dossier_id, accusateur, accuse, texte, historique.to_string(), total as i64, echecs as i64, en_cours as i64, type_litige],
            );
            let _ = conn.execute(
                "INSERT INTO reputation (uq, orange) VALUES (?1, 1)
                 ON CONFLICT(uq) DO UPDATE SET orange = orange + 1, updated_at = datetime('now')",
                params![accuse],
            );
            HttpResponse::Ok().json(json!({
                "status": "success",
                "dossier_id": dossier_id,
                "nb_evenements": total,
                "echecs": echecs,
                "en_cours": en_cours,
                "message": "Signalement credibles. Le dossier est public."
            }))
        }
    }
}

// ============================================================================
// RACHAT ET BLOCAGES
// ============================================================================

/// Incremente le compteur de rachat et leve le blocage si 10 rouges effaces.
pub fn achat_reussi(conn: &rusqlite::Connection, uq: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO reputation (uq, vert, blanc, vert_depuis_rouge)
         VALUES (?1, 1, 1, 1)
         ON CONFLICT(uq) DO UPDATE SET
           vert = COALESCE(reputation.vert, 0) + 1,
           vert_depuis_rouge = COALESCE(reputation.vert_depuis_rouge, 0) + 1,
           blanc = CASE
               WHEN COALESCE(reputation.rouge, 0) = 0 THEN COALESCE(reputation.blanc, 0) + 1
               WHEN COALESCE(reputation.vert, 0) + 1 >= COALESCE(reputation.vert_seuil_sanction, 0)
                    THEN COALESCE(reputation.blanc, 0) + 1
               ELSE COALESCE(reputation.blanc, 0)
           END,
           updated_at = datetime('now')",
        rusqlite::params![uq],
    )
    .map_err(|e| e.to_string())?;

    conn.execute(
        "UPDATE reputation SET
           rouge = CASE WHEN vert_depuis_rouge >= 10 AND rouge > 0 THEN rouge - 1 ELSE rouge END,
           vert_depuis_rouge = CASE WHEN vert_depuis_rouge >= 10 THEN 0 ELSE vert_depuis_rouge END
         WHERE uq = ?1",
        rusqlite::params![uq],
    )
    .map_err(|e| e.to_string())?;

    let rouge: i64 = conn
        .query_row(
            "SELECT COALESCE(rouge, 0) FROM reputation WHERE uq = ?1",
            rusqlite::params![uq],
            |row| row.get(0),
        )
        .unwrap_or(0);

    if rouge == 0 {
        conn.execute(
            "UPDATE accounts SET
               p2p_blocked = 0,
               zando_blocked = 0,
               retrait_blocked = 0
             WHERE uq_number = ?1",
            rusqlite::params![uq],
        )
        .map_err(|e| e.to_string())?;
        // Q17 — leve aussi les blocages marketplace/livraison (best effort :
        // les colonnes sont créées par marketplace::ensure_tables).
        let _ = conn.execute(
            "UPDATE accounts SET marketplace_blocked = 0, livraison_blocked = 0
             WHERE uq_number = ?1",
            rusqlite::params![uq],
        );
        let _ = conn.execute(
            "UPDATE reputation SET vert_seuil_sanction = 0 WHERE uq = ?1",
            rusqlite::params![uq],
        );
    }

    Ok(())
}

/// Verifie si un utilisateur est bloque pour une operation donnee.
pub fn est_bloque(conn: &rusqlite::Connection, uq: &str, operation: &str) -> bool {
    let (p2p, zando, retrait, mkt, livraison): (i64, i64, i64, i64, i64) = conn
        .query_row(
            "SELECT
            COALESCE(p2p_blocked, 0),
            COALESCE(zando_blocked, 0),
            COALESCE(retrait_blocked, 0),
            COALESCE(marketplace_blocked, 0),
            COALESCE(livraison_blocked, 0)
         FROM accounts WHERE uq_number = ?1",
            rusqlite::params![uq],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .unwrap_or((0, 0, 0, 0, 0));

    match operation {
        "p2p_vendre" | "p2p_creer_offre" => p2p == 1,
        "zando_publier" | "zando_vendre" => zando == 1,
        "retrait" => retrait == 1,
        "marketplace" | "marketplace_vendre" | "marketplace_acheter" => mkt == 1,
        "livraison" => livraison == 1,
        _ => false,
    }
}

/// POST /api/reputation/{uq}/achat -- simuler un achat reussi (test)
/// ⚠️ OWNER UNIQUEMENT : sans ce garde, n'importe qui s'attribuait
/// des Verts et levait ses blocages p2p/zando/retrait.
pub async fn test_achat(req: HttpRequest, path: web::Path<String>) -> HttpResponse {
    if let Err(r) = crate::auth_guard::exiger_owner(&req) {
        return r;
    }
    let uq = path.into_inner();
    let db = Database::shared("uq_platform_db.json");
    let conn = match db.conn() {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": e.to_string()
            }))
        }
    };

    match achat_reussi(&conn, &uq) {
        Ok(()) => {
            let (vert, blanc, rouge, noir, vdr): (i64, i64, i64, i64, i64) = conn.query_row(
                "SELECT vert, blanc, rouge, noir, vert_depuis_rouge FROM reputation WHERE uq = ?1",
                rusqlite::params![uq],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            ).unwrap_or((0, 0, 0, 0, 0));

            HttpResponse::Ok().json(json!({
                "status": "success",
                "vert": vert,
                "blanc": blanc,
                "rouge": rouge,
                "noir": noir,
                "vert_depuis_rouge": vdr
            }))
        }
        Err(e) => HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": e
        })),
    }
}

// ============================================================================
// JUGEMENT DES DOSSIERS (J+3) + PURGE
// ============================================================================

/// Juge les dossiers qui ont 3 jours ou plus.
pub fn juger_dossiers_expires(db: &Database) -> Result<Vec<String>, String> {
    let conn = db.conn().map_err(|e| e.to_string())?;

    let mut stmt = conn
        .prepare(
            "SELECT id, accusateur, accuse, type_litige
         FROM dossiers
         WHERE statut = 'en_cours'
           AND datetime(created_at, '+3 days') <= datetime('now')",
        )
        .map_err(|e| e.to_string())?;

    let dossiers: Vec<(String, String, String, Option<String>)> = stmt
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    let mut juges = Vec::new();

    for (dossier_id, _accusateur, accuse, type_litige) in dossiers {
        // 1. Recuperer les votes
        let (likes, likes_inverses): (i64, i64) = conn
            .query_row(
                "SELECT
                COALESCE(SUM(CASE WHEN vote = 'like' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN vote = 'like_inverse' THEN 1 ELSE 0 END), 0)
             FROM dossier_votes WHERE dossier_id = ?1",
                rusqlite::params![dossier_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap_or((0, 0));

        // 2. Recuperer les commentateurs
        let mut stmt_com = conn
            .prepare("SELECT auteur FROM dossier_commentaires WHERE dossier_id = ?1")
            .map_err(|e| e.to_string())?;

        let commentateurs: Vec<String> = stmt_com
            .query_map(rusqlite::params![dossier_id], |row| row.get(0))
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();

        let mut positifs = 0;
        let mut litigieux = 0;

        for commentateur in &commentateurs {
            let (rouge, noir): (i64, i64) = conn
                .query_row(
                    "SELECT COALESCE(rouge, 0), COALESCE(noir, 0)
                 FROM reputation WHERE uq = ?1",
                    rusqlite::params![commentateur],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap_or((0, 0));

            if rouge == 0 && noir == 0 {
                positifs += 1;
            } else {
                litigieux += 1;
            }
        }

        // 3. Decision (les votes sont un indicateur supplementaire)
        let decision = if litigieux > positifs || likes_inverses > likes {
            "sanction"
        } else {
            "rejet"
        };

        // 4. Appliquer la sanction
        if decision == "sanction" {
            let _ = conn.execute(
                "INSERT INTO reputation (uq, rouge, noir, vert, vert_seuil_sanction)
                 VALUES (?1, 1, 1, 0, 0)
                 ON CONFLICT(uq) DO UPDATE SET
                   vert_seuil_sanction = COALESCE(reputation.vert, 0),
                   rouge = COALESCE(reputation.rouge, 0) + 1,
                   noir = COALESCE(reputation.noir, 0) + 1,
                   vert = MAX(0, COALESCE(reputation.vert, 0) - 10)",
                rusqlite::params![accuse],
            );

            match type_litige.as_deref() {
                Some("p2p") => {
                    let _ = conn.execute(
                        "UPDATE accounts SET p2p_blocked = 1 WHERE uq_number = ?1",
                        rusqlite::params![accuse],
                    );
                }
                Some("zando") => {
                    let _ = conn.execute(
                        "UPDATE accounts SET zando_blocked = 1 WHERE uq_number = ?1",
                        rusqlite::params![accuse],
                    );
                    let _ = conn.execute(
                        "UPDATE shop_items SET published = 0 WHERE shop_id = ?1",
                        rusqlite::params![accuse],
                    );
                }
                _ => {
                    let _ = conn.execute(
                        "UPDATE accounts SET retrait_blocked = 1 WHERE uq_number = ?1",
                        rusqlite::params![accuse],
                    );
                }
            }
        } else {
            let _ = conn.execute(
                "UPDATE reputation SET orange = CASE WHEN orange > 0 THEN orange - 1 ELSE 0 END WHERE uq = ?1",
                rusqlite::params![accuse],
            );
        }

        // Litige marketplace : le verdict tranche aussi le séquestre.
        // sanction → remboursement acheteur ; rejet → libération vendeur/livreur.
        if type_litige.as_deref() == Some("marketplace") {
            let _ = crate::marketplace::resoudre_litige(&dossier_id, decision == "sanction");
        }

        // 5. Purger le dossier
        let _ = conn.execute(
            "DELETE FROM dossier_votes WHERE dossier_id = ?1",
            rusqlite::params![dossier_id],
        );
        let _ = conn.execute(
            "DELETE FROM dossier_commentaires WHERE dossier_id = ?1",
            rusqlite::params![dossier_id],
        );
        let _ = conn.execute(
            "DELETE FROM dossiers WHERE id = ?1",
            rusqlite::params![dossier_id],
        );

        juges.push(dossier_id);
    }

    Ok(juges)
}

// ============================================================================
// COULEUR DOMINANTE (pour l'anneau du profil public)
// ============================================================================

pub fn couleur_dominante(
    vert: i64,
    blanc: i64,
    orange: i64,
    rouge: i64,
    _noir: i64,
) -> &'static str {
    if rouge > 0 {
        "rouge"
    } else if orange > 0 {
        "orange"
    } else if vert > 0 {
        "vert"
    } else if blanc > 0 {
        "blanc"
    } else {
        "blanc"
    }
}

// ============================================================================
// ROUTES DES DOSSIERS PUBLICS
// ============================================================================

/// GET /api/dossiers
pub async fn list_dossiers() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    let conn = match db.conn() {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error", "message": e.to_string()
            }))
        }
    };

    let mut stmt = match conn.prepare(
        "SELECT id, accusateur, accuse, texte_accusation, nb_evenements, echecs, en_cours, type_litige, created_at
         FROM dossiers WHERE statut = 'en_cours' ORDER BY created_at DESC LIMIT 50"
    ) {
        Ok(s) => s,
        Err(e) => return HttpResponse::InternalServerError().json(json!({
            "status": "error", "message": e.to_string()
        })),
    };

    let dossiers: Vec<serde_json::Value> = stmt
        .query_map([], |row| {
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "accusateur": row.get::<_, String>(1)?,
                "accuse": row.get::<_, String>(2)?,
                "texte": row.get::<_, String>(3)?,
                "nb_evenements": row.get::<_, i64>(4)?,
                "echecs": row.get::<_, i64>(5)?,
                "en_cours": row.get::<_, i64>(6)?,
                "type_litige": row.get::<_, Option<String>>(7)?,
                "created_at": row.get::<_, String>(8)?,
            }))
        })
        .map(|r| r.filter_map(|x| x.ok()).collect::<Vec<_>>())
        .unwrap_or_default();

    HttpResponse::Ok().json(json!({
        "status": "success",
        "dossiers": dossiers,
        "total": dossiers.len()
    }))
}

/// GET /api/dossiers/{id}
pub async fn get_dossier(path: web::Path<String>) -> HttpResponse {
    let id = path.into_inner();
    let db = Database::shared("uq_platform_db.json");
    let conn = match db.conn() {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError()
                .json(json!({"status":"error","message":e.to_string()}))
        }
    };

    let dossier = match conn.query_row(
        "SELECT id, accusateur, accuse, texte_accusation, nb_evenements, echecs, en_cours, type_litige, created_at
         FROM dossiers WHERE id = ?1",
        rusqlite::params![id],
        |row| {
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "accusateur": row.get::<_, String>(1)?,
                "accuse": row.get::<_, String>(2)?,
                "texte": row.get::<_, String>(3)?,
                "nb_evenements": row.get::<_, i64>(4)?,
                "echecs": row.get::<_, i64>(5)?,
                "en_cours": row.get::<_, i64>(6)?,
                "type_litige": row.get::<_, Option<String>>(7)?,
                "created_at": row.get::<_, String>(8)?,
            }))
        },
    ) {
        Ok(d) => d,
        Err(_) => return HttpResponse::NotFound().json(json!({"status":"error","message":"Dossier introuvable"})),
    };

    let (likes, likes_inverses): (i64, i64) = conn
        .query_row(
            "SELECT COALESCE(SUM(CASE WHEN vote = 'like' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN vote = 'like_inverse' THEN 1 ELSE 0 END), 0)
         FROM dossier_votes WHERE dossier_id = ?1",
            rusqlite::params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap_or((0, 0));

    let mut stmt = match conn.prepare(
        "SELECT auteur, texte, photo, created_at FROM dossier_commentaires
         WHERE dossier_id = ?1 ORDER BY created_at ASC LIMIT 50",
    ) {
        Ok(s) => s,
        Err(_) => {
            return HttpResponse::InternalServerError()
                .json(json!({"status":"error","message":"Erreur commentaires"}))
        }
    };

    let commentaires: Vec<serde_json::Value> = stmt
        .query_map(rusqlite::params![id], |row| {
            Ok(json!({
                "auteur": row.get::<_, String>(0)?,
                "texte": row.get::<_, String>(1)?,
                "photo": row.get::<_, Option<String>>(2)?,
                "created_at": row.get::<_, String>(3)?,
            }))
        })
        .map(|r| r.filter_map(|x| x.ok()).collect::<Vec<_>>())
        .unwrap_or_default();

    HttpResponse::Ok().json(json!({
        "status": "success",
        "dossier": dossier,
        "votes": { "likes": likes, "likes_inverses": likes_inverses },
        "commentaires": commentaires
    }))
}

/// POST /api/dossiers/{id}/vote
pub async fn vote_dossier(
    req: HttpRequest,
    path: web::Path<String>,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let claims = match exiger_auth(&req) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let dossier_id = path.into_inner();
    let vote = body.get("vote").and_then(|v| v.as_str()).unwrap_or("like");

    if vote != "like" && vote != "like_inverse" {
        return HttpResponse::BadRequest()
            .json(json!({"status":"error","message":"Vote invalide"}));
    }

    let db = Database::shared("uq_platform_db.json");
    let conn = match db.conn() {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError()
                .json(json!({"status":"error","message":e.to_string()}))
        }
    };

    let id = format!("vote_{}", Uuid::new_v4());
    let _ = conn.execute(
        "INSERT INTO dossier_votes (id, dossier_id, votant, vote) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(dossier_id, votant) DO UPDATE SET vote = ?4",
        rusqlite::params![id, dossier_id, claims.sub, vote],
    );

    HttpResponse::Ok().json(json!({"status": "success"}))
}

/// POST /api/dossiers/{id}/comment
pub async fn comment_dossier(
    req: HttpRequest,
    path: web::Path<String>,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let claims = match exiger_auth(&req) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let dossier_id = path.into_inner();
    let texte = body
        .get("texte")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let photo = body.get("photo").and_then(|v| v.as_str()).unwrap_or("");

    if texte.len() > 30 {
        return HttpResponse::BadRequest()
            .json(json!({"status":"error","message":"30 caracteres maximum"}));
    }
    if texte.is_empty() && photo.is_empty() {
        return HttpResponse::BadRequest()
            .json(json!({"status":"error","message":"Texte ou photo obligatoire"}));
    }

    let db = Database::shared("uq_platform_db.json");
    let conn = match db.conn() {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError()
                .json(json!({"status":"error","message":e.to_string()}))
        }
    };

    let nb: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM dossier_commentaires WHERE dossier_id = ?1 AND auteur = ?2",
            rusqlite::params![dossier_id, claims.sub],
            |row| row.get(0),
        )
        .unwrap_or(0);

    if nb >= 2 {
        return HttpResponse::BadRequest()
            .json(json!({"status":"error","message":"2 commentaires maximum"}));
    }

    let (accusateur, accuse): (String, String) = match conn.query_row(
        "SELECT accusateur, accuse FROM dossiers WHERE id = ?1",
        rusqlite::params![dossier_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ) {
        Ok(v) => v,
        Err(_) => {
            return HttpResponse::NotFound()
                .json(json!({"status":"error","message":"Dossier introuvable"}))
        }
    };

    let historique_reel =
        verifier_historique_reel(&conn, &claims.sub, &accusateur, &accuse).unwrap_or(false);
    if !historique_reel {
        return HttpResponse::Forbidden()
            .json(json!({"status":"error","message":"Pas d'historique reel"}));
    }

    let id = format!("com_{}", Uuid::new_v4());
    let _ = conn.execute(
        "INSERT INTO dossier_commentaires (id, dossier_id, auteur, texte, photo) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![id, dossier_id, claims.sub, texte, if photo.is_empty() { None } else { Some(photo) }],
    );

    HttpResponse::Ok().json(json!({"status": "success"}))
}

// ============================================================================
// PURGE DES DONNEES > 7 JOURS (par donnee individuelle)
// ============================================================================

/// Supprime les donnees de plus de 7 jours.
/// Parcourt TOUTES les bases par compte (data/users/*.db) + la base centrale.
/// Ne touche PAS : reputation, profile_ratings, publications, shop_items.
pub fn purger_donnees_anciennes() -> Result<(i64, i64, i64), String> {
    use rusqlite::Connection;

    let seuil = chrono::Utc::now().timestamp() - 7 * 86400;
    let mut total_tx: i64 = 0;

    // 1. Purger chaque base par compte (data/users/*.db)
    let users_dir = std::path::Path::new("data/users");
    if users_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(users_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("db") {
                    if let Ok(conn) = Connection::open(&path) {
                        // Purger transactions > 7 jours
                        let n_tx = conn
                            .execute(
                                "DELETE FROM transactions WHERE created_at < ?1",
                                rusqlite::params![seuil],
                            )
                            .unwrap_or(0) as i64;
                        total_tx += n_tx;

                        // Purger messages > 7 jours (si la table existe)
                        let _ = conn.execute(
                            "DELETE FROM messages WHERE timestamp < ?1",
                            rusqlite::params![seuil],
                        );

                        // Purger my_purchases > 7 jours
                        let _ = conn.execute(
                            "DELETE FROM my_purchases WHERE created_at < ?1",
                            rusqlite::params![seuil],
                        );

                        // Purger my_sales > 7 jours
                        let _ = conn.execute(
                            "DELETE FROM my_sales WHERE created_at < ?1",
                            rusqlite::params![seuil],
                        );

                        if n_tx > 0 {
                            log::info!("Purge {} : {} transactions", path.display(), n_tx);
                        }
                    }
                }
            }
        }
    }

    // 2. Purger la base centrale (chat_messages, offers)
    let db = Database::shared("uq_platform_db.json");
    let conn = db.conn().map_err(|e| e.to_string())?;

    let total_chat = conn
        .execute(
            "DELETE FROM chat_messages WHERE timestamp < ?1",
            rusqlite::params![seuil],
        )
        .unwrap_or(0) as i64;

    let total_offers = conn
        .execute(
            "DELETE FROM offers WHERE created_at < ?1",
            rusqlite::params![seuil],
        )
        .unwrap_or(0) as i64;

    Ok((total_tx, total_chat, total_offers))
}
