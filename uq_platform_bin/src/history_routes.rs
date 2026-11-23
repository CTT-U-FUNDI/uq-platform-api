// ====================
// 🌐 HISTORY ROUTES — Historiques locaux + récupération par voisins
// ====================
//
// Routes de la spécification :
//   POST /api/history/request-recovery   (étape 1)
//   POST /api/history/provide-fragment   (étape 2)
//   POST /api/history/validate-recovery  (étape 4)
//   POST /api/history/cleanup-old        (auto-suppression > 1 an)
//
// Plus :
//   GET  /api/history/pending-requests   (les voisins voient les demandes)
//   POST /api/history/reconstruct        (étape 3 : reconstitution)
//   GET  /api/history/recovery/{id}      (état d'une demande)
//   GET  /api/history/{uq}               (état + intégrité de l'historique)
//   POST /api/history/{uq}/verify         (vérification du scellé)
//   GET  /api/history/{uq}/chain          (la chaîne locale, pour contrôle)
//   GET  /api/history/blockorb/size       (taille fixe du BlockOrb)
//
// ⚠️ Aucune de ces routes ne déplace de monnaie : elles ne touchent qu'à
//    l'historique. L'invariant des 40 Md BORB n'est jamais concerné.
// ====================

use actix_web::{web, HttpRequest, HttpResponse};
use rusqlite::Connection;
use serde_json::json;

use crate::auto_cleanup;
use crate::history_recovery;
use crate::local_history;
use crate::p2p_history;

// ====================
// OUTILS
// ====================

fn roles_autorises() -> Vec<&'static str> {
    vec!["user", "admin", "guardian", "owner"]
}

fn err_json(code: u16, message: String) -> HttpResponse {
    match code {
        400 => HttpResponse::BadRequest().json(json!({"status":"error","message":message})),
        403 => HttpResponse::Forbidden().json(json!({"status":"error","message":message})),
        404 => HttpResponse::NotFound().json(json!({"status":"error","message":message})),
        _ => HttpResponse::InternalServerError().json(json!({"status":"error","message":message})),
    }
}

/// Retrouve la DB locale qui contient une demande de récupération.
/// (le nœud central héberge les demandes dans la DB du demandeur)
fn trouver_db_demande(request_id: &str) -> Option<(String, String)> {
    let entrees = std::fs::read_dir("data/users").ok()?;
    for entree in entrees.flatten() {
        let nom = entree.file_name().to_string_lossy().to_string();
        if !nom.starts_with("uq_") || !nom.ends_with(".db") {
            continue;
        }
        let brut = &nom[3..nom.len() - 3];
        if brut.is_empty() || !brut.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let chemin = format!("data/users/{}", nom);
        if let Ok(conn) = Connection::open(&chemin) {
            let existe: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='recovery_requests'",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            if existe == 0 {
                continue;
            }
            let present: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM recovery_requests WHERE id = ?1",
                    rusqlite::params![request_id],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            if present > 0 {
                return Some((format!("UQ-{}", brut), chemin));
            }
        }
    }
    None
}

fn ouvrir_db(uq_number: &str) -> Result<Connection, HttpResponse> {
    let chemin = history_recovery::db_path(uq_number);
    if !std::path::Path::new(&chemin).exists() {
        return Err(err_json(404, format!("Compte introuvable : {}", uq_number)));
    }
    Connection::open(&chemin).map_err(|e| err_json(500, format!("Erreur DB : {}", e)))
}

// ====================
// 📜 ÉTAT DE L'HISTORIQUE LOCAL (scellé par le cadenas)
// ====================

