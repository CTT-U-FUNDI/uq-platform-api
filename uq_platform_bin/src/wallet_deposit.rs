// ====================
// WALLET DEPOSIT — Dépôts mobile money déclarés puis validés par le owner
// ====================
//
// Flux :
//   1. L'utilisateur envoie l'argent depuis son téléphone (hors app), puis
//      déclare le dépôt : identité réelle, opérateur, numéro expéditeur,
//      devise, montant, montant exact envoyé, pseudo UQ et code SMS
//      ("code jetable") reçu de l'opérateur.
//   2. Le dépôt est enregistré `en_attente` et le propriétaire est notifié.
//   3. Le propriétaire vérifie la réception sur son propre mobile money puis
//      confirme ou refuse depuis le coffre (/api/owner/depots).
//   4. La confirmation crédite `multi_wallets` (devise) ET la réserve BORB
//      du compte, au taux oracle du jour — les deux soldes restent adossés
//      l'un à l'autre comme pour une conversion.
//
// AGRÉGATEURS : la table porte déjà `canal` ('manuel' | 'agrégateur'),
// `provider` et `api_reference`. La route POST /api/wallet/depot/webhook
// est réservée : quand une passerelle (CinetPay, Flutterwave…) sera
// branchée, son webhook appellera la même fonction de crédit que
// `owner_confirmer` — aucune migration nécessaire.
// ====================

use actix_web::{web, HttpRequest, HttpResponse};
use rusqlite::{params, Connection};
use serde_json::json;

use crate::api_handlers::{
    calculate_currency_to_borb_points, chemin_base_compte, taux_live_borb,
    ConvertedTransferAmountError,
};
use crate::auth_guard::{exiger_owner, exiger_soi_ou_owner, OWNER_UQ};
use crate::frais_plateforme::est_fiat;
use crate::trading_routes::push_notification;

/// Un dépôt non traité expire après 48 h : la référence et le code SMS
/// ne doivent pas rester valables indéfiniment.
const DEPOT_EXPIRATION_SECS: i64 = 48 * 3600;
/// Plafond de sécurité sur le montant déclaré (évite les fautes de frappe).
const DEPOT_MONTANT_MAX: f64 = 100_000_000.0;

const COLONNES: &str = "id, uq, nom, prenom, operateur, numero, devise, montant,
    montant_exact, uq_pseudo, code_sms, canal, provider, api_reference,
    statut, motif, owner_uq, traite_at, created_at";

fn maintenant() -> i64 {
    chrono::Utc::now().timestamp()
}

fn ouvrir_index() -> Result<Connection, String> {
    let conn = Connection::open(crate::uq_index::uq_index_path())
        .map_err(|e| format!("Index indisponible ({})", e))?;
    let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
    assurer_table(&conn)?;
    Ok(conn)
}

fn assurer_table(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS wallet_deposits (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            uq TEXT NOT NULL,
            nom TEXT NOT NULL,
            prenom TEXT NOT NULL,
            operateur TEXT NOT NULL,
            numero TEXT NOT NULL,
            devise TEXT NOT NULL,
            montant REAL NOT NULL,
            montant_exact REAL NOT NULL,
            uq_pseudo TEXT NOT NULL DEFAULT '',
            code_sms TEXT NOT NULL,
            canal TEXT NOT NULL DEFAULT 'manuel',
            provider TEXT,
            api_reference TEXT,
            statut TEXT NOT NULL DEFAULT 'en_attente',
            motif TEXT,
            owner_uq TEXT,
            traite_at INTEGER,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_wallet_deposits_statut
            ON wallet_deposits(statut, created_at);
        CREATE INDEX IF NOT EXISTS idx_wallet_deposits_uq
            ON wallet_deposits(uq, created_at);",
    )
    .map_err(|e| format!("Initialisation des dépôts impossible: {}", e))
}

/// Fait expirer les dépôts `en_attente` trop vieux. Appelé avant chaque
/// lecture : pas de tâche planifiée nécessaire.
fn expirer_perimes(conn: &Connection) -> Result<(), String> {
    conn.execute(
        "UPDATE wallet_deposits SET statut = 'expire'
         WHERE statut = 'en_attente' AND created_at < ?1",
        params![maintenant() - DEPOT_EXPIRATION_SECS],
    )
    .map(|_| ())
    .map_err(|e| format!("Expiration des dépôts impossible: {}", e))
}

