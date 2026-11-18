// ====================
// 💱 TRADING — Marché de devises, carnet d'ordres, P2P mobile, notifications
// ====================
//
// Règles :
//  * Aucun prix n'est inventé : tout vient de l'oracle (`taux_du_jour`).
//  * La réserve de sécurité (BORB / BORB) n'est jamais une devise tradée.
//  * Un ordre « market » s'exécute au prix de l'oracle ; un ordre « limit »
//    immobilise les fonds (débit immédiat, remboursement à l'annulation).
//  * Le P2P mobile utilise le numéro du profil (`mobile_money_number`) et
//    ouvre un canal privé dans le chat UQ pour chaque commande.
//  * Les notifications sont stockées par écran (`scope`) : kyc, coffre,
//    zando, trading, chat.
// ====================

use actix_web::{web, HttpRequest, HttpResponse};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

/// Devises exclues : la réserve n'est pas une monnaie.
fn devise_exclue(code: &str) -> bool {
    matches!(code, "BORB" | "BORB")
}

fn code_devise(valeur: &str) -> String {
    valeur.trim().to_uppercase()
}

fn maintenant() -> i64 {
    chrono::Utc::now().timestamp()
}

fn texte(body: &Value, cle: &str) -> String {
    body.get(cle)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string()
}

fn nombre(body: &Value, cle: &str) -> f64 {
    match body.get(cle) {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0),
        Some(Value::String(s)) => s.trim().replace(',', ".").parse().unwrap_or(0.0),
        _ => 0.0,
    }
}

// ====================
// TABLES
// ====================

pub fn ensure_tables(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS trading_orders (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            uq TEXT NOT NULL,
            pair TEXT NOT NULL,
            base TEXT NOT NULL,
            quote TEXT NOT NULL,
            side TEXT NOT NULL,
            order_type TEXT NOT NULL,
            price REAL NOT NULL,
            amount REAL NOT NULL,
            filled REAL DEFAULT 0,
            total REAL DEFAULT 0,
            status TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_trading_orders_uq ON trading_orders(uq, status);
        CREATE INDEX IF NOT EXISTS idx_trading_orders_pair ON trading_orders(pair, status);

        CREATE TABLE IF NOT EXISTS trading_trades (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            order_id INTEGER NOT NULL,
            pair TEXT NOT NULL,
            uq TEXT NOT NULL,
            side TEXT NOT NULL,
            price REAL NOT NULL,
            amount REAL NOT NULL,
            total REAL NOT NULL,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_trading_trades_pair ON trading_trades(pair, id);

        CREATE TABLE IF NOT EXISTS p2p_offers (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            uq TEXT NOT NULL,
            side TEXT NOT NULL,
            currency TEXT NOT NULL,
            rate REAL NOT NULL,
            min_amount REAL NOT NULL,
            max_amount REAL NOT NULL,
            operator TEXT DEFAULT '',
            city TEXT DEFAULT '',
            phone TEXT DEFAULT '',
            status TEXT NOT NULL DEFAULT 'open',
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_p2p_offers_currency ON p2p_offers(currency, status);

        CREATE TABLE IF NOT EXISTS p2p_orders (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            offer_id INTEGER NOT NULL,
            buyer TEXT NOT NULL,
            seller TEXT NOT NULL,
            currency TEXT NOT NULL,
            amount REAL NOT NULL,
            usd_amount REAL NOT NULL,
            rate REAL NOT NULL,
            phone TEXT DEFAULT '',
            operator TEXT DEFAULT '',
            status TEXT NOT NULL,
            chat_group_id TEXT DEFAULT '',
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_p2p_orders_buyer ON p2p_orders(buyer, id);
        CREATE INDEX IF NOT EXISTS idx_p2p_orders_seller ON p2p_orders(seller, id);

        CREATE TABLE IF NOT EXISTS screen_notifications (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            uq TEXT NOT NULL,
            scope TEXT NOT NULL,
            title TEXT NOT NULL,
            body TEXT DEFAULT '',
            reference TEXT DEFAULT '',
            read INTEGER DEFAULT 0,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_screen_notifications_uq
            ON screen_notifications(uq, read);

        CREATE TABLE IF NOT EXISTS screen_ack (
            uq TEXT NOT NULL,
            reference TEXT NOT NULL,
            read_at INTEGER NOT NULL,
            PRIMARY KEY (uq, reference)
        );

        /* Historique des transferts partagé avec /api/account/wallets/transfers. */
        CREATE TABLE IF NOT EXISTS wallet_transfers (
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
            ON wallet_transfers(recipient_uq, created_at);

        /* Frais effectivement prélevés sur les ordres exécutés (Case 2). */

        /* Épargne Earn : verrou 30 jours, rendement versé depuis Case 2. */
        CREATE TABLE IF NOT EXISTS earn_positions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            uq TEXT NOT NULL,
            currency TEXT NOT NULL,
            principal REAL NOT NULL,
            rewards REAL NOT NULL DEFAULT 0,
            status TEXT NOT NULL DEFAULT 'active',
            locked_at INTEGER NOT NULL,
            unlock_at INTEGER NOT NULL,
            withdrawn_at INTEGER,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_earn_positions_uq
            ON earn_positions(uq, status);
        CREATE INDEX IF NOT EXISTS idx_earn_positions_currency
            ON earn_positions(currency, status);

        CREATE TABLE IF NOT EXISTS earn_rewards (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            position_id INTEGER NOT NULL,
            uq TEXT NOT NULL,
            currency TEXT NOT NULL,
            amount REAL NOT NULL,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_earn_rewards_uq
            ON earn_rewards(uq, created_at);
        CREATE INDEX IF NOT EXISTS idx_earn_rewards_currency
            ON earn_rewards(currency, created_at);

        /* Part des frais (Case 2) par devise : disponible = collected - distributed. */
        CREATE TABLE IF NOT EXISTS earn_pool (
            currency TEXT PRIMARY KEY,
            collected REAL NOT NULL DEFAULT 0,
            distributed REAL NOT NULL DEFAULT 0,
            updated_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS earn_state (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            last_distribution_at INTEGER NOT NULL DEFAULT 0
        );
        INSERT OR IGNORE INTO earn_state (id, last_distribution_at) VALUES (1, 0);
        "#,
    )
    .map_err(|e| format!("Trading : migration impossible ({})", e))?;

    // Suivi d'escrow P2P : ces colonnes ne peuvent pas passer par un
    // `CREATE TABLE IF NOT EXISTS` sur une base déjà créée.
    for (table, colonne, definition) in [
        ("p2p_offers", "taken_amount", "REAL DEFAULT 0"),
        ("p2p_offers", "rate_base", "TEXT DEFAULT ''"),
        ("p2p_offers", "amount_unit", "TEXT DEFAULT 'DEVISE'"),
        ("p2p_orders", "escrow_points", "INTEGER DEFAULT 0"),
        ("p2p_orders", "proof_code", "TEXT"),
        // Une commande met deux lignes face a face : celle que l'acheteur doit
        // payer, et la sienne d'ou part le paiement. Les deux preuves se
        // controlent chacune contre la bonne.
        ("p2p_orders", "buyer_phone", "TEXT NOT NULL DEFAULT ''"),
        ("p2p_orders", "seller_phone", "TEXT NOT NULL DEFAULT ''"),
    ] {
        ajouter_colonne_manquante(conn, table, colonne, definition)?;
    }
    // Les commandes heritees ne portaient que le numero declare par l'acheteur,
    // loge dans `phone` : le backfill le range du cote ou il a toujours ete.
    let _ = conn.execute(
        "UPDATE p2p_orders SET buyer_phone = COALESCE(phone, '') WHERE COALESCE(buyer_phone, '') = ''",
        [],
    );

    // Les preuves de paiement (images + analyses + issues apprises) partagent
    // la base d'index, là où vivent déjà les commandes.
    crate::p2p_preuve::assurer_tables(conn)?;

    crate::frais_plateforme::assurer_tables(conn)
        .map_err(|e| format!("Trading : tables de frais impossibles ({})", e))
}

/// Ajoute une colonne seulement si `PRAGMA table_info` ne la trouve pas déjà,
/// au lieu d'avaler l'erreur d'un `ALTER TABLE` systématique.
fn ajouter_colonne_manquante(
    conn: &Connection,
    table: &str,
    colonne: &str,
    definition: &str,
) -> Result<(), String> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({})", table))
        .map_err(|e| format!("Lecture du schéma {} ({})", table, e))?;
    let deja_la = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| e.to_string())?
        .flatten()
        .any(|nom| nom == colonne);
    if deja_la {
        return Ok(());
    }
    conn.execute(
        &format!("ALTER TABLE {} ADD COLUMN {} {}", table, colonne, definition),
        [],
    )
    .map_err(|e| format!("Colonne {}.{} ajoutée impossible ({})", table, colonne, e))?;
    Ok(())
}

fn ouvrir_index() -> Result<Connection, String> {
    let conn = Connection::open("uq_index.db")
        .map_err(|e| format!("Index indisponible ({})", e))?;
    let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
    ensure_tables(&conn)?;
    Ok(conn)
}

// ====================
// PROFIL / PIN
// ====================

struct Profil {
    pseudo: String,
    display_currency: String,
    phone: String,
    operator: String,
    kyc: bool,
}

fn lire_profil(conn: &Connection, uq: &str) -> Option<Profil> {
    conn.query_row(
        "SELECT COALESCE(uq_pseudo, ''), COALESCE(display_currency, ''),
                COALESCE(mobile_money_number, ''), COALESCE(mobile_money_operator, ''),
                COALESCE(kyc_verified, 0)
         FROM uq_index WHERE UPPER(uq_number) = UPPER(?1)",
        params![uq],
        |row| {
            Ok(Profil {
                pseudo: row.get(0)?,
                display_currency: row.get(1)?,
                phone: row.get(2)?,
                operator: row.get(3)?,
                kyc: row.get::<_, i64>(4)? == 1,
            })
        },
    )
    .ok()
}

fn chemin_compte(uq: &str) -> String {
    format!("data/users/{}.db", uq.replace('-', "_").to_lowercase())
}

/// PIN configuré sur la base du compte ?
fn pin_configure(uq: &str) -> bool {
    let chemin = chemin_compte(uq);
    if !std::path::Path::new(&chemin).exists() {
        return false;
    }
    let Ok(conn) = Connection::open(&chemin) else {
        return false;
    };
    let _ = crate::pin_manager::init_pin_table(&conn);
    conn.query_row(
        "SELECT CASE WHEN COALESCE(pin_hash, '') <> '' AND pin_enabled = 1 THEN 1 ELSE 0 END
         FROM pin_status WHERE id = 1",
        [],
        |row| row.get::<_, i64>(0),
    )
    .map(|v| v == 1)
    .unwrap_or(false)
}

/// Vérifie le PIN du compte. Un compte sans PIN configuré laisse passer,
/// comme le fait déjà `/api/account/pin/verify`.
fn verifier_pin(uq: &str, pin: &str) -> Result<bool, String> {
    let chemin = chemin_compte(uq);
    if !std::path::Path::new(&chemin).exists() {
        return Err("Compte introuvable".to_string());
    }
    let conn =
        Connection::open(&chemin).map_err(|e| format!("Base du compte illisible ({})", e))?;
    let _ = crate::pin_manager::init_pin_table(&conn);
    if !pin_configure(uq) {
        return Ok(true);
    }
    crate::pin_manager::verify_pin(&conn, pin).map_err(|e| e.to_string())
}

// ====================
// ORACLE / PAIRES
// ====================

fn lire_taux(rates: &HashMap<String, f64>, code: &str) -> Option<f64> {
    if code.eq_ignore_ascii_case("USD") {
        return Some(1.0);
    }
    rates
        .get(code)
        .copied()
        .filter(|valeur| *valeur > 0.0)
}

/// Contre-valeur USD d'un montant détenu ou échangé dans une devise.
/// `None` = l'oracle n'a jamais observé cette devise : le montant reste
/// non évalué. Le remplacer par 1 pour 1 gonflerait un total d'un air
/// comptable, et un volume de marché d'un air réel.
fn valeur_usd(rates: &HashMap<String, f64>, code: &str, montant: f64) -> Option<f64> {
    lire_taux(rates, code).map(|units_per_usd| montant / units_per_usd)
}

/// Prix d'une paire : unités de `quote` pour 1 `base`.
/// Les taux de l'oracle sont exprimés en unités de devise par USD, donc
/// 1 base = base_usd unités de `quote_usd`… par USD : le prix est
/// base_usd / quote_usd.
fn prix_paire(rates: &HashMap<String, f64>, base: &str, quote: &str) -> Option<f64> {
    let base_usd = lire_taux(rates, base)?;
    let quote_usd = lire_taux(rates, quote)?;
    let prix = base_usd / quote_usd;
    if prix.is_finite() && prix > 0.0 {
        Some(prix)
    } else {
        None
    }
}

fn parse_paire(paire: &str) -> Option<(String, String)> {
    let propre = paire.trim().to_uppercase().replace('/', "-");
    let mut morceaux = propre.split('-').filter(|m| !m.is_empty());
    let base = morceaux.next()?.to_string();
    let quote = morceaux.next()?.to_string();
    if morceaux.next().is_some() || base == quote {
        return None;
    }
    if devise_exclue(&base) || devise_exclue(&quote) {
        return None;
    }
    Some((base, quote))
}

fn nom_paire(base: &str, quote: &str) -> String {
    format!("{}-{}", base, quote)
}

// ====================
// SOLDES (multi_wallets)
// ====================

fn crediter(
    conn: &Connection,
    uq: &str,
    devise: &str,
    montant: f64,
    horodatage: i64,
) -> Result<(), String> {
    if montant <= 0.0 {
        return Ok(());
    }
    conn.execute(
        "INSERT INTO multi_wallets (uq_number, currency, balance, updated_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(uq_number, currency)
         DO UPDATE SET balance = multi_wallets.balance + excluded.balance,
                       updated_at = excluded.updated_at",
        params![uq, code_devise(devise), montant, horodatage],
    )
    .map_err(|e| format!("Crédit impossible ({})", e))?;
    Ok(())
}

fn debiter(
    conn: &Connection,
    uq: &str,
    devise: &str,
    montant: f64,
    horodatage: i64,
) -> Result<bool, String> {
    let touches = conn
        .execute(
            "UPDATE multi_wallets
             SET balance = balance - ?1, updated_at = ?2
             WHERE UPPER(uq_number) = UPPER(?3) AND UPPER(currency) = UPPER(?4)
               AND balance >= ?1",
            params![montant, horodatage, uq, devise],
        )
        .map_err(|e| format!("Débit impossible ({})", e))?;
    Ok(touches == 1)
}

// ====================
// NOTIFICATIONS
// ====================

fn notifier(conn: &Connection, uq: &str, scope: &str, titre: &str, corps: &str, reference: &str) {
    let _ = conn.execute(
        "INSERT INTO screen_notifications (uq, scope, title, body, reference, read, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6)",
        params![uq, scope, titre, corps, reference, maintenant()],
    );
}

/// Point d'entrée pour les autres modules (Zando, KYC…).
pub fn push_notification(uq: &str, scope: &str, titre: &str, corps: &str, reference: &str) {
    if uq.trim().is_empty() {
        return;
    }
    if let Ok(conn) = ouvrir_index() {
        notifier(&conn, uq, scope, titre, corps, reference);
    }
}

/// Ordre de repli des catégories quand leur activité est aussi récente :
/// ce qui porte de l'argent remonte en premier.
const ORDRE_CATEGORIES: [&str; 5] = ["trading", "coffre", "zando", "chat", "kyc"];

/// Une non-lue rend sa catégorie plus « vivante » que n'importe quel événement
/// lu : les horodatages sont de l'ordre du milliard, ce milliard pèse donc
/// réellement plus que la date.
const POIDS_NON_LUE: i64 = 1_000_000_000_000;

/// Les références dont l'objet est encore vivant. Un ordre limite exécuté ou
/// annulé, une commande P2P terminée et une épargne retirée en sortent : la
/// tête épinglée ne garde que ce qui se déroule maintenant.
fn references_en_cours(conn: &Connection, uq: &str) -> HashSet<String> {
    let requetes: [(&str, &str); 3] = [
        (
            "order",
            "SELECT id FROM trading_orders WHERE UPPER(uq) = UPPER(?1) AND status = 'open'",
        ),
        (
            "p2p",
            "SELECT id FROM p2p_orders
             WHERE (UPPER(buyer) = UPPER(?1) OR UPPER(seller) = UPPER(?1))
               AND status IN ('pending_payment', 'paid')",
        ),
        (
            "earn",
            "SELECT id FROM earn_positions WHERE UPPER(uq) = UPPER(?1) AND status = 'active'",
        ),
    ];

    let mut actives = HashSet::new();
    for (prefixe, sql) in requetes {
        if let Ok(mut stmt) = conn.prepare(sql) {
            if let Ok(rows) = stmt.query_map(params![uq], |row| row.get::<_, i64>(0)) {
                for identifiant in rows.flatten() {
                    actives.insert(format!("{}:{}", prefixe, identifiant));
                }
            }
        }
    }
    actives
}

/// Ordre du flux :
/// 1. les éléments « en cours » forment une tête épinglée, jamais noyée dans
///    l'historique — c'est ce qui attend l'utilisateur maintenant ;
/// 2. le reste est groupé par catégorie, la catégorie la plus vivante d'abord ;
/// 3. dans chaque groupe, les non-lues en haut, puis l'heure décroissante.
/// Le tri est stable : à clé égale l'ordre de production est conservé, ce qui
/// laisse les signaux d'état (`id = 0`, tous horodatés de l'appel) entre eux.
fn trier_notifications(notifications: &mut [Value]) {
    let lue = |n: &Value| n.get("read").and_then(|v| v.as_bool()).unwrap_or(false);
    let date = |n: &Value| {
        n.get("created_at")
            .and_then(|v| v.as_i64())
            .unwrap_or_default()
    };
    let cle = |n: &Value| n.get("id").and_then(|v| v.as_i64()).unwrap_or_default();
    let en_cours = |n: &Value| n.get("en_cours").and_then(|v| v.as_bool()).unwrap_or(false);
    let categorie = |n: &Value| -> String {
        n.get("scope")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };

    // La place d'une catégorie se décide sur son élément le plus récent,
    // priorité aux non-lues, puis sur l'ordre de repli.
    let mut vivacite: HashMap<String, i64> = HashMap::new();
    for n in notifications.iter() {
        let marque = date(n) + if lue(n) { 0 } else { POIDS_NON_LUE };
        let entry = vivacite.entry(categorie(n)).or_insert(i64::MIN);
        if marque > *entry {
            *entry = marque;
        }
    }
    let mut classees: Vec<String> = vivacite.keys().cloned().collect();
    classees.sort_by(|a, b| {
        vivacite[b]
            .cmp(&vivacite[a])
            .then_with(|| rang_categorie(a).cmp(&rang_categorie(b)))
    });
    let places: HashMap<String, usize> = classees
        .into_iter()
        .enumerate()
        .map(|(index, scope)| (scope, index))
        .collect();

    notifications.sort_by(|a, b| {
        en_cours(b)
            .cmp(&en_cours(a))
            .then_with(|| {
                // La tête épinglée reste une liste chronologique : grouper par
                // catégorie y casserait l'ordre d'attente réel.
                if en_cours(a) || en_cours(b) {
                    std::cmp::Ordering::Equal
                } else {
                    place(&places, &categorie(a)).cmp(&place(&places, &categorie(b)))
                }
            })
            // `read` faux (non lue) passe avant `read` vrai : comparaison montante.
            .then_with(|| lue(a).cmp(&lue(b)))
            .then_with(|| date(b).cmp(&date(a)))
            .then_with(|| cle(b).cmp(&cle(a)))
    });
}

fn rang_categorie(scope: &str) -> usize {
    ORDRE_CATEGORIES
        .iter()
        .position(|nom| *nom == scope)
        .unwrap_or(ORDRE_CATEGORIES.len())
}

fn place(places: &HashMap<String, usize>, scope: &str) -> usize {
    *places.get(scope).unwrap_or(&usize::MAX)
}

fn masquer_telephone(numero: &str) -> String {
    let chiffres: String = numero.chars().filter(|c| c.is_ascii_digit()).collect();
    if chiffres.len() < 6 {
        return numero.trim().to_string();
    }
    format!("{} ••• {}", &chiffres[..4], &chiffres[chiffres.len() - 2..])
}

// ====================
// HELPERS RÉPONSES
// ====================

fn refus(message: &str, code: &str) -> HttpResponse {
    HttpResponse::BadRequest().json(json!({
        "status": "error",
        "code": code,
        "message": message
    }))
}

fn echec(message: String) -> HttpResponse {
    HttpResponse::InternalServerError().json(json!({
        "status": "error",
        "message": message
    }))
}

fn statistiques_paire(conn: &Connection, paire: &str, depuis: i64) -> (i64, f64, Option<f64>) {
    let (nombre_trades, volume): (i64, f64) = conn
        .query_row(
            "SELECT COUNT(*), COALESCE(SUM(total), 0) FROM trading_trades
             WHERE pair = ?1 AND created_at >= ?2",
            params![paire, depuis],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap_or((0, 0.0));
    let dernier = conn
        .query_row(
            "SELECT price FROM trading_trades WHERE pair = ?1 ORDER BY id DESC LIMIT 1",
            params![paire],
            |row| row.get::<_, f64>(0),
        )
        .ok();
    (nombre_trades, volume, dernier)
}

fn ordre_json(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "id": row.get::<_, i64>(0)?,
        "uq": row.get::<_, String>(1)?,
        "pair": row.get::<_, String>(2)?,
        "base": row.get::<_, String>(3)?,
        "quote": row.get::<_, String>(4)?,
        "side": row.get::<_, String>(5)?,
        "type": row.get::<_, String>(6)?,
        "price": row.get::<_, f64>(7)?,
        "amount": row.get::<_, f64>(8)?,
        "filled": row.get::<_, f64>(9)?,
        "total": row.get::<_, f64>(10)?,
        "status": row.get::<_, String>(11)?,
        "created_at": row.get::<_, i64>(12)?,
        "updated_at": row.get::<_, i64>(13)?,
    }))
}

const COLONNES_ORDRE: &str =
    "id, uq, pair, base, quote, side, order_type, price, amount, filled, total, status, created_at, updated_at";

// ====================
// GET /api/trading/markets
// ====================

pub async fn trading_markets() -> HttpResponse {
    let taux = crate::public_routes::taux_du_jour().await;
    let conn = match ouvrir_index() {
        Ok(conn) => conn,
        Err(e) => return echec(e),
    };
    let depuis = maintenant() - 86_400;

    let mut codes: Vec<&String> = taux.rates.keys().collect();
    codes.sort();

    let mut marches: Vec<Value> = Vec::new();
    for code in codes {
        if code.eq_ignore_ascii_case("USD") || devise_exclue(&code.to_uppercase()) {
            continue;
        }
        let prix = match lire_taux(&taux.rates, code) {
            Some(prix) if prix > 0.0 => prix,
            _ => continue,
        };
        let paire = nom_paire("USD", &code.to_uppercase());
        let (trades, volume, dernier) = statistiques_paire(&conn, &paire, depuis);
        marches.push(json!({
            "pair": paire,
            "base": "USD",
            "quote": code.to_uppercase(),
            "price": prix,
            "last": dernier.unwrap_or(prix),
            "trades_24h": trades,
            "volume_24h": volume
        }));
    }

    HttpResponse::Ok().json(json!({
        "status": "success",
        "base": taux.base,
        "updated_at": taux.updated_at,
        "source": taux.source,
        "markets": marches
    }))
}

// ====================
// GET /api/trading/context/{uq}
// ====================

pub async fn trading_context(req: HttpRequest, path: web::Path<String>) -> HttpResponse {
    let uq = path.into_inner();
    if let Err(response) = crate::auth_guard::exiger_soi_ou_owner(&req, &uq) {
        return response;
    }
    let conn = match ouvrir_index() {
        Ok(conn) => conn,
        Err(e) => return echec(e),
    };
    let profil = match lire_profil(&conn, &uq) {
        Some(profil) => profil,
        None => {
            return HttpResponse::NotFound().json(json!({
                "status": "error",
                "message": "Compte introuvable"
            }))
        }
    };
    let taux = crate::public_routes::taux_du_jour().await;

    let mut soldes: Vec<Value> = Vec::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT currency, COALESCE(balance, 0) FROM multi_wallets
         WHERE UPPER(uq_number) = UPPER(?1)
           AND COALESCE(balance, 0) > 0
           AND UPPER(currency) NOT IN ('BORB', 'BORB')
         ORDER BY currency",
    ) {
        if let Ok(rows) = stmt.query_map(params![uq], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
        }) {
            for (code, solde) in rows.flatten() {
                let code = code.to_uppercase();
                // devise non observée → null, pas un taux de complaisance
                soldes.push(json!({
                    "code": code,
                    "balance": solde,
                    "usd_value": valeur_usd(&taux.rates, &code, solde)
                }));
            }
        }
    }

    HttpResponse::Ok().json(json!({
        "status": "success",
        "uq": uq,
        "profile": {
            "pseudo": profil.pseudo,
            "phone": profil.phone,
            "operator": profil.operator,
            "kyc_verified": profil.kyc,
            "display_currency": profil.display_currency
        },
        "phone_ready": !profil.phone.trim().is_empty(),
        "pin_configured": pin_configure(&uq),
        "balances": soldes,
        "oracle": {
            "base": taux.base,
            "updated_at": taux.updated_at,
            "source": taux.source,
            "borb_usd": crate::persistence::load_prix().prix_actuel.parse::<f64>().ok()
        }
    }))
}

// ====================
// GET /api/trading/overview/{uq}
// ====================

/// Agrégat de lecture : soldes, fonds réservés par les ordres limites, compteurs
/// d'ordres, trades et commandes P2P. Toutes les valeurs sortent des tables
/// réelles ; aucun rendement ni prix n'est inventé ici.
pub async fn trading_overview(req: HttpRequest, path: web::Path<String>) -> HttpResponse {
    let uq = path.into_inner();
    if let Err(response) = crate::auth_guard::exiger_soi_ou_owner(&req, &uq) {
        return response;
    }
    let conn = match ouvrir_index() {
        Ok(conn) => conn,
        Err(e) => return echec(e),
    };
    let profil = match lire_profil(&conn, &uq) {
        Some(profil) => profil,
        None => {
            return HttpResponse::NotFound().json(json!({
                "status": "error",
                "message": "Compte introuvable"
            }))
        }
    };
    let taux = crate::public_routes::taux_du_jour().await;
    let horodatage = maintenant();
    let depuis = horodatage - 86_400;

    let mut soldes: Vec<Value> = Vec::new();
    let mut disponibles_usd = 0.0;
    // Devises que l'oracle n'a pas observées : leur montant n'entre pas dans le
    // total, et le client doit pouvoir le dire au lieu de laisser croire à un
    // portefeuille plus petit.
    let mut non_evalues: Vec<String> = Vec::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT currency, COALESCE(balance, 0) FROM multi_wallets
         WHERE UPPER(uq_number) = UPPER(?1)
           AND COALESCE(balance, 0) > 0
           AND UPPER(currency) NOT IN ('BORB', 'BORB')
         ORDER BY currency",
    ) {
        if let Ok(rows) = stmt.query_map(params![uq], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
        }) {
            for (code, solde) in rows.flatten() {
                let code = code.to_uppercase();
                let valeur = valeur_usd(&taux.rates, &code, solde);
                match valeur {
                    Some(v) => disponibles_usd += v,
                    None => non_evalues.push(code.clone()),
                }
                soldes.push(json!({
                    "code": code,
                    "balance": solde,
                    "usd_value": valeur
                }));
            }
        }
    }

    let mut statuts: HashMap<String, i64> = HashMap::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT status, COUNT(*) FROM trading_orders
         WHERE UPPER(uq) = UPPER(?1) GROUP BY status",
    ) {
        if let Ok(rows) = stmt.query_map(params![uq], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        }) {
            for (statut, nombre) in rows.flatten() {
                statuts.insert(statut, nombre);
            }
        }
    }

    // Les ordres limites débitent déjà le solde : on expose ici ce qui est immobilisé.
    let mut reserves: HashMap<String, f64> = HashMap::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT side, base, quote, amount, price FROM trading_orders
         WHERE UPPER(uq) = UPPER(?1) AND status = 'open'",
    ) {
        if let Ok(rows) = stmt.query_map(params![uq], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, f64>(3)?,
                row.get::<_, f64>(4)?,
            ))
        }) {
            for (cote, base, quote, montant, prix) in rows.flatten() {
                let (devise, valeur) = if cote == "buy" {
                    (quote, montant * prix)
                } else {
                    (base, montant)
                };
                *reserves.entry(devise.to_uppercase()).or_insert(0.0) += valeur;
            }
        }
    }

    let mut reserves_json: Vec<Value> = Vec::new();
    let mut reserves_usd = 0.0;
    for (devise, montant) in reserves {
        let valeur = valeur_usd(&taux.rates, &devise, montant);
        match valeur {
            Some(v) => reserves_usd += v,
            None => non_evalues.push(devise.clone()),
        }
        reserves_json.push(json!({
            "currency": devise,
            "amount": montant,
            "usd_value": valeur
        }));
    }
    reserves_json.sort_by(|a, b| {
        a.get("currency")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .cmp(b.get("currency").and_then(|v| v.as_str()).unwrap_or(""))
    });

    let mut trades_24h: i64 = 0;
    let mut volume_trades_usd_24h = 0.0;
    if let Ok(mut stmt) = conn.prepare(
        "SELECT pair, total FROM trading_trades
         WHERE UPPER(uq) = UPPER(?1) AND created_at >= ?2",
    ) {
        if let Ok(rows) = stmt.query_map(params![uq, depuis], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
        }) {
            for (paire, total) in rows.flatten() {
                trades_24h += 1;
                let quote = paire.split('-').nth(1).unwrap_or("");
                volume_trades_usd_24h += valeur_usd(&taux.rates, quote, total).unwrap_or(0.0);
            }
        }
    }

    let total_trades: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM trading_trades WHERE UPPER(uq) = UPPER(?1)",
            params![uq],
            |row| row.get(0),
        )
        .unwrap_or(0);

    let mut derniers_trades: Vec<Value> = Vec::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT pair, side, price, amount, total, created_at FROM trading_trades
         WHERE UPPER(uq) = UPPER(?1) ORDER BY id DESC LIMIT 10",
    ) {
        if let Ok(rows) = stmt.query_map(params![uq], |row| {
            Ok(json!({
                "pair": row.get::<_, String>(0)?,
                "side": row.get::<_, String>(1)?,
                "price": row.get::<_, f64>(2)?,
                "amount": row.get::<_, f64>(3)?,
                "total": row.get::<_, f64>(4)?,
                "created_at": row.get::<_, i64>(5)?
            }))
        }) {
            derniers_trades.extend(rows.flatten());
        }
    }

    let mut p2p_statuts: HashMap<String, i64> = HashMap::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT status, COUNT(*) FROM p2p_orders
         WHERE UPPER(buyer) = UPPER(?1) OR UPPER(seller) = UPPER(?1)
         GROUP BY status",
    ) {
        if let Ok(rows) = stmt.query_map(params![uq], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        }) {
            for (statut, nombre) in rows.flatten() {
                p2p_statuts.insert(statut, nombre);
            }
        }
    }
    let p2p_actives = p2p_statuts.get("pending_payment").copied().unwrap_or(0)
        + p2p_statuts.get("paid").copied().unwrap_or(0);
    let p2p_terminees = p2p_statuts.get("completed").copied().unwrap_or(0);
    let p2p_annulees = p2p_statuts.get("cancelled").copied().unwrap_or(0);

    let mut nombre_paires = 0;
    for (code, prix) in &taux.rates {
        if code.eq_ignore_ascii_case("USD") || devise_exclue(&code.to_uppercase()) {
            continue;
        }
        if *prix > 0.0 {
            nombre_paires += 1;
        }
    }

    let mut marches_trades_24h: i64 = 0;
    let mut marches_volume_usd_24h = 0.0;
    if let Ok(mut stmt) = conn.prepare(
        "SELECT pair, total FROM trading_trades WHERE created_at >= ?1",
    ) {
        if let Ok(rows) = stmt.query_map(params![depuis], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
        }) {
            for (paire, total) in rows.flatten() {
                marches_trades_24h += 1;
                let quote = paire.split('-').nth(1).unwrap_or("");
                marches_volume_usd_24h += valeur_usd(&taux.rates, quote, total).unwrap_or(0.0);
            }
        }
    }

    let devise_affichage = if profil.display_currency.trim().is_empty() {
        "USD".to_string()
    } else {
        profil.display_currency.to_uppercase()
    };
    let total_usd = disponibles_usd + reserves_usd;
    let (devise_rendue, total_affiche) = match lire_taux(&taux.rates, &devise_affichage) {
        Some(taux_devise) if taux_devise > 0.0 => {
            (devise_affichage, total_usd * taux_devise)
        }
        _ => ("USD".to_string(), total_usd),
    };

    HttpResponse::Ok().json(json!({
        "status": "success",
        "uq": uq,
        "updated_at": horodatage,
        "oracle": {
            "base": taux.base,
            "updated_at": taux.updated_at,
            "source": taux.source,
            "borb_usd": crate::persistence::load_prix().prix_actuel.parse::<f64>().ok()
        },
        "portfolio": {
            "display_currency": devise_rendue,
            "available_usd": disponibles_usd,
            "locked_usd": reserves_usd,
            "total_usd": total_usd,
            "total_display": total_affiche,
            "non_evaluees": non_evalues,
            "balances": soldes
        },
        "orders": {
            "open": statuts.get("open").copied().unwrap_or(0),
            "filled": statuts.get("filled").copied().unwrap_or(0),
            "cancelled": statuts.get("cancelled").copied().unwrap_or(0),
            "total": statuts.values().sum::<i64>(),
            "reserved": reserves_json
        },
        "trades": {
            "count_24h": trades_24h,
            "volume_usd_24h": volume_trades_usd_24h,
            "count_total": total_trades,
            "last": derniers_trades
        },
        "p2p": {
            "active": p2p_actives,
            "completed": p2p_terminees,
            "cancelled": p2p_annulees,
            "total": p2p_actives + p2p_terminees + p2p_annulees
        },
        "markets": {
            "pairs": nombre_paires,
            "trades_24h": marches_trades_24h,
            "volume_usd_24h": marches_volume_usd_24h
        }
    }))
}