/// GET /api/history/{uq}
pub async fn history_status(req: HttpRequest, path: web::Path<String>) -> HttpResponse {
    let claims = match crate::auth::AuthManager::require_role(&req, &roles_autorises()) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    let uq = path.into_inner();

    let est_admin = claims.role == "admin"
        || claims.role == "owner"
        || crate::owner_guard::check_is_owner(&claims.sub);
    if !est_admin && !claims.sub.eq_ignore_ascii_case(&uq) {
        return err_json(
            403,
            "Tu ne peux consulter que ton propre historique".to_string(),
        );
    }

    let conn = match ouvrir_db(&uq) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    match local_history::verifier_scelle(&conn, &uq) {
        Ok(etat) => HttpResponse::Ok().json(json!({
            "status": "success",
            "uq_number": uq,
            "historique": etat,
            "regle": "historique local, verrouille par le cadenas, auto-supprime apres 1 an"
        })),
        Err(e) => err_json(500, e),
    }
}

/// POST /api/history/{uq}/verify — vérifie le scellé (détection de fraude)
pub async fn history_verify(req: HttpRequest, path: web::Path<String>) -> HttpResponse {
    let claims = match crate::auth::AuthManager::require_role(&req, &roles_autorises()) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    let uq = path.into_inner();

    let est_admin = claims.role == "admin"
        || claims.role == "owner"
        || crate::owner_guard::check_is_owner(&claims.sub);
    if !est_admin && !claims.sub.eq_ignore_ascii_case(&uq) {
        return err_json(
            403,
            "Verification reservee au proprietaire du compte".to_string(),
        );
    }

    let conn = match ouvrir_db(&uq) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    match local_history::verifier_scelle(&conn, &uq) {
        Ok(etat) => {
            if !etat.integre {
                eprintln!("🚨 HISTORIQUE MODIFIE detecte pour {}", uq);
            }
            HttpResponse::Ok().json(json!({
                "status": if etat.integre { "success" } else { "error" },
                "uq_number": uq,
                "integre": etat.integre,
                "chaine_valide": etat.chaine_valide,
                "tx_count": etat.tx_count,
                "message": if etat.integre {
                    "Historique intact : le scelle correspond au cadenas"
                } else {
                    "ALERTE : historique modifie ou cadenas absent"
                }
            }))
        }
        Err(e) => err_json(500, e),
    }
}

/// GET /api/history/{uq}/chain — la chaîne locale (borb_id / prev_hash / bloc)
pub async fn history_chain(req: HttpRequest, path: web::Path<String>) -> HttpResponse {
    let claims = match crate::auth::AuthManager::require_role(&req, &roles_autorises()) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    let uq = path.into_inner();

    let est_admin = claims.role == "admin"
        || claims.role == "owner"
        || crate::owner_guard::check_is_owner(&claims.sub);
    if !est_admin && !claims.sub.eq_ignore_ascii_case(&uq) {
        return err_json(403, "Chaine reservee au proprietaire du compte".to_string());
    }

    let conn = match ouvrir_db(&uq) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    if let Err(e) = local_history::init_tables(&conn) {
        return err_json(500, e);
    }

    match local_history::charger_chaine(&conn) {
        Ok(chaine) => {
            let valide = crate::blockchain_lib::verify_chain(&chaine);
            let tx: Vec<serde_json::Value> = chaine
                .iter()
                .rev()
                .take(200)
                .map(|t| {
                    json!({
                        "borb_id": t.borb_id,
                        "prev_hash": t.prev_hash,
                        "sender_uq": t.sender_uq,
                        "recipient_uq": t.recipient_uq,
                        "amount": t.amount,
                        "fee": t.fee,
                        "timestamp": t.timestamp,
                    })
                })
                .collect();

            HttpResponse::Ok().json(json!({
                "status": "success",
                "uq_number": uq,
                "chaine_valide": valide,
                "total": chaine.len(),
                "chain_hash": crate::blockchain_lib::chain_hash(&chaine),
                "transactions": tx
            }))
        }
        Err(e) => err_json(500, e),
    }
}

// ====================
// ÉTAPE 1 — DEMANDE DE RÉCUPÉRATION
// ====================