fn ligne_vers_json(row: &rusqlite::Row<'_>) -> rusqlite::Result<serde_json::Value> {
    Ok(json!({
        "id": row.get::<_, i64>(0)?,
        "uq": row.get::<_, String>(1)?,
        "nom": row.get::<_, String>(2)?,
        "prenom": row.get::<_, String>(3)?,
        "operateur": row.get::<_, String>(4)?,
        "numero": row.get::<_, String>(5)?,
        "devise": row.get::<_, String>(6)?,
        "montant": row.get::<_, f64>(7)?,
        "montant_exact": row.get::<_, f64>(8)?,
        "uq_pseudo": row.get::<_, String>(9)?,
        "code_sms": row.get::<_, String>(10)?,
        "canal": row.get::<_, String>(11)?,
        "provider": row.get::<_, Option<String>>(12)?,
        "api_reference": row.get::<_, Option<String>>(13)?,
        "statut": row.get::<_, String>(14)?,
        "motif": row.get::<_, Option<String>>(15)?,
        "owner_uq": row.get::<_, Option<String>>(16)?,
        "traite_at": row.get::<_, Option<i64>>(17)?,
        "created_at": row.get::<_, i64>(18)?,
    }))
}

fn lire_depot(conn: &Connection, id: i64) -> Result<Option<serde_json::Value>, String> {
    let mut stmt = conn
        .prepare(&format!("SELECT {} FROM wallet_deposits WHERE id = ?1", COLONNES))
        .map_err(|e| format!("Lecture du dépôt impossible: {}", e))?;
    stmt.query_row(params![id], ligne_vers_json)
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            autre => Err(format!("Lecture du dépôt impossible: {}", autre)),
        })
}

fn texte_obligatoire(body: &serde_json::Value, cle: &str) -> Result<String, HttpResponse> {
    let valeur = body
        .get(cle)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if valeur.is_empty() {
        return Err(HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": format!("Le champ '{}' est requis.", cle)
        })));
    }
    Ok(valeur)
}

fn montant_valide(body: &serde_json::Value, cle: &str) -> Result<f64, HttpResponse> {
    let brut = body.get(cle).and_then(|v| v.as_f64()).unwrap_or(0.0);
    let arrondi = (brut * 100.0).round() / 100.0;
    if !brut.is_finite() || brut <= 0.0 || arrondi != brut || brut > DEPOT_MONTANT_MAX {
        return Err(HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": format!("Le champ '{}' doit être un montant positif au centième près.", cle)
        })));
    }
    Ok(brut)
}

// ====================
// CÔTÉ UTILISATEUR
// ====================

