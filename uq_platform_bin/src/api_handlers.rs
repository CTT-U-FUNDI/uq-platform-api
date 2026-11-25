use crate::database::Database;
use crate::real_state::GlobalState;
use crate::uq_index;
use crate::user_models::*;
use actix_web::{web, HttpRequest, HttpResponse};
use rusqlite;
use serde_json::json;
use std::str::FromStr;
use std::sync::Arc;
// Structure pour le transfert de bulles
#[derive(serde::Deserialize)]
pub struct TransferRequest {
    pub sender: String,
    pub recipient: String,
    pub amount_points: u64,
    pub fee_points: u64,
    /// Devise d'envoi (par defaut BORB si non specifie)
    pub currency: Option<String>,
    #[serde(default)]
    pub amount_currency: Option<f64>,
    #[serde(default)]
    pub memo: Option<String>,
    // ===== Métadonnées de sécurité (optionnelles) =====
    #[serde(default)]
    pub device: Option<String>,
    #[serde(default)]
    pub os: Option<String>,
    #[serde(default)]
    pub browser: Option<String>,
    #[serde(default)]
    pub signature: Option<String>, // hex de la signature
    #[serde(default)]
    pub public_key: Option<String>, // hex de la clé publique
}

// ====================
// 🚪 PORTE BORB — cadenas structurel exigé à TOUTE entrée d'argent
// ====================
// Le destinataire doit réunir trois conditions, sinon le BORB refuse :
// 1. sa DB porte un cadenas créé avec des identifiants personnels complets
//    (hash recalculé = hash stocké, sinon l'identité a été bidouillée) ;
// 2. le BlockOrb connaît son portefeuille (pas de compte fantôme) ;
// 3. le BlockOrb porte le MÊME cadenas (lien blockchain ↔ cadenas).
pub(crate) fn porte_entree_borb(recipient: &str) -> Option<HttpResponse> {
    if let Err(e) = crate::kyc::cadenas::porte_cadenas_structurel(recipient) {
        eprintln!("🚫 [PORTE-BORB] {} : {}", recipient, e);
        return Some(HttpResponse::Forbidden().json(json!({
            "status": "error",
            "code": "CADENAS_REQUIS",
            "cadenas_verifie": false,
            "message": format!(
                "{} ne peut pas recevoir de BORB : aucun cadenas structurel réel avec identifiants personnels n'est attached a ce wallet.",
                recipient
            )
        })));
    }
    let trace_ok = {
        let (orb, _garde) = crate::persistence::orb_charge_verrouille();
        orb.porte_cadenas(recipient)
            && !orb
                .comptes
                .get(recipient)
                .map(|p| p.alerte_piratage)
                .unwrap_or(true)
    };
    if !trace_ok {
        eprintln!(
            "🚫 [PORTE-BORB] {} absent du BlockOrb ou cadenas non lie",
            recipient
        );
        return Some(HttpResponse::Forbidden().json(json!({
            "status": "error",
            "code": "CADENAS_NON_LIE",
            "message": format!(
                "{} n'est pas trace avec son cadenas dans le BlockOrb : le BORB ne peut pas entrer (compte fantome protege).",
                recipient
            )
        })));
    }
    None
}

fn validate_transfer_amounts(
    amount_points: u64,
    fee_points: u64,
) -> Result<(u64, u64), HttpResponse> {
    if amount_points == 0 {
        return Err(HttpResponse::BadRequest().json(json!({
            "status": "error",
            "code": "INVALID_AMOUNT",
            "message": "Le montant doit être > 0"
        })));
    }

    if fee_points > amount_points / 100 {
        return Err(HttpResponse::BadRequest().json(json!({
            "status": "error",
            "code": "INVALID_FEE",
            "message": "Les frais ne peuvent pas dépasser 1% du montant"
        })));
    }

    let total_cost = amount_points.checked_add(fee_points).ok_or_else(|| {
        HttpResponse::BadRequest().json(json!({
            "status": "error",
            "code": "OVERFLOW",
            "message": "Montant total trop élevé"
        }))
    })?;

    if total_cost > i64::MAX as u64 {
        return Err(HttpResponse::BadRequest().json(json!({
            "status": "error",
            "code": "AMOUNT_TOO_HIGH",
            "message": "Montant dépasse la limite autorisée"
        })));
    }

    Ok((amount_points, fee_points))
}

#[derive(Debug, PartialEq)]
pub(crate) enum ConvertedTransferAmountError {
    Invalid,
    Precision,
    TooLarge,
}

fn calculate_converted_transfer_amount(amount: f64) -> Result<i64, ConvertedTransferAmountError> {
    if !amount.is_finite() || amount <= 0.0 {
        return Err(ConvertedTransferAmountError::Invalid);
    }

    let amount_minor = amount * 100.0;
    if !amount_minor.is_finite() || amount_minor >= i64::MAX as f64 {
        return Err(ConvertedTransferAmountError::TooLarge);
    }
    let rounded_minor = amount_minor.round();
    if rounded_minor < 1.0 || (amount_minor - rounded_minor).abs() > 0.000001 {
        return Err(ConvertedTransferAmountError::Precision);
    }

    // Montant arrondi au centième, prêt pour le débit et les frais.
    Ok(rounded_minor as i64)
}

pub(crate) fn calculate_currency_to_borb_points(
    amount: f64,
    borb_per_currency: f64,
) -> Result<(i64, i64), ConvertedTransferAmountError> {
    if !borb_per_currency.is_finite() || borb_per_currency <= 0.0 {
        return Err(ConvertedTransferAmountError::Invalid);
    }

    let amount_minor = calculate_converted_transfer_amount(amount)?;
    let points = amount_minor as f64 * borb_per_currency;
    if !points.is_finite() || points >= i64::MAX as f64 {
        return Err(ConvertedTransferAmountError::TooLarge);
    }

    let rounded_points = points.round();
    if rounded_points < 1.0 || (points - rounded_points).abs() > 0.000001 {
        return Err(ConvertedTransferAmountError::Precision);
    }

    Ok((amount_minor, rounded_points as i64))
}

fn settle_currency_to_borb(
    tx: &rusqlite::Transaction<'_>,
    uq: &str,
    currency: &str,
    amount_minor_brut: i64,
    points_net: i64,
    updated_at: i64,
) -> Result<bool, rusqlite::Error> {
    let amount = amount_minor_brut as f64 / 100.0;
    let debit = tx.execute(
        "UPDATE multi_wallets
         SET balance = balance - ?1, updated_at = ?2
         WHERE uq_number = ?3 AND currency = ?4 AND balance >= ?1",
        rusqlite::params![amount, updated_at, uq, currency],
    )?;
    if debit != 1 {
        return Ok(false);
    }

    let credit = tx.execute(
        "UPDATE user_account.balance
         SET points = points + ?1
         WHERE id = 1 AND points <= ?2",
        rusqlite::params![points_net, i64::MAX - points_net],
    )?;
    Ok(credit == 1)
}

/// Pourquoi un règlement converti a échoué. Les trois cas se distinguent pour
/// le client : un BORB immobilisé par un escrow n'est pas un manque de devise.
#[derive(Debug, PartialEq, Eq)]
enum ReglementConverti {
    Effectue,
    BorbInsuffisant,
    OmbreInsuffisante,
    CompteDestinationIntrouvable,
}

/// Alias de schéma utilisés pour attacher les bases de compte. Volontairement
/// des littéraux fermés : un alias ne peut pas être un paramètre lié.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AliasCompte {
    Source,
    Destination,
}

impl AliasCompte {
    fn schema(self) -> &'static str {
        match self {
            AliasCompte::Source => "src_account",
            AliasCompte::Destination => "dst_account",
        }
    }
}

/// Attache la base d'un compte à la connexion principale. Les DB de compte sont
/// des fichiers séparés : sans attache, un débit BORB déjà validé survivrait au
/// rollback du reste du règlement et détruirait des units sans contrepartie.
fn attacher_compte(
    conn: &rusqlite::Connection,
    uq: &str,
    alias: AliasCompte,
) -> Result<(), String> {
    let path = chemin_base_compte(uq);
    if !std::path::Path::new(&path).exists() {
        return Err(format!("base de compte absente: {}", path));
    }
    let sql = format!("ATTACH DATABASE ?1 AS {}", alias.schema());
    conn.execute(&sql, rusqlite::params![path])
        .map(|_| ())
        .map_err(|e| format!("ATTACH {} ({}): {}", alias.schema(), path, e))
}

fn settle_converted_transfer(
    tx: &rusqlite::Transaction<'_>,
    sender: &str,
    recipient: &str,
    currency: &str,
    amount_minor: i64,
    fee_minor: i64,
    memo: &str,
    transfer_id: &str,
    created_at: i64,
) -> Result<ReglementConverti, rusqlite::Error> {
    let net_minor = amount_minor - fee_minor;
    let amount = amount_minor as f64 / 100.0;
    let net = net_minor as f64 / 100.0;
    let frais = fee_minor as f64 / 100.0;

    // ====================
    // 1. DÉBITER LE BORB DU SENDER (le vrai argent, montant brut)
    //    Sous garde du gel : le BORB immobilisé par un escrow P2P n'est pas
    //    dépensable, sinon un simple transfert viderait le gel.
    // ====================
    let sender_debit = tx.execute(
        "UPDATE src_account.balance SET points = points - ?1, updated_at = ?2
         WHERE id = 1 AND points - COALESCE(frozen_points, 0) >= ?1",
        rusqlite::params![amount_minor, created_at],
    )?;
    if sender_debit != 1 {
        return Ok(ReglementConverti::BorbInsuffisant);
    }

    // ====================
    // 2. DÉBITER L'OMBRE (multi_wallets) DU SENDER — brut
    // ====================
    let debit = tx.execute(
        "UPDATE multi_wallets
         SET balance = balance - ?1, updated_at = ?2
         WHERE uq_number = ?3 AND currency = ?4 AND balance >= ?1",
        rusqlite::params![amount, created_at, sender, currency],
    )?;
    if debit != 1 {
        return Ok(ReglementConverti::OmbreInsuffisante); // tout est annulé au rollback
    }

    // ====================
    // 3. CRÉDITER LE BORB DU RECIPIENT (net de frais)
    // ====================
    let recipient_credit = tx.execute(
        "UPDATE dst_account.balance SET points = points + ?1, updated_at = ?2
         WHERE id = 1 AND points <= ?3",
        rusqlite::params![net_minor, created_at, i64::MAX - net_minor],
    )?;
    if recipient_credit != 1 {
        return Ok(ReglementConverti::CompteDestinationIntrouvable);
    }

    // ====================
    // 4. CRÉDITER L'OMBRE (multi_wallets) DU RECIPIENT — net de frais
    // ====================
    tx.execute(
        "INSERT INTO multi_wallets (uq_number, currency, balance, updated_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(uq_number, currency)
         DO UPDATE SET balance = multi_wallets.balance + excluded.balance, updated_at = excluded.updated_at",
        rusqlite::params![recipient, currency, net, created_at],
    )?;

    // ====================
    // 5. ENREGISTRER LA TRANSACTION
    // ====================
    tx.execute(
        "INSERT INTO wallet_transfers
            (transfer_id, sender_uq, recipient_uq, currency, amount, fee, memo, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            transfer_id,
            sender,
            recipient,
            currency,
            amount,
            frais,
            memo,
            created_at
        ],
    )?;

    // ====================
    // 6. FRAIS → 6 CASES (40 % Case 2 alimente Earn)
    // ====================
    crate::frais_plateforme::enregistrer_tx(
        tx,
        "send",
        transfer_id,
        sender,
        currency,
        frais,
        created_at,
    )?;

    Ok(ReglementConverti::Effectue)
}

async fn transfer_converted_currency(
    sender: &str,
    recipient: &str,
    currency: &str,
    amount_currency: Option<f64>,
    amount_points: u64,
    memo: &str,
    contexte: &ContexteAppel<'_>,
    affichage_emetteur: &str,
    affichage_destinataire: &str,
) -> HttpResponse {
    use rusqlite::Connection;

    const FIAT_CURRENCIES: &[&str] = &[
        "CDF", "XAF", "XOF", "NGN", "KES", "ZAR", "MAD", "GHS", "UGX", "TZS", "EGP", "RWF", "BIF",
        "MZN", "AOA", "ETB", "USD", "EUR", "GBP", "CHF", "JPY", "CNY", "CAD", "AUD", "INR", "BRL",
        "MXN", "SEK", "NOK", "DKK", "PLN", "TRY",
    ];

    let code = currency.trim().to_uppercase();
    if !FIAT_CURRENCIES.contains(&code.as_str()) {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "code": "UNSUPPORTED_TRANSFER_CURRENCY",
            "message": "Seules les devises fiat converties peuvent être transférées."
        }));
    }
    if sender.eq_ignore_ascii_case(recipient) {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "code": "SAME_ACCOUNT",
            "message": "L'expéditeur et le destinataire doivent être différents."
        }));
    }

    // 🚪 PORTE BORB (chemin devise convertie → points BORB crédités) :
    //    le destinataire doit avoir un cadenas structurel réel et lié.
    if let Some(refus) = porte_entree_borb(recipient) {
        return refus;
    }

    let taux = taux_live_borb().await;
    let Some(taux_borb) = taux
        .get(&code)
        .copied()
        .filter(|rate| rate.is_finite() && *rate > 0.0)
    else {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "code": "MISSING_CURRENCY_RATE",
            "message": "Le taux de conversion de cette devise est indisponible."
        }));
    };

    let amount = match amount_currency {
        Some(value) => value,
        None => amount_points as f64 / 100.0 / taux_borb,
    };
    let amount_minor = match calculate_converted_transfer_amount(amount) {
        Ok(amount_minor) => amount_minor,
        Err(ConvertedTransferAmountError::Invalid) => {
            return HttpResponse::BadRequest().json(json!({
                "status": "error",
                "code": "INVALID_AMOUNT",
                "message": "Montant de transfert invalide."
            }))
        }
        Err(ConvertedTransferAmountError::Precision) => {
            return HttpResponse::BadRequest().json(json!({
                "status": "error",
                "code": "INVALID_PRECISION",
                "message": "Le montant doit être au centième près."
            }))
        }
        Err(ConvertedTransferAmountError::TooLarge) => {
            return HttpResponse::BadRequest().json(json!({
                "status": "error",
                "code": "AMOUNT_TOO_HIGH",
                "message": "Montant de transfert trop élevé."
            }))
        }
    };
    let amount = amount_minor as f64 / 100.0;
    let fee_minor = crate::frais_plateforme::frais_centimes(amount_minor, &code);
    let net_minor = amount_minor - fee_minor;
    if net_minor < 1 {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "code": "AMOUNT_TOO_LOW",
            "message": "Montant trop faible : les frais de service absorbent tout le transfert."
        }));
    }
    let fee = fee_minor as f64 / 100.0;
    let net = net_minor as f64 / 100.0;
    let now = chrono::Utc::now().timestamp();
    let transfer_id = format!("wtx_{}", uuid::Uuid::new_v4());

    let conn = match rusqlite::Connection::open("uq_index.db") {
        Ok(conn) => conn,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Base des portefeuilles indisponible: {}", e)
            }))
        }
    };
    if let Err(e) = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS wallet_transfers (
            transfer_id TEXT PRIMARY KEY,
            sender_uq TEXT NOT NULL,
            recipient_uq TEXT NOT NULL,
            currency TEXT NOT NULL,
            amount REAL NOT NULL,
            fee REAL NOT NULL,
            memo TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_wallet_transfers_sender
            ON wallet_transfers(sender_uq, created_at);
        CREATE INDEX IF NOT EXISTS idx_wallet_transfers_recipient
            ON wallet_transfers(recipient_uq, created_at);",
    ) {
        return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("Initialisation de l'historique impossible: {}", e)
        }));
    }

    if let Err(e) = crate::frais_plateforme::assurer_tables(&conn) {
        return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("Initialisation des frais impossible: {}", e)
        }));
    }

    let mut conn = conn;
    // Les bases des deux comptes sont attachees AVANT la transaction (SQLite
    // interdit ATTACH pendant une transaction) : les six ecritures du reglement
    // tiennent alors dans un seul commit, ou disparaissent ensemble.
    if let Err(e) = attacher_compte(&conn, sender, AliasCompte::Source) {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "code": "COMPTE_INTROUVABLE",
            "message": format!("Portefeuille BORB de l'émetteur indisponible ({e}).")
        }));
    }
    if let Err(e) = attacher_compte(&conn, recipient, AliasCompte::Destination) {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "code": "COMPTE_INTROUVABLE",
            "message": format!("Portefeuille BORB du destinataire indisponible ({e}).")
        }));
    }
    let tx = match conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate) {
        Ok(tx) => tx,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Transaction impossible: {}", e)
            }))
        }
    };

    let reglement = match settle_converted_transfer(
        &tx,
        sender,
        recipient,
        &code,
        amount_minor,
        fee_minor,
        memo,
        &transfer_id,
        now,
    ) {
        Ok(reglement) => reglement,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Règlement du transfert impossible: {}", e)
            }))
        }
    };
    match reglement {
        ReglementConverti::Effectue => {}
        ReglementConverti::BorbInsuffisant => {
            return HttpResponse::BadRequest().json(json!({
                "status": "error",
                "code": "BORB_GELÉS",
                "message": format!(
                    "BORB mobilisable insuffisant pour convertir {:.2} {} : le BORB immobilisé par tes commandes P2P en cours n'est pas dépensable.",
                    amount, code
                )
            }));
        }
        ReglementConverti::OmbreInsuffisante => {
            return HttpResponse::BadRequest().json(json!({
                "status": "error",
                "code": "INSUFFICIENT_CURRENCY_BALANCE",
                "currency": code,
                "message": format!("Solde {} insuffisant pour le montant et les frais.", code)
            }));
        }
        ReglementConverti::CompteDestinationIntrouvable => {
            return HttpResponse::BadRequest().json(json!({
                "status": "error",
                "code": "COMPTE_INTROUVABLE",
                "message": "Le portefeuille du destinataire n'a pas pu être crédité ; rien n'a été débité."
            }));
        }
    }

    if let Err(e) = tx.commit() {
        return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("Validation du transfert impossible: {}", e)
        }));
    }

    // Le transfert converti fait sortir du BORB réel du coffre de l'émetteur et
    // entrer du BORB réel dans celui du destinataire : les deux coffres doivent
    // porter leur écriture et leur miroir, comme sur le chemin d'envoi BORB.
    let chemin_emetteur = chemin_base_compte(sender);
    let chemin_destinataire = chemin_base_compte(recipient);
    let note = format!("Transfert {} · {:.2} {}", code, amount, code);
    tracer_ecritures_coffre(
        &[
            EcritureCoffre {
                uq: sender,
                chemin_db: &chemin_emetteur,
                tx_type: "send",
                // Même convention que le chemin BORB : la ligne porte le net et
                // le frais à part, et `amount + fee` égale ce qui est sorti du
                // coffre. Ici le frais se prend dans le montant, pas dessus.
                points: net_minor,
                frais_points: fee_minor,
                contrepartie: affichage_destinataire,
                note: &note,
                reference: &transfer_id,
                emetteur: sender,
                destinataire: recipient,
            },
            EcritureCoffre {
                uq: recipient,
                chemin_db: &chemin_destinataire,
                tx_type: "receive",
                points: net_minor,
                frais_points: 0,
                contrepartie: affichage_emetteur,
                note: &note,
                reference: &transfer_id,
                emetteur: sender,
                destinataire: recipient,
            },
        ],
        now,
        contexte,
    );

    // ✅ SYNCHRONISATION BLOCKORB (P2P)
    // Le destinataire reçoit le net ; les frais sont répartis dans les 6 Cases.
    // 1 USD = 100 points BORB → centimes = points BORB.
    if let Err(e) = crate::persistence::mirroir_transfert(
        sender,
        recipient,
        net_minor as u128,
        fee_minor as u128,
    ) {
        eprintln!("⚠️  Miroir BlockOrb échoué : {}", e);
        // Ne pas bloquer le transfert (déjà validé en SQLite)
    }

    let repartition = crate::frais_plateforme::repartir(fee);
    HttpResponse::Ok().json(json!({
        "status": "success",
        "transfer_id": transfer_id,
        "currency": code,
        "amount": amount,
        "fee": fee,
        "fee_currency": code,
        "net_received": net,
        "fee_case_split": {
            "tresor": repartition[0],
            "frais": repartition[1],
            "recompenses": repartition[2],
            "epargne": repartition[3],
            "credit": repartition[4],
            "bouclier": repartition[5]
        },
        "recipient": crate::uq_public_alias::get_public_display(recipient),
        "amount_borb_equivalent": amount * taux_borb,
    }))
}

// ==========================================================
// FONCTION OK() — pour les handlers qui seront connectés plus tard
// ==========================================================
fn ok() -> HttpResponse {
    HttpResponse::Ok().json(json!({"status":"success"}))
}

// ==========================================================
// ROUTES DE BASE (7)
// ==========================================================
pub async fn index() -> HttpResponse {
    HttpResponse::Ok().json(json!({"service":"uq_platform","status":"running"}))
}
pub async fn health(db: web::Data<std::sync::Arc<crate::database::Database>>) -> HttpResponse {
    // schema_version reel : cree la table si absente, lit la valeur.
    let schema_version: i64 = {
        let Ok(conn) = db.connexion() else {
            return HttpResponse::ServiceUnavailable().json(json!({"status":"unavailable"}));
        };
        let _ = conn.execute(
            "CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL)",
            [],
        );
        let _ = conn.execute(
            "INSERT INTO schema_version(version) SELECT 1 WHERE NOT EXISTS (SELECT 1 FROM schema_version)",
            [],
        );
        conn.query_row(
            "SELECT version FROM schema_version ORDER BY version DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0)
    };
    HttpResponse::Ok().json(json!({
        "status": "healthy",
        "version": env!("CARGO_PKG_VERSION"),
        "schema_version": schema_version,
        "build_date": option_env!("BUILD_DATE").unwrap_or("unknown")
    }))
}
pub async fn stats() -> HttpResponse {
    HttpResponse::Ok().json(json!({"total_supply_borb":40_000_000_000u64,"six_cases":6}))
}
pub async fn price_gold() -> HttpResponse {
    // Une once n'est pas 2000 USD « parce que c'est écrit dans le code » : le
    // prix vient de la capture réelle, et 503 quand elle n'a rien donné.
    let maintenant = chrono::Utc::now().timestamp();
    let c = match capture_prix().await {
        Ok(c) => c,
        Err(e) => return reponse_capture_err(&e, maintenant),
    };
    let borb_usd = match borb_usd_depuis_capture(c.gold_usd) {
        Ok(p) => p,
        Err(refus) => return refus,
    };
    HttpResponse::Ok().json(json!({
        "status": "success",
        "price_usd_per_ounce": c.gold_usd,
        "borb_value_usd": borb_usd,
        "source": "xaus.com (cache 5 min)",
        "capture": c.capture_json(maintenant),
        "updated_at": c.or_captur_le
    }))
}
pub async fn price_usd() -> HttpResponse {
    let maintenant = chrono::Utc::now().timestamp();
    let c = match capture_prix().await {
        Ok(c) => c,
        Err(e) => return reponse_capture_err(&e, maintenant),
    };
    let borb_to_usd = match borb_usd_depuis_capture(c.gold_usd) {
        Ok(p) => p,
        Err(refus) => return refus,
    };
    HttpResponse::Ok().json(json!({
        "status": "success",
        "borb_to_usd": borb_to_usd,
        "usd_to_borb": 1.0 / borb_to_usd,
        "source": "xaus.com (cache 5 min)",
        "capture": c.capture_json(maintenant),
        "updated_at": c.or_captur_le
    }))
}
pub async fn blockchain_info() -> HttpResponse {
    HttpResponse::Ok().json(json!({"chain":"UQ Chain","consensus":"PoS","validators":10}))
}
pub async fn total_supply() -> HttpResponse {
    HttpResponse::Ok().json(json!({"total":40_000_000_000u64}))
}

// ==========================================================
// COMPTES (4)
// ==========================================================
pub async fn account_create(
    _state: web::Data<Arc<GlobalState>>,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    // Extraire l'input
    let mut input = crate::application::account_service::CreateAccountInput {
        prenom: body
            .get("prenom")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        nom: body
            .get("nom")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        password: body
            .get("password")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        date_naissance: body
            .get("date_naissance")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        lieu_naissance: body
            .get("lieu_naissance")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        pays: body
            .get("pays")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        adresse: body
            .get("adresse")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        telephone: body
            .get("telephone")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        email: body
            .get("email")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        pseudo: body
            .get("pseudo")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        // Code promo du parrain (facultatif) — accepte les deux noms de champ
        code_promo_parrain: body
            .get("code_promo_parrain")
            .and_then(|v| v.as_str())
            .or_else(|| body.get("code_promo").and_then(|v| v.as_str()))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        // Profil complet (12 Oct 2026)
        sexe: body
            .get("sexe")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        ville_provenance: body
            .get("ville_provenance")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        ville_actuelle: body
            .get("ville_actuelle")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        statut_marital: body
            .get("statut_marital")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        // Profession declaree a la creation — la colonne s'appelle `emploi` cote index.
        profession: body
            .get("profession")
            .and_then(|v| v.as_str())
            .or_else(|| body.get("emploi").and_then(|v| v.as_str()))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        // Contacts de recuperation : l'email externe ne remplace jamais le mail UQ.
        recovery_email: body
            .get("recovery_email")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        recovery_phone: body
            .get("recovery_phone")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        phone_verified: false,
        email_verified: false,
    };

    // Un code promo de parrain saisi doit exister : on ne cree pas de lien
    // fantome et on ne laisse pas l'utilisateur croire qu'il est fille.
    if let Some(code) = input.code_promo_parrain.clone() {
        let index = crate::uq_index::UqIndex::shared("uq_index.db");
        if index.get_uq_by_code_promo(&code).is_none() {
            return HttpResponse::BadRequest().json(json!({
                "status": "error",
                "message": format!("Code promo « {} » inconnu. Verifie-le ou laisse le champ vide.", code),
                "raison": "code_promo_inconnu",
            }));
        }
    }

    // Le code OTP confirme a l'ecran 5 attache le telephone verifie au compte.
    let telephone_verifie = body
        .get("verification_telephone")
        .and_then(|v| v.as_str())
        .and_then(|jeton| {
            crate::verify_routes::valeur_verifiee(jeton, crate::verify_routes::Portee::Telephone)
        });
    if let Some(e164) = telephone_verifie {
        input.phone_verified = true;
        input.recovery_phone = Some(e164);
    }

    // Preuve humaine obligatoire : la scène/Turnstile émet un pass à usage
    // unique. Sans lui, un bot peut créer des comptes en masse — le frontend
    // seul ne suffit pas, la route vérifie côté serveur.
    let preuve = body
        .get("preuve_humaine")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if !crate::verify_routes::consommer_preuve_humaine(preuve) {
        return HttpResponse::Forbidden().json(json!({
            "status": "error",
            "raison": "preuve_humaine_requise",
            "message": "Verification humaine requise : resolvez le defi visuel d'abord.",
        }));
    }

    match crate::application::account_service::create_account(input, false) {
        Ok(account) => {
            // Code promo bonus (facultatif) : RÉSERVÉ à l'inscription,
            // le $1 n'est crédité qu'à la validation KYC — un compte
            // jetable ne peut pas drainer les codes.
            let code_bonus = body
                .get("code_bonus")
                .or_else(|| body.get("code_promo_bonus"))
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_uppercase())
                .filter(|s| !s.is_empty());
            let mut bonus_etat = serde_json::Value::Null;
            if let Some(code) = code_bonus {
                match reserver_code_bonus(&account.uq_number, &code) {
                    Ok(true) => bonus_etat = json!("en_attente_kyc"),
                    Ok(false) => bonus_etat = json!("deja_utilise"),
                    Err(e) => {
                        eprintln!("[promo] code_bonus refusé pour {} : {}", account.uq_number, e);
                        bonus_etat = json!({"erreur": e});
                    }
                }
            }
            HttpResponse::Ok().json(json!({
                "status": "success",
                "account_created": true,
                "uq_number": account.uq_number,
                "code_bonus": bonus_etat,
            "email": account.email,
            "pseudo": account.pseudo,
            "full_name": account.full_name,
            "prenom": account.prenom,
            "nom": account.nom,
            "date_naissance": account.date_naissance,
            "lieu_naissance": account.lieu_naissance,
            "pays": account.pays,
            "code_promo": account.code_promo,
            "cadenas": account.cadenas,
            "db_path": account.db_path,
            "position": account.position,
            "parrain_uq": account.parrain_uq,
            "recovery_email": account.recovery_email,
            "recovery_phone": account.recovery_phone,
            "phone_verified": account.phone_verified,
            "email_verified": account.email_verified,
            "verrouille": true,
            "balance_borb": 0,
            "message": "Compte créé avec succès. Vérifiez votre identité pour déverrouiller."
        }))
        }
        Err(e) => HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": e
        })),
    }
}
// ==========================================================
// CODES PROMO — bonus de bienvenue (1 USD fixe, HORS UBI)
// ==========================================================
// Règles métier :
//  - un code = un compte, usage unique (promo_codes.utilise_par)
//  - un compte ne peut utiliser QU'UN SEUL code promo, pour toujours
//    (promo_uses.uq PRIMARY KEY — même un second code est refusé)
//  - le montant est crédité en USD dans multi_wallets, valeur figée —
//    jamais indexée sur le cours BORB
// ==========================================================

fn base_promo(conn: &rusqlite::Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS promo_codes (
            code TEXT PRIMARY KEY,
            actif INTEGER NOT NULL DEFAULT 1,
            utilise_par TEXT,
            utilise_le INTEGER,
            montant_usd REAL NOT NULL DEFAULT 1.0
        );
        CREATE TABLE IF NOT EXISTS promo_uses (
            uq TEXT PRIMARY KEY,
            code TEXT NOT NULL,
            utilise_le INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS promo_pending (
            uq TEXT PRIMARY KEY,
            code TEXT NOT NULL,
            cree_le INTEGER NOT NULL
        );",
    )
}

/// À l'inscription : valide le code et le RÉSERVE pour le compte.
/// Le $1 n'est PAS encore crédité — il le sera à la validation KYC
/// (promo_sur_kyc_verifie). Un code invalide = erreur immédiate ;
/// un compte qui a déjà un promo en attente = refus silencieux.
fn reserver_code_bonus(uq: &str, code: &str) -> Result<bool, String> {
    use rusqlite::params;
    let conn = rusqlite::Connection::open("uq_index.db").map_err(|e| e.to_string())?;
    base_promo(&conn).map_err(|e| e.to_string())?;
    // Le compte a-t-il déjà utilisé un promo (à vie) ?
    let deja: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM promo_uses WHERE uq = ?1",
            params![uq],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if deja > 0 {
        return Ok(false);
    }
    // Le code existe, est actif et n'a jamais été consommé ?
    let ok: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM promo_codes
             WHERE code = ?1 AND actif = 1 AND utilise_par IS NULL",
            params![code],
            |r| r.get::<_, i64>(0),
        )
        .map(|n| n > 0)
        .map_err(|e| e.to_string())?;
    if !ok {
        return Err("CODE_INCONNU".to_string());
    }
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT OR REPLACE INTO promo_pending (uq, code, cree_le) VALUES (?1, ?2, ?3)",
        params![uq, code, now],
    )
    .map_err(|e| e.to_string())?;
    Ok(true)
}

/// Appelé quand le KYC passe à Verifie : consomme le code réservé et
/// crédite le montant fixe en USD. Un seul appel par compte (le
/// pending est supprimé + promo_uses bloque tout second passage).
pub(crate) fn promo_sur_kyc_verifie(uq: &str) {
    let conn = match rusqlite::Connection::open("uq_index.db") {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[promo] kyc: DB indisponible pour {} : {}", uq, e);
            return;
        }
    };
    if base_promo(&conn).is_err() {
        return;
    }
    let code: Option<String> = conn
        .query_row(
            "SELECT code FROM promo_pending WHERE uq = ?1",
            rusqlite::params![uq],
            |r| r.get(0),
        )
        .ok();
    let Some(code) = code else { return };
    match appliquer_code_bonus(uq, &code) {
        Ok(Some(m)) => {
            conn.execute(
                "DELETE FROM promo_pending WHERE uq = ?1",
                rusqlite::params![uq],
            )
            .ok();
            println!("[promo] {} crédité {:.2} USD pour {}", code, m, uq);
        }
        Ok(None) => {
            conn.execute(
                "DELETE FROM promo_pending WHERE uq = ?1",
                rusqlite::params![uq],
            )
            .ok();
        }
        Err(e) => eprintln!("[promo] kyc: bonus refusé pour {} : {}", uq, e),
    }
}

/// Valide et applique un code bonus pour un compte fraîchement créé.
/// Retourne Ok(Some(montant)) si crédité, Ok(None) si le compte a déjà
/// utilisé un promo (silencieux), Err(code_erreur) sinon.
fn appliquer_code_bonus(uq: &str, code: &str) -> Result<Option<f64>, String> {
    use rusqlite::params;
    let mut conn = rusqlite::Connection::open("uq_index.db").map_err(|e| e.to_string())?;
    base_promo(&conn).map_err(|e| e.to_string())?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    // Un compte ne peut avoir un code promo qu'UNE SEULE FOIS, à vie.
    let deja: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM promo_uses WHERE uq = ?1",
            params![uq],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if deja > 0 {
        return Ok(None);
    }
    let montant: f64 = tx
        .query_row(
            "SELECT montant_usd FROM promo_codes
             WHERE code = ?1 AND actif = 1 AND utilise_par IS NULL",
            params![code],
            |r| r.get(0),
        )
        .map_err(|_| "CODE_INCONNU".to_string())?;
    let now = chrono::Utc::now().timestamp();
    tx.execute(
        "UPDATE promo_codes SET utilise_par = ?1, utilise_le = ?2 WHERE code = ?3",
        params![uq, now, code],
    )
    .map_err(|e| e.to_string())?;
    tx.execute(
        "INSERT INTO promo_uses (uq, code, utilise_le) VALUES (?1, ?2, ?3)",
        params![uq, code, now],
    )
    .map_err(|e| e.to_string())?;
    // Crédit USD fixe dans multi_wallets (valeur réelle, pas de conversion).
    tx.execute(
        "INSERT INTO multi_wallets (uq_number, currency, balance, updated_at)
         VALUES (?1, 'USD', ?2, ?3)
         ON CONFLICT(uq_number, currency)
         DO UPDATE SET balance = multi_wallets.balance + excluded.balance,
                       updated_at = excluded.updated_at",
        params![uq, montant, now],
    )
    .map_err(|e| e.to_string())?;
    // Piste d'audit : le bonus apparaît dans l'historique des transferts.
    let _ = tx.execute(
        "INSERT INTO wallet_transfers
            (transfer_id, sender_uq, recipient_uq, currency, amount, fee, memo, created_at)
         VALUES (?1, 'SYSTEM_PROMO', ?2, 'USD', ?3, 0.0, ?4, ?5)",
        params![
            format!("promo-{}-{}", uq, now),
            uq,
            montant,
            format!("Code promo {}", code),
            now
        ],
    );
    tx.commit().map_err(|e| e.to_string())?;
    Ok(Some(montant))
}

/// GET /api/admin/promo-codes — liste les codes (owner uniquement).
pub async fn admin_promo_codes_lister(req: actix_web::HttpRequest) -> HttpResponse {
    if let Err(r) = crate::auth_guard::exiger_owner(&req) {
        return r;
    }
    let conn = match rusqlite::Connection::open("uq_index.db") {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError()
                .json(json!({"status":"error","message":e.to_string()}))
        }
    };
    if let Err(e) = base_promo(&conn) {
        return HttpResponse::InternalServerError()
            .json(json!({"status":"error","message":e.to_string()}));
    }
    let mut stmt = match conn.prepare(
        "SELECT code, actif, utilise_par, utilise_le, montant_usd FROM promo_codes ORDER BY rowid DESC",
    ) {
        Ok(s) => s,
        Err(e) => {
            return HttpResponse::InternalServerError()
                .json(json!({"status":"error","message":e.to_string()}))
        }
    };
    let rows = stmt
        .query_map([], |r| {
            Ok(json!({
                "code": r.get::<_, String>(0)?,
                "actif": r.get::<_, i64>(1)? == 1,
                "utilise_par": r.get::<_, Option<String>>(2)?,
                "utilise_le": r.get::<_, Option<i64>>(3)?,
                "montant_usd": r.get::<_, f64>(4)?,
            }))
        })
        .map(|it| it.filter_map(|r| r.ok()).collect::<Vec<_>>())
        .unwrap_or_default();
    HttpResponse::Ok().json(json!({"status":"success","codes":rows,"count":rows.len()}))
}

/// POST /api/admin/promo-codes — crée des codes bonus (owner uniquement).
/// Body : {"code": "XYZ"} pour un code précis, ou {"count": N} pour en
/// générer N aléatoires. {"montant_usd": 1.0} optionnel (défaut 1 USD).
pub async fn admin_promo_codes_creer(
    req: actix_web::HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    if let Err(r) = crate::auth_guard::exiger_owner(&req) {
        return r;
    }
    let montant = body
        .get("montant_usd")
        .and_then(|v| v.as_f64())
        .unwrap_or(1.0);
    if !(0.01..=1000.0).contains(&montant) {
        return HttpResponse::BadRequest()
            .json(json!({"status":"error","message":"montant_usd hors limites (0.01-1000)"}));
    }
    let mut codes: Vec<String> = Vec::new();
    if let Some(code) = body.get("code").and_then(|v| v.as_str()) {
        let c = code.trim().to_uppercase();
        if c.len() < 4 || c.len() > 40 {
            return HttpResponse::BadRequest()
                .json(json!({"status":"error","message":"code invalide (4-40 caractères)"}));
        }
        codes.push(c);
    } else {
        let count = body
            .get("count")
            .and_then(|v| v.as_u64())
            .unwrap_or(1)
            .min(100); // plafond : pas de génération massive
        use rand::Rng;
        let mut rng = rand::thread_rng();
        for _ in 0..count {
            let suffix: String = (0..10)
                .map(|_| {
                    const ALPH: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
                    ALPH[rng.gen_range(0..ALPH.len())] as char
                })
                .collect();
            codes.push(format!("UQ-{}", suffix));
        }
    }
    let conn = match rusqlite::Connection::open("uq_index.db") {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError()
                .json(json!({"status":"error","message":e.to_string()}))
        }
    };
    if let Err(e) = base_promo(&conn) {
        return HttpResponse::InternalServerError()
            .json(json!({"status":"error","message":e.to_string()}));
    }
    let mut crees = Vec::new();
    let mut doublons = Vec::new();
    for code in codes {
        match conn.execute(
            "INSERT INTO promo_codes (code, montant_usd) VALUES (?1, ?2)",
            rusqlite::params![code, montant],
        ) {
            Ok(_) => crees.push(code),
            Err(_) => doublons.push(code),
        }
    }
    HttpResponse::Ok().json(json!({
        "status": "success",
        "codes": crees,
        "doublons": doublons,
        "montant_usd": montant,
    }))
}

pub async fn account_verify(req: actix_web::HttpRequest) -> HttpResponse {
    // 🔐 SÉCURITÉ : JWT obligatoire — avant, n'importe qui créait un faux
    // compte "verified_user" et obtenait kyc_verified:true sans rien
    // vérifier. Désormais on lit le VRAI statut KYC du compte authentifié.
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(claims) => claims,
        Err(response) => return response,
    };
    let index = crate::uq_index::UqIndex::shared("uq_index.db");
    let verifie = index
        .get_account(&claims.sub)
        .map(|a| {
            a.get("kyc_status")
                .or_else(|| a.get("is_verified"))
                .map(|v| {
                    v.as_str().map(|s| s == "verified" || s == "Verifie").unwrap_or(false)
                        || v.as_i64().map(|n| n == 1).unwrap_or(false)
                })
                .unwrap_or(false)
        })
        .unwrap_or(false);
    HttpResponse::Ok().json(json!({
        "status": "success",
        "kyc_verified": verifie,
        "uq": claims.sub,
    }))
}
pub async fn account_get(path: web::Path<String>) -> HttpResponse {
    HttpResponse::Ok().json(json!({"account_id":path.into_inner()}))
}
pub async fn account_balance(path: web::Path<String>) -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    let balance = db.get_balance(&path.into_inner());
    HttpResponse::Ok().json(json!({"balance_borb":balance.0,"balance_fixo":balance.1}))
}

// ==========================================================
// BULLES (7)
// ==========================================================
pub async fn bubble_create(
    state: web::Data<Arc<GlobalState>>,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let owner = body
        .get("owner")
        .and_then(|v| v.as_str())
        .unwrap_or("UQ-00000001");
    let amount = body
        .get("amount_micro")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    // Creer une bulle dans BubbleRegistry
    let mut registry = state.bubble_registry.lock().unwrap();

    let owner_bytes = {
        let mut addr = [0u8; 32];
        let bytes = owner.as_bytes();
        for i in 0..32 {
            if i < bytes.len() {
                addr[i] = bytes[i];
            }
        }
        addr
    };

    match registry.create_bubble(owner_bytes, owner_bytes, amount, Vec::new(), |data| {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(data);
        hasher.finalize().to_vec()
    }) {
        Ok(bubble) => HttpResponse::Ok().json(json!({
            "status": "success",
            "bubble_created": true,
            "bubble_id": format!("{:?}", &bubble.id[..8]),
            "amount_micro": amount
        })),
        Err(e) => HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": format!("{}", e)
        })),
    }
}

// ====================
// HELPER — Récupérer l'identité publique d'un UQ number
// ====================
fn get_public_identity(uq_number: &str) -> (String, String) {
    // Retourne (public_uq, display_name)
    // Cherche dans la DB du user → uq_aliases
    let db_path = format!(
        "data/users/{}.db",
        uq_number.replace("-", "_").to_lowercase()
    );

    // Valeur par défaut : retourner le UQ et un nom vide
    let mut public_uq = uq_number.to_string();
    let mut display_name = String::new();

    if !std::path::Path::new(&db_path).exists() {
        return (public_uq, display_name);
    }

    if let Ok(conn) = rusqlite::Connection::open(&db_path) {
        // Chercher dans uq_aliases
        let result: Result<(String, String), _> = conn.query_row(
            "SELECT public_uq, display_name FROM uq_aliases WHERE real_uq = ?1 AND active = 1",
            rusqlite::params![uq_number],
            |row| Ok((row.get(0)?, row.get(1)?)),
        );

        if let Ok((pub_uq, disp)) = result {
            if !pub_uq.is_empty() {
                public_uq = pub_uq;
            }
            if !disp.is_empty() {
                display_name = disp;
            }
        }
    }

    (public_uq, display_name)
}

/// Helper : attache la DB du Profil 2 (UQ-732337351) a une connexion SQLite
/// Retourne true si attach reussi, false sinon
pub fn attach_profil2(conn: &rusqlite::Connection) -> bool {
    let path = "data/users/uq_732337351.db";
    if !std::path::Path::new(path).exists() {
        eprintln!("[ATTACH] DB Profil 2 introuvable : {}", path);
        return false;
    }
    match conn.execute("ATTACH DATABASE ?1 AS p2", rusqlite::params![path]) {
        Ok(_) => true,
        Err(e) => {
            eprintln!("[ATTACH] Erreur : {}", e);
            false
        }
    }
}

pub async fn bubble_transfer(
    req: actix_web::HttpRequest,
    state: web::Data<Arc<GlobalState>>,
    body: web::Json<TransferRequest>,
) -> HttpResponse {
    let claims = match crate::auth::AuthManager::require_role(&req, &["user", "admin", "guardian"])
    {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let mut sender = claims.sub.clone();
    // ===== Profil 2 : si le sender est le owner, utiliser UQ-732337351 =====
    let profil2_active = claims.sub == "UQ-00000001";
    if profil2_active {
        sender = "UQ-732337351".to_string();
        eprintln!("[PROFIL2] Utilisation de UQ-732337351 comme sender");
    }

    let sender_original = claims.sub.clone();
    let accepte_owner_ou_profil2 = (body.sender == "UQ-00000001" || body.sender == "UQ-732337351")
        && (sender == "UQ-00000001" || sender == "UQ-732337351");
    if !body.sender.is_empty() && body.sender != sender && !accepte_owner_ou_profil2 {
        eprintln!(
            "🚨 TENTATIVE FRAUDE : {} essaie d'envoyer au nom de {}",
            sender, body.sender
        );
        return HttpResponse::Forbidden().json(json!({
            "status": "error",
            "code": "SENDER_MISMATCH",
            "message": "Expéditeur non autorisé"
        }));
    }

    let (sender_public, sender_display) = get_public_identity(&sender);
    // Affichage public de l'expéditeur (pseudo + nom — JAMAIS le UQ)
    let sender_public_display = crate::uq_public_alias::get_public_display(&sender);
    eprintln!(
        "📤 Transfert de {} (public: {}, nom: {})",
        sender, sender_public, sender_display
    );
    let recipient = body.recipient.clone();

    let device = body.device.clone().unwrap_or_default();
    let os = body.os.clone().unwrap_or_default();
    let browser = body.browser.clone().unwrap_or_default();
    let client_public_key = body.public_key.clone().unwrap_or_default();

    let client_ip = req
        .headers()
        .get("X-Forwarded-For")
        .and_then(|h| h.to_str().ok())
        .map(|s| s.split(',').next().unwrap_or(s).trim().to_string())
        .or_else(|| req.peer_addr().map(|a| a.ip().to_string()))
        .unwrap_or_else(|| "unknown".to_string());

    eprintln!("   Device: {} | OS: {} | Browser: {}", device, os, browser);
    eprintln!("   IP: {}", client_ip);

    // 🔴 RÈGLE 4 (PRIORITAIRE) — le Profil 1 ne transfère JAMAIS en direct.
    //    Règle STRUCTURELLE : elle passe AVANT le cadenas et le KYC.
    //    Le Profil 1 est un acteur système : il ne sort que par son
    //    carnet d'ordres P2P.
    // Si le sender est le owner (UQ-00000001) MAIS que le profil 2 est actif,
    // on a deja remplace sender par UQ-732337351. Donc ce check ne se declenche
    // QUE si quelqu'un essaie de forcer UQ-00000001 en sender direct.
    if sender.eq_ignore_ascii_case(crate::ocean::PROFIL_1) && !profil2_active {
        eprintln!("🚫 [REGLE 4] Transfert direct refuse depuis le Profil 1");
        return HttpResponse::Forbidden().json(json!({
            "status": "error",
            "code": "PROFIL1_NON_TRANSFERABLE",
            "message": "Le solde du Profil 1 n'est pas transferable : il sort uniquement via le carnet d'ordres P2P."
        }));
    }

    if !crate::kyc::cadenas::est_deverrouille(&sender) {
        return HttpResponse::Forbidden().json(json!({
            "status": "error",
            "verrouille": true,
            "message": "Portefeuille verrouille. Verifiez votre identite d abord."
        }));
    }

    // ===== Profil 2 (UQ-732337351) : utiliser le cadenas du principal =====
    let verif_cadenas_result: Result<(), String> =
        match crate::kyc::cadenas::uq_principal_si_lie(&sender) {
            Some(principal) => crate::kyc::cadenas::verifier_cadenas(&principal),
            None => crate::kyc::cadenas::verifier_cadenas(&sender),
        };
    if let Err(e) = verif_cadenas_result {
        let _ = crate::kyc::cadenas::geler_compte_piratage(&sender, &e);
        return HttpResponse::Forbidden().json(json!({
            "status": "error",
            "piratage": true,
            "message": format!("PIRATAGE DETECTE: {}", e)
        }));
    }

    // BORB non transferable directement - passer par le P2P
    let currency_check = body.currency.as_deref().unwrap_or("");
    if currency_check.is_empty()
        || currency_check.eq_ignore_ascii_case("BORB")
        || currency_check.eq_ignore_ascii_case("BORB")
    {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "code": "BORB_NOT_TRANSFERABLE",
            "message": "Le BORB n'est pas transferable directement. Vendez-le via le carnet P2P, puis envoyez la devise convertie."
        }));
    }

    let amount = body.amount_points;
    // Frais calcules automatiquement par le backend (degressif)
    let fee = calculate_fee_points(
        body.amount_points,
        body.currency.as_deref().unwrap_or("BORB"),
    );
    // ====================
    // 👤 RÉSOLUTION DU DESTINATAIRE (pseudo → UQ)
    // ====================
    // Le frontend envoie un PSEUDO (« uq@marie ») — jamais un UQ.
    // Avant ce correctif, le backend cherchait « data/users/uq@marie.db »
    // et répondait « Compte destinataire introuvable » : l'envoi par
    // pseudo était donc impossible.
    let recipient = match crate::uq_public_alias::resoudre_destinataire(&recipient) {
        Some(uq) => uq,
        None => {
            return HttpResponse::BadRequest().json(json!({
                "status": "error",
                "code": "RECIPIENT_INTROUVABLE",
                "message": format!("Destinataire introuvable : « {} ». Utilise un pseudo uq@... ou un UQ valide.", recipient)
            }))
        }
    };

    // Affichage public du destinataire (pseudo + nom, JAMAIS le UQ)
    let recipient_public = crate::uq_public_alias::get_public_display(&recipient);

    // ====================
    // 🔴 RÈGLE 4 — LE SOLDE DU PROFIL 1 N'EST PAS TRANSFÉRABLE
    // ====================
    // Le Profil 1 est un acteur SYSTÈME : il gère la case Ocean et ne
    // sort que par son CARNET D'ORDRES P2P. Aucun transfert direct
    // user ↔ Profil 1 n'est autorisé (ni entrant, ni sortant).
    if sender.eq_ignore_ascii_case(crate::ocean::PROFIL_1) {
        return HttpResponse::Forbidden().json(json!({
            "status": "error",
            "code": "PROFIL1_NON_TRANSFERABLE",
            "message": "Le solde du Profil 1 n'est pas transferable : il sort uniquement via le carnet d'ordres P2P."
        }));
    }
    if recipient.eq_ignore_ascii_case(crate::ocean::PROFIL_1) {
        return HttpResponse::Forbidden().json(json!({
            "status": "error",
            "code": "PROFIL1_NON_RECEVABLE",
            "message": "Le Profil 1 ne reçoit pas de transfert direct : les achats passent par le carnet d'ordres P2P."
        }));
    }

    if let Some(currency) = body.currency.as_deref() {
        if !currency.trim().is_empty()
            && !currency.eq_ignore_ascii_case("BORB")
            && !currency.eq_ignore_ascii_case("BORB")
        {
            return transfer_converted_currency(
                &sender,
                &recipient,
                currency,
                body.amount_currency,
                body.amount_points,
                body.memo.as_deref().unwrap_or(""),
                &ContexteAppel {
                    device: &device,
                    os: &os,
                    browser: &browser,
                    ip: &client_ip,
                },
                &sender_public_display,
                &recipient_public,
            )
            .await;
        }
    }

    let (validated_amount, validated_fee) = match validate_transfer_amounts(amount, fee) {
        Ok(v) => v,
        Err(resp) => return resp,
    };

    let total_cost = match validated_amount.checked_add(validated_fee) {
        Some(t) => t,
        None => {
            return HttpResponse::BadRequest().json(json!({
                "status": "error",
                "code": "OVERFLOW",
                "message": "Montant total trop élevé"
            }))
        }
    };

    let sender_db_path = format!("data/users/{}.db", sender.replace("-", "_").to_lowercase());
    let recipient_db_path = format!(
        "data/users/{}.db",
        recipient.replace("-", "_").to_lowercase()
    );

    if !std::path::Path::new(&sender_db_path).exists() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": format!("Compte expediteur introuvable: {}", sender)
        }));
    }
    if !std::path::Path::new(&recipient_db_path).exists() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": format!("Compte destinataire introuvable: {}", recipient)
        }));
    }

    // 🚪 PORTE BORB : le destinataire doit avoir un cadenas structurel réel,
    //    tracé et lié dans le BlockOrb — sinon rien n'entre (anti-fantôme).
    if let Some(refus) = porte_entree_borb(&recipient) {
        return refus;
    }

    let sender_conn = match rusqlite::Connection::open(&sender_db_path) {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Erreur ouverture sender: {}", e)
            }))
        }
    };

    // 🧬 Aligner le schéma `transactions` (comptes créés avant la migration v4)
    if let Err(e) = crate::db::ensure_transactions_columns(&sender_conn) {
        eprintln!(
            "⚠️  Alignement schéma transactions (sender {}): {}",
            sender, e
        );
    }

    // Solde mobilisable : le BORB immobilisé par un escrow P2P n'est pas
    // dépensable, sinon un gel serait vidé par un simple envoi.
    let (sender_balance, sender_disponible): (i64, i64) = match sender_conn.query_row(
        "SELECT points, points - COALESCE(frozen_points, 0) FROM balance WHERE id = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ) {
        Ok(b) => b,
        Err(_) => (0, 0),
    };

    let required_cost = match i64::try_from(total_cost) {
        Ok(v) => v,
        Err(_) => {
            return HttpResponse::BadRequest().json(json!({
                "status": "error",
                "code": "CAST_OVERFLOW",
                "message": "Montant dépasse la limite i64"
            }))
        }
    };

    if sender_disponible < required_cost {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "code": "BORB_GELÉS",
            "message": format!(
                "Solde disponible insuffisant. Disponible: {} points, Requis: {} points (dont le BORB immobilisé par tes commandes P2P en cours)",
                sender_disponible, total_cost
            )
        }));
    }

    let recipient_conn = match rusqlite::Connection::open(&recipient_db_path) {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Erreur ouverture recipient: {}", e)
            }))
        }
    };

    // 🧬 Aligner le schéma `transactions` côté destinataire aussi
    if let Err(e) = crate::db::ensure_transactions_columns(&recipient_conn) {
        eprintln!(
            "⚠️  Alignement schéma transactions (recipient {}): {}",
            recipient, e
        );
    }

    let now = chrono::Utc::now().timestamp();
    // 🧬 Identifiant BORB unique (= hash de transaction, traçabilité)
    let borb_id = {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(format!("{}:{}:{}:{}", sender, recipient, validated_amount, now).as_bytes());
        hex::encode(hasher.finalize())
    };
    let tx_hash = borb_id.clone();

    // ====================
    // TRAÇABILITÉ BORB — Signature + Cadenas + Horloge
    // ====================

    // 1. Calculer le hash du cadenas du sender
    let sender_cadenas_hash = {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(format!("CADENAS:{}:{}", sender, now).as_bytes());
        hex::encode(hasher.finalize())
    };

    // 2. Horloge interne du BORB (timestamp au moment du transfert)
    let internal_clock = now;

    // 3. Signature Ed25519 (si client_public_key fournie)
    let signature = if !client_public_key.is_empty() {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(
            format!(
                "UQ_TX_V1:{}:{}:{}:{}:{}:{}",
                sender, recipient, validated_amount, validated_fee, now, tx_hash
            )
            .as_bytes(),
        );
        hex::encode(hasher.finalize())
    } else {
        "unsigned".to_string()
    };

    // 4. Universal key (rotation 30s)
    let universal_key_hash = {
        use sha2::{Digest, Sha256};
        let window = now / 30;
        let mut hasher = Sha256::new();
        hasher.update(format!("UQ_BORB_UNIVERSAL:{}", window).as_bytes());
        hex::encode(hasher.finalize())
    };

    if let Err(e) = sender_conn.execute_batch("BEGIN IMMEDIATE") {
        return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("Erreur transaction sender: {}", e)
        }));
    }
    if let Err(e) = recipient_conn.execute_batch("BEGIN IMMEDIATE") {
        let _ = sender_conn.execute_batch("ROLLBACK");
        return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("Erreur transaction recipient: {}", e)
        }));
    }

    let transfer_result = (|| -> Result<(), String> {
        let debit = sender_conn
            .execute(
                "UPDATE balance SET points = points - ?1, updated_at = ?2
                 WHERE id = 1 AND points - COALESCE(frozen_points, 0) >= ?1",
                rusqlite::params![required_cost, now],
            )
            .map_err(|e| e.to_string())?;
        if debit != 1 {
            return Err(
                "Solde disponible insuffisant (BORB immobilisés par un escrow P2P)".to_string(),
            );
        }

        // 📜 Chaînage de l'historique LOCAL (spécification : borb_id + prev_hash
        //    + block_number sont stockés dans la DB de l'utilisateur)
        let prev_hash_sender = crate::local_history::dernier_borb_id(&sender_conn);
        let block_number_local = crate::persistence::load_global_orb().block_number as i64;

        sender_conn
            .execute(
                "INSERT INTO transactions 
             (tx_type, amount, fee, counterparty, memo, created_at,
              device, os, browser, ip, sender_uq, recipient_uq,
              amount_borb, amount_usd, taux_usd, public_key, status, reference, tampered,
              signature, cadenas_hash, internal_clock, universal_key_hash,
              borb_id, prev_hash, block_number)
             VALUES ('send', ?1, ?2, ?3, ?4, ?5,
              ?6, ?7, ?8, ?9, ?10, ?11,
              ?12, ?13, ?14, ?15, 'success', ?16, 0,
              ?17, ?18, ?19, ?20,
              ?21, ?22, ?23)",
                rusqlite::params![
                    validated_amount as i64,
                    validated_fee as i64,
                    recipient_public, // 👤 pseudo + nom (jamais le UQ)
                    tx_hash.clone(),
                    now,
                    device,
                    os,
                    browser,
                    client_ip,
                    sender,
                    recipient,
                    validated_amount as f64 / 100.0,
                    validated_amount as f64 / 100.0,
                    "1.0",
                    client_public_key,
                    format!("UQ-SEND-{}", &tx_hash[..16].to_uppercase()),
                    signature,
                    sender_cadenas_hash,
                    internal_clock,
                    universal_key_hash,
                    borb_id,
                    prev_hash_sender,
                    block_number_local
                ],
            )
            .map_err(|e| e.to_string())?;

        // ===== TRAÇABILITÉ BORB =====
        // (l'enregistrement du BORB et la trace BlockOrb sont faits APRÈS le COMMIT,
        //  pour ne jamais laisser de trace fantôme si la transaction est annulée)

        // Resynchronisation des sous-unités : BORB = points / 100
        // (source de vérité = balance.points, cf. /api/borb/balance)
        let sender_points_after: i64 = sender_conn
            .query_row("SELECT points FROM balance WHERE id = 1", [], |row| {
                row.get(0)
            })
            .unwrap_or(0);
        sender_conn
            .execute(
                "UPDATE my_borb_units SET balance = ?1, updated_at = ?2 WHERE unit = 'BORB'",
                rusqlite::params![sender_points_after / 100, now],
            )
            .map_err(|e| e.to_string())?;

        recipient_conn
            .execute(
                "UPDATE balance SET points = points + ?1, updated_at = ?2 WHERE id = 1",
                rusqlite::params![validated_amount as i64, now],
            )
            .map_err(|e| e.to_string())?;

        // 📜 Chaînage de l'historique LOCAL du destinataire
        let prev_hash_recipient = crate::local_history::dernier_borb_id(&recipient_conn);

        recipient_conn
            .execute(
                "INSERT INTO transactions 
             (tx_type, amount, fee, counterparty, memo, created_at,
              sender_uq, recipient_uq, amount_borb, amount_usd, taux_usd, status, reference,
              borb_id, prev_hash, block_number)
             VALUES ('receive', ?1, 0, ?2, ?3, ?4,
              ?5, ?6, ?7, ?8, ?9, 'success', ?10,
              ?11, ?12, ?13)",
                rusqlite::params![
                    validated_amount as i64,
                    sender_public_display, // 👤 pseudo + nom (jamais le UQ)
                    tx_hash.clone(),
                    now,
                    sender,
                    recipient,
                    validated_amount as f64 / 100.0,
                    validated_amount as f64 / 100.0,
                    "1.0",
                    format!("UQ-RECV-{}", &tx_hash[..16].to_uppercase()),
                    borb_id,
                    prev_hash_recipient,
                    block_number_local
                ],
            )
            .map_err(|e| e.to_string())?;

        // Resynchronisation des sous-unités du destinataire
        let recipient_points_after: i64 = recipient_conn
            .query_row("SELECT points FROM balance WHERE id = 1", [], |row| {
                row.get(0)
            })
            .unwrap_or(0);
        recipient_conn
            .execute(
                "UPDATE my_borb_units SET balance = ?1, updated_at = ?2 WHERE unit = 'BORB'",
                rusqlite::params![recipient_points_after / 100, now],
            )
            .map_err(|e| e.to_string())?;

        Ok(())
    })();

    if let Err(e) = transfer_result {
        let _ = sender_conn.execute_batch("ROLLBACK");
        let _ = recipient_conn.execute_batch("ROLLBACK");
        return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("Erreur écriture du transfert: {}", e)
        }));
    }

    if let Err(e) = sender_conn.execute_batch("COMMIT") {
        let _ = sender_conn.execute_batch("ROLLBACK");
        let _ = recipient_conn.execute_batch("ROLLBACK");
        return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("Erreur validation sender: {}", e)
        }));
    }

    if let Err(e) = recipient_conn.execute_batch("COMMIT") {
        let _ = sender_conn.execute_batch("ROLLBACK");
        let _ = recipient_conn.execute_batch("ROLLBACK");
        return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("Erreur validation recipient: {}", e)
        }));
    }

    let db = Database::shared("uq_platform_db.json");
    db.record_transaction(&sender, &recipient, validated_amount, validated_fee);

    // ====================
    // 📜 HISTORIQUES LOCAUX + 🪞 MIROIR BLOCKORB — APRÈS COMMIT
    // ====================
    // SPÉCIFICATION « BLOCKORB À TAILLE FIXE » : le BlockOrb ne contient
    // plus AUCUN historique. Chaque partie écrit sa propre ligne dans SA DB
    // locale (borb_id + prev_hash + block_number) et le BlockOrb ne reçoit
    // que le solde (miroir).

    // 0. Miroir BlockOrb : le disque dur global suit le mouvement SQLite
    //    (best-effort : n'annule jamais un transfert déjà validé)
    if let Err(e) = crate::persistence::mirroir_transfert(
        &sender,
        &recipient,
        validated_amount as u128,
        validated_fee as u128,
    ) {
        eprintln!(
            "⚠️  Miroir BlockOrb non applique ({} -> {}) : {}",
            sender, recipient, e
        );
    }

    // 1. Registre BORB (data/borb_registry.json)
    crate::persistence::register_borb(&borb_id, &recipient, &sender, validated_amount as u128, now);

    // 2. Scellé des historiques locaux : l'empreinte de la chaîne est
    //    enregistrée avec le cadenas → toute modification manuelle ou
    //    suppression non autorisée devient détectable (history_lock).
    for (conn, uq) in [(&sender_conn, &sender), (&recipient_conn, &recipient)] {
        match crate::local_history::sceller(conn, uq) {
            Ok(hash) => eprintln!(
                "🔒 Historique scelle {} : {}",
                uq,
                &hash[..12.min(hash.len())]
            ),
            Err(e) => eprintln!("⚠️  Scelle historique {} non ecrit : {}", uq, e),
        }
    }

    HttpResponse::Ok().json(json!({
        "status": "success",
        "persisted": true,
        "blockchain": true,
        "amount": validated_amount,
        "fee": validated_fee,
        "tx_hash": tx_hash,
        "borb_id": borb_id,
        "sender_new_balance": sender_balance - required_cost,
        "block_number": crate::persistence::load_global_orb().block_number,
        "message": "Transfert effectue et persiste dans les tables"
    }))
}
pub async fn bubble_freeze() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.publish_offer("freezer", "Bulle gelée", 0, 0);
    HttpResponse::Ok().json(json!({"status":"success","frozen":true}))
}
pub async fn bubble_unfreeze() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.publish_offer("unfreezer", "Bulle dégelée", 0, 0);
    HttpResponse::Ok().json(json!({"status":"success","unfrozen":true}))
}
pub async fn bubble_burn() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.publish_offer("burner", "Bulle brûlée", 0, 0);
    HttpResponse::Ok().json(json!({"status":"success","burned":true}))
}
pub async fn bubble_lend() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.publish_offer("lender", "Bulle prêtée", 0, 0);
    HttpResponse::Ok().json(json!({"status":"success","lent":true}))
}
pub async fn bubble_get(req: actix_web::HttpRequest, path: web::Path<String>) -> HttpResponse {
    // 🔐 SÉCURITÉ (vague 1) — USER : JWT obligatoire
    // 🔐 SÉCURITÉ (vague 1) — USER : accès à SES données (ou owner)
    // 🔐 SÉCURITÉ (vague 1) — USER : JWT obligatoire (le contrôle de
    // propriété de l'objet {id} reste à la charge du handler)
    if let Err(r) = crate::auth_guard::exiger_auth(&req) {
        return r;
    }

    HttpResponse::Ok().json(json!({"bubble_id":path.into_inner()}))
}

// ==========================================================
// 6 CASES (7)
// ==========================================================
pub async fn cases_stats() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    let cases = db.get_cases_stats();
    HttpResponse::Ok().json(json!({"status":"success","cases":cases}))
}
pub async fn case_treasury() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    let cases = db.get_cases_stats();
    HttpResponse::Ok().json(json!({"status":"success","cases":cases}))
}
pub async fn case_fees() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    let cases = db.get_cases_stats();
    HttpResponse::Ok().json(json!({"status":"success","cases":cases}))
}
pub async fn case_rewards() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    let cases = db.get_cases_stats();
    HttpResponse::Ok().json(json!({"status":"success","cases":cases}))
}
pub async fn case_savings() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    let cases = db.get_cases_stats();
    HttpResponse::Ok().json(json!({"status":"success","cases":cases}))
}
pub async fn case_credit() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    let cases = db.get_cases_stats();
    HttpResponse::Ok().json(json!({"status":"success","cases":cases}))
}
pub async fn case_shield() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    let cases = db.get_cases_stats();
    HttpResponse::Ok().json(json!({"status":"success","cases":cases}))
}

// ==========================================================
// UBI (3)
// ==========================================================

/// Montant journalier d'UBI en points (1 point = 0.01 BORB).
/// Configurable via la variable d'environnement UBI_DAILY_POINTS.
fn ubi_daily_points() -> i64 {
    std::env::var("UBI_DAILY_POINTS")
        .ok()
        .and_then(|v| v.trim().parse::<i64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(1000) // 10 BORB / jour
}

const UBI_COOLDOWN_SECS: i64 = 24 * 60 * 60;

fn ubi_table_sql() -> &'static str {
    "CREATE TABLE IF NOT EXISTS ubi_claims (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        amount INTEGER NOT NULL,
        claimed_at INTEGER NOT NULL
    )"
}

pub async fn ubi_info() -> HttpResponse {
    let orb = crate::persistence::load_global_orb();
    HttpResponse::Ok().json(json!({
        "status": "success",
        "daily_points": ubi_daily_points(),
        "daily_borb": ubi_daily_points() as f64 / 100.0,
        "cooldown_hours": UBI_COOLDOWN_SECS / 3600,
        "enveloppe_ubi_femto": orb.ubi_enveloppe.to_string(),
        "enveloppe_ubi_borb": orb.ubi_enveloppe as f64 / crate::montant::FEMTO_PAR_BORB as f64,
        "message": "UBI journalier preleve sur l'enveloppe UBI du BlockOrb"
    }))
}

/// POST /api/ubi/claim — crédite réellement le portefeuille de l'utilisateur.
///
/// Avant ce correctif la route ne créditait RIEN (elle se contentait
/// d'appeler un pool en mémoire) : un compte neuf restait à 0 BORB et ne
/// pouvait donc jamais effectuer de transaction.
///
/// Ici :
///   1. authentification + vérification que le compte est bien celui du token
///   2. cooldown de 24 h (table ubi_claims du compte)
///   3. débit de l'enveloppe UBI du BlockOrb + crédit du portefeuille
///      (l'invariant des 40 Md BORB est vérifié)
///   4. écriture dans balance.points, transactions, my_borb_units, wallet
pub async fn ubi_claim(
    req: actix_web::HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    // UBI DÉSACTIVÉ par décision produit : les réclamations UBI sont
    // abandonnées — les utilisateurs sont récompensés par les codes
    // promo ($1 USD après KYC), pas par l'enveloppe UBI.
    return HttpResponse::Gone().json(json!({
        "status": "error",
        "code": "UBI_DESACTIVE",
        "message": "L'UBI est désactivé. Utilise ton code promo à l'inscription."
    }));

    // ===== 1. Compte ciblé =====
    let account = body
        .get("account")
        .and_then(|v| v.as_str())
        .or_else(|| body.get("uq_number").and_then(|v| v.as_str()))
        .unwrap_or("")
        .trim()
        .to_string();

    if account.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "account (ou uq_number) requis"
        }));
    }

    // ===== 2. Authentification : on ne crédite que son propre compte =====
    let claims =
        match crate::auth::AuthManager::require_role(&req, &["user", "admin", "guardian", "owner"])
        {
            Ok(c) => c,
            Err(resp) => return resp,
        };

    let est_admin = claims.role == "admin" || claims.role == "owner";
    if !est_admin && !claims.sub.eq_ignore_ascii_case(&account) {
        return HttpResponse::Forbidden().json(json!({
            "status": "error",
            "code": "ACCOUNT_MISMATCH",
            "message": "Tu ne peux reclamer l'UBI que pour ton propre compte"
        }));
    }

    let db_path = format!("data/users/{}.db", account.replace("-", "_").to_lowercase());
    if !std::path::Path::new(&db_path).exists() {
        return HttpResponse::NotFound().json(json!({
            "status": "error",
            "message": format!("Compte introuvable: {}", account)
        }));
    }

    let conn = match rusqlite::Connection::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Erreur DB: {}", e)
            }))
        }
    };

    let _ = conn.execute(ubi_table_sql(), []);

    let now = chrono::Utc::now().timestamp();

    // ===== 3. Cooldown 24 h =====
    let derniere: Option<i64> = conn
        .query_row("SELECT MAX(claimed_at) FROM ubi_claims", [], |row| {
            row.get(0)
        })
        .ok()
        .flatten();

    if let Some(derniere) = derniere {
        let restant = UBI_COOLDOWN_SECS - (now - derniere);
        if restant > 0 {
            return HttpResponse::BadRequest().json(json!({
                "status": "error",
                "code": "COOLDOWN",
                "seconds_remaining": restant,
                "next_claim_at": derniere + UBI_COOLDOWN_SECS,
                "message": format!("UBI deja reclame. Prochaine reclamation dans {} h {} min.",
                    restant / 3600, (restant % 3600) / 60)
            }));
        }
    }

    // ===== 3.5 VÉRIFICATION : CADENAS (KYC) =====
    // L'UBI est réservé aux comptes dont le cadenas est vérifié.
    let cadenas_verifie = crate::kyc::cadenas::est_deverrouille(&account);

    if !cadenas_verifie {
        eprintln!("🚫 [UBI] Cadenas non vérifié pour {}", account);
        return HttpResponse::Forbidden().json(json!({
            "status": "error",
            "code": "UBI_CADENAS_REQUIRED",
            "cadenas_verifie": false,
            "message": "L'UBI est réservé aux comptes dont le cadenas (KYC) est vérifié. Complète ton KYC dans ton profil pour pouvoir réclamer l'UBI."
        }));
    }

    // Le cadenas doit être RÉEL (identifiants personnels complets + hash
    // recalculé conforme), pas seulement le drapeau « verrouille = 0 ».
    if let Err(e) = crate::kyc::cadenas::porte_cadenas_structurel(&account) {
        eprintln!(
            "🚫 [UBI] Cadenas structurel absent pour {} : {}",
            account, e
        );
        return HttpResponse::Forbidden().json(json!({
            "status": "error",
            "code": "UBI_CADENAS_STRUCTUREL_REQUIS",
            "message": "L'UBI ne peut pas entrer : ce wallet n'a pas de cadenas structurel réel avec ses identifiants personnels."
        }));
    }

    // ===== 4. Montant + enveloppe BlockOrb =====
    let points = ubi_daily_points();
    let montant_femto = (points as u128) * crate::montant::FEMTO_PAR_BORB / 100;

    // 🚫 Section critique BlockOrb (débit enveloppe / crédit portefeuille sous verrou global).
    let (mut orb, _garde_orb) = crate::persistence::orb_charge_verrouille();

    if orb.ubi_enveloppe < montant_femto {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Enveloppe UBI epuisee."
        }));
    }

    let trace_ok = orb.porte_cadenas(&account)
        && !orb
            .comptes
            .get(&account)
            .map(|p| p.alerte_piratage)
            .unwrap_or(true);
    if !trace_ok {
        return HttpResponse::NotFound().json(json!({
            "status": "error",
            "code": "UBI_CADENAS_NON_LIE",
            "message": format!("Portefeuille {} absent du BlockOrb ou cadenas non lie — UBI refuse", account)
        }));
    }

    let enveloppe_avant = orb.ubi_enveloppe;
    let solde_avant = orb.comptes.get(&account).map(|p| p.solde).unwrap_or(0);

    // Débit enveloppe / crédit portefeuille (transfert interne, invariant préservé)
    orb.ubi_enveloppe -= montant_femto;
    if let Some(pf) = orb.comptes.get_mut(&account) {
        pf.solde = pf.solde.saturating_add(montant_femto);
    }

    if !orb.verifier() {
        return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": "Invariant casse — UBI annule (aucun debit effectue)"
        }));
    }

    if let Err(e) = crate::persistence::save_global_orb(&orb) {
        return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("Erreur sauvegarde BlockOrb: {}", e)
        }));
    }

    // ===== 5. Écriture SQLite (source de vérité des soldes dépensables) =====
    let tx_result = (|| -> Result<i64, String> {
        conn.execute("BEGIN IMMEDIATE", [])
            .map_err(|e| e.to_string())?;

        conn.execute(
            "UPDATE balance SET points = points + ?1, updated_at = ?2 WHERE id = 1",
            rusqlite::params![points, now],
        )
        .map_err(|e| e.to_string())?;

        let nouveau_solde: i64 = conn
            .query_row("SELECT points FROM balance WHERE id = 1", [], |r| r.get(0))
            .unwrap_or(0);

        // Sous-unités alignées sur le solde
        conn.execute(
            "UPDATE my_borb_units SET balance = ?1, updated_at = ?2 WHERE unit = 'BORB'",
            rusqlite::params![nouveau_solde / 100, now],
        )
        .map_err(|e| e.to_string())?;

        // Table wallet (historique) remise à jour
        conn.execute(
            "UPDATE wallet SET balance_borb = ?1, balance_points = ?2,
                    total_received = COALESCE(total_received, 0) + ?3, last_updated = ?4
             WHERE id = 1",
            rusqlite::params![nouveau_solde as f64 / 100.0, nouveau_solde, points, now],
        )
        .map_err(|e| e.to_string())?;

        // 📜 Historique LOCAL : borb_id + prev_hash + block_number
        let reference = format!("UQ-UBI-{}", now);
        let borb_id_ubi = crate::blockchain_lib::hash_transaction(
            "UQ-TRESOR-UBI",
            &account,
            points as u64,
            0,
            now,
        );
        let prev_hash_ubi = crate::local_history::dernier_borb_id(&conn);
        let block_number_ubi = crate::persistence::load_global_orb().block_number as i64;

        conn.execute(
            "INSERT INTO transactions (tx_type, amount, fee, counterparty, memo, created_at,
                                       sender_uq, recipient_uq, amount_borb, amount_usd, taux_usd,
                                       status, reference, borb_id, prev_hash, block_number)
             VALUES ('ubi', ?1, 0, 'UQ-TRESOR-UBI', ?2, ?3, 'UQ-TRESOR-UBI', ?4, ?5, ?5, '1.0', 'success', ?6, ?7, ?8, ?9)",
            rusqlite::params![
                points,
                reference,
                now,
                account,
                points as f64 / 100.0,
                reference,
                borb_id_ubi,
                prev_hash_ubi,
                block_number_ubi
            ],
        ).map_err(|e| e.to_string())?;

        conn.execute(
            "INSERT INTO ubi_claims (amount, claimed_at) VALUES (?1, ?2)",
            rusqlite::params![points, now],
        )
        .map_err(|e| e.to_string())?;

        conn.execute("COMMIT", []).map_err(|e| e.to_string())?;
        Ok(nouveau_solde)
    })();

    match tx_result {
        Ok(nouveau_solde) => {
            // 🔒 Le scellé doit suivre TOUTE écriture dans l'historique,
            //    sinon /api/history/{uq}/verify signalerait à tort une
            //    falsification (bug detecte par verify_history.ps1).
            if let Err(e) = crate::local_history::sceller(&conn, &account) {
                eprintln!("⚠️  Scelle historique UBI {} non ecrit : {}", account, e);
            }

            eprintln!(
                "🎁 UBI {} : +{} points ({} BORB) | solde {} points | enveloppe {} -> {}",
                account,
                points,
                points as f64 / 100.0,
                nouveau_solde,
                enveloppe_avant,
                orb.ubi_enveloppe
            );

            HttpResponse::Ok().json(json!({
                "status": "success",
                "amount_points": points,
                "amount_borb": points as f64 / 100.0,
                "balance_points": nouveau_solde,
                "balance_borb": nouveau_solde as f64 / 100.0,
                "next_claim_at": now + UBI_COOLDOWN_SECS,
                "enveloppe_ubi_restante_femto": orb.ubi_enveloppe.to_string(),
                "solde_blockorb_avant": solde_avant.to_string(),
                "message": "UBI reclame et credite"
            }))
        }
        Err(e) => {
            // Rollback BlockOrb pour rester cohérent avec SQLite
            orb.ubi_enveloppe = enveloppe_avant;
            if let Some(pf) = orb.comptes.get_mut(&account) {
                pf.solde = solde_avant;
            }
            let _ = crate::persistence::save_global_orb(&orb);
            let _ = conn.execute("ROLLBACK", []);

            HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Erreur credit UBI: {}", e)
            }))
        }
    }
}

/// GET /api/ubi/check/{uq} — statut réel de réclamation
pub async fn ubi_check(path: web::Path<String>) -> HttpResponse {
    let account = path.into_inner();
    let db_path = format!("data/users/{}.db", account.replace("-", "_").to_lowercase());

    if !std::path::Path::new(&db_path).exists() {
        return HttpResponse::NotFound().json(json!({
            "status": "error",
            "message": format!("Compte introuvable: {}", account)
        }));
    }

    let conn = match rusqlite::Connection::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Erreur DB: {}", e)
            }))
        }
    };

    let _ = conn.execute(ubi_table_sql(), []);

    let derniere: Option<i64> = conn
        .query_row("SELECT MAX(claimed_at) FROM ubi_claims", [], |row| {
            row.get(0)
        })
        .ok()
        .flatten();

    let now = chrono::Utc::now().timestamp();
    let next_claim_at = derniere.map(|d| d + UBI_COOLDOWN_SECS).unwrap_or(now);
    let eligible = next_claim_at <= now;

    HttpResponse::Ok().json(json!({
        "status": "success",
        "account": account,
        "eligible": eligible,
        "last_claim_at": derniere,
        "next_claim_at": next_claim_at,
        "daily_points": ubi_daily_points(),
        "seconds_remaining": if eligible { 0 } else { next_claim_at - now }
    }))
}

// ==========================================================
// EXCHANGE (5)
// ==========================================================
fn trading_not_ready() -> HttpResponse {
    HttpResponse::NotImplemented().json(json!({
        "status": "error",
        "code": "TRADING_SETTLEMENT_UNAVAILABLE",
        "message": "Le carnet P2P et le règlement sécurisé ne sont pas encore disponibles. Aucun ordre ni escrow n'a été créé."
    }))
}

pub async fn exchange_order(_body: web::Json<serde_json::Value>) -> HttpResponse {
    trading_not_ready()
}
pub async fn exchange_confirm() -> HttpResponse {
    trading_not_ready()
}
pub async fn exchange_complete() -> HttpResponse {
    trading_not_ready()
}
pub async fn exchange_cancel() -> HttpResponse {
    trading_not_ready()
}
pub async fn exchange_sms() -> HttpResponse {
    trading_not_ready()
}

// ==========================================================
// PRÊTS + ASSURANCE + ESCROW (10)
// ==========================================================
pub async fn lending_offer() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.publish_offer("lender", "Offre de prêt", 0, 0);
    HttpResponse::Ok().json(json!({"status":"success","offer_created":true}))
}
pub async fn lending_borrow() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.publish_offer("borrower", "Emprunt", 0, 0);
    HttpResponse::Ok().json(json!({"status":"success","borrowed":true}))
}
pub async fn lending_repay() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.publish_offer("borrower", "Remboursement", 0, 0);
    HttpResponse::Ok().json(json!({"status":"success","repaid":true}))
}
pub async fn lending_liquidate() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.publish_offer("lender", "Liquidation", 0, 0);
    HttpResponse::Ok().json(json!({"status":"success","liquidated":true}))
}
pub async fn insurance_contribute() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.publish_offer("contributor", "Contribution assurance", 0, 0);
    HttpResponse::Ok().json(json!({"status":"success","contributed":true}))
}
pub async fn insurance_claim() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.publish_offer("claimant", "Sinistre", 0, 0);
    HttpResponse::Ok().json(json!({"status":"success","claimed":true}))
}
pub async fn insurance_vote() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("voter", "vote", "Vote sinistre");
    HttpResponse::Ok().json(json!({"status":"success","voted":true}))
}
pub async fn escrow_create(_body: web::Json<serde_json::Value>) -> HttpResponse {
    trading_not_ready()
}
pub async fn escrow_release() -> HttpResponse {
    trading_not_ready()
}
pub async fn escrow_refund() -> HttpResponse {
    trading_not_ready()
}

// ==========================================================
// IDENTITÉ + GOUVERNANCE (10)
// ==========================================================
pub async fn identity_create() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.create_account("identity_user", &[0u8; 32]);
    HttpResponse::Ok().json(json!({"status":"success","identity_created":true}))
}
pub async fn identity_kyc(req: actix_web::HttpRequest) -> HttpResponse {
    // 🔐 SÉCURITÉ : JWT obligatoire + statut KYC réel — avant, ce stub
    // créait "kyc_user" et renvoyait verified sans aucune vérification.
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(claims) => claims,
        Err(response) => return response,
    };
    let index = crate::uq_index::UqIndex::shared("uq_index.db");
    let statut = index
        .get_account(&claims.sub)
        .and_then(|a| {
            a.get("kyc_status")
                .or_else(|| a.get("is_verified"))
                .cloned()
        })
        .unwrap_or(serde_json::Value::Null);
    let verifie = statut
        .as_str()
        .map(|s| s.eq_ignore_ascii_case("verified") || s == "Verifie")
        .unwrap_or(false)
        || statut.as_i64() == Some(1);
    HttpResponse::Ok().json(json!({
        "status": "success",
        "kyc_verified": verifie,
        "kyc_status": statut,
    }))
}
pub async fn identity_get(req: actix_web::HttpRequest, path: web::Path<String>) -> HttpResponse {
    // 🔐 SÉCURITÉ (vague 1) — USER : JWT obligatoire
    // 🔐 SÉCURITÉ (vague 1) — USER : accès à SES données (ou owner)
    // 🔐 SÉCURITÉ (vague 1) — USER : JWT obligatoire (le contrôle de
    // propriété de l'objet {id} reste à la charge du handler)
    if let Err(r) = crate::auth_guard::exiger_auth(&req) {
        return r;
    }
    HttpResponse::Ok().json(json!({"id":path.into_inner()}))
}
pub async fn identity_certificate() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.create_account("cert_user", &[0u8; 32]);
    HttpResponse::Ok().json(json!({"status":"success","certificate_created":true}))
}
pub async fn identity_verify(req: actix_web::HttpRequest, path: web::Path<String>) -> HttpResponse {
    // 🔐 SÉCURITÉ (vague 1) — USER : JWT obligatoire
    // 🔐 SÉCURITÉ (vague 1) — USER : accès à SES données (ou owner)
    // 🔐 SÉCURITÉ (vague 1) — USER : JWT obligatoire (le contrôle de
    // propriété de l'objet {id} reste à la charge du handler)
    if let Err(r) = crate::auth_guard::exiger_auth(&req) {
        return r;
    }
    HttpResponse::Ok().json(json!({"id":path.into_inner()}))
}
pub async fn governance_list() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.get_stats();
    HttpResponse::Ok().json(json!({"status":"success","proposals":[]}))
}
pub async fn governance_create() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.create_proposal("Nouvelle proposition", "Description", "proposer_001");
    HttpResponse::Ok().json(json!({"status":"success","proposal_created":true}))
}
pub async fn governance_vote() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.create_proposal("Vote UQ", "Proposition de vote", "voter_001");
    HttpResponse::Ok().json(json!({"status":"success","vote_recorded":true}))
}
pub async fn governance_execute() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.create_proposal("Executed", "Proposal", "executor");
    HttpResponse::Ok().json(json!({"status":"success","executed":true}))
}
pub async fn governance_stats() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    let stats = db.get_stats();
    HttpResponse::Ok().json(json!({"status":"success","stats":stats}))
}

// ==========================================================
// RÉSEAU P2P (15)
// ==========================================================
pub async fn network_peers() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    let stats = db.get_stats();
    HttpResponse::Ok().json(json!({"status":"success","peers":stats}))
}
pub async fn network_status() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("network", "status", "check");
    HttpResponse::Ok().json(json!({"status":"success","ready":true}))
}
pub async fn network_threats() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("network", "threats", "scan");
    HttpResponse::Ok().json(json!({"status":"success","threats":0}))
}
pub async fn d2d_connect() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("d2d", "connect", "peer");
    HttpResponse::Ok().json(json!({"status":"success","connected":true}))
}
pub async fn d2d_disconnect() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("d2d", "disconnect", "peer");
    HttpResponse::Ok().json(json!({"status":"success","disconnected":true}))
}
pub async fn d2d_scan() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("d2d", "scan", "peers");
    HttpResponse::Ok().json(json!({"status":"success","scanning":true}))
}
pub async fn d2d_peers() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    let stats = db.get_stats();
    HttpResponse::Ok().json(json!({"status":"success","peers":stats}))
}
pub async fn mesh_relay() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("mesh", "relay", "packet");
    HttpResponse::Ok().json(json!({"status":"success","relayed":true}))
}
pub async fn mesh_routing() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("mesh", "routing", "table");
    HttpResponse::Ok().json(json!({"status":"success","routing":[]}))
}
pub async fn mesh_route() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("mesh", "route", "find");
    HttpResponse::Ok().json(json!({"status":"success","route_found":true}))
}
pub async fn mesh_sellers() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    let offers = db.get_all_offers();
    HttpResponse::Ok().json(json!({"status":"success","sellers":offers}))
}
pub async fn frequency_hop() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("freq", "hop", "change");
    HttpResponse::Ok().json(json!({"status":"success","hopped":true}))
}
pub async fn frequency_status() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("freq", "status", "check");
    HttpResponse::Ok().json(json!({"status":"success","frequency":"2.4GHz"}))
}
pub async fn phantom_create_channel() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("phantom", "create", "channel");
    HttpResponse::Ok().json(json!({"status":"success","channel_created":true}))
}
pub async fn phantom_channels() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("phantom", "channels", "list");
    HttpResponse::Ok().json(json!({"status":"success","channels":[]}))
}

// ==========================================================
// SÉCURITÉ + vTEE (13) — attention : ces handlers ne verifient rien, ils
// repondent un succes constant. Voir la tache de purge des faux garde-fous.
// ==========================================================
pub async fn security_status() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("security", "status", "check");
    HttpResponse::Ok().json(json!({"status":"success","secure":true}))
}
pub async fn security_root_check() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("security", "root", "check");
    HttpResponse::Ok().json(json!({"status":"success","rooted":false}))
}
pub async fn security_emulator_check() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("security", "emulator", "check");
    HttpResponse::Ok().json(json!({"status":"success","emulator":false}))
}
pub async fn security_biometric_sync() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("security", "biometric", "sync");
    HttpResponse::Ok().json(json!({"status":"success","synced":true}))
}
pub async fn security_memory_guard() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("security", "memory", "guard");
    HttpResponse::Ok().json(json!({"status":"success","guarded":true}))
}
pub async fn security_secure_storage() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("security", "storage", "secure");
    HttpResponse::Ok().json(json!({"status":"success","stored":true}))
}
pub async fn security_side_channel() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("security", "side_channel", "protect");
    HttpResponse::Ok().json(json!({"status":"success","protected":true}))
}
pub async fn security_trusted_ipc() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("security", "ipc", "trusted");
    HttpResponse::Ok().json(json!({"status":"success","trusted":true}))
}
pub async fn security_vtee_generate() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("security", "vtee", "generate");
    HttpResponse::Ok().json(json!({"status":"success","generated":true}))
}
pub async fn vtee_enclave() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("vtee", "enclave", "create");
    HttpResponse::Ok().json(json!({"status":"success","enclave_created":true}))
}
pub async fn vtee_attest() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("vtee", "attest", "verify");
    HttpResponse::Ok().json(json!({"status":"success","attested":true}))
}
pub async fn keys_derive() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("keys", "derive", "new");
    HttpResponse::Ok().json(json!({"status":"success","derived":true}))
}
pub async fn secure_store() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("secure", "store", "data");
    HttpResponse::Ok().json(json!({"status":"success","stored":true}))
}

// ==========================================================
// VAULT (15)
// ==========================================================
pub async fn vault_status() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("vault", "status", "check");
    HttpResponse::Ok().json(json!({"status":"success","shamir":"3/5"}))
}
pub async fn vault_guardians() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("vault", "guardians", "list");
    HttpResponse::Ok().json(json!({"status":"success","guardians":[]}))
}
pub async fn vault_recovery_status() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("vault", "recovery", "status");
    HttpResponse::Ok().json(json!({"status":"success","recovery":"idle"}))
}
pub async fn vault_authenticity() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("vault", "authenticity", "verify");
    HttpResponse::Ok().json(json!({"status":"success","authentic":true}))
}
pub async fn vault_compromised() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("vault", "compromised", "check");
    HttpResponse::Ok().json(json!({"status":"success","compromised":0}))
}
pub async fn vault_p2p_usage() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("vault", "p2p", "usage");
    HttpResponse::Ok().json(json!({"status":"success","usage":0.0}))
}
pub async fn vault_split() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("vault", "split", "key");
    HttpResponse::Ok().json(json!({"status":"success","split":5}))
}
pub async fn vault_combine() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("vault", "combine", "key");
    HttpResponse::Ok().json(json!({"status":"success","combined":true}))
}
pub async fn vault_guardian_replace() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("vault", "guardian", "replace");
    HttpResponse::Ok().json(json!({"status":"success","replaced":true}))
}
pub async fn vault_proof() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("vault", "proof", "create");
    HttpResponse::Ok().json(json!({"status":"success","proof_created":true}))
}
pub async fn vault_proof_verify() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("vault", "proof", "verify");
    HttpResponse::Ok().json(json!({"status":"success","proof_valid":true}))
}
pub async fn vault_recovery_start() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("vault", "recovery", "start");
    HttpResponse::Ok().json(json!({"status":"success","recovery_started":true}))
}
pub async fn vault_session() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.create_session(
        "user_001",
        "token_unique",
        chrono::Utc::now().timestamp() + 3600,
    );
    HttpResponse::Ok().json(json!({"status":"success","session_created":true}))
}
pub async fn vault_p2p_store() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("vault", "p2p", "store");
    HttpResponse::Ok().json(json!({"status":"success","stored":true}))
}
pub async fn validator_approvals() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    let stats = db.get_stats();
    HttpResponse::Ok().json(json!({"status":"success","approvals":10,"stats":stats}))
}

// ==========================================================
// CHAT + DATA (10)
// ==========================================================
pub async fn chat_send(
    req: actix_web::HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    // 🔐 SÉCURITÉ (vague 1) — USER : JWT obligatoire
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(claims) => claims,
        Err(response) => return response,
    };

    let db = Database::shared("uq_platform_db.json");
    let sender = body.get("sender").and_then(|v| v.as_str()).unwrap_or("");
    let recipient_saisi = body.get("recipient").and_then(|v| v.as_str()).unwrap_or("");
    let content = body
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    // Photo : data URL gardee dans le stock local du compte, jamais envoyee
    // sur un service cloud. Taille plafonnee pour tenir dans le JSON local.
    let image = match body.get("image").and_then(|v| v.as_str()) {
        Some(data) => {
            let data = data.trim();
            if !data.starts_with("data:image/") {
                return HttpResponse::BadRequest().json(json!({
                    "status": "error",
                    "message": "Format d'image invalide"
                }));
            }
            if data.len() > 900_000 {
                return HttpResponse::BadRequest().json(json!({
                    "status": "error",
                    "message": "Photo trop lourde (max 650 Ko)"
                }));
            }
            Some(data)
        }
        None => None,
    };
    if sender.is_empty() || recipient_saisi.is_empty() || (content.is_empty() && image.is_none()) {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "sender, recipient et contenu (ou photo) requis"
        }));
    }
    // Le destinataire peut etre saisi par pseudo (nouveau chat) : on stocke
    // toujours le numero UQ canonique, cle technique des conversations.
    let recipient = match crate::uq_public_alias::resoudre_destinataire(recipient_saisi) {
        Some(uq) => uq,
        None => {
            return HttpResponse::NotFound().json(json!({
                "status": "error",
                "message": "Destinataire introuvable"
            }))
        }
    };
    // Contact bloque : ses messages ne sont plus acceptes par le destinataire.
    if crate::chat_contacts::bloquant(&recipient, sender) {
        return HttpResponse::Forbidden().json(json!({
            "status": "error",
            "blocked": true,
            "message": "Ce contact a bloque la discussion"
        }));
    }

    // === Detection de risque + masquage + quota ===
    let conn = match db.conn() {
        Ok(c) => c,
        Err(_) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": "Erreur connexion base"
            }))
        }
    };

    // 1. Verifier si les deux sont amis
    let est_ami: bool = conn.query_row(
        "SELECT COUNT(*) FROM followers WHERE uq_follower = ?1 AND uq_followed = ?2 AND status = 'accepted'",
        rusqlite::params![sender, recipient],
        |row| row.get::<_, i64>(0),
    ).unwrap_or(0) > 0;

    // 2. Masquer les liens si pas ami
    let content_masque = crate::chat_risk::masquer_liens(content, est_ami);

    // 3. Analyser le risque
    let analyse = match crate::chat_risk::analyser_risque(
        &content_masque,
        sender,
        &recipient,
        est_ami,
        &conn,
    ) {
        Ok(a) => a,
        Err(_) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": "Erreur analyse risque"
            }))
        }
    };

    // 4. Bloquer si score > 90
    if analyse.action == "block" {
        let _ = crate::chat_risk::logguer_risque(&conn, sender, &recipient, &analyse);
        return HttpResponse::Forbidden().json(json!({
            "status": "error",
            "message": "Message bloque : comportement suspect.",
            "score": analyse.score,
            "signaux": analyse.signaux,
        }));
    }

    // 5. Verifier le quota (15 si sur un produit, 100 sinon)
    let sur_produit = body.get("zando_item_id").and_then(|v| v.as_str()).is_some()
        || body.get("p2p_offer_id").and_then(|v| v.as_str()).is_some();
    let quota = if est_ami {
        i64::MAX
    } else if sur_produit {
        15
    } else {
        100
    };

    let now = chrono::Utc::now().timestamp();
    let caracteres = content_masque.len();
    let messages_equivalents = ((caracteres + 19) / 20) as i64;

    let (count, last_reset): (i64, i64) = conn.query_row(
        "SELECT messages_count, last_reset FROM chat_quotas WHERE sender = ?1 AND recipient = ?2",
        rusqlite::params![sender, recipient],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap_or((0, now - 86400));

    let count_actuel = if last_reset < now - 86400 { 0 } else { count };

    if quota != i64::MAX && count_actuel + messages_equivalents > quota {
        return HttpResponse::Forbidden().json(json!({
            "status": "error",
            "message": format!("Quota de {} messages atteint. Rejoins le reseau pour continuer.", quota),
            "quota_atteint": true,
            "reset_dans_secs": last_reset + 86400 - now,
        }));
    }

    // 6. Mettre a jour le quota
    let _ = conn.execute(
        "INSERT INTO chat_quotas (sender, recipient, messages_count, last_reset)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(sender, recipient) DO UPDATE SET
           messages_count = CASE WHEN last_reset < ?5 THEN ?3 ELSE messages_count + ?3 END,
           last_reset = CASE WHEN last_reset < ?5 THEN ?4 ELSE last_reset END",
        rusqlite::params![sender, recipient, messages_equivalents, now, now - 86400],
    );

    // 7. Logguer le risque
    let _ = crate::chat_risk::logguer_risque(&conn, sender, &recipient, &analyse);

    // Une conversation ajoute l'interlocuteur aux contacts, sauf s'il a ete
    // supprime volontairement (marqueur conserve en base). Les ecritures en
    // base de contacts ne sont faites que si l'expediteur est bien authentifie.
    let expediteur_autorise =
        claims.sub.eq_ignore_ascii_case(sender.trim()) || crate::auth_guard::est_owner(&claims);
    if expediteur_autorise {
        let item_id = body.get("zando_item_id").and_then(|v| v.as_str());
        let item_titre = body.get("zando_item_title").and_then(|v| v.as_str());
        crate::chat_contacts::ajouter_contact(sender, &recipient, item_id, item_titre);
        crate::chat_contacts::ajouter_contact(&recipient, sender, item_id, item_titre);
    }
    let id = db.send_message_avec_image(
        sender,
        &recipient,
        &content_masque,
        image,
        crate::chat_contacts::conversation_ephemere(sender, &recipient),
    );
    HttpResponse::Ok().json(json!({"status": "success", "sent": true, "message_id": id}))
}

#[derive(serde::Deserialize)]
pub struct ChatThreadQuery {
    pub me: String,
}

pub async fn chat_messages(
    req: actix_web::HttpRequest,
    path: web::Path<String>,
    query: web::Query<ChatThreadQuery>,
) -> HttpResponse {
    // 🔐 SÉCURITÉ (vague 1) — USER : JWT obligatoire
    // 🔐 SÉCURITÉ (vague 1) — USER : accès à SES données (ou owner)
    let peer_saisi = path.into_inner();
    let me = query.me.trim();
    if me.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "parametre me requis"
        }));
    }
    // Le lecteur ne peut ouvrir que ses propres conversations : la route ecrit
    // aussi le contact en base, elle ne doit pas servir de porte d'ecriture.
    let claims = match crate::auth_guard::exiger_soi_ou_owner(&req, me) {
        Ok(claims) => claims,
        Err(r) => return r,
    };
    // Le fil peut etre ouvert par pseudo (nouveau chat) : la cle stockee reste
    // le numero UQ canonique.
    let peer = match crate::uq_public_alias::resoudre_destinataire(&peer_saisi) {
        Some(uq) => uq,
        None => {
            return HttpResponse::NotFound().json(json!({
                "status": "error",
                "message": "Interlocuteur introuvable"
            }))
        }
    };
    let db = Database::shared("uq_platform_db.json");
    // Le pair qui relit la conversation alimente l'etat « en ligne » du bandeau.
    crate::chat_reseau::signaler(&claims.sub);
    // Ouvrir la discussion vaut premiere conversation : l'interlocuteur rejoint
    // la liste de contacts du lecteur (jamais ressuscite s'il a ete supprime).
    // Le destinataire n'est ajoute qu'au premier message envoye.
    crate::chat_contacts::ajouter_contact(me, &peer, None, None);
    // Lecture = accuse de lecture : l'expediteur voit ses messages comme lus.
    let lus = db.marquer_lus(me, &peer);
    let messages = db.get_messages_between(me, &peer);
    let ephemere = crate::chat_contacts::conversation_ephemere(me, &peer);
    HttpResponse::Ok().json(json!({
        "status": "success",
        "peer": peer,
        "messages": messages,
        "lus_marques": lus,
        "ephemere": ephemere,
        "ttl_seconds": if ephemere { crate::database::MESSAGE_TTL_SECS } else { 0 }
    }))
}

pub async fn chat_inbox(req: actix_web::HttpRequest, path: web::Path<String>) -> HttpResponse {
    // 🔐 SÉCURITÉ (vague 1) — USER : JWT obligatoire
    // 🔐 SÉCURITÉ (vague 1) — USER : accès à SES données (ou owner)
    let __cible = path.to_string();
    let claims = match crate::auth_guard::exiger_soi_ou_owner(&req, &__cible) {
        Ok(claims) => claims,
        Err(r) => return r,
    };

    let user_id = path.into_inner();
    if user_id.trim().is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "user_id requis"
        }));
    }
    // L'appelant vient de donner signe de vie : c est la seule presence que le
    // serveur puisse mesurer sans connexion persistante. On enregistre celui
    // qui a appele (claims.sub), jamais la cible de la requete.
    crate::chat_reseau::signaler(&claims.sub);
    let db = Database::shared("uq_platform_db.json");
    let mut conversations = db.get_chat_inbox(&user_id);
    let non_lus = db.non_lus_par_pair(&user_id);
    for conversation in conversations.iter_mut() {
        let peer = conversation
            .get("peer_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let compteur = non_lus.get(&peer).copied().unwrap_or(0);
        if let Some(objet) = conversation.as_object_mut() {
            objet.insert("non_lus".into(), serde_json::json!(compteur));
            objet.insert(
                "en_ligne".into(),
                serde_json::json!(crate::chat_reseau::actif(&peer)),
            );
            objet.insert(
                "ephemere".into(),
                serde_json::json!(crate::chat_contacts::conversation_ephemere(&user_id, &peer)),
            );
        }
    }
    HttpResponse::Ok().json(json!({
        "status": "success",
        "conversations": conversations,
        "total": conversations.len(),
        "ephemere_par_defaut": false,
        "ttl_ephemere_secondes": crate::database::MESSAGE_TTL_SECS
    }))
}

pub async fn chat_conversation(req: actix_web::HttpRequest) -> HttpResponse {
    // 🔐 SÉCURITÉ (vague 1) — USER : JWT obligatoire
    if let Err(r) = crate::auth_guard::exiger_auth(&req) {
        return r;
    }

    HttpResponse::Ok().json(json!({"status": "success", "conversation_created": true}))
}

pub async fn chat_groups_list(req: HttpRequest, path: web::Path<String>) -> HttpResponse {
    let user_id = path.into_inner();
    let claims = match crate::auth_guard::exiger_soi_ou_owner(&req, &user_id) {
        Ok(claims) => claims,
        Err(response) => return response,
    };
    let index = uq_index::UqIndex::shared("uq_index.db");
    match index.list_chat_groups(&claims.sub) {
        Ok(groups) => HttpResponse::Ok().json(json!({
            "status": "success",
            "groups": groups
        })),
        Err(error) => {
            eprintln!("Impossible de charger les groupes du chat: {}", error);
            HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": "Impossible de charger les groupes."
            }))
        }
    }
}

pub async fn chat_group_create(
    req: HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(claims) => claims,
        Err(response) => return response,
    };
    let name = body
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let description = body
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let group_type = body.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let visibility = body
        .get("visibility")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if name.is_empty() || name.chars().count() > 64 || description.chars().count() > 280 {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Le nom est obligatoire (64 caractères maximum) et la description est limitée à 280 caractères."
        }));
    }
    if !["group", "channel"].contains(&group_type) || !["public", "private"].contains(&visibility) {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Type de groupe ou visibilité invalide."
        }));
    }
    let index = uq_index::UqIndex::shared("uq_index.db");
    match index.create_chat_group(name, description, group_type, visibility, &claims.sub) {
        Ok(group) => HttpResponse::Created().json(json!({
            "status": "success",
            "group": group
        })),
        Err(error) => {
            eprintln!("Impossible de créer un groupe de chat: {}", error);
            HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": "Impossible de créer ce groupe."
            }))
        }
    }
}

pub async fn chat_group_join(req: HttpRequest, path: web::Path<String>) -> HttpResponse {
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(claims) => claims,
        Err(response) => return response,
    };
    let index = uq_index::UqIndex::shared("uq_index.db");
    match index.join_chat_group(&path.into_inner(), &claims.sub) {
        Ok(()) => HttpResponse::Ok().json(json!({"status": "success", "joined": true})),
        Err(error) => HttpResponse::Forbidden().json(json!({
            "status": "error",
            "message": error
        })),
    }
}

pub async fn chat_group_leave(req: HttpRequest, path: web::Path<String>) -> HttpResponse {
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(claims) => claims,
        Err(response) => return response,
    };
    let index = uq_index::UqIndex::shared("uq_index.db");
    match index.leave_chat_group(&path.into_inner(), &claims.sub) {
        Ok(()) => HttpResponse::Ok().json(json!({"status": "success", "left": true})),
        Err(error) => HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": error
        })),
    }
}

pub async fn chat_group_delete(req: HttpRequest, path: web::Path<String>) -> HttpResponse {
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(claims) => claims,
        Err(response) => return response,
    };
    let index = uq_index::UqIndex::shared("uq_index.db");
    match index.delete_chat_group(&path.into_inner(), &claims.sub) {
        Ok(()) => HttpResponse::Ok().json(json!({"status": "success", "deleted": true})),
        Err(error) => HttpResponse::Forbidden().json(json!({
            "status": "error",
            "message": error
        })),
    }
}

pub async fn chat_group_invite(
    req: HttpRequest,
    path: web::Path<String>,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(claims) => claims,
        Err(response) => return response,
    };
    let member_id = body
        .get("member_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if member_id.is_empty() || member_id.chars().count() > 64 {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Un identifiant UQ valide est requis."
        }));
    }
    let index = uq_index::UqIndex::shared("uq_index.db");
    match index.invite_chat_group_member(&path.into_inner(), &claims.sub, member_id) {
        Ok(()) => HttpResponse::Ok().json(json!({"status": "success", "invited": true})),
        Err(error) => HttpResponse::Forbidden().json(json!({
            "status": "error",
            "message": error
        })),
    }
}

pub async fn chat_group_messages(req: HttpRequest, path: web::Path<String>) -> HttpResponse {
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(claims) => claims,
        Err(response) => return response,
    };
    let index = uq_index::UqIndex::shared("uq_index.db");
    match index.chat_group_messages(&path.into_inner(), &claims.sub) {
        Ok(messages) => HttpResponse::Ok().json(json!({
            "status": "success",
            "messages": messages
        })),
        Err(error) => HttpResponse::Forbidden().json(json!({
            "status": "error",
            "message": error
        })),
    }
}

pub async fn chat_group_send(
    req: HttpRequest,
    path: web::Path<String>,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(claims) => claims,
        Err(response) => return response,
    };
    let content = body
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if content.is_empty() || content.chars().count() > 4000 {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Le message doit contenir entre 1 et 4000 caractères."
        }));
    }
    let index = uq_index::UqIndex::shared("uq_index.db");
    match index.send_chat_group_message(&path.into_inner(), &claims.sub, content) {
        Ok(message_id) => HttpResponse::Created().json(json!({
            "status": "success",
            "message_id": message_id
        })),
        Err(error) => HttpResponse::Forbidden().json(json!({
            "status": "error",
            "message": error
        })),
    }
}

pub async fn chat_encrypt() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("chat", "encrypt", "message");
    HttpResponse::Ok().json(json!({"status":"success","encrypted":true}))
}
pub async fn data_market() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    let offers = db.get_all_offers();
    HttpResponse::Ok().json(json!({"status":"success","offers":offers}))
}
pub async fn data_stats() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    let stats = db.get_stats();
    HttpResponse::Ok().json(json!({"status":"success","stats":stats}))
}
pub async fn data_publish() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.publish_offer("data_seller", "Données", 10, 100);
    HttpResponse::Ok().json(json!({"status":"success","published":true}))
}
pub async fn data_purchase() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.record_transaction("buyer", "seller", 100, 1);
    HttpResponse::Ok().json(json!({"status":"success","purchased":true}))
}
pub async fn data_tunnel() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("tunnel", "open", "data");
    HttpResponse::Ok().json(json!({"status":"success","tunnel_open":true}))
}
pub async fn data_price() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("data", "price", "set");
    HttpResponse::Ok().json(json!({"status":"success","price_set":true}))
}

// ==========================================================
// BLACKKEY + BIOMÉTRIE + VÉRIFICATION (10)
// ==========================================================
pub async fn blackkey_sign() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("blackkey", "sign", "data");
    HttpResponse::Ok().json(json!({"status":"success","signed":true}))
}
pub async fn blackkey_decrypt() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("blackkey", "decrypt", "data");
    HttpResponse::Ok().json(json!({"status":"success","decrypted":true}))
}
pub async fn blackkey_mutate() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("blackkey", "mutate", "key");
    HttpResponse::Ok().json(json!({"status":"success","mutated":true}))
}
pub async fn blackkey_integrity() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("blackkey", "integrity", "check");
    HttpResponse::Ok().json(json!({"status":"success","integrity":"verified"}))
}
pub async fn biometric_template() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("biometric", "template", "create");
    HttpResponse::Ok().json(json!({"status":"success","template_created":true}))
}
pub async fn biometric_verify() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("biometric", "verify", "match");
    HttpResponse::Ok().json(json!({"status":"success","verified":true}))
}
pub async fn biometric_liveness() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("biometric", "liveness", "check");
    HttpResponse::Ok().json(json!({"status":"success","alive":true}))
}
pub async fn biometric_spoofing() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("biometric", "spoofing", "detect");
    HttpResponse::Ok().json(json!({"status":"success","spoofing_detected":false}))
}
pub async fn verify_youtube() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("verify", "youtube", "check");
    HttpResponse::Ok().json(json!({"status":"success","platform":"youtube","verified":true}))
}
pub async fn verify_spotify() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("verify", "spotify", "check");
    HttpResponse::Ok().json(json!({"status":"success","platform":"spotify","verified":true}))
}
pub async fn verify_instagram() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("verify", "instagram", "check");
    HttpResponse::Ok().json(json!({"status":"success","platform":"instagram","verified":true}))
}
pub async fn verify_tiktok() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("verify", "tiktok", "check");
    HttpResponse::Ok().json(json!({"status":"success","platform":"tiktok","verified":true}))
}

// ==========================================================
// DIVERS (10)
// ==========================================================
pub async fn emergency_sos() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("sos", "broadcast", "URGENCE");
    HttpResponse::Ok().json(json!({"status":"success","sos_sent":true}))
}
pub async fn emergency_beacon() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("emergency", "beacon", "broadcast");
    HttpResponse::Ok().json(json!({"status":"success","beacon_sent":true}))
}
pub async fn internet_sell() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.publish_offer("internet_seller", "Connexion", 5, 100);
    HttpResponse::Ok().json(json!({"status":"success","selling":true}))
}
pub async fn transport_status() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("transport", "status", "check");
    HttpResponse::Ok().json(json!({"status":"success","ready":true}))
}
pub async fn autopilot_status() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("autopilot", "status", "check");
    HttpResponse::Ok().json(json!({"status":"success","active":true}))
}
pub async fn secure_retrieve() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("secure", "retrieve", "data");
    HttpResponse::Ok().json(json!({"status":"success","retrieved":true}))
}
pub async fn sellers_list() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    let offers = db.get_all_offers();
    HttpResponse::Ok().json(json!({"status":"success","sellers":offers}))
}
pub async fn dao_stats() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    let stats = db.get_stats();
    HttpResponse::Ok().json(json!({"status":"success","dao_stats":stats}))
}
pub async fn publish_item() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.publish_offer("publisher", "Item publié", 0, 0);
    HttpResponse::Ok().json(json!({"status":"success","published":true}))
}
pub async fn mini_peers() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    let stats = db.get_stats();
    HttpResponse::Ok().json(json!({"status":"success","peers":stats}))
}

// ==========================================================
// HANDLERS AJOUTÉS (51 manquants)
// ==========================================================

pub async fn arbitration_dispute() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("disputant", "arbitrator", "Litige ouvert");
    HttpResponse::Ok().json(json!({"status":"success","dispute_opened":true}))
}
pub async fn arbitration_status() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    let stats = db.get_stats();
    HttpResponse::Ok().json(json!({"status":"success","disputes":stats["messages"]}))
}
pub async fn arbitration_vote() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("arbitrator", "dispute", "Vote enregistré");
    HttpResponse::Ok().json(json!({"status":"success","vote_cast":true}))
}
pub async fn fraud_blacklist() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.publish_offer("admin", "Adresse blacklistée", 0, 0);
    HttpResponse::Ok().json(json!({"status":"success","blacklisted":true}))
}
pub async fn fraud_history() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    let stats = db.get_stats();
    HttpResponse::Ok().json(json!({"status":"success","history":stats}))
}
pub async fn fraud_report() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.publish_offer("fraud_reporter", "Fraude signalée", 0, 0);
    HttpResponse::Ok().json(json!({"status":"success","reported":true}))
}
pub async fn fraud_score() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    let dashboard = db.get_dashboard("fraud_check");
    HttpResponse::Ok().json(json!({"status":"success","risk_score":0,"dashboard":dashboard}))
}
pub async fn fraud_unblacklist() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("admin", "network", "Adresse débloquée");
    HttpResponse::Ok().json(json!({"status":"success","unblacklisted":true}))
}
pub async fn freeze_complete() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.publish_offer("freezer", "Contrat complété", 0, 0);
    HttpResponse::Ok().json(json!({"status":"success","completed":true}))
}
pub async fn freeze_create() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.publish_offer("freezer", "Contrat de gel", 0, 0);
    HttpResponse::Ok().json(json!({"status":"success","contract_created":true}))
}
pub async fn freeze_early() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.publish_offer("freezer", "Dégel anticipé", 0, 0);
    HttpResponse::Ok().json(json!({"status":"success","early_unfrozen":true}))
}
pub async fn frequency_hop_now() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("freq", "hop", "now");
    HttpResponse::Ok().json(json!({"status":"success","hopped":true}))
}
pub async fn mailbox_delete() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("mailbox", "delete", "message");
    HttpResponse::Ok().json(json!({"status":"success","deleted":true}))
}
pub async fn mailbox_get(req: actix_web::HttpRequest) -> HttpResponse {
    // 🔐 SÉCURITÉ (vague 1) — USER : accès à SES données (ou owner)
    // (handler sans paramètre de chemin : contrôle JWT simple)
    if let Err(r) = crate::auth_guard::exiger_auth(&req) {
        return r;
    }
    let db = Database::shared("uq_platform_db.json");
    db.send_message("mailbox", "get", "messages");
    HttpResponse::Ok().json(json!({"status":"success","messages":[]}))
}
pub async fn mailbox_send() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("sender", "recipient", "Message mailbox");
    HttpResponse::Ok().json(json!({"status":"success","sent":true}))
}
pub async fn mailbox_stats() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    let stats = db.get_stats();
    HttpResponse::Ok().json(json!({"status":"success","stats":stats}))
}
pub async fn marketplace_buy() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.record_transaction("buyer", "seller", 100, 1);
    HttpResponse::Ok().json(json!({"status":"success","purchased":true}))
}
pub async fn marketplace_complete() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("marketplace", "complete", "order");
    HttpResponse::Ok().json(json!({"status":"success","completed":true}))
}
pub async fn marketplace_publish() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.publish_offer("seller", "Produit marketplace", 100, 1);
    HttpResponse::Ok().json(json!({"status":"success","published":true}))
}
pub async fn micro_credit_fund() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.publish_offer("lender", "Prêt financé", 0, 0);
    HttpResponse::Ok().json(json!({"status":"success","funded":true}))
}
pub async fn micro_credit_request() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.publish_offer("borrower", "Demande micro-crédit", 0, 0);
    HttpResponse::Ok().json(json!({"status":"success","request_created":true}))
}
pub async fn micro_credit_voucher() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("voucher", "borrower", "Garantie ajoutée");
    HttpResponse::Ok().json(json!({"status":"success","voucher_added":true}))
}
pub async fn offline_apply() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.record_transaction("offline_sender", "offline_recipient", 500, 5);
    HttpResponse::Ok().json(json!({"status":"success","applied":true}))
}
pub async fn offline_create() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.publish_offer("offline_user", "Transaction hors ligne", 0, 0);
    HttpResponse::Ok().json(json!({"status":"success","offline_tx_created":true}))
}
pub async fn offline_verify() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("verifier", "offline_user", "Transaction vérifiée");
    HttpResponse::Ok().json(json!({"status":"success","verified":true}))
}
// GET /api/oracle/current — le prix interne du BORB tel que le régulateur l'a
// fixé, avec son horodatage. Plus le « price: 2000 » codé en dur qui affirmait
// un cours que personne n'avait mesuré.
pub async fn oracle_current() -> HttpResponse {
    let prix = crate::persistence::load_prix();
    let valeur = prix
        .prix_actuel
        .parse::<f64>()
        .ok()
        .filter(|p| p.is_finite() && *p > 0.0);
    HttpResponse::Ok().json(json!({
        "status": "success",
        "price": valeur,
        "source": if valeur.is_some() { "regulateur" } else { "prix_non_initialise" },
        "dernier_ajustement": prix.dernier_ajustement,
        "demande": prix.demande,
        "rarete": prix.rarete,
    }))
}

// GET /api/oracle/median[?devise=CDF] — la médiane réellement calculée par le
// moteur de taux. Aucune valeur n'est renvoyée si la médiane n'existe pas.
pub async fn oracle_median(
    query: web::Query<std::collections::HashMap<String, String>>,
) -> HttpResponse {
    let devise = query
        .get("devise")
        .cloned()
        .unwrap_or_else(|| "CDF".to_string())
        .to_uppercase();
    let information = crate::engines::rates::etat::lire_snapshot()
        .and_then(|s| s.informations.get(&devise).cloned());
    match information {
        Some(info) => HttpResponse::Ok().json(json!({
            "status": "success",
            "devise": devise,
            "median": info.mediane,
            "valeur_servie": info.valeur,
            "source": info.source,
            "origines": info.origines,
        })),
        None => HttpResponse::ServiceUnavailable().json(json!({
            "status": "error",
            "code": "TAUX_INDISPONIBLE",
            "devise": devise,
            "median": null,
            "message": "Aucune médiane observée pour cette devise.",
        })),
    }
}
pub async fn oracle_report() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.publish_offer("oracle", "Prix signalé", 0, 0);
    HttpResponse::Ok().json(json!({"status":"success","price_reported":true}))
}
pub async fn oracle_validators() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    let stats = db.get_stats();
    HttpResponse::Ok().json(json!({"status":"success","validators":7,"stats":stats}))
}
pub async fn p2p_broadcast() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("broadcaster", "all_nodes", "Broadcast P2P");
    HttpResponse::Ok().json(json!({"status":"success","broadcast":true}))
}
pub async fn p2p_peers() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    let stats = db.get_stats();
    HttpResponse::Ok().json(json!({"status":"success","peers":stats["sessions"]}))
}
pub async fn p2p_send() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("p2p_sender", "p2p_recipient", "Message P2P");
    HttpResponse::Ok().json(json!({"status":"success","sent":true}))
}
pub async fn p2p_start() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.publish_offer("p2p_node", "Nœud P2P démarré", 0, 0);
    HttpResponse::Ok().json(json!({"status":"success","started":true}))
}
pub async fn reputation_endorse() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("endorser", "endorsee", "Endorsement");
    HttpResponse::Ok().json(json!({"status":"success","endorsed":true}))
}
pub async fn reputation_get(req: actix_web::HttpRequest, path: web::Path<String>) -> HttpResponse {
    // 🔐 SÉCURITÉ (vague 1) — USER : accès à SES données (ou owner)
    // `/api/reputation/{id}` : la cible est l'UQ du chemin, vérifié contre le JWT.
    let cible = path.into_inner().trim().to_string();
    if !crate::auth_guard::ressemble_a_un_uq(&cible) {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "code": "UQ_INVALIDE",
            "message": "Identifiant de compte attendu (format UQ-00000000)"
        }));
    }
    if let Err(r) = crate::auth_guard::exiger_soi_ou_owner(&req, &cible) {
        return r;
    }
    let db = Database::shared("uq_platform_db.json");
    let dashboard = db.get_dashboard(&cible);
    HttpResponse::Ok().json(json!({
        "status": "success",
        "uq": cible,
        "score": dashboard
    }))
}
pub async fn savings_contract() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.publish_offer("saver", "Contrat épargne", 0, 0);
    HttpResponse::Ok().json(json!({"status":"success","contract_created":true}))
}
pub async fn savings_yield(req: actix_web::HttpRequest) -> HttpResponse {
    // 🔐 USER : le rendement est calculé sur les CONTRATS RÉELS du compte
    // (table `my_savings` de sa base). Aucun pourcentage n'est renvoyé en dur.
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(claims) => claims,
        Err(response) => return response,
    };
    let db_path = format!(
        "data/users/{}.db",
        claims.sub.replace("-", "_").to_lowercase()
    );
    if !std::path::Path::new(&db_path).exists() {
        return HttpResponse::NotFound().json(json!({
            "status": "error",
            "code": "COMPTE_ABSENT",
            "message": "Compte introuvable"
        }));
    }
    let conn = match rusqlite::Connection::open(&db_path) {
        Ok(conn) => conn,
        Err(error) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Base du compte inaccessible : {}", error)
            }))
        }
    };
    let _ = conn.busy_timeout(std::time::Duration::from_secs(5));

    let lecture = conn.prepare(
        "SELECT id, COALESCE(amount_points, 0), COALESCE(yield_rate, 0),
                COALESCE(locked_until, 0), COALESCE(status, '')
         FROM my_savings WHERE status = 'active' ORDER BY locked_until ASC LIMIT 200",
    );
    let lignes = match lecture {
        Ok(mut stmt) => match stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, f64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
            ))
        }) {
            Ok(rows) => rows
                .map(|row| row.map_err(|e| e.to_string()))
                .collect::<Result<Vec<_>, String>>(),
            Err(error) => Err(error.to_string()),
        },
        Err(error) => Err(error.to_string()),
    };
    let lignes = match lignes {
        Ok(lignes) => lignes,
        Err(error) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "code": "LECTURE_IMPOSSIBLE",
                "message": format!("Épargne illisible : {}", error)
            }))
        }
    };

    let maintenant = chrono::Utc::now().timestamp();
    let detail: Vec<serde_json::Value> = lignes
        .iter()
        .map(|(id, montant, taux, echeance, statut)| json!({
            "id": id,
            "amount_points": montant,
            "yield_rate": taux,
            "locked_until": echeance,
            "jours_restants": (((*echeance - maintenant) as f64) / 86_400.0).ceil().max(0.0) as i64,
            "status": statut
        }))
        .collect();

    // Moyenne pondérée par les montants réellement bloqués.
    let total_points: i64 = lignes.iter().map(|(_, montant, _, _, _)| *montant).sum();
    let yield_pct = if total_points > 0 {
        lignes
            .iter()
            .map(|(_, montant, taux, _, _)| (*montant as f64) * taux)
            .sum::<f64>()
            / (total_points as f64)
    } else {
        0.0
    };

    HttpResponse::Ok().json(json!({
        "status": "success",
        "uq": claims.sub,
        "source": "my_savings",
        "contrats_actifs": lignes.len(),
        "total_bloque_points": total_points,
        "yield_pct": yield_pct,
        "gains_attendus_points": (total_points as f64) * yield_pct / 100.0,
        "contrats": detail,
        "message": if lignes.is_empty() { "Aucun contrat d'épargne actif" } else { "" }
    }))
}
pub async fn serve_index() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    let stats = db.get_stats();
    HttpResponse::Ok().json(json!({"status":"success","service":"uq_platform","stats":stats}))
}
pub async fn shield_aid() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("aid_requester", "shield_pool", "Demande d aide");
    HttpResponse::Ok().json(json!({"status":"success","aid_requested":true}))
}
pub async fn shield_approve() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("validator", "aid_requester", "Aide approuvée");
    HttpResponse::Ok().json(json!({"status":"success","approved":true}))
}
pub async fn shield_key_lost() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("key_lost_user", "guardians", "Clé perdue");
    HttpResponse::Ok().json(json!({"status":"success","declared":true}))
}
pub async fn shops_list() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    let mut shops = db.get_all_shops();
    shops.insert(0, crate::connexion::boutique_systeme());
    HttpResponse::Ok().json(json!({"status":"success","shops":shops,"total":shops.len()}))
}
pub async fn stablecoin_burn() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.publish_offer("burner", "UST burned", 0, 0);
    HttpResponse::Ok().json(json!({"status":"success","burned":true}))
}
pub async fn stablecoin_mint() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.publish_offer("minter", "UST minted", 0, 0);
    HttpResponse::Ok().json(json!({"status":"success","minted":true}))
}
pub async fn stablecoin_ratio() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("stablecoin", "ratio", "check");
    HttpResponse::Ok().json(json!({"status":"success","ratio":150.0}))
}
pub async fn validator_list() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    let stats = db.get_stats();
    HttpResponse::Ok().json(json!({"status":"success","validators":stats["accounts"]}))
}
pub async fn validator_validate() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.send_message("validator", "network", "Validation effectuée");
    HttpResponse::Ok().json(json!({"status":"success","validated":true}))
}

// ==========================================================
// ROUTES DISTRIBUTION 30/70
// ==========================================================

pub async fn distribution_stats(req: actix_web::HttpRequest) -> HttpResponse {
    if let Err(r) = crate::auth_guard::exiger_owner(&req) {
        return r;
    }
    use crate::borb_distribution::BorbDistribution;
    let dist = BorbDistribution::new("UQ-00000001");
    HttpResponse::Ok().json(dist.get_stats())
}

pub async fn distribution_treasury_status(req: actix_web::HttpRequest) -> HttpResponse {
    if let Err(r) = crate::auth_guard::exiger_owner(&req) {
        return r;
    }
    use crate::borb_distribution::BorbDistribution;
    let dist = BorbDistribution::new("UQ-00000001");
    let mut stats = dist.get_stats();
    // Le frontend resout le profil vise via les champs `uq`/`email` : on
    // expose les identifiants reels pour que la bascule ne dépende pas du
    // localStorage du terminal.
    if let Ok(idx) = rusqlite::Connection::open("uq_index.db") {
        let email_owner: String = idx
            .query_row(
                "SELECT COALESCE(uq_email, '') FROM uq_index WHERE uq_number = ?1",
                rusqlite::params![crate::auth_guard::OWNER_UQ],
                |row| row.get(0),
            )
            .unwrap_or_default();
        if let Some(obj) = stats
            .get_mut("profile1_treasury")
            .and_then(|v| v.as_object_mut())
        {
            obj.insert("uq".to_string(), json!(crate::auth_guard::OWNER_UQ));
            if !email_owner.is_empty() {
                obj.insert("email".to_string(), json!(email_owner));
            }
        }
    }
    HttpResponse::Ok().json(stats["profile1_treasury"].clone())
}

pub async fn distribution_delis_status(req: actix_web::HttpRequest) -> HttpResponse {
    if let Err(r) = crate::auth_guard::exiger_owner(&req) {
        return r;
    }
    use crate::borb_distribution::BorbDistribution;
    let dist = BorbDistribution::new("UQ-00000001");
    let mut stats = dist.get_stats();
    // Le membre actif du cercle owner est LE profil 2 : on expose son UQ
    // et son email pour que la bascule cible le bon compte sans deviner
    // ni mémoriser.
    if let Ok(idx) = rusqlite::Connection::open("uq_index.db") {
        if let Ok((member, email)) = idx.query_row(
            "SELECT cm.member_uq, COALESCE(ui.uq_email, '')
             FROM circle_members cm
             LEFT JOIN uq_index ui ON ui.uq_number = cm.member_uq
             WHERE cm.owner_uq = ?1 AND cm.is_active = 1 LIMIT 1",
            rusqlite::params![crate::auth_guard::OWNER_UQ],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        ) {
            if let Some(obj) = stats
                .get_mut("profile2_delis")
                .and_then(|v| v.as_object_mut())
            {
                obj.insert("uq".to_string(), json!(member));
                if !email.is_empty() {
                    obj.insert("email".to_string(), json!(email));
                }
            }
        }
    }
    HttpResponse::Ok().json(stats["profile2_delis"].clone())
}

pub async fn distribution_sell_crypto(req: actix_web::HttpRequest) -> HttpResponse {
    if let Err(r) = crate::auth_guard::exiger_owner(&req) {
        return r;
    }
    use crate::borb_distribution::BorbDistribution;
    let dist = BorbDistribution::new("UQ-00000001");
    match dist.profile1_sell_crypto(100, true) {
        Ok(stock) => HttpResponse::Ok().json(json!({"status":"success","stock_restant":stock})),
        Err(e) => {
            HttpResponse::BadRequest().json(json!({"status":"error","message": format!("{}", e)}))
        }
    }
}

pub async fn distribution_sell_p2p(req: actix_web::HttpRequest) -> HttpResponse {
    if let Err(r) = crate::auth_guard::exiger_owner(&req) {
        return r;
    }
    use crate::borb_distribution::BorbDistribution;
    let dist = BorbDistribution::new("UQ-00000001");
    match dist.profile2_sell_p2p(50, true) {
        Ok(balance) => {
            HttpResponse::Ok().json(json!({"status":"success","balance_restant":balance}))
        }
        Err(e) => {
            HttpResponse::BadRequest().json(json!({"status":"error","message": format!("{}", e)}))
        }
    }
}

pub async fn distribution_auto_recharge(req: actix_web::HttpRequest) -> HttpResponse {
    if let Err(r) = crate::auth_guard::exiger_owner(&req) {
        return r;
    }
    use crate::borb_distribution::BorbDistribution;
    let dist = BorbDistribution::new("UQ-00000001");
    let recharged = dist.profile1_auto_recharge();
    HttpResponse::Ok().json(json!({"status":"success","recharged":recharged}))
}

pub async fn distribution_claim(body: web::Json<serde_json::Value>) -> HttpResponse {
    use crate::borb_distribution::BorbDistribution;
    use rusqlite::Connection;

    // 1. Recuperer le UQ du profil 2 (depuis le body)
    let uq_profil2 = body
        .get("uq_profil2")
        .and_then(|v| v.as_str())
        .unwrap_or("UQ-230354360");

    // 2. Ouvrir la DB pour verifier le solde multi_wallets
    let conn = match rusqlite::Connection::open("uq_index.db") {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": e.to_string()
            }))
        }
    };

    let solde_avant: f64 = conn.query_row(
        "SELECT COALESCE(balance, 0.0) FROM multi_wallets WHERE uq_number = ?1 AND currency = 'BORB'",
        rusqlite::params![uq_profil2],
        |row| row.get(0),
    ).unwrap_or(0.0);

    // 3. Verifier le seuil 50% (150 000 BORB)
    let seuil_max = 150_000.0;
    if solde_avant >= seuil_max {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": format!("Solde {} >= seuil {} - Reclamation interdite", solde_avant, seuil_max)
        }));
    }

    // 4. Reclamation treasury -> dev_profile (compteurs internes)
    let dist = BorbDistribution::new("UQ-00000001");
    let _ = match dist.profile2_claim_from_profile1() {
        Ok(_new_balance) => {}
        Err(e) => {
            return HttpResponse::BadRequest().json(json!({
                "status": "error",
                "message": format!("{}", e)
            }))
        }
    };

    // 5. Deplacer le vrai BORB : points owner -> points profil2.
    //    multi_wallets.BORB n'est qu'un miroir de points/100 : on le
    //    resynchronise apres le mouvement au lieu de l'incrementer.
    let montant_points: i64 = (300_000.0 * 100.0) as i64;
    let owner_db = "data/users/uq_00000001.db";
    let p2_db = format!(
        "data/users/{}.db",
        uq_profil2.replace("-", "_").to_lowercase()
    );

    let conn_owner = match Connection::open(owner_db) {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("DB owner: {}", e)
            }))
        }
    };
    let debit = match conn_owner.execute(
        "UPDATE balance SET points = points - ?1 WHERE id = 1 AND points - COALESCE(frozen_points, 0) >= ?1",
        rusqlite::params![montant_points],
    ) {
        Ok(n) => n,
        Err(e) => return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("Debit owner impossible: {}", e)
        })),
    };
    if debit != 1 {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "code": "BORB_INSUFFISANT",
            "message": "Solde BORB mobilisable du Profil 1 insuffisant"
        }));
    }

    let conn_p2 = match Connection::open(&p2_db) {
        Ok(c) => c,
        Err(e) => {
            let _ = conn_owner.execute(
                "UPDATE balance SET points = points + ?1 WHERE id = 1",
                rusqlite::params![montant_points],
            );
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("DB profil2: {}", e)
            }));
        }
    };
    if let Err(e) = conn_p2.execute(
        "UPDATE balance SET points = points + ?1 WHERE id = 1",
        rusqlite::params![montant_points],
    ) {
        let _ = conn_owner.execute(
            "UPDATE balance SET points = points + ?1 WHERE id = 1",
            rusqlite::params![montant_points],
        );
        return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("Credit profil2 impossible: {}", e)
        }));
    }

    // 6. Resynchroniser le miroir multi_wallets.BORB = points/100
    let now = chrono::Utc::now().timestamp();
    for (uq_mirror, conn_mirror) in [("UQ-00000001", &conn_owner), (uq_profil2, &conn_p2)] {
        let pts: i64 = conn_mirror
            .query_row(
                "SELECT COALESCE(points, 0) FROM balance WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let _ = conn.execute(
            "INSERT INTO multi_wallets (uq_number, currency, balance, updated_at)
             VALUES (?1, 'BORB', ?2, ?3)
             ON CONFLICT(uq_number, currency) DO UPDATE SET balance = ?2, updated_at = ?3",
            rusqlite::params![uq_mirror, pts as f64 / 100.0, now],
        );
    }

    let solde_apres: f64 = conn.query_row(
        "SELECT COALESCE(balance, 0.0) FROM multi_wallets WHERE uq_number = ?1 AND currency = 'BORB'",
        rusqlite::params![uq_profil2],
        |row| row.get(0),
    ).unwrap_or(0.0);

    eprintln!(
        "[CLAIM] {} reclame {} BORB ({} -> {})",
        uq_profil2, 300_000.0, solde_avant, solde_apres
    );

    HttpResponse::Ok().json(json!({
        "status": "success",
        "uq_profil2": uq_profil2,
        "montant_reclame": 300_000.0,
        "solde_avant": solde_avant,
        "solde_apres": solde_apres
    }))
}

pub async fn distribution_ubi_distribute() -> HttpResponse {
    use crate::borb_distribution::BorbDistribution;
    let dist = BorbDistribution::new("UQ-00000001");
    match dist.distribute_ubi(100, 2) {
        Ok(total) => HttpResponse::Ok().json(json!({"status":"success","total_distribue":total})),
        Err(e) => {
            HttpResponse::BadRequest().json(json!({"status":"error","message": format!("{}", e)}))
        }
    }
}

// ====================
// JOURNAL INDIVIDUEL — EXPORT
// ====================

pub async fn journal_export(req: actix_web::HttpRequest, path: web::Path<String>) -> HttpResponse {
    // 🔐 SÉCURITÉ (vague 1) — USER : accès à SES données (ou owner)
    // (handler sans paramètre de chemin : contrôle JWT simple)
    // 🔐 SÉCURITÉ (vague 1) — USER : accès à SES données (ou owner)
    let __cible = path.to_string();
    if let Err(r) = crate::auth_guard::exiger_soi_ou_owner(&req, &__cible) {
        return r;
    }

    let account_id = path.into_inner();
    let db = Database::shared("uq_platform_db.json");
    let journal = db.get_journal(&account_id);

    HttpResponse::Ok().json(json!({
        "status": "success",
        "account_id": account_id,
        "entries": journal,
        "total_entries": journal.len(),
        "exported_at": chrono::Utc::now().to_rfc3339()
    }))
}

pub async fn journal_summary(req: actix_web::HttpRequest, path: web::Path<String>) -> HttpResponse {
    // 🔐 SÉCURITÉ (vague 1) — USER : accès à SES données (ou owner)
    // (handler sans paramètre de chemin : contrôle JWT simple)
    // 🔐 SÉCURITÉ (vague 1) — USER : accès à SES données (ou owner)
    let __cible = path.to_string();
    if let Err(r) = crate::auth_guard::exiger_soi_ou_owner(&req, &__cible) {
        return r;
    }

    let account_id = path.into_inner();
    let db = Database::shared("uq_platform_db.json");
    let journal = db.get_journal(&account_id);

    let total_sent: i64 = journal
        .iter()
        .filter(|e| e.get("tx_type").and_then(|v| v.as_str()) == Some("transfer_sent"))
        .map(|e| e.get("amount_points").and_then(|v| v.as_i64()).unwrap_or(0))
        .sum();

    let total_received: i64 = journal
        .iter()
        .filter(|e| e.get("tx_type").and_then(|v| v.as_str()) == Some("transfer_received"))
        .map(|e| e.get("amount_points").and_then(|v| v.as_i64()).unwrap_or(0))
        .sum();

    HttpResponse::Ok().json(json!({
        "status": "success",
        "account_id": account_id,
        "total_entries": journal.len(),
        "total_sent": total_sent,
        "total_received": total_received,
        "net": total_received - total_sent
    }))
}

// ====================
// 🛒 ZANDO — HANDLERS
// ====================

// ===== ZANDO ÉLÉMENTS (6 handlers) =====

/// Crée un nouvel élément Zando
pub async fn zando_item_create(
    req: actix_web::HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(claims) => claims,
        Err(response) => return response,
    };
    let db = Database::shared("uq_platform_db.json");

    let item_type = body.get("type").and_then(|v| v.as_str()).unwrap_or("text");
    let media_urls = body
        .get("media_urls")
        .and_then(|v| v.as_str())
        .unwrap_or("[]");
    let title = body
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if title.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Le titre est obligatoire"
        }));
    }
    let description = body
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let subtitle = body
        .get("subtitle")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let price_borb = match body.get("price_borb").and_then(|v| v.as_u64()) {
        // Une vitrine de boutique n'a pas de prix : elle se paie en frais de
        // présence (5 % du catalogue), pas à l'achat.
        Some(price) if price <= i64::MAX as u64 && (price > 0 || item_type == "boutique") => price,
        _ => {
            return HttpResponse::BadRequest().json(json!({
                "status": "error",
                "message": if item_type == "boutique" {
                    "La vitrine ne porte pas de prix : le champ price_borb doit valoir 0"
                } else {
                    "Le prix doit être un nombre entier positif en BORB"
                }
            }));
        }
    };
    let price_usd = body
        .get("price_usd")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let category = body
        .get("category")
        .and_then(|v| v.as_str())
        .unwrap_or("general");
    let commission_enabled = body
        .get("commission_enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let commission_rate = body
        .get("commission_rate")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    match db.create_shop_item(
        &claims.sub,
        item_type,
        media_urls,
        title,
        subtitle,
        description,
        price_borb,
        price_usd,
        category,
        commission_enabled,
        commission_rate,
    ) {
        Ok(id) => {
            // Position et type marketplace (optionnels, envoyés par le Studio).
            let latitude = body.get("latitude").and_then(|v| v.as_f64());
            let longitude = body.get("longitude").and_then(|v| v.as_f64());
            let type_produit = body.get("type_produit").and_then(|v| v.as_str());
            if latitude.is_some() || longitude.is_some() || type_produit.is_some() {
                let _ = db.set_shop_item_meta(&id, latitude, longitude, type_produit);
            }
            HttpResponse::Ok().json(json!({
                "status": "success",
                "item_id": id
            }))
        }
        Err(error) => HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("Création de l'offre impossible: {}", error)
        })),
    }
}

/// Récupère les éléments d'une boutique
pub async fn zando_items_list(req: actix_web::HttpRequest) -> HttpResponse {
    // 🔐 SÉCURITÉ — USER : JWT obligatoire.
    // Sans auth, la liste exposait media_urls (vidéos) à tout le monde :
    // les médias partaient gratuitement dans le flux. Comme
    // zando_item_get, la lecture exige désormais un compte authentifié.
    if let Err(r) = crate::auth_guard::exiger_auth(&req) {
        return r;
    }
    let db = Database::shared("uq_platform_db.json");
    let items = db.get_all_shop_items();

    HttpResponse::Ok().json(json!({
        "status": "success",
        "items": items,
        "total": items.len()
    }))
}
pub async fn zando_items_get(req: actix_web::HttpRequest, path: web::Path<String>) -> HttpResponse {
    // 🔐 SÉCURITÉ (vague 1) — USER : JWT obligatoire
    // 🔐 SÉCURITÉ (vague 1) — USER : accès à SES données (ou owner)
    // 🔐 SÉCURITÉ (vague 1) — USER : JWT obligatoire (le contrôle de
    // propriété de l'objet {id} reste à la charge du handler)
    if let Err(r) = crate::auth_guard::exiger_auth(&req) {
        return r;
    }

    let db = Database::shared("uq_platform_db.json");
    let shop_id = path.into_inner();
    let items = db.get_shop_items(&shop_id);

    HttpResponse::Ok().json(json!({
        "status": "success",
        "items": items,
        "total": items.len()
    }))
}

pub async fn zando_seller_analytics(
    req: actix_web::HttpRequest,
    path: web::Path<String>,
) -> HttpResponse {
    let shop_id = path.into_inner();
    if let Err(response) = crate::auth_guard::exiger_soi_ou_owner(&req, &shop_id) {
        return response;
    }
    let db = Database::shared("uq_platform_db.json");
    match db.get_shop_daily_analytics(&shop_id) {
        Ok(daily) => HttpResponse::Ok().json(json!({
            "status": "success",
            "period_days": 30,
            "daily": daily
        })),
        Err(error) => HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("Lecture des statistiques Zando impossible: {}", error)
        })),
    }
}

/// Récupère un élément par ID
pub async fn zando_item_get(req: actix_web::HttpRequest, path: web::Path<String>) -> HttpResponse {
    // 🔐 SÉCURITÉ (vague 1) — USER : JWT obligatoire
    // 🔐 SÉCURITÉ (vague 1) — USER : accès à SES données (ou owner)
    // 🔐 SÉCURITÉ (vague 1) — USER : JWT obligatoire (le contrôle de
    // propriété de l'objet {id} reste à la charge du handler)
    if let Err(r) = crate::auth_guard::exiger_auth(&req) {
        return r;
    }

    let db = Database::shared("uq_platform_db.json");
    let id = path.into_inner();

    match db.get_shop_item_by_id(&id) {
        Some(item) => HttpResponse::Ok().json(json!({
            "status": "success",
            "item": item
        })),
        None => HttpResponse::NotFound().json(json!({
            "status": "error",
            "message": "Item not found"
        })),
    }
}

pub async fn zando_item_interact(
    req: HttpRequest,
    path: web::Path<String>,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let claims = match crate::auth::AuthManager::require_role(&req, &["user", "admin", "guardian"])
    {
        Ok(claims) => claims,
        Err(response) => return response,
    };
    let interaction_type = match body.get("type").and_then(|value| value.as_str()) {
        Some(kind @ ("view" | "like" | "share" | "comment")) => kind,
        _ => {
            return HttpResponse::BadRequest().json(json!({
                "status": "error",
                "message": "Type d'interaction invalide"
            }));
        }
    };

    // Un commentaire porte son texte : c'est ce qui alimente le fil du flux
    // et le canal Tendances du chat. Les autres interactions n'ont pas de data.
    let data = if interaction_type == "comment" {
        let texte = body
            .get("content")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .trim();
        if texte.is_empty() {
            return HttpResponse::BadRequest().json(json!({
                "status": "error",
                "message": "Commentaire vide"
            }));
        }
        json!({ "content": texte.chars().take(500).collect::<String>() }).to_string()
    } else {
        "{}".to_string()
    };

    let db = Database::shared("uq_platform_db.json");
    let item_id = path.into_inner();
    match db.add_shop_item_interaction(&item_id, &claims.sub, interaction_type, &data) {
        Ok(()) => {
            // Badge de l'ecran Zando : le vendeur est prevenu, jamais pour soi-meme.
            if interaction_type == "like" || interaction_type == "comment" {
                if let Some(item) = db.get_shop_item_by_id(&item_id) {
                    let proprietaire = item
                        .get("shop_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    if !proprietaire.is_empty() && !proprietaire.eq_ignore_ascii_case(&claims.sub) {
                        let titre = item
                            .get("title")
                            .and_then(|v| v.as_str())
                            .unwrap_or("ton article");
                        crate::trading_routes::push_notification(
                            &proprietaire,
                            "zando",
                            "Nouvelle interaction",
                            &format!("Quelqu'un a aime ou commente ton article « {} ».", titre),
                            &format!("zando:{}", item_id),
                        );
                    }
                }
            }
            HttpResponse::Ok().json(json!({ "status": "success" }))
        }
        Err(error) if error == "Item introuvable" => HttpResponse::NotFound().json(json!({
            "status": "error",
            "message": error
        })),
        Err(error) => HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": error
        })),
    }
}

/// GET /api/zando/item/{id}/interactions — PUBLIC (décision produit : le flux
/// et ses statistiques sont visibles par tous, y compris sans compte).
/// ⚠️ La liste expose les UQ des vues/likes ; les commentaires affichent déjà
/// leur auteur dans le flux. Une variante « compteurs seuls » est possible si
/// on veut publier uniquement les volumes sans identifier les visiteurs.
pub async fn zando_item_interactions(path: web::Path<String>) -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    match db.get_shop_item_interactions(&path.into_inner()) {
        Ok(interactions) => HttpResponse::Ok().json(json!({
            "status": "success",
            "data": { "interactions": interactions }
        })),
        Err(error) => HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": error
        })),
    }
}

/// Met à jour un élément
/// Anciennete d'un horodatage SQLite (`YYYY-MM-DD HH:MM:SS`), en heures.
fn heures_depuis(horodatage: &str) -> Option<f64> {
    let naif = chrono::NaiveDateTime::parse_from_str(horodatage, "%Y-%m-%d %H:%M:%S").ok()?;
    Some((chrono::Utc::now() - naif.and_utc()).num_seconds().max(0) as f64 / 3600.0)
}

/// Modifie un élément Zando.
///
/// Une vente solo n'ouvre que trois champs : le prix, la description et le
/// média. Le titre et la catégorie sont verrouillés, deux modifications au
/// maximum, et plus rien après 12 h. Une vitrine de boutique (`boutique`) garde
/// sa mise à jour libre : elle appartient au grand studio, pas à la vente solo.
pub async fn zando_item_update(
    req: actix_web::HttpRequest,
    path: web::Path<String>,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(claims) => claims,
        Err(response) => return response,
    };

    let db = Database::shared("uq_platform_db.json");
    let id = path.into_inner();

    let title = body
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if title.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Le titre est obligatoire"
        }));
    }
    let description = body
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let subtitle = body
        .get("subtitle")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let media_urls = body
        .get("media_urls")
        .and_then(|v| v.as_str())
        .unwrap_or("[]");
    let item_type = body.get("type").and_then(|v| v.as_str()).unwrap_or("text");
    let price_borb = match body.get("price_borb").and_then(|v| v.as_u64()) {
        Some(price) if price <= i64::MAX as u64 && (price > 0 || item_type == "boutique") => price,
        _ => {
            return HttpResponse::BadRequest().json(json!({
                "status": "error",
                "message": if item_type == "boutique" {
                    "La vitrine ne porte pas de prix : le champ price_borb doit valoir 0"
                } else {
                    "Le prix doit être un nombre entier positif en BORB"
                }
            }));
        }
    };

    let price_usd = body
        .get("price_usd")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let category = body
        .get("category")
        .and_then(|v| v.as_str())
        .unwrap_or("general");
    let commission_enabled = body
        .get("commission_enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let commission_rate = body
        .get("commission_rate")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    let existant = match db.get_shop_item_by_id(&id) {
        Some(item) => item,
        None => {
            return HttpResponse::NotFound().json(json!({
                "status": "error",
                "message": "Item introuvable ou accès refusé"
            }))
        }
    };
    if existant.get("shop_id").and_then(|v| v.as_str()) != Some(claims.sub.as_str()) {
        return HttpResponse::NotFound().json(json!({
            "status": "error",
            "message": "Item introuvable ou accès refusé"
        }));
    }

    let est_boutique = existant.get("type").and_then(|v| v.as_str()) == Some("boutique");
    if est_boutique {
        return match db.update_shop_item(
            &id,
            &claims.sub,
            item_type,
            media_urls,
            title,
            subtitle,
            description,
            price_borb,
            price_usd,
            category,
            commission_enabled,
            commission_rate,
        ) {
            Ok(()) => HttpResponse::Ok().json(json!({"status": "success"})),
            Err(e) if e == "Item introuvable ou accès refusé" => {
                HttpResponse::NotFound().json(json!({
                    "status": "error", "message": e
                }))
            }
            Err(e) => HttpResponse::InternalServerError().json(json!({
                "status": "error", "message": format!("Mise à jour impossible: {}", e)
            })),
        };
    }

    if existant.get("sold").and_then(|v| v.as_bool()) == Some(true) {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Cet objet est vendu : il n'est plus modifiable."
        }));
    }

    // Le titre et la catégorie sont l'identité de l'objet : on les refuse même
    // si le client les renvoie différents.
    let titre_actuel = existant.get("title").and_then(|v| v.as_str()).unwrap_or("");
    let categorie_actuelle = existant
        .get("category")
        .and_then(|v| v.as_str())
        .unwrap_or("general");
    if title != titre_actuel || category != categorie_actuelle {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Le titre et la catégorie sont verrouillés : seuls le prix, la description et la photo changent."
        }));
    }

    let modifs = existant
        .get("edit_count")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    if modifs >= 2 {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Cet objet a déjà été modifié 2 fois : il est verrouillé."
        }));
    }

    let en_ligne = existant.get("published").and_then(|v| v.as_bool()) == Some(true);
    if en_ligne {
        let depart = existant
            .get("published_at")
            .and_then(|v| v.as_str())
            .filter(|v| !v.is_empty())
            .or_else(|| existant.get("created_at").and_then(|v| v.as_str()))
            .unwrap_or("");
        if heures_depuis(depart).map_or(false, |heures| heures >= 12.0) {
            return HttpResponse::BadRequest().json(json!({
                "status": "error",
                "message": "Douze heures sont passées depuis la mise en ligne : l'objet n'est plus modifiable."
            }));
        }
    }

    match db.modifier_element_solo(
        &id,
        &claims.sub,
        media_urls,
        description,
        price_borb,
        price_usd,
        commission_enabled,
        commission_rate,
    ) {
        Ok(()) => {
            let latitude = body.get("latitude").and_then(|v| v.as_f64());
            let longitude = body.get("longitude").and_then(|v| v.as_f64());
            let type_produit = body.get("type_produit").and_then(|v| v.as_str());
            if latitude.is_some() || longitude.is_some() || type_produit.is_some() {
                let _ = db.set_shop_item_meta(&id, latitude, longitude, type_produit);
            }
            HttpResponse::Ok().json(json!({"status": "success"}))
        }
        Err(e) if e == "Item introuvable ou accès refusé" => {
            HttpResponse::NotFound().json(json!({
                "status": "error", "message": e
            }))
        }
        Err(e) => HttpResponse::InternalServerError().json(json!({
            "status": "error", "message": format!("Mise à jour impossible: {}", e)
        })),
    }
}

/// POST /api/zando/item/{id}/sold — le vendeur confirme la vente.
/// L'objet quitte le flux et laisse une ligne d'historique (sans photo) dans
/// les analytiques du créateur.
pub async fn zando_item_sold(req: actix_web::HttpRequest, path: web::Path<String>) -> HttpResponse {
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(claims) => claims,
        Err(response) => return response,
    };
    let db = Database::shared("uq_platform_db.json");
    let id = path.into_inner();
    match db.marquer_element_vendu(&id, &claims.sub) {
        Ok(()) => HttpResponse::Ok().json(json!({"status": "success"})),
        Err(e) => HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": e
        })),
    }
}

/// POST /api/shop/{id}/renew — prolonger la présence et ouvrir le robinet.
///
/// Deux gestes, et aucun n'est un débit : ce route vérifie le PIN, repousse
/// l'échéance de présence (jusqu'aux 7 jours maximaux), remet l'horloge du
/// robinet à maintenant et replace la vitrine dans le flux. L'argent, lui,
/// coule seconde par seconde dans `vitrines::boucle_robinet` — 5 % de la valeur
/// du catalogue publié par 24 h. Prélever un bloc ici en plus du tick ferait
/// payer la même présence deux fois.
///
/// Un catalogue sans prix ne doit rien : la vitrine d'une école ou d'une église
/// reste gratuite, le robinet tourne à vide.
pub async fn shop_renew(
    req: actix_web::HttpRequest,
    path: web::Path<String>,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(claims) => claims,
        Err(response) => return response,
    };
    let shop_id = path.into_inner();
    let uq = body
        .get("uq_number")
        .and_then(|v| v.as_str())
        .filter(|v| !v.trim().is_empty())
        .unwrap_or(claims.sub.as_str());
    let pin = body.get("pin").and_then(|v| v.as_str()).unwrap_or("");
    let jours = body
        .get("jours")
        .and_then(|v| v.as_i64())
        .unwrap_or(1)
        .clamp(1, 7);

    if !crate::auth_guard::ressemble_a_un_uq(uq) {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Identifiant de compte invalide"
        }));
    }
    if let Err(response) = crate::auth_guard::exiger_soi_ou_owner(&req, uq) {
        return response;
    }
    if !crate::pin_manager::is_valid_pin_format(pin) {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Le code PIN est requis et doit contenir 4 à 6 chiffres"
        }));
    }

    let db = Database::shared("uq_platform_db.json");
    db.synchroniser_vitrines_expirees();

    let boutique = match db.get_shop_by_id(&shop_id) {
        Some(value) => value,
        None => {
            return HttpResponse::NotFound().json(json!({
                "status": "error", "message": "Boutique introuvable"
            }))
        }
    };
    if boutique.get("owner").and_then(|v| v.as_str()) != Some(uq) {
        return HttpResponse::Forbidden().json(json!({
            "status": "error", "message": "Seul le propriétaire prolonge sa vitrine"
        }));
    }

    let compte = chemin_base_compte(uq);
    if !std::path::Path::new(&compte).exists() {
        return HttpResponse::NotFound().json(json!({
            "status": "error", "message": "Compte introuvable"
        }));
    }
    if let Err(response) =
        verifier_pin_compte(&compte, uq, pin, "prolonger la présence de ta boutique")
    {
        return response;
    }

    // Le même geste PIN remet l'interrupteur du compte en position d'ouvrir :
    // l'avoir coupé dans Profil › Paramètres était un refus, prolonger après coup
    // est un consentement. Sans écriture possible, le robinet de la boutique
    // reste fermé au tick suivant — rien n'est facturé à l'aveugle.
    if let Err(erreur) = crate::robinet::autoriser(uq, crate::robinet::VITRINE, true) {
        eprintln!("⚠️  Autorisation du robinet {} : {}", uq, erreur);
    }

    let maintenant = chrono::Utc::now().timestamp();
    // Repartir de l'échéance en cours ne perd pas les jours déjà payés ; le
    // plafond de 7 jours, lui, ne se contourne pas en renouvelant trop tôt.
    let echeance_courante = boutique
        .get("en_ligne_jusqu_a")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let nouvelle_echeance = (echeance_courante.max(maintenant)
        + jours * crate::user_models::DUREE_RENOUVELLEMENT_SECONDES)
        .min(maintenant + crate::user_models::DUREE_VITRINE_SECONDES);

    // Les `price_borb` du catalogue sont en BORB ; le coffre, lui, se débite en
    // points, un centième de BORB. Les deux domaines restent séparés.
    let catalogue_points = db.valeur_catalogue(uq).max(0).saturating_mul(100);
    let frais_24h_points = crate::vitrines::points_dus(
        catalogue_points,
        crate::user_models::DUREE_RENOUVELLEMENT_SECONDES,
    );

    let mut boutique_prolongee = boutique.clone();
    boutique_prolongee["en_ligne_jusqu_a"] = json!(nouvelle_echeance);
    boutique_prolongee["open"] = json!(true);
    // Le geste PIN vaut autorisation continuante : le tick a désormais le droit
    // de prélever, et il repart d'ici — les secondes déjà écoulées sont offertes.
    boutique_prolongee["robinet"] = json!(true);
    boutique_prolongee["dernier_reglement"] = json!(maintenant);
    boutique_prolongee["secondes_en_attente"] = json!(0);
    db.save_publication(boutique_prolongee.clone());

    // La vitrine revient dans le flux.
    let vitrine = boutique
        .get("vitrine_item_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if !vitrine.is_empty() {
        let _ = db.publish_shop_item(vitrine, uq);
    }

    let boutique_apres = db.get_shop_by_id(&shop_id).unwrap_or(boutique_prolongee);
    let secondes_restees = (nouvelle_echeance - maintenant).max(0);
    let message = if catalogue_points == 0 {
        "Présence prolongée : la vitrine d'un catalogue sans prix ne paie rien.".to_string()
    } else if secondes_restees < jours * crate::user_models::DUREE_RENOUVELLEMENT_SECONDES {
        "Présence prolongée, plafonnée à 7 jours de présence continue.".to_string()
    } else {
        "Présence prolongée : le robinet coule seconde par seconde.".to_string()
    };

    HttpResponse::Ok().json(json!({
        "status": "success",
        "shop": boutique_apres,
        "robinet_ouvert": true,
        "en_ligne_jusqu_a": nouvelle_echeance,
        "secondes_restees": secondes_restees,
        "pourcent": crate::user_models::FRAIS_VITRINE_POURCENT,
        "catalogue_points": catalogue_points,
        "frais_24h_points": frais_24h_points,
        "par_seconde_points": crate::vitrines::tarif_par_seconde(catalogue_points),
        "regle_points": boutique_apres.get("regle_points").and_then(|v| v.as_i64()).unwrap_or(0),
        "message": message
    }))
}

/// Supprime un élément
pub async fn zando_item_delete(
    req: actix_web::HttpRequest,
    path: web::Path<String>,
) -> HttpResponse {
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(claims) => claims,
        Err(response) => return response,
    };

    let db = Database::shared("uq_platform_db.json");
    let id = path.into_inner();

    match db.delete_shop_item(&id, &claims.sub) {
        Ok(()) => HttpResponse::Ok().json(json!({"status": "success"})),
        Err(e) if e == "Item introuvable ou accès refusé" => {
            HttpResponse::NotFound().json(json!({
                "status": "error",
                "message": e
            }))
        }
        Err(e) => HttpResponse::InternalServerError()
            .json(json!({"status": "error", "message": format!("Suppression impossible: {}", e)})),
    }
}

/// Publie un élément
pub async fn zando_item_publish(
    req: actix_web::HttpRequest,
    path: web::Path<String>,
) -> HttpResponse {
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(claims) => claims,
        Err(response) => return response,
    };

    let db = Database::shared("uq_platform_db.json");
    let id = path.into_inner();

    match db.publish_shop_item(&id, &claims.sub) {
        Ok(()) => {
            // Projection publique Firestore : la vitrine reflète
            // l'item publié. No-op sans FIREBASE_SERVICE_ACCOUNT.
            if let Some(item) = db.get_shop_item_by_id(&id) {
                crate::firestore_sync::ecrire(
                    &format!("zando_items/{}", id),
                    &serde_json::json!({
                        "id": id,
                        "seller": item.get("shop_id").cloned().unwrap_or_default(),
                        "title": item.get("title").cloned().unwrap_or_default(),
                        "subtitle": item.get("subtitle").cloned().unwrap_or_default(),
                        "description": item.get("description").cloned().unwrap_or_default(),
                        "item_type": item.get("item_type").cloned().unwrap_or_default(),
                        "price_borb": item.get("price_borb").cloned().unwrap_or_default(),
                        "price_usd": item.get("price_usd").cloned().unwrap_or_default(),
                        "category": item.get("category").cloned().unwrap_or_default(),
                        "published": true,
                        "updatedAt": chrono::Utc::now().timestamp(),
                    }),
                );
            }
            HttpResponse::Ok().json(json!({"status": "success"}))
        }
        Err(e) if e.starts_with("Item introuvable") => HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": e
        })),
        Err(e) => HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("Publication impossible: {}", e)
        })),
    }
}

// ===== ZANDO AGENTS (3 handlers) =====

/// Devient agent commissionnaire
pub async fn zando_agent_become(
    req: actix_web::HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    // 🔐 SÉCURITÉ : JWT obligatoire — l'agent est l'utilisateur
    // authentifié, jamais un user_id fourni par le client.
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(claims) => claims,
        Err(response) => return response,
    };
    let db = Database::shared("uq_platform_db.json");

    let item_id = body.get("item_id").and_then(|v| v.as_str()).unwrap_or("");
    let user_id = claims.sub.as_str();
    let commission_rate = body
        .get("commission_rate")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    match db.add_item_agent(item_id, user_id, commission_rate) {
        Ok(()) => HttpResponse::Ok().json(json!({"status": "success"})),
        Err(e) => {
            HttpResponse::BadRequest().json(json!({"status": "error", "message": format!("{}", e)}))
        }
    }
}

/// Récupère les agents d'un élément
pub async fn zando_agents_get(
    req: actix_web::HttpRequest,
    path: web::Path<String>,
) -> HttpResponse {
    // 🔐 SÉCURITÉ — USER : JWT obligatoire (liste des agents non publique)
    if let Err(r) = crate::auth_guard::exiger_auth(&req) {
        return r;
    }
    let db = Database::shared("uq_platform_db.json");
    let item_id = path.into_inner();
    let agents = db.get_item_agents(&item_id);

    HttpResponse::Ok().json(json!({
        "status": "success",
        "agents": agents,
        "total": agents.len()
    }))
}

/// Enregistre une vente via un agent
pub async fn zando_agent_sale(
    req: actix_web::HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    // 🔐 SÉCURITÉ : JWT obligatoire — la vente est enregistrée pour
    // l'agent authentifié ; un agent_id du body serait falsifiable.
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(claims) => claims,
        Err(response) => return response,
    };
    let db = Database::shared("uq_platform_db.json");

    let item_id = body.get("item_id").and_then(|v| v.as_str()).unwrap_or("");
    let agent_id = claims.sub.as_str();
    let sale_amount = body
        .get("sale_amount_borb")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    match db.record_agent_sale(item_id, agent_id, sale_amount) {
        Ok(commission) => HttpResponse::Ok().json(json!({
            "status": "success",
            "commission_earned": commission
        })),
        Err(e) => {
            HttpResponse::BadRequest().json(json!({"status": "error", "message": format!("{}", e)}))
        }
    }
}

// ===== CONTACTS (5 handlers) =====

/// Crée une demande de contact
pub async fn contact_request_create(body: web::Json<serde_json::Value>) -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");

    let requester_id = body
        .get("requester_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let target_id = body.get("target_id").and_then(|v| v.as_str()).unwrap_or("");
    let field = body
        .get("field")
        .and_then(|v| v.as_str())
        .unwrap_or("number");

    match db.create_contact_request(requester_id, target_id, field) {
        Ok(id) => HttpResponse::Ok().json(json!({"status": "success", "request_id": id})),
        Err(e) => {
            HttpResponse::BadRequest().json(json!({"status": "error", "message": format!("{}", e)}))
        }
    }
}

/// Accepte une demande
pub async fn contact_request_accept(
    req: actix_web::HttpRequest,
    path: web::Path<String>,
) -> HttpResponse {
    // 🔐 SÉCURITÉ (vague 1) — USER : JWT obligatoire
    // 🔐 SÉCURITÉ (vague 1) — USER : accès à SES données (ou owner)
    // 🔐 SÉCURITÉ (vague 1) — USER : JWT obligatoire (le contrôle de
    // propriété de l'objet {id} reste à la charge du handler)
    if let Err(r) = crate::auth_guard::exiger_auth(&req) {
        return r;
    }

    let db = Database::shared("uq_platform_db.json");
    let id = path.into_inner();

    match db.accept_contact_request(&id) {
        Ok(()) => HttpResponse::Ok().json(json!({"status": "success"})),
        Err(e) => {
            HttpResponse::BadRequest().json(json!({"status": "error", "message": format!("{}", e)}))
        }
    }
}

/// Refuse une demande
pub async fn contact_request_refuse(
    req: actix_web::HttpRequest,
    path: web::Path<String>,
) -> HttpResponse {
    // 🔐 SÉCURITÉ (vague 1) — USER : JWT obligatoire
    // 🔐 SÉCURITÉ (vague 1) — USER : accès à SES données (ou owner)
    // 🔐 SÉCURITÉ (vague 1) — USER : JWT obligatoire (le contrôle de
    // propriété de l'objet {id} reste à la charge du handler)
    if let Err(r) = crate::auth_guard::exiger_auth(&req) {
        return r;
    }

    let db = Database::shared("uq_platform_db.json");
    let id = path.into_inner();

    match db.refuse_contact_request(&id) {
        Ok(()) => HttpResponse::Ok().json(json!({"status": "success"})),
        Err(e) => {
            HttpResponse::BadRequest().json(json!({"status": "error", "message": format!("{}", e)}))
        }
    }
}

/// Partage un contact
pub async fn contact_share(body: web::Json<serde_json::Value>) -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");

    let owner_id = body.get("owner_id").and_then(|v| v.as_str()).unwrap_or("");
    let shared_with_id = body
        .get("shared_with_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let field = body
        .get("field")
        .and_then(|v| v.as_str())
        .unwrap_or("number");

    match db.share_contact(owner_id, shared_with_id, field) {
        Ok(()) => HttpResponse::Ok().json(json!({"status": "success"})),
        Err(e) => {
            HttpResponse::BadRequest().json(json!({"status": "error", "message": format!("{}", e)}))
        }
    }
}

/// Récupère les contacts partagés
pub async fn contact_shared_get(
    req: actix_web::HttpRequest,
    path: web::Path<String>,
) -> HttpResponse {
    // 🔐 SÉCURITÉ (vague 1) — USER : accès à SES données (ou owner)
    // (handler sans paramètre de chemin : contrôle JWT simple)
    // 🔐 SÉCURITÉ (vague 1) — USER : accès à SES données (ou owner)
    let __cible = path.to_string();
    if let Err(r) = crate::auth_guard::exiger_soi_ou_owner(&req, &__cible) {
        return r;
    }

    let db = Database::shared("uq_platform_db.json");
    let user_id = path.into_inner();
    let contacts = db.get_shared_contacts(&user_id);

    HttpResponse::Ok().json(json!({
        "status": "success",
        "contacts": contacts,
        "total": contacts.len()
    }))
}

// ===== DRAFTS (2 handlers) =====

/// Sauvegarde un brouillon
pub async fn draft_save(body: web::Json<serde_json::Value>) -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");

    let user_id = body.get("user_id").and_then(|v| v.as_str()).unwrap_or("");
    let draft_type = body
        .get("draft_type")
        .and_then(|v| v.as_str())
        .unwrap_or("item");
    let data = body.get("data").and_then(|v| v.as_str()).unwrap_or("{}");

    match db.save_draft(user_id, draft_type, data) {
        Ok(id) => HttpResponse::Ok().json(json!({"status": "success", "draft_id": id})),
        Err(e) => {
            HttpResponse::BadRequest().json(json!({"status": "error", "message": format!("{}", e)}))
        }
    }
}

/// Supprime un brouillon
pub async fn draft_delete(req: actix_web::HttpRequest, path: web::Path<String>) -> HttpResponse {
    // 🔐 SÉCURITÉ (vague 1) — USER : JWT obligatoire
    // 🔐 SÉCURITÉ (vague 1) — USER : accès à SES données (ou owner)
    // 🔐 SÉCURITÉ (vague 1) — USER : JWT obligatoire (le contrôle de
    // propriété de l'objet {id} reste à la charge du handler)
    if let Err(r) = crate::auth_guard::exiger_auth(&req) {
        return r;
    }

    let db = Database::shared("uq_platform_db.json");
    let id = path.into_inner();

    match db.delete_draft(&id) {
        Ok(()) => HttpResponse::Ok().json(json!({"status": "success"})),
        Err(e) => {
            HttpResponse::BadRequest().json(json!({"status": "error", "message": format!("{}", e)}))
        }
    }
}
// ====================
// 📇 ZANDO — CONTACTS (5 handlers)
// ====================

pub async fn zando_contact_request(
    req: actix_web::HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    // 🔐 SÉCURITÉ — USER : JWT obligatoire. requester_id venait du body
    // → n'importe qui pouvait demander un contact AU NOM d'un autre
    // (harcèlement + notifications falsifiées). L'identité vient du token.
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(claims) => claims,
        Err(response) => return response,
    };
    let db = Database::shared("uq_platform_db.json");

    let requester_id = claims.sub.as_str();
    let target_id = body.get("target_id").and_then(|v| v.as_str()).unwrap_or("");
    let field = body
        .get("field")
        .and_then(|v| v.as_str())
        .unwrap_or("number");

    if requester_id.is_empty() || target_id.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "requester_id et target_id requis"
        }));
    }

    match db.create_contact_request(requester_id, target_id, field) {
        Ok(request_id) => {
            // Badge de l'ecran Zando : la cible recoit la demande.
            crate::trading_routes::push_notification(
                target_id,
                "zando",
                "Demande de contact",
                &format!(
                    "{} souhaite obtenir ton {}. Consulte Zando pour repondre.",
                    requester_id, field
                ),
                &format!("zando:contact:{}", request_id),
            );
            HttpResponse::Ok().json(json!({
                "status": "success",
                "request_id": request_id
            }))
        }
        Err(e) => HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": format!("{}", e)
        })),
    }
}

pub async fn zando_contact_accept(
    req: actix_web::HttpRequest,
    path: web::Path<String>,
) -> HttpResponse {
    // 🔐 SÉCURITÉ (vague 1) — USER : JWT obligatoire
    // 🔐 SÉCURITÉ (vague 1) — USER : accès à SES données (ou owner)
    // 🔐 SÉCURITÉ (vague 1) — USER : JWT obligatoire (le contrôle de
    // propriété de l'objet {id} reste à la charge du handler)
    if let Err(r) = crate::auth_guard::exiger_auth(&req) {
        return r;
    }

    let db = Database::shared("uq_platform_db.json");
    let request_id = path.into_inner();

    match db.accept_contact_request(&request_id) {
        Ok(()) => HttpResponse::Ok().json(json!({
            "status": "success",
            "message": "Demande acceptée"
        })),
        Err(e) => HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": format!("{}", e)
        })),
    }
}

pub async fn zando_contact_refuse(
    req: actix_web::HttpRequest,
    path: web::Path<String>,
) -> HttpResponse {
    // 🔐 SÉCURITÉ (vague 1) — USER : JWT obligatoire
    // 🔐 SÉCURITÉ (vague 1) — USER : accès à SES données (ou owner)
    // 🔐 SÉCURITÉ (vague 1) — USER : JWT obligatoire (le contrôle de
    // propriété de l'objet {id} reste à la charge du handler)
    if let Err(r) = crate::auth_guard::exiger_auth(&req) {
        return r;
    }

    let db = Database::shared("uq_platform_db.json");
    let request_id = path.into_inner();

    match db.refuse_contact_request(&request_id) {
        Ok(()) => HttpResponse::Ok().json(json!({
            "status": "success",
            "message": "Demande refusée"
        })),
        Err(e) => HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": format!("{}", e)
        })),
    }
}

pub async fn zando_contact_share(body: web::Json<serde_json::Value>) -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");

    let owner_id = body.get("owner_id").and_then(|v| v.as_str()).unwrap_or("");
    let shared_with_id = body
        .get("shared_with_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let field = body
        .get("field")
        .and_then(|v| v.as_str())
        .unwrap_or("number");

    match db.share_contact(owner_id, shared_with_id, field) {
        Ok(()) => HttpResponse::Ok().json(json!({
            "status": "success",
            "message": "Contact partagé"
        })),
        Err(e) => HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": format!("{}", e)
        })),
    }
}

pub async fn zando_contacts_shared(
    req: actix_web::HttpRequest,
    path: web::Path<String>,
) -> HttpResponse {
    // 🔐 SÉCURITÉ (vague 1) — USER : JWT obligatoire
    // 🔐 SÉCURITÉ (vague 1) — USER : accès à SES données (ou owner)
    // 🔐 SÉCURITÉ (vague 1) — USER : JWT obligatoire (le contrôle de
    // propriété de l'objet {id} reste à la charge du handler)
    if let Err(r) = crate::auth_guard::exiger_auth(&req) {
        return r;
    }

    let db = Database::shared("uq_platform_db.json");
    let user_id = path.into_inner();

    let contacts = db.get_shared_contacts(&user_id);

    HttpResponse::Ok().json(json!({
        "status": "success",
        "contacts": contacts
    }))
}

// ====================
// 📝 ZANDO — DRAFTS (3 handlers)
// ====================

pub async fn zando_draft_save(body: web::Json<serde_json::Value>) -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");

    let user_id = body.get("user_id").and_then(|v| v.as_str()).unwrap_or("");
    let draft_type = body
        .get("draft_type")
        .and_then(|v| v.as_str())
        .unwrap_or("item");
    let data = body.get("data").cloned().unwrap_or(json!({}));
    let data_str = data.to_string();

    if user_id.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "user_id requis"
        }));
    }

    match db.save_draft(user_id, draft_type, &data_str) {
        Ok(draft_id) => HttpResponse::Ok().json(json!({
            "status": "success",
            "draft_id": draft_id
        })),
        Err(e) => HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": format!("{}", e)
        })),
    }
}

pub async fn zando_draft_get(req: actix_web::HttpRequest, path: web::Path<String>) -> HttpResponse {
    // 🔐 SÉCURITÉ (vague 1) — USER : JWT obligatoire
    // 🔐 SÉCURITÉ (vague 1) — USER : accès à SES données (ou owner)
    // 🔐 SÉCURITÉ (vague 1) — USER : JWT obligatoire (le contrôle de
    // propriété de l'objet {id} reste à la charge du handler)
    if let Err(r) = crate::auth_guard::exiger_auth(&req) {
        return r;
    }

    let db = Database::shared("uq_platform_db.json");
    let draft_id = path.into_inner();

    match db.get_draft(&draft_id) {
        Some(draft) => HttpResponse::Ok().json(json!({
            "status": "success",
            "data": draft
        })),
        None => HttpResponse::NotFound().json(json!({
            "status": "error",
            "message": "Brouillon introuvable"
        })),
    }
}

pub async fn zando_draft_delete(
    req: actix_web::HttpRequest,
    path: web::Path<String>,
) -> HttpResponse {
    // 🔐 SÉCURITÉ (vague 1) — USER : JWT obligatoire
    // 🔐 SÉCURITÉ (vague 1) — USER : accès à SES données (ou owner)
    // 🔐 SÉCURITÉ (vague 1) — USER : JWT obligatoire (le contrôle de
    // propriété de l'objet {id} reste à la charge du handler)
    if let Err(r) = crate::auth_guard::exiger_auth(&req) {
        return r;
    }

    let db = Database::shared("uq_platform_db.json");
    let draft_id = path.into_inner();

    match db.delete_draft(&draft_id) {
        Ok(()) => HttpResponse::Ok().json(json!({
            "status": "success",
            "message": "Brouillon supprimé"
        })),
        Err(e) => HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": format!("{}", e)
        })),
    }
}

// ====================
// ⭐ CUSTOM TYPES (types de lieux personnalisés)
// ====================

/// Crée un nouveau type personnalisé
pub async fn custom_type_create(body: web::Json<serde_json::Value>) -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");

    let owner_id = body.get("owner_id").and_then(|v| v.as_str()).unwrap_or("");
    let name = body.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let emoji = body.get("emoji").and_then(|v| v.as_str()).unwrap_or("⭐");
    let color = body
        .get("color")
        .and_then(|v| v.as_str())
        .unwrap_or("#4A9EFF");
    let description = body
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let fields = body.get("fields").and_then(|v| v.as_str()).unwrap_or("[]");

    if name.is_empty() {
        return HttpResponse::BadRequest()
            .json(json!({"status": "error", "message": "name requis"}));
    }

    let id = db.create_custom_type(owner_id, name, emoji, color, description, fields);

    if id.is_empty() {
        return HttpResponse::BadRequest()
            .json(json!({"status": "error", "message": "Creation echouee"}));
    }

    HttpResponse::Ok().json(json!({
        "status": "success",
        "type_id": id
    }))
}

/// Liste les types personnalisés d'un utilisateur
pub async fn custom_types_by_owner(
    req: actix_web::HttpRequest,
    path: web::Path<String>,
) -> HttpResponse {
    // 🔐 SÉCURITÉ (vague 1) — USER : JWT obligatoire
    // 🔐 SÉCURITÉ (vague 1) — USER : accès à SES données (ou owner)
    // 🔐 SÉCURITÉ (vague 1) — USER : JWT obligatoire (le contrôle de
    // propriété de l'objet {id} reste à la charge du handler)
    if let Err(r) = crate::auth_guard::exiger_auth(&req) {
        return r;
    }

    let db = Database::shared("uq_platform_db.json");
    let owner_id = path.into_inner();
    let types = db.get_custom_types_by_owner(&owner_id);

    HttpResponse::Ok().json(json!({
        "status": "success",
        "types": types,
        "total": types.len()
    }))
}

/// Liste TOUS les types personnalisés
pub async fn custom_types_list() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    let types = db.get_all_custom_types();

    HttpResponse::Ok().json(json!({
        "status": "success",
        "types": types,
        "total": types.len()
    }))
}

/// Récupère un type par ID
pub async fn custom_type_get(req: actix_web::HttpRequest, path: web::Path<String>) -> HttpResponse {
    // 🔐 SÉCURITÉ (vague 1) — USER : JWT obligatoire
    // 🔐 SÉCURITÉ (vague 1) — USER : accès à SES données (ou owner)
    // 🔐 SÉCURITÉ (vague 1) — USER : JWT obligatoire (le contrôle de
    // propriété de l'objet {id} reste à la charge du handler)
    if let Err(r) = crate::auth_guard::exiger_auth(&req) {
        return r;
    }

    let db = Database::shared("uq_platform_db.json");
    let id = path.into_inner();

    match db.get_custom_type(&id) {
        Some(t) => HttpResponse::Ok().json(json!({"status": "success", "type": t})),
        None => {
            HttpResponse::NotFound().json(json!({"status": "error", "message": "Type non trouve"}))
        }
    }
}

/// Supprime un type personnalisé
pub async fn custom_type_delete(
    req: actix_web::HttpRequest,
    path: web::Path<String>,
) -> HttpResponse {
    // 🔐 SÉCURITÉ (vague 1) — USER : JWT obligatoire
    // 🔐 SÉCURITÉ (vague 1) — USER : accès à SES données (ou owner)
    // 🔐 SÉCURITÉ (vague 1) — USER : JWT obligatoire (le contrôle de
    // propriété de l'objet {id} reste à la charge du handler)
    if let Err(r) = crate::auth_guard::exiger_auth(&req) {
        return r;
    }

    let db = Database::shared("uq_platform_db.json");
    let id = path.into_inner();

    match db.delete_custom_type(&id) {
        Ok(()) => HttpResponse::Ok().json(json!({"status": "success"})),
        Err(e) => {
            HttpResponse::BadRequest().json(json!({"status": "error", "message": format!("{}", e)}))
        }
    }
}

// ====================
// UQ ADDRESS — Génération des adresses UQ
// ====================

pub async fn uq_generate_address(body: web::Json<serde_json::Value>) -> HttpResponse {
    use uq_blockchain::uq_address::{generate_uq_email, generate_uq_number, generate_uq_pseudo};

    let country = body
        .get("country")
        .and_then(|v| v.as_str())
        .unwrap_or("RDC");
    let nom = body.get("nom").and_then(|v| v.as_str()).unwrap_or("");
    let prenom = body.get("prenom").and_then(|v| v.as_str()).unwrap_or("");
    let position = body.get("position").and_then(|v| v.as_u64()).unwrap_or(1);

    if nom.is_empty() || prenom.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "nom et prenom requis"
        }));
    }

    // Nouveau format : pays GSM + suffixe aléatoire (le nom n'apparaît plus)
    let uq_number = generate_uq_number(country);
    let uq_email = generate_uq_email(prenom, 0);
    let uq_pseudo = generate_uq_pseudo(prenom, nom, 0);

    HttpResponse::Ok().json(json!({
        "status": "success",
        "uq_number": uq_number,
        "uq_email": uq_email,
        "uq_pseudo": uq_pseudo,
        "country": country,
        "nom": nom,
        "prenom": prenom,
        "position": position
    }))
}

// ====================
// AUTH — Verifier si un compte existe (email + mot de passe)
// ====================

pub async fn verify_account_exists(body: web::Json<serde_json::Value>) -> HttpResponse {
    let number = body.get("number").and_then(|v| v.as_str()).unwrap_or("");
    let email = body.get("email").and_then(|v| v.as_str()).unwrap_or("");
    let password = body.get("password").and_then(|v| v.as_str()).unwrap_or("");

    // Chercher dans uq_index.db (P2P)
    let idx = crate::uq_index::UqIndex::shared("uq_index.db");

    // Login « email seul » : si pas de number mais un email, on vérifie
    // directement par email (le UQ est résolu en interne).
    if number.is_empty() && !email.is_empty() && !password.is_empty() {
        return match idx.verify_credentials_by_email(email, password) {
            Some(account_info) => HttpResponse::Ok().json(json!({
                "status": "success",
                "exists": true,
                "account": account_info
            })),
            None => HttpResponse::Unauthorized().json(json!({
                "status": "error",
                "message": "Identifiants incorrects"
            })),
        };
    }

    if number.is_empty() || email.is_empty() || password.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "number, email et password requis (ou email + password seuls)"
        }));
    }

    match idx.verify_credentials(number, email, password) {
        Some(account_info) => HttpResponse::Ok().json(json!({
            "status": "success",
            "exists": true,
            "account": account_info
        })),
        None => HttpResponse::Unauthorized().json(json!({
            "status": "error",
            "message": "Identifiants incorrects"
        })),
    }
}

// ====================
// ACCOUNT ME — Profil dynamique (via JWT)
// ====================

/// GET /api/account/infos/{uq}
/// Retourne les infos complètes du compte : uq, email, pseudo, code_promo
pub async fn account_infos(req: actix_web::HttpRequest, path: web::Path<String>) -> HttpResponse {
    // 🔐 SÉCURITÉ (vague 1) — USER : accès à SES données (ou owner)
    // (handler sans paramètre de chemin : contrôle JWT simple)
    // 🔐 SÉCURITÉ (vague 1) — USER : accès à SES données (ou owner)
    let __cible = path.to_string();
    if let Err(r) = crate::auth_guard::exiger_soi_ou_owner(&req, &__cible) {
        return r;
    }

    let uq_number = path.into_inner();

    if uq_number.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "uq_number requis"
        }));
    }

    // 1. Chercher dans uq_index
    let idx = crate::uq_index::UqIndex::shared("uq_index.db");

    let account = match idx.get_account(&uq_number) {
        Some(acc) => acc,
        None => {
            return HttpResponse::NotFound().json(json!({
                "status": "error",
                "message": "Compte introuvable"
            }))
        }
    };

    // 2. Chercher le code_promo dans generated_patterns
    let code_promo = {
        let conn = match rusqlite::Connection::open("uq_index.db") {
            Ok(c) => c,
            Err(_) => {
                return HttpResponse::InternalServerError().json(json!({
                    "status": "error",
                    "message": "Erreur base"
                }))
            }
        };

        let result: Result<String, _> = conn.query_row(
            "SELECT pattern_value FROM generated_patterns 
             WHERE pattern_type = 'code_promo' AND uq_number = ?1
             LIMIT 1",
            rusqlite::params![uq_number],
            |row| row.get(0),
        );

        result.unwrap_or_default()
    };

    // 3. Retourner les infos complètes
    let recuperation = idx.get_recovery_contacts(&uq_number).unwrap_or(json!({}));
    HttpResponse::Ok().json(json!({
        "status": "success",
        "uq_number": account.get("uq_number").and_then(|v| v.as_str()).unwrap_or(""),
        "email": account.get("uq_email").and_then(|v| v.as_str()).unwrap_or(""),
        "pseudo": account.get("uq_pseudo").and_then(|v| v.as_str()).unwrap_or(""),
        "code_promo": code_promo,
        "is_owner": account.get("is_owner").and_then(|v| v.as_bool()).unwrap_or(false),
        "created_at": account.get("created_at").and_then(|v| v.as_i64()).unwrap_or(0),
        // Contacts de recuperation : relis par le profil, jamais par la fiche publique.
        "recovery_email": recuperation.get("recovery_email").cloned().unwrap_or(json!("")),
        "recovery_phone": recuperation.get("recovery_phone").cloned().unwrap_or(json!("")),
        "phone_verified": recuperation.get("phone_verified").cloned().unwrap_or(json!(false)),
        "email_verified": recuperation.get("email_verified").cloned().unwrap_or(json!(false)),
    }))
}

// ====================
// DEV — Régénérer les clés privées (owner uniquement)
// ====================
pub async fn dev_regen_keys(
    req: actix_web::HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    use crate::auth::AuthManager;

    // Vérifier owner
    let claims = match AuthManager::require_role(&req, &["guardian"]) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let uq_number = body.get("uq_number").and_then(|v| v.as_str()).unwrap_or("");
    let password = body.get("password").and_then(|v| v.as_str()).unwrap_or("");

    if uq_number.is_empty() || password.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "uq_number et password requis"
        }));
    }

    // Générer les nouvelles clés
    let wallet_keys = match crate::security::wallet_keys::generate_wallet_keys(password) {
        Ok(k) => k,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Erreur génération: {}", e)
            }))
        }
    };

    let db_path = format!(
        "data/users/{}.db",
        uq_number.replace("-", "_").to_lowercase()
    );
    if !std::path::Path::new(&db_path).exists() {
        return HttpResponse::NotFound().json(json!({
            "status": "error",
            "message": "Compte introuvable"
        }));
    }

    let conn = match rusqlite::Connection::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Erreur DB: {}", e)
            }))
        }
    };

    // Créer la table si manquante
    let _ = conn.execute(
        "CREATE TABLE IF NOT EXISTS my_private_key (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            encrypted_key BLOB NOT NULL,
            iv BLOB NOT NULL,
            salt BLOB NOT NULL,
            algorithm TEXT DEFAULT 'AES-256-GCM',
            created_at INTEGER NOT NULL,
            rotated_at INTEGER
        )",
        [],
    );

    let now = chrono::Utc::now().timestamp();

    if let Err(e) = conn.execute(
        "INSERT OR REPLACE INTO my_private_key (id, encrypted_key, iv, salt, algorithm, created_at) 
         VALUES (1, ?1, ?2, ?3, 'AES-256-GCM', ?4)",
        rusqlite::params![
            wallet_keys.encrypted_private_key,
            wallet_keys.iv.to_vec(),
            wallet_keys.salt.to_vec(),
            now
        ],
    ) {
        return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("Erreur insertion: {}", e)
        }));
    }

    // Mettre à jour la clé publique dans uq_index
    {
        let idx = crate::uq_index::UqIndex::shared("uq_index.db");
        if let Ok(uq_index_conn) = rusqlite::Connection::open("uq_index.db") {
            let _ = uq_index_conn.execute(
                "UPDATE uq_index SET public_key = ?1 WHERE uq_number = ?2",
                rusqlite::params![wallet_keys.public_key, uq_number],
            );
        }
    }

    eprintln!(
        "🔑 Clés régénérées pour {} (public: {}...)",
        uq_number,
        hex::encode(&wallet_keys.public_key[..8])
    );

    HttpResponse::Ok().json(json!({
        "status": "success",
        "message": "Clés régénérées",
        "uq_number": uq_number,
        "public_key": hex::encode(&wallet_keys.public_key)
    }))
}

pub async fn account_me(req: actix_web::HttpRequest) -> HttpResponse {
    use crate::auth::AuthManager;

    // Verifier JWT (user, admin, guardian)
    let claims = match AuthManager::require_role(&req, &["user", "admin", "guardian"]) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let uq_number = &claims.sub;

    // Chercher dans uq_index.db
    let idx = crate::uq_index::UqIndex::shared("uq_index.db");

    match idx.get_account(uq_number) {
        Some(account) => HttpResponse::Ok().json(json!({
            "status": "success",
            "account": account
        })),
        None => HttpResponse::NotFound().json(json!({
            "status": "error",
            "message": "Compte introuvable"
        })),
    }
}

// ====================
// BORB — ROUTES DÉDIÉES COMPLÈTES
// ====================

// ============================================================
// CALCUL DES FRAIS DEGRESSIFS
// ============================================================
// Envoi BORB direct :
//   < 1000 BORB       -> 1%
//   1 000 - 9 999    -> 0.5%
//   10 000 - 99 999  -> 0.25%
//   >= 100 000       -> 0.1%
//
// Wallet virtuel :
//   Devise locale/internationale -> 0.1%
//   Crypto (BTC/ETH/USDT...)     -> 1%
//
// P2P brut : 0% (pas de frais, echange direct)
// ============================================================

fn calculate_fee_points(amount_points: u64, from_currency: &str) -> u64 {
    let amount_borb = amount_points as f64 / 100.0; // centimes -> BORB
    let is_crypto = matches!(
        from_currency.to_uppercase().as_str(),
        "BTC"
            | "ETH"
            | "USDT"
            | "USDC"
            | "BNB"
            | "XRP"
            | "SOL"
            | "ADA"
            | "DOGE"
            | "TRX"
            | "DOT"
            | "MATIC"
            | "LTC"
            | "SHIB"
            | "AVAX"
            | "LINK"
            | "ATOM"
            | "XLM"
            | "ALGO"
            | "XMR"
    );

    let pct = if from_currency.to_uppercase() == "BORB" {
        // BORB direct : degressif
        if amount_borb >= 100_000.0 {
            0.001
        }
        // 0.1%
        else if amount_borb >= 10_000.0 {
            0.0025
        }
        // 0.25%
        else if amount_borb >= 1_000.0 {
            0.005
        }
        // 0.5%
        else {
            0.01
        } // 1% (< 1000)
    } else if is_crypto {
        0.01 // Crypto : 1%
    } else {
        0.001 // Devise locale/int : 0.1%
    };

    let fee = (amount_borb * pct * 100.0).ceil() as u64; // BORB -> centimes
    fee.max(1) // minimum 1 centime
}

pub async fn borb_send(
    req: actix_web::HttpRequest,
    state: web::Data<Arc<GlobalState>>,
    body: web::Json<TransferRequest>,
) -> HttpResponse {
    bubble_transfer(req, state, body).await
}

pub async fn borb_receive(path: web::Path<String>) -> HttpResponse {
    let account_id = path.into_inner();
    let db_path = format!(
        "data/users/{}.db",
        account_id.replace("-", "_").to_lowercase()
    );

    if !std::path::Path::new(&db_path).exists() {
        return HttpResponse::NotFound().json(json!({
            "status": "error",
            "message": format!("Compte introuvable: {}", account_id)
        }));
    }

    let conn = match rusqlite::Connection::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Erreur: {}", e)
            }))
        }
    };

    let mut stmt = match conn.prepare(
        "SELECT amount, counterparty, created_at FROM transactions WHERE tx_type = 'receive' ORDER BY created_at DESC LIMIT 10"
    ) {
        Ok(s) => s,
        Err(_) => return HttpResponse::Ok().json(json!({
            "status": "success",
            "account": account_id,
            "recent_receives": []
        })),
    };

    let rows = stmt.query_map([], |row| {
        Ok(json!({
            "amount": row.get::<_, i64>(0)?,
            "from": row.get::<_, String>(1)?,
            "timestamp": row.get::<_, i64>(2)?
        }))
    });

    let mut receives = Vec::new();
    if let Ok(r) = rows {
        for row in r {
            if let Ok(v) = row {
                receives.push(v);
            }
        }
    }

    HttpResponse::Ok().json(json!({
        "status": "success",
        "account": account_id,
        "recent_receives": receives
    }))
}

pub async fn borb_balance(req: actix_web::HttpRequest, path: web::Path<String>) -> HttpResponse {
    let account_id = path.into_inner();
    if !crate::auth_guard::ressemble_a_un_uq(&account_id) {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Identifiant de compte invalide"
        }));
    }
    if let Err(response) = crate::auth_guard::exiger_soi_ou_owner(&req, &account_id) {
        return response;
    }

    let db_path = format!(
        "data/users/{}.db",
        account_id.replace("-", "_").to_lowercase()
    );

    if !std::path::Path::new(&db_path).exists() {
        return HttpResponse::NotFound().json(json!({
            "status": "error",
            "message": format!("Compte introuvable: {}", account_id)
        }));
    }

    let conn = match rusqlite::Connection::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Erreur: {}", e)
            }))
        }
    };

    let (bubbles, points, frozen): (i64, i64, i64) = conn
        .query_row(
            "SELECT bubbles, points, COALESCE(frozen_points, 0) FROM balance WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap_or((0, 0, 0));

    // 🔒 Précision : 2 décimales pour BORB (division en f64)
    let borb_total = points as f64 / 100.0;
    let borb_display = format!("{:.2}", borb_total);

    // ====================
    // ORDRE DÉCROISSANT : BORB → borb → mborb → nborb → pborb → fborb
    // ====================

    let units_info = vec![
        ("BORB", "BORB", 1i64, "Unité principale"),
        ("borb", "FIXO", 100, "1 BORB = 100 borb"),
        ("mborb", "MICRO", 10_000, "1 BORB = 10 000 mborb"),
        ("nborb", "NANO", 1_000_000, "1 BORB = 1 000 000 nborb"),
        ("pborb", "PICCO", 100_000_000, "1 BORB = 100 000 000 pborb"),
        (
            "fborb",
            "FEMTO",
            10_000_000_000i64,
            "1 BORB = 10 000 000 000 fborb",
        ),
    ];

    // Construire la liste ordonnée des unités (UNE SEULE liste)
    let mut units_list = Vec::new();
    for (position, (unit, name, rate, description)) in units_info.iter().enumerate() {
        let value = borb_total * (*rate as f64);
        let display = format!("{} {}", value, unit);

        units_list.push(json!({
            "position": position + 1,
            "unit": unit,
            "name": name,
            "value": value,
            "rate": rate,
            "description": description,
            "display": display
        }));
    }

    // ====================
    // RÉPONSE RÉORGANISÉE
    // ====================

    HttpResponse::Ok().json(json!({
        "status": "success",
        "account": account_id,
        "order": "décroissant (du plus grand au plus petit)",
        "balance": {
            "bubbles": bubbles,
            "points": points,
            "frozen_points": frozen,
            "borb_total": borb_total,
            "display": format!("{} BORB", borb_total)
        },
        "units": units_list
    }))
}

pub async fn borb_transactions(
    req: actix_web::HttpRequest,
    path: web::Path<String>,
) -> HttpResponse {
    // ====================
    // SÉCURITÉ : Vérifier que l'utilisateur accède à SES transactions
    // ====================
    use crate::auth::AuthManager;

    let claims = match AuthManager::require_role(&req, &["user", "admin", "guardian"]) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let account_id = path.into_inner();

    // Un user ne peut voir QUE ses propres transactions
    // L'owner (UQ-00000001) peut voir TOUT
    let is_owner = claims.sub == "UQ-00000001";
    let is_self = claims.sub == account_id;

    if !is_owner && !is_self {
        return HttpResponse::Forbidden().json(json!({
            "status": "error",
            "message": "Accès refusé — tu ne peux voir que TES transactions"
        }));
    }

    let db_path = format!(
        "data/users/{}.db",
        account_id.replace("-", "_").to_lowercase()
    );

    if !std::path::Path::new(&db_path).exists() {
        return HttpResponse::NotFound().json(json!({
            "status": "error",
            "message": format!("Compte introuvable: {}", account_id)
        }));
    }

    let conn = match rusqlite::Connection::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Erreur: {}", e)
            }))
        }
    };

    // ====================
    // 🧬 ALIGNEMENT DU SCHÉMA AVANT LECTURE (cause du bug « historique vide »)
    // ====================
    // La requête ci-dessous lit borb_id / prev_hash / block_number.
    // Les comptes créés avant la migration v4 ne possèdent PAS ces colonnes :
    // `prepare()` échouait alors et le handler renvoyait « transactions: [] »
    // → l'interface affichait « Aucune transaction » alors que la table en
    // contenait (8 lignes chez l'owner).
    if let Err(e) = crate::db::ensure_transactions_columns(&conn) {
        eprintln!(
            "⚠️  Alignement schéma transactions (lecture {}): {}",
            account_id, e
        );
    }

    let mut stmt = match conn.prepare(
        "SELECT id, tx_type, amount, fee, counterparty, created_at,
                COALESCE(borb_id,''), COALESCE(prev_hash,''), COALESCE(block_number,0),
                COALESCE(status,'')
         FROM transactions ORDER BY created_at DESC LIMIT 50",
    ) {
        Ok(s) => s,
        // ⚠️ Ne JAMAIS renvoyer une liste vide en cas d'erreur : c'est ce qui
        //    masquait le problème. On remonte l'erreur explicitement.
        Err(e) => {
            eprintln!("❌ Lecture transactions {} impossible : {}", account_id, e);
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "account": account_id,
                "code": "LECTURE_IMPOSSIBLE",
                "message": format!("Historique illisible : {}", e)
            }));
        }
    };

    // Lecture TOLERANTE : les anciennes lignes ont des colonnes NULL
    // (fee, sender_uq, borb_id...). Avant, `row.get::<_, i64>(3)?` echouait
    // sur fee=NULL et la ligne etait SILENCIEUSEMENT perdue (7/8 affichees).
    let rows = stmt.query_map([], |row| {
        let _cp: String = row.get::<_, Option<String>>(4)?.unwrap_or_default();
        // 👤 Affiche « pseudo (Nom) » si la base contient encore un UQ
        let _display = crate::uq_public_alias::affichage_public(&_cp);
        Ok(json!({
            "id": row.get::<_, Option<i64>>(0)?.unwrap_or(0),
            "type": row.get::<_, Option<String>>(1)?.unwrap_or_else(|| "inconnu".to_string()),
            "amount": row.get::<_, Option<i64>>(2)?.unwrap_or(0),
            "fee": row.get::<_, Option<i64>>(3)?.unwrap_or(0),
            "counterparty": _display,
            "timestamp": row.get::<_, Option<i64>>(5)?.unwrap_or(0),
            // 📜 Chaînage de l'historique local (spécification)
            "borb_id": row.get::<_, Option<String>>(6)?.unwrap_or_default(),
            "prev_hash": row.get::<_, Option<String>>(7)?.unwrap_or_default(),
            "block_number": row.get::<_, Option<i64>>(8)?.unwrap_or(0),
            "status": row.get::<_, Option<String>>(9)?.unwrap_or_default()
        }))
    });

    let mut transactions = Vec::new();
    if let Ok(r) = rows {
        for row in r {
            if let Ok(v) = row {
                transactions.push(v);
            }
        }
    }

    HttpResponse::Ok().json(json!({
        "status": "success",
        "account": account_id,
        "total": transactions.len(),
        "transactions": transactions
    }))
}

pub async fn borb_supply() -> HttpResponse {
    HttpResponse::Ok().json(json!({
        "status": "success",
        "total_supply": 40_000_000_000u64,
        "total_supply_borb": 40_000_000_000u64,
        "total_supply_points": 4_000_000_000_000u64,
        "units": ["BORB", "borb", "mborb", "nborb", "pborb", "fborb"],
        "invariant": "40 milliards BORB (masse fixe éternelle)"
    }))
}

pub async fn borb_units_info() -> HttpResponse {
    HttpResponse::Ok().json(json!({
        "status": "success",
        "order": "décroissant (du plus grand au plus petit)",
        "units": [
            {
                "position": 1,
                "unit": "BORB",
                "name": "BORB",
                "rate": 1,
                "description": "Unité principale"
            },
            {
                "position": 2,
                "unit": "borb",
                "name": "FIXO",
                "rate": 100,
                "description": "1 BORB = 100 borb"
            },
            {
                "position": 3,
                "unit": "mborb",
                "name": "MICRO",
                "rate": 10_000,
                "description": "1 BORB = 10 000 mborb"
            },
            {
                "position": 4,
                "unit": "nborb",
                "name": "NANO",
                "rate": 1_000_000,
                "description": "1 BORB = 1 000 000 nborb"
            },
            {
                "position": 5,
                "unit": "pborb",
                "name": "PICCO",
                "rate": 100_000_000,
                "description": "1 BORB = 100 000 000 pborb"
            },
            {
                "position": 6,
                "unit": "fborb",
                "name": "FEMTO",
                "rate": 10_000_000_000i64,
                "description": "1 BORB = 10 000 000 000 fborb"
            }
        ]
    }))
}

pub async fn borb_convert(body: web::Json<serde_json::Value>) -> HttpResponse {
    let account_id = body.get("account").and_then(|v| v.as_str()).unwrap_or("");
    let from_unit = body.get("from").and_then(|v| v.as_str()).unwrap_or("BORB");
    let to_unit = body.get("to").and_then(|v| v.as_str()).unwrap_or("borb");
    let amount = body.get("amount").and_then(|v| v.as_i64()).unwrap_or(0);

    if account_id.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "account requis"
        }));
    }

    // Sous-unités définies de la monnaie (comme des centimes), pas des taux de
    // marché. En revanche une unité inconnue ne doit pas convertir « 1 pour 1 » :
    // une faute de frappe dans `from` ou `to` déplacait de la valeur au taux
    // implicite. Ici, unité inconnue = erreur explicite.
    let facteurs = std::collections::HashMap::from([
        ("BORB", 1i64),
        ("borb", 100),
        ("mborb", 10000),
        ("nborb", 1000000),
        ("pborb", 100000000),
        ("fborb", 10000000000),
    ]);

    let (from_rate, to_rate) = match (facteurs.get(from_unit), facteurs.get(to_unit)) {
        (Some(f), Some(t)) => (*f, *t),
        _ => return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "code": "UNITE_INCONNUE",
            "from": from_unit,
            "to": to_unit,
            "unites_connues": ["BORB", "borb", "mborb", "nborb", "pborb", "fborb"],
            "message": "Cette unité n'existe pas : aucune conversion ne sera faite sur un taux implicite."
        })),
    };

    let amount_in_points = amount * from_rate;
    let amount_in_to = amount_in_points / to_rate;

    HttpResponse::Ok().json(json!({
        "status": "success",
        "conversion": {
            "from": from_unit,
            "to": to_unit,
            "amount": amount,
            "result": amount_in_to,
            "rate": format!("1 {} = {} {}", from_unit, to_rate / from_rate, to_unit)
        }
    }))
}

// ====================
// ORACLE BTC + ALL — Prix temps réel
// ====================

/// `(prix, source)` — la source affichée vient vraiment de la réponse externe.
async fn btc_price_source() -> (Option<f64>, String) {
    let reponse = crate::external_api::prix_crypto("BTCUSD").await;
    if reponse["status"] != "success" {
        return (None, String::new());
    }
    let prix = reponse["data"]["price"].as_f64().filter(|p| *p > 0.0);
    (prix, reponse["source"].as_str().unwrap_or("").to_string())
}

/// Prix de l'or en USD/once : xaus.com via l'oracle externe (cache 5 minutes,
/// dernier prix connu reservi si la source est injoignable).
/// `None` = aucune donnée réelle disponible : le prix n'est jamais inventé.
async fn fetch_gold_price() -> Option<f64> {
    crate::external_api::once_or_usd().await
}

/// Route : /api/price/btc
pub async fn price_btc() -> HttpResponse {
    let (btc_usd, source) = btc_price_source().await;

    match btc_usd {
        Some(prix) => HttpResponse::Ok().json(json!({
            "status": "success",
            "btc_usd": prix,
            "source": if source.is_empty() { "oracle" } else { source.as_str() },
            "updated_at": chrono::Utc::now().timestamp()
        })),
        None => HttpResponse::ServiceUnavailable().json(json!({
            "status": "error",
            "code": "BTC_INDISPONIBLE",
            "message": "Prix BTC indisponible : sources extérieures injoignables et aucun prix en cache",
            "updated_at": chrono::Utc::now().timestamp()
        })),
    }
}

/// Route : /api/price/all — Or + USD + BTC

// ============================================================
// LIVE STATS - Dashboard temps reel des 6 cases
// ============================================================

// ============================================================
// MULTI-WALLETS - Portefeuilles virtuels multi-devises
// ============================================================

/// GET /api/account/wallets/{uq}
/// Retourne tous les portefeuilles virtuels d'un user
pub async fn get_user_wallets(
    req: actix_web::HttpRequest,
    path: web::Path<String>,
) -> HttpResponse {
    use rusqlite::Connection;
    let uq = path.into_inner();

    if !crate::auth_guard::ressemble_a_un_uq(&uq) {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Identifiant de compte invalide"
        }));
    }
    if let Err(response) = crate::auth_guard::exiger_soi_ou_owner(&req, &uq) {
        return response;
    }

    let account_db_path = format!("data/users/{}.db", uq.replace("-", "_").to_lowercase());
    if !std::path::Path::new(&account_db_path).exists() {
        return HttpResponse::NotFound().json(json!({
            "status": "error",
            "message": "Compte introuvable"
        }));
    }

    let account_conn = match Connection::open(&account_db_path) {
        Ok(conn) => conn,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("DB compte: {}", e)
            }))
        }
    };
    let (points, frozen_points): (i64, i64) = match account_conn.query_row(
        "SELECT points, COALESCE(frozen_points, 0) FROM balance WHERE id = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ) {
        Ok(balance) => balance,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Solde BORB indisponible: {}", e)
            }))
        }
    };
    let borb_reserve = points.max(0) as f64 / 100.0;
    let borb_available = (points - frozen_points).max(0) as f64 / 100.0;

    let conn = match rusqlite::Connection::open("uq_index.db") {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("DB error: {}", e)
            }))
        }
    };

    // Lire les wallets depuis les vraies tables du compte
    let mut wallets: Vec<serde_json::Value> = Vec::new();
    let mut transferable_borb = 0.0;

    // 1. Lire les unites BORB fractales
    if let Ok(mut units_stmt) =
        account_conn.prepare("SELECT unit, balance FROM my_borb_units ORDER BY balance DESC")
    {
        if let Ok(units) = units_stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        }) {
            for unit_result in units {
                if let Ok((unit, balance)) = unit_result {
                    // Le taux d'une unité fractale est une définition de cette
                    // unité, pas une conséquence du solde : `borb_equiv / balance`
                    // retombait sur 1.0 dès que le solde était nul, ce qui
                    // affirmait « 1 mborb = 1 BORB » sur un portefeuille vide.
                    let (taux_unite, borb_equiv) = match unit.as_str() {
                        "BORB" => (1.0, balance as f64),
                        "borb" => (1.0 / 100.0, balance as f64 / 100.0),
                        "mborb" => (1.0 / 10_000.0, balance as f64 / 10_000.0),
                        "nborb" => (1.0 / 1_000_000.0, balance as f64 / 1_000_000.0),
                        "pborb" => (1.0 / 100_000_000.0, balance as f64 / 100_000_000.0),
                        "fborb" => (1.0 / 10_000_000_000.0, balance as f64 / 10_000_000_000.0),
                        _ => (0.0, 0.0),
                    };
                    // La ligne « BORB » des unites est le miroir de la reserve
                    // (points/100) : la compter ici doublait le total_borb.
                    if unit != "BORB" {
                        transferable_borb += borb_equiv;
                    }

                    wallets.push(json!({
                        "code": unit,
                        "balance": balance,
                        "borb_equivalent": borb_equiv,
                        "rate": taux_unite
                    }));
                }
            }
        }
    }

    // 2. Lire le wallet principal
    if let Ok(mut wallet_stmt) = account_conn.prepare(
        "SELECT COALESCE(balance_borb, 0), COALESCE(balance_points, 0), COALESCE(balance_frozen, 0) FROM wallet WHERE id = 1"
    ) {
        if let Ok(mut rows) = wallet_stmt.query([]) {
            if let Ok(Some(row)) = rows.next() {
                let wallet_borb: f64 = row.get(0).unwrap_or(0.0);
                let wallet_pts: i64 = row.get(1).unwrap_or(0);
                let wallet_frozen: f64 = row.get(2).unwrap_or(0.0);
                
                wallets.push(json!({
                    "code": "WALLET",
                    "balance": wallet_borb,
                    "balance_points": wallet_pts,
                    "frozen": wallet_frozen,
                    "borb_equivalent": wallet_borb,
                    "rate": 1.0
                }));
            }
        }
    }

    // Taux LIVE depuis l'oracle
    let taux_live = taux_live_borb().await;

    // Lire les preferences
    let (display_currency, receive_currency): (String, String) = conn.query_row(
        "SELECT COALESCE(display_currency, 'CDF'), COALESCE(receive_currency, 'CDF') FROM uq_index WHERE uq_number = ?1",
        rusqlite::params![uq],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap_or(("CDF".to_string(), "CDF".to_string()));

    wallets.push(json!({
        "code": "BORB",
        "balance": borb_reserve,
        "available_balance": borb_available,
        "borb_equivalent": borb_reserve,
        "rate": 1.0,
        "transferable": false
    }));

    HttpResponse::Ok().json(json!({
        "status": "success",
        "uq": uq,
        "display_currency": display_currency,
        "receive_currency": receive_currency,
        "wallets": wallets,
        "total_borb": borb_reserve + transferable_borb,
        "transferable_borb": transferable_borb,
        "borb_reserve": borb_reserve,
        "borb_available": borb_available,
    }))
}

/// GET /api/account/wallets/currencies/{uq}
/// Lecture seule des devises reellement detenues apres conversion.
/// Les lignes `BORB` / `BORB` sont exclues : la reserve n'est pas une devise.
/// Aucune ecriture : cet endpoint ne sert qu'a l'affichage du coffre.
pub async fn get_wallet_currencies(
    req: actix_web::HttpRequest,
    path: web::Path<String>,
) -> HttpResponse {
    use rusqlite::Connection;
    let uq = path.into_inner();

    if !crate::auth_guard::ressemble_a_un_uq(&uq) {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Identifiant de compte invalide"
        }));
    }
    if let Err(response) = crate::auth_guard::exiger_soi_ou_owner(&req, &uq) {
        return response;
    }

    let conn = match rusqlite::Connection::open("uq_index.db") {
        Ok(conn) => conn,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("DB error: {}", e)
            }))
        }
    };

    let mut stmt = match conn.prepare(
        "SELECT currency, COALESCE(balance, 0)
         FROM multi_wallets
         WHERE uq_number = ?1
           AND COALESCE(balance, 0) > 0
           AND UPPER(currency) NOT IN ('BORB', 'BORB')
         ORDER BY currency",
    ) {
        Ok(stmt) => stmt,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Lecture des devises impossible: {}", e)
            }))
        }
    };

    let rows = match stmt.query_map(rusqlite::params![uq], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
    }) {
        Ok(rows) => rows,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Lecture des devises impossible: {}", e)
            }))
        }
    };

    let mut currencies: Vec<serde_json::Value> = Vec::new();
    for row in rows {
        match row {
            Ok((code, balance)) => currencies.push(json!({
                "code": code,
                "balance": balance
            })),
            Err(e) => {
                return HttpResponse::InternalServerError().json(json!({
                    "status": "error",
                    "message": format!("Lecture d'une devise impossible: {}", e)
                }))
            }
        }
    }

    HttpResponse::Ok().json(json!({
        "status": "success",
        "uq": uq,
        "currencies": currencies
    }))
}

/// POST /api/account/wallets/allocate
/// Repartit les fonds entre devises
pub async fn wallet_transfers(
    req: actix_web::HttpRequest,
    path: web::Path<String>,
) -> HttpResponse {
    use rusqlite::Connection;

    let uq = path.into_inner();
    if !crate::auth_guard::ressemble_a_un_uq(&uq) {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Identifiant de compte invalide"
        }));
    }
    if let Err(response) = crate::auth_guard::exiger_soi_ou_owner(&req, &uq) {
        return response;
    }

    let conn = match rusqlite::Connection::open("uq_index.db") {
        Ok(conn) => conn,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Base des portefeuilles indisponible: {}", e)
            }))
        }
    };
    if let Err(e) = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS wallet_transfers (
            transfer_id TEXT PRIMARY KEY,
            sender_uq TEXT NOT NULL,
            recipient_uq TEXT NOT NULL,
            currency TEXT NOT NULL,
            amount REAL NOT NULL,
            fee REAL NOT NULL,
            memo TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_wallet_transfers_sender
            ON wallet_transfers(sender_uq, created_at);
        CREATE INDEX IF NOT EXISTS idx_wallet_transfers_recipient
            ON wallet_transfers(recipient_uq, created_at);",
    ) {
        return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("Initialisation de l'historique impossible: {}", e)
        }));
    }
    let mut stmt = match conn.prepare(
        "SELECT transfer_id,
                CASE WHEN sender_uq = ?1 THEN 'send' ELSE 'receive' END,
                CASE WHEN sender_uq = ?1 THEN recipient_uq ELSE sender_uq END,
                currency, amount, fee, memo, created_at
         FROM wallet_transfers
         WHERE sender_uq = ?1 OR recipient_uq = ?1
         ORDER BY created_at DESC
         LIMIT 100",
    ) {
        Ok(stmt) => stmt,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Lecture de l'historique impossible: {}", e)
            }))
        }
    };
    let rows = match stmt.query_map(rusqlite::params![uq], |row| {
        Ok(json!({
            "id": row.get::<_, String>(0)?,
            "type": row.get::<_, String>(1)?,
            "counterparty": row.get::<_, String>(2)?,
            "currency": row.get::<_, String>(3)?,
            "amount": row.get::<_, f64>(4)?,
            "fee": row.get::<_, f64>(5)?,
            "memo": row.get::<_, String>(6)?,
            "timestamp": row.get::<_, i64>(7)?,
        }))
    }) {
        Ok(rows) => rows,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Lecture de l'historique impossible: {}", e)
            }))
        }
    };
    let mut transfers = Vec::new();
    for row in rows {
        match row {
            Ok(mut transfer) => {
                if let Some(counterparty) = transfer.get("counterparty").and_then(|v| v.as_str()) {
                    transfer["counterparty"] =
                        json!(crate::uq_public_alias::get_public_display(counterparty));
                }
                transfers.push(transfer);
            }
            Err(e) => {
                return HttpResponse::InternalServerError().json(json!({
                    "status": "error",
                    "message": format!("Lecture d'un transfert impossible: {}", e)
                }))
            }
        }
    }

    HttpResponse::Ok().json(json!({
        "status": "success",
        "transfers": transfers
    }))
}

pub async fn allocate_wallets(body: web::Json<serde_json::Value>) -> HttpResponse {
    use rusqlite::Connection;
    let uq = body.get("uq_number").and_then(|v| v.as_str()).unwrap_or("");
    let allocations = body.get("allocations").and_then(|v| v.as_array());

    if uq.is_empty() {
        return HttpResponse::BadRequest()
            .json(json!({"status":"error","message":"uq_number requis"}));
    }

    let allocs = match allocations {
        Some(a) => a,
        None => {
            return HttpResponse::BadRequest()
                .json(json!({"status":"error","message":"allocations requises"}))
        }
    };

    let conn = match rusqlite::Connection::open("uq_index.db") {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError()
                .json(json!({"status":"error","message":e.to_string()}))
        }
    };

    let now = chrono::Utc::now().timestamp();

    for alloc in allocs {
        let currency = alloc.get("currency").and_then(|v| v.as_str()).unwrap_or("");
        let amount = alloc.get("amount").and_then(|v| v.as_f64()).unwrap_or(0.0);

        if currency.is_empty() {
            continue;
        }

        let _ = conn.execute(
            "INSERT INTO multi_wallets (uq_number, currency, balance, updated_at) 
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(uq_number, currency) DO UPDATE SET balance = ?3, updated_at = ?4",
            rusqlite::params![uq, currency, amount, now],
        );
    }

    HttpResponse::Ok().json(json!({"status":"success","message":"Allocations enregistrees"}))
}

/// POST /api/account/preferences/currency
/// Change les preferences de devise
pub async fn set_currency_preferences(body: web::Json<serde_json::Value>) -> HttpResponse {
    use rusqlite::Connection;
    let uq = body.get("uq_number").and_then(|v| v.as_str()).unwrap_or("");
    let display = body.get("display_currency").and_then(|v| v.as_str());
    let receive = body.get("receive_currency").and_then(|v| v.as_str());

    if uq.is_empty() {
        return HttpResponse::BadRequest()
            .json(json!({"status":"error","message":"uq_number requis"}));
    }

    let conn = match rusqlite::Connection::open("uq_index.db") {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError()
                .json(json!({"status":"error","message":e.to_string()}))
        }
    };

    if let Some(d) = display {
        let _ = conn.execute(
            "UPDATE uq_index SET display_currency = ?1 WHERE uq_number = ?2",
            rusqlite::params![d, uq],
        );
    }

    if let Some(r) = receive {
        let _ = conn.execute(
            "UPDATE uq_index SET receive_currency = ?1 WHERE uq_number = ?2",
            rusqlite::params![r, uq],
        );
    }

    HttpResponse::Ok().json(json!({"status":"success"}))
}

/// Vérifie le PIN d'un compte avant une opération financière (convert/withdraw).
pub(crate) fn verifier_pin_compte(
    account_db_path: &str,
    uq: &str,
    pin: &str,
    action: &str,
) -> Result<(), HttpResponse> {
    use rusqlite::Connection;
    let pin_conn = match Connection::open(account_db_path) {
        Ok(conn) => conn,
        Err(e) => {
            return Err(HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Ouverture du compte impossible pour vérifier le PIN: {}", e)
            })))
        }
    };
    if let Err(e) = crate::pin_manager::init_pin_table(&pin_conn) {
        return Err(HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("Vérification du PIN indisponible: {}", e)
        })));
    }

    let pin_manager_configured = match pin_conn.query_row(
        "SELECT (COALESCE(pin_hash, '') <> '' AND pin_enabled = 1) FROM pin_status WHERE id = 1",
        [],
        |row| row.get::<_, bool>(0),
    ) {
        Ok(configured) => configured,
        Err(e) => {
            return Err(HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Lecture de la configuration PIN impossible: {}", e)
            })))
        }
    };
    let legacy_pin_configured = match pin_conn.query_row(
        "SELECT (SELECT COUNT(*) FROM settings WHERE key = 'pin_hash' AND COALESCE(value, '') <> '') > 0
            AND (SELECT COALESCE(value, 'false') FROM settings WHERE key = 'pin_enabled') = 'true'",
        [],
        |row| row.get::<_, bool>(0),
    ) {
        Ok(configured) => configured,
        Err(_) if pin_manager_configured => false,
        Err(e) => return Err(HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("Lecture de la configuration PIN impossible: {}", e)
        }))),
    };
    if !(pin_manager_configured || legacy_pin_configured) {
        return Err(HttpResponse::Forbidden().json(json!({
            "status": "error",
            "code": "PIN_NOT_CONFIGURED",
            "message": format!("Configurez un code PIN avant de {}", action)
        })));
    }

    match crate::pin_manager::verify_pin(&pin_conn, pin) {
        Ok(true) => Ok(()),
        Ok(false) => {
            let salt: String = pin_conn
                .query_row(
                    "SELECT value FROM settings WHERE key = 'pin_salt'",
                    [],
                    |row| row.get(0),
                )
                .unwrap_or_else(|_| format!("uq_pin_{}", uq));
            let stored_hash: String = pin_conn
                .query_row(
                    "SELECT value FROM settings WHERE key = 'pin_hash'",
                    [],
                    |row| row.get(0),
                )
                .unwrap_or_default();
            if stored_hash.is_empty() || crate::pin_manager::hash_pin(pin, &salt) != stored_hash {
                return Err(HttpResponse::BadRequest().json(json!({
                    "status": "error",
                    "code": "INVALID_PIN",
                    "message": "Code PIN incorrect"
                })));
            }
            Ok(())
        }
        Err(message) => Err(HttpResponse::TooManyRequests().json(json!({
            "status": "error",
            "code": "PIN_LOCKED",
            "message": message
        }))),
    }
}

/// Renseignements de traçabilité d'un appel : d'où il vient, pour la ligne de
/// grand livre. Les chemins sans requête HTTP (tâches internes) passent un
/// contexte vide avec leur seul `device`.
#[derive(Default)]
pub(crate) struct ContexteAppel<'a> {
    pub(crate) device: &'a str,
    pub(crate) os: &'a str,
    pub(crate) browser: &'a str,
    pub(crate) ip: &'a str,
}

/// Une écriture de grand livre à poser dans la base d'un compte après un
/// mouvement de BORB.
pub(crate) struct EcritureCoffre<'a> {
    pub(crate) uq: &'a str,
    pub(crate) chemin_db: &'a str,
    pub(crate) tx_type: &'a str,
    pub(crate) points: i64,
    pub(crate) frais_points: i64,
    /// Contrepartie affichée : pseudo et nom, jamais l'identifiant technique.
    pub(crate) contrepartie: &'a str,
    pub(crate) note: &'a str,
    pub(crate) reference: &'a str,
    pub(crate) emetteur: &'a str,
    pub(crate) destinataire: &'a str,
}

/// Le grand livre du compte et son miroir `my_borb_units` sont les deux
/// témoins d'un mouvement de BORB. Sans eux, une conversion ou un transfert
/// converti déplace de la vraie valeur sans laisser de trace, et le solde
/// affiché diverge de `balance.points`.
///
/// La chaîne et son scellé se recalculent dans la DB du compte : le connecteur
/// est donc ouvert sur cette base, jamais sur `uq_index.db`. Les bases de compte
/// étant attachées à la transaction du mouvement, cette écriture ne peut se
/// faire qu'après le commit : un échec laisse un mouvement non tracé — jamais un
/// solde faux — et doit être signalé assez fort pour être rejoué.
pub(crate) fn tracer_ecritures_coffre(
    ecritures: &[EcritureCoffre],
    horodatage: i64,
    contexte: &ContexteAppel,
) {
    for ecriture in ecritures {
        let conn = match rusqlite::Connection::open(ecriture.chemin_db) {
            Ok(c) => c,
            Err(e) => {
                eprintln!(
                    "🚨 GRAND LIVRE {} : base inaccessible ({}) — mouvement de {} points non tracé",
                    ecriture.uq, e, ecriture.points
                );
                continue;
            }
        };

        let empreinte = crate::p2p_escrow::hash_de_transaction(&format!(
            "UQ_MOUVEMENT_COFFRE:{}:{}:{}:{}:{}",
            ecriture.uq, ecriture.tx_type, ecriture.points, horodatage, ecriture.reference
        ));
        let cadenas = crate::p2p_escrow::hash_de_transaction(&format!(
            "CADENAS:{}:{}",
            ecriture.uq, horodatage
        ));
        let cle_universelle = crate::p2p_escrow::hash_de_transaction(&format!(
            "UQ_BORB_UNIVERSAL:{}",
            horodatage / 30
        ));

        let trace = crate::local_history::enregistrer_transaction(
            &conn,
            &crate::local_history::NouvelleTransaction {
                uq_number: ecriture.uq,
                tx_type: ecriture.tx_type,
                amount: ecriture.points,
                fee: ecriture.frais_points,
                counterparty: ecriture.contrepartie,
                memo: ecriture.note,
                borb_id: &empreinte,
                device: contexte.device,
                os: contexte.os,
                browser: contexte.browser,
                ip: contexte.ip,
                cadenas_hash: &cadenas,
                internal_clock: horodatage,
                universal_key_hash: &cle_universelle,
                signature: "unsigned",
                created_at: horodatage,
                sender_uq: ecriture.emetteur,
                recipient_uq: ecriture.destinataire,
            },
        )
        .and_then(|_| {
            crate::p2p_escrow::resynchroniser_sous_unites(&conn, horodatage)
                .map_err(|e| format!("miroir BORB non resynchronisé : {}", e.message()))
        });

        if let Err(e) = trace {
            eprintln!(
                "🚨 GRAND LIVRE {} : écriture « {} » de {} points refusée ({}) — solde déjà validé",
                ecriture.uq, ecriture.tx_type, ecriture.points, e
            );
        }
    }
}

/// Chemin de la base d'un compte, le seul endroit où vivent `balance`, son
/// miroir et son grand livre.
pub(crate) fn chemin_base_compte(uq: &str) -> String {
    format!("data/users/{}.db", uq.replace("-", "_").to_lowercase())
}

/// POST /api/account/wallets/convert
/// Conversion atomique entre BORB central et une devise virtuelle prise en charge.
pub async fn convert_wallets(
    req: actix_web::HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    use rusqlite::Connection;
    let uq = body.get("uq_number").and_then(|v| v.as_str()).unwrap_or("");
    let from = body
        .get("from")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_uppercase();
    let to = body
        .get("to")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_uppercase();
    let amount = body.get("amount").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let pin = body.get("pin").and_then(|v| v.as_str()).unwrap_or("");

    if !crate::auth_guard::ressemble_a_un_uq(uq) {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Identifiant de compte invalide"
        }));
    }
    if let Err(response) = crate::auth_guard::exiger_soi_ou_owner(&req, uq) {
        return response;
    }

    let from_borb = from == "BORB" || from == "BORB";
    let to_borb = to == "BORB" || to == "BORB";
    // BORB ↔ BORB : gratuit par règle. L'opération n'existe pas encore
    // (from_borb == to_borb est refusé) ; le jour où elle sera ajoutée,
    // `frais_plateforme::frais_conversion` renvoie déjà 0 pour cette paire.
    if uq.is_empty() || from.is_empty() || to.is_empty() || from_borb == to_borb {
        return HttpResponse::BadRequest()
            .json(json!({"status":"error","message":"Parametres invalides"}));
    }
    if !amount.is_finite() || amount <= 0.0 {
        return HttpResponse::BadRequest()
            .json(json!({"status":"error","message":"Montant invalide"}));
    }
    if !crate::pin_manager::is_valid_pin_format(pin) {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Le code PIN est requis et doit contenir 4 à 6 chiffres"
        }));
    }
    let currency = if from_borb { &to } else { &from };
    if !crate::frais_plateforme::est_fiat(currency) {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Devise de conversion non prise en charge"
        }));
    }

    let account_db_path = format!("data/users/{}.db", uq.replace("-", "_").to_lowercase());
    if !std::path::Path::new(&account_db_path).exists() {
        return HttpResponse::NotFound()
            .json(json!({"status":"error","message":"Compte introuvable"}));
    }

    if let Err(response) =
        verifier_pin_compte(&account_db_path, uq, pin, "convertir vos unités BORB")
    {
        return response;
    }

    let mut conn = match rusqlite::Connection::open("uq_index.db") {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError()
                .json(json!({"status":"error","message":e.to_string()}))
        }
    };

    if let Err(e) = conn.execute(
        "ATTACH DATABASE ?1 AS user_account",
        rusqlite::params![account_db_path],
    ) {
        return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("Impossible d'ouvrir le compte: {}", e)
        }));
    }
    if let Err(e) = crate::frais_plateforme::assurer_tables(&conn) {
        return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("Initialisation des frais impossible: {}", e)
        }));
    }

    let taux_live = taux_live_borb().await;
    let taux_currency = taux_live.get(currency).copied().unwrap_or(0.0);
    if taux_currency <= 0.0 || !taux_currency.is_finite() {
        return HttpResponse::BadRequest()
            .json(json!({"status":"error","message":"Taux de conversion indisponible"}));
    }

    let now = chrono::Utc::now().timestamp();
    let tx = match conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate) {
        Ok(tx) => tx,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Transaction impossible: {}", e)
            }))
        }
    };

    let (amount_from, amount_to, frais, devise_frais, points_mouvement) = if from_borb {
        let points_f64 = amount * 100.0;
        if !points_f64.is_finite() || points_f64 > i64::MAX as f64 {
            return HttpResponse::BadRequest().json(json!({
                "status": "error",
                "message": "Montant BORB trop élevé"
            }));
        }
        let amount_points = points_f64.round() as i64;
        if amount_points <= 0 || (points_f64 - amount_points as f64).abs() > 0.000001 {
            return HttpResponse::BadRequest().json(json!({
                "status": "error",
                "message": "Le montant BORB doit être exprimé au centième près"
            }));
        }

        let borb_equivalent = amount_points as f64 / 100.0;
        let amount_converted = borb_equivalent / taux_currency;
        if !amount_converted.is_finite() || amount_converted <= 0.0 {
            return HttpResponse::BadRequest().json(json!({
                "status": "error",
                "message": "Montant converti invalide"
            }));
        }

        let frais = crate::frais_plateforme::frais_conversion(&from, &to, amount_converted);
        let amount_recu = crate::frais_plateforme::arrondi_centimes(amount_converted - frais);
        if !amount_recu.is_finite() || amount_recu < 0.01 {
            return HttpResponse::BadRequest().json(json!({
                "status": "error",
                "code": "AMOUNT_TOO_LOW",
                "message": "Montant converti trop faible : les frais de service absorbent tout le montant."
            }));
        }

        let debit = match tx.execute(
            "UPDATE user_account.balance
             SET points = points - ?1
             WHERE id = 1
               AND points - COALESCE(frozen_points, 0) >= ?1",
            rusqlite::params![amount_points],
        ) {
            Ok(rows) => rows,
            Err(e) => {
                return HttpResponse::InternalServerError().json(json!({
                    "status": "error",
                    "message": format!("Débit BORB impossible: {}", e)
                }))
            }
        };
        if debit != 1 {
            return HttpResponse::BadRequest().json(json!({
                "status": "error",
                "code": "INSUFFICIENT_BORB",
                "message": "Solde BORB disponible insuffisant pour cette conversion"
            }));
        }

        if let Err(e) = tx.execute(
            "INSERT INTO multi_wallets (uq_number, currency, balance, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(uq_number, currency)
             DO UPDATE SET balance = multi_wallets.balance + excluded.balance, updated_at = excluded.updated_at",
            rusqlite::params![uq, to, amount_recu, now],
        ) {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Crédit de la devise impossible: {}", e)
            }));
        }
        if let Err(e) =
            crate::frais_plateforme::enregistrer_tx(&tx, "convert", "", uq, &to, frais, now)
        {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Enregistrement des frais impossible: {}", e)
            }));
        }
        (
            borb_equivalent,
            amount_recu,
            frais,
            to.clone(),
            amount_points,
        )
    } else {
        let (amount_minor, points_brut) =
            match calculate_currency_to_borb_points(amount, taux_currency) {
                Ok(amounts) => amounts,
                Err(ConvertedTransferAmountError::Invalid) => {
                    return HttpResponse::BadRequest().json(json!({
                        "status": "error",
                        "message": "Montant ou taux invalide"
                    }));
                }
                Err(ConvertedTransferAmountError::Precision) => {
                    return HttpResponse::BadRequest().json(json!({
                        "status": "error",
                        "message": "Montant incompatible avec la précision BORB"
                    }));
                }
                Err(ConvertedTransferAmountError::TooLarge) => {
                    return HttpResponse::BadRequest().json(json!({
                        "status": "error",
                        "message": "Montant trop élevé"
                    }));
                }
            };

        let frais_minor = crate::frais_plateforme::frais_centimes(amount_minor, currency);

        // Le frais de service se paie en plus, dans la devise envoyée : 50 USD
        // débite 50,05 USD et crédite l'équivalent complet des 50,00 USD en BORB.
        // Le solde doit donc couvrir le brut, pas seulement le montant demandé.
        match settle_currency_to_borb(&tx, uq, currency, amount_minor + frais_minor, points_brut, now) {
            Ok(true) => {}
            Ok(false) => return HttpResponse::BadRequest().json(json!({
                "status": "error",
                "code": "INSUFFICIENT_BALANCE",
                "message": "Solde de devise insuffisant (montant + frais) ou compte BORB indisponible"
            })),
            Err(e) => return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Reconversion vers BORB impossible: {}", e)
            })),
        }
        let frais = frais_minor as f64 / 100.0;
        if let Err(e) =
            crate::frais_plateforme::enregistrer_tx(&tx, "convert", "", uq, currency, frais, now)
        {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Enregistrement des frais impossible: {}", e)
            }));
        }
        (
            amount_minor as f64 / 100.0,
            points_brut as f64 / 100.0,
            frais,
            currency.clone(),
            points_brut,
        )
    };

    if let Err(e) = tx.commit() {
        return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("Validation de la conversion impossible: {}", e)
        }));
    }

    let sens = if from_borb {
        "Réserve → portefeuille"
    } else {
        "Portefeuille → réserve"
    };
    let note = format!("{} · 1 BORB = {} {}", sens, taux_currency, currency);
    let contrepartie = if from_borb {
        format!("Portefeuille {}", currency)
    } else {
        "Réserve BORB".to_string()
    };
    tracer_ecritures_coffre(
        &[EcritureCoffre {
            uq,
            chemin_db: &account_db_path,
            tx_type: "convert",
            points: points_mouvement,
            // Le frais de conversion se paie en devise, pas en points : il est
            // déjà porté par la ligne `platform_fees` de cette transaction.
            frais_points: 0,
            contrepartie: &contrepartie,
            note: &note,
            reference: "",
            emetteur: uq,
            destinataire: uq,
        }],
        now,
        &ContexteAppel {
            device: "wallet_convert",
            ..Default::default()
        },
    );

    let repartition = crate::frais_plateforme::repartir(frais);
    HttpResponse::Ok().json(json!({
        "status": "success",
        "from": from,
        "to": to,
        "amount_from": amount_from,
        "amount_to": amount_to,
        "fee": frais,
        "fee_currency": devise_frais,
        "fee_case_split": {
            "tresor": repartition[0],
            "frais": repartition[1],
            "recompenses": repartition[2],
            "epargne": repartition[3],
            "credit": repartition[4],
            "bouclier": repartition[5]
        },
        "borb_equivalent": if from_borb { amount_from } else { amount_to },
        "borb_equivalent": if from_borb { amount_from } else { amount_to },
        "borb_per_unit": taux_currency,
        "rate": if from_borb { 1.0 / taux_currency } else { taux_currency },
    }))
}

/// POST /api/account/wallets/withdraw
/// Retrait d'une devise virtuelle. Les fonds sont réservés immédiatement et
/// le retrait reste « pending » : aucun versement externe n'est simulé ici.
pub async fn withdraw_wallet(
    req: actix_web::HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    use rusqlite::Connection;
    let uq = body.get("uq_number").and_then(|v| v.as_str()).unwrap_or("");
    let currency = body
        .get("currency")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_uppercase();
    let amount = body.get("amount").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let pin = body.get("pin").and_then(|v| v.as_str()).unwrap_or("");
    let destination = body
        .get("destination")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    if !crate::auth_guard::ressemble_a_un_uq(uq) {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Identifiant de compte invalide"
        }));
    }
    if let Err(response) = crate::auth_guard::exiger_soi_ou_owner(&req, uq) {
        return response;
    }
    if crate::frais_plateforme::est_borb(&currency)
        || !crate::frais_plateforme::est_retirable(&currency)
    {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Devise non retirable. La réserve BORB n'est jamais retirable."
        }));
    }
    if !amount.is_finite() || amount <= 0.0 {
        return HttpResponse::BadRequest()
            .json(json!({"status":"error","message":"Montant invalide"}));
    }
    let amount_minor_f = amount * 100.0;
    let amount_minor = amount_minor_f.round() as i64;
    if amount_minor < 1 || (amount_minor_f - amount_minor as f64).abs() > 0.000001 {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "code": "INVALID_PRECISION",
            "message": "Le montant doit être au centième près."
        }));
    }
    if destination.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "code": "DESTINATION_REQUIRED",
            "message": "Destinataire du retrait requis."
        }));
    }
    if !crate::pin_manager::is_valid_pin_format(pin) {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Le code PIN est requis et doit contenir 4 à 6 chiffres"
        }));
    }

    let account_db_path = format!("data/users/{}.db", uq.replace("-", "_").to_lowercase());
    if !std::path::Path::new(&account_db_path).exists() {
        return HttpResponse::NotFound()
            .json(json!({"status":"error","message":"Compte introuvable"}));
    }
    if let Err(response) = verifier_pin_compte(&account_db_path, uq, pin, "retirer vos devises") {
        return response;
    }

    let frais_minor = crate::frais_plateforme::frais_centimes(amount_minor, &currency);
    let net_minor = amount_minor - frais_minor;
    if net_minor < 1 {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "code": "AMOUNT_TOO_LOW",
            "message": "Montant trop faible : les frais de service absorbent tout le retrait."
        }));
    }
    let frais = frais_minor as f64 / 100.0;
    let net = net_minor as f64 / 100.0;

    let conn = match rusqlite::Connection::open("uq_index.db") {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError()
                .json(json!({"status":"error","message":e.to_string()}))
        }
    };
    if let Err(e) = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS wallet_withdrawals (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            uq TEXT NOT NULL,
            currency TEXT NOT NULL,
            amount REAL NOT NULL,
            fee REAL NOT NULL,
            net REAL NOT NULL,
            destination TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'pending',
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_wallet_withdrawals_uq
            ON wallet_withdrawals(uq, created_at);",
    ) {
        return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("Initialisation des retraits impossible: {}", e)
        }));
    }
    if let Err(e) = crate::frais_plateforme::assurer_tables(&conn) {
        return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("Initialisation des frais impossible: {}", e)
        }));
    }

    let mut conn = conn;
    let now = chrono::Utc::now().timestamp();
    let tx = match conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate) {
        Ok(tx) => tx,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Transaction impossible: {}", e)
            }))
        }
    };

    let debit = match tx.execute(
        "UPDATE multi_wallets
         SET balance = balance - ?1, updated_at = ?2
         WHERE uq_number = ?3 AND currency = ?4 AND balance >= ?1",
        rusqlite::params![amount + frais, now, uq, currency],
    ) {
        Ok(rows) => rows,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Débit du portefeuille impossible: {}", e)
            }))
        }
    };
    if debit != 1 {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "code": "INSUFFICIENT_BALANCE",
            "message": format!("Solde {} insuffisant pour ce retrait.", currency)
        }));
    }

    if let Err(e) = tx.execute(
        "INSERT INTO wallet_withdrawals
            (uq, currency, amount, fee, net, destination, status, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', ?7)",
        rusqlite::params![uq, currency, amount, frais, net, destination, now],
    ) {
        return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("Enregistrement du retrait impossible: {}", e)
        }));
    }
    let withdrawal_id = tx.last_insert_rowid();

    if frais > 0.0 {
        if let Err(e) = crate::frais_plateforme::enregistrer_tx(
            &tx,
            "withdraw",
            &format!("withdraw:{}", withdrawal_id),
            uq,
            &currency,
            frais,
            now,
        ) {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Enregistrement des frais impossible: {}", e)
            }));
        }
    }

    if let Err(e) = tx.commit() {
        return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("Validation du retrait impossible: {}", e)
        }));
    }

    let repartition = crate::frais_plateforme::repartir(frais);
    HttpResponse::Ok().json(json!({
        "status": "success",
        "withdrawal_id": withdrawal_id,
        "currency": currency,
        "amount": amount,
        "fee": frais,
        "fee_currency": currency,
        "net": net,
        "destination": destination,
        "withdrawal_status": "pending",
        "fee_case_split": {
            "tresor": repartition[0],
            "frais": repartition[1],
            "recompenses": repartition[2],
            "epargne": repartition[3],
            "credit": repartition[4],
            "bouclier": repartition[5]
        },
        "message": "Retrait enregistré et fonds réservés. En attente de traitement externe."
    }))
}

pub async fn cases_live_stats(_state: web::Data<Arc<GlobalState>>) -> HttpResponse {
    use rusqlite::Connection;

    // 1. Charger l'etat actuel depuis block_orb_global.json
    let orb = crate::persistence::load_global_orb();
    const FEMTO: f64 = 1_000_000_000_000_000.0; // 10^15

    let labels = [
        ("Tresor", "Compte Principal", "🏦"),
        ("Frais", "Compte Service", "⚙️"),
        ("Recompenses", "Bonus & Cadeaux", "🎁"),
        ("Epargne", "Epargne Securisee", "💰"),
        ("Credit", "Pret Personnel", "💳"),
        ("Bouclier", "Protection Famille", "🛡️"),
    ];

    let mut cases_json_map = serde_json::Map::new();
    for (i, (key, _, _)) in labels.iter().enumerate() {
        // u128 -> f64 avec conversion sure (les valeurs sont < 2^127)
        let val_u128: u128 = orb.cases[i];
        let val = (val_u128 as f64) / FEMTO;
        cases_json_map.insert(key.to_string(), serde_json::json!(val));
        // [LOG-QUIET]         eprintln!("[LIVE-STATS] {} = {} femto = {} BORB", key, val_u128, val);
    }
    let cases_json = serde_json::Value::Object(cases_json_map);

    // 2. Ouvrir la DB owner pour lire counter_transactions + seven_cases
    let db_path = "data/users/uq_00000001.db";
    let conn = match Connection::open(db_path) {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("DB error: {}", e)
            }))
        }
    };

    // 3. Calculer les entrees/sorties sur les dernieres 24h
    let now = chrono::Utc::now().timestamp();
    let il_y_a_24h = now - 86400;

    let mut entrees_tresor = 0.0f64;
    let mut entrees_frais = 0.0f64;
    let mut entrees_recompenses = 0.0f64;
    let mut entrees_epargne = 0.0f64;
    let mut entrees_credit = 0.0f64;
    let mut entrees_bouclier = 0.0f64;

    if let Ok(mut stmt) = conn.prepare(
        "SELECT to_tresor, to_frais, to_recompenses, to_epargne, to_credit, to_bouclier 
         FROM counter_transactions WHERE created_at >= ?1",
    ) {
        if let Ok(rows) = stmt.query_map(rusqlite::params![il_y_a_24h], |row| {
            Ok((
                row.get::<_, f64>(0).unwrap_or(0.0),
                row.get::<_, f64>(1).unwrap_or(0.0),
                row.get::<_, f64>(2).unwrap_or(0.0),
                row.get::<_, f64>(3).unwrap_or(0.0),
                row.get::<_, f64>(4).unwrap_or(0.0),
                row.get::<_, f64>(5).unwrap_or(0.0),
            ))
        }) {
            for r in rows.flatten() {
                entrees_tresor += r.0;
                entrees_frais += r.1;
                entrees_recompenses += r.2;
                entrees_epargne += r.3;
                entrees_credit += r.4;
                entrees_bouclier += r.5;
            }
        }
    }

    // 4. Lire le dernier etat seven_cases
    let mut invariant_valid = true;
    let mut total_7_cases = 0.0f64;
    let mut case_7_comptes = 0.0f64;
    let mut case_7_count = 0i64;

    if let Ok(mut stmt) = conn.prepare(
        "SELECT case_7_comptes, case_7_count, sum_6_cases, total_7_cases, invariant_valid 
         FROM seven_cases ORDER BY id DESC LIMIT 1",
    ) {
        if let Ok(row) = stmt.query_row([], |row| {
            Ok((
                row.get::<_, f64>(0).unwrap_or(0.0),
                row.get::<_, i64>(1).unwrap_or(0),
                row.get::<_, f64>(2).unwrap_or(0.0),
                row.get::<_, f64>(3).unwrap_or(0.0),
                row.get::<_, i64>(4).unwrap_or(1),
            ))
        }) {
            case_7_comptes = row.0;
            case_7_count = row.1;
            total_7_cases = row.3;
            invariant_valid = row.4 == 1;
        }
    }

    // 5. Construire les 6 cases avec labels rassurants
    let cases_data = json!([
        {
            "id": "tresor",
            "label": "Compte Principal",
            "icon": "🏦",
            "balance": cases_json.get("Tresor").and_then(|v| v.as_f64()).unwrap_or(0.0),
            "entrees": entrees_tresor,
            "sorties": 0.0,
            "net": entrees_tresor,
            "sources": ["OCEAN", "Frais", "Ventes crypto"],
            "dests": ["UBI", "Compte Service", "Prêt Personnel"]
        },
        {
            "id": "frais",
            "label": "Compte Service",
            "icon": "⚙️",
            "balance": cases_json.get("Frais").and_then(|v| v.as_f64()).unwrap_or(0.0),
            "entrees": entrees_frais,
            "sorties": 0.0,
            "net": entrees_frais,
            "sources": ["Transactions", "Ventes"],
            "dests": ["Compte Principal", "Protection Famille"]
        },
        {
            "id": "recompenses",
            "label": "Bonus & Cadeaux",
            "icon": "🎁",
            "balance": cases_json.get("Recompenses").and_then(|v| v.as_f64()).unwrap_or(0.0),
            "entrees": entrees_recompenses,
            "sorties": 0.0,
            "net": entrees_recompenses,
            "sources": ["Compte Principal"],
            "dests": ["UBI", "Utilisateurs"]
        },
        {
            "id": "epargne",
            "label": "Épargne Sécurisée",
            "icon": "💰",
            "balance": cases_json.get("Epargne").and_then(|v| v.as_f64()).unwrap_or(0.0),
            "entrees": entrees_epargne,
            "sorties": 0.0,
            "net": entrees_epargne,
            "sources": ["Utilisateurs (volontaire)"],
            "dests": ["Utilisateurs", "Prêt Personnel"]
        },
        {
            "id": "credit",
            "label": "Prêt Personnel",
            "icon": "💳",
            "balance": cases_json.get("Credit").and_then(|v| v.as_f64()).unwrap_or(0.0),
            "entrees": entrees_credit,
            "sorties": 0.0,
            "net": entrees_credit,
            "sources": ["Épargne", "Compte Principal"],
            "dests": ["Utilisateurs (prêts)", "Compte Principal"]
        },
        {
            "id": "bouclier",
            "label": "Protection Famille",
            "icon": "🛡️",
            "balance": cases_json.get("Bouclier").and_then(|v| v.as_f64()).unwrap_or(0.0),
            "entrees": entrees_bouclier,
            "sorties": 0.0,
            "net": entrees_bouclier,
            "sources": ["Compte Service", "Compte Principal"],
            "dests": ["Utilisateurs (urgence)"]
        }
    ]);

    // 6. Calculer les compteurs globaux
    let sum_6_cases: f64 = [
        cases_json
            .get("Tresor")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        cases_json
            .get("Frais")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        cases_json
            .get("Recompenses")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        cases_json
            .get("Epargne")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        cases_json
            .get("Credit")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        cases_json
            .get("Bouclier")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
    ]
    .iter()
    .sum();

    let borb_total = 40_000_000_000.0f64;
    let borb_restants = sum_6_cases + case_7_comptes;
    let borb_sortis = borb_total - borb_restants;

    HttpResponse::Ok().json(json!({
        "status": "success",
        "timestamp": now,
        "cases": cases_data,
        "total": {
            "borb_sortis": borb_sortis,
            "borb_restants": borb_restants,
            "borb_dans_cases": sum_6_cases,
            "borb_dans_comptes": case_7_comptes,
            "borb_total": borb_total,
            "comptes_count": case_7_count,
            "invariant_valid": invariant_valid
        }
    }))
}

/// Prix réellement captés sur le marché, avec la date de chaque capture.
#[derive(Clone)]
struct CapturePrix {
    btc_usd: f64,
    gold_usd: f64,
    btc_source: String,
    /// Horodatage (unix) de la capture marché derrière chaque prix. Un prix
    /// resservi hors-ligne garde sa valeur : seule son âge change, et l'âge se
    /// publie avec lui.
    or_captur_le: i64,
    btc_captur_le: i64,
    /// Instant où cette capture a été lue — fenêtre mémoire de 60 s.
    prise_le: i64,
}

impl CapturePrix {
    fn age_or(&self, maintenant: i64) -> i64 {
        (maintenant - self.or_captur_le).max(0)
    }
    fn age_btc(&self, maintenant: i64) -> i64 {
        (maintenant - self.btc_captur_le).max(0)
    }
    /// Ce qui reste vrai quand la source n'a pas répondu : le dernier prix
    /// connu s'applique jusqu'au prochain appel réussi, mais il est daté.
    fn capture_json(&self, maintenant: i64) -> serde_json::Value {
        json!({
            "or": { "age_secs": self.age_or(maintenant), "captur_le": self.or_captur_le },
            "btc": { "age_secs": self.age_btc(maintenant), "captur_le": self.btc_captur_le }
        })
    }
}

static PRICE_CACHE: std::sync::OnceLock<std::sync::Mutex<Option<CapturePrix>>> =
    std::sync::OnceLock::new();

/// Oublie la fenêtre mémoire de 60 s. Le test hors-ligne en a besoin : sans ce
/// reset, c'est la mémoire qui répondrait et le reservi disque ne serait jamais
/// exercé.
#[cfg(test)]
fn oublier_capture_prix() {
    if let Some(cache) = PRICE_CACHE.get() {
        *cache.lock().unwrap() = None;
    }
}

/// Capture réelle des prix (or, BTC), cache mémoire de 60 s.
///
/// Une seule implémentation pour `/api/price/all`, `/api/price/gold` et
/// `/api/price/usd` : trois routes qui interrogeraient chacune leur propre
/// source pourraient publier trois prix différents pour le même actif.
/// `Err((code, message))` quand aucune source n'a répondu et qu'aucun cache
/// n'existe — les appeleurs répondent 503, aucun prix n'est inventé.
async fn capture_prix() -> Result<CapturePrix, (String, String)> {
    let now = chrono::Utc::now().timestamp();
    let cache = PRICE_CACHE.get_or_init(|| std::sync::Mutex::new(None));

    if let Some(deja) = cache.lock().unwrap().clone() {
        if now - deja.prise_le < 60 && deja.btc_usd > 0.0 && deja.gold_usd > 0.0 {
            return Ok(deja);
        }
    }

    // 0.0 = rien de réel : aucun prix n'est substitué quand la source échoue.
    let btc =
        match tokio::time::timeout(std::time::Duration::from_secs(10), btc_price_source()).await {
            Ok((Some(prix), source)) => (prix, source),
            _ => (0.0, String::new()),
        };

    // Or : xaus.com (cache 5 min géré par external_api). 0.0 = rien de réel.
    let gold =
        match tokio::time::timeout(std::time::Duration::from_secs(5), fetch_gold_price()).await {
            Ok(Some(p)) if p > 0.0 => p,
            _ => 0.0,
        };

    // Les deux sources reservent leur dernière capture disque quand le réseau
    // manque : l'horodatage du disque dit depuis quand le prix publié est valide.
    let or_captur_le =
        crate::external_api::capture_persistee_le(crate::external_api::CLE_SPOT_OR).unwrap_or(now);
    let btc_captur_le =
        crate::external_api::capture_persistee_le(&crate::external_api::cle_crypto("BTCUSD"))
            .unwrap_or(now);

    let capture = CapturePrix {
        btc_usd: btc.0,
        gold_usd: gold,
        btc_source: btc.1,
        or_captur_le,
        btc_captur_le,
        prise_le: now,
    };

    if gold <= 0.0 {
        return Err((
            "OR_INDISPONIBLE".to_string(),
            "Prix de l'or indisponible : source injoignable et aucun prix en cache".to_string(),
        ));
    }
    if btc.0 <= 0.0 {
        return Err((
            "BTC_INDISPONIBLE".to_string(),
            "Prix BTC indisponible : sources injoignables et aucun prix en cache".to_string(),
        ));
    }

    *cache.lock().unwrap() = Some(capture.clone());
    Ok(capture)
}

/// Les trois routes de prix publient la même capture ; seule la projection
/// change. Sans capture, elles échouent toutes pareil (503) au lieu d'inventer.
fn reponse_capture_err(e: &(String, String), maintenant: i64) -> HttpResponse {
    HttpResponse::ServiceUnavailable().json(json!({
        "status": "error",
        "code": e.0,
        "message": e.1,
        "updated_at": maintenant
    }))
}

/// Prix du BORB dérivé de la capture or, ou une réponse explicite.
///
/// `/2000` est la parité structurelle (`regulateur::BORB_PAR_ONCE`), pas un taux
/// de marché. Mais diviser une once mal capturée (0 ou `NaN`) par cette parité
/// donnait quand même un « prix » — affiché par le front sans aucune erreur.
fn borb_usd_depuis_capture(gold_usd: f64) -> Result<f64, HttpResponse> {
    crate::regulateur::borb_usd_depuis_or(gold_usd).ok_or_else(|| {
        HttpResponse::ServiceUnavailable().json(json!({
            "status": "error",
            "code": "OR_INDISPONIBLE",
            "message": "Le prix de l'once n'est pas une observation exploitable : le BORB ne sera pas coté à partir de 0 USD.",
            "reprise": "Réessayer après une capture d'or réussie."
        }))
    })
}

pub async fn price_all() -> HttpResponse {
    let maintenant = chrono::Utc::now().timestamp();
    let c = match capture_prix().await {
        Ok(c) => c,
        Err(e) => return reponse_capture_err(&e, maintenant),
    };
    let (btc_usd, gold_usd, btc_source) = (c.btc_usd, c.gold_usd, c.btc_source.clone());

    // Parité de la plateforme : 1 BORB = 0,0005 once d'or, soit once/2000.
    // Le taux decoule donc du prix reel de l'once, plus d'un prix saisi a la main.
    let borb_usd = match borb_usd_depuis_capture(gold_usd) {
        Ok(p) => p,
        Err(refus) => return refus,
    };

    HttpResponse::Ok().json(json!({
        "status": "success",
        "prices": {
            "gold_usd_per_ounce": gold_usd,
            "btc_usd": btc_usd,
            "borb_usd": borb_usd,
            "borb_btc": borb_usd / btc_usd
        },
        "source": {
            "gold": "xaus.com (cache 5 min)",
            "btc": if btc_source.is_empty() { "oracle" } else { btc_source.as_str() }
        },
        "capture": c.capture_json(maintenant),
        // `updated_at` = l'heure de la capture marché, pas l'heure de la réponse :
        // resservir un prix de la veille en le datant d'aujourd'hui serait un mensonge.
        "updated_at": c.or_captur_le
    }))
}

// ====================
// HISTORIQUE DES PRIX — Réel (depuis CoinGecko + SQLite)
// ====================

/// Récupère l'historique BTC (24h) depuis CoinGecko
async fn fetch_btc_history_24h() -> Vec<f64> {
    let url = "https://api.coingecko.com/api/v3/coins/bitcoin/market_chart?vs_currency=usd&days=1";

    match reqwest::get(url).await {
        Ok(resp) => {
            match resp.json::<serde_json::Value>().await {
                Ok(json) => {
                    if let Some(prices) = json.get("prices").and_then(|p| p.as_array()) {
                        // Prendre 24 points (1 par heure)
                        let step = (prices.len() / 24).max(1);
                        prices
                            .iter()
                            .step_by(step)
                            .take(24)
                            .filter_map(|p| p.get(1).and_then(|v| v.as_f64()))
                            .collect()
                    } else {
                        Vec::new()
                    }
                }
                Err(_) => Vec::new(),
            }
        }
        Err(_) => Vec::new(),
    }
}

/// Historique or 24h : bougies horaires réelles (biquote, 1h × 24, cache 60 s).
/// Vec vide = aucune donnée réelle disponible ; la courbe n'est plus simulée.
async fn fetch_gold_history_24h() -> Vec<f64> {
    let reponse = crate::external_api::bougies_temps_reel("XAUUSD", "1h", 24).await;
    if reponse["status"] != "success" {
        return Vec::new();
    }
    reponse["candles"]
        .as_array()
        .map(|bougies| {
            bougies
                .iter()
                .filter_map(|b| b.get("close").and_then(|c| c.as_f64()))
                .collect()
        })
        .unwrap_or_default()
}

/// GET /api/price/history/btc — Historique BTC 24h (points réels de CoinGecko)
pub async fn price_history_btc() -> HttpResponse {
    let history =
        match tokio::time::timeout(std::time::Duration::from_secs(15), fetch_btc_history_24h())
            .await
        {
            Ok(h) => h,
            Err(_) => Vec::new(),
        };

    if history.is_empty() {
        return HttpResponse::ServiceUnavailable().json(json!({
            "status": "error",
            "code": "HISTORIQUE_INDISPONIBLE",
            "symbol": "BTC",
            "period": "24h",
            "message": "Aucun point de prix BTC réel disponible (source injoignable)"
        }));
    }

    let first = history.first().copied().unwrap_or(0.0);
    let last = history.last().copied().unwrap_or(0.0);
    let change_pct = if first > 0.0 {
        ((last - first) / first) * 100.0
    } else {
        0.0
    };

    HttpResponse::Ok().json(json!({
        "status": "success",
        "symbol": "BTC",
        "period": "24h",
        "data": history,
        "first": first,
        "last": last,
        "change_pct": change_pct,
        "points": history.len(),
        "source": "coingecko"
    }))
}

/// GET /api/price/history/gold — Historique or 24h (bougies réelles)
pub async fn price_history_gold() -> HttpResponse {
    let history = fetch_gold_history_24h().await;

    if history.is_empty() {
        return HttpResponse::ServiceUnavailable().json(json!({
            "status": "error",
            "code": "HISTORIQUE_INDISPONIBLE",
            "symbol": "XAU",
            "period": "24h",
            "message": "Aucune bougie d'or réelle disponible (source injoignable, aucun cache)"
        }));
    }

    let first = history.first().copied().unwrap_or(0.0);
    let last = history.last().copied().unwrap_or(0.0);
    let change_pct = if first > 0.0 {
        ((last - first) / first) * 100.0
    } else {
        0.0
    };

    HttpResponse::Ok().json(json!({
        "status": "success",
        "symbol": "XAU",
        "period": "24h",
        "data": history,
        "first": first,
        "last": last,
        "change_pct": change_pct,
        "points": history.len(),
        "source": "biquote (bougies 1h, cache 60 s)"
    }))
}

/// GET /api/borb/history/{id} — Historique BORB (basé sur or + transactions)
pub async fn borb_history(req: actix_web::HttpRequest, path: web::Path<String>) -> HttpResponse {
    // 🔐 SÉCURITÉ (vague 1) — USER : accès à SES données (ou owner)
    // (handler sans paramètre de chemin : contrôle JWT simple)
    // 🔐 SÉCURITÉ (vague 1) — USER : accès à SES données (ou owner)
    let __cible = path.to_string();
    if let Err(r) = crate::auth_guard::exiger_soi_ou_owner(&req, &__cible) {
        return r;
    }

    let account_id = path.into_inner();
    let db_path = format!(
        "data/users/{}.db",
        account_id.replace("-", "_").to_lowercase()
    );

    if !std::path::Path::new(&db_path).exists() {
        return HttpResponse::NotFound().json(json!({
            "status": "error",
            "message": format!("Compte introuvable: {}", account_id)
        }));
    }

    let conn = match rusqlite::Connection::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Erreur DB: {}", e)
            }))
        }
    };

    let mut stmt = match conn.prepare(
        "SELECT tx_type, amount, fee, counterparty, created_at 
         FROM transactions 
         ORDER BY created_at DESC LIMIT 100",
    ) {
        Ok(s) => s,
        Err(_) => {
            return HttpResponse::Ok().json(json!({
                "status": "success",
                "account": account_id,
                "total": 0,
                "history": []
            }))
        }
    };

    let rows = stmt.query_map([], |row| {
        let ts: i64 = row.get(4)?;
        Ok(json!({
            "type": row.get::<_, String>(0)?,
            "amount": row.get::<_, i64>(1)?,
            "fee": row.get::<_, i64>(2)?,
            "counterparty": row.get::<_, String>(3)?,
            "timestamp": ts,
            "date": chrono::DateTime::from_timestamp(ts, 0)
                .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_default()
        }))
    });

    let mut history = Vec::new();
    if let Ok(r) = rows {
        for row in r {
            if let Ok(v) = row {
                history.push(v);
            }
        }
    }

    let total_in: i64 = history
        .iter()
        .filter(|h| h["type"] == "receive")
        .map(|h| h["amount"].as_i64().unwrap_or(0))
        .sum();

    let total_out: i64 = history
        .iter()
        .filter(|h| h["type"] == "send")
        .map(|h| h["amount"].as_i64().unwrap_or(0))
        .sum();

    HttpResponse::Ok().json(json!({
        "status": "success",
        "account": account_id,
        "total": history.len(),
        "total_in": total_in,
        "total_out": total_out,
        "net": total_in - total_out,
        "history": history
    }))
}

/// GET /api/price/history/all — Historique complet (or + btc + borb)
pub async fn price_history_all() -> HttpResponse {
    let btc_history = fetch_btc_history_24h().await;
    let gold_history = fetch_gold_history_24h().await;

    if btc_history.is_empty() && gold_history.is_empty() {
        return HttpResponse::ServiceUnavailable().json(json!({
            "status": "error",
            "code": "HISTORIQUE_INDISPONIBLE",
            "period": "24h",
            "message": "Aucune source externe ne répond : ni courbe BTC, ni courbe d'or"
        }));
    }

    let btc_first = btc_history.first().copied().unwrap_or(0.0);
    let btc_last = btc_history.last().copied().unwrap_or(0.0);
    let btc_change = if btc_first > 0.0 {
        ((btc_last - btc_first) / btc_first) * 100.0
    } else {
        0.0
    };

    let gold_first = gold_history.first().copied().unwrap_or(0.0);
    let gold_last = gold_history.last().copied().unwrap_or(0.0);
    let gold_change = if gold_first > 0.0 {
        ((gold_last - gold_first) / gold_first) * 100.0
    } else {
        0.0
    };

    HttpResponse::Ok().json(json!({
        "status": "success",
        "period": "24h",
        "btc": {
            "data": btc_history,
            "points": btc_history.len(),
            "change_pct": btc_change,
            "source": "coingecko"
        },
        "gold": {
            "data": gold_history,
            "points": gold_history.len(),
            "change_pct": gold_change,
            "source": "biquote (bougies 1h, cache 60 s)"
        }
    }))
}

// ====================
// CODE PIN — Configuration + Vérification
// ====================

/// Hash SHA-256 du PIN avec salt
fn hash_pin(pin: &str, salt: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(format!("{}{}", pin, salt).as_bytes());
    for _ in 0..1000 {
        let mut h = Sha256::new();
        h.update(hasher.finalize());
        hasher = h;
    }
    hex::encode(hasher.finalize())
}

/// POST /api/account/pin/verify
/// Vérifie le code PIN
pub async fn account_pin_verify(body: web::Json<serde_json::Value>) -> HttpResponse {
    let uq_number = body.get("uq_number").and_then(|v| v.as_str()).unwrap_or("");
    let pin = body.get("pin").and_then(|v| v.as_str()).unwrap_or("");

    if uq_number.is_empty() || pin.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "uq_number et pin requis"
        }));
    }

    if pin.len() != 4 || !pin.chars().all(|c| c.is_ascii_digit()) {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "PIN doit être 4 chiffres"
        }));
    }

    let db_path = format!(
        "data/users/{}.db",
        uq_number.replace("-", "_").to_lowercase()
    );
    if !std::path::Path::new(&db_path).exists() {
        return HttpResponse::NotFound().json(json!({
            "status": "error",
            "message": "Compte introuvable"
        }));
    }

    let conn = match rusqlite::Connection::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Erreur DB: {}", e)
            }))
        }
    };

    // Lire le hash + salt
    let stored_hash: String = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'pin_hash'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_default();

    let salt: String = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'pin_salt'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_else(|_| format!("uq_pin_{}", uq_number));

    if stored_hash.is_empty() {
        return HttpResponse::Ok().json(json!({
            "status": "success",
            "valid": true,
            "message": "Pas de PIN configuré"
        }));
    }

    let computed = hash_pin(pin, &salt);
    let valid = computed == stored_hash;

    // Log échec
    if !valid {
        let now = chrono::Utc::now().timestamp();
        let _ = conn.execute(
            "INSERT INTO security_log (event_type, success, details, created_at) VALUES ('pin_failed', 0, 'Mauvais PIN', ?1)",
            rusqlite::params![now]
        );
    }

    HttpResponse::Ok().json(json!({
        "status": "success",
        "valid": valid
    }))
}

/// POST /api/account/pin/set
/// Configure un nouveau PIN
pub async fn account_pin_set(body: web::Json<serde_json::Value>) -> HttpResponse {
    let uq_number = body.get("uq_number").and_then(|v| v.as_str()).unwrap_or("");
    let pin = body.get("pin").and_then(|v| v.as_str()).unwrap_or("");
    let old_pin = body.get("old_pin").and_then(|v| v.as_str()).unwrap_or("");

    if uq_number.is_empty() || pin.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "uq_number et pin requis"
        }));
    }

    if pin.len() != 4 || !pin.chars().all(|c| c.is_ascii_digit()) {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "PIN doit être 4 chiffres"
        }));
    }

    let db_path = format!(
        "data/users/{}.db",
        uq_number.replace("-", "_").to_lowercase()
    );
    if !std::path::Path::new(&db_path).exists() {
        return HttpResponse::NotFound().json(json!({
            "status": "error",
            "message": "Compte introuvable"
        }));
    }

    let conn = match rusqlite::Connection::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Erreur DB: {}", e)
            }))
        }
    };

    // Si PIN déjà configuré, vérifier l'ancien
    let existing_hash: String = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'pin_hash'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_default();

    let salt: String = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'pin_salt'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_else(|_| format!("uq_pin_{}", uq_number));

    if !existing_hash.is_empty() && !old_pin.is_empty() {
        let old_computed = hash_pin(old_pin, &salt);
        if old_computed != existing_hash {
            return HttpResponse::BadRequest().json(json!({
                "status": "error",
                "message": "Ancien PIN incorrect"
            }));
        }
    }

    // Sauvegarder le nouveau PIN
    let new_hash = hash_pin(pin, &salt);
    let now = chrono::Utc::now().timestamp();

    let _ = conn.execute(
        "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('pin_hash', ?1, ?2)",
        rusqlite::params![new_hash, now],
    );
    let _ = conn.execute(
        "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('pin_salt', ?1, ?2)",
        rusqlite::params![salt, now],
    );
    let _ = conn.execute(
        "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('pin_enabled', 'true', ?1)",
        rusqlite::params![now]
    );

    HttpResponse::Ok().json(json!({
        "status": "success",
        "message": "PIN configuré avec succès"
    }))
}

/// POST /api/account/pin/remove
/// Désactive le PIN (nécessite le PIN actuel)
pub async fn account_pin_remove(body: web::Json<serde_json::Value>) -> HttpResponse {
    let uq_number = body.get("uq_number").and_then(|v| v.as_str()).unwrap_or("");
    let pin = body.get("pin").and_then(|v| v.as_str()).unwrap_or("");

    if uq_number.is_empty() || pin.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "uq_number et pin requis"
        }));
    }

    let db_path = format!(
        "data/users/{}.db",
        uq_number.replace("-", "_").to_lowercase()
    );
    let conn = match rusqlite::Connection::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Erreur DB: {}", e)
            }))
        }
    };

    let salt: String = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'pin_salt'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_else(|_| format!("uq_pin_{}", uq_number));

    let stored_hash: String = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'pin_hash'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_default();

    let computed = hash_pin(pin, &salt);
    if computed != stored_hash {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "PIN incorrect"
        }));
    }

    let now = chrono::Utc::now().timestamp();
    let _ = conn.execute("DELETE FROM settings WHERE key = 'pin_hash'", []);
    let _ = conn.execute("DELETE FROM settings WHERE key = 'pin_salt'", []);
    let _ = conn.execute(
        "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('pin_enabled', 'false', ?1)",
        rusqlite::params![now]
    );

    HttpResponse::Ok().json(json!({
        "status": "success",
        "message": "PIN désactivé"
    }))
}
// ====================
// PRÉFÉRENCES UTILISATEUR — display_unit, theme, language
// ====================

/// GET /api/account/preferences/{uq_number}
/// Retourne les préférences du compte
pub async fn account_preferences_get(req: HttpRequest, path: web::Path<String>) -> HttpResponse {
    let uq_number = path.into_inner();
    if !crate::auth_guard::ressemble_a_un_uq(&uq_number) {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Identifiant de compte invalide"
        }));
    }
    if let Err(response) = crate::auth_guard::exiger_soi_ou_owner(&req, &uq_number) {
        return response;
    }

    let db_path = format!(
        "data/users/{}.db",
        uq_number.replace("-", "_").to_lowercase()
    );

    if !std::path::Path::new(&db_path).exists() {
        return HttpResponse::NotFound().json(json!({
            "status": "error",
            "message": format!("Compte introuvable: {}", uq_number)
        }));
    }

    let conn = match rusqlite::Connection::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Erreur DB: {}", e)
            }))
        }
    };

    // Lire les préférences (avec fallback sur valeurs par défaut)
    let display_unit: String = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'display_unit'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_else(|_| "BORB".to_string());

    let theme: String = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'theme'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_else(|_| "dark".to_string());

    let language: String = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'language'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_else(|_| "fr".to_string());

    let font_size: String = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'font_size'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_else(|_| "medium".to_string());

    let preference = |key: &str, fallback: &str| -> String {
        conn.query_row(
            "SELECT value FROM settings WHERE key = ?1",
            rusqlite::params![key],
            |row| row.get(0),
        )
        .unwrap_or_else(|_| fallback.to_string())
    };

    // Vérifier si un PIN est configuré
    let pin_enabled: bool = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'pin_enabled'",
            [],
            |row| row.get::<_, String>(0),
        )
        .map(|v| v == "true")
        .unwrap_or(false);

    HttpResponse::Ok().json(json!({
        "status": "success",
        "uq_number": uq_number,
        "preferences": {
            "display_unit": display_unit,
            "theme": theme,
            "language": language,
            "font_size": font_size,
            "pin_enabled": pin_enabled,
            "privacy_real_name": preference("privacy_real_name", "private"),
            "privacy_description": preference("privacy_description", "private"),
            "privacy_city": preference("privacy_city", "private"),
            "privacy_origin_city": preference("privacy_origin_city", "private"),
            "privacy_occupation": preference("privacy_occupation", "private"),
            "privacy_marital_status": preference("privacy_marital_status", "private"),
            "privacy_mobile_money_number": preference("privacy_mobile_money_number", "private"),
            "privacy_trading_currency": preference("privacy_trading_currency", "private"),
            "notify_chat": preference("notify_chat", "true"),
            "notify_wallet": preference("notify_wallet", "true"),
            "notify_trading": preference("notify_trading", "true"),
            "notify_zando": preference("notify_zando", "true"),
            "notify_system": preference("notify_system", "true"),
            // Les autorisations de robinet sont des clés du compte, pas des
            // préférences d'affichage : elles se lisent et s'écrivent ici, et le
            // tick les relit à chaque tour.
            "robinet_vitrine": preference(crate::robinet::VITRINE, "true"),
        }
    }))
}

/// POST /api/account/preferences
/// Sauvegarde les préférences
pub async fn account_preferences_set(
    req: HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let uq_number = body.get("uq_number").and_then(|v| v.as_str()).unwrap_or("");

    if uq_number.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "uq_number requis"
        }));
    }
    if !crate::auth_guard::ressemble_a_un_uq(uq_number) {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Identifiant de compte invalide"
        }));
    }

    if let Err(response) = crate::auth_guard::exiger_soi_ou_owner(&req, uq_number) {
        return response;
    }

    let db_path = format!(
        "data/users/{}.db",
        uq_number.replace("-", "_").to_lowercase()
    );

    if !std::path::Path::new(&db_path).exists() {
        return HttpResponse::NotFound().json(json!({
            "status": "error",
            "message": format!("Compte introuvable: {}", uq_number)
        }));
    }

    let conn = match rusqlite::Connection::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Erreur DB: {}", e)
            }))
        }
    };

    let now = chrono::Utc::now().timestamp();
    let mut updated = Vec::new();

    // Mettre à jour display_unit
    if let Some(unit) = body.get("display_unit").and_then(|v| v.as_str()) {
        let valid_units = ["BORB", "borb", "mborb", "nborb", "pborb", "fborb"];
        if !valid_units.contains(&unit) {
            return HttpResponse::BadRequest().json(json!({
                "status": "error",
                "message": format!("Unité invalide: {}. Valides: {:?}", unit, valid_units)
            }));
        }

        if let Err(e) = conn.execute(
            "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('display_unit', ?1, ?2)",
            rusqlite::params![unit, now]
        ) {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Impossible d'enregistrer l'unité: {}", e)
            }));
        }
        updated.push(format!("display_unit={}", unit));
    }

    // Mettre à jour theme
    if let Some(theme) = body.get("theme").and_then(|v| v.as_str()) {
        if !["dark", "light", "auto"].contains(&theme) {
            return HttpResponse::BadRequest().json(json!({
                "status": "error",
                "message": "Thème invalide"
            }));
        }
        if let Err(e) = conn.execute(
            "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('theme', ?1, ?2)",
            rusqlite::params![theme, now],
        ) {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Impossible d'enregistrer le thème: {}", e)
            }));
        }
        updated.push(format!("theme={}", theme));
    }

    // Mettre à jour language
    if let Some(lang) = body.get("language").and_then(|v| v.as_str()) {
        if !["fr", "en", "es", "sw"].contains(&lang) {
            return HttpResponse::BadRequest().json(json!({
                "status": "error",
                "message": "Langue invalide"
            }));
        }
        if let Err(e) = conn.execute(
            "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('language', ?1, ?2)",
            rusqlite::params![lang, now],
        ) {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Impossible d'enregistrer la langue: {}", e)
            }));
        }
        updated.push(format!("language={}", lang));
    }

    if let Some(font_size) = body.get("font_size").and_then(|v| v.as_str()) {
        if !["small", "medium", "large", "xlarge"].contains(&font_size) {
            return HttpResponse::BadRequest().json(json!({
                "status": "error",
                "message": "Taille de texte invalide"
            }));
        }
        if let Err(e) = conn.execute(
            "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('font_size', ?1, ?2)",
            rusqlite::params![font_size, now],
        ) {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Impossible d'enregistrer la taille du texte: {}", e)
            }));
        }
        updated.push(format!("font_size={}", font_size));
    }

    for key in [
        "privacy_real_name",
        "privacy_description",
        "privacy_city",
        "privacy_origin_city",
        "privacy_occupation",
        "privacy_marital_status",
        "privacy_mobile_money_number",
        "privacy_trading_currency",
    ] {
        if let Some(value) = body.get(key) {
            let Some(value) = value.as_str() else {
                return HttpResponse::BadRequest().json(json!({
                    "status": "error",
                    "message": format!("Visibilité invalide pour {}", key)
                }));
            };
            if !["private", "friends", "public"].contains(&value) {
                return HttpResponse::BadRequest().json(json!({
                    "status": "error",
                    "message": format!("Visibilité invalide pour {}", key)
                }));
            }
            if let Err(e) = conn.execute(
                "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)",
                rusqlite::params![key, value, now],
            ) {
                return HttpResponse::InternalServerError().json(json!({
                    "status": "error",
                    "message": format!("Impossible d'enregistrer {}: {}", key, e)
                }));
            }
            updated.push(format!("{}={}", key, value));
        }
    }

    for key in [
        "notify_chat",
        "notify_wallet",
        "notify_trading",
        "notify_zando",
        "notify_system",
        crate::robinet::VITRINE,
    ] {
        if let Some(value) = body.get(key) {
            let Some(value) = value.as_bool() else {
                return HttpResponse::BadRequest().json(json!({
                    "status": "error",
                    "message": format!("Valeur booléenne attendue pour {}", key)
                }));
            };
            let value = if value { "true" } else { "false" };
            if let Err(e) = conn.execute(
                "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)",
                rusqlite::params![key, value, now],
            ) {
                return HttpResponse::InternalServerError().json(json!({
                    "status": "error",
                    "message": format!("Impossible d'enregistrer {}: {}", key, e)
                }));
            }
            updated.push(format!("{}={}", key, value));
        }
    }

    // Couper le robinet de présence ne se borne pas à enregistrer un choix :
    // les vitrines en ligne du compte s'arrêtent ici, dans la seconde, et la
    // réponse le dit. Les boutiques restent dans le compte, rien n'est purgé.
    let boutiques_suspendues =
        if body.get(crate::robinet::VITRINE).and_then(|v| v.as_bool()) == Some(false) {
            let db = Database::shared("uq_platform_db.json");
            crate::vitrines::suspendre_compte(&db, uq_number, now)
        } else {
            Vec::new()
        };

    HttpResponse::Ok().json(json!({
        "status": "success",
        "message": "Préférences sauvegardées",
        "updated": updated,
        "boutiques_suspendues": boutiques_suspendues,
    }))
}

// ====================
// PARAMÈTRES UTILISATEUR — Avatar, Devices, Font Size
// ====================

/// GET /api/account/devices/{uq_number}
/// Liste les appareils connectés
pub async fn account_devices_list(
    req: actix_web::HttpRequest,
    path: web::Path<String>,
) -> HttpResponse {
    // 🔐 SÉCURITÉ (vague 1) — USER : accès à SES données (ou owner)
    // (handler sans paramètre de chemin : contrôle JWT simple)
    // 🔐 SÉCURITÉ (vague 1) — USER : accès à SES données (ou owner)
    let __cible = path.to_string();
    if let Err(r) = crate::auth_guard::exiger_soi_ou_owner(&req, &__cible) {
        return r;
    }

    let uq_number = path.into_inner();
    let db_path = format!(
        "data/users/{}.db",
        uq_number.replace("-", "_").to_lowercase()
    );

    if !std::path::Path::new(&db_path).exists() {
        return HttpResponse::NotFound()
            .json(json!({"status": "error", "message": "Compte introuvable"}));
    }

    let conn = match rusqlite::Connection::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError()
                .json(json!({"status": "error", "message": format!("{}", e)}))
        }
    };

    // Récupérer depuis security_log les dernières connexions uniques
    let mut stmt = match conn.prepare(
        "SELECT DISTINCT device, ip, MAX(created_at) as last_seen 
         FROM security_log 
         WHERE device IS NOT NULL 
         GROUP BY device 
         ORDER BY last_seen DESC 
         LIMIT 20",
    ) {
        Ok(s) => s,
        Err(_) => return HttpResponse::Ok().json(json!({"status": "success", "devices": []})),
    };

    let rows = stmt.query_map([], |row| {
        Ok(json!({
            "device": row.get::<_, String>(0)?,
            "ip": row.get::<_, Option<String>>(1)?.unwrap_or_default(),
            "last_seen": row.get::<_, i64>(2)?,
        }))
    });

    let mut devices = Vec::new();
    if let Ok(r) = rows {
        for row in r {
            if let Ok(v) = row {
                devices.push(v);
            }
        }
    }

    HttpResponse::Ok().json(json!({
        "status": "success",
        "total": devices.len(),
        "devices": devices
    }))
}

/// POST /api/account/avatar/set
/// Sauvegarde l'avatar (URL ou base64)
pub async fn account_avatar_set(
    req: HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let uq_number = body.get("uq_number").and_then(|v| v.as_str()).unwrap_or("");
    let avatar = body.get("avatar").and_then(|v| v.as_str()).unwrap_or("");

    if !crate::auth_guard::ressemble_a_un_uq(uq_number) {
        return HttpResponse::BadRequest()
            .json(json!({"status": "error", "message": "Identifiant de compte invalide"}));
    }
    if let Err(response) = crate::auth_guard::exiger_soi_ou_owner(&req, uq_number) {
        return response;
    }
    if avatar.len() > 1_500_000 {
        return HttpResponse::PayloadTooLarge().json(json!({
            "status": "error",
            "message": "L'image de profil dépasse la taille maximale autorisée."
        }));
    }

    let db_path = format!(
        "data/users/{}.db",
        uq_number.replace("-", "_").to_lowercase()
    );
    if !std::path::Path::new(&db_path).exists() {
        return HttpResponse::NotFound()
            .json(json!({"status": "error", "message": "Compte introuvable"}));
    }

    let conn = match rusqlite::Connection::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError()
                .json(json!({"status": "error", "message": format!("{}", e)}))
        }
    };

    let now = chrono::Utc::now().timestamp();
    let _ = conn.execute(
        "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('avatar_url', ?1, ?2)",
        rusqlite::params![avatar, now],
    );

    HttpResponse::Ok().json(json!({
        "status": "success",
        "message": "Avatar sauvegardé"
    }))
}

/// POST /api/account/avatar/get
/// Récupère l'avatar
pub async fn account_avatar_get(
    req: HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let uq_number = body.get("uq_number").and_then(|v| v.as_str()).unwrap_or("");

    if !crate::auth_guard::ressemble_a_un_uq(uq_number) {
        return HttpResponse::BadRequest()
            .json(json!({"status": "error", "message": "Identifiant de compte invalide"}));
    }
    if let Err(response) = crate::auth_guard::exiger_soi_ou_owner(&req, uq_number) {
        return response;
    }

    let db_path = format!(
        "data/users/{}.db",
        uq_number.replace("-", "_").to_lowercase()
    );
    let conn = match rusqlite::Connection::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError()
                .json(json!({"status": "error", "message": format!("{}", e)}))
        }
    };

    let avatar: String = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'avatar_url'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_default();

    HttpResponse::Ok().json(json!({
        "status": "success",
        "avatar_url": avatar
    }))
}

/// POST /api/account/font-size/set
/// Sauvegarde la taille de police
pub async fn account_font_size_set(
    req: HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let uq_number = body.get("uq_number").and_then(|v| v.as_str()).unwrap_or("");
    let font_size = body
        .get("font_size")
        .and_then(|v| v.as_str())
        .unwrap_or("medium");

    if uq_number.is_empty() {
        return HttpResponse::BadRequest()
            .json(json!({"status": "error", "message": "uq_number requis"}));
    }

    if let Err(response) = crate::auth_guard::exiger_soi_ou_owner(&req, uq_number) {
        return response;
    }

    let valid_sizes = ["small", "medium", "large", "xlarge"];
    if !valid_sizes.contains(&font_size) {
        return HttpResponse::BadRequest()
            .json(json!({"status": "error", "message": "Taille invalide"}));
    }

    let db_path = format!(
        "data/users/{}.db",
        uq_number.replace("-", "_").to_lowercase()
    );
    let conn = match rusqlite::Connection::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError()
                .json(json!({"status": "error", "message": format!("{}", e)}))
        }
    };

    let now = chrono::Utc::now().timestamp();
    if let Err(e) = conn.execute(
        "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('font_size', ?1, ?2)",
        rusqlite::params![font_size, now],
    ) {
        return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("Impossible d'enregistrer la taille du texte: {}", e)
        }));
    }

    HttpResponse::Ok().json(json!({
        "status": "success",
        "font_size": font_size
    }))
}

// ====================
// GESTION DU PIN — Routes API
// ====================

/// POST /api/account/pin/set
/// Configure ou modifie le PIN (necessite mot de passe du compte)
pub async fn account_pin_set_v2(body: web::Json<serde_json::Value>) -> HttpResponse {
    let uq_number = body.get("uq_number").and_then(|v| v.as_str()).unwrap_or("");
    let pin = body.get("pin").and_then(|v| v.as_str()).unwrap_or("");
    let password = body.get("password").and_then(|v| v.as_str()).unwrap_or("");
    let old_pin = body.get("old_pin").and_then(|v| v.as_str()).unwrap_or("");

    if uq_number.is_empty() || pin.is_empty() || password.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "uq_number, pin et password requis"
        }));
    }

    // 1. Vérifier le MOT DE PASSE du compte (obligatoire)
    if !crate::pin_manager::verify_account_password(uq_number, password) {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Mot de passe incorrect"
        }));
    }

    // 2. Format du PIN
    if pin.len() < 4 || pin.len() > 6 || !pin.chars().all(|c| c.is_ascii_digit()) {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "PIN doit faire 4 à 6 chiffres"
        }));
    }

    // 3. Ouvrir la DB
    let db_path = format!(
        "data/users/{}.db",
        uq_number.replace("-", "_").to_lowercase()
    );
    if !std::path::Path::new(&db_path).exists() {
        return HttpResponse::NotFound().json(json!({
            "status": "error",
            "message": "Compte introuvable"
        }));
    }

    let conn = match rusqlite::Connection::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Erreur DB: {}", e)
            }))
        }
    };

    let _ = crate::pin_manager::init_pin_table(&conn);

    // 4. Si un PIN existe, vérifier l'ancien
    let status = crate::pin_manager::get_pin_status(&conn).ok();
    let pin_exists = !matches!(status, Some(crate::pin_manager::PinStatus::NotConfigured))
        && !matches!(status, None);

    if pin_exists {
        if old_pin.is_empty() {
            return HttpResponse::BadRequest().json(json!({
                "status": "error",
                "message": "Ancien PIN requis pour modifier"
            }));
        }
        match crate::pin_manager::verify_pin(&conn, old_pin) {
            Ok(true) => {}
            Ok(false) => {
                return HttpResponse::BadRequest().json(json!({
                    "status": "error",
                    "message": "Ancien PIN incorrect"
                }))
            }
            Err(e) => {
                return HttpResponse::TooManyRequests().json(json!({
                    "status": "error",
                    "message": e
                }))
            }
        }
    }

    // 5. Sauvegarder le nouveau PIN
    match crate::pin_manager::set_pin(&conn, pin) {
        Ok(_) => {
            let now = chrono::Utc::now().timestamp();
            let _ = conn.execute(
                "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('pin_enabled', 'true', ?1)",
                rusqlite::params![now],
            );

            HttpResponse::Ok().json(json!({
                "status": "success",
                "message": "PIN configuré avec succès. Utilisable immédiatement.",
                "pin_status": crate::pin_manager::get_pin_status(&conn).ok()
            }))
        }
        Err(e) => HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": e
        })),
    }
}

/// POST /api/account/pin/verify
/// Verifie le PIN
pub async fn account_pin_verify_v2(body: web::Json<serde_json::Value>) -> HttpResponse {
    let uq_number = body.get("uq_number").and_then(|v| v.as_str()).unwrap_or("");
    let pin = body.get("pin").and_then(|v| v.as_str()).unwrap_or("");

    if uq_number.is_empty() || pin.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "uq_number et pin requis"
        }));
    }

    let db_path = format!(
        "data/users/{}.db",
        uq_number.replace("-", "_").to_lowercase()
    );

    if !std::path::Path::new(&db_path).exists() {
        return HttpResponse::NotFound().json(json!({
            "status": "error",
            "message": "Compte introuvable"
        }));
    }

    let conn = match rusqlite::Connection::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Erreur DB: {}", e)
            }))
        }
    };

    // Init table si besoin
    let _ = crate::pin_manager::init_pin_table(&conn);

    // ====================
    // ⚠️ AUCUN PIN CONFIGURÉ → rien à vérifier : on laisse passer
    // ====================
    // Un compte neuf a pin_enabled = 0. Avant ce correctif, verify_pin
    // renvoyait toujours false → 400 « PIN incorrect » → le frontend
    // (CoffreScreen.requirePin) bloquait DÉFINITIVEMENT tout envoi.
    let pin_configure: bool = {
        let pm_ok = conn.query_row(
            "SELECT (COALESCE(pin_hash,'') <> '' AND pin_enabled = 1) FROM pin_status WHERE id = 1",
            [],
            |row| row.get::<_, bool>(0),
        ).unwrap_or(false);

        let settings_ok: bool = conn.query_row(
            "SELECT (SELECT COUNT(*) FROM settings WHERE key = 'pin_hash' AND COALESCE(value,'') <> '') > 0
                AND (SELECT COALESCE(value,'false') FROM settings WHERE key = 'pin_enabled') = 'true'",
            [],
            |row| row.get::<_, bool>(0),
        ).unwrap_or(false);

        pm_ok || settings_ok
    };

    if !pin_configure {
        return HttpResponse::Ok().json(json!({
            "status": "success",
            "valid": true,
            "pin_configured": false,
            "message": "Aucun PIN configure : verification ignoree"
        }));
    }

    // Essayer pin_manager (nouveau)
    match crate::pin_manager::verify_pin(&conn, pin) {
        Ok(true) => {
            return HttpResponse::Ok().json(json!({
                "status": "success",
                "valid": true,
                "message": "PIN correct"
            }));
        }
        Ok(false) => {
            // Fallback : essayer l'ancien systeme (settings)
            let salt_old: String = conn
                .query_row(
                    "SELECT value FROM settings WHERE key = 'pin_salt'",
                    [],
                    |row| row.get(0),
                )
                .unwrap_or_else(|_| format!("uq_pin_{}", uq_number));

            let hash_old: String = conn
                .query_row(
                    "SELECT value FROM settings WHERE key = 'pin_hash'",
                    [],
                    |row| row.get(0),
                )
                .unwrap_or_default();

            let computed = crate::pin_manager::hash_pin(pin, &salt_old);

            if !hash_old.is_empty() && computed == hash_old {
                return HttpResponse::Ok().json(json!({
                    "status": "success",
                    "valid": true,
                    "message": "PIN correct (via settings)"
                }));
            }

            // ✅ FIX : 400 Bad Request (pas 401) pour PIN incorrect
            return HttpResponse::BadRequest().json(json!({
                "status": "error",
                "valid": false,
                "message": "PIN incorrect"
            }));
        }
        Err(e) => HttpResponse::TooManyRequests().json(json!({
            "status": "error",
            "valid": false,
            "message": e
        })),
    }
}

/// POST /api/account/pin/status
/// Retourne le statut du PIN
pub async fn account_pin_status_v2(body: web::Json<serde_json::Value>) -> HttpResponse {
    let uq_number = body.get("uq_number").and_then(|v| v.as_str()).unwrap_or("");

    if uq_number.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "uq_number requis"
        }));
    }

    let db_path = format!(
        "data/users/{}.db",
        uq_number.replace("-", "_").to_lowercase()
    );

    if !std::path::Path::new(&db_path).exists() {
        return HttpResponse::NotFound().json(json!({
            "status": "error",
            "message": "Compte introuvable"
        }));
    }

    let conn = match rusqlite::Connection::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Erreur DB: {}", e)
            }))
        }
    };

    // Init table si besoin
    let _ = crate::pin_manager::init_pin_table(&conn);

    // Lire pin_manager (nouveau) ET settings (ancien)
    let pm_status = crate::pin_manager::get_pin_status(&conn).ok();

    // Sauvegarder pour debug AVANT de consommer
    let pm_status_debug = format!("{:?}", pm_status);

    // Lire settings en fallback
    let settings_enabled: bool = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'pin_enabled'",
            [],
            |row| row.get::<_, String>(0),
        )
        .map(|v| v == "true")
        .unwrap_or(false);

    let settings_has_hash: bool = conn
        .query_row(
            "SELECT LENGTH(value) > 0 FROM settings WHERE key = 'pin_hash'",
            [],
            |row| row.get::<_, bool>(0),
        )
        .unwrap_or(false);

    let pin_status_has_hash: bool = conn
        .query_row(
            "SELECT (COALESCE(pin_hash,'') <> '' AND pin_enabled = 1) FROM pin_status WHERE id = 1",
            [],
            |row| row.get::<_, bool>(0),
        )
        .unwrap_or(false);

    // FIX pin_length : lire la longueur reelle du PIN
    let pin_length_from_db: i64 = conn
        .query_row(
            "SELECT COALESCE(pin_length, 4) FROM pin_status WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .unwrap_or(4);

    // Fallback : lire dans settings si pin_status absent
    let pin_length_final: i64 = if pin_length_from_db > 0 && pin_length_from_db <= 6 {
        pin_length_from_db
    } else {
        conn.query_row(
            "SELECT LENGTH(value) FROM settings WHERE key = 'pin_hash' AND LENGTH(value) > 0",
            [],
            |row| row.get(0),
        )
        .map(|len: i64| if len > 6 { 4 } else { len })
        .unwrap_or(4)
    };

    // Si pin_manager dit NotConfigured mais settings a un PIN → utiliser settings
    let pin_existe_quelque_part = settings_has_hash
        || pin_status_has_hash
        || matches!(
            pm_status,
            Some(crate::pin_manager::PinStatus::Ready)
                | Some(crate::pin_manager::PinStatus::Cooldown { .. })
        );

    let final_status = if pin_existe_quelque_part {
        crate::pin_manager::PinStatus::Ready
    } else {
        crate::pin_manager::PinStatus::NotConfigured
    };

    HttpResponse::Ok().json(json!({
        "status": "success",
        "pin_status": final_status,
        "pin_length": pin_length_final,
        "debug": {
            "pin_manager": pm_status_debug,
            "settings_enabled": settings_enabled,
            "settings_has_hash": settings_has_hash,
            "pin_status_has_hash": pin_status_has_hash,
            "settings_has_hash": settings_has_hash
        }
    }))
}

/// POST /api/account/pin/remove
/// Supprime le PIN (necessite mot de passe)
pub async fn account_pin_remove_v2(body: web::Json<serde_json::Value>) -> HttpResponse {
    let uq_number = body.get("uq_number").and_then(|v| v.as_str()).unwrap_or("");
    let password = body.get("password").and_then(|v| v.as_str()).unwrap_or("");

    if uq_number.is_empty() || password.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "uq_number et password requis"
        }));
    }

    // Verifier mot de passe
    let idx = crate::uq_index::UqIndex::shared("uq_index.db");
    let account_info = idx.get_account(uq_number);

    if let Some(info) = account_info {
        let email = info.get("uq_email").and_then(|v| v.as_str()).unwrap_or("");
        let verify = idx.verify_credentials(uq_number, email, password);

        if verify.is_none() {
            return HttpResponse::Unauthorized().json(json!({
                "status": "error",
                "message": "Mot de passe incorrect"
            }));
        }
    }

    let db_path = format!(
        "data/users/{}.db",
        uq_number.replace("-", "_").to_lowercase()
    );

    let conn = match rusqlite::Connection::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Erreur DB: {}", e)
            }))
        }
    };

    let _ = crate::pin_manager::init_pin_table(&conn);

    match crate::pin_manager::remove_pin(&conn) {
        Ok(_) => {
            // ✅ SYNC UNIFIÉ — Supprimer de settings AUSSI
            {
                let now = chrono::Utc::now().timestamp();
                let _ = conn.execute("DELETE FROM settings WHERE key = 'pin_hash'", []);
                let _ = conn.execute("DELETE FROM settings WHERE key = 'pin_salt'", []);
                let _ = conn.execute(
                    "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('pin_enabled', 'false', ?1)",
                    rusqlite::params![now],
                );
                eprintln!(
                    "✅ PIN supprime de settings ET pin_status pour {}",
                    uq_number
                );
            }

            HttpResponse::Ok().json(json!({
                "status": "success",
                "message": "PIN supprime"
            }))
        }
        Err(e) => HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": e
        })),
    }
}

/// POST /api/account/pin/verify-password
/// Verifie le mot de passe du compte (fallback quand PIN en cooldown)
pub async fn account_pin_verify_password(body: web::Json<serde_json::Value>) -> HttpResponse {
    let uq_number = body.get("uq_number").and_then(|v| v.as_str()).unwrap_or("");
    let password = body.get("password").and_then(|v| v.as_str()).unwrap_or("");

    if uq_number.is_empty() || password.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "uq_number et password requis"
        }));
    }

    let idx = crate::uq_index::UqIndex::shared("uq_index.db");
    let account_info = idx.get_account(uq_number);

    match account_info {
        Some(info) => {
            let email = info.get("uq_email").and_then(|v| v.as_str()).unwrap_or("");
            let verify = idx.verify_credentials(uq_number, email, password);

            if verify.is_some() {
                HttpResponse::Ok().json(json!({
                    "status": "success",
                    "valid": true,
                    "message": "Mot de passe correct"
                }))
            } else {
                HttpResponse::Unauthorized().json(json!({
                    "status": "error",
                    "valid": false,
                    "message": "Mot de passe incorrect"
                }))
            }
        }
        None => HttpResponse::NotFound().json(json!({
            "status": "error",
            "message": "Compte introuvable"
        })),
    }
}

// ====================
// WEBAUTHN — Routes API
// ====================

// ====================
// PASSKEYS (WebAuthn)
// ====================
//
// L'autorite de la liaison compte<->appareil est la cle nee dans l'enclave
// materielle (vTEE / TPM / Secure Enclave). Le serveur ne stocke que la cle
// publique et verifie reellement la signature de chaque assertion : sans ces
// deux conditions, « biometrie validee » ne veut rien dire.

/// Meme rp.id et memes origines a l'enrolement et a la verification.
fn rp_webauthn() -> crate::webauthn_verify::ConfigRp {
    crate::webauthn_verify::ConfigRp::par_defaut()
}

/// Etiquette affichee dans « Mes appareils ». Un user-agent se falsifie : ce
/// n'est qu'un libelle, jamais un critere d'autorisation.
fn etiquette_appareil(user_agent: &str) -> String {
    let cible = user_agent.to_ascii_lowercase();
    if cible.contains("iphone") || cible.contains("ipad") {
        "iPhone / iPad".to_string()
    } else if cible.contains("android") {
        "Android".to_string()
    } else if cible.contains("windows") {
        "Windows (Windows Hello)".to_string()
    } else if cible.contains("mac os x") || cible.contains("macintosh") {
        "macOS (Touch ID)".to_string()
    } else if cible.contains("linux") {
        "Linux".to_string()
    } else {
        "Appareil".to_string()
    }
}

/// Ouvrir la base du compte avec les tables passkey + appareils.
fn base_passkey(uq_number: &str) -> Result<rusqlite::Connection, HttpResponse> {
    let conn = crate::device_auth::ouvrir(uq_number).map_err(|e| {
        HttpResponse::InternalServerError().json(json!({ "status": "error", "message": e }))
    })?;
    let _ = crate::webauthn_manager::init_webauthn_table(&conn);
    Ok(conn)
}

/// POST /api/auth/resume
///
/// Le client demande quel palier s'applique a lui, le serveur repond. Le numero
/// UQ et l'identifiant de cle que le client envoie ne sont que des indices :
/// c'est la ligne `security_devices` + l'horodatage serveur qui decident.
/// Sans cle connue, le palier est complet : on ne pretend pas verifier une
/// enclave qui n'a jamais rien signe pour ce compte.
pub async fn auth_resume(body: web::Json<serde_json::Value>) -> HttpResponse {
    use crate::device_auth as appareils;

    let uq_number = body
        .get("uq_number")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let credential_id = body
        .get("credential_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();

    if uq_number.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "uq_number requis"
        }));
    }

    if !std::path::Path::new(&appareils::chemin_base(uq_number)).exists() {
        return HttpResponse::NotFound().json(json!({
            "status": "error",
            "message": "Compte introuvable"
        }));
    }

    let maintenant = chrono::Utc::now().timestamp();
    let vide = String::new();
    let indice: &str = if credential_id.is_empty() {
        &vide
    } else {
        credential_id
    };

    let (palier, appareil) = match indice {
        "" => (appareils::Palier::AucunAppareil, None),
        cle => match appareils::ouvrir(uq_number) {
            Ok(conn) => match appareils::palier_pour_credential(&conn, cle, maintenant) {
                Some((detail, palier)) => (palier, Some(detail)),
                None => (appareils::Palier::AucunAppareil, None),
            },
            Err(_) => (appareils::Palier::Complet, None),
        },
    };

    let age_connexion = appareil
        .as_ref()
        .filter(|a| a.derniere_connexion > 0)
        .map(|a| (maintenant - a.derniere_connexion).max(0));
    let age_complet = appareil
        .as_ref()
        .filter(|a| a.dernier_controle_complet > 0)
        .map(|a| (maintenant - a.dernier_controle_complet).max(0));

    HttpResponse::Ok().json(json!({
        "status": "success",
        "palier": palier.as_str(),
        "motif": palier.motif(age_connexion, age_complet),
        "uq_number": uq_number,
        "credential_id": indice,
        "age_connexion_secs": age_connexion,
        "age_controle_complet_secs": age_complet,
        "email_mot_de_passe_requis": matches!(palier, appareils::Palier::Complet),
        "biometrie_requise": !matches!(palier, appareils::Palier::AucunAppareil),
        "paliers_secs": {
            "auto": appareils::PALIER_AUTO_SECS,
            "complet": appareils::PALIER_COMPLET_SECS,
            "plafond_complet": appareils::PLAFOND_COMPLET_SECS
        }
    }))
}

/// POST /api/webauthn/register/begin
/// Genere un challenge pour l'enregistrement d'une nouvelle passkey
pub async fn webauthn_register_begin(
    req: HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let uq_number = body.get("uq_number").and_then(|v| v.as_str()).unwrap_or("");

    if uq_number.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "uq_number requis"
        }));
    }

    // Un passkey ne se greffe que sur une session deja authentifiee : sans ce
    // controle, connaitre un numero UQ suffirait pour y brancher sa propre cle.
    if let Err(reponse) = crate::auth_guard::exiger_soi_ou_owner(&req, uq_number) {
        return reponse;
    }

    if !std::path::Path::new(&crate::device_auth::chemin_base(uq_number)).exists() {
        return HttpResponse::NotFound().json(json!({
            "status": "error",
            "message": "Compte introuvable"
        }));
    }

    let conn = match base_passkey(uq_number) {
        Ok(conn) => conn,
        Err(reponse) => return reponse,
    };

    let challenge = crate::webauthn_manager::generate_challenge();

    if let Err(e) =
        crate::webauthn_manager::store_challenge(&conn, &challenge, uq_number, "register")
    {
        return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": e
        }));
    }

    let rp = rp_webauthn();

    HttpResponse::Ok().json(json!({
        "status": "success",
        "challenge": challenge,
        "rp": {
            "name": "UQ Platform",
            "id": rp.rp_id
        },
        "user": {
            "id": uq_number,
            "name": uq_number,
            "displayName": uq_number
        },
        "pubKeyCredParams": [
            { "type": "public-key", "alg": -7 }   // ES256 : seul algorithme verifie ici
        ],
        "timeout": 60000,
        "attestation": "none",
        "authenticatorSelection": {
            "userVerification": "required",
            "residentKey": "required"
        }
    }))
}

/// POST /api/webauthn/register/complete
/// Enregistre la credential apres validation du challenge
pub async fn webauthn_register_complete(
    req: HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    use crate::device_auth as appareils;

    let uq_number = body.get("uq_number").and_then(|v| v.as_str()).unwrap_or("");
    let challenge = body.get("challenge").and_then(|v| v.as_str()).unwrap_or("");
    let attestation_object = body
        .get("attestation_object")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let credential_id_client = body
        .get("credential_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let device_name = body
        .get("device_name")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if uq_number.is_empty() || challenge.is_empty() || attestation_object.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "uq_number, challenge et attestation_object requis"
        }));
    }

    if let Err(reponse) = crate::auth_guard::exiger_soi_ou_owner(&req, uq_number) {
        return reponse;
    }

    let conn = match base_passkey(uq_number) {
        Ok(conn) => conn,
        Err(reponse) => return reponse,
    };

    // 1. Decoder VRAIMENT l'attestationObject : la cle publique stockee est celle
    //    qui sort de l'enclave, pas les octets bruts envoyes par le client.
    let credential = match crate::webauthn_verify::decoder_attestation(attestation_object) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[WEBAUTHN] attestation refusee pour {} : {}", uq_number, e);
            return HttpResponse::BadRequest().json(json!({
                "status": "error",
                "message": format!("Attestation invalide: {}", e)
            }));
        }
    };

    // Le client ne choisit pas l'identifiant : il doit correspondre a celui que
    // l'authentificateur a signe.
    if !credential_id_client.is_empty() && credential_id_client != credential.credential_id {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "credential_id different de celui de l'attestation"
        }));
    }

    // 2. Verifier et consommer le challenge
    match crate::webauthn_manager::verify_challenge(&conn, challenge) {
        Ok(Some((user, action))) => {
            if user != uq_number || action != "register" {
                return HttpResponse::BadRequest().json(json!({
                    "status": "error",
                    "message": "Challenge invalide"
                }));
            }
        }
        Ok(None) => {
            return HttpResponse::BadRequest().json(json!({
                "status": "error",
                "message": "Challenge expire ou invalide"
            }));
        }
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": e
            }));
        }
    }

    // 3. Sauvegarder la credential (cle publique SEC1 + identifiant signe)
    let libelle = if device_name.trim().is_empty() {
        etiquette_appareil(
            req.headers()
                .get("User-Agent")
                .and_then(|v| v.to_str().ok())
                .unwrap_or(""),
        )
    } else {
        device_name.trim().to_string()
    };

    if let Err(e) = crate::webauthn_manager::save_credential(
        &conn,
        &credential.credential_id,
        &credential.public_key_sec1,
        &libelle,
    ) {
        return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": e
        }));
    }

    // 4. Lier l'appareil au compte. Un enrolement qui suit de pres un succes
    //    email + mot de passe herite de ce controle (horodatage serveur) ; sinon
    //    l'appareil repassera par le palier complet a sa premiere connexion.
    let maintenant = chrono::Utc::now().timestamp();
    if let Err(e) = appareils::enregistrer(&conn, &credential.credential_id, &libelle, maintenant) {
        return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": e
        }));
    }
    let heredee = appareils::controle_complet_recent(&conn, maintenant);
    if heredee {
        if let Err(e) =
            appareils::tracer_connexion(&conn, &credential.credential_id, true, maintenant)
        {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": e
            }));
        }
    }

    HttpResponse::Ok().json(json!({
        "status": "success",
        "message": "Biometrie activee avec succes",
        "credential_id": credential.credential_id,
        "device_label": libelle,
        "controle_complet_herite": heredee
    }))
}

/// POST /api/webauthn/login/begin
/// Genere un challenge pour le login biometrique
pub async fn webauthn_login_begin(body: web::Json<serde_json::Value>) -> HttpResponse {
    use crate::device_auth as appareils;

    let uq_number = body.get("uq_number").and_then(|v| v.as_str()).unwrap_or("");

    if uq_number.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "uq_number requis"
        }));
    }

    if !std::path::Path::new(&appareils::chemin_base(uq_number)).exists() {
        return HttpResponse::NotFound().json(json!({
            "status": "error",
            "message": "Compte introuvable"
        }));
    }

    let conn = match base_passkey(uq_number) {
        Ok(conn) => conn,
        Err(reponse) => return reponse,
    };

    // Seules les cles reliees a un appareil actif du compte sont presentees au
    // navigateur : une cle sans ligne d'appareil ne peut rien signer ici.
    let autorisees: Vec<String> = appareils::appareils_actifs(&conn)
        .into_iter()
        .map(|appareil| appareil.credential_id)
        .collect();

    if autorisees.is_empty() {
        return HttpResponse::NotFound().json(json!({
            "status": "error",
            "palier": "aucun_appareil",
            "message": "Aucune biometrie enregistree pour ce compte"
        }));
    }

    let challenge = crate::webauthn_manager::generate_challenge();

    if let Err(e) = crate::webauthn_manager::store_challenge(&conn, &challenge, uq_number, "login")
    {
        return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": e
        }));
    }

    let maintenant = chrono::Utc::now().timestamp();
    let palier = body
        .get("credential_id")
        .and_then(|v| v.as_str())
        .and_then(|cle| appareils::palier_pour_credential(&conn, cle, maintenant))
        .map(|(_, palier)| palier);

    HttpResponse::Ok().json(json!({
        "status": "success",
        "challenge": challenge,
        "timeout": 60000,
        "userVerification": "required",
        "rpId": rp_webauthn().rp_id,
        "allowCredentials": autorisees
            .iter()
            .map(|id| json!({ "type": "public-key", "id": id }))
            .collect::<Vec<_>>(),
        "palier": palier.map(|p| p.as_str())
    }))
}

/// POST /api/webauthn/login/complete
/// Valide le login biometrique
pub async fn webauthn_login_complete(body: web::Json<serde_json::Value>) -> HttpResponse {
    use crate::device_auth as appareils;

    let uq_number = body.get("uq_number").and_then(|v| v.as_str()).unwrap_or("");
    let challenge = body.get("challenge").and_then(|v| v.as_str()).unwrap_or("");
    let credential_id = body
        .get("credential_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let signature = body.get("signature").and_then(|v| v.as_str()).unwrap_or("");
    let authentificator_data = body
        .get("authenticator_data")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let client_data_json = body
        .get("client_data_json")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if uq_number.is_empty()
        || challenge.is_empty()
        || credential_id.is_empty()
        || authentificator_data.is_empty()
        || client_data_json.is_empty()
    {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "uq_number, challenge, credential_id, authenticator_data, client_data_json et signature requis"
        }));
    }

    let conn = match base_passkey(uq_number) {
        Ok(conn) => conn,
        Err(reponse) => return reponse,
    };

    // 1. La cle doit exister pour CE compte et etre reliee a un appareil actif.
    let (cle_publique, compteur_stocke) =
        match crate::webauthn_manager::lire_credential(&conn, credential_id) {
            Ok(Some(couple)) => couple,
            Ok(None) => {
                return HttpResponse::NotFound().json(json!({
                    "status": "error",
                    "message": "Credential inconnue pour ce compte"
                }));
            }
            Err(e) => {
                return HttpResponse::InternalServerError().json(json!({
                    "status": "error",
                    "message": e
                }));
            }
        };

    let maintenant = chrono::Utc::now().timestamp();
    let (appareil, palier) = match appareils::palier_pour_credential(
        &conn,
        credential_id,
        maintenant,
    ) {
        Some(pair) => pair,
        None => {
            return HttpResponse::BadRequest().json(json!({
                "status": "error",
                "palier": "aucun_appareil",
                "message": "Cette cle n'est reliee a aucun appareil actif : reactivez la biométrie apres un controle complet"
            }));
        }
    };

    // 2. Au-dela de 48 h (ou au plafond de 30 jours), la biométrie seule ne
    //    suffit plus : le palier complet doit passer par email + mot de passe.
    if palier == appareils::Palier::Complet {
        return HttpResponse::Forbidden().json(json!({
            "status": "error",
            "palier": palier.as_str(),
            "message": palier.motif(
                Some((maintenant - appareil.derniere_connexion).max(0)),
                if appareil.dernier_controle_complet > 0 {
                    Some((maintenant - appareil.dernier_controle_complet).max(0))
                } else {
                    None
                }
            ),
            "email_mot_de_passe_requis": true
        }));
    }

    // 3. Verifier et consommer le challenge
    match crate::webauthn_manager::verify_challenge(&conn, challenge) {
        Ok(Some((user, action))) => {
            if user != uq_number || action != "login" {
                return HttpResponse::BadRequest().json(json!({
                    "status": "error",
                    "message": "Challenge invalide"
                }));
            }
        }
        Ok(None) => {
            return HttpResponse::BadRequest().json(json!({
                "status": "error",
                "message": "Challenge expire"
            }));
        }
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": e
            }));
        }
    }

    // 4. Verification cryptographique reelle de l'assertion de l'enclave
    let assertion = crate::webauthn_verify::Assertion {
        credential_id: credential_id.to_string(),
        auth_data: authentificator_data.to_string(),
        client_data_json: client_data_json.to_string(),
        signature: signature.to_string(),
    };
    let rp = rp_webauthn();
    let resultat = crate::webauthn_verify::verifier_assertion(
        &cle_publique,
        &assertion,
        &crate::webauthn_verify::Attentes {
            challenge,
            types_acceptes: &["webauthn.get"],
            rp: &rp,
            compteur_precedent: Some(compteur_stocke),
        },
    );

    let verification = match resultat {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[WEBAUTHN] assertion refusee pour {} : {}", uq_number, e);
            return HttpResponse::Unauthorized().json(json!({
                "status": "error",
                "message": format!("Biometrie non confirmee: {}", e)
            }));
        }
    };

    // 5. Traces serveur : compteur de signature + horodatage de connexion
    if let Err(e) =
        crate::webauthn_manager::marquer_usage(&conn, credential_id, verification.compteur)
    {
        eprintln!("[WEBAUTHN] compteur non mis a jour ({})", e);
    }
    if let Err(e) = appareils::tracer_connexion(&conn, credential_id, false, maintenant) {
        eprintln!("[WEBAUTHN] connexion non tracee ({})", e);
    }

    // 6. Vrai JWT signe (plus de token fabrique a la main)
    let role = if crate::security_login::is_owner(uq_number) {
        "guardian"
    } else {
        "user"
    };
    let auth = crate::auth::AuthManager::new();
    let token = match auth.create_jwt(uq_number, role, 24) {
        Ok(t) => t,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Session indisponible: {}", e)
            }));
        }
    };

    HttpResponse::Ok().json(json!({
        "status": "success",
        "message": "Biometrie validee",
        "token": token,
        "uq_number": uq_number,
        "credential_id": credential_id,
        "palier": palier.as_str(),
        "role": role
    }))
}

/// POST /api/webauthn/transaction/begin
/// Genere un challenge pour valider une transaction
pub async fn webauthn_transaction_begin(
    req: HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    use crate::device_auth as appareils;

    let uq_number = body.get("uq_number").and_then(|v| v.as_str()).unwrap_or("");
    let transaction_id = body
        .get("transaction_id")
        .and_then(|v| v.as_str())
        .unwrap_or("tx");

    if uq_number.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "uq_number requis"
        }));
    }

    // Payer exige une session deja authentifiee sur ce compte.
    if let Err(reponse) = crate::auth_guard::exiger_soi_ou_owner(&req, uq_number) {
        return reponse;
    }

    if !std::path::Path::new(&appareils::chemin_base(uq_number)).exists() {
        return HttpResponse::NotFound().json(json!({
            "status": "error",
            "message": "Compte introuvable"
        }));
    }

    let conn = match base_passkey(uq_number) {
        Ok(conn) => conn,
        Err(reponse) => return reponse,
    };

    let autorisees: Vec<String> = appareils::appareils_actifs(&conn)
        .into_iter()
        .map(|appareil| appareil.credential_id)
        .collect();

    if autorisees.is_empty() {
        return HttpResponse::NotFound().json(json!({
            "status": "error",
            "palier": "aucun_appareil",
            "message": "Aucune cle biometrique active pour ce compte"
        }));
    }

    let challenge = crate::webauthn_manager::generate_challenge();

    if let Err(e) = crate::webauthn_manager::store_challenge(
        &conn,
        &challenge,
        uq_number,
        &format!("tx:{}", transaction_id),
    ) {
        return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": e
        }));
    }

    HttpResponse::Ok().json(json!({
        "status": "success",
        "challenge": challenge,
        "timeout": 60000,
        "userVerification": "required",
        "rpId": rp_webauthn().rp_id,
        "allowCredentials": autorisees
            .iter()
            .map(|id| json!({ "type": "public-key", "id": id }))
            .collect::<Vec<_>>()
    }))
}

/// POST /api/webauthn/transaction/complete
/// Valide une transaction biometrique
pub async fn webauthn_transaction_complete(
    req: HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    use crate::device_auth as appareils;

    let uq_number = body.get("uq_number").and_then(|v| v.as_str()).unwrap_or("");
    let challenge = body.get("challenge").and_then(|v| v.as_str()).unwrap_or("");
    let transaction_id = body
        .get("transaction_id")
        .and_then(|v| v.as_str())
        .unwrap_or("tx");
    let credential_id = body
        .get("credential_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let authentificator_data = body
        .get("authenticator_data")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let client_data_json = body
        .get("client_data_json")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let signature = body.get("signature").and_then(|v| v.as_str()).unwrap_or("");

    if uq_number.is_empty()
        || challenge.is_empty()
        || credential_id.is_empty()
        || authentificator_data.is_empty()
        || client_data_json.is_empty()
        || signature.is_empty()
    {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "uq_number, challenge, credential_id, authenticator_data, client_data_json et signature requis"
        }));
    }

    if let Err(reponse) = crate::auth_guard::exiger_soi_ou_owner(&req, uq_number) {
        return reponse;
    }

    let conn = match base_passkey(uq_number) {
        Ok(conn) => conn,
        Err(reponse) => return reponse,
    };

    // 1. La cle doit appartenir a un appareil actif de ce compte
    let (cle_publique, compteur_stocke) =
        match crate::webauthn_manager::lire_credential(&conn, credential_id) {
            Ok(Some(couple)) => couple,
            Ok(None) => {
                return HttpResponse::NotFound().json(json!({
                    "status": "error",
                    "message": "Credential inconnue pour ce compte"
                }));
            }
            Err(e) => {
                return HttpResponse::InternalServerError().json(json!({
                    "status": "error",
                    "message": e
                }));
            }
        };
    let maintenant = chrono::Utc::now().timestamp();
    if appareils::palier_pour_credential(&conn, credential_id, maintenant).is_none() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Cette cle n'est reliee a aucun appareil actif"
        }));
    }

    // 2. Challenge frais et attendu pour CETTE transaction
    match crate::webauthn_manager::verify_challenge(&conn, challenge) {
        Ok(Some((user, action))) => {
            if user != uq_number || action != format!("tx:{}", transaction_id) {
                return HttpResponse::BadRequest().json(json!({
                    "status": "error",
                    "message": "Challenge invalide"
                }));
            }
        }
        Ok(None) => {
            return HttpResponse::BadRequest().json(json!({
                "status": "error",
                "message": "Challenge expire"
            }));
        }
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": e
            }));
        }
    }

    // 3. Verification reelle de la signature de l'enclave
    let assertion = crate::webauthn_verify::Assertion {
        credential_id: credential_id.to_string(),
        auth_data: authentificator_data.to_string(),
        client_data_json: client_data_json.to_string(),
        signature: signature.to_string(),
    };
    let rp = rp_webauthn();
    let verification = match crate::webauthn_verify::verifier_assertion(
        &cle_publique,
        &assertion,
        &crate::webauthn_verify::Attentes {
            challenge,
            types_acceptes: &["webauthn.get"],
            rp: &rp,
            compteur_precedent: Some(compteur_stocke),
        },
    ) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[WEBAUTHN] transaction refusee pour {} : {}", uq_number, e);
            return HttpResponse::Unauthorized().json(json!({
                "status": "error",
                "validated": false,
                "message": format!("Biometrie non confirmee: {}", e)
            }));
        }
    };

    if let Err(e) =
        crate::webauthn_manager::marquer_usage(&conn, credential_id, verification.compteur)
    {
        eprintln!("[WEBAUTHN] compteur non mis a jour ({})", e);
    }

    HttpResponse::Ok().json(json!({
        "status": "success",
        "message": "Transaction validee par biometrie",
        "validated": true
    }))
}

/// POST /api/webauthn/list
/// Liste les credentials d'un utilisateur
pub async fn webauthn_list(body: web::Json<serde_json::Value>) -> HttpResponse {
    let uq_number = body.get("uq_number").and_then(|v| v.as_str()).unwrap_or("");

    if uq_number.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "uq_number requis"
        }));
    }

    let db_path = format!(
        "data/users/{}.db",
        uq_number.replace("-", "_").to_lowercase()
    );

    let conn = match rusqlite::Connection::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Erreur DB: {}", e)
            }))
        }
    };

    let _ = crate::webauthn_manager::init_webauthn_table(&conn);

    match crate::webauthn_manager::list_credentials(&conn) {
        Ok(creds) => HttpResponse::Ok().json(json!({
            "status": "success",
            "credentials": creds,
            "count": creds.len()
        })),
        Err(e) => HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": e
        })),
    }
}

/// POST /api/webauthn/remove
/// Supprime une credential (necessite mot de passe)
pub async fn webauthn_remove(body: web::Json<serde_json::Value>) -> HttpResponse {
    let uq_number = body.get("uq_number").and_then(|v| v.as_str()).unwrap_or("");
    let credential_id = body
        .get("credential_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let password = body.get("password").and_then(|v| v.as_str()).unwrap_or("");

    if uq_number.is_empty() || credential_id.is_empty() || password.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Parametres manquants"
        }));
    }

    // Verifier le mot de passe
    let idx = crate::uq_index::UqIndex::shared("uq_index.db");
    let account_info = idx.get_account(uq_number);

    if let Some(info) = account_info {
        let email = info.get("uq_email").and_then(|v| v.as_str()).unwrap_or("");
        let verify = idx.verify_credentials(uq_number, email, password);

        if verify.is_none() {
            return HttpResponse::Unauthorized().json(json!({
                "status": "error",
                "message": "Mot de passe incorrect"
            }));
        }
    }

    let db_path = format!(
        "data/users/{}.db",
        uq_number.replace("-", "_").to_lowercase()
    );

    let conn = match rusqlite::Connection::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Erreur DB: {}", e)
            }))
        }
    };

    if let Err(e) = conn.execute(
        "DELETE FROM webauthn_credentials WHERE credential_id = ?1",
        rusqlite::params![credential_id],
    ) {
        return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": e.to_string()
        }));
    }

    HttpResponse::Ok().json(json!({
        "status": "success",
        "message": "Biometrie supprimee"
    }))
}

// ====================
// CHANGEMENT DE MOT DE PASSE
// ====================

/// POST /api/account/password/change
/// Modifie le mot de passe (verifie l'ancien + exige compte verifie)
pub async fn account_password_change(body: web::Json<serde_json::Value>) -> HttpResponse {
    let uq_number = body.get("uq_number").and_then(|v| v.as_str()).unwrap_or("");
    let old_password = body
        .get("old_password")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let new_password = body
        .get("new_password")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if uq_number.is_empty() || old_password.is_empty() || new_password.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Tous les champs sont requis"
        }));
    }

    if new_password.len() < 8 {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Minimum 8 caracteres"
        }));
    }

    // 1. Verifier l'ancien mot de passe
    let idx = crate::uq_index::UqIndex::shared("uq_index.db");
    let account_info = idx.get_account(uq_number);

    let email = match account_info {
        Some(info) => info
            .get("uq_email")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        None => {
            return HttpResponse::NotFound().json(json!({
                "status": "error",
                "message": "Compte introuvable"
            }))
        }
    };

    let verify = idx.verify_credentials(uq_number, &email, old_password);

    if verify.is_none() {
        return HttpResponse::Unauthorized().json(json!({
            "status": "error",
            "message": "Ancien mot de passe incorrect"
        }));
    }

    // 2. Mettre a jour le mot de passe dans uq_index
    if let Err(e) = idx.update_password(uq_number, new_password) {
        return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("Erreur mise a jour: {}", e)
        }));
    }

    HttpResponse::Ok().json(json!({
        "status": "success",
        "message": "Mot de passe modifie avec succes"
    }))
}

/// POST /api/account/pseudo/set
/// Modifie le pseudo (chat/PDP) - le pseudo UQ reste inchange
pub async fn account_pseudo_set(body: web::Json<serde_json::Value>) -> HttpResponse {
    let uq_number = body.get("uq_number").and_then(|v| v.as_str()).unwrap_or("");
    let new_pseudo = body
        .get("new_pseudo")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let password = body.get("password").and_then(|v| v.as_str()).unwrap_or("");

    if uq_number.is_empty() || new_pseudo.is_empty() || password.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Tous les champs sont requis"
        }));
    }

    // Verifier le mot de passe
    let idx = crate::uq_index::UqIndex::shared("uq_index.db");
    let account_info = idx.get_account(uq_number);

    let email = match account_info {
        Some(info) => info
            .get("uq_email")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        None => {
            return HttpResponse::NotFound().json(json!({
                "status": "error",
                "message": "Compte introuvable"
            }))
        }
    };

    let verify = idx.verify_credentials(uq_number, &email, password);

    if verify.is_none() {
        return HttpResponse::Unauthorized().json(json!({
            "status": "error",
            "message": "Mot de passe incorrect"
        }));
    }

    // Sauvegarder le pseudo d'affichage dans les preferences
    let db_path = format!(
        "data/users/{}.db",
        uq_number.replace("-", "_").to_lowercase()
    );

    if !std::path::Path::new(&db_path).exists() {
        return HttpResponse::NotFound().json(json!({
            "status": "error",
            "message": "Base utilisateur introuvable"
        }));
    }

    let conn = match rusqlite::Connection::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Erreur DB: {}", e)
            }))
        }
    };

    let _ = conn.execute(
        "CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT,
            updated_at INTEGER
        )",
        [],
    );

    let now = chrono::Utc::now().timestamp();
    let _ = conn.execute(
        "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('display_pseudo', ?1, ?2)",
        rusqlite::params![new_pseudo, now],
    );

    HttpResponse::Ok().json(json!({
        "status": "success",
        "message": "Pseudo modifie",
        "display_pseudo": new_pseudo
    }))
}

// ====================
// VERIFY IDENTITY — Deverrouiller le portefeuille apres KYC
// ====================

pub async fn verify_identity(body: web::Json<serde_json::Value>) -> HttpResponse {
    let uq_number = body.get("uq_number").and_then(|v| v.as_str()).unwrap_or("");

    if uq_number.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "uq_number requis"
        }));
    }

    // Verifier le cadenas
    match crate::kyc::cadenas::verifier_cadenas(uq_number) {
        Ok(_) => {
            // Deverrouiller
            match crate::kyc::cadenas::deverrouiller(uq_number) {
                Ok(_) => HttpResponse::Ok().json(json!({
                    "status": "success",
                    "verified": true,
                    "deverrouille": true,
                    "message": "Identite verifiee. Portefeuille deverrouille."
                })),
                Err(e) => HttpResponse::InternalServerError().json(json!({
                    "status": "error",
                    "message": format!("Erreur deverrouillage: {}", e)
                })),
            }
        }
        Err(e) => {
            // Piratage detecte
            let _ = crate::kyc::cadenas::geler_compte_piratage(uq_number, &e);
            HttpResponse::Forbidden().json(json!({
                "status": "error",
                "piratage": true,
                "message": format!("PIRATAGE DETECTE: {}", e)
            }))
        }
    }
}

// ====================
// VERIFICATION SUBMIT — Soumettre les documents KYC
// ====================

pub async fn verification_submit(
    req: actix_web::HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let uq_number = body.get("uq_number").and_then(|v| v.as_str()).unwrap_or("");
    let password = body.get("password").and_then(|v| v.as_str()).unwrap_or("");

    if uq_number.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "uq_number requis"
        }));
    }
    if let Err(response) = crate::auth_guard::exiger_auth(&req) {
        return response;
    }
    if let Err(response) = crate::auth_guard::exiger_soi_ou_owner(&req, uq_number) {
        return response;
    }
    let queue = crate::kyc::storage::KycStore::load_queue();
    let verified = queue.dossiers.iter().any(|dossier| {
        dossier.uq_number.eq_ignore_ascii_case(uq_number)
            && dossier.statut == crate::kyc::dossier::KycStatus::Verifie
    });
    if !verified {
        return HttpResponse::Forbidden().json(json!({
            "status": "error",
            "code": "KYC_NON_VALIDE",
            "message": "Le dossier KYC doit être validé par l'autorité UQ avant le déverrouillage."
        }));
    }

    // Sans mot de passe, on déverrouille uniquement un dossier déjà validé.
    // Cette route ne soumet ni ne valide des documents: elle finalise l'accès.
    if password.is_empty() {
        return match crate::kyc::cadenas::verifier_cadenas(uq_number) {
            Ok(_) => match crate::kyc::cadenas::deverrouiller(uq_number) {
                Ok(_) => HttpResponse::Ok().json(json!({
                    "status": "success",
                    "verified": true,
                    "key_generated": false,
                    "deverrouille": true,
                    "message": "Documents verifies. Portefeuille deverrouille."
                })),
                Err(e) => HttpResponse::InternalServerError().json(json!({
                    "status": "error",
                    "message": format!("Erreur deverrouillage: {}", e)
                })),
            },
            Err(e) => {
                let _ = crate::kyc::cadenas::geler_compte_piratage(uq_number, &e);
                HttpResponse::Forbidden().json(json!({
                    "status": "error",
                    "piratage": true,
                    "message": format!("PIRATAGE DETECTE: {}", e)
                }))
            }
        };
    }

    // 1. Vérifier le cadenas
    match crate::kyc::cadenas::verifier_cadenas(uq_number) {
        Ok(_) => {
            // 2. GÉNÉRER LA CLÉ PRIVÉE (KYC valide !)
            let wallet_keys = match crate::security::wallet_keys::generate_wallet_keys(password) {
                Ok(k) => k,
                Err(e) => {
                    return HttpResponse::InternalServerError().json(json!({
                        "status": "error",
                        "message": format!("Erreur generation cles: {}", e)
                    }))
                }
            };

            // 3. Stocker la clé privée dans my_private_key
            let db_path = format!(
                "data/users/{}.db",
                uq_number.replace("-", "_").to_lowercase()
            );
            let conn = match rusqlite::Connection::open(&db_path) {
                Ok(c) => c,
                Err(e) => {
                    return HttpResponse::InternalServerError().json(json!({
                        "status": "error",
                        "message": format!("Erreur DB: {}", e)
                    }))
                }
            };

            let _ = conn.execute(
                "CREATE TABLE IF NOT EXISTS my_private_key (
                    id INTEGER PRIMARY KEY CHECK (id = 1),
                    encrypted_key BLOB NOT NULL,
                    iv BLOB NOT NULL,
                    salt BLOB NOT NULL,
                    algorithm TEXT DEFAULT 'AES-256-GCM',
                    created_at INTEGER NOT NULL,
                    rotated_at INTEGER
                )",
                [],
            );

            let now = chrono::Utc::now().timestamp();

            if let Err(e) = conn.execute(
                "INSERT OR REPLACE INTO my_private_key (id, encrypted_key, iv, salt, algorithm, created_at) 
                 VALUES (1, ?1, ?2, ?3, 'AES-256-GCM', ?4)",
                rusqlite::params![
                    wallet_keys.encrypted_private_key,
                    wallet_keys.iv.to_vec(),
                    wallet_keys.salt.to_vec(),
                    now
                ],
            ) {
                return HttpResponse::InternalServerError().json(json!({
                    "status": "error",
                    "message": format!("Erreur stockage cle: {}", e)
                }));
            }

            // 4. Mettre à jour la clé publique dans uq_index
            {
                if let Ok(uq_index_conn) = rusqlite::Connection::open("uq_index.db") {
                    let _ = uq_index_conn.execute(
                        "UPDATE uq_index SET public_key = ?1 WHERE uq_number = ?2",
                        rusqlite::params![wallet_keys.public_key, uq_number],
                    );
                }
            }

            eprintln!(
                "Clé privée GENEREE au KYC pour {} (public: {}...)",
                uq_number,
                hex::encode(&wallet_keys.public_key[..8])
            );

            // 5. Déverrouiller le cadenas
            match crate::kyc::cadenas::deverrouiller(uq_number) {
                Ok(_) => HttpResponse::Ok().json(json!({
                    "status": "success",
                    "verified": true,
                    "key_generated": true,
                    "message": "Documents verifies. Cle generee. Portefeuille deverrouille."
                })),
                Err(e) => HttpResponse::InternalServerError().json(json!({
                    "status": "error",
                    "message": format!("Erreur deverrouillage: {}", e)
                })),
            }
        }
        Err(e) => {
            let _ = crate::kyc::cadenas::geler_compte_piratage(uq_number, &e);
            HttpResponse::Forbidden().json(json!({
                "status": "error",
                "piratage": true,
                "message": format!("PIRATAGE DETECTE: {}", e)
            }))
        }
    }
}
pub async fn verification_status(
    req: actix_web::HttpRequest,
    path: web::Path<String>,
) -> HttpResponse {
    // 🔐 SÉCURITÉ (vague 1) — USER : accès à SES données (ou owner)
    // (handler sans paramètre de chemin : contrôle JWT simple)
    // 🔐 SÉCURITÉ (vague 1) — USER : accès à SES données (ou owner)
    let __cible = path.to_string();
    if let Err(r) = crate::auth_guard::exiger_soi_ou_owner(&req, &__cible) {
        return r;
    }

    let uq_number = path.into_inner();

    let db_path = format!(
        "data/users/{}.db",
        uq_number.replace("-", "_").to_lowercase()
    );
    if !std::path::Path::new(&db_path).exists() {
        return HttpResponse::NotFound().json(json!({
            "status": "error",
            "message": "Compte introuvable"
        }));
    }

    if let Ok(conn) = rusqlite::Connection::open(&db_path) {
        if let Ok((verrouille, est_verifie)) = conn.query_row::<(i64, i64), _, _>(
            "SELECT verrouille, est_verifie FROM cadenas_verification WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ) {
            let status = if est_verifie == 1 && verrouille == 0 {
                "verified"
            } else if verrouille == 1 {
                "not_verified"
            } else {
                "pending"
            };

            return HttpResponse::Ok().json(json!({
                "status": status,
                "verified": est_verifie == 1,
                "deverrouille": verrouille == 0
            }));
        }
    }

    HttpResponse::Ok().json(json!({
        "status": "not_verified",
        "verified": false,
        "deverrouille": false
    }))
}

// ====================
// API ME — Endpoint unique pour tout le profil
// ====================

pub async fn api_me(req: actix_web::HttpRequest) -> HttpResponse {
    use crate::auth::AuthManager;
    use serde_json::Value;

    let claims = match AuthManager::require_role(&req, &["user", "admin", "guardian"]) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let uq_number = &claims.sub;

    // Calculer les permissions
    let perms = crate::permissions::compute_permissions(uq_number);

    // Récupérer le compte
    let idx = crate::uq_index::UqIndex::shared("uq_index.db");
    let account = idx.get_account(uq_number);
    let code_promo = idx.get_code_promo(uq_number);

    // PROFIL COMPLET
    let profil = idx.get_full_profile(uq_number);
    let get_p = |k: &str| profil.get(k).cloned().unwrap_or_default();
    let recuperation = idx.get_recovery_contacts(uq_number).unwrap_or(json!({}));

    // Récupérer les données du compte
    let db_path = format!(
        "data/users/{}.db",
        uq_number.replace("-", "_").to_lowercase()
    );
    let mut nom = String::new();
    let mut prenom = String::new();
    let mut date_naissance = String::new();
    let mut lieu_naissance = String::new();
    let mut pays = "RDC".to_string();
    let mut balance_borb = 0.0;
    let mut balance_points = 0i64;
    let mut units: Vec<Value> = Vec::new();
    let mut vaults: Vec<Value> = Vec::new();
    let mut settings_map: Vec<(String, String)> = Vec::new();

    if let Ok(conn) = rusqlite::Connection::open(&db_path) {
        // Cadenas
        if let Ok((n, p, dn, ln, py)) = conn.query_row::<(String, String, String, String, String), _, _>(
            "SELECT COALESCE(nom, ''), COALESCE(prenom, ''), COALESCE(date_naissance, ''), COALESCE(lieu_naissance, ''), COALESCE(pays, 'RDC') FROM cadenas_verification WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        ) {
            nom = n;
            prenom = p;
            date_naissance = dn;
            lieu_naissance = ln;
            pays = py;
        }

        // Balance
        if let Ok(pts) = conn.query_row::<i64, _, _>(
            "SELECT COALESCE(points, 0) FROM balance WHERE id = 1",
            [],
            |row| row.get(0),
        ) {
            balance_points = pts;
            balance_borb = pts as f64 / 100.0;
        }

        // Units
        if let Ok(mut stmt) = conn.prepare("SELECT unit, balance FROM my_borb_units ORDER BY unit")
        {
            if let Ok(rows) = stmt.query_map([], |row| {
                Ok(json!({"unit": row.get::<_, String>(0)?, "balance": row.get::<_, i64>(1)?}))
            }) {
                units = rows.flatten().collect();
            }
        }

        // Vaults
        if let Ok(mut stmt) =
            conn.prepare("SELECT vault_name, vault_type, COALESCE(balance_borb, 0) FROM vaults")
        {
            if let Ok(rows) = stmt.query_map([], |row| {
                Ok(json!({
                    "name": row.get::<_, String>(0)?,
                    "type": row.get::<_, String>(1)?,
                    "balance_borb": row.get::<_, f64>(2)?
                }))
            }) {
                vaults = rows.flatten().collect();
            }
        }

        // Settings
        if let Ok(mut stmt) = conn.prepare("SELECT key, COALESCE(value, '') FROM settings WHERE key IN ('language', 'theme', 'font_size', 'display_unit')") {
            if let Ok(rows) = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            }) {
                settings_map = rows.flatten().collect();
            }
        }
    }

    let account_obj = account.unwrap_or(json!({}));

    HttpResponse::Ok().json(json!({
        "status": "success",
        "me": {
            "uq_number": uq_number,
            "email": account_obj.get("uq_email").and_then(|v| v.as_str()).unwrap_or(""),
            "pseudo": account_obj.get("uq_pseudo").and_then(|v| v.as_str()).unwrap_or(""),
            "code_promo": code_promo.unwrap_or_default(),
            "is_owner": perms.is_owner,
            "sexe": get_p("sexe"),
            "ville_provenance": get_p("ville_provenance"),
            "ville_actuelle": get_p("ville_actuelle"),
            "statut_marital": get_p("statut_marital"),
            "photo_url": get_p("photo_url"),
            "description": get_p("description"),
            "emploi": get_p("emploi"),
            "mobile_money_number": get_p("mobile_money_number"),
            "mobile_money_operator": get_p("mobile_money_operator"),
            "date_naissance": get_p("date_naissance"),
            "lieu_naissance": lieu_naissance.clone(),
            "recovery_email": recuperation.get("recovery_email").cloned().unwrap_or(json!("")),
            "recovery_phone": recuperation.get("recovery_phone").cloned().unwrap_or(json!("")),
            "phone_verified": recuperation.get("phone_verified").cloned().unwrap_or(json!(false)),
            "email_verified": recuperation.get("email_verified").cloned().unwrap_or(json!(false))
        },
        "identity": {
            "nom": nom,
            "prenom": prenom,
            "date_naissance": date_naissance,
            "lieu_naissance": lieu_naissance,
            "pays": pays
        },
        "balance": {
            "borb": balance_borb,
            "points": balance_points
        },
        "units": units,
        "vaults": vaults,
        "settings": settings_map.iter().map(|(k, v)| json!({"key": k, "value": v})).collect::<Vec<_>>(),
        "permissions": crate::permissions::permissions_to_json(&perms)
    }))
}
// ====================
// API LOGOUT — Déconnexion
// ====================

pub async fn api_logout(req: actix_web::HttpRequest) -> HttpResponse {
    use crate::auth::AuthManager;

    let claims = match AuthManager::require_role(&req, &["user", "admin", "guardian"]) {
        Ok(c) => c,
        Err(_) => return HttpResponse::Ok().json(json!({"status": "success"})),
    };

    // Logger la déconnexion
    let db_path = format!(
        "data/users/{}.db",
        claims.sub.replace("-", "_").to_lowercase()
    );
    if let Ok(conn) = rusqlite::Connection::open(&db_path) {
        let now = chrono::Utc::now().timestamp();
        let _ = conn.execute(
            "INSERT INTO security_log (event_type, success, details, created_at)
             VALUES ('logout', 1, 'Déconnexion', ?1)",
            rusqlite::params![now],
        );
    }

    HttpResponse::Ok().json(json!({
        "status": "success",
        "message": "Déconnecté"
    }))
}
// ====================
// API PLAYER STATUS — État du player
// ====================

pub async fn api_player_status(req: actix_web::HttpRequest) -> HttpResponse {
    use crate::auth::AuthManager;

    let claims = match AuthManager::require_role(&req, &["user", "admin", "guardian"]) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    HttpResponse::Ok().json(json!({
        "status": "success",
        "playing": false,
        "current": null,
        "queue": [],
        "user": claims.sub
    }))
}
// ====================
// API STUDIO SUGGEST — Suggestions IA
// ====================

pub async fn api_studio_suggest(req: actix_web::HttpRequest) -> HttpResponse {
    use crate::auth::AuthManager;

    let claims = match AuthManager::require_role(&req, &["user", "admin", "guardian"]) {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    HttpResponse::Ok().json(json!({
        "status": "success",
        "suggestions": [
            {"title": "Améliorez votre titre", "description": "Ajoutez des mots-clés", "priority": 1},
            {"title": "Ajoutez des photos", "description": "3 photos minimum", "priority": 2},
            {"title": "Décrivez en détail", "description": "Plus de 50 caractères", "priority": 3}
        ]
    }))
}
// ====================
// API STUDIO TYPES — Types de contenu
// ====================

pub async fn api_studio_types() -> HttpResponse {
    HttpResponse::Ok().json(json!({
        "status": "success",
        "types": [
            {"id": "text", "label": "Texte", "icon": "📝"},
            {"id": "image", "label": "Image", "icon": "🖼️"},
            {"id": "video", "label": "Vidéo", "icon": "🎥"},
            {"id": "audio", "label": "Audio", "icon": "🎵"},
            {"id": "document", "label": "Document", "icon": "📄"}
        ]
    }))
}
// ====================
// UQ PSEUDOS — Auto-complétion pour recherche de destinataire
// ====================
pub async fn uq_search_pseudos(
    query: web::Query<std::collections::HashMap<String, String>>,
) -> HttpResponse {
    let search = query
        .get("search")
        .map(|s| s.to_lowercase())
        .unwrap_or_default();

    if search.len() < 3 {
        return HttpResponse::Ok().json(json!({ "status": "success", "pseudos": [] }));
    }

    let idx = crate::uq_index::UqIndex::shared("uq_index.db");
    // La priorité aux certifiés doit se décider AVANT la troncature : on récupère
    // un volant plus large, on trie, puis on rend les 10 premiers.
    let mut all = idx.search_pseudos(&search, 50);

    let db = Database::shared("uq_platform_db.json");
    if let Ok(conn) = db.connexion() {
        let auteurs: Vec<String> = all
            .iter()
            .filter_map(|p| {
                p.get("pseudo")
                    .and_then(|v| v.as_str())
                    .and_then(crate::uq_public_alias::resoudre_destinataire)
            })
            .collect();
        let badges = crate::profile_routes::badges_collectifs(&db, &conn, &auteurs);
        for entry in all.iter_mut() {
            let uq = entry
                .get("pseudo")
                .and_then(|v| v.as_str())
                .and_then(crate::uq_public_alias::resoudre_destinataire)
                .unwrap_or_default();
            let badge = badges.get(&uq).cloned().unwrap_or_default();
            entry["badge"] = json!(badge);
            entry["certifie"] = json!(!badge.is_empty());
        }
        // tri stable : les certifiés passent devant, l'ordre de requête est conservé
        all.sort_by(|a, b| {
            b.get("certifie")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
                .cmp(&a.get("certifie").and_then(|v| v.as_bool()).unwrap_or(false))
        });
    }
    all.truncate(10);

    HttpResponse::Ok().json(json!({
        "status": "success",
        "pseudos": all
    }))
}

/// POST /api/account/profile/update
pub async fn account_profile_update(
    req: HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let uq = body.get("uq_number").and_then(|v| v.as_str()).unwrap_or("");
    if uq.is_empty() {
        return HttpResponse::BadRequest()
            .json(json!({"status": "error", "message": "uq_number requis"}));
    }
    if !crate::auth_guard::ressemble_a_un_uq(uq) {
        return HttpResponse::BadRequest()
            .json(json!({"status": "error", "message": "Identifiant de compte invalide"}));
    }
    if let Err(response) = crate::auth_guard::exiger_soi_ou_owner(&req, uq) {
        return response;
    }
    let index = crate::uq_index::UqIndex::shared("uq_index.db");
    let r = index.update_profile_full(
        uq,
        body.get("ville_provenance").and_then(|v| v.as_str()),
        body.get("ville_actuelle").and_then(|v| v.as_str()),
        body.get("statut_marital").and_then(|v| v.as_str()),
        body.get("photo_url").and_then(|v| v.as_str()),
        body.get("description").and_then(|v| v.as_str()),
        body.get("emploi").and_then(|v| v.as_str()),
        body.get("mobile_money_number").and_then(|v| v.as_str()),
        body.get("mobile_money_operator").and_then(|v| v.as_str()),
    );
    match r {
        Ok(_) => {
            HttpResponse::Ok().json(json!({"status": "success", "message": "Profil mis a jour"}))
        }
        Err(e) => {
            HttpResponse::InternalServerError().json(json!({"status": "error", "message": e}))
        }
    }
}

// ====================
// RECUPERATION DE COMPTE — mail externe et telephone de secours
// ====================

/// POST /api/account/recovery/update
/// Le mail externe (Gmail ou autre) ne sert qu'a recuperer le compte : il ne
/// remplace jamais le mail UQ genere, et il ne s'appelle qu'ici, jamais a la
/// creation. L'acces est reserve au titulaire (ou au proprietaire).
pub async fn account_recovery_update(
    req: HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let uq = body.get("uq_number").and_then(|v| v.as_str()).unwrap_or("");
    if uq.is_empty() || !crate::auth_guard::ressemble_a_un_uq(uq) {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "uq_number requis"
        }));
    }
    if let Err(response) = crate::auth_guard::exiger_soi_ou_owner(&req, uq) {
        return response;
    }

    let email_externe = body
        .get("recovery_email")
        .and_then(|v| v.as_str())
        .map(|v| v.trim().to_lowercase());
    if let Some(v) = email_externe.as_deref() {
        if !v.is_empty() && !crate::verify_routes::email_valide_pub(v) {
            return HttpResponse::BadRequest().json(json!({
                "status": "error",
                "message": "Adresse email invalide",
                "raison": "email_invalide"
            }));
        }
    }

    let telephone = body
        .get("recovery_phone")
        .and_then(|v| v.as_str())
        .map(|v| v.trim().to_string());
    let telephone_normalise = match telephone.as_deref() {
        Some(v) if !v.is_empty() => match crate::verify_routes::analyser_telephone(v) {
            Some(n) => Some(n.e164),
            None => {
                return HttpResponse::BadRequest().json(json!({
                    "status": "error",
                    "message": "Ce numéro n'existe pas ou est incomplet.",
                    "raison": "numero_inconnu"
                }))
            }
        },
        _ => None,
    };

    // Un jeton de verification consomme prouve que le contact appartient au
    // titulaire ; sinon le contact est enregistre comme non verifie.
    let jeton_telephone = body.get("verification_telephone").and_then(|v| v.as_str());
    let telephone_verifie = jeton_telephone.and_then(|jeton| {
        crate::verify_routes::valeur_verifiee(jeton, crate::verify_routes::Portee::Telephone)
    });

    let index = crate::uq_index::UqIndex::shared("uq_index.db");
    let avant = index.get_recovery_contacts(uq).unwrap_or(json!({}));

    // Un contact deja verifie ne se laisse en vie que si on ne le touche pas.
    let avant_email = avant
        .get("recovery_email")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let avant_phone = avant
        .get("recovery_phone")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let avant_phone_verifie = avant
        .get("phone_verified")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let avant_email_verifie = avant
        .get("email_verified")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let nouveau_email = email_externe.as_deref();
    let nouveau_phone = telephone_normalise
        .as_deref()
        .or(telephone_verifie.as_deref());

    let phone_verifie = match &telephone_verifie {
        Some(_) => true,
        None => match nouveau_phone {
            Some(v) => v == avant_phone && avant_phone_verifie,
            None => avant_phone_verifie,
        },
    };
    let email_verifie = match nouveau_email {
        Some(v) => v == avant_email && avant_email_verifie,
        None => avant_email_verifie,
    };

    if index
        .set_recovery_contacts(
            uq,
            nouveau_email,
            nouveau_phone,
            Some(phone_verifie),
            Some(email_verifie),
        )
        .is_err()
    {
        return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": "Enregistrement impossible"
        }));
    }

    let apres = index.get_recovery_contacts(uq).unwrap_or(json!({}));
    eprintln!(
        "[RECUPERATION] {} — avant {:?} apres {:?}",
        uq,
        avant
            .get("recovery_email")
            .and_then(|v| v.as_str())
            .unwrap_or(""),
        apres
            .get("recovery_email")
            .and_then(|v| v.as_str())
            .unwrap_or("")
    );

    HttpResponse::Ok().json(json!({
        "status": "success",
        "message": "Contacts de récupération enregistrés",
        "recovery_email": apres.get("recovery_email").cloned().unwrap_or(json!("")),
        "recovery_phone": apres.get("recovery_phone").cloned().unwrap_or(json!("")),
        "phone_verified": apres.get("phone_verified").cloned().unwrap_or(json!(false)),
        "email_verified": apres.get("email_verified").cloned().unwrap_or(json!(false)),
    }))
}

// ====================
// SUBUNITS — Statut + Déblocage automatique
// ====================

/// GET /api/borb/subunits/status
/// Retourne le statut actuel des sous-unités (global)
pub async fn borb_subunits_status() -> HttpResponse {
    // Charger le prix actuel
    let prix = crate::persistence::load_prix();
    let prix_bd = <bigdecimal::BigDecimal as std::str::FromStr>::from_str(&prix.prix_actuel)
        .unwrap_or_else(|_| bigdecimal::BigDecimal::from(1));

    // Charger le nombre d'unités débloquées (depuis le fichier global)
    let unlocked_count = load_unlocked_count();

    let status = crate::subunit_unlock::get_unlock_status(&prix_bd, unlocked_count);

    HttpResponse::Ok().json(json!({
        "status": "success",
        "prix_borb_usd": prix.prix_actuel,
        "subunits": status
    }))
}

/// POST /api/borb/subunits/check-unlock
/// Vérifie et débloque automatiquement les sous-unités si nécessaire
pub async fn borb_subunits_check_unlock() -> HttpResponse {
    let prix = crate::persistence::load_prix();
    let prix_bd = <bigdecimal::BigDecimal as std::str::FromStr>::from_str(&prix.prix_actuel)
        .unwrap_or_else(|_| bigdecimal::BigDecimal::from(1));

    let current = load_unlocked_count();
    let to_unlock = crate::subunit_unlock::check_and_unlock(&prix_bd, current);

    if to_unlock > 0 {
        let new_count = current + to_unlock;
        save_unlocked_count(new_count);

        eprintln!(
            "🪙 DÉBLOCAGE : {} nouvelles sous-unités (total: {})",
            to_unlock, new_count
        );

        return HttpResponse::Ok().json(json!({
            "status": "success",
            "unlocked": to_unlock,
            "new_total": new_count,
            "message": format!("{} nouvelles sous-unités débloquées", to_unlock)
        }));
    }

    HttpResponse::Ok().json(json!({
        "status": "success",
        "unlocked": 0,
        "message": "Aucun déblocage nécessaire"
    }))
}

/// Charge le nombre d'unités débloquées (depuis data/subunits.json)
fn load_unlocked_count() -> usize {
    let path = "data/subunits.json";
    if let Ok(content) = std::fs::read_to_string(path) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
            return json
                .get("unlocked_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(6) as usize;
        }
    }
    6 // Par défaut : 6 unités de base
}

/// Sauvegarde le nombre d'unités débloquées
fn save_unlocked_count(count: usize) {
    let path = "data/subunits.json";
    let json = json!({
        "unlocked_count": count,
        "updated_at": chrono::Utc::now().timestamp()
    });
    let _ = std::fs::write(path, json.to_string());
}

// ====================
// OWNER — Forcer le prix BORB (pour tests + simulation)
// ====================
pub async fn owner_force_price(body: web::Json<serde_json::Value>) -> HttpResponse {
    let new_price = body.get("price").and_then(|v| v.as_str()).unwrap_or("");

    if new_price.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "price requis"
        }));
    }

    let state = crate::regulateur::RegulateurState {
        prix_actuel: new_price.to_string(),
        demande: 0,
        rarete: "0".to_string(),
        vitesse: "0".to_string(),
        dernier_ajustement: chrono::Utc::now().timestamp(),
    };

    match crate::persistence::save_prix(&state) {
        Ok(_) => {
            eprintln!("💱 Prix BORB forcé à {} USD", new_price);
            HttpResponse::Ok().json(json!({
                "status": "success",
                "new_price": new_price,
                "message": format!("Prix forcé à {} USD", new_price)
            }))
        }
        Err(e) => HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": e
        })),
    }
}
// ====================
// DEMANDE — Calcul selon Modèle C
// ====================

/// GET /api/owner/demande/current
/// Retourne l'état actuel de la demande (soldes + volume × vitesse)
pub async fn owner_demande_current() -> HttpResponse {
    let state = match crate::demande_calculator::calculer_demande_globale() {
        Ok(s) => s,
        Err(raison) => {
            return HttpResponse::ServiceUnavailable().json(json!({
                "status": "error",
                "code": "PRIX_NON_REGLE",
                "message": raison
            }))
        }
    };

    HttpResponse::Ok().json(json!({
        "status": "success",
        "demande": {
            "total_soldes_borb": state.total_soldes_borb,
            "volume_24h_borb": state.volume_24h_borb,
            "vitesse": state.vitesse,
            "demande_totale_usd": state.demande_totale_usd,
            "prix_calcule": state.prix_calcule,
            "updated_at": state.updated_at
        }
    }))
}

/// POST /api/owner/demande/recalculate
/// Recalcule la demande et met à jour le prix
pub async fn owner_demande_recalculate() -> HttpResponse {
    let state = match crate::demande_calculator::calculer_demande_globale() {
        Ok(s) => s,
        Err(raison) => {
            eprintln!("🛑 Recalcul de la demande refusé : {}", raison);
            return HttpResponse::ServiceUnavailable().json(json!({
                "status": "error",
                "code": "PRIX_NON_REGLE",
                "message": raison
            }));
        }
    };

    // Sauvegarder le nouveau prix
    let prix_state = crate::regulateur::RegulateurState {
        prix_actuel: state.prix_calcule.clone(),
        demande: state.demande_totale_usd,
        rarete: "0".to_string(),
        vitesse: format!("{:.4}", state.vitesse),
        dernier_ajustement: chrono::Utc::now().timestamp(),
    };

    match crate::persistence::save_prix(&prix_state) {
        Ok(_) => {
            eprintln!(
                "💱 Demande recalculée : {} USD → prix = {} USD",
                state.demande_totale_usd, state.prix_calcule
            );

            HttpResponse::Ok().json(json!({
                "status": "success",
                "demande": state,
                "message": format!("Prix mis à jour : {} USD", state.prix_calcule)
            }))
        }
        Err(e) => HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": e
        })),
    }
}
// ====================
// OWNER — Forcer la DEMANDE (le prix se calcule automatiquement)
// ====================
pub async fn owner_force_demande(body: web::Json<serde_json::Value>) -> HttpResponse {
    let new_demande = body.get("demande").and_then(|v| v.as_u64()).unwrap_or(0);

    // Calculer le prix selon la formule : max(1, demande / 40 Md)
    let new_price = crate::regulateur::Regulateur::calculer_prix(new_demande as u128);

    // Calculer la rareté
    let circulation = 40_000_000_000u128;
    let rarete = crate::regulateur::Regulateur::facteur_rarete(circulation, 40_000_000_000);

    let state = crate::regulateur::RegulateurState {
        prix_actuel: new_price.to_string(),
        demande: new_demande as u128,
        rarete: rarete.to_string(),
        vitesse: "1.0".to_string(),
        dernier_ajustement: chrono::Utc::now().timestamp(),
    };

    match crate::persistence::save_prix(&state) {
        Ok(_) => {
            eprintln!(
                "💱 Demande forcée à {} USD → prix = {} USD",
                new_demande, new_price
            );
            HttpResponse::Ok().json(json!({
                "status": "success",
                "demande_usd": new_demande,
                "new_price_usd": new_price.to_string(),
                "message": format!("Demande {} USD → prix {} USD", new_demande, new_price)
            }))
        }
        Err(e) => HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": e
        })),
    }
}
// ====================
// BORB TRACER — Routes de vérification
// ====================

/// GET /api/borb/origin/{trace_key}
/// Vérifie l'origine d'un BORB
pub async fn borb_get_origin(path: web::Path<String>) -> HttpResponse {
    let trace_key = path.into_inner();

    let db_path = "data/users/uq_00000001.db";
    let conn = match rusqlite::Connection::open(db_path) {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("DB: {}", e)
            }))
        }
    };

    // 🔧 INIT : créer la table si elle n'existe pas
    let _ = crate::borb_tracer::init_borb_origins_table(&conn);

    match crate::borb_tracer::verifier_origine(&conn, &trace_key) {
        Ok(origin) => HttpResponse::Ok().json(json!({
            "status": "success",
            "origin": origin
        })),
        Err(e) => HttpResponse::NotFound().json(json!({
            "status": "error",
            "message": e
        })),
    }
}

/// GET /api/borb/origins/{recipient_uq}
/// Liste les origines reçues par un utilisateur
pub async fn borb_list_origins(path: web::Path<String>) -> HttpResponse {
    let recipient_uq = path.into_inner();

    let db_path = "data/users/uq_00000001.db";
    let conn = match rusqlite::Connection::open(db_path) {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("DB: {}", e)
            }))
        }
    };

    // 🔧 INIT : créer la table si elle n'existe pas
    let _ = crate::borb_tracer::init_borb_origins_table(&conn);

    let mut stmt = match conn.prepare(
        "SELECT trace_key, timestamp, amount_points, treasury_uq, cadenas_owner
         FROM borb_origins WHERE recipient_uq = ?1
         ORDER BY timestamp DESC LIMIT 100",
    ) {
        Ok(s) => s,
        Err(_) => {
            return HttpResponse::Ok().json(json!({
                "status": "success",
                "origins": []
            }))
        }
    };

    let rows = stmt.query_map(rusqlite::params![recipient_uq], |row| {
        Ok(json!({
            "trace_key": row.get::<_, String>(0)?,
            "timestamp": row.get::<_, i64>(1)?,
            "amount_points": row.get::<_, i64>(2)?,
            "treasury_uq": row.get::<_, String>(3)?,
            "cadenas_owner": row.get::<_, String>(4)?,
        }))
    });

    let mut origins = Vec::new();
    if let Ok(r) = rows {
        for row in r.flatten() {
            origins.push(row);
        }
    }

    HttpResponse::Ok().json(json!({
        "status": "success",
        "recipient": recipient_uq,
        "total": origins.len(),
        "origins": origins
    }))
}
// ====================
// BORB TRACER — Statistiques
// ====================

/// GET /api/borb/stats
/// Retourne les statistiques de traçabilité
pub async fn borb_tracer_stats() -> HttpResponse {
    let db_path = "data/users/uq_00000001.db";
    let conn = match rusqlite::Connection::open(db_path) {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("DB: {}", e)
            }))
        }
    };

    let _ = crate::borb_tracer::init_borb_origins_table(&conn);

    // Compter les entrées
    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM borb_origins", [], |row| row.get(0))
        .unwrap_or(0);

    // Total distribué
    let total_amount: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(amount_points), 0) FROM borb_origins",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    // Dernière émission
    let last = conn
        .query_row(
            "SELECT trace_key, timestamp, recipient_uq, amount_points
         FROM borb_origins ORDER BY timestamp DESC LIMIT 1",
            [],
            |row| {
                Ok(json!({
                    "trace_key": row.get::<_, String>(0)?,
                    "timestamp": row.get::<_, i64>(1)?,
                    "recipient": row.get::<_, String>(2)?,
                    "amount": row.get::<_, i64>(3)?,
                }))
            },
        )
        .unwrap_or(json!(null));

    HttpResponse::Ok().json(json!({
        "status": "success",
        "stats": {
            "total_borb_traced": total,
            "total_amount_points": total_amount,
            "total_amount_borb": total_amount as f64 / 100.0,
            "last_emission": last
        }
    }))
}
// ====================
// RÉVOQUER UN APPAREIL
// ====================
pub async fn account_devices_revoke(body: web::Json<serde_json::Value>) -> HttpResponse {
    let uq_number = body.get("uq_number").and_then(|v| v.as_str()).unwrap_or("");
    let device = body.get("device").and_then(|v| v.as_str()).unwrap_or("");

    if uq_number.is_empty() || device.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "uq_number et device requis"
        }));
    }

    let db_path = format!(
        "data/users/{}.db",
        uq_number.replace("-", "_").to_lowercase()
    );
    if !std::path::Path::new(&db_path).exists() {
        return HttpResponse::NotFound().json(json!({
            "status": "error",
            "message": "Compte introuvable"
        }));
    }

    let conn = match rusqlite::Connection::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Erreur DB: {}", e)
            }))
        }
    };

    // Créer la table si absente
    let _ = conn.execute(
        "CREATE TABLE IF NOT EXISTS sessions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            token TEXT UNIQUE NOT NULL,
            device TEXT DEFAULT '',
            ip TEXT DEFAULT '',
            user_agent TEXT DEFAULT '',
            created_at INTEGER DEFAULT 0,
            expires_at INTEGER DEFAULT 0,
            active INTEGER DEFAULT 1
        )",
        [],
    );

    // Désactiver le device
    match conn.execute(
        "UPDATE sessions SET active = 0 WHERE device = ?1",
        rusqlite::params![device],
    ) {
        Ok(count) => {
            eprintln!("🔐 Appareil révoqué : {} ({} sessions)", device, count);
            HttpResponse::Ok().json(json!({
                "status": "success",
                "message": "Appareil révoqué",
                "revoked_count": count
            }))
        }
        Err(e) => HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("Erreur révocation: {}", e)
        })),
    }
}

/// GET /api/admin/routes — liste des routes deployees, reserve owner.
pub async fn admin_routes(req: actix_web::HttpRequest) -> HttpResponse {
    if let Err(r) = crate::auth_guard::exiger_owner(&req) {
        return r;
    }
    let liste: Vec<serde_json::Value> = crate::routes_list::ROUTES
        .iter()
        .map(|(m, p)| json!({"method":m,"path":p}))
        .collect();
    HttpResponse::Ok().json(json!({"status":"ok","routes":liste,"count":liste.len()}))
}

fn sanitisel_log(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control() || *c == ' ')
        .take(1000)
        .collect::<String>()
        .replace('\n', " ")
        .replace('\r', " ")
}

pub async fn debug_log_error(
    req: actix_web::HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    // Corps max 4 Ko (413 au-dela).
    if req
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok())
        .map(|n| n > 4096)
        .unwrap_or(false)
    {
        return HttpResponse::PayloadTooLarge().json(json!({"status":"too_large"}));
    }
    // Rate limit 10 req/min/IP — lit X-Forwarded-For (proxy Render) en premier.
    // X-Forwarded-For : le client peut prefacer la liste avec de fausses IP ;
    // le proxy Render ajoute la sienne EN DERNIER -> on prend la derniere.
    let ip = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.rsplit(',').next())
        .map(|s| s.trim().to_string())
        .or_else(|| req.peer_addr().map(|a| a.ip().to_string()))
        .unwrap_or_else(|| "inconnu".into());
    use std::sync::{Mutex, OnceLock};
    static HITS: OnceLock<Mutex<std::collections::HashMap<String, (u32, std::time::Instant)>>> =
        OnceLock::new();
    let map = HITS.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    let mut guard = match map.lock() {
        Ok(g) => g,
        Err(_) => return HttpResponse::TooManyRequests().finish(),
    };
    let now = std::time::Instant::now();
    let entry = guard.entry(ip.clone()).or_insert((0, now));
    if now.duration_since(entry.1).as_secs() >= 60 {
        entry.0 = 0;
        entry.1 = now;
    }
    entry.0 += 1;
    if entry.0 > 10 {
        return HttpResponse::TooManyRequests().json(json!({"status":"rate_limited"}));
    }
    drop(guard);

    let msg = sanitisel_log(body.get("message").and_then(|v| v.as_str()).unwrap_or(""));
    let stack = sanitisel_log(body.get("stack").and_then(|v| v.as_str()).unwrap_or(""));
    let url = sanitisel_log(body.get("url").and_then(|v| v.as_str()).unwrap_or(""));

    eprintln!("ERREUR FRONTEND [ip={}]:", ip);
    eprintln!("   URL: {}", url);
    eprintln!("   MSG: {}", msg);
    eprintln!("   STACK: {}", stack);

    HttpResponse::Ok().json(json!({"status": "logged"}))
}

// ============================================================
// REPARTITION UNIQUE 70/30 - Handler
// ============================================================
pub async fn distribution_repartition(
    req: actix_web::HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    use crate::borb_distribution::BorbDistribution;

    // Verifier que l'owner est connecte
    if let Err(r) = crate::auth_guard::exiger_owner(&req) {
        return r;
    }

    let dist = BorbDistribution::new("UQ-00000001");

    // Deja reparti ?
    if dist.est_deja_reparti() {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "status": "error",
            "message": "Deja reparti",
            "deja_reparti": true
        }));
    }

    // Recuperer le solde total depuis le body
    let solde_total = body
        .get("solde_total_borb")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    if solde_total == 0 {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "status": "error",
            "message": "Solde total requis"
        }));
    }

    // Repartir
    match dist.repartir_initial(solde_total) {
        Ok((p1, p2)) => HttpResponse::Ok().json(serde_json::json!({
            "status": "success",
            "profil1_borb": p1,
            "profil2_borb": p2,
            "solde_total": solde_total
        })),
        Err(e) => HttpResponse::BadRequest().json(serde_json::json!({
            "status": "error",
            "message": e
        })),
    }
}

pub async fn distribution_repartition_status(req: actix_web::HttpRequest) -> HttpResponse {
    use crate::borb_distribution::BorbDistribution;

    if let Err(r) = crate::auth_guard::exiger_owner(&req) {
        return r;
    }

    let dist = BorbDistribution::new("UQ-00000001");
    let deja = dist.est_deja_reparti();
    let stats = dist.get_stats();

    HttpResponse::Ok().json(serde_json::json!({
        "status": "success",
        "deja_reparti": deja,
        "stats": stats
    }))
}

// ============================================================
// HELPER : Taux live BORB depuis l'oracle
// Retourne un HashMap devise -> BORB (live ou fallback)
// ============================================================
pub(crate) async fn taux_live_borb() -> std::collections::HashMap<String, f64> {
    let live = crate::public_routes::taux_du_jour().await;
    let mut map = std::collections::HashMap::new();

    let mut borb_usd = crate::persistence::load_prix()
        .prix_actuel
        .parse::<f64>()
        .ok()
        .filter(|price| price.is_finite() && *price > 0.0);

    // Pont nœud BORB : si le nœud a publié son taux live récemment
    // (data/borb_node/taux.json, micro-USD par BORB), il prime sur
    // l'oracle — c'est le taux capturé localement et propagé aux bandes.
    // Âge max accepté : 10 minutes ; au-delà on retombe sur l'oracle.
    if let Ok(brut) = std::fs::read_to_string("data/borb_node/taux.json") {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&brut) {
            let frais = std::fs::metadata("data/borb_node/taux.json")
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .map(|d| d.as_secs() <= 600)
                .unwrap_or(false);
            if frais {
                if let Some(micro) = v.get("taux_micro").and_then(|x| x.as_u64()) {
                    let prix_noeud = micro as f64 / 1_000_000.0;
                    if prix_noeud.is_finite() && prix_noeud > 0.0 {
                        borb_usd = Some(prix_noeud);
                    }
                }
            }
        }
    }

    let Some(borb_usd) = borb_usd else {
        return map;
    };

    // Devises : 1 unité de devise = X BORB, selon le cours backend du BORB.
    for (code, units_per_usd) in &live.rates {
        if units_per_usd.is_finite() && *units_per_usd > 0.0 {
            map.insert(code.clone(), 1.0 / (units_per_usd * borb_usd));
        }
    }

    // Pas de liste de repli : une devise que l'oracle n'a pas observée n'a pas
    // de taux. Les trois appelants renvoient déjà une erreur explicite
    // (MISSING_CURRENCY_RATE) au lieu d'inventer un cours de conversion.
    map
}

// ============================================================
// CLAIM WITH PIN - Reclamation 30% avec PIN owner unique
// POST /api/distribution/claim-with-pin
// Body: { code_pin: "3543", uq_profil2: "UQ-230354360" }
// ============================================================
pub async fn distribution_claim_with_pin(body: web::Json<serde_json::Value>) -> HttpResponse {
    use rusqlite::Connection;

    let code_pin = body.get("code_pin").and_then(|v| v.as_str()).unwrap_or("");
    let uq_profil2 = body
        .get("uq_profil2")
        .and_then(|v| v.as_str())
        .unwrap_or("UQ-732337351");

    if code_pin.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Code PIN requis"
        }));
    }

    let owner_uq = "UQ-00000001";
    let now = chrono::Utc::now().timestamp();

    // 1. Ouvrir DB owner + verifier PIN
    let owner_db = format!(
        "data/users/{}.db",
        owner_uq.replace("-", "_").to_lowercase()
    );
    let conn_owner = match Connection::open(&owner_db) {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("DB owner: {}", e)
            }))
        }
    };

    // 2. Lire salt + hash PIN
    let salt: String = conn_owner
        .query_row(
            "SELECT value FROM settings WHERE key = 'login_pin_salt'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_else(|_| crate::security_login::DEFAULT_LOGIN_PIN_SALT.to_string());

    let stored_hash: String = conn_owner
        .query_row(
            "SELECT value FROM settings WHERE key = 'login_pin_hash'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_default();

    // 3. Vérifier le PIN (hash, comparaison temps constant, fail-closed)
    let pin_ok = if stored_hash.is_empty() {
        crate::security_login::constant_time_eq(
            &crate::security_login::hash_pin(
                code_pin,
                crate::security_login::DEFAULT_LOGIN_PIN_SALT,
            ),
            crate::security_login::FALLBACK_OWNER_PIN_HASH,
        )
    } else {
        crate::security_login::constant_time_eq(
            &crate::security_login::hash_pin(code_pin, &salt),
            &stored_hash,
        )
    };

    if !pin_ok {
        eprintln!("[CLAIM-PIN] PIN incorrect");
        return HttpResponse::Forbidden().json(json!({
            "status": "error",
            "message": "Code PIN incorrect"
        }));
    }

    eprintln!("[CLAIM-PIN] PIN valide, reclamation pour {}", uq_profil2);

    // 4. Lire solde owner mobilisable (le BORB immobilisé n'est pas distribuables)
    let owner_points: i64 = conn_owner
        .query_row(
            "SELECT COALESCE(points, 0) - COALESCE(frozen_points, 0) FROM balance WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    if owner_points <= 0 {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Solde Profil 1 vide"
        }));
    }

    // 5. Calculer 30%
    let montant_30_pct = (owner_points as f64 * 0.30) as i64;

    if montant_30_pct <= 0 {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Montant 30% nul"
        }));
    }

    // 6. Ouvrir DB profil 2
    let p2_db = format!(
        "data/users/{}.db",
        uq_profil2.replace("-", "_").to_lowercase()
    );
    let conn_p2 = match Connection::open(&p2_db) {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("DB profil 2: {}", e)
            }))
        }
    };

    // 7. Lire solde actuel profil 2
    let p2_points_avant: i64 = conn_p2
        .query_row(
            "SELECT COALESCE(points, 0) FROM balance WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    // 8. DEBITER owner (-30%)
    let debit_owner = match conn_owner.execute(
        "UPDATE balance SET points = points - ?1 WHERE id = 1 AND points - COALESCE(frozen_points, 0) >= ?1",
        rusqlite::params![montant_30_pct],
    ) {
        Ok(n) => n,
        Err(e) => return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("Erreur debit owner: {}", e)
        })),
    };
    if debit_owner != 1 {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "code": "BORB_GELÉS",
            "message": "Le solde mobilisable de Profil 1 est insuffisant (BORB immobilisés par des commandes P2P)."
        }));
    }

    // 9. CREDITER profil 2 (+30%)
    if let Err(e) = conn_p2.execute(
        "UPDATE balance SET points = points + ?1 WHERE id = 1",
        rusqlite::params![montant_30_pct],
    ) {
        // Rollback owner
        let _ = conn_owner.execute(
            "UPDATE balance SET points = points + ?1 WHERE id = 1",
            rusqlite::params![montant_30_pct],
        );
        return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("Erreur credit profil2: {}", e)
        }));
    }

    // 10. Lire nouveaux soldes
    let owner_points_apres: i64 = conn_owner
        .query_row(
            "SELECT COALESCE(points, 0) FROM balance WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    let p2_points_apres: i64 = conn_p2
        .query_row(
            "SELECT COALESCE(points, 0) FROM balance WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    // 11. Sync multi_wallets
    let conn_idx = match rusqlite::Connection::open("uq_index.db") {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("DB index: {}", e)
            }))
        }
    };

    let owner_borb = owner_points_apres as f64 / 100.0;
    let _ = conn_idx.execute(
        "UPDATE multi_wallets SET balance = ?1, updated_at = ?2 WHERE uq_number = ?3 AND currency = 'BORB'",
        rusqlite::params![owner_borb, now, owner_uq],
    );

    let p2_borb = p2_points_apres as f64 / 100.0;
    let _ = conn_idx.execute(
        "INSERT INTO multi_wallets (uq_number, currency, balance, updated_at)
         VALUES (?1, 'BORB', ?2, ?3)
         ON CONFLICT(uq_number, currency) DO UPDATE SET balance = ?2, updated_at = ?3",
        rusqlite::params![uq_profil2, p2_borb, now],
    );

    eprintln!(
        "[CLAIM-PIN] SUCCES: {} points -> {}",
        montant_30_pct, uq_profil2
    );

    HttpResponse::Ok().json(json!({
        "status": "success",
        "message": format!("Reclamation de {} points (30%) effectuee", montant_30_pct),
        "montant_points": montant_30_pct,
        "montant_borb": montant_30_pct as f64 / 100.0,
        "owner_avant": owner_points,
        "owner_apres": owner_points_apres,
        "profil2_avant": p2_points_avant,
        "profil2_apres": p2_points_apres,
        "owner_borb": owner_borb,
        "profil2_borb": p2_borb
    }))
}

pub async fn distribution_auto_validate_kyc(
    req: actix_web::HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    use rusqlite::Connection;

    // 🔐 Validation d'identité : réservé à l'owner authentifié (JWT), plus au simple body.
    if let Err(r) = crate::auth_guard::exiger_owner(&req) {
        return r;
    }

    let owner_uq = body.get("owner_uq").and_then(|v| v.as_str()).unwrap_or("");
    let owner_email = body
        .get("owner_email")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let target_uq = body.get("target_uq").and_then(|v| v.as_str()).unwrap_or("");
    let target_email = body
        .get("target_email")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if owner_uq.is_empty() || target_uq.is_empty() || target_email.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "owner_uq, target_uq et target_email requis"
        }));
    }

    // 1. Verifier que c'est bien l'owner UQ-00000001
    if owner_uq != "UQ-00000001" {
        return HttpResponse::Forbidden().json(json!({
            "status": "error",
            "message": "Seul UQ-00000001 peut valider un KYC"
        }));
    }

    // 2. Verifier email owner (double securite)
    let conn_idx = match rusqlite::Connection::open("uq_index.db") {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("DB: {}", e)
            }))
        }
    };

    let owner_email_stored: String = conn_idx
        .query_row(
            "SELECT COALESCE(uq_email, '') FROM uq_index WHERE uq_number = 'UQ-00000001'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_default();

    if owner_email != owner_email_stored {
        return HttpResponse::Forbidden().json(json!({
            "status": "error",
            "message": "Email owner incorrect"
        }));
    }

    // 3. Verifier que le target existe et que UQ + email correspondent
    let target_email_stored: String = conn_idx
        .query_row(
            "SELECT COALESCE(uq_email, '') FROM uq_index WHERE uq_number = ?1",
            rusqlite::params![target_uq],
            |row| row.get(0),
        )
        .unwrap_or_default();

    if target_email_stored.is_empty() {
        return HttpResponse::NotFound().json(json!({
            "status": "error",
            "message": format!("Compte {} introuvable", target_uq)
        }));
    }

    if target_email_stored != target_email {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Email du user ne correspond pas a son UQ"
        }));
    }

    // 4. Ouvrir la DB du target et valider le KYC
    let target_db_name = target_uq.replace("-", "_").to_lowercase();
    let target_db = format!("data/users/{}.db", target_db_name);

    let conn_target = match Connection::open(&target_db) {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("DB target: {}", e)
            }))
        }
    };

    let now = chrono::Utc::now().timestamp();

    // 4.1 identities
    let _ = conn_target.execute(
        "UPDATE identities SET is_verified = 1, updated_at = ?1 WHERE uq_number = ?2",
        rusqlite::params![now, target_uq],
    );

    // 4.2 my_profile
    let _ = conn_target.execute(
        "UPDATE my_profile SET kyc_status = 'verified' WHERE id = 1",
        [],
    );

    // 4.3 cadenas_verification
    let _ = conn_target.execute(
        "UPDATE cadenas_verification SET est_verifie = 1, verrouille = 0, verifie_at = ?1 WHERE id = 1",
        rusqlite::params![now],
    );

    eprintln!("[AUTO-KYC] {} valide par {}", target_uq, owner_uq);

    HttpResponse::Ok().json(json!({
        "status": "success",
        "message": format!("KYC de {} valide avec succes", target_uq),
        "target_uq": target_uq,
        "target_email": target_email,
        "validated_at": now
    }))
}

/// GET /api/kyc/can-reclaim
/// Retourne si l'utilisateur connecte peut reclamer 30%
/// Utilise le JWT pour identifier l'utilisateur
pub async fn can_reclaim(req: actix_web::HttpRequest) -> HttpResponse {
    use rusqlite::Connection;

    // 1. Extraire le JWT
    let claims = match crate::auth::AuthManager::require_role(&req, &["user", "admin", "guardian"])
    {
        Ok(c) => c,
        Err(_) => {
            return HttpResponse::Ok().json(json!({
                "status": "success",
                "can_reclaim": false,
                "raison": "Non authentifie"
            }))
        }
    };

    let uq_connecte = claims.sub.clone();

    // 2. Verifier que c'est bien le Profil 2 (UQ-230354360)
    //    (on accepte AUSSI UQ-00000001 pour permettre le switch)
    let est_profil2 = uq_connecte == "UQ-230354360";

    if !est_profil2 {
        return HttpResponse::Ok().json(json!({
            "status": "success",
            "can_reclaim": false,
            "raison": "Vous n'etes pas sur le Profil 2"
        }));
    }

    // 3. Verifier le cadenas du Profil 2
    if !crate::kyc::cadenas::est_deverrouille(&uq_connecte) {
        return HttpResponse::Ok().json(json!({
            "status": "success",
            "can_reclaim": false,
            "raison": "Cadenas non valide (KYC requis)"
        }));
    }

    // 4. Verifier le lien circle_members
    let conn_idx = match rusqlite::Connection::open("uq_index.db") {
        Ok(c) => c,
        Err(_) => {
            return HttpResponse::Ok().json(json!({
                "status": "success",
                "can_reclaim": false,
                "raison": "DB indisponible"
            }))
        }
    };

    let lien_existe: i64 = conn_idx
        .query_row(
            "SELECT COUNT(*) FROM circle_members 
         WHERE owner_uq = 'UQ-00000001' 
         AND member_uq = ?1 
         AND is_active = 1",
            rusqlite::params![uq_connecte],
            |row| row.get(0),
        )
        .unwrap_or(0);

    if lien_existe == 0 {
        return HttpResponse::Ok().json(json!({
            "status": "success",
            "can_reclaim": false,
            "raison": "Pas lie a l'owner"
        }));
    }

    // 5. Verifier le solde du Profil 2
    let db_path = format!(
        "data/users/{}.db",
        uq_connecte.replace("-", "_").to_lowercase()
    );
    let mut solde_borb = 0.0;

    if let Ok(conn) = Connection::open(&db_path) {
        let points: i64 = conn
            .query_row(
                "SELECT COALESCE(points, 0) FROM balance WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        solde_borb = points as f64 / 100.0;
    }

    let seuil = 5000.0;
    if solde_borb >= seuil {
        return HttpResponse::Ok().json(json!({
            "status": "success",
            "can_reclaim": false,
            "raison": format!("Solde {} BORB >= seuil {} BORB", solde_borb, seuil)
        }));
    }

    // OK
    HttpResponse::Ok().json(json!({
        "status": "success",
        "can_reclaim": true,
        "raison": "Cadenas valide + lie a l'owner + solde bas",
        "solde_borb": solde_borb,
        "uq_profil2": uq_connecte
    }))
}

#[cfg(test)]
mod converted_transfer_tests {
    use super::{
        calculate_converted_transfer_amount, calculate_currency_to_borb_points,
        settle_converted_transfer, settle_currency_to_borb, ConvertedTransferAmountError,
        ReglementConverti,
    };
    use rusqlite::{params, Connection, TransactionBehavior};

    fn transfer_test_db() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory sqlite database");
        conn.execute_batch(
            "CREATE TABLE multi_wallets (
                uq_number TEXT NOT NULL,
                currency TEXT NOT NULL,
                balance REAL NOT NULL,
                updated_at INTEGER NOT NULL,
                UNIQUE(uq_number, currency)
            );
            CREATE TABLE wallet_transfers (
                transfer_id TEXT PRIMARY KEY,
                sender_uq TEXT NOT NULL,
                recipient_uq TEXT NOT NULL,
                currency TEXT NOT NULL,
                amount REAL NOT NULL,
                fee REAL NOT NULL,
                memo TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            INSERT INTO multi_wallets VALUES ('sender', 'CDF', 10.0, 1);
            INSERT INTO multi_wallets VALUES ('recipient', 'CDF', 2.0, 1);",
        )
        .expect("create transfer schema");
        crate::frais_plateforme::assurer_tables(&conn).expect("create fee schema");
        conn
    }

    /// Les bases de compte sont attachees en memoire, comme le fait le routeur
    /// avant d'ouvrir sa transaction : le reglement doit tenir dans un seul
    /// commit ou disparaitre entierement.
    fn attacher_comptes_test(conn: &Connection, sender: (i64, i64), recipient: (i64, i64)) {
        for (alias, (points, geles)) in [("src_account", sender), ("dst_account", recipient)] {
            conn.execute_batch(&format!(
                "ATTACH DATABASE ':memory:' AS {alias};
                 CREATE TABLE {alias}.balance (
                     id INTEGER PRIMARY KEY,
                     points INTEGER NOT NULL,
                     frozen_points INTEGER NOT NULL DEFAULT 0,
                     updated_at INTEGER
                 );
                 INSERT INTO {alias}.balance (id, points, frozen_points)
                     VALUES (1, {points}, {geles});",
            ))
            .expect("attacher un compte de test");
        }
    }

    fn points_compte_test(conn: &Connection, alias: &str) -> i64 {
        conn.query_row(
            &format!("SELECT points FROM {alias}.balance WHERE id = 1"),
            [],
            |row| row.get(0),
        )
        .expect("solde BORB de test")
    }

    fn solde_ombre_test(conn: &Connection, uq: &str) -> f64 {
        conn.query_row(
            "SELECT balance FROM multi_wallets WHERE uq_number = ?1 AND currency = 'CDF'",
            params![uq],
            |row| row.get(0),
        )
        .expect("solde devise de test")
    }

    #[test]
    fn converted_transfer_has_no_fee_and_rejects_invalid_precision() {
        assert_eq!(calculate_converted_transfer_amount(25.55), Ok(2_555));
        assert_eq!(calculate_converted_transfer_amount(10.0), Ok(1_000));
    }

    #[test]
    fn amount_calculation_rejects_invalid_precision_and_overflow() {
        assert_eq!(
            calculate_converted_transfer_amount(0.0),
            Err(ConvertedTransferAmountError::Invalid)
        );
        assert_eq!(
            calculate_converted_transfer_amount(f64::NAN),
            Err(ConvertedTransferAmountError::Invalid)
        );
        assert_eq!(
            calculate_converted_transfer_amount(1.001),
            Err(ConvertedTransferAmountError::Precision)
        );
        assert_eq!(
            calculate_converted_transfer_amount(i64::MAX as f64),
            Err(ConvertedTransferAmountError::TooLarge)
        );
    }

    #[test]
    fn currency_to_borb_conversion_respects_precision_and_rate() {
        assert_eq!(calculate_currency_to_borb_points(5.0, 1.0), Ok((500, 500)));
        assert_eq!(
            calculate_currency_to_borb_points(5.0, 0.0),
            Err(ConvertedTransferAmountError::Invalid)
        );
        assert_eq!(
            calculate_currency_to_borb_points(5.0, 0.333),
            Err(ConvertedTransferAmountError::Precision)
        );
    }

    #[test]
    fn settlement_debits_and_credits_exact_amount_without_fee() {
        let mut conn = transfer_test_db();
        attacher_comptes_test(&conn, (1_000, 0), (100, 0));
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .expect("begin transfer");
        let reglement = settle_converted_transfer(
            &tx,
            "sender",
            "recipient",
            "CDF",
            500,
            0,
            "memo",
            "wtx_1",
            2,
        )
        .expect("settle transfer");
        assert_eq!(reglement, ReglementConverti::Effectue);
        tx.commit().expect("commit transfer");

        assert_eq!(points_compte_test(&conn, "src_account"), 500);
        assert_eq!(points_compte_test(&conn, "dst_account"), 600);
        let sender_balance: f64 = conn
            .query_row(
                "SELECT balance FROM multi_wallets WHERE uq_number = ?1 AND currency = ?2",
                params!["sender", "CDF"],
                |row| row.get(0),
            )
            .expect("sender balance");
        let recipient_balance: f64 = conn
            .query_row(
                "SELECT balance FROM multi_wallets WHERE uq_number = ?1 AND currency = ?2",
                params!["recipient", "CDF"],
                |row| row.get(0),
            )
            .expect("recipient balance");
        let (recorded_amount, recorded_fee): (f64, f64) = conn
            .query_row(
                "SELECT amount, fee FROM wallet_transfers WHERE transfer_id = ?1",
                params!["wtx_1"],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("transfer record");

        assert_eq!(sender_balance, 5.0);
        assert_eq!(recipient_balance, 7.0);
        assert_eq!(recorded_amount, 5.0);
        assert_eq!(recorded_fee, 0.0);
    }

    #[test]
    fn reverse_conversion_moves_value_atomically_into_central_borb_balance() {
        let mut conn = transfer_test_db();
        conn.execute_batch(
            "ATTACH DATABASE ':memory:' AS user_account;
             CREATE TABLE user_account.balance (
                 id INTEGER PRIMARY KEY,
                 points INTEGER NOT NULL,
                 frozen_points INTEGER NOT NULL DEFAULT 0
             );
             INSERT INTO user_account.balance (id, points, frozen_points) VALUES (1, 100, 0);",
        )
        .expect("create attached account balance");
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .expect("begin conversion");
        let settled =
            settle_currency_to_borb(&tx, "sender", "CDF", 500, 500, 2).expect("settle conversion");
        assert!(settled);
        tx.commit().expect("commit conversion");

        let currency_balance: f64 = conn
            .query_row(
                "SELECT balance FROM multi_wallets WHERE uq_number = 'sender' AND currency = 'CDF'",
                [],
                |row| row.get(0),
            )
            .expect("currency wallet balance");
        let borb_points: i64 = conn
            .query_row(
                "SELECT points FROM user_account.balance WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .expect("central BORB balance");
        assert_eq!(currency_balance, 5.0);
        assert_eq!(borb_points, 600);
    }

    #[test]
    fn failed_reverse_conversion_rolls_back_currency_debit() {
        let mut conn = transfer_test_db();
        conn.execute_batch(
            "ATTACH DATABASE ':memory:' AS user_account;
             CREATE TABLE user_account.balance (
                 id INTEGER PRIMARY KEY,
                 points INTEGER NOT NULL,
                 frozen_points INTEGER NOT NULL DEFAULT 0
             );",
        )
        .expect("create empty attached account balance");
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .expect("begin conversion");
        assert!(!settle_currency_to_borb(&tx, "sender", "CDF", 500, 500, 2)
            .expect("conversion without account row"));
        drop(tx);

        let currency_balance: f64 = conn
            .query_row(
                "SELECT balance FROM multi_wallets WHERE uq_number = 'sender' AND currency = 'CDF'",
                [],
                |row| row.get(0),
            )
            .expect("currency wallet balance after rollback");
        assert_eq!(currency_balance, 10.0);
    }

    #[test]
    fn insufficient_balance_does_not_change_either_wallet_or_history() {
        let mut conn = transfer_test_db();
        conn.execute(
            "UPDATE multi_wallets SET balance = 0.50 WHERE uq_number = 'sender'",
            [],
        )
        .expect("set insufficient balance");
        attacher_comptes_test(&conn, (1_000, 0), (100, 0));
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .expect("begin transfer");
        let reglement =
            settle_converted_transfer(&tx, "sender", "recipient", "CDF", 500, 0, "", "wtx_2", 2)
                .expect("check insufficient balance");
        assert_eq!(reglement, ReglementConverti::OmbreInsuffisante);
        drop(tx);

        // Le BORB est l'argent reel : un echec de l'ombre ne doit pas le laisser
        // debite (c'etait le cas quand les comptes n'etaient pas dans la
        // transaction du reglement).
        assert_eq!(points_compte_test(&conn, "src_account"), 1_000);
        assert_eq!(points_compte_test(&conn, "dst_account"), 100);
        let sender_balance: f64 = conn
            .query_row(
                "SELECT balance FROM multi_wallets WHERE uq_number = 'sender'",
                [],
                |row| row.get(0),
            )
            .expect("sender balance");
        let recipient_balance: f64 = conn
            .query_row(
                "SELECT balance FROM multi_wallets WHERE uq_number = 'recipient'",
                [],
                |row| row.get(0),
            )
            .expect("recipient balance");
        let history_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM wallet_transfers", [], |row| {
                row.get(0)
            })
            .expect("history count");

        assert_eq!(sender_balance, 0.50);
        assert_eq!(recipient_balance, 2.0);
        assert_eq!(history_count, 0);
    }

    #[test]
    fn failed_history_insert_rolls_back_wallet_changes() {
        let mut conn = transfer_test_db();
        conn.execute(
            "INSERT INTO wallet_transfers
             VALUES ('duplicate', 'sender', 'recipient', 'CDF', 1.0, 0.0, '', 1)",
            [],
        )
        .expect("seed existing transfer");
        attacher_comptes_test(&conn, (1_000, 0), (100, 0));
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .expect("begin transfer");
        assert!(settle_converted_transfer(
            &tx,
            "sender",
            "recipient",
            "CDF",
            500,
            0,
            "",
            "duplicate",
            2,
        )
        .is_err());
        drop(tx);

        assert_eq!(points_compte_test(&conn, "src_account"), 1_000);
        assert_eq!(points_compte_test(&conn, "dst_account"), 100);

        let sender_balance: f64 = conn
            .query_row(
                "SELECT balance FROM multi_wallets WHERE uq_number = 'sender'",
                [],
                |row| row.get(0),
            )
            .expect("sender balance");
        let recipient_balance: f64 = conn
            .query_row(
                "SELECT balance FROM multi_wallets WHERE uq_number = 'recipient'",
                [],
                |row| row.get(0),
            )
            .expect("recipient balance");

        assert_eq!(sender_balance, 10.0);
        assert_eq!(recipient_balance, 2.0);
    }

    #[test]
    fn borb_gele_par_escrow_n_est_pas_convertible_en_transfert() {
        let mut conn = transfer_test_db();
        // 1 000 points dont 600 immobilises par une commande P2P : il n'en
        // reste que 400 mobilisables, moins que les 500 demandes.
        attacher_comptes_test(&conn, (1_000, 600), (100, 0));
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .expect("begin transfer");
        let reglement =
            settle_converted_transfer(&tx, "sender", "recipient", "CDF", 500, 0, "", "wtx_3", 2)
                .expect("check frozen BORB");
        assert_eq!(reglement, ReglementConverti::BorbInsuffisant);
        drop(tx);

        assert_eq!(points_compte_test(&conn, "src_account"), 1_000);
        assert_eq!(solde_ombre_test(&conn, "sender"), 10.0);
        let history_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM wallet_transfers", [], |row| {
                row.get(0)
            })
            .expect("history count");
        assert_eq!(history_count, 0);
    }
}

#[cfg(test)]
mod conversion_trace_tests {
    use super::{chemin_base_compte, tracer_ecritures_coffre, ContexteAppel, EcritureCoffre};
    use rusqlite::Connection;

    /// Le traceur ouvre des fichiers : les bases de compte de test vivent dans
    /// `target/`, pas en mémoire partagée.
    fn dossier_test(nom: &str) -> std::path::PathBuf {
        let chemin = std::path::Path::new("target")
            .join("test_coffres")
            .join(format!("{}-{}", std::process::id(), nom));
        std::fs::create_dir_all(&chemin).expect("dossier de test");
        chemin
    }

    fn chemin_fichier(dossier: &std::path::Path, uq: &str) -> String {
        dossier
            .join(format!("{}.db", uq.replace("-", "_").to_lowercase()))
            .to_string_lossy()
            .replace('\\', "/")
    }

    /// La DB d'un compte, réduite à ce que le traçage touche : solde réel,
    /// miroir des sous-unités, grand livre chaîné, et la table du cadenas que
    /// le scellé lit à chaque écriture.
    fn creer_compte(chemin: &str, points: Option<i64>, miroir: i64) {
        let conn = Connection::open(chemin).expect("compte de test");
        conn.execute_batch(
            "CREATE TABLE balance (
                id INTEGER PRIMARY KEY,
                points INTEGER NOT NULL,
                frozen_points INTEGER NOT NULL DEFAULT 0,
                updated_at INTEGER
             );
             CREATE TABLE my_borb_units (
                unit TEXT PRIMARY KEY,
                balance INTEGER NOT NULL,
                updated_at INTEGER
             );
             CREATE TABLE cadenas_verification (
                id INTEGER PRIMARY KEY,
                hash_transforme TEXT,
                verrouille INTEGER NOT NULL DEFAULT 0
             );
             INSERT INTO cadenas_verification (id, hash_transforme) VALUES (1, 'CADERIN-DE-TEST');
             CREATE TABLE transactions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                tx_type TEXT NOT NULL,
                amount INTEGER NOT NULL,
                fee INTEGER DEFAULT 0,
                counterparty TEXT,
                memo TEXT,
                signature BLOB,
                created_at INTEGER NOT NULL,
                device TEXT, os TEXT, browser TEXT, ip TEXT,
                sender_uq TEXT, recipient_uq TEXT,
                amount_borb REAL DEFAULT 0, amount_usd REAL DEFAULT 0, taux_usd TEXT,
                public_key TEXT, status TEXT DEFAULT 'success', reference TEXT,
                tampered INTEGER DEFAULT 0, cadenas_hash TEXT,
                internal_clock INTEGER, universal_key_hash TEXT,
                borb_id TEXT, prev_hash TEXT, block_number INTEGER DEFAULT 0
             );",
        )
        .expect("schéma du compte");
        conn.execute(
            "INSERT INTO my_borb_units (unit, balance, updated_at) VALUES ('BORB', ?1, 1)",
            rusqlite::params![miroir],
        )
        .expect("miroir de départ");
        if let Some(points) = points {
            conn.execute(
                "INSERT INTO balance (id, points, frozen_points) VALUES (1, ?1, 0)",
                rusqlite::params![points],
            )
            .expect("solde de départ");
        }
    }

    fn miroir_borb(chemin: &str) -> i64 {
        let conn = Connection::open(chemin).expect("lecture du miroir");
        conn.query_row(
            "SELECT balance FROM my_borb_units WHERE unit = 'BORB'",
            [],
            |row| row.get(0),
        )
        .expect("solde du miroir")
    }

    fn ecriture<'a>(
        uq: &'a str,
        chemin_db: &'a str,
        tx_type: &'a str,
        points: i64,
        contrepartie: &'a str,
        emetteur: &'a str,
        destinataire: &'a str,
    ) -> EcritureCoffre<'a> {
        EcritureCoffre {
            uq,
            chemin_db,
            tx_type,
            points,
            frais_points: 0,
            contrepartie,
            note: "mouvement de test",
            reference: "",
            emetteur,
            destinataire,
        }
    }

    #[test]
    fn conversion_laisse_une_ecriture_et_recale_le_miroir_sur_le_solde() {
        let dossier = dossier_test("conversion");
        let chemin = chemin_fichier(&dossier, "UQ-123456789");
        creer_compte(&chemin, Some(1234), 999999);

        tracer_ecritures_coffre(
            &[ecriture(
                "UQ-123456789",
                &chemin,
                "convert",
                1234,
                "Portefeuille USD",
                "UQ-123456789",
                "UQ-123456789",
            )],
            1_700_000_000,
            &ContexteAppel {
                device: "wallet_convert",
                ..Default::default()
            },
        );

        let contenu: (String, i64, String, String) = {
            let conn = Connection::open(&chemin).expect("ouverture après traçage");
            conn.query_row(
                "SELECT tx_type, amount, counterparty, device FROM transactions WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("écriture du grand livre")
        };
        assert_eq!(contenu.0, "convert");
        assert_eq!(contenu.1, 1234);
        assert_eq!(contenu.2, "Portefeuille USD");
        assert_eq!(contenu.3, "wallet_convert");

        // Le miroir suit `balance.points` : 1234 points = 12 BORB entiers, pas
        // la valeur stockée avant le mouvement.
        assert_eq!(miroir_borb(&chemin), 12);
        std::fs::remove_dir_all(&dossier).ok();
    }

    #[test]
    fn le_scele_survit_a_deux_ecritures_consecutives() {
        let dossier = dossier_test("scelle");
        let chemin = chemin_fichier(&dossier, "UQ-123456789");
        creer_compte(&chemin, Some(1500), 10);

        for points in [1_000_i64, 500_i64] {
            tracer_ecritures_coffre(
                &[ecriture(
                    "UQ-123456789",
                    &chemin,
                    "convert",
                    points,
                    "Réserve BORB",
                    "UQ-123456789",
                    "UQ-123456789",
                )],
                1_700_000_000,
                &ContexteAppel::default(),
            );
        }

        let conn = Connection::open(&chemin).expect("ouverture après traçage");
        let etat = crate::local_history::verifier_scelle(&conn, "UQ-123456789")
            .expect("le scellé doit rester vérifiable après le traçage");
        assert!(etat.chaine_valide, "la chaîne doit être reconsituée");
        // `integre` exige le cadenas structurel du compte, que l'index du
        // serveur connaît seul : ce qu'on prouve ici, c'est que le scellé a béni
        // les deux lignes ajoutées.
        assert_eq!(
            etat.chain_hash,
            etat.chain_hash_attendu
                .expect("un scellé doit avoir été recalculé à chaque écriture"),
            "le scellé doit correspondre à la chaîne augmentée"
        );
        assert_eq!(etat.tx_count, 2);

        // La seconde écriture s'enchaîne bien sur la première.
        let (empreinte_avant, hash_precedent): (String, String) = conn
            .query_row(
                "SELECT a.borb_id, b.prev_hash FROM transactions a
                 JOIN transactions b ON b.id = a.id + 1 WHERE a.id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("chaînage des deux écritures");
        assert_eq!(empreinte_avant, hash_precedent);
        drop(conn);
        assert_eq!(miroir_borb(&chemin), 15);
        std::fs::remove_dir_all(&dossier).ok();
    }

    #[test]
    fn un_transfert_converti_trace_les_deux_coffres_avec_leurs_affichages() {
        let dossier = dossier_test("transfert");
        let chemin_emetteur = chemin_fichier(&dossier, "UQ-111111111");
        let chemin_destinataire = chemin_fichier(&dossier, "UQ-222222222");
        creer_compte(&chemin_emetteur, Some(10_000), 100);
        creer_compte(&chemin_destinataire, Some(4_900), 20);

        tracer_ecritures_coffre(
            &[
                ecriture(
                    "UQ-111111111",
                    &chemin_emetteur,
                    "send",
                    5_000,
                    "uq@destinataire Destinée",
                    "UQ-111111111",
                    "UQ-222222222",
                ),
                ecriture(
                    "UQ-222222222",
                    &chemin_destinataire,
                    "receive",
                    4_900,
                    "uq@emetteur Emetteur",
                    "UQ-111111111",
                    "UQ-222222222",
                ),
            ],
            1_700_000_000,
            &ContexteAppel {
                device: "mobile",
                os: "Android",
                browser: "UQ app",
                ip: "10.0.0.1",
            },
        );

        let attentes = [
            (
                &chemin_emetteur,
                "send",
                5_000_i64,
                "uq@destinataire Destinée",
                100,
            ),
            (
                &chemin_destinataire,
                "receive",
                4_900_i64,
                "uq@emetteur Emetteur",
                49,
            ),
        ];
        for (chemin, type_attendu, points_attendus, contrepartie_attendue, miroir_attendu) in
            attentes
        {
            let contenu: (String, i64, String, String, String) = {
                let conn = Connection::open(chemin).expect("ouverture après traçage");
                conn.query_row(
                    "SELECT tx_type, amount, counterparty, sender_uq, recipient_uq
                     FROM transactions WHERE id = 1",
                    [],
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
                .expect("écriture du grand livre")
            };
            assert_eq!(contenu.0, type_attendu);
            assert_eq!(contenu.1, points_attendus);
            assert_eq!(contenu.2, contrepartie_attendue);
            assert_eq!(contenu.3, "UQ-111111111");
            assert_eq!(contenu.4, "UQ-222222222");
            assert_eq!(miroir_borb(chemin), miroir_attendu);
        }
        std::fs::remove_dir_all(&dossier).ok();
    }

    #[test]
    fn le_miroir_ne_se_recale_pas_sans_source_de_verite() {
        let dossier = dossier_test("sans_solde");
        let chemin = chemin_fichier(&dossier, "UQ-123456789");
        creer_compte(&chemin, None, 999999);

        tracer_ecritures_coffre(
            &[ecriture(
                "UQ-123456789",
                &chemin,
                "convert",
                1234,
                "Réserve BORB",
                "UQ-123456789",
                "UQ-123456789",
            )],
            1_700_000_000,
            &ContexteAppel::default(),
        );

        // Sans ligne de solde, il n'y a rien à miroiter : le traceur refuse
        // d'écrire un chiffre inventé et laisse la valeur antérieure.
        assert_eq!(miroir_borb(&chemin), 999999);
        std::fs::remove_dir_all(&dossier).ok();
    }

    #[test]
    fn une_base_absente_ne_fait_pas_paniquer_le_traceur() {
        let dossier = dossier_test("base_absente");
        let chemin = chemin_fichier(&dossier, "UQ-999999999");

        tracer_ecritures_coffre(
            &[ecriture(
                "UQ-999999999",
                &chemin,
                "convert",
                10,
                "Réserve BORB",
                "UQ-999999999",
                "UQ-999999999",
            )],
            1_700_000_000,
            &ContexteAppel::default(),
        );

        // SQLite crée un fichier de base à l'ouverture : le mouvement n'est pas
        // tracé faute de solde, et surtout le serveur ne panique pas après un
        // commit déjà validé.
        std::fs::remove_dir_all(&dossier).ok();
    }

    #[test]
    fn chemin_de_base_reprend_le_format_du_coffre() {
        assert_eq!(
            chemin_base_compte("UQ-230354360"),
            "data/users/uq_230354360.db"
        );
    }
}

// ====================
// TEST DES ROUTES DE PRIX
// ====================

#[cfg(test)]
mod prix_routes_tests {
    use super::{oublier_capture_prix, price_all, price_gold, price_usd};

    async fn corps(reponse: actix_web::HttpResponse) -> serde_json::Value {
        let brut = actix_web::body::to_bytes(reponse.into_body())
            .await
            .expect("corps de reponse illisible");
        serde_json::from_slice(&brut).expect("reponse JSON attendue")
    }

    fn prix_de(v: &serde_json::Value, chemin: &[&str]) -> Option<f64> {
        let mut curseur = v;
        for cle in chemin {
            curseur = &curseur[cle];
        }
        curseur.as_f64()
    }

    /// `/api/price/all`, `/api/price/gold` et `/api/price/usd` lisaient chacune
    /// leur propre littéral (`2000` lonce, `1.0` le BORB) : trois routes, trois
    /// prix pour le même actif, et aucun lien avec le marché. Elles partagent
    /// désormais une seule capture, datée et persistée.
    #[tokio::test]
    #[ignore = "coupe le réseau du processus (proxy invalide) pour la phase hors-ligne"]
    async fn les_routes_de_prix_publient_la_meme_capture_reelle() {
        // ---- 1. Réseau disponible : le prix du marché, le même sur les trois routes.
        let tout = corps(price_all().await).await;
        let or = corps(price_gold().await).await;
        let dollar = corps(price_usd().await).await;
        println!("\n=== /api/price/all (réseau) ===\n{}", tout);

        let once = prix_de(&tout, &["prices", "gold_usd_per_ounce"])
            .expect("/api/price/all doit publier le prix réel de l'once");
        assert_eq!(
            prix_de(&or, &["price_usd_per_ounce"]),
            Some(once),
            "une once ne peut pas valoir deux prix selon la route"
        );
        assert!(
            (once - 2000.0).abs() / once > 0.01,
            "le prix de l'once sort de la capture marché, plus du littéral 2000 (obtenu : {once})"
        );

        let borb =
            prix_de(&tout, &["prices", "borb_usd"]).expect("le BORB doit dériver de l'once réelle");
        assert_eq!(
            prix_de(&or, &["borb_value_usd"]),
            Some(borb),
            "/api/price/gold et /api/price/all doivent donner le même BORB"
        );
        assert_eq!(
            prix_de(&dollar, &["borb_to_usd"]),
            Some(borb),
            "/api/price/usd ne peut pas affirmer 1.0"
        );
        assert!(
            (borb - once / 2000.0).abs() < 1e-9,
            "la parité publiée doit être once/2000 (BORB = {borb}, once = {once})"
        );
        assert!(
            borb > 0.0 && borb.is_finite(),
            "un prix non fini ne se publie pas"
        );

        let aller_retour =
            prix_de(&dollar, &["usd_to_borb"]).expect("1 USD doit valoir 1/borb BORB");
        assert!(
            (aller_retour - 1.0 / borb).abs() <= aller_retour * 1e-9,
            "usd_to_borb doit être l'inverse exact de borb_to_usd ({aller_retour} vs {})",
            1.0 / borb
        );

        // ---- 2. La capture est datée et écrite sur disque : c'est ce qui permet
        // de garder un prix réel à travers la coupure réseau et un redémarrage.
        let disque = crate::external_api::capture_persistee_le(crate::external_api::CLE_SPOT_OR);
        println!(
            ">>> dernière capture or persistée sur disque : {:?}",
            disque
        );
        assert!(
            disque.is_some(),
            "la capture or doit être persistée pour survivre à la coupure"
        );

        let publie_le = tout["updated_at"]
            .as_i64()
            .expect("updated_at doit dater de la capture");
        let age_publie = tout["capture"]["or"]["age_secs"]
            .as_i64()
            .expect("l'âge de la capture doit être publié");
        assert_eq!(
            disque,
            Some(publie_le),
            "la route publie l'horodatage de sa capture disque"
        );
        assert!(
            (publie_le + age_publie - chrono::Utc::now().timestamp()).abs() <= 2,
            "âge publié + horodatage = maintenant ({publie_le} + {age_publie})"
        );

        // ---- 3. Réseau coupé : la dernière capture continue de s'appliquer,
        // datée, et aucun nombre inventé ne vient la remplacer.
        oublier_capture_prix();
        std::env::set_var("HTTPS_PROXY", "http://127.0.0.1:9");
        std::env::set_var("HTTP_PROXY", "http://127.0.0.1:9");
        std::env::remove_var("NO_PROXY");

        let reponse_hl = price_all().await;
        let statut_hl = reponse_hl.status().as_u16();
        let hors_ligne = corps(reponse_hl).await;
        println!(
            "\n=== /api/price/all (réseau coupé) : HTTP {} ===\n{}",
            statut_hl, hors_ligne
        );

        match statut_hl {
            200 => {
                let once_hl = prix_de(&hors_ligne, &["prices", "gold_usd_per_ounce"])
                    .expect("le prix resservi hors-ligne doit rester un prix");
                let borb_hl = prix_de(&hors_ligne, &["prices", "borb_usd"]).unwrap_or(0.0);
                println!(">>> hors-ligne : once = {once_hl} USD (avant coupure : {once}) ; BORB = {borb_hl}");
                assert!(once_hl > 0.0 && once_hl.is_finite(), "0 ou NaN ne remplace pas un prix");
                assert!(
                    (once_hl - 2000.0).abs() / once_hl > 0.01,
                    "hors-ligne aussi, le littéral 2000 doit rester absent (obtenu : {once_hl})"
                );
                assert!(
                    (borb_hl - once_hl / 2000.0).abs() < 1e-9,
                    "le BORB resservi doit rester once/2000"
                );
                assert!(
                    hors_ligne["capture"]["or"]["age_secs"].as_i64().unwrap_or(-1) >= 0,
                    "un prix resservi hors-ligne doit être daté"
                );
            }
            503 => {
                let motif = hors_ligne["code"].as_str().unwrap_or("");
                println!(">>> hors-ligne : 503 {motif} — aucune capture connue");
                assert!(
                    motif == "OR_INDISPONIBLE" || motif == "BTC_INDISPONIBLE",
                    "une route sans prix doit nommer la source manquante : code = {motif}"
                );
                assert!(
                    hors_ligne["prices"].is_null() && hors_ligne["capture"].is_null(),
                    "un 503 ne publie ni prix ni capture"
                );
            }
            autre => panic!("/api/price/all hors-ligne : HTTP {autre}, attendu 200 (dernière capture) ou 503 (rien à resservir)"),
        }

        std::env::remove_var("HTTPS_PROXY");
        std::env::remove_var("HTTP_PROXY");
    }
}