// ====================
// GET /api/trading/funding/{uq}
// ====================

/// Historique réel des mouvements de portefeuille (envois et reçus).
/// L'application n'a pas de dépôt/retrait externe : cette route ne simule donc
/// pas un "Funding" bancaire, elle expose uniquement les transferts existants.
pub async fn trading_funding(req: HttpRequest, path: web::Path<String>) -> HttpResponse {
    let uq = path.into_inner();
    if let Err(response) = crate::auth_guard::exiger_soi_ou_owner(&req, &uq) {
        return response;
    }
    let conn = match ouvrir_index() {
        Ok(conn) => conn,
        Err(e) => return echec(e),
    };
    if lire_profil(&conn, &uq).is_none() {
        return HttpResponse::NotFound().json(json!({
            "status": "error",
            "message": "Compte introuvable"
        }));
    }

    let mut totaux: Vec<Value> = Vec::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT currency,
                SUM(CASE WHEN UPPER(sender_uq) = UPPER(?1) THEN amount ELSE 0 END),
                SUM(CASE WHEN UPPER(recipient_uq) = UPPER(?1) THEN amount ELSE 0 END),
                SUM(CASE WHEN UPPER(sender_uq) = UPPER(?1) THEN fee ELSE 0 END),
                COUNT(*)
         FROM wallet_transfers
         WHERE UPPER(sender_uq) = UPPER(?1) OR UPPER(recipient_uq) = UPPER(?1)
         GROUP BY currency
         ORDER BY currency",
    ) {
        if let Ok(rows) = stmt.query_map(params![uq], |row| {
            Ok(json!({
                "currency": row.get::<_, String>(0)?,
                "sent": row.get::<_, f64>(1)?,
                "received": row.get::<_, f64>(2)?,
                "fees": row.get::<_, f64>(3)?,
                "count": row.get::<_, i64>(4)?
            }))
        }) {
            totaux.extend(rows.flatten());
        }
    }

    let mut transferts: Vec<Value> = Vec::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT transfer_id,
                CASE WHEN UPPER(sender_uq) = UPPER(?1) THEN 'send' ELSE 'receive' END,
                CASE WHEN UPPER(sender_uq) = UPPER(?1) THEN recipient_uq ELSE sender_uq END,
                currency, amount, fee, memo, created_at
         FROM wallet_transfers
         WHERE UPPER(sender_uq) = UPPER(?1) OR UPPER(recipient_uq) = UPPER(?1)
         ORDER BY created_at DESC
         LIMIT 100",
    ) {
        if let Ok(rows) = stmt.query_map(params![uq], |row| {
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "type": row.get::<_, String>(1)?,
                "counterparty": row.get::<_, String>(2)?,
                "currency": row.get::<_, String>(3)?,
                "amount": row.get::<_, f64>(4)?,
                "fee": row.get::<_, f64>(5)?,
                "memo": row.get::<_, String>(6)?,
                "timestamp": row.get::<_, i64>(7)?
            }))
        }) {
            for transfert in rows.flatten() {
                let mut valeur = transfert;
                if let Some(contrepartie) = valeur.get("counterparty").and_then(|v| v.as_str()) {
                    valeur["counterparty"] =
                        json!(crate::uq_public_alias::get_public_display(contrepartie));
                }
                transferts.push(valeur);
            }
        }
    }

    HttpResponse::Ok().json(json!({
        "status": "success",
        "uq": uq,
        "totals": totaux,
        "transfers": transferts
    }))
}

// ====================
// EARN — épargne verrouillée financée par Case 2
// ====================

/// Devises éligibles à l'épargne (règle produit).
const EARN_CURRENCIES: [&str; 3] = crate::frais_plateforme::DEVISES_EARN;
const EARN_LOCK_DAYS: i64 = 30;
const EARN_EARLY_PENALTY_PCT: f64 = 0.10;
/// Part des frais affectée à Case 2 (40 %) — source d'Earn.
const EARN_FEE_SHARE_PCT: f64 = crate::frais_plateforme::PART_CASE_FRAIS;
const EARN_DISTRIBUTION_SECS: i64 = 86_400;

fn devise_earn(brut: &str) -> Option<String> {
    crate::frais_plateforme::devise_earn(brut)
}

/// Retourne (anticipe, penalite, montant_rendu) pour un retrait Earn.
fn calcul_retrait(
    principal: f64,
    rewards: f64,
    deverrouillage: i64,
    horodatage: i64,
) -> (bool, f64, f64) {
    let anticipe = horodatage < deverrouillage;
    let penalite = if anticipe {
        principal * EARN_EARLY_PENALTY_PCT
    } else {
        0.0
    };
    (anticipe, penalite, (principal - penalite + rewards).max(0.0))
}

/// Barème de service : 0,1 % fiat, 1 % crypto (source unique : frais_plateforme).
fn taux_frais_trading(code: &str) -> f64 {
    crate::frais_plateforme::taux_frais(code)
}