/// POST /api/wallet/depot — déclare un dépôt mobile money déjà envoyé.
/// Body : { uq_number, nom, prenom, operateur, numero, devise, montant,
///          montant_exact, code_sms, pin }
pub async fn declarer(req: HttpRequest, body: web::Json<serde_json::Value>) -> HttpResponse {
    let uq = body.get("uq_number").and_then(|v| v.as_str()).unwrap_or("");
    if !crate::auth_guard::ressemble_a_un_uq(uq) {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Identifiant de compte invalide"
        }));
    }
    if let Err(response) = exiger_soi_ou_owner(&req, uq) {
        return response;
    }

    let nom = match texte_obligatoire(&body, "nom") {
        Ok(v) => v,
        Err(r) => return r,
    };
    let prenom = match texte_obligatoire(&body, "prenom") {
        Ok(v) => v,
        Err(r) => return r,
    };
    let operateur = match texte_obligatoire(&body, "operateur") {
        Ok(v) => v,
        Err(r) => return r,
    };
    let numero = match texte_obligatoire(&body, "numero") {
        Ok(v) => v,
        Err(r) => return r,
    };
    let code_sms = match texte_obligatoire(&body, "code_sms") {
        Ok(v) => v,
        Err(r) => return r,
    };
    if code_sms.len() < 4 || code_sms.len() > 20
        || !code_sms.chars().all(|c| c.is_ascii_alphanumeric())
    {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Le code de transaction doit contenir 4 à 20 caractères alphanumériques."
        }));
    }
    let devise = body
        .get("devise")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_uppercase();
    if !est_fiat(&devise) {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Devise non prise en charge pour un dépôt mobile money (fiat uniquement)."
        }));
    }
    let montant = match montant_valide(&body, "montant") {
        Ok(v) => v,
        Err(r) => return r,
    };
    // `montant_exact` = somme réellement partie du mobile money ; s'il est
    // absent on retombe sur le montant déclaré.
    let montant_exact = body
        .get("montant_exact")
        .and_then(|v| v.as_f64())
        .filter(|v| v.is_finite() && *v > 0.0)
        .unwrap_or(montant);
    if !montant_exact.is_finite() || montant_exact <= 0.0 || montant_exact > DEPOT_MONTANT_MAX {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Le montant exact envoyé est invalide."
        }));
    }
    let uq_pseudo = body
        .get("uq_pseudo")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let pin = body.get("pin").and_then(|v| v.as_str()).unwrap_or("");
    if !crate::pin_manager::is_valid_pin_format(pin) {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Le code PIN est requis et doit contenir 4 à 6 chiffres"
        }));
    }

    let account_db_path = chemin_base_compte(uq);
    if !std::path::Path::new(&account_db_path).exists() {
        return HttpResponse::NotFound().json(json!({"status":"error","message":"Compte introuvable"}));
    }
    if let Err(response) =
        crate::api_handlers::verifier_pin_compte(&account_db_path, uq, pin, "déclarer un dépôt")
    {
        return response;
    }

    let conn = match ouvrir_index() {
        Ok(c) => c,
        Err(e) => return HttpResponse::InternalServerError().json(json!({"status":"error","message":e})),
    };
    let now = maintenant();
    let insert = conn.execute(
        "INSERT INTO wallet_deposits
            (uq, nom, prenom, operateur, numero, devise, montant, montant_exact,
             uq_pseudo, code_sms, canal, statut, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'manuel', 'en_attente', ?11)",
        params![
            uq, nom, prenom, operateur, numero, devise, montant, montant_exact,
            uq_pseudo, code_sms, now
        ],
    );
    let depot_id = match insert {
        Ok(_) => conn.last_insert_rowid(),
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Enregistrement du dépôt impossible: {}", e)
            }))
        }
    };

    push_notification(
        OWNER_UQ,
        "wallet",
        "Nouveau dépôt mobile money",
        &format!(
            "{} {} (@{}) déclare {} {} via {} — code {}",
            prenom, nom, uq_pseudo, montant, devise, operateur, code_sms
        ),
        &format!("depot:{}", depot_id),
    );

    HttpResponse::Ok().json(json!({
        "status": "success",
        "deposit_id": depot_id,
        "statut": "en_attente",
        "message": "Dépôt déclaré. Le propriétaire vérifie la réception puis crédite ton wallet."
    }))
}

/// GET /api/wallet/depot/{uq} — historique des dépôts de l'utilisateur.
pub async fn mes_depots(req: HttpRequest, path: web::Path<String>) -> HttpResponse {
    let uq = path.into_inner();
    if !crate::auth_guard::ressemble_a_un_uq(&uq) {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Identifiant de compte invalide"
        }));
    }
    if let Err(response) = exiger_soi_ou_owner(&req, &uq) {
        return response;
    }
    let conn = match ouvrir_index() {
        Ok(c) => c,
        Err(e) => return HttpResponse::InternalServerError().json(json!({"status":"error","message":e})),
    };
    if let Err(e) = expirer_perimes(&conn) {
        return HttpResponse::InternalServerError().json(json!({"status":"error","message":e}));
    }
    let mut stmt = match conn.prepare(&format!(
        "SELECT {} FROM wallet_deposits WHERE UPPER(uq) = UPPER(?1)
         ORDER BY created_at DESC LIMIT 50",
        COLONNES
    )) {
        Ok(s) => s,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Lecture des dépôts impossible: {}", e)
            }))
        }
    };
    let depots: Vec<serde_json::Value> = stmt
        .query_map(params![uq], ligne_vers_json)
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default();
    HttpResponse::Ok().json(json!({ "status": "success", "depots": depots }))
}