/// POST /api/history/request-recovery
/// body : { "lost_uq": "UQ-...", "date_range": "2026-01-01 to 2026-12-31",
///          "reason": "...", "signature": "..." }
pub async fn history_request_recovery(
    req: HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let claims = match crate::auth::AuthManager::require_role(&req, &roles_autorises()) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    // Par défaut : le compte appelant est celui qui a perdu son historique
    let lost_uq = body
        .get("lost_uq")
        .and_then(|v| v.as_str())
        .unwrap_or(&claims.sub)
        .to_string();

    // Le demandeur ne peut agir que pour son propre compte (sauf admin)
    let est_admin = claims.role == "admin" || claims.role == "owner";
    if !est_admin && !lost_uq.eq_ignore_ascii_case(&claims.sub) {
        return err_json(403, "Recuperation limitee a ton propre compte".to_string());
    }

    // Bornes de dates : soit date_range, soit date_from/date_to
    let (date_from, date_to) = if let Some(range) = body.get("date_range").and_then(|v| v.as_str())
    {
        match p2p_history::parser_date_range(range) {
            Some((f, t)) => (f, t),
            None => {
                return err_json(
                    400,
                    "date_range invalide (attendu : 'AAAA-MM-JJ to AAAA-MM-JJ')".to_string(),
                )
            }
        }
    } else {
        (
            body.get("date_from").and_then(|v| v.as_i64()).unwrap_or(0),
            body.get("date_to")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(|| chrono::Utc::now().timestamp()),
        )
    };

    let raison = body.get("reason").and_then(|v| v.as_str()).unwrap_or("");
    let signature = body.get("signature").and_then(|v| v.as_str());

    let conn = match ouvrir_db(&lost_uq) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    match history_recovery::creer_demande(
        &conn,
        &claims.sub,
        &lost_uq,
        date_from,
        date_to,
        raison,
        signature,
    ) {
        Ok(demande) => HttpResponse::Ok().json(json!({
            "status": "success",
            "etape": 1,
            "request_id": demande.request_id,
            "requester_uq": demande.requester_uq,
            "lost_uq": demande.lost_uq,
            "date_range": demande.date_range,
            "message": "Demande publiee : les voisins peuvent fournir leurs fragments"
        })),
        Err(e) => err_json(400, e),
    }
}

/// GET /api/history/pending-requests — ce qu'un voisin peut aider à restaurer
pub async fn history_pending_requests(req: HttpRequest) -> HttpResponse {
    let _claims = match crate::auth::AuthManager::require_role(&req, &roles_autorises()) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let mut demandes: Vec<serde_json::Value> = Vec::new();

    // Les demandes vivent dans la DB du demandeur → on balaie les comptes
    let Ok(entrees) = std::fs::read_dir("data/users") else {
        return err_json(500, "data/users illisible".to_string());
    };

    for entree in entrees.flatten() {
        let nom = entree.file_name().to_string_lossy().to_string();
        if !nom.starts_with("uq_") || !nom.ends_with(".db") {
            continue;
        }
        let brut = &nom[3..nom.len() - 3];
        if brut.is_empty() || !brut.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let chemin = format!("data/users/{}", nom);
        if let Ok(conn) = Connection::open(&chemin) {
            if let Ok(mut liste) = history_recovery::demandes_ouvertes(&conn) {
                demandes.append(&mut liste);
            }
        }
    }

    HttpResponse::Ok().json(json!({
        "status": "success",
        "total": demandes.len(),
        "demandes": demandes,
        "message": "Fournis un fragment via POST /api/history/provide-fragment"
    }))
}

// ====================
// ÉTAPE 2 — UN VOISIN FOURNIT SES FRAGMENTS
// ====================