/// Verse les frais Case 2 accumulés, au plus une fois par jour.
/// Le montant est réparti au prorata des montants verrouillés de la devise.
fn assurer_distribution(conn: &Connection) -> Result<(), String> {
    let horodatage = maintenant();
    let dernier: i64 = conn
        .query_row(
            "SELECT last_distribution_at FROM earn_state WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if horodatage - dernier < EARN_DISTRIBUTION_SECS {
        return Ok(());
    }

    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("Distribution Earn impossible ({})", e))?;

    let dernier: i64 = tx
        .query_row(
            "SELECT last_distribution_at FROM earn_state WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if horodatage - dernier >= EARN_DISTRIBUTION_SECS {
        let mut devises: Vec<String> = Vec::new();
        if let Ok(mut stmt) = tx.prepare(
            "SELECT currency FROM earn_pool WHERE collected > distributed ORDER BY currency",
        ) {
            if let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(0)) {
                devises.extend(rows.flatten());
            }
        }

        for devise in devises {
            distribuer_devise(&tx, &devise, horodatage)?;
        }

        tx.execute(
            "UPDATE earn_state SET last_distribution_at = ?1 WHERE id = 1",
            params![horodatage],
        )
        .map_err(|e| format!("Distribution Earn impossible ({})", e))?;
    }

    tx.commit()
        .map_err(|e| format!("Distribution Earn impossible ({})", e))?;
    Ok(())
}

fn distribuer_devise(tx: &Connection, devise: &str, horodatage: i64) -> Result<(), String> {
    let (collecte, distribue): (f64, f64) = tx
        .query_row(
            "SELECT collected, distributed FROM earn_pool WHERE currency = ?1",
            params![devise],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|e| format!("Case 2 illisible ({})", e))?;
    let disponible = collecte - distribue;
    if disponible <= 0.0 {
        return Ok(());
    }

    let mut positions: Vec<(i64, f64)> = Vec::new();
    if let Ok(mut stmt) = tx.prepare(
        "SELECT id, principal FROM earn_positions
         WHERE status = 'active' AND UPPER(currency) = UPPER(?1)
         ORDER BY id",
    ) {
        if let Ok(rows) = stmt.query_map(params![devise], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?))
        }) {
            positions.extend(rows.flatten());
        }
    }

    let total: f64 = positions.iter().map(|(_, principal)| principal).sum();
    if positions.is_empty() || total <= 0.0 {
        return Ok(());
    }

    let mut verses = 0.0;
    for (index, (position_id, principal)) in positions.iter().enumerate() {
        let part = if index + 1 == positions.len() {
            disponible - verses
        } else {
            disponible * principal / total
        };
        if part <= 0.0 {
            continue;
        }
        tx.execute(
            "INSERT INTO earn_rewards (position_id, uq, currency, amount, created_at)
             SELECT id, uq, currency, ?2, ?3 FROM earn_positions WHERE id = ?1",
            params![position_id, part, horodatage],
        )
        .map_err(|e| format!("Versement Earn impossible ({})", e))?;
        tx.execute(
            "UPDATE earn_positions SET rewards = rewards + ?1, updated_at = ?2 WHERE id = ?3",
            params![part, horodatage, position_id],
        )
        .map_err(|e| format!("Versement Earn impossible ({})", e))?;
        verses += part;
    }

    tx.execute(
        "UPDATE earn_pool SET distributed = distributed + ?1, updated_at = ?2 WHERE currency = ?3",
        params![disponible, horodatage, devise],
    )
    .map_err(|e| format!("Case 2 illisible ({})", e))?;
    Ok(())
}

// ====================
// GET /api/trading/earn/context/{uq}
// ====================

/// Produits Earn : taux journalier et APY réellement constatés sur la
/// récompense versée aux 24 dernières heures, jamais un taux annoncé.
fn produits_earn(conn: &Connection, horodatage: i64) -> Vec<Value> {
    let mut produits: Vec<Value> = Vec::new();
    for devise in EARN_CURRENCIES {
        let (collecte, distribue): (f64, f64) = conn
            .query_row(
                "SELECT collected, distributed FROM earn_pool WHERE currency = ?1",
                params![devise],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap_or((0.0, 0.0));
        let verrouille: f64 = conn
            .query_row(
                "SELECT COALESCE(SUM(principal), 0) FROM earn_positions
                 WHERE status = 'active' AND UPPER(currency) = UPPER(?1)",
                params![devise],
                |row| row.get(0),
            )
            .unwrap_or(0.0);
        let verses_24h: f64 = conn
            .query_row(
                "SELECT COALESCE(SUM(amount), 0) FROM earn_rewards
                 WHERE UPPER(currency) = UPPER(?1) AND created_at >= ?2",
                params![devise, horodatage - EARN_DISTRIBUTION_SECS],
                |row| row.get(0),
            )
            .unwrap_or(0.0);
        let taux_jour = if verrouille > 0.0 {
            verses_24h / verrouille
        } else {
            0.0
        };
        let apy = if taux_jour > 0.0 {
            ((1.0 + taux_jour).powf(365.0) - 1.0) * 100.0
        } else {
            0.0
        };
        produits.push(json!({
            "currency": devise,
            "staked": verrouille,
            "rewarded_24h": verses_24h,
            "daily_pct": taux_jour * 100.0,
            "apy_pct": apy,
            "case2_available": (collecte - distribue).max(0.0)
        }));
    }
    produits
}

pub async fn trading_earn_context(req: HttpRequest, path: web::Path<String>) -> HttpResponse {
    let uq = path.into_inner();
    if let Err(response) = crate::auth_guard::exiger_soi_ou_owner(&req, &uq) {
        return response;
    }
    let conn = match ouvrir_index() {
        Ok(conn) => conn,
        Err(e) => return echec(e),
    };
    if lire_profil(&conn, &uq).is_none() {
        return HttpResponse::NotFound().json(json!({
            "status": "error",
            "message": "Compte introuvable"
        }));
    }
    if let Err(e) = assurer_distribution(&conn) {
        return echec(e);
    }

    let horodatage = maintenant();

    let mut wallets: Vec<Value> = Vec::new();
    for devise in EARN_CURRENCIES {
        let solde: f64 = conn
            .query_row(
                "SELECT COALESCE(balance, 0) FROM multi_wallets
                 WHERE UPPER(uq_number) = UPPER(?1) AND UPPER(currency) = UPPER(?2)",
                params![uq, devise],
                |row| row.get(0),
            )
            .unwrap_or(0.0);
        wallets.push(json!({ "currency": devise, "balance": solde }));
    }

    let produits = produits_earn(&conn, horodatage);

    let mut positions: Vec<Value> = Vec::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT id, currency, principal, rewards, locked_at, unlock_at
         FROM earn_positions
         WHERE UPPER(uq) = UPPER(?1) AND status = 'active'
         ORDER BY id DESC LIMIT 50",
    ) {
        if let Ok(rows) = stmt.query_map(params![uq], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, f64>(2)?,
                row.get::<_, f64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
            ))
        }) {
            for (id, devise, principal, rewards, locked_at, unlock_at) in rows.flatten() {
                let jours_restants = ((unlock_at - horodatage) as f64 / 86400.0)
                    .ceil()
                    .max(0.0) as i64;
                let penalite = if horodatage < unlock_at {
                    principal * EARN_EARLY_PENALTY_PCT
                } else {
                    0.0
                };
                positions.push(json!({
                    "id": id,
                    "currency": devise,
                    "principal": principal,
                    "rewards": rewards,
                    "locked_at": locked_at,
                    "unlock_at": unlock_at,
                    "days_left": jours_restants,
                    "can_withdraw": horodatage >= unlock_at,
                    "early_penalty_preview": penalite
                }));
            }
        }
    }

    let mut historique: Vec<Value> = Vec::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT position_id, currency, amount, created_at FROM earn_rewards
         WHERE UPPER(uq) = UPPER(?1) ORDER BY id DESC LIMIT 20",
    ) {
        if let Ok(rows) = stmt.query_map(params![uq], |row| {
            Ok(json!({
                "position_id": row.get::<_, i64>(0)?,
                "currency": row.get::<_, String>(1)?,
                "amount": row.get::<_, f64>(2)?,
                "created_at": row.get::<_, i64>(3)?
            }))
        }) {
            historique.extend(rows.flatten());
        }
    }

    HttpResponse::Ok().json(json!({
        "status": "success",
        "uq": uq,
        "updated_at": horodatage,
        "policy": {
            "currencies": EARN_CURRENCIES,
            "lock_days": EARN_LOCK_DAYS,
            "early_penalty_pct": EARN_EARLY_PENALTY_PCT * 100.0,
            "fee_share_pct": EARN_FEE_SHARE_PCT * 100.0,
            "distribution_hours": EARN_DISTRIBUTION_SECS / 3600,
            "trading_fee_fiat_pct": taux_frais_trading("USD") * 100.0,
            "trading_fee_crypto_pct": taux_frais_trading("BTC") * 100.0,
            "fee_sources": ["send", "market", "convert", "withdraw"],
            "fee_case_split": {
                "tresor": crate::frais_plateforme::REPARTITION_CASES[0] * 100.0,
                "frais": crate::frais_plateforme::REPARTITION_CASES[1] * 100.0,
                "recompenses": crate::frais_plateforme::REPARTITION_CASES[2] * 100.0,
                "epargne": crate::frais_plateforme::REPARTITION_CASES[3] * 100.0,
                "credit": crate::frais_plateforme::REPARTITION_CASES[4] * 100.0,
                "bouclier": crate::frais_plateforme::REPARTITION_CASES[5] * 100.0
            },
            "limit_orders_fee_free_until_execution": true,
            "borb_borb_fee_free": true
        },
        "wallets": wallets,
        "products": produits,
        "positions": positions,
        "history": historique
    }))
}

// ====================
// GET /api/trading/earn/rewards/{uq}
// ====================

/// Périodes acceptées par la courbe de rendement.
const EARN_PERIODES: [(&str, i64); 3] = [
    ("7j", 7 * 86_400),
    ("30j", 30 * 86_400),
    ("1an", 365 * 86_400),
];

/// Courbe privée des récompenses Earn, agrégée par jour et par devise.
/// Chaque point vient de `earn_rewards` : rien n'est interpolé ni simulé.
pub async fn trading_earn_rewards(
    req: HttpRequest,
    path: web::Path<String>,
    query: web::Query<HashMap<String, String>>,
) -> HttpResponse {
    let uq = path.into_inner();
    if let Err(response) = crate::auth_guard::exiger_soi_ou_owner(&req, &uq) {
        return response;
    }

    let periode_brut = query
        .get("periode")
        .map(|v| v.trim().to_string())
        .unwrap_or_else(|| "30j".to_string());
    let Some((periode, secondes)) = EARN_PERIODES
        .iter()
        .find(|(nom, _)| *nom == periode_brut)
        .copied()
    else {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "code": "PERIODE invalide",
            "message": "Période acceptée : 7j, 30j ou 1an",
            "allowed": EARN_PERIODES.iter().map(|(nom, _)| *nom).collect::<Vec<_>>()
        }));
    };

    let conn = match ouvrir_index() {
        Ok(conn) => conn,
        Err(e) => return echec(e),
    };
    if lire_profil(&conn, &uq).is_none() {
        return HttpResponse::NotFound().json(json!({
            "status": "error",
            "message": "Compte introuvable"
        }));
    }

    let horodatage = maintenant();
    let depuis = horodatage - secondes;

    fn recompenses_par_jour(
        conn: &Connection,
        uq: &str,
        depuis: i64,
    ) -> Result<Vec<(String, String, f64, i64)>, String> {
        let mut stmt = conn
            .prepare(
                "SELECT date(created_at, 'unixepoch'), currency,
                        COALESCE(SUM(amount), 0), COUNT(*)
                 FROM earn_rewards
                 WHERE UPPER(uq) = UPPER(?1) AND created_at >= ?2
                 GROUP BY 1, 2
                 ORDER BY 1 ASC, 2 ASC",
            )
            .map_err(|e| e.to_string())?;
        let lignes = stmt
            .query_map(params![uq, depuis], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, f64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        lignes
            .map(|ligne| ligne.map_err(|e| e.to_string()))
            .collect()
    }

    let lignes = match recompenses_par_jour(&conn, &uq, depuis) {
        Ok(lignes) => lignes,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "code": "LECTURE_IMPOSSIBLE",
                "message": format!("Historique des récompenses illisible : {}", e)
            }))
        }
    };

    let mut serie: Vec<Value> = Vec::new();
    let mut cours: HashMap<String, f64> = HashMap::new();
    let mut recompenses_comptees = 0i64;

    for (jour, devise, montant, lignes_jour) in lignes {
        let cumulative = cours.entry(devise.clone()).or_insert(0.0);
        *cumulative += montant;
        recompenses_comptees += lignes_jour;
        serie.push(json!({
            "date": jour,
            "currency": devise,
            "amount": montant,
            "cumulative": *cumulative,
            "reward_count": lignes_jour
        }));
    }

    let mut par_devise: Vec<&String> = cours.keys().collect();
    par_devise.sort();
    let totaux: Vec<Value> = par_devise
        .iter()
        .map(|devise| json!({ "currency": devise, "total": cours[devise.as_str()] }))
        .collect();

    HttpResponse::Ok().json(json!({
        "status": "success",
        "uq": uq,
        "periode": periode,
        "secondes": secondes,
        "depuis": depuis,
        "updated_at": horodatage,
        "series": serie,
        "totaux": totaux,
        "recompenses_comptees": recompenses_comptees,
        "points": serie.len(),
        "produits": produits_earn(&conn, horodatage)
    }))
}

// ====================
// POST /api/trading/earn/subscribe
// ====================

pub async fn trading_earn_subscribe(req: HttpRequest, body: web::Json<Value>) -> HttpResponse {
    let uq = texte(&body, "uq");
    if uq.is_empty() {
        return refus("Compte requis", "UQ_REQUIS");
    }
    if let Err(response) = crate::auth_guard::exiger_soi_ou_owner(&req, &uq) {
        return response;
    }
    let devise = match devise_earn(&texte(&body, "currency")) {
        Some(devise) => devise,
        None => return refus("Devise non éligible (USD, CDF, EUR)", "DEVISE_INVALIDE"),
    };
    let montant = nombre(&body, "amount");
    if !montant.is_finite() || montant <= 0.0 {
        return refus("Montant invalide", "MONTANT_INVALIDE");
    }
    let pin = texte(&body, "pin");
    if pin.is_empty() {
        return refus("Code PIN requis", "PIN_REQUIS");
    }
    match verifier_pin(&uq, &pin) {
        Ok(true) => {}
        Ok(false) => return refus("Code PIN incorrect", "PIN_INCORRECT"),
        Err(e) => return echec(e),
    }

    let conn = match ouvrir_index() {
        Ok(conn) => conn,
        Err(e) => return echec(e),
    };
    if lire_profil(&conn, &uq).is_none() {
        return HttpResponse::NotFound().json(json!({
            "status": "error",
            "message": "Compte introuvable"
        }));
    }
    if let Err(e) = assurer_distribution(&conn) {
        return echec(e);
    }

    let horodatage = maintenant();
    let deverrouillage = horodatage + EARN_LOCK_DAYS * 86_400;
    let tx = match conn.unchecked_transaction() {
        Ok(tx) => tx,
        Err(e) => return echec(format!("Ouverture impossible ({})", e)),
    };

    match debiter(&tx, &uq, &devise, montant, horodatage) {
        Ok(true) => {}
        Ok(false) => {
            return refus(
                &format!("Solde {} insuffisant", devise),
                "SOLDE_INSUFFISANT",
            )
        }
        Err(e) => return echec(e),
    }

    if let Err(e) = tx.execute(
        "INSERT INTO earn_positions
            (uq, currency, principal, rewards, status, locked_at, unlock_at, created_at, updated_at)
         VALUES (?1, ?2, ?3, 0, 'active', ?4, ?5, ?4, ?4)",
        params![uq, devise, montant, horodatage, deverrouillage],
    ) {
        return echec(format!("Ouverture impossible ({})", e));
    }
    let position_id = tx.last_insert_rowid();

    if let Err(e) = tx.commit() {
        return echec(format!("Ouverture non validée ({})", e));
    }

    notifier(
        &conn,
        &uq,
        "trading",
        "Épargne ouverte",
        &format!(
            "{} {} verrouillés jusqu'au {}",
            montant,
            devise,
            chrono::DateTime::from_timestamp(deverrouillage, 0)
                .map(|date| date.format("%d/%m/%Y").to_string())
                .unwrap_or_else(|| deverrouillage.to_string())
        ),
        &format!("earn:{}", position_id),
    );

    HttpResponse::Ok().json(json!({
        "status": "success",
        "position": {
            "id": position_id,
            "currency": devise,
            "principal": montant,
            "locked_at": horodatage,
            "unlock_at": deverrouillage,
            "lock_days": EARN_LOCK_DAYS
        }
    }))
}

// ====================
// POST /api/trading/earn/withdraw
// ====================