/// POST /api/wallet/depot/{id}/annuler — l'utilisateur annule son propre
/// dépôt tant qu'il est `en_attente` (avant ou après envoi : à lui de ne pas
/// envoyer l'argent si le dépôt est annulé).
pub async fn annuler(req: HttpRequest, path: web::Path<i64>) -> HttpResponse {
    let id = path.into_inner();
    // L'autorisation porte sur le dépôt lui-même : on lit d'abord pour
    // connaître le uq propriétaire de la déclaration.
    let conn = match ouvrir_index() {
        Ok(c) => c,
        Err(e) => return HttpResponse::InternalServerError().json(json!({"status":"error","message":e})),
    };
    let depot = match lire_depot(&conn, id) {
        Ok(Some(d)) => d,
        Ok(None) => {
            return HttpResponse::NotFound().json(json!({
                "status": "error",
                "message": "Dépôt introuvable"
            }))
        }
        Err(e) => return HttpResponse::InternalServerError().json(json!({"status":"error","message":e})),
    };
    let uq = depot.get("uq").and_then(|v| v.as_str()).unwrap_or("").to_string();
    if let Err(response) = exiger_soi_ou_owner(&req, &uq) {
        return response;
    }
    if depot.get("statut").and_then(|v| v.as_str()) != Some("en_attente") {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Seul un dépôt en attente peut être annulé."
        }));
    }
    match conn.execute(
        "UPDATE wallet_deposits SET statut = 'annule', traite_at = ?1
         WHERE id = ?2 AND statut = 'en_attente'",
        params![maintenant(), id],
    ) {
        Ok(1) => {}
        Ok(_) => {
            return HttpResponse::Conflict().json(json!({
                "status": "error",
                "message": "Le dépôt a déjà été traité."
            }))
        }
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Annulation impossible: {}", e)
            }))
        }
    }
    HttpResponse::Ok().json(json!({ "status": "success", "message": "Dépôt annulé." }))
}

/// POST /api/wallet/depot/webhook — PLACE RÉSERVÉE aux agrégateurs de
/// paiement (CinetPay, Flutterwave, agrégateur mobile money local…).
///
/// Contrat prévu : le provider appelle cette route avec
/// { provider, api_reference, code_transaction, statut } ; le backend
/// vérifiera la signature du provider puis créditera via la même fonction
/// que `owner_confirmer`. En attendant, le canal reste 'manuel'.
pub async fn webhook_aggregateur(_body: web::Json<serde_json::Value>) -> HttpResponse {
    HttpResponse::NotImplemented().json(json!({
        "status": "error",
        "code": "AGGREGATOR_NOT_CONFIGURED",
        "message": "Passerelle de paiement non configurée. Les dépôts sont déclarés manuellement et validés par le propriétaire."
    }))
}

// ====================
// CÔTÉ PROPRIÉTAIRE
// ====================

/// GET /api/owner/depots?statut=en_attente — file des dépôts à traiter.
pub async fn owner_liste(req: HttpRequest, query: web::Query<std::collections::HashMap<String, String>>) -> HttpResponse {
    if let Err(response) = exiger_owner(&req) {
        return response;
    }
    let conn = match ouvrir_index() {
        Ok(c) => c,
        Err(e) => return HttpResponse::InternalServerError().json(json!({"status":"error","message":e})),
    };
    if let Err(e) = expirer_perimes(&conn) {
        return HttpResponse::InternalServerError().json(json!({"status":"error","message":e}));
    }
    let statut = query.get("statut").map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let sql = match &statut {
        Some(_) => format!(
            "SELECT {} FROM wallet_deposits WHERE statut = ?1 ORDER BY created_at DESC LIMIT 100",
            COLONNES
        ),
        None => format!(
            "SELECT {} FROM wallet_deposits ORDER BY
                CASE WHEN statut = 'en_attente' THEN 0 ELSE 1 END,
                created_at DESC LIMIT 100",
            COLONNES
        ),
    };
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Lecture des dépôts impossible: {}", e)
            }))
        }
    };
    let depots: Vec<serde_json::Value> = match &statut {
        Some(s) => stmt
            .query_map(params![s], ligne_vers_json)
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default(),
        None => stmt
            .query_map([], ligne_vers_json)
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default(),
    };
    HttpResponse::Ok().json(json!({ "status": "success", "depots": depots }))
}