/// POST /api/history/provide-fragment
/// body : { "request_id": "...", "lost_uq": "UQ-...",
///          "date_from": 0, "date_to": 0, "signature": "..." }
pub async fn history_provide_fragment(
    req: HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let claims = match crate::auth::AuthManager::require_role(&req, &roles_autorises()) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let request_id = body
        .get("request_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if request_id.is_empty() {
        return err_json(400, "request_id requis".to_string());
    }

    // Retrouver la demande → on connaît le compte perdu et le demandeur
    let Some((demandeur_uq, chemin_demandeur)) = trouver_db_demande(request_id) else {
        return err_json(404, format!("Demande introuvable : {}", request_id));
    };

    let lost_uq = body
        .get("lost_uq")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            Connection::open(&chemin_demandeur)
                .ok()
                .and_then(|c| {
                    c.query_row(
                        "SELECT COALESCE(public_uq,'') FROM recovery_requests WHERE id = ?1",
                        rusqlite::params![request_id],
                        |r| r.get::<_, String>(0),
                    )
                    .ok()
                })
                .unwrap_or_default()
        });

    if lost_uq.is_empty() {
        return err_json(400, "lost_uq introuvable pour cette demande".to_string());
    }

    // Fenêtre temporelle (celle de la demande par défaut)
    let conn_demandeur = match Connection::open(&chemin_demandeur) {
        Ok(c) => c,
        Err(e) => return err_json(500, format!("Erreur DB demandeur : {}", e)),
    };
    let (date_from, date_to): (i64, i64) = conn_demandeur
        .query_row(
            "SELECT date_from, date_to FROM recovery_requests WHERE id = ?1",
            rusqlite::params![request_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap_or((0, chrono::Utc::now().timestamp()));

    // Le voisin cherche dans SA DB locale
    let conn_voisin = match ouvrir_db(&claims.sub) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let fragments = match history_recovery::chercher_fragments(
        &conn_voisin,
        &claims.sub,
        &lost_uq,
        date_from,
        date_to,
    ) {
        Ok(f) => f,
        Err(e) => return err_json(500, e),
    };

    if fragments.is_empty() {
        return HttpResponse::Ok().json(json!({
            "status": "success",
            "etape": 2,
            "voisin": claims.sub,
            "lost_uq": lost_uq,
            "fragments": 0,
            "fragments_envoyes": 0,
            "fragments_ajoutes": 0,
            "message": "Aucune transaction te concernant dans cette periode"
        }));
    }

    let signature = body.get("signature").and_then(|v| v.as_str());

    // 1. Le voisin garde sa propre trace de l'aide apportée
    let fragment = match history_recovery::enregistrer_fragment_chez_voisin(
        &conn_voisin,
        request_id,
        &claims.sub,
        &lost_uq,
        &fragments,
        signature,
    ) {
        Ok(f) => f,
        Err(e) => return err_json(400, e),
    };

    // 2. Le fragment est déposé chez le demandeur (reconstitution)
    let ajoutes = match history_recovery::deposer_fragment_chez_demandeur(
        &conn_demandeur,
        request_id,
        &fragment,
    ) {
        Ok(n) => n,
        Err(e) => return err_json(400, e),
    };

    // 3. Le voisin est enregistré comme validateur (il atteste ses fragments)
    let etat = history_recovery::valider_recuperation(
        &conn_demandeur,
        request_id,
        &claims.sub,
        true,
        "fragment fourni",
        None,
    )
    .ok();

    HttpResponse::Ok().json(json!({
        "status": "success",
        "etape": 2,
        "request_id": request_id,
        "voisin": claims.sub,
        "lost_uq": lost_uq,
        "fragments_envoyes": fragments.len(),
        "fragments_ajoutes": ajoutes,
        "confirmations": etat.as_ref().map(|e| e.confirmations).unwrap_or(0),
        "valide": etat.as_ref().map(|e| e.valide).unwrap_or(false),
        "demandeur": demandeur_uq,
        "message": "Fragment transmis au demandeur"
    }))
}

// ====================
// ÉTAPE 3 — RECONSTITUTION
// ====================

/// POST /api/history/reconstruct  body : { "request_id": "..." }
pub async fn history_reconstruct(
    req: HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let claims = match crate::auth::AuthManager::require_role(&req, &roles_autorises()) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let request_id = body
        .get("request_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if request_id.is_empty() {
        return err_json(400, "request_id requis".to_string());
    }

    let Some((demandeur_uq, chemin)) = trouver_db_demande(request_id) else {
        return err_json(404, format!("Demande introuvable : {}", request_id));
    };

    let est_admin = claims.role == "admin" || claims.role == "owner";
    if !est_admin && !claims.sub.eq_ignore_ascii_case(&demandeur_uq) {
        return err_json(
            403,
            "Seul le demandeur peut reconstituer son historique".to_string(),
        );
    }

    let conn = match Connection::open(&chemin) {
        Ok(c) => c,
        Err(e) => return err_json(500, format!("Erreur DB : {}", e)),
    };

    let lost_uq: String = conn
        .query_row(
            "SELECT COALESCE(public_uq,'') FROM recovery_requests WHERE id = ?1",
            rusqlite::params![request_id],
            |r| r.get(0),
        )
        .unwrap_or_else(|_| demandeur_uq.clone());

    match history_recovery::reconstruire_historique(&conn, &lost_uq, request_id) {
        Ok(n) => HttpResponse::Ok().json(json!({
            "status": "success",
            "etape": 3,
            "request_id": request_id,
            "lost_uq": lost_uq,
            "transactions_reconstruites": n,
            "message": "Historique reconstitue depuis les fragments des voisins (soldes intacts)"
        })),
        Err(e) => err_json(400, e),
    }
}

/// GET /api/history/recovery/{request_id}
pub async fn history_recovery_status(req: HttpRequest, path: web::Path<String>) -> HttpResponse {
    let _claims = match crate::auth::AuthManager::require_role(&req, &roles_autorises()) {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    let request_id = path.into_inner();

    let Some((_demandeur, chemin)) = trouver_db_demande(&request_id) else {
        return err_json(404, format!("Demande introuvable : {}", request_id));
    };

    let conn = match Connection::open(&chemin) {
        Ok(c) => c,
        Err(e) => return err_json(500, format!("Erreur DB : {}", e)),
    };

    match history_recovery::etat_demande(&conn, &request_id) {
        Ok(etat) => HttpResponse::Ok().json(json!({"status": "success", "recovery": etat})),
        Err(e) => err_json(404, e),
    }
}

// ====================
// ÉTAPE 4 — VALIDATION PAR LES VOISINS (3+)
// ====================

/// POST /api/history/validate-recovery
/// body : { "request_id": "...", "approved": true, "comment": "...", "signature": "..." }
pub async fn history_validate_recovery(
    req: HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let claims = match crate::auth::AuthManager::require_role(&req, &roles_autorises()) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let request_id = body
        .get("request_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if request_id.is_empty() {
        return err_json(400, "request_id requis".to_string());
    }

    let approuve = body
        .get("approved")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let commentaire = body.get("comment").and_then(|v| v.as_str()).unwrap_or("");
    let signature = body.get("signature").and_then(|v| v.as_str());

    let Some((_demandeur, chemin)) = trouver_db_demande(request_id) else {
        return err_json(404, format!("Demande introuvable : {}", request_id));
    };

    let conn = match Connection::open(&chemin) {
        Ok(c) => c,
        Err(e) => return err_json(500, format!("Erreur DB : {}", e)),
    };

    match history_recovery::valider_recuperation(
        &conn,
        request_id,
        &claims.sub,
        approuve,
        commentaire,
        signature,
    ) {
        Ok(etat) => HttpResponse::Ok().json(json!({
            "status": "success",
            "etape": 4,
            "recovery": etat,
            "message": if etat.valide {
                "Historique valide par 3 voisins ou plus"
            } else {
                "Confirmation enregistree : il faut 3 voisins"
            }
        })),
        Err(e) => err_json(400, e),
    }
}

// ====================
// AUTO-SUPPRESSION > 1 AN
// ====================

/// POST /api/history/cleanup-old
///
/// RÈGLE 2 : la purge est TOUJOURS limitée au compte courant.
/// Aucun paramètre ne permet de purger le compte d'un autre : un `uq`
/// différent de l'appelant est refusé (même pour un admin, la règle est
/// « chaque compte gère SON cycle »).
///
/// body optionnel : { "force": true } pour ignorer la fenêtre de 24 h
/// (utile pour les tests ; sans `force`, une seule passe par 24 h).
pub async fn history_cleanup_old(
    req: HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let claims = match crate::auth::AuthManager::require_role(&req, &roles_autorises()) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    // Refus explicite de purger un autre compte
    if let Some(demande) = body.get("uq").and_then(|v| v.as_str()) {
        if !demande.eq_ignore_ascii_case(&claims.sub) {
            return err_json(
                403,
                format!(
                    "Purge limitee au compte courant : {} ne peut pas purger {}",
                    claims.sub, demande
                ),
            );
        }
    }

    let forcer = body.get("force").and_then(|v| v.as_bool()).unwrap_or(false);

    let conn = match ouvrir_db(&claims.sub) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    match auto_cleanup::purger_compte(&conn, &claims.sub, chrono::Utc::now().timestamp(), forcer) {
        Ok(rapport) => HttpResponse::Ok().json(json!({
            "status": "success",
            "portee": "compte courant uniquement",
            "uq_number": rapport.uq_number,
            "lignes_supprimees": rapport.lignes_supprimees,
            "reste_a_purger": rapport.reste_a_purger,
            "deja_purge_recemment": rapport.deja_purge_recemment,
            "derniere_purge_at": rapport.derniere_purge_at,
            "prochaine_purge_at": rapport.prochaine_purge_at,
            "age_max_jours": rapport.age_max_jours,
            "mode": rapport.mode,
            "message": if rapport.deja_purge_recemment {
                "Deja purge dans les dernieres 24 h (ajoute \"force\": true pour relancer)"
            } else {
                "Purge unitaire effectuee (1 transaction a la fois, soldes intacts)"
            }
        })),
        Err(e) => err_json(500, e),
    }
}

// ====================
// SUPERVISION : BLOCKORB À TAILLE FIXE
// ====================

/// GET /api/history/blockorb/size
pub async fn blockorb_size() -> HttpResponse {
    let orb = crate::persistence::load_global_orb();
    let octets = crate::persistence::taille_blockorb();
    let max = crate::blockchain_lib::MAX_BLOCKORB_BYTES;

    HttpResponse::Ok().json(json!({
        "status": "success",
        "blockorb_octets": octets,
        "blockorb_octets_serialises": orb.taille_octets(),
        "maximum_octets": max,
        "taille_ok": octets <= max,
        "comptes": orb.comptes.len(),
        "block_number": orb.block_number,
        "last_block_hash": orb.last_block_hash,
        "contient_historique": false,
        "invariant_ok": orb.verifier(),
        "total_femto": orb.total().to_string(),
        "regle": "Le BlockOrb ne stocke que l'etat present : soldes, cases, enveloppe UBI, bloc"
    }))
}

// ====================
// POST /api/owner/kyc/auto-validate  (OWNER)
// Valide d'office le KYC du propriétaire (Mission 7).
// ====================

pub async fn owner_kyc_auto_validate(req: HttpRequest) -> HttpResponse {
    if let Err(r) = crate::auth_guard::exiger_owner(&req) {
        return r;
    }
    match crate::kyc::cadenas::valider_kyc_owner_auto() {
        Ok(_) => HttpResponse::Ok().json(json!({
            "status": "success",
            "message": "KYC owner auto-valide — cadenas ouvert",
            "uq_number": crate::kyc::cadenas::OWNER_UQ_NUMBER
        })),
        Err(e) => HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": e
        })),
    }
}