pub async fn trading_earn_withdraw(req: HttpRequest, body: web::Json<Value>) -> HttpResponse {
    let uq = texte(&body, "uq");
    if uq.is_empty() {
        return refus("Compte requis", "UQ_REQUIS");
    }
    if let Err(response) = crate::auth_guard::exiger_soi_ou_owner(&req, &uq) {
        return response;
    }
    let position_id = nombre(&body, "position_id") as i64;
    if position_id <= 0 {
        return refus("Position invalide", "POSITION_INVALIDE");
    }
    let pin = texte(&body, "pin");
    if pin.is_empty() {
        return refus("Code PIN requis", "PIN_REQUIS");
    }
    match verifier_pin(&uq, &pin) {
        Ok(true) => {}
        Ok(false) => return refus("Code PIN incorrect", "PIN_INCORRECT"),
        Err(e) => return echec(e),
    }

    let conn = match ouvrir_index() {
        Ok(conn) => conn,
        Err(e) => return echec(e),
    };
    if let Err(e) = assurer_distribution(&conn) {
        return echec(e);
    }

    let horodatage = maintenant();
    let tx = match conn.unchecked_transaction() {
        Ok(tx) => tx,
        Err(e) => return echec(format!("Retrait impossible ({})", e)),
    };

    let position: Option<(String, f64, f64, i64)> = tx
        .query_row(
            "SELECT currency, principal, rewards, unlock_at FROM earn_positions
             WHERE id = ?1 AND UPPER(uq) = UPPER(?2) AND status = 'active'",
            params![position_id, uq],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .ok();
    let (devise, principal, rewards, deverrouillage) = match position {
        Some(position) => position,
        None => return refus("Position introuvable ou déjà retirée", "POSITION_INTROUVABLE"),
    };

    let (anticipe, penalite, retour) =
        calcul_retrait(principal, rewards, deverrouillage, horodatage);

    let touches = tx.execute(
        "UPDATE earn_positions SET status = 'withdrawn', withdrawn_at = ?1, updated_at = ?1
         WHERE id = ?2 AND status = 'active'",
        params![horodatage, position_id],
    );
    match touches {
        Ok(1) => {}
        _ => return refus("Position déjà retirée", "POSITION_INTROUVABLE"),
    }

    if let Err(e) = crediter(&tx, &uq, &devise, retour, horodatage) {
        return echec(e);
    }
    if penalite > 0.0 {
        if let Err(e) = tx.execute(
            "INSERT INTO earn_pool (currency, collected, distributed, updated_at)
             VALUES (?1, ?2, 0, ?3)
             ON CONFLICT(currency) DO UPDATE SET
                 collected = earn_pool.collected + excluded.collected,
                 updated_at = excluded.updated_at",
            params![devise, penalite, horodatage],
        ) {
            return echec(format!("Pénalité non enregistrée ({})", e));
        }
    }

    if let Err(e) = tx.commit() {
        return echec(format!("Retrait non validé ({})", e));
    }

    notifier(
        &conn,
        &uq,
        "trading",
        if anticipe {
            "Retrait anticipé"
        } else {
            "Épargne retirée"
        },
        &format!("{} {} crédités (pénalité {} {})", retour, devise, penalite, devise),
        &format!("earn:{}", position_id),
    );

    HttpResponse::Ok().json(json!({
        "status": "success",
        "position_id": position_id,
        "currency": devise,
        "principal": principal,
        "rewards": rewards,
        "early": anticipe,
        "penalty": penalite,
        "returned": retour
    }))
}

// ====================
// GET /api/trading/orderbook/{pair}
// ====================

fn niveaux(conn: &Connection, paire: &str, cote: &str, descendant: bool) -> Vec<Value> {
    let ordre = if descendant { "DESC" } else { "ASC" };
    let sql = format!(
        "SELECT price, SUM(amount - filled) AS reste, COUNT(*) AS nombre
         FROM trading_orders
         WHERE pair = ?1 AND status = 'open' AND side = ?2
         GROUP BY price
         HAVING SUM(amount - filled) > 0
         ORDER BY price {}
         LIMIT 20",
        ordre
    );
    let mut resultat = Vec::new();
    let Ok(mut stmt) = conn.prepare(&sql) else {
        return resultat;
    };
    if let Ok(rows) = stmt.query_map(params![paire, cote], |row| {
        Ok(json!({
            "price": row.get::<_, f64>(0)?,
            "amount": row.get::<_, f64>(1)?,
            "orders": row.get::<_, i64>(2)?
        }))
    }) {
        resultat.extend(rows.flatten());
    }
    resultat
}

pub async fn trading_orderbook(path: web::Path<String>) -> HttpResponse {
    let (base, quote) = match parse_paire(&path.into_inner()) {
        Some(paire) => paire,
        None => return refus("Paire inconnue", "PAIRE_INVALIDE"),
    };
    let paire = nom_paire(&base, &quote);
    let conn = match ouvrir_index() {
        Ok(conn) => conn,
        Err(e) => return echec(e),
    };
    let taux = crate::public_routes::taux_du_jour().await;
    let (_, _, dernier) = statistiques_paire(&conn, &paire, 0);

    HttpResponse::Ok().json(json!({
        "status": "success",
        "pair": paire,
        "base": base,
        "quote": quote,
        "oracle_price": prix_paire(&taux.rates, &base, &quote),
        "last_price": dernier,
        "bids": niveaux(&conn, &paire, "buy", true),
        "asks": niveaux(&conn, &paire, "sell", false)
    }))
}

// ====================
// GET /api/trading/trades/{pair}
// ====================

pub async fn trading_trades_list(path: web::Path<String>) -> HttpResponse {
    let (base, quote) = match parse_paire(&path.into_inner()) {
        Some(paire) => paire,
        None => return refus("Paire inconnue", "PAIRE_INVALIDE"),
    };
    let paire = nom_paire(&base, &quote);
    let conn = match ouvrir_index() {
        Ok(conn) => conn,
        Err(e) => return echec(e),
    };

    let mut trades: Vec<Value> = Vec::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT price, amount, total, side, created_at FROM trading_trades
         WHERE pair = ?1 ORDER BY id DESC LIMIT 50",
    ) {
        if let Ok(rows) = stmt.query_map(params![paire], |row| {
            Ok(json!({
                "price": row.get::<_, f64>(0)?,
                "amount": row.get::<_, f64>(1)?,
                "total": row.get::<_, f64>(2)?,
                "side": row.get::<_, String>(3)?,
                "created_at": row.get::<_, i64>(4)?
            }))
        }) {
            trades.extend(rows.flatten());
        }
    }

    HttpResponse::Ok().json(json!({
        "status": "success",
        "pair": paire,
        "base": base,
        "quote": quote,
        "trades": trades
    }))
}

// ====================
// POST /api/trading/orders
// ====================

pub async fn trading_order_create(req: HttpRequest, body: web::Json<Value>) -> HttpResponse {
    let uq = texte(&body, "uq");
    if uq.is_empty() {
        return refus("Compte requis", "UQ_REQUIS");
    }
    if let Err(response) = crate::auth_guard::exiger_soi_ou_owner(&req, &uq) {
        return response;
    }

    let (base, quote) = match parse_paire(&texte(&body, "pair")) {
        Some(paire) => paire,
        None => return refus("Paire inconnue", "PAIRE_INVALIDE"),
    };
    let cote = texte(&body, "side").to_lowercase();
    if cote != "buy" && cote != "sell" {
        return refus("Sens invalide (buy ou sell)", "SENS_INVALIDE");
    }
    let type_ordre = {
        let brut = texte(&body, "type").to_lowercase();
        if brut.is_empty() {
            "market".to_string()
        } else {
            brut
        }
    };
    if type_ordre != "market" && type_ordre != "limit" {
        return refus("Type d'ordre invalide", "TYPE_INVALIDE");
    }
    let montant = nombre(&body, "amount");
    if !montant.is_finite() || montant <= 0.0 {
        return refus("Montant invalide", "MONTANT_INVALIDE");
    }

    let pin = texte(&body, "pin");
    if pin.is_empty() {
        return refus("Code PIN requis pour un ordre", "PIN_REQUIS");
    }
    match verifier_pin(&uq, &pin) {
        Ok(true) => {}
        Ok(false) => return refus("Code PIN incorrect", "PIN_INCORRECT"),
        Err(e) => return echec(e),
    }

    let taux = crate::public_routes::taux_du_jour().await;
    let prix_oracle = match prix_paire(&taux.rates, &base, &quote) {
        Some(prix) => prix,
        None => return refus("Paire indisponible dans l'oracle", "PAIRE_INDISPONIBLE"),
    };
    let prix = if type_ordre == "limit" {
        let brut = nombre(&body, "price");
        if !brut.is_finite() || brut <= 0.0 {
            return refus("Prix limite invalide", "PRIX_INVALIDE");
        }
        brut
    } else {
        prix_oracle
    };
    let total = montant * prix;
    if !total.is_finite() || total <= 0.0 {
        return refus("Montant total invalide", "TOTAL_INVALIDE");
    }

    let conn = match ouvrir_index() {
        Ok(conn) => conn,
        Err(e) => return echec(e),
    };
    if lire_profil(&conn, &uq).is_none() {
        return HttpResponse::NotFound().json(json!({
            "status": "error",
            "message": "Compte introuvable"
        }));
    }

    let horodatage = maintenant();
    // Frais de service : uniquement sur les ordres exécutés au marché.
    // Un ordre limite n'est pas exécuté (aucun moteur de matching) : 0 frais.
    // Le barème suit l'actif négocié (base), pas la devise de règlement :
    // acheter 0,001 BTC se facture 1 % même si l'USD sert de support.
    let frais = if type_ordre == "market" {
        crate::frais_plateforme::arrondi_centimes(total * taux_frais_trading(&base))
    } else {
        0.0
    };
    let (devise_debite, montant_debite) = if cote == "buy" {
        (quote.clone(), total + frais)
    } else {
        (base.clone(), montant)
    };

    let tx = match conn.unchecked_transaction() {
        Ok(tx) => tx,
        Err(e) => return echec(format!("Ordre impossible ({})", e)),
    };

    match debiter(&tx, &uq, &devise_debite, montant_debite, horodatage) {
        Ok(true) => {}
        Ok(false) => {
            return refus(
                &format!(
                    "Solde {} insuffisant : {:.2} requis",
                    devise_debite, montant_debite
                ),
                "SOLDE_INSUFFISANT",
            )
        }
        Err(e) => return echec(e),
    }

    let statut = if type_ordre == "market" { "filled" } else { "open" };
    let rempli = if statut == "filled" { montant } else { 0.0 };
    let paire = nom_paire(&base, &quote);

    if let Err(e) = tx.execute(
        "INSERT INTO trading_orders
            (uq, pair, base, quote, side, order_type, price, amount, filled, total,
             status, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            uq,
            paire,
            base,
            quote,
            cote,
            type_ordre,
            prix,
            montant,
            rempli,
            total,
            statut,
            horodatage,
            horodatage
        ],
    ) {
        return echec(format!("Enregistrement de l'ordre impossible ({})", e));
    }
    let identifiant = tx.last_insert_rowid();

    if statut == "filled" {
        let (devise_creditee, montant_credite) = if cote == "buy" {
            (base.clone(), montant)
        } else {
            (quote.clone(), total - frais)
        };
        if let Err(e) = crediter(&tx, &uq, &devise_creditee, montant_credite, maintenant()) {
            return echec(e);
        }
        if let Err(e) = tx.execute(
            "INSERT INTO trading_trades (order_id, pair, uq, side, price, amount, total, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                identifiant,
                paire,
                uq,
                cote,
                prix,
                montant,
                total,
                maintenant()
            ],
        ) {
            return echec(format!("Enregistrement de l'exécution impossible ({})", e));
        }
        if frais > 0.0 {
            if let Err(e) = crate::frais_plateforme::enregistrer_tx(
                &tx,
                "market",
                &format!("order:{}", identifiant),
                &uq,
                &quote,
                frais,
                horodatage,
            ) {
                return echec(format!("Frais non enregistrés ({})", e));
            }
        }
    }

    if let Err(e) = tx.commit() {
        return echec(format!("Ordre non validé ({})", e));
    }

    if statut == "filled" {
        notifier(
            &conn,
            &uq,
            "trading",
            "Ordre exécuté",
            &format!(
                "{} {:.2} {} à {:.4} {}",
                if cote == "buy" { "Achat" } else { "Vente" },
                montant,
                base,
                prix,
                quote
            ),
            &format!("order:{}", identifiant),
        );
    } else {
        notifier(
            &conn,
            &uq,
            "trading",
            "Ordre limite placé",
            &format!(
                "{} {:.2} {} à {:.4} {} — fonds immobilisés",
                if cote == "buy" { "Achat" } else { "Vente" },
                montant,
                base,
                prix,
                quote
            ),
            &format!("order:{}", identifiant),
        );
    }

    let ordre = conn
        .query_row(
            &format!("SELECT {} FROM trading_orders WHERE id = ?1", COLONNES_ORDRE),
            params![identifiant],
            ordre_json,
        )
        .unwrap_or(Value::Null);

    let repartition = crate::frais_plateforme::repartir(frais);
    HttpResponse::Ok().json(json!({
        "status": "success",
        "execution": if type_ordre == "market" { "oracle" } else { "limit" },
        "oracle_price": prix_oracle,
        "fee": frais,
        "fee_currency": quote,
        "fee_share_earn": repartition[1],
        "fee_case_split": {
            "tresor": repartition[0],
            "frais": repartition[1],
            "recompenses": repartition[2],
            "epargne": repartition[3],
            "credit": repartition[4],
            "bouclier": repartition[5]
        },
        "order": ordre
    }))
}

// ====================
// GET /api/trading/orders/{uq}
// ====================

pub async fn trading_orders_list(req: HttpRequest, path: web::Path<String>) -> HttpResponse {
    let uq = path.into_inner();
    if let Err(response) = crate::auth_guard::exiger_soi_ou_owner(&req, &uq) {
        return response;
    }
    let conn = match ouvrir_index() {
        Ok(conn) => conn,
        Err(e) => return echec(e),
    };

    let mut ordres: Vec<Value> = Vec::new();
    let sql = format!(
        "SELECT {} FROM trading_orders WHERE UPPER(uq) = UPPER(?1) ORDER BY id DESC LIMIT 100",
        COLONNES_ORDRE
    );
    if let Ok(mut stmt) = conn.prepare(&sql) {
        if let Ok(rows) = stmt.query_map(params![uq], ordre_json) {
            ordres.extend(rows.flatten());
        }
    }
    let ouverts = ordres
        .iter()
        .filter(|ordre| ordre.get("status").and_then(|v| v.as_str()) == Some("open"))
        .count();

    HttpResponse::Ok().json(json!({
        "status": "success",
        "uq": uq,
        "open_count": ouverts,
        "orders": ordres
    }))
}

// ====================
// POST /api/trading/orders/{id}/cancel
// ====================

pub async fn trading_order_cancel(
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<Value>,
) -> HttpResponse {
    let identifiant = path.into_inner();
    let uq = texte(&body, "uq");
    if uq.is_empty() {
        return refus("Compte requis", "UQ_REQUIS");
    }
    if let Err(response) = crate::auth_guard::exiger_soi_ou_owner(&req, &uq) {
        return response;
    }
    let conn = match ouvrir_index() {
        Ok(conn) => conn,
        Err(e) => return echec(e),
    };

    let ordre = conn.query_row(
        "SELECT side, base, quote, price, amount, status, uq, pair FROM trading_orders WHERE id = ?1",
        params![identifiant],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, f64>(3)?,
                row.get::<_, f64>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
            ))
        },
    );
    let (cote, base, quote, prix, montant, statut, proprietaire, paire) = match ordre {
        Ok(valeurs) => valeurs,
        Err(_) => {
            return HttpResponse::NotFound().json(json!({
                "status": "error",
                "message": "Ordre introuvable"
            }))
        }
    };
    if !proprietaire.eq_ignore_ascii_case(&uq) {
        return HttpResponse::Forbidden().json(json!({
            "status": "error",
            "message": "Cet ordre ne t'appartient pas"
        }));
    }
    if statut != "open" {
        return refus("Cet ordre n'est plus annulable", "ORDRE_CLOS");
    }

    let horodatage = maintenant();
    let remboursement = if cote == "buy" {
        crediter(&conn, &uq, &quote, montant * prix, horodatage)
    } else {
        crediter(&conn, &uq, &base, montant, horodatage)
    };
    if let Err(e) = remboursement {
        return echec(e);
    }
    let _ = conn.execute(
        "UPDATE trading_orders SET status = 'cancelled', updated_at = ?1 WHERE id = ?2",
        params![horodatage, identifiant],
    );
    notifier(
        &conn,
        &uq,
        "trading",
        "Ordre annulé",
        &format!("{} {} — fonds restitués", paire, identifiant),
        &format!("order:{}", identifiant),
    );

    HttpResponse::Ok().json(json!({
        "status": "success",
        "message": "Ordre annulé, fonds restitués"
    }))
}

// ====================
// P2P MOBILE
// ====================

fn ouvrir_canal_p2p(
    acheteur: &str,
    vendeur: &str,
    commande: i64,
    devise: &str,
    montant: f64,
    borb: f64,
    taux: f64,
    telephone_vendeur: &str,
) -> String {
    let index = crate::uq_index::UqIndex::shared("uq_index.db");
    let nom = format!("P2P {} #{}", devise, commande);
    let description = format!(
        "Commande P2P mobile : {} BORB contre {:.2} {} (1 BORB = {:.2} {}).",
        forme_borb(borb),
        montant,
        devise,
        taux,
        devise
    );
    let groupe = match index.create_chat_group(&nom, &description, "group", "private", acheteur) {
        Ok(valeur) => valeur
            .get("group_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        Err(e) => {
            eprintln!("P2P : canal impossible ({})", e);
            return String::new();
        }
    };
    if groupe.is_empty() {
        return groupe;
    }
    if let Err(e) = index.invite_chat_group_member(&groupe, acheteur, vendeur) {
        eprintln!("P2P : invitation impossible ({})", e);
    }
    let message = format!(
        "Commande P2P #{} ouverte.\nAcheteur : {}\nVendeur : {}\nÀ payer : {:.2} {} au {}\nContrepartie : {} BORB (1 BORB = {:.2} {})\n\nÉtapes : l'acheteur paie puis annonce le paiement, le vendeur vérifie la réception puis libère les BORB.",
        commande,
        crate::uq_public_alias::get_public_display(acheteur),
        crate::uq_public_alias::get_public_display(vendeur),
        montant,
        devise,
        telephone_vendeur,
        forme_borb(borb),
        taux,
        devise
    );
    if let Err(e) = index.send_chat_group_message(&groupe, acheteur, &message) {
        eprintln!("P2P : message impossible ({})", e);
    }
    groupe
}

fn commande_json(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    let points: i64 = row.get(14)?;
    Ok(json!({
        "id": row.get::<_, i64>(0)?,
        "offer_id": row.get::<_, i64>(1)?,
        "buyer": row.get::<_, String>(2)?,
        "seller": row.get::<_, String>(3)?,
        "currency": row.get::<_, String>(4)?,
        "amount": row.get::<_, f64>(5)?,
        "usd_amount": row.get::<_, f64>(6)?,
        "rate": row.get::<_, f64>(7)?,
        "phone": row.get::<_, String>(8)?,
        "operator": row.get::<_, String>(9)?,
        "status": row.get::<_, String>(10)?,
        "chat_group_id": row.get::<_, String>(11)?,
        "created_at": row.get::<_, i64>(12)?,
        "updated_at": row.get::<_, i64>(13)?,
        "escrow_points": points,
        "proof_code": row.get::<_, String>(18).unwrap_or_default(),
        "borb_amount": points as f64 / crate::p2p_escrow::POINTS_PAR_BORB as f64,
        "rate_base": row.get::<_, String>(15)?,
        "buyer_phone": row.get::<_, String>(16)?,
        "seller_phone": row.get::<_, String>(17)?,
        "seller_display": crate::uq_public_alias::get_public_display(
            &row.get::<_, String>(3)?
        ),
    }))
}

const COLONNES_COMMANDE: &str = "id, offer_id, buyer, seller, currency, amount, usd_amount, rate,
     phone, operator, status, chat_group_id, created_at, updated_at, COALESCE(escrow_points, 0),
     (SELECT COALESCE(rate_base, '') FROM p2p_offers
       WHERE p2p_offers.id = p2p_orders.offer_id),
     COALESCE(buyer_phone, ''), COALESCE(seller_phone, ''), COALESCE(proof_code, '')";

// ====================
// GET /api/p2p/market/offers
// ====================

/// Base unique de tous les taux P2P : le BORB est l'actif échangé, la devise
/// n'est que le rail de paiement extérieur. Une offre dont `rate_base` diffère
/// vient de l'ancien moteur (taux par USD) et n'est plus prenable.
pub const BASE_TAUX_P2P: &str = "BORB";

/// Écart toléré au taux de référence avant qu'une offre soit signalée comme
/// chère ou bon marché. Un signal, jamais un blocage : le vendeur reste libre
/// de son prix.
pub const ECART_TAUX_SIGNAL_PCT: f64 = 25.0;

/// Indicateur non bloquant de l'écart au taux de référence (le taux oracle
/// pouvant manquer, l'absence de référence est explicitement signalée).
fn signal_taux(ecart: Option<f64>) -> Value {
    match ecart {
        None => json!({ "niveau": "indisponible", "sens": "inconnu",
            "message": "Référence de taux indisponible pour cette devise." }),
        Some(v) if v > ECART_TAUX_SIGNAL_PCT => json!({
            "niveau": "attention", "sens": "eleve", "ecart_pct": v,
            "message": "Ce taux est bien au-dessus du marché : les acheteurs le verront." }),
        Some(v) if v < -ECART_TAUX_SIGNAL_PCT => json!({
            "niveau": "attention", "sens": "bas", "ecart_pct": v,
            "message": "Ce taux est bien en dessous du marché : tu vendrais ton BORB trop peu cher." }),
        Some(v) => json!({ "niveau": "ok", "sens": "proche", "ecart_pct": v,
            "message": "Taux proche de la référence du jour." }),
    }
}