/// Crédite un dépôt validé : multi_wallets (devise) + réserve BORB du
/// compte, dans UNE transaction incluant la base du compte (attachée).
/// Partagée entre la validation manuelle (owner) et le futur webhook
/// agrégateur — `canal` distingue la provenance.
async fn crediter_depot(
    conn: &mut Connection,
    depot: &serde_json::Value,
    owner_uq: &str,
) -> Result<serde_json::Value, String> {
    let id = depot.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
    let uq = depot.get("uq").and_then(|v| v.as_str()).unwrap_or("");
    let devise = depot
        .get("devise")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_uppercase();
    // On crédite le montant EXACT réellement envoyé sur le mobile money.
    let montant = depot
        .get("montant_exact")
        .and_then(|v| v.as_f64())
        .or_else(|| depot.get("montant").and_then(|v| v.as_f64()))
        .unwrap_or(0.0);

    let taux_live = taux_live_borb().await;
    let taux = taux_live.get(&devise).copied().unwrap_or(0.0);
    if taux <= 0.0 || !taux.is_finite() {
        return Err(format!("Taux de conversion indisponible pour {}", devise));
    }
    let (_amount_minor, points) =
        match calculate_currency_to_borb_points(montant, taux) {
            Ok(v) => v,
            Err(ConvertedTransferAmountError::Invalid) => {
                return Err("Montant de dépôt invalide".to_string())
            }
            Err(ConvertedTransferAmountError::Precision) => {
                return Err("Montant de dépôt incompatible avec la précision BORB".to_string())
            }
            Err(ConvertedTransferAmountError::TooLarge) => {
                return Err("Montant de dépôt trop élevé".to_string())
            }
        };

    let account_db = chemin_base_compte(uq);
    if !std::path::Path::new(&account_db).exists() {
        return Err("Base du compte déposant introuvable".to_string());
    }
    conn.execute(
        "ATTACH DATABASE ?1 AS user_account",
        params![account_db],
    )
    .map_err(|e| format!("ATTACH compte impossible: {}", e))?;

    let now = maintenant();
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| format!("Transaction impossible: {}", e))?;

    // Garde anti double-crédit : le statut doit encore être en_attente.
    let updated = tx
        .execute(
            "UPDATE wallet_deposits
             SET statut = 'credite', owner_uq = ?1, traite_at = ?2
             WHERE id = ?3 AND statut = 'en_attente'",
            params![owner_uq, now, id],
        )
        .map_err(|e| format!("Validation du dépôt impossible: {}", e))?;
    if updated != 1 {
        return Err("Dépôt déjà traité ou introuvable".to_string());
    }

    tx.execute(
        "INSERT INTO multi_wallets (uq_number, currency, balance, updated_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(uq_number, currency)
         DO UPDATE SET balance = multi_wallets.balance + excluded.balance,
                       updated_at = excluded.updated_at",
        params![uq, devise, montant, now],
    )
    .map_err(|e| format!("Crédit de la devise impossible: {}", e))?;

    tx.execute(
        "UPDATE user_account.balance
         SET points = points + ?1
         WHERE id = 1 AND points <= ?2",
        params![points, i64::MAX - points],
    )
    .map_err(|e| format!("Crédit BORB impossible: {}", e))?;

    tx.commit()
        .map_err(|e| format!("Validation du crédit impossible: {}", e))?;

    Ok(json!({
        "devise": devise,
        "montant": montant,
        "points_borb": points,
    }))
}