pub async fn p2p_offers_list(
    req: HttpRequest,
    query: web::Query<HashMap<String, String>>,
) -> HttpResponse {
    if let Err(response) = crate::auth_guard::exiger_auth(&req) {
        return response;
    }
    let conn = match ouvrir_index() {
        Ok(conn) => conn,
        Err(e) => return echec(e),
    };
    let moi = query.get("uq").cloned().unwrap_or_default();

    // Référence du taux : 1 BORB dans la devise de l'offre, composée à partir du
    // prix interne du BORB (USD) et du taux oracle 1 USD = X devise.
    let fiat = crate::public_routes::taux_du_jour().await;
    let prix_borb_usd: f64 = crate::persistence::load_prix()
        .prix_actuel
        .parse()
        .unwrap_or(0.0);

    let mut sql = String::from(
        "SELECT o.id, o.uq, COALESCE(i.uq_pseudo, ''), o.side, o.currency, o.rate,
                o.min_amount, o.max_amount, o.operator, o.city, o.phone, o.status, o.created_at,
                COALESCE(o.taken_amount, 0), o.rate_base, COALESCE(o.amount_unit, 'DEVISE')
         FROM p2p_offers o
         LEFT JOIN uq_index i ON UPPER(i.uq_number) = UPPER(o.uq)
         WHERE o.status = 'open'",
    );
    let mut filtres: Vec<String> = Vec::new();
    if let Some(devise) = query.get("currency") {
        let devise = code_devise(devise);
        if !devise.is_empty() {
            sql.push_str(" AND UPPER(o.currency) = ?");
            filtres.push(devise);
        }
    }
    if let Some(cote) = query.get("side") {
        let cote = cote.trim().to_lowercase();
        if cote == "buy" || cote == "sell" {
            sql.push_str(" AND o.side = ?");
            filtres.push(cote);
        }
    }
    sql.push_str(" ORDER BY o.id DESC LIMIT 100");

    let mut offres: Vec<Value> = Vec::new();
    let mut statement = match conn.prepare(&sql) {
        Ok(stmt) => stmt,
        Err(e) => return echec(format!("Lecture des offres impossible ({})", e)),
    };
    let rows = statement.query_map(rusqlite::params_from_iter(filtres.iter()), |row| {
        let uq_offre: String = row.get(1)?;
        let devise: String = row.get(4)?;
        let taux_offre: f64 = row.get(5)?;
        let maximum: f64 = row.get(7)?;
        let deja_pris: f64 = row.get(13)?;
        let unite: String = row.get(15)?;
        let en_borb = unite == "BORB";
        // Une offre en unités BORB expose ses bornes dans les deux monnaies :
        // BORB pour la saisie de l'acheteur, devise pour le paiement hors app.
        let (min_borb, max_borb, restant_borb) = if en_borb {
            (row.get::<_, f64>(6)?, maximum, (maximum - deja_pris).max(0.0))
        } else {
            (0.0, 0.0, 0.0)
        };
        let (minimum_devise, maximum_devise, deja_pris_devise) = if en_borb {
            (min_borb * taux_offre, max_borb * taux_offre, deja_pris * taux_offre)
        } else {
            (row.get::<_, f64>(6)?, maximum, deja_pris)
        };
        let reference = lire_taux(&fiat.rates, &devise)
            .map(|par_usd| prix_borb_usd * par_usd)
            .filter(|valeur| valeur.is_finite() && *valeur > 0.0);
        let ecart = reference
            .filter(|valeur| valeur > &0.0)
            .map(|valeur| (taux_offre - valeur) / valeur * 100.0);
        Ok(json!({
            "id": row.get::<_, i64>(0)?,
            "uq": uq_offre,
            "pseudo": row.get::<_, String>(2)?,
            "side": row.get::<_, String>(3)?,
            "currency": devise,
            "rate": taux_offre,
            "rate_base": row.get::<_, String>(14)?,
            "amount_unit": unite,
            "min_amount": minimum_devise,
            "max_amount": maximum_devise,
            "taken_amount": deja_pris_devise,
            "remaining": (maximum_devise - deja_pris_devise).max(0.0),
            "min_borb": en_borb.then_some(min_borb).unwrap_or(0.0),
            "max_borb": en_borb.then_some(max_borb).unwrap_or(0.0),
            "remaining_borb": en_borb.then_some(restant_borb).unwrap_or(0.0),
            "operator": row.get::<_, String>(8)?,
            "city": row.get::<_, String>(9)?,
            "phone": masquer_telephone(&row.get::<_, String>(10)?),
            "status": row.get::<_, String>(11)?,
            "created_at": row.get::<_, i64>(12)?,
            "mine": !moi.is_empty() && uq_offre.eq_ignore_ascii_case(&moi),
            "reference_rate": reference,
            "rate_deviation_pct": ecart,
            "rate_signal": signal_taux(ecart),
            "escrow_asset": BASE_TAUX_P2P
        }))
    });
    if let Ok(rows) = rows {
        offres.extend(rows.flatten());
    }

    HttpResponse::Ok().json(json!({
        "status": "success",
        "offers": offres
    }))
}

// ====================
// POST /api/p2p/market/offers
// ====================

pub async fn p2p_offer_create(req: HttpRequest, body: web::Json<Value>) -> HttpResponse {
    let uq = texte(&body, "uq");
    if uq.is_empty() {
        return refus("Compte requis", "UQ_REQUIS");
    }
    if let Err(response) = crate::auth_guard::exiger_soi_ou_owner(&req, &uq) {
        return response;
    }
    let conn = match ouvrir_index() {
        Ok(conn) => conn,
        Err(e) => return echec(e),
    };
    let profil = match lire_profil(&conn, &uq) {
        Some(profil) => profil,
        None => {
            return HttpResponse::NotFound().json(json!({
                "status": "error",
                "message": "Compte introuvable"
            }))
        }
    };
    let telephone = profil.phone.trim().to_string();
    if telephone.is_empty() {
        return refus(
            "Renseigne ton numéro mobile money dans ton profil avant de publier une offre P2P.",
            "MOBILE_MONEY_MANQUANT",
        );
    }

    let cote = texte(&body, "side").to_lowercase();
    if cote != "buy" && cote != "sell" {
        return refus("Sens invalide (buy ou sell)", "SENS_INVALIDE");
    }
    let devise_saisie = texte(&body, "currency");
    // Le vendeur encaisse dans sa devise de paiement ; défaut USD. Ce qui est
    // vendu est du BORB, ancré à la base du marché 1 BORB = 1 USD.
    let devise = if devise_saisie.trim().is_empty() {
        "USD".to_string()
    } else {
        code_devise(&devise_saisie)
    };
    if devise.is_empty() || devise_exclue(&devise) {
        return refus("Devise invalide", "DEVISE_INVALIDE");
    }
    let taux = crate::public_routes::taux_du_jour().await;
    if lire_taux(&taux.rates, &devise).is_none() {
        return refus("Devise inconnue de l'oracle", "DEVISE_INCONNUE");
    }
    // Le vendeur fixe son taux local : « 1 USD = X devise ». Comme la base est
    // 1 BORB = 1 USD, ce même nombre est la devise encaissée pour 1 BORB.
    // Une offre en USD est donc mécaniquement à 1 BORB = 1 USD.
    let mut cote_taux = nombre(&body, "rate");
    if devise == "USD" && (!cote_taux.is_finite() || cote_taux <= 0.0) {
        cote_taux = 1.0;
    }
    // Les bornes de vente s'expriment en BORB (l'actif immobilisé). `min_amount`
    // et `max_amount` restent acceptés : ancienne unité devise, offre DEVISE.
    let mut unite = "BORB";
    let mut minimum = nombre(&body, "min_borb");
    let mut maximum = nombre(&body, "max_borb");
    if !minimum.is_finite()
        || minimum <= 0.0
        || !maximum.is_finite()
        || maximum < minimum
    {
        unite = "DEVISE";
        minimum = nombre(&body, "min_amount");
        maximum = nombre(&body, "max_amount");
    }
    if !cote_taux.is_finite() || cote_taux <= 0.0 {
        return refus("Taux invalide", "TAUX_INVALIDE");
    }
    if !minimum.is_finite() || minimum <= 0.0 || !maximum.is_finite() || maximum < minimum {
        return refus(
            "Bornes invalides (le maximum doit dépasser le minimum)",
            "BORNES_INVALIDES",
        );
    }
    let operateur = {
        let choisi = texte(&body, "operator");
        if choisi.is_empty() {
            profil.operator.clone()
        } else {
            choisi
        }
    };
    let ville = texte(&body, "city");

    // Référence : 1 BORB dans la devise, prix interne du BORB × taux oracle.
    let prix_borb_usd: f64 = crate::persistence::load_prix()
        .prix_actuel
        .parse()
        .unwrap_or(0.0);
    let reference = lire_taux(&taux.rates, &devise)
        .map(|par_usd| prix_borb_usd * par_usd)
        .filter(|valeur| valeur.is_finite() && *valeur > 0.0);
    // Plancher P2P : jamais moins de 500 CDF (equivalent dans la devise de
    // l'offre). Au-dessus, le vendeur configure librement ; le taux live UQ
    // reste la reference affichee (reference_rate).
    let plancher = {
        let cdf = lire_taux(&taux.rates, "CDF");
        let par_devise = lire_taux(&taux.rates, &devise);
        match (cdf, par_devise) {
            (Some(cdf_par_usd), Some(devise_par_usd)) if cdf_par_usd > 0.0 => {
                // 500 CDF en USD, converti dans la devise de l'offre
                Some(500.0 / cdf_par_usd * devise_par_usd)
            }
            _ => None,
        }
    };
    if let Some(min) = plancher {
        if cote_taux < min {
            cote_taux = min;
        }
    }
    let ecart = reference.map(|valeur| (cote_taux - valeur) / valeur * 100.0);

    let insertion = conn.execute(
        "INSERT INTO p2p_offers
            (uq, side, currency, rate, rate_base, min_amount, max_amount, operator, city, phone, status, created_at, amount_unit)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'open', ?11, ?12)",
        params![
            uq,
            cote,
            devise,
            cote_taux,
            BASE_TAUX_P2P,
            minimum,
            maximum,
            operateur,
            ville,
            telephone,
            maintenant(),
            unite
        ],
    );
    if let Err(e) = insertion {
        return echec(format!("Publication de l'offre impossible ({})", e));
    }
    let identifiant = conn.last_insert_rowid();

    // Les deux unités sont renvoyées pour que la liste affiche la même offre
    // que le GET : bornes BORB pour la saisie, bornes devise pour le paiement.
    let (min_borb, max_borb) = if unite == "BORB" {
        (minimum, maximum)
    } else {
        (minimum / cote_taux, maximum / cote_taux)
    };

    HttpResponse::Ok().json(json!({
        "status": "success",
        "offer": {
            "id": identifiant,
            "uq": uq,
            "side": cote,
            "currency": devise,
            "rate": cote_taux,
            "rate_base": BASE_TAUX_P2P,
            "amount_unit": unite,
            "reference_rate": reference,
            "rate_deviation_pct": ecart,
            "rate_signal": signal_taux(ecart),
            "borb_min": min_borb,
            "borb_max": max_borb,
            "min_borb": min_borb,
            "max_borb": max_borb,
            "remaining_borb": max_borb,
            "min_amount": min_borb * cote_taux,
            "max_amount": max_borb * cote_taux,
            "taken_amount": 0.0,
            "remaining": max_borb * cote_taux,
            "operator": operateur,
            "city": ville,
            "phone": masquer_telephone(&telephone),
            "status": "open",
            "created_at": maintenant()
        }
    }))
}

// ====================
// POST /api/p2p/market/offers/{id}/take
// ====================

/// L'acheteur peut raisonner dans la devise de son choix, même différente de
/// celle du vendeur : le pont passe par l'USD (les deux devises sont cotées en
/// unités par USD par l'oracle), puis le taux de l'offre donne la quantité de
/// BORB. `None` = une des deux devises n'est pas cotée : rien n'est inventé.
fn borb_pour_saisie_en_devise(
    rates: &HashMap<String, f64>,
    montant: f64,
    devise_saisie: &str,
    devise_offre: &str,
    taux_offre: f64,
) -> Option<f64> {
    let par_saisie = lire_taux(rates, devise_saisie)?;
    let par_offre = lire_taux(rates, devise_offre)?;
    if !montant.is_finite() || montant <= 0.0 || !taux_offre.is_finite() || taux_offre <= 0.0 {
        return None;
    }
    let usd = montant / par_saisie;
    Some(usd * par_offre / taux_offre)
}

pub async fn p2p_offer_take(
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<Value>,
) -> HttpResponse {
    let offre_id = path.into_inner();
    let uq = texte(&body, "uq");
    if uq.is_empty() {
        return refus("Compte requis", "UQ_REQUIS");
    }
    if let Err(response) = crate::auth_guard::exiger_soi_ou_owner(&req, &uq) {
        return response;
    }
    // L'acheteur saisit une quantité de BORB (`borb_amount`) sur les offres
    // récentes ; `amount` reste la saisie en devise des offres héritées.
    let montant_saisi = nombre(&body, "amount");
    let borb_saisi = nombre(&body, "borb_amount");
    let pin = texte(&body, "pin");
    if pin.is_empty() {
        return refus("Code PIN requis pour prendre une offre", "PIN_REQUIS");
    }
    match verifier_pin(&uq, &pin) {
        Ok(true) => {}
        Ok(false) => return refus("Code PIN incorrect", "PIN_INCORRECT"),
        Err(e) => return echec(e),
    }

    let conn = match ouvrir_index() {
        Ok(conn) => conn,
        Err(e) => return echec(e),
    };
    let offre = conn.query_row(
        "SELECT uq, side, currency, rate, rate_base, min_amount, max_amount,
                COALESCE(taken_amount, 0), operator, phone, status,
                COALESCE(amount_unit, 'DEVISE')
         FROM p2p_offers WHERE id = ?1",
        params![offre_id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, f64>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, f64>(5)?,
                row.get::<_, f64>(6)?,
                row.get::<_, f64>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, String>(10)?,
                row.get::<_, String>(11)?,
            ))
        },
    );
    let (
        createur,
        cote,
        devise,
        cote_taux,
        base_taux,
        minimum,
        maximum,
        deja_pris,
        operateur,
        telephone_offre,
        statut,
        unite,
    ) = match offre
    {
        Ok(valeurs) => valeurs,
        Err(_) => {
            return HttpResponse::NotFound().json(json!({
                "status": "error",
                "message": "Offre introuvable"
            }))
        }
    };
    if statut != "open" {
        return refus("Cette offre n'est plus disponible", "OFFRE_FERMEE");
    }
    if base_taux != BASE_TAUX_P2P {
        return refus(
            "Cette offre affiche encore un taux par USD. Le vendeur doit la supprimer et la recréer en « 1 BORB = X devise ».",
            "OFFRE_OBSOLETE",
        );
    }
    if createur.eq_ignore_ascii_case(&uq) {
        return refus("Tu ne peux pas prendre ta propre offre", "PROPRE_OFFRE");
    }
    // La quantité se vérifie dans l'unité de l'offre : sur les offres récentes
    // l'acheteur déclare les BORB qu'il veut recevoir — ou le montant qu'il
    // peut payer dans la devise de son choix, converti au taux de l'offre —
    // et le paiement dû au vendeur se déduit du taux.
    let (points, montant) = if unite == "BORB" {
        let borb_declare = if borb_saisi.is_finite() && borb_saisi > 0.0 {
            borb_saisi
        } else if montant_saisi.is_finite() && montant_saisi > 0.0 {
            let entree = code_devise(&texte(&body, "pay_currency"));
            if entree.is_empty() {
                return refus(
                    "Saisis une quantité de BORB, ou un montant avec sa devise",
                    "MONTANT_INVALIDE",
                );
            }
            let fiat = crate::public_routes::taux_du_jour().await;
            match borb_pour_saisie_en_devise(
                &fiat.rates,
                montant_saisi,
                &entree,
                &devise,
                cote_taux,
            ) {
                Some(borb) => borb,
                None => {
                    return refus(
                        &format!(
                            "Devise {} inconnue de l'oracle : impossible de la convertir en BORB.",
                            entree
                        ),
                        "DEVISE_INCONNUE",
                    )
                }
            }
        } else {
            return refus(
                "Saisis la quantité de BORB que tu veux acheter",
                "MONTANT_INVALIDE",
            );
        };
        if borb_declare < minimum || borb_declare > maximum {
            return refus(
                &format!(
                    "La quantité doit être comprise entre {:.2} et {:.2} BORB.",
                    minimum, maximum
                ),
                "MONTANT_HORS_BORNES",
            );
        }
        let restant_borb = (maximum - deja_pris).max(0.0);
        if borb_declare > restant_borb {
            return refus(
                &format!("Il ne reste que {:.2} BORB sur cette offre.", restant_borb),
                "OFFRE_EPUISEE",
            );
        }
        let points = match crate::p2p_escrow::points_pour_borb(borb_declare) {
            Ok(points) => points,
            Err(e) => return refus(&e.message(), "TAUX_MONTANT_INVALIDES"),
        };
        let borb_arondi = points as f64 / crate::p2p_escrow::POINTS_PAR_BORB as f64;
        if borb_arondi > restant_borb {
            return refus(
                &format!("Il ne reste que {:.2} BORB sur cette offre.", restant_borb),
                "OFFRE_EPUISEE",
            );
        }
        let montant = borb_arondi * cote_taux;
        (points, montant)
    } else {
        let montant = montant_saisi;
        if !montant.is_finite() || montant <= 0.0 {
            return refus("Montant invalide", "MONTANT_INVALIDE");
        }
        if montant < minimum || montant > maximum {
            return refus(
                &format!(
                    "Le montant doit être compris entre {:.2} et {:.2} {}",
                    minimum, maximum, devise
                ),
                "MONTANT_HORS_BORNES",
            );
        }
        let restant = (maximum - deja_pris).max(0.0);
        if montant > restant {
            return refus(
                &format!("Il ne reste que {:.2} {} sur cette offre.", restant, devise),
                "OFFRE_EPUISEE",
            );
        }
        let points = match crate::p2p_escrow::points_pour_devise(montant, cote_taux) {
            Ok(points) => points,
            Err(e) => return refus(&e.message(), "TAUX_MONTANT_INVALIDES"),
        };
        (points, montant)
    };
    let borb = points as f64 / crate::p2p_escrow::POINTS_PAR_BORB as f64;

    let (vendeur, acheteur) = if cote == "sell" {
        (createur.clone(), uq.clone())
    } else {
        (uq.clone(), createur.clone())
    };

    let profil_acheteur = lire_profil(&conn, &acheteur);
    let telephone_acheteur = profil_acheteur
        .as_ref()
        .map(|profil| profil.phone.clone())
        .unwrap_or_default();
    let operateur_acheteur = profil_acheteur
        .as_ref()
        .map(|profil| profil.operator.clone())
        .unwrap_or_default();
    let kyc_acheteur = profil_acheteur
        .as_ref()
        .map(|profil| profil.kyc)
        .unwrap_or(false);

    if telephone_acheteur.trim().is_empty() {
        return refus(
            "Renseigne ton numéro mobile money dans ton profil avant de prendre une offre.",
            "MOBILE_MONEY_MANQUANT",
        );
    }

    // Consommation atomique : deux prises simultanées ne peuvent pas dépasser
    // le maximum de l'offre, même si la lecture du « restant » était faussée.
    // `taken_amount` se compte dans l'unité de l'offre : BORB ou devise.
    let consomme = if unite == "BORB" { borb } else { montant };
    let consommation = conn.execute(
        "UPDATE p2p_offers
            SET taken_amount = COALESCE(taken_amount, 0) + ?1,
                status = CASE
                    WHEN COALESCE(taken_amount, 0) + ?1 >= max_amount THEN 'filled'
                    ELSE status
                END
          WHERE id = ?2 AND status = 'open'
            AND COALESCE(taken_amount, 0) + ?1 <= max_amount",
        params![consomme, offre_id],
    );
    match consommation {
        Ok(1) => {}
        Ok(_) => {
            return refus("Cette offre n'est plus disponible", "OFFRE_FERMEE");
        }
        Err(e) => return echec(format!("Réservation de l'offre impossible ({})", e)),
    }

    let horodatage = maintenant();
    if let Err(e) = crate::p2p_escrow::geler(&vendeur, points, horodatage) {
        rendre_quantite(&conn, offre_id, borb, montant);
        return refus(&e.message(), motif_gel(&e));
    }

    let telephone_vendeur = match lire_profil(&conn, &vendeur) {
        Some(profil) if !profil.phone.trim().is_empty() => profil.phone,
        _ => telephone_offre,
    };

    let insertion = conn.execute(
        "INSERT INTO p2p_orders
            (offer_id, buyer, seller, currency, amount, usd_amount, rate, phone, operator,
             status, escrow_points, chat_group_id, created_at, updated_at,
             buyer_phone, seller_phone, proof_code)
         VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?7, ?8, 'pending_payment', ?9, '', ?10, ?10, ?11, ?12, ?13)",
        params![
            offre_id,
            acheteur,
            vendeur,
            devise,
            montant,
            cote_taux,
            telephone_acheteur,
            operateur_acheteur,
            points,
            horodatage,
            telephone_acheteur,
            telephone_vendeur,
            &format!("{:04}", (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos() as u64 % 10000) as u32)
        ],
    );
    if let Err(e) = insertion {
        if let Err(erreur_degel) = crate::p2p_escrow::degeler(&vendeur, points, maintenant()) {
            eprintln!(
                "🚨 ESCROW commande en création : gel annulable impossible ({} : {}) — à vérifier manuellement",
                vendeur,
                erreur_degel.message()
            );
        }
        rendre_quantite(&conn, offre_id, borb, montant);
        return echec(format!("Création de la commande impossible ({})", e));
    }
    let commande_id = conn.last_insert_rowid();
    // Notification vendeur : un acheteur est interesse — defi chrono lance
    notifier(&conn, &vendeur, "p2p", "Nouvelle demande d'achat",
        "Un acheteur s'interesse a ton offre — tu as 1h pour conclure le paiement.",
        &format!("p2p_order:{}", commande_id));

    let groupe = ouvrir_canal_p2p(
        &acheteur,
        &vendeur,
        commande_id,
        &devise,
        montant,
        borb,
        cote_taux,
        &telephone_vendeur,
    );
    if !groupe.is_empty() {
        let _ = conn.execute(
            "UPDATE p2p_orders SET chat_group_id = ?1 WHERE id = ?2",
            params![groupe, commande_id],
        );
    }

    let nom_vendeur = crate::uq_public_alias::get_public_display(&vendeur);
    let nom_officiel_ventes = crate::uq_public_alias::nom_enregistre(&vendeur);
    let nom_acheteur = crate::uq_public_alias::get_public_display(&acheteur);
    let borb_affiche = forme_borb(borb);

    notifier(
        &conn,
        &vendeur,
        "trading",
        "Nouvelle commande P2P",
        &format!(
            "{} BORB sont gelés pour la commande #{} : {} doit te payer {:.2} {} sur ton mobile money. Vérifie la réception avant de libérer.",
            borb_affiche, commande_id, nom_acheteur, montant, devise
        ),
        &format!("p2p:{}", commande_id),
    );
    notifier(
        &conn,
        &acheteur,
        "trading",
        "Commande P2P à payer",
        &format!(
            "Paie {:.2} {} sur le numéro {} pour recevoir {} BORB. Note le nom complet que ton opérateur affiche après le paiement : tu devras le recopier sur la preuve, la plateforme ne te le montre pas.",
            montant, devise, telephone_vendeur, borb_affiche
        ),
        &format!("p2p:{}", commande_id),
    );

    // Le test de connaissance du nom ne tient que si le vendeur a enregistré
    // le nom de son compte mobile money. Sans lui, la preuve de l'acheteur
    // reste muet sur ce signal : mieux vaut le dire ici que le cacher.
    if nom_officiel_ventes.is_none() {
        notifier(
            &conn,
            &vendeur,
            "trading",
            "Enregistre ton nom officiel",
            "L'acheteur doit recopier le nom que son opérateur affiche après t'avoir payé. Ton profil ne porte pas ce nom : ce contrôle ne pourra pas se faire sur cette commande.",
            &format!("p2p:{}", commande_id),
        );
    }

    HttpResponse::Ok().json(json!({
        "status": "success",
        "order": {
            "id": commande_id,
            "offer_id": offre_id,
            "buyer": acheteur,
            "seller": vendeur,
            "seller_display": nom_vendeur,
            "currency": devise,
            "amount": montant,
            "borb_amount": borb,
            "escrow_points": points,
            "rate": cote_taux,
            "rate_base": BASE_TAUX_P2P,
            "operator": operateur,
            "status": "pending_payment",
            "buyer_phone": telephone_acheteur.clone(),
            "seller_phone": telephone_vendeur.clone(),
            // L'acheteur qui vient de prendre l'offre a déjà sa ligne de
            // preuve : la facture n'attend pas la liste pour l'afficher.
            "numero_de_preuve": telephone_vendeur.clone(),
            "created_at": horodatage
        },
        "seller_phone": telephone_vendeur.clone(),
        "buyer_phone": telephone_acheteur.clone(),
        // Celui qui prend l'offre est l'acheteur : sa preuve devra montrer la
        // ligne du vendeur, pas la sienne.
        "numero_de_preuve": telephone_vendeur,
        "buyer_kyc_verified": kyc_acheteur,
        "instructions": "Envoie le montant au numéro mobile money du vendeur, joins la capture de ton paiement et recopie le nom complet que ton opérateur affiche après le paiement — la plateforme ne te montre jamais ce nom, c'est ce qui prouve que le bon bénéficiaire a été payé. Le moteur analyse, puis ouvre « J'ai payé ». Une fois libérés, les BORB entrent dans ton coffre : vendables en Trading et convertibles en devise virtuelle pour transférer, mais pas transférables directement."
    }))
}

/// Le code d'erreur du gel : le solde insuffisant mérite un code distinct, le
/// reste est une incohérence de la commande elle-même.
fn motif_gel(erreur: &crate::p2p_escrow::ErreurEscrow) -> &'static str {
    use crate::p2p_escrow::ErreurEscrow as E;
    match erreur {
        E::SoldeInsuffisant => "VENDEUR_SANS_BORB",
        E::MontantNonValide | E::TauxNonValide => "MONTANT_INVALIDE",
        E::GelInsuffisant | E::CompteIntrouvable => "COMPTE_INVALIDE",
        E::CadenasRequis => "CADENAS_REQUIS",
        E::Ecriture(_) => "ECRITURE_IMPOSSIBLE",
    }
}

/// Quantité de BORB telle qu'affichée aux membres : deux décimales, sauf
/// lorsque la valeur est entière.
fn forme_borb(borb: f64) -> String {
    if (borb - borb.round()).abs() < 0.005 {
        format!("{:.0}", borb)
    } else {
        format!("{:.2}", borb)
    }
}

// ====================
// GET /api/p2p/market/orders/{uq}
// ====================

pub async fn p2p_orders_list(req: HttpRequest, path: web::Path<String>) -> HttpResponse {
    let uq = path.into_inner();
    if let Err(response) = crate::auth_guard::exiger_soi_ou_owner(&req, &uq) {
        return response;
    }
    let conn = match ouvrir_index() {
        Ok(conn) => conn,
        Err(e) => return echec(e),
    };

    let mut commandes: Vec<Value> = Vec::new();
    let sql = format!(
        "SELECT {} FROM p2p_orders
         WHERE UPPER(buyer) = UPPER(?1) OR UPPER(seller) = UPPER(?1)
         ORDER BY id DESC LIMIT 100",
        COLONNES_COMMANDE
    );
    if let Ok(mut stmt) = conn.prepare(&sql) {
        if let Ok(rows) = stmt.query_map(params![uq], commande_json) {
            for commande in rows.flatten() {
                let acheteur = valeur_texte(&commande, "buyer");
                let vendeur = valeur_texte(&commande, "seller");
                let role = if acheteur.eq_ignore_ascii_case(&uq) {
                    "buyer"
                } else {
                    "seller"
                };
                let peer = if role == "buyer" {
                    vendeur.clone()
                } else {
                    acheteur.clone()
                };
                let mut valeur = commande;
                if let Some(objet) = valeur.as_object_mut() {
                    objet.insert("role".to_string(), json!(role));
                    objet.insert("peer".to_string(), json!(peer));
                }
                // La ligne que cette partie doit montrer sur sa preuve, résolue
                // côté serveur : l'acheteur paie le numéro du vendeur, le vendeur
                // reçoit depuis celui de l'acheteur. Le client n'a rien à deviner,
                // et une commande héritée retrouve le numéro du profil.
                let numero_de_partie = if role == "buyer" {
                    ligne_de_paiement(
                        &conn,
                        valeur_texte(&valeur, "seller_phone").as_str(),
                        &vendeur,
                    )
                } else {
                    // Le vendeur ne lit pas le numéro complet de son acheteur :
                    // ce qu'il recopie est confronté à la ligne enregistrée ici
                    // même, côté serveur, et non à une étiquette qu'on lui montre.
                    masquer_telephone(
                        &ligne_de_paiement(
                            &conn,
                            valeur_texte(&valeur, "buyer_phone").as_str(),
                            &acheteur,
                        )
                    )
                };
                // La ligne privée de l'acheteur ne voyage pas en clair vers le
                // vendeur : le contrôle du numéro se fait sur la valeur stockée.
                let prives_de_partie = if role != "buyer" {
                    Some((
                        masquer_telephone(&valeur_texte(&valeur, "buyer_phone")),
                        valeur_texte(&valeur, "seller_phone"),
                    ))
                } else {
                    None
                };
                // L'état des preuves voyage avec la commande : l'acheteur sait
                // s'il peut annoncer le paiement, le vendeur voit le verdict en
                // face avant de libérer les BORB.
                let identifiant = valeur.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
                let mienne = crate::p2p_preuve::derniere_preuve_de(&conn, identifiant, &uq)
                    .unwrap_or(None)
                    .unwrap_or(Value::Null);
                let adverse_uq = if role == "buyer" { vendeur } else { acheteur };
                let adverse = crate::p2p_preuve::derniere_preuve_de(&conn, identifiant, &adverse_uq)
                    .unwrap_or(None)
                    .unwrap_or(Value::Null);
                if let Some(objet) = valeur.as_object_mut() {
                    if let Some((masque, sienne)) = prives_de_partie {
                        objet.insert("buyer_phone".to_string(), json!(masque));
                        objet.insert("seller_phone".to_string(), json!(sienne));
                    }
                    objet.insert("numero_de_preuve".to_string(), json!(numero_de_partie));
                    objet.insert("my_proof".to_string(), mienne);
                    objet.insert("peer_proof".to_string(), adverse);
                }
                commandes.push(valeur);
            }
        }
    }

    HttpResponse::Ok().json(json!({
        "status": "success",
        "uq": uq,
        "orders": commandes
    }))
}

fn valeur_texte(valeur: &Value, cle: &str) -> String {
    valeur
        .get(cle)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

// ====================
// TRANSITIONS P2P
// ====================

struct CommandeP2P {
    acheteur: String,
    vendeur: String,
    devise: String,
    montant: f64,
    statut: String,
    groupe: String,
    offre_id: i64,
    points: i64,
    /// Numéro mobile money déclaré par l'acheteur à la prise d'offre. C'est la
    /// ligne d'où part le paiement : ce que la preuve du vendeur doit montrer.
    telephone_acheteur: String,
    /// Numéro que l'acheteur doit payer : la ligne mobile money du vendeur.
    telephone_vendeur: String,
    cree_le: i64,
    /// Code 4 chiffres de cette commande : l'acheteur l'ecrit sur sa preuve papier.
    proof_code: String,
}

fn charger_commande(conn: &Connection, identifiant: i64) -> Result<CommandeP2P, String> {
    conn.query_row(
        "SELECT buyer, seller, currency, amount, status, chat_group_id,
                offer_id, COALESCE(escrow_points, 0), COALESCE(phone, ''), created_at,
                COALESCE(buyer_phone, ''), COALESCE(seller_phone, ''), COALESCE(proof_code, '')
         FROM p2p_orders WHERE id = ?1",
        params![identifiant],
        |row| {
            Ok(CommandeP2P {
                acheteur: row.get(0)?,
                vendeur: row.get(1)?,
                devise: row.get(2)?,
                montant: row.get(3)?,
                statut: row.get(4)?,
                groupe: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
                offre_id: row.get(6)?,
                points: row.get(7)?,
                telephone_acheteur: premier_non_vide(row.get(10)?, row.get(8)?),
                telephone_vendeur: row.get(11)?,
                cree_le: row.get(9)?,
                proof_code: row.get(12)?,
            })
        },
    )
    .map_err(|_| "Commande introuvable".to_string())
}

/// La ligne que la preuve doit montrer. Pour l'acheteur c'est le numéro du
/// vendeur — celui qu'il a composé pour payer ; pour le vendeur c'est le numéro
/// d'où le paiement est parti. Une commande héritée, écrite avant ces colonnes,
/// relit le profil du compte destinataire plutôt que de laisser le signal muet.
fn numero_attendu(conn: &Connection, commande: &CommandeP2P, role: &str) -> String {
    if role == "buyer" {
        ligne_de_paiement(conn, &commande.telephone_vendeur, &commande.vendeur)
    } else {
        ligne_de_paiement(conn, &commande.telephone_acheteur, &commande.acheteur)
    }
}

/// Une ligne mobile money porteuse : la valeur de la commande, sinon celle du
/// profil du compte concerne. Renvoie une chaine vide quand rien ne permet de
/// comparer, ce que le moteur lit comme un signal muet et non comme un tort.
fn ligne_de_paiement(conn: &Connection, stocke: &str, compte: &str) -> String {
    if !stocke.trim().is_empty() {
        return stocke.trim().to_string();
    }
    lire_profil(conn, compte)
        .map(|profil| profil.phone)
        .unwrap_or_default()
}

/// Le premier des deux qui porte quelque chose : les lignes anciennes n'ont
/// rempli que `phone`.
fn premier_non_vide(riche: String, herdite: String) -> String {
    if riche.trim().is_empty() {
        herdite
    } else {
        riche
    }
}

fn message_canal(groupe: &str, expediteur: &str, texte: &str) {
    if groupe.is_empty() {
        return;
    }
    let index = crate::uq_index::UqIndex::shared("uq_index.db");
    if let Err(e) = index.send_chat_group_message(groupe, expediteur, texte) {
        eprintln!("P2P : message de suivi impossible ({})", e);
    }
}

/// Transition d'état conditionnelle : la commande ne bouge que si elle est
/// encore dans un des états attendus. Deux libérations simultanées ne peuvent
/// donc pas toutes deux réussir.
fn passer_statut(conn: &Connection, identifiant: i64, attendus: &[&str], vers: &str) -> bool {
    let marques = attendus.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "UPDATE p2p_orders SET status = ?1, updated_at = ?2 WHERE id = ?3 AND status IN ({})",
        marques
    );
    let horodatage = maintenant();
    let mut valeurs: Vec<&dyn rusqlite::ToSql> = vec![&vers, &horodatage, &identifiant];
    for attendu in attendus {
        valeurs.push(attendu);
    }
    matches!(conn.execute(&sql, rusqlite::params_from_iter(valeurs.iter())), Ok(1))
}

/// Remise en vente de la quantité d'une offre dont la commande saute. La
/// quantité consommée était exprimée dans l'unité de l'offre : BORB pour les
/// offres récentes, devise pour les héritées — le remboursement suit.
fn rendre_quantite(conn: &Connection, offre_id: i64, borb: f64, montant_devise: f64) {
    let _ = conn.execute(
        "UPDATE p2p_offers
            SET taken_amount = CASE
                    WHEN COALESCE(taken_amount, 0) >= (CASE WHEN COALESCE(amount_unit, 'DEVISE') = 'BORB' THEN ?1 ELSE ?2 END)
                        THEN taken_amount - (CASE WHEN COALESCE(amount_unit, 'DEVISE') = 'BORB' THEN ?1 ELSE ?2 END)
                    ELSE 0
                END,
                status = CASE WHEN status = 'filled' THEN 'open' ELSE status END
          WHERE id = ?3",
        params![borb, montant_devise, offre_id],
    );
}

/// Le moteur ne parle pas à la place des parties : il mesure, juge, et rend
/// ses raisons. L'image et les valeurs recopiées viennent de l'une des deux
/// parties ; `role` est déduit de la commande, jamais confié au client.
pub async fn p2p_proof_attach(
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<Value>,
) -> HttpResponse {
    let identifiant = path.into_inner();
    let uq = texte(&body, "uq");
    if let Err(response) = crate::auth_guard::exiger_soi_ou_owner(&req, &uq) {
        return response;
    }
    let conn = match ouvrir_index() {
        Ok(conn) => conn,
        Err(e) => return echec(e),
    };
    let commande = match charger_commande(&conn, identifiant) {
        Ok(commande) => commande,
        Err(e) => return refus(&e, "COMMANDE_INTROUVABLE"),
    };
    let role = if commande.acheteur.eq_ignore_ascii_case(&uq) {
        "buyer"
    } else if commande.vendeur.eq_ignore_ascii_case(&uq) {
        "seller"
    } else {
        return HttpResponse::Forbidden().json(json!({
            "status": "error",
            "message": "Tu ne participes pas à cette commande"
        }));
    };
    if commande.statut != "pending_payment" && commande.statut != "paid" {
        return refus("Cette commande n'accepte plus de preuve", "STATUT_INVALIDE");
    }

    let octets = match crate::p2p_preuve::decoder_image(&texte(&body, "image")) {
        Ok(octets) => octets,
        Err(e) => return refus(&e, "IMAGE_INVALIDE"),
    };
    let face = if role == "buyer" {
        &commande.vendeur
    } else {
        &commande.acheteur
    };
    // Le nom que la capture doit montrer est le nom officiel du compte
    // destinataire : celui que l'opérateur affiche après le paiement, et que la
    // plateforme ne montre à personne. Le recopier correctement se mérite, c'est
    // le test de connaissance. Faute de nom enregistré, le moteur comparerait
    // une étiquette publique et se déclarerait invérifiable.
    let nom_officiel = crate::uq_public_alias::nom_enregistre(face);
    let attendu = crate::p2p_preuve::Attendu {
        montant: commande.montant,
        devise: commande.devise.clone(),
        numero: numero_attendu(&conn, &commande, role),
        nom: nom_officiel
            .clone()
            .unwrap_or_else(|| crate::uq_public_alias::get_public_display(face)),
        nom_officiel: nom_officiel.is_some(),
        cree_le: commande.cree_le,
    };
    let saisi = crate::p2p_preuve::Saisie {
        montant: nombre(&body, "montant"),
        devise: code_devise(&texte(&body, "devise")),
        numero: texte(&body, "numero"),
        nom: texte(&body, "nom"),
    };
    if !(saisi.montant.is_finite() && saisi.montant > 0.0) {
        return refus(
            "Recopie le montant visible sur ta capture, en chiffres.",
            "MONTANT_INVALIDE",
        );
    }
    // Recopier est une déclaration, pas une case pré-remplie : ce qui n'est pas
    // écrit ne se prouve pas, et ce qui est écrit faux se conteste.
    if role == "buyer" && saisi.numero.trim().is_empty() {
        return refus(
            "Recopie le numéro affiché sur ton reçu, tel que ton opérateur l'écrit.",
            "NUMERO_REQUIS",
        );
    }
    // Sans nom recopié, le test de connaissance ne peut pas avoir lieu : la
    // preuve de l'acheteur est refusée avant analyse. Celle du vendeur reste un
    // contrôle d'à-côté, elle n'impose rien à qui n'a rien à prouver.
    if role == "buyer" && saisi.nom.trim().is_empty() {
        return refus(
            "Recopie le nom complet que ton opérateur affiche après le paiement : c'est lui qui prouve que tu as payé le bon bénéficiaire.",
            "NOM_REQUIS",
        );
    }

    let analyse = crate::p2p_preuve::analyser(
        &octets,
        &saisi,
        &attendu,
        &crate::p2p_preuve::charger_poids(&conn),
    );
    let enregistree = match crate::p2p_preuve::inserer_preuve(
        &conn,
        identifiant,
        role,
        &uq,
        &saisi,
        &octets,
        &analyse,
    ) {
        Ok(preuve) => preuve,
        Err(e) => return echec(e),
    };

    HttpResponse::Ok().json(json!({
        "status": "success",
        "preuve_id": enregistree.identifiant,
        "role": role,
        "analyse": analyse.as_json(),
        "attendu": json!({
            "montant": attendu.montant,
            "devise": attendu.devise,
            "numero": attendu.numero,
            // Le nom servi ici reste l'étiquette publique : le nom officiel ne
            // sort pas du serveur, sinon le test de connaissance n'aurait plus
            // d'objet. `nom_officiel` dit au client si la comparaison a pu avoir
            // lieu, et `analyse.nom_verifie` donne son verdict.
            "nom": crate::uq_public_alias::get_public_display(face),
            "nom_officiel": attendu.nom_officiel,
        }),
    }))
}