/// POST /api/owner/depots/{id}/confirmer — le propriétaire a reçu l'argent
/// sur son mobile money : le wallet et la réserve sont crédités.
pub async fn owner_confirmer(req: HttpRequest, path: web::Path<i64>) -> HttpResponse {
    let claims = match exiger_owner(&req) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let id = path.into_inner();
    let mut conn = match ouvrir_index() {
        Ok(c) => c,
        Err(e) => return HttpResponse::InternalServerError().json(json!({"status":"error","message":e})),
    };
    if let Err(e) = expirer_perimes(&conn) {
        return HttpResponse::InternalServerError().json(json!({"status":"error","message":e}));
    }
    let depot = match lire_depot(&conn, id) {
        Ok(Some(d)) => d,
        Ok(None) => {
            return HttpResponse::NotFound().json(json!({
                "status": "error",
                "message": "Dépôt introuvable"
            }))
        }
        Err(e) => return HttpResponse::InternalServerError().json(json!({"status":"error","message":e})),
    };
    if depot.get("statut").and_then(|v| v.as_str()) != Some("en_attente") {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Seul un dépôt en attente peut être confirmé."
        }));
    }

    match crediter_depot(&mut conn, &depot, &claims.sub).await {
        Ok(credit) => {
            let uq = depot.get("uq").and_then(|v| v.as_str()).unwrap_or("");
            let montant = depot
                .get("montant_exact")
                .and_then(|v| v.as_f64())
                .or_else(|| depot.get("montant").and_then(|v| v.as_f64()))
                .unwrap_or(0.0);
            let devise = depot.get("devise").and_then(|v| v.as_str()).unwrap_or("");
            push_notification(
                uq,
                "wallet",
                "Dépôt validé",
                &format!(
                    "Ton dépôt de {} {} a été confirmé et crédité ({} BORB en réserve).",
                    montant, devise,
                    credit.get("points_borb").and_then(|v| v.as_f64()).unwrap_or(0.0) / 100.0
                ),
                &format!("depot:{}", id),
            );
            HttpResponse::Ok().json(json!({
                "status": "success",
                "deposit_id": id,
                "statut": "credite",
                "credit": credit,
                "message": "Dépôt confirmé et crédité."
            }))
        }
        Err(e) => HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": e
        })),
    }
}

/// POST /api/owner/depots/{id}/refuser — body { motif? } : le propriétaire
/// n'a PAS reçu l'argent (ou litige) : le dépôt est clos sans crédit.
pub async fn owner_refuser(req: HttpRequest, path: web::Path<i64>, body: web::Json<serde_json::Value>) -> HttpResponse {
    let claims = match exiger_owner(&req) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let id = path.into_inner();
    let motif = body
        .get("motif")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let conn = match ouvrir_index() {
        Ok(c) => c,
        Err(e) => return HttpResponse::InternalServerError().json(json!({"status":"error","message":e})),
    };
    let depot = match lire_depot(&conn, id) {
        Ok(Some(d)) => d,
        Ok(None) => {
            return HttpResponse::NotFound().json(json!({
                "status": "error",
                "message": "Dépôt introuvable"
            }))
        }
        Err(e) => return HttpResponse::InternalServerError().json(json!({"status":"error","message":e})),
    };
    if depot.get("statut").and_then(|v| v.as_str()) != Some("en_attente") {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Seul un dépôt en attente peut être refusé."
        }));
    }
    match conn.execute(
        "UPDATE wallet_deposits
         SET statut = 'refuse', motif = ?1, owner_uq = ?2, traite_at = ?3
         WHERE id = ?4 AND statut = 'en_attente'",
        params![motif, claims.sub, maintenant(), id],
    ) {
        Ok(1) => {}
        Ok(_) => {
            return HttpResponse::Conflict().json(json!({
                "status": "error",
                "message": "Le dépôt a déjà été traité."
            }))
        }
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Refus impossible: {}", e)
            }))
        }
    }
    let uq = depot.get("uq").and_then(|v| v.as_str()).unwrap_or("");
    push_notification(
        uq,
        "wallet",
        "Dépôt refusé",
        &format!(
            "Ton dépôt n'a pas été confirmé{}.",
            if motif.is_empty() { String::new() } else { format!(": {}", motif) }
        ),
        &format!("depot:{}", id),
    );
    HttpResponse::Ok().json(json!({ "status": "success", "message": "Dépôt refusé." }))
}