/// Les deux preuves d'une commande, images comprises : chacune doit pouvoir
/// montrer la capture de l'autre. La garde est faite sur les deux parties.
pub async fn p2p_proof_list(
    req: HttpRequest,
    path: web::Path<i64>,
    query: web::Query<HashMap<String, String>>,
) -> HttpResponse {
    let identifiant = path.into_inner();
    let uq = query.get("uq").cloned().unwrap_or_default();
    if uq.is_empty() {
        return refus("Identifiant UQ requis", "UQ_REQUIS");
    }
    if let Err(response) = crate::auth_guard::exiger_soi_ou_owner(&req, &uq) {
        return response;
    }
    let conn = match ouvrir_index() {
        Ok(conn) => conn,
        Err(e) => return echec(e),
    };
    let commande = match charger_commande(&conn, identifiant) {
        Ok(commande) => commande,
        Err(e) => return refus(&e, "COMMANDE_INTROUVABLE"),
    };
    if !commande.acheteur.eq_ignore_ascii_case(&uq) && !commande.vendeur.eq_ignore_ascii_case(&uq) {
        return HttpResponse::Forbidden().json(json!({
            "status": "error",
            "message": "Tu ne participes pas à cette commande"
        }));
    }
    let preuves = match crate::p2p_preuve::lister_preuves(&conn, identifiant) {
        Ok(preuves) => preuves,
        Err(e) => return echec(e),
    };
    HttpResponse::Ok().json(json!({ "status": "success", "preuves": preuves }))
}

/// L'image d'une preuve, ouverte aux deux parties de la commande : la capture
/// de l'un se contrôle chez l'autre. Les métadonnées restent dans la base,
/// seules ces octets circulent.
pub async fn p2p_proof_image(
    req: HttpRequest,
    path: web::Path<i64>,
    query: web::Query<HashMap<String, String>>,
) -> HttpResponse {
    let identifiant = path.into_inner();
    let uq = query.get("uq").cloned().unwrap_or_default();
    if uq.is_empty() {
        return refus("Identifiant UQ requis", "UQ_REQUIS");
    }
    if let Err(response) = crate::auth_guard::exiger_soi_ou_owner(&req, &uq) {
        return response;
    }
    let conn = match ouvrir_index() {
        Ok(conn) => conn,
        Err(e) => return echec(e),
    };
    let preuve = match crate::p2p_preuve::preuve_de(&conn, identifiant) {
        Ok(Some(preuve)) => preuve,
        Ok(None) => return refus("Aucune preuve sous cet identifiant", "PREUVE_INTROUVABLE"),
        Err(e) => return echec(e),
    };
    let commande_id = preuve.get("order_id").and_then(|v| v.as_i64()).unwrap_or(0);
    let commande = match charger_commande(&conn, commande_id) {
        Ok(commande) => commande,
        Err(e) => return refus(&e, "COMMANDE_INTROUVABLE"),
    };
    if !commande.acheteur.eq_ignore_ascii_case(&uq) && !commande.vendeur.eq_ignore_ascii_case(&uq) {
        return HttpResponse::Forbidden().json(json!({
            "status": "error",
            "message": "Tu ne participes pas à cette commande"
        }));
    }
    let image = match crate::p2p_preuve::image_de(&conn, identifiant) {
        Ok(Some(image)) => image,
        Ok(None) => return refus("Aucune preuve sous cet identifiant", "PREUVE_INTROUVABLE"),
        Err(e) => return echec(e),
    };
    HttpResponse::Ok().json(json!({
        "status": "success",
        "preuve_id": identifiant,
        "order_id": commande_id,
        "role": preuve.get("role").cloned().unwrap_or(Value::Null),
        "verdict": preuve.get("verdict").cloned().unwrap_or(Value::Null),
        "image": image,
    }))
}

pub async fn p2p_order_confirm(
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<Value>,
) -> HttpResponse {
    let identifiant = path.into_inner();
    let uq = texte(&body, "uq");
    if let Err(response) = crate::auth_guard::exiger_soi_ou_owner(&req, &uq) {
        return response;
    }
    let conn = match ouvrir_index() {
        Ok(conn) => conn,
        Err(e) => return echec(e),
    };
    let commande = match charger_commande(&conn, identifiant) {
        Ok(commande) => commande,
        Err(e) => return refus(&e, "COMMANDE_INTROUVABLE"),
    };
    if !commande.acheteur.eq_ignore_ascii_case(&uq) {
        return HttpResponse::Forbidden().json(json!({
            "status": "error",
            "message": "Seul l'acheteur peut annoncer le paiement"
        }));
    }
    if commande.statut != "pending_payment" {
        return refus("Cette commande n'attend pas de paiement", "STATUT_INVALIDE");
    }

    // Code de preuve : l'acheteur doit saisir le code 4 chiffres de CETTE
    // commande (celui qu'il devait ecrire sur sa preuve papier). Sans lui,
    // n'importe quelle vieille capture pourrait servir.
    let code_saisi = texte(&body, "proof_code").trim().to_string();
    if !commande.proof_code.is_empty() {
        if code_saisi.is_empty() {
            return refus(
                "Saisis le code a 4 chiffres affiche pour cette commande — tu aurais du l'ecrire sur ta preuve papier.",
                "CODE_REQUIS",
            );
        }
        if code_saisi != commande.proof_code {
            return refus(
                "Code incorrect : ce n'est pas le code de cette transaction. Verifie celui affiche dans ta commande.",
                "CODE_INCORRECT",
            );
        }
    }

    // « J'ai payé » ne s'ouvre qu'avec une preuve analysée. Le moteur tranche,
    // l'humain garde le dernier mot sur un doute ou un écart, et cette
    // dérogation est écrite noir sur blanc pour le vendeur.
    let preuve = match crate::p2p_preuve::derniere_preuve_de(&conn, identifiant, &commande.acheteur) {
        Ok(preuve) => preuve,
        Err(e) => return echec(e),
    };
    let preuve = match preuve {
        Some(preuve) => preuve,
        None => return refus(
            "Ajoute une capture de ton paiement mobile money avant d'annoncer que tu as payé.",
            "PREUVE_REQUISE",
        ),
    };
    let verdict = preuve
        .get("verdict")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let conformite = preuve.get("conformite").and_then(|v| v.as_f64()).unwrap_or(0.0);
    // Le moteur écarte déjà une divergence de nom, mais la raison doit être
    // dite en clair : corriger une capture n'a rien à voir avec corriger un
    // nom mal recopié.
    let nom_divergent = preuve.get("nom_ok").and_then(|v| v.as_i64()).unwrap_or(0) == -1;
    if verdict == "rejet" {
        let derogue = body
            .get("derogation")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !derogue {
            if nom_divergent {
                return refus(
                    "Le nom que tu as recopié n'est pas le nom enregistré de ce vendeur. Si ton paiement est bien parti, recopie le nom tel que ton opérateur l'affiche ; sinon reprends la capture.",
                    "NOM_DIVERGENT",
                );
            }
            return refus(
                "La preuve a été écartée par l'analyse. Corrige la capture, ou assume le doute en cochant la déclaration.",
                "PREUVE_ECARTEE",
            );
        }
        if let Err(e) = crate::p2p_preuve::marquer_derogue(&conn, identifiant, &commande.acheteur) {
            return echec(e);
        }
    }

    if !passer_statut(&conn, identifiant, &["pending_payment"], "paid") {
        return refus("Cette commande n'attend plus de paiement", "STATUT_INVALIDE");
    }
    let nom_acheteur = crate::uq_public_alias::get_public_display(&commande.acheteur);
    let signal = match verdict.as_str() {
        "valide" => "Analyse : preuve conforme.".to_string(),
        "doute" => format!("Analyse : doute ({} % de conformité).", (conformite * 100.0).round() as i64),
        _ if nom_divergent => {
            "Analyse : le nom recopié par l'acheteur ne correspond pas au nom enregistré de ton compte, et il a assumé cette divergence.".to_string()
        }
        _ => "Analyse : preuve écartée, l'acheteur a assumé le doute.".to_string(),
    };
    notifier(
        &conn,
        &commande.vendeur,
        "trading",
        "Paiement annoncé",
        &format!(
            "{} annonce un paiement de {:.2} {} pour la commande #{} — {}. Vérifie ton mobile money puis libère les BORB.",
            nom_acheteur, commande.montant, commande.devise, identifiant, signal
        ),
        &format!("p2p:{}", identifiant),
    );
    message_canal(
        &commande.groupe,
        &commande.acheteur,
        &format!("Paiement mobile money envoyé, preuve jointe. {}", signal),
    );

    HttpResponse::Ok().json(json!({
        "status": "success",
        "message": "Paiement annoncé au vendeur",
        "preuve_id": preuve.get("id").and_then(|v| v.as_i64()),
        "verdict": verdict,
        "conformite": conformite,
        "derogue": verdict == "rejet"
    }))
}

pub async fn p2p_order_complete(
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<Value>,
) -> HttpResponse {
    let identifiant = path.into_inner();
    let uq = texte(&body, "uq");
    if let Err(response) = crate::auth_guard::exiger_soi_ou_owner(&req, &uq) {
        return response;
    }
    let conn = match ouvrir_index() {
        Ok(conn) => conn,
        Err(e) => return echec(e),
    };
    let commande = match charger_commande(&conn, identifiant) {
        Ok(commande) => commande,
        Err(e) => return refus(&e, "COMMANDE_INTROUVABLE"),
    };
    if !commande.vendeur.eq_ignore_ascii_case(&uq) {
        return HttpResponse::Forbidden().json(json!({
            "status": "error",
            "message": "Seul le vendeur peut libérer les fonds"
        }));
    }
    if commande.statut != "paid" {
        return refus(
            "Le paiement n'a pas encore été annoncé par l'acheteur",
            "PAIEMENT_NON_ANONCE",
        );
    }

    if commande.points == 0 {
        return refus(
            "Cette commande est antérieure au gel BORB : rien n'est immobilisé côté vendeur. Annule-la et ouvre une nouvelle commande.",
            "COMMANDE_OBSOLETE",
        );
    }
    if !passer_statut(&conn, identifiant, &["paid"], "completed") {
        return refus("Cette commande n'attend plus de libération", "STATUT_INVALIDE");
    }

    if let Err(e) = crate::p2p_escrow::liberer(
        &commande.vendeur,
        &commande.acheteur,
        commande.points,
        identifiant,
        maintenant(),
    ) {
        passer_statut(&conn, identifiant, &["completed"], "paid");
        return refus(
            &format!(
                "Libération impossible ({}). La commande reste payable.",
                e.message()
            ),
            "LIBERATION_IMPOSSIBLE",
        );
    }

    let borb = commande.points as f64 / crate::p2p_escrow::POINTS_PAR_BORB as f64;
    let borb_affiche = forme_borb(borb);
    let nom_vendeur = crate::uq_public_alias::get_public_display(&commande.vendeur);
    notifier(
        &conn,
        &commande.acheteur,
        "trading",
        "Commande P2P terminée",
        &format!(
            "{} BORB sont crédités dans ton coffre par {}. Vendables en Trading, convertibles en devise virtuelle pour transférer : les BORB ne se transfèrent pas directement.",
            borb_affiche, nom_vendeur
        ),
        &format!("p2p:{}", identifiant),
    );
    notifier(
        &conn,
        &commande.vendeur,
        "trading",
        "Commande P2P terminée",
        &format!(
            "{} BORB transférés à l'acheteur pour {:.2} {}.",
            borb_affiche, commande.montant, commande.devise
        ),
        &format!("p2p:{}", identifiant),
    );
    message_canal(
        &commande.groupe,
        &commande.vendeur,
        &format!("{} BORB libérés : la commande est terminée.", borb_affiche),
    );

    // Le vendeur a confirmé la réception : les preuves de cette commande
    // deviennent un cas « réussi » pour le moteur, qui recalcule ses poids.
    let _ = crate::p2p_preuve::marquer_issue(&conn, identifiant, "completed");
    let modele = crate::p2p_preuve::recalibrer(&conn)
        .unwrap_or_else(|_| serde_json::json!({ "erreur": "apprentissage indisponible" }));

    HttpResponse::Ok().json(json!({
        "status": "success",
        "message": "Commande terminée, BORB transférés",
        "borb_amount": borb,
        "escrow_points": commande.points,
        "apprentissage": modele
    }))
}

pub async fn p2p_order_cancel(
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<Value>,
) -> HttpResponse {
    let identifiant = path.into_inner();
    let uq = texte(&body, "uq");
    if let Err(response) = crate::auth_guard::exiger_soi_ou_owner(&req, &uq) {
        return response;
    }
    let conn = match ouvrir_index() {
        Ok(conn) => conn,
        Err(e) => return echec(e),
    };
    let commande = match charger_commande(&conn, identifiant) {
        Ok(commande) => commande,
        Err(e) => return refus(&e, "COMMANDE_INTROUVABLE"),
    };
    let est_acheteur = commande.acheteur.eq_ignore_ascii_case(&uq);
    let est_vendeur = commande.vendeur.eq_ignore_ascii_case(&uq);
    if !est_acheteur && !est_vendeur {
        return HttpResponse::Forbidden().json(json!({
            "status": "error",
            "message": "Tu ne participes pas à cette commande"
        }));
    }
    if commande.statut != "pending_payment" && commande.statut != "paid" {
        return refus("Cette commande n'est plus annulable", "STATUT_INVALIDE");
    }
    let litige = body
        .get("litige")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    if !passer_statut(
        &conn,
        identifiant,
        &["pending_payment", "paid"],
        "cancelled",
    ) {
        return refus("Cette commande n'est plus annulable", "STATUT_INVALIDE");
    }

    if commande.points > 0 {
        if let Err(e) = crate::p2p_escrow::degeler(&commande.vendeur, commande.points, maintenant())
        {
            passer_statut(&conn, identifiant, &["cancelled"], commande.statut.as_str());
            return refus(
                &format!("Dégel impossible ({}).", e.message()),
                "LIBERATION_IMPOSSIBLE",
            );
        }
    } else {
        // Commande ouverte par l'ancienne version : la devise avait été débitée
        // du coffre, rien n'était gelé en BORB. On restitue ce qui l'avait été.
        if let Err(e) = crediter(
            &conn,
            &commande.vendeur,
            &commande.devise,
            commande.montant,
            maintenant(),
        ) {
            passer_statut(&conn, identifiant, &["cancelled"], commande.statut.as_str());
            return echec(e);
        }
    }
    rendre_quantite(
        &conn,
        commande.offre_id,
        commande.points as f64 / crate::p2p_escrow::POINTS_PAR_BORB as f64,
        commande.montant,
    );

    let autre = if est_acheteur {
        commande.vendeur.clone()
    } else {
        commande.acheteur.clone()
    };
    let detail = if commande.points > 0 {
        format!(
            "{} BORB de {} sont débloqués",
            forme_borb(commande.points as f64 / crate::p2p_escrow::POINTS_PAR_BORB as f64),
            crate::uq_public_alias::get_public_display(&commande.vendeur)
        )
    } else {
        format!("{:.2} {} restitués", commande.montant, commande.devise)
    };
    notifier(
        &conn,
        &autre,
        "trading",
        "Commande P2P annulée",
        &format!("{}, la commande #{} est annulée.", detail, identifiant),
        &format!("p2p:{}", identifiant),
    );
    message_canal(&commande.groupe, &uq, &format!("Commande annulée, {}.", detail));

    // Une annulation signalée comme litige est un cas d'école : le moteur
    // apprend que les preuves de cette commande n'ont pas convaincu.
    let _ = crate::p2p_preuve::marquer_issue(
        &conn,
        identifiant,
        if litige { "litige" } else { "cancelled" },
    );
    let modele = if litige {
        crate::p2p_preuve::recalibrer(&conn)
            .unwrap_or_else(|_| serde_json::json!({ "erreur": "apprentissage indisponible" }))
    } else {
        serde_json::Value::Null
    };

    HttpResponse::Ok().json(json!({
        "status": "success",
        "message": "Commande annulée, fonds débloqués",
        "borb_amount": if commande.points > 0 { Some(commande.points as f64 / crate::p2p_escrow::POINTS_PAR_BORB as f64) } else { None },
        "apprentissage": modele
    }))
}

// ====================
// GET /api/notifications/screen/{uq}
// ====================

pub async fn screen_notifications(
    req: HttpRequest,
    path: web::Path<String>,
) -> HttpResponse {
    let uq = path.into_inner();
    if let Err(response) = crate::auth_guard::exiger_soi_ou_owner(&req, &uq) {
        return response;
    }
    let conn = match ouvrir_index() {
        Ok(conn) => conn,
        Err(e) => return echec(e),
    };

    let mut notifications: Vec<Value> = Vec::new();

    // --- signaux dérivés d'un état réel ---
    // Une alerte d'état s'efface quand elle est acquittée ; elle revient si la
    // situation change (référence différente), jamais par simple relecture.
    let mut acquittees: Vec<String> = Vec::new();
    if let Ok(mut stmt) = conn.prepare("SELECT reference FROM screen_ack WHERE UPPER(uq) = UPPER(?1)") {
        if let Ok(rows) = stmt.query_map(params![uq], |row| row.get::<_, String>(0)) {
            acquittees.extend(rows.flatten());
        }
    }
    let deja_vue = |reference: &str| acquittees.iter().any(|valeur| valeur == reference);

    // Un signal d'état décrit la situation AU MOMENT de l'appel : il porte donc
    // l'horodatage de l'appel, ce qui le place en tête du flux horodaté.
    let horodatage_live = maintenant();

    if let Some(profil) = lire_profil(&conn, &uq) {
        if !profil.kyc {
            notifications.push(json!({
                "id": 0,
                "scope": "kyc",
                "title": "Vérifie ton identité",
                "body": "Ton KYC n'est pas encore validé. Certaines fonctions restent limitées.",
                "reference": "live:kyc",
                "read": deja_vue("live:kyc"),
                "created_at": horodatage_live
            }));
        } else {
            notifications.push(json!({
                "id": 0,
                "scope": "kyc",
                "title": "Vérification réussie",
                "body": "Ton identité est validée. Toutes les fonctions sont ouvertes.",
                "reference": "live:kyc-ok",
                "read": deja_vue("live:kyc-ok"),
                "created_at": horodatage_live
            }));
        }
        if profil.phone.trim().is_empty() {
            notifications.push(json!({
                "id": 0,
                "scope": "trading",
                "title": "Numéro mobile money manquant",
                "body": "Ajoute ton numéro dans ton profil pour utiliser le P2P mobile.",
                "reference": "live:phone",
                "read": deja_vue("live:phone"),
                "created_at": horodatage_live
            }));
        }
    }

    if !pin_configure(&uq) {
        notifications.push(json!({
            "id": 0,
            "scope": "coffre",
            "title": "Protège ton coffre",
            "body": "Aucun code PIN n'est configuré : tes envois et tes ordres ne sont pas protégés.",
            "reference": "live:pin",
            "read": deja_vue("live:pin"),
            "created_at": horodatage_live
        }));
    }

    let ordres_ouverts: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM trading_orders WHERE UPPER(uq) = UPPER(?1) AND status = 'open'",
            params![uq],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if ordres_ouverts > 0 {
        notifications.push(json!({
            "id": 0,
            "scope": "coffre",
            "title": "Fonds immobilisés",
            "body": format!("{} ordre(s) en cours immobilisent des devises.", ordres_ouverts),
            "reference": "live:locked",
            "read": deja_vue("live:locked"),
            "created_at": horodatage_live
        }));
    }

    // Messages directs : la vraie boîte de réception porte l'état « non lu ».
    let conversations = crate::database::Database::shared("uq_platform_db.json").get_chat_inbox(&uq);
    let messages_non_lus = conversations
        .iter()
        .filter(|conversation| {
            conversation
                .get("unread")
                .and_then(|valeur| valeur.as_bool())
                .unwrap_or(false)
        })
        .count();
    if messages_non_lus > 0 {
        notifications.push(json!({
            "id": 0,
            "scope": "chat",
            "title": "Messages non lus",
            "body": format!("{} conversation(s) t'attendent dans le chat UQ.", messages_non_lus),
            "reference": "live:chat",
            "read": deja_vue("live:chat"),
            "created_at": horodatage_live
        }));
    }

    // --- notifications stockées (trading, P2P, Zando…) ---
    if let Ok(mut stmt) = conn.prepare(
        "SELECT id, scope, title, body, COALESCE(reference, ''), COALESCE(read, 0), created_at
         FROM screen_notifications
         WHERE UPPER(uq) = UPPER(?1)
         ORDER BY created_at DESC, id DESC LIMIT 100",
    ) {
        if let Ok(rows) = stmt.query_map(params![uq], |row| {
            Ok(json!({
                "id": row.get::<_, i64>(0)?,
                "scope": row.get::<_, String>(1)?,
                "title": row.get::<_, String>(2)?,
                "body": row.get::<_, String>(3)?,
                "reference": row.get::<_, String>(4)?,
                "read": row.get::<_, i64>(5)? == 1,
                "created_at": row.get::<_, i64>(6)?
            }))
        }) {
            notifications.extend(rows.flatten());
        }
    }

    // --- ce qui se déroule maintenant ---
    // La tête épinglée se décide sur l'état réel des objets, pas sur le texte
    // d'une notification écrite au moment où l'événement est survenu.
    let actives = references_en_cours(&conn, &uq);
    for notification in notifications.iter_mut() {
        let reference = notification
            .get("reference")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let en_cours = if reference == "live:locked" {
            ordres_ouverts > 0
        } else {
            actives.contains(&reference)
        };
        if let Some(objet) = notification.as_object_mut() {
            objet.insert("en_cours".to_string(), json!(en_cours));
        }
    }

    trier_notifications(&mut notifications);

    let mut compteurs = serde_json::Map::new();
    for scope in ["kyc", "coffre", "zando", "trading", "chat"] {
        compteurs.insert(scope.to_string(), json!(0));
    }
    let mut total = 0i64;
    let mut tete = 0i64;
    for notification in &notifications {
        if notification
            .get("en_cours")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            tete += 1;
        }
        let lu = notification.get("read").and_then(|v| v.as_bool()).unwrap_or(false);
        if lu {
            continue;
        }
        total += 1;
        if let Some(scope) = notification.get("scope").and_then(|v| v.as_str()) {
            let courant = compteurs.get(scope).and_then(|v| v.as_i64()).unwrap_or(0);
            compteurs.insert(scope.to_string(), json!(courant + 1));
        }
    }

    HttpResponse::Ok().json(json!({
        "status": "success",
        "uq": uq,
        "total_unread": total,
        "total_en_cours": tete,
        "counts": compteurs,
        "notifications": notifications
    }))
}

// ====================
// POST /api/notifications/screen/read
// ====================

pub async fn screen_notifications_read(req: HttpRequest, body: web::Json<Value>) -> HttpResponse {
    let uq = texte(&body, "uq");
    if uq.is_empty() {
        return refus("Compte requis", "UQ_REQUIS");
    }
    if let Err(response) = crate::auth_guard::exiger_soi_ou_owner(&req, &uq) {
        return response;
    }
    let conn = match ouvrir_index() {
        Ok(conn) => conn,
        Err(e) => return echec(e),
    };

    let identifiants: Vec<i64> = body
        .get("ids")
        .and_then(|v| v.as_array())
        .map(|valeurs| {
            valeurs
                .iter()
                .filter_map(|valeur| valeur.as_i64())
                .collect()
        })
        .unwrap_or_default();
    let scope = texte(&body, "scope").to_lowercase();
    let references: Vec<String> = body
        .get("references")
        .and_then(|v| v.as_array())
        .map(|valeurs| {
            valeurs
                .iter()
                .filter_map(|valeur| valeur.as_str().map(|v| v.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let tout = body
        .get("all")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if tout || (identifiants.is_empty() && scope.is_empty() && references.is_empty()) {
        let _ = conn.execute(
            "UPDATE screen_notifications SET read = 1 WHERE UPPER(uq) = UPPER(?1)",
            params![uq],
        );
    } else {
        for identifiant in identifiants {
            let _ = conn.execute(
                "UPDATE screen_notifications SET read = 1
                 WHERE id = ?1 AND UPPER(uq) = UPPER(?2)",
                params![identifiant, uq],
            );
        }
        if !scope.is_empty() {
            let _ = conn.execute(
                "UPDATE screen_notifications SET read = 1
                 WHERE UPPER(uq) = UPPER(?1) AND scope = ?2",
                params![uq, scope],
            );
        }
    }
    for reference in references {
        if reference.trim().is_empty() {
            continue;
        }
        let _ = conn.execute(
            "INSERT OR REPLACE INTO screen_ack (uq, reference, read_at) VALUES (?1, ?2, ?3)",
            params![uq, reference, maintenant()],
        );
    }

    HttpResponse::Ok().json(json!({
        "status": "success",
        "message": "Notifications mises à jour"
    }))
}

#[cfg(test)]
mod notifications_tri_tests {
    use super::*;

    fn notification(id: i64, scope: &str, lue: bool, created_at: i64) -> Value {
        json!({ "id": id, "scope": scope, "read": lue, "created_at": created_at, "en_cours": false })
    }

    fn en_cours(id: i64, scope: &str, lue: bool, created_at: i64) -> Value {
        let mut valeur = notification(id, scope, lue, created_at);
        valeur["en_cours"] = json!(true);
        valeur
    }

    fn scopes(flux: &[Value]) -> Vec<String> {
        flux.iter()
            .map(|n| n["scope"].as_str().unwrap().to_string())
            .collect()
    }

    /// Un élément « en cours » est épinglé devant tout le reste, même s'il est
    /// plus ancien et déjà lu : c'est une tête de notification, pas un tri.
    #[test]
    fn la_tete_epinglee_passe_devant_meme_une_non_lue_plus_recente() {
        let mut flux = vec![
            notification(9, "trading", false, 5_000),
            en_cours(4, "trading", true, 1_000),
        ];
        trier_notifications(&mut flux);
        assert_eq!(flux[0]["en_cours"].as_bool().unwrap(), true);
        assert_eq!(flux[1]["en_cours"].as_bool().unwrap(), false);
    }

    /// Dans la tête épinglée, on reste en non-lues puis heure décroissante :
    /// grouper par catégorie y casserait l'ordre d'attente réel.
    #[test]
    fn la_tete_epinglee_garde_l_ordre_chronologique() {
        let mut flux = vec![
            en_cours(1, "trading", false, 1_000),
            en_cours(2, "zando", false, 9_000),
            en_cours(3, "trading", true, 5_000),
        ];
        trier_notifications(&mut flux);
        assert_eq!(scopes(&flux), vec!["zando", "trading", "trading"]);
    }

    /// Deux catégories entrelacées dans le temps ressortent groupées : la tête
    /// de groupe se décide sur la non-lue la plus récente de chaque catégorie.
    #[test]
    fn les_categories_sont_regroupees_et_la_plus_vivante_reste_en_tete() {
        let mut flux = vec![
            notification(1, "zando", false, 8_000),
            notification(2, "trading", false, 7_000),
            notification(3, "zando", true, 6_000),
            notification(4, "trading", false, 5_000),
        ];
        trier_notifications(&mut flux);
        assert_eq!(scopes(&flux), vec!["zando", "zando", "trading", "trading"]);
        let trading: Vec<i64> = flux[2..]
            .iter()
            .map(|n| n["id"].as_i64().unwrap())
            .collect();
        assert_eq!(trading, vec![2, 4]);
        // Le zando lu (id 3) suit le zando non lu malgré un horodatage plus
        // ancien que le trading : le groupe ne se casse pas.
        assert_eq!(flux[1]["id"].as_i64().unwrap(), 3);
    }

    /// À statut égal, c'est l'horodatage qui décide, puis l'identifiant.
    #[test]
    fn a_statut_egal_la_plus_recente_dabord() {
        let mut flux = vec![
            notification(1, "trading", false, 1_000),
            notification(2, "trading", false, 5_000),
            notification(3, "zando", true, 4_000),
            notification(4, "zando", true, 4_000),
        ];
        trier_notifications(&mut flux);
        let clefs: Vec<(bool, i64, i64)> = flux
            .iter()
            .map(|n| {
                (
                    n["read"].as_bool().unwrap(),
                    n["created_at"].as_i64().unwrap(),
                    n["id"].as_i64().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            clefs,
            vec![(false, 5_000, 2), (false, 1_000, 1), (true, 4_000, 4), (true, 4_000, 3)]
        );
    }

    /// Les signaux d'état portent le même horodatage que l'appel : à vivacité
    /// égale, l'ordre de repli des catégories décide (ce qui porte de l'argent
    /// d'abord), pas l'ordre de production.
    #[test]
    fn a_horodatage_egal_les_categories_suivent_l_ordre_de_repli() {
        let maintenant = 7_777;
        let mut flux = vec![
            notification(0, "kyc", false, maintenant),
            notification(0, "coffre", false, maintenant),
            notification(0, "chat", false, maintenant),
            notification(8, "trading", false, maintenant - 60),
        ];
        trier_notifications(&mut flux);
        assert_eq!(scopes(&flux), vec!["coffre", "chat", "kyc", "trading"]);
    }

    /// Une catégorie muette (tout lu) reste derrière une catégorie qui attend
    /// une réponse, même si son dernier événement est plus ancien.
    #[test]
    fn une_non_lue_remonte_sa_categorie_devant_une_lue_recente() {
        let mut flux = vec![
            notification(1, "zando", true, 9_000),
            notification(2, "trading", false, 1_000),
        ];
        trier_notifications(&mut flux);
        assert_eq!(scopes(&flux), vec!["trading", "zando"]);
    }

    fn base() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        ensure_tables(&conn).unwrap();
        conn
    }

    fn ordre(conn: &Connection, uq: &str, statut: &str) -> i64 {
        conn.execute(
            "INSERT INTO trading_orders
                (uq, pair, base, quote, side, order_type, price, amount, status, created_at, updated_at)
             VALUES (?1, 'USDCONG', 'USD', 'CONG', 'buy', 'limit', 1.0, 10.0, ?2, 0, 0)",
            params![uq, statut],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn commande_p2p(conn: &Connection, acheteur: &str, vendeur: &str, statut: &str) -> i64 {
        conn.execute(
            "INSERT INTO p2p_orders
                (offer_id, buyer, seller, currency, amount, usd_amount, rate, status, created_at, updated_at)
             VALUES (1, ?1, ?2, 'CONG', 1000.0, 10.0, 100.0, ?3, 0, 0)",
            params![acheteur, vendeur, statut],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn epargne(conn: &Connection, uq: &str, statut: &str) -> i64 {
        conn.execute(
            "INSERT INTO earn_positions
                (uq, currency, principal, rewards, status, locked_at, unlock_at, created_at, updated_at)
             VALUES (?1, 'USD', 100.0, 0, ?2, 0, 0, 0, 0)",
            params![uq, statut],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    /// « En cours » se lit sur l'état réel de l'objet, pas sur le texte de la
    /// notification : un ordre exécuté ou une commande terminée sortent de la
    /// tête épinglée, un ordre ouvert y entre.
    #[test]
    fn les_references_actives_suivent_les_statuts_reels() {
        let conn = base();
        let ouvert = ordre(&conn, "UQ-1", "open");
        let execute = ordre(&conn, "UQ-1", "filled");
        let annule = ordre(&conn, "UQ-1", "cancelled");
        let a_payer = commande_p2p(&conn, "UQ-1", "UQ-2", "pending_payment");
        let paiement_annonce = commande_p2p(&conn, "UQ-2", "UQ-1", "paid");
        let terminee = commande_p2p(&conn, "UQ-1", "UQ-2", "completed");
        let sautee = commande_p2p(&conn, "UQ-1", "UQ-2", "cancelled");
        let position = epargne(&conn, "UQ-1", "active");
        let retiree = epargne(&conn, "UQ-1", "withdrawn");

        let actives = references_en_cours(&conn, "UQ-1");

        for attendue in [
            format!("order:{}", ouvert),
            format!("p2p:{}", a_payer),
            format!("p2p:{}", paiement_annonce),
            format!("earn:{}", position),
        ] {
            assert!(actives.contains(&attendue), "{attendue} devrait être en cours");
        }
        for morte in [
            format!("order:{}", execute),
            format!("order:{}", annule),
            format!("p2p:{}", terminee),
            format!("p2p:{}", sautee),
            format!("earn:{}", retiree),
        ] {
            assert!(!actives.contains(&morte), "{morte} ne devrait plus être en cours");
        }
    }
}

#[cfg(test)]
mod earn_tests {
    use super::*;

    fn base() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        ensure_tables(&conn).unwrap();
        conn
    }

    fn position(conn: &Connection, uq: &str, devise: &str, principal: f64) -> i64 {
        conn.execute(
            "INSERT INTO earn_positions
                (uq, currency, principal, rewards, status, locked_at, unlock_at, created_at, updated_at)
             VALUES (?1, ?2, ?3, 0, 'active', 0, 0, 0, 0)",
            params![uq, devise, principal],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn distribue_les_frais_au_prorata() {
        let conn = base();
        let petit = position(&conn, "UQ-1", "USD", 100.0);
        let grand = position(&conn, "UQ-2", "USD", 300.0);
        conn.execute(
            "INSERT INTO earn_pool (currency, collected, distributed, updated_at)
             VALUES ('USD', 40, 0, 0)",
            [],
        )
        .unwrap();

        assurer_distribution(&conn).unwrap();

        let r1: f64 = conn
            .query_row(
                "SELECT rewards FROM earn_positions WHERE id = ?1",
                params![petit],
                |row| row.get(0),
            )
            .unwrap();
        let r2: f64 = conn
            .query_row(
                "SELECT rewards FROM earn_positions WHERE id = ?1",
                params![grand],
                |row| row.get(0),
            )
            .unwrap();
        assert!((r1 - 10.0).abs() < 1e-9);
        assert!((r2 - 30.0).abs() < 1e-9);
        let distribue: f64 = conn
            .query_row(
                "SELECT distributed FROM earn_pool WHERE currency = 'USD'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!((distribue - 40.0).abs() < 1e-9);
    }

    #[test]
    fn distribution_unique_par_jour() {
        let conn = base();
        position(&conn, "UQ-1", "USD", 100.0);
        conn.execute(
            "INSERT INTO earn_pool (currency, collected, distributed, updated_at)
             VALUES ('USD', 40, 0, 0)",
            [],
        )
        .unwrap();
        assurer_distribution(&conn).unwrap();
        conn.execute(
            "UPDATE earn_pool SET collected = collected + 10 WHERE currency = 'USD'",
            [],
        )
        .unwrap();
        assurer_distribution(&conn).unwrap();
        let rewards: f64 = conn
            .query_row(
                "SELECT COALESCE(SUM(rewards), 0) FROM earn_positions WHERE currency = 'USD'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!((rewards - 40.0).abs() < 1e-9);
    }

    #[test]
    fn penalite_anticipee() {
        let (anticipe, penalite, rendu) = calcul_retrait(100.0, 5.0, 1_000, 500);
        assert!(anticipe);
        assert!((penalite - 10.0).abs() < 1e-9);
        assert!((rendu - 95.0).abs() < 1e-9);

        let (anticipe, penalite, rendu) = calcul_retrait(100.0, 5.0, 1_000, 1_000);
        assert!(!anticipe);
        assert_eq!(penalite, 0.0);
        assert!((rendu - 105.0).abs() < 1e-9);
    }

    #[test]
    fn bareme_et_devises() {
        assert!((taux_frais_trading("USD") - 0.001).abs() < 1e-12);
        assert!((taux_frais_trading("CDF") - 0.001).abs() < 1e-12);
        assert!((taux_frais_trading("BTC") - 0.01).abs() < 1e-12);
        assert!(devise_earn("eur").is_some());
        assert!(devise_earn("BORB").is_none());
        assert!(devise_earn("BORB").is_none());
    }
}

#[cfg(test)]
mod p2p_saisie_tests {
    use super::*;

    fn rates() -> HashMap<String, f64> {
        HashMap::from([
            ("CDF".to_string(), 2800.0),
            ("EUR".to_string(), 0.9),
        ])
    }

    #[test]
    fn saisie_dans_la_devise_de_l_offre() {
        // 2 800 CDF à un offre cotée 2 800 CDF le BORB = 1 BORB.
        let borb = borb_pour_saisie_en_devise(&rates(), 2800.0, "CDF", "CDF", 2800.0).unwrap();
        assert!((borb - 1.0).abs() < 1e-9);
    }

    #[test]
    fn saisie_dans_une_autre_devise_passe_par_l_usd() {
        // 0,9 EUR = 1 USD = 2 800 CDF = 1 BORB au taux de l'offre.
        let borb = borb_pour_saisie_en_devise(&rates(), 0.9, "EUR", "CDF", 2800.0).unwrap();
        assert!((borb - 1.0).abs() < 1e-9);
        // L'USD est la base : 1 USD saisi sur une offre en USD à 1 = 1 BORB.
        let borb_usd = borb_pour_saisie_en_devise(&rates(), 2.5, "USD", "USD", 1.0).unwrap();
        assert!((borb_usd - 2.5).abs() < 1e-9);
    }

    #[test]
    fn devise_non_cotee_et_montants_deformes_refusent_la_conversion() {
        assert!(borb_pour_saisie_en_devise(&rates(), 10.0, "XXX", "CDF", 2800.0).is_none());
        assert!(borb_pour_saisie_en_devise(&rates(), 0.0, "EUR", "CDF", 2800.0).is_none());
        assert!(borb_pour_saisie_en_devise(&rates(), -5.0, "EUR", "CDF", 2800.0).is_none());
        assert!(borb_pour_saisie_en_devise(&rates(), 5.0, "EUR", "CDF", 0.0).is_none());
    }
}
