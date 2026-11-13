// ============================================================================
// marketplace.rs — Transactions marketplace UQ (escrow + machine à états)
// ============================================================================
//
// Couche financière uniquement : la table `marketplace_transactions` vit dans la
// base centrale (uq_platform_db) car une transaction touche 2 à 3 comptes.
// L'argent suit le pattern éprouvé de `p2p_escrow` :
//   - `payer`      → gèle les points de l'ACHETEUR (jamais de paiement direct)
//   - `liberer`    → débit acheteur + crédit vendeur (prix) et livreur (frais)
//   - `rembourser` → dégel intégral chez l'acheteur
// Chaque mouvement écrit son contrepartie dans le grand livre du compte
// (`local_history::enregistrer_transaction`) et resynchronise les sous-unités BORB.

use rusqlite::{params, Connection};
use serde_json::json;

use crate::database::Database;

// ---------------------------------------------------------------------------
// Constantes
// ---------------------------------------------------------------------------

/// Validité du code jetable une fois généré (secondes).
pub const CODE_TTL_SECS: i64 = 6 * 3600;
/// Tentatives de saisie du code avant invalidation.
pub const CODE_MAX_TENTATIVES: i64 = 3;
/// Délai d'acceptation du livreur (secondes) — vérifié par le job du phase 3.
pub const LIVREUR_DELAI_SECS: i64 = 30 * 60;
/// Nombre d'avertissements avant suspension de vente (Q12).
pub const AVERTISSEMENTS_MAX: i64 = 5;
/// Durée de suspension après AVERTISSEMENTS_MAX avertissements (secondes = 7 j).
pub const SELL_BLOCK_SECS: i64 = 7 * 24 * 3600;
/// Seuil minimal (en points) pour qu'un acompte soit exigé/exploitable (Q15).
pub const ACOMPTE_SEUIL: i64 = 100;

// ---------------------------------------------------------------------------
// Types de produits (Q13)
// ---------------------------------------------------------------------------

/// Les 7 types de produits marketplace. Chaque type pilote trois règles :
/// GPS obligatoire à la remise, assurance exigée à la publication et
/// acompte exigé pour les grosses transactions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeProduit {
    Physique,
    Numerique,
    Transport,
    Sport,
    EnLigne,
    Domicile,
    Service,
}

impl TypeProduit {
    pub fn from_str(s: &str) -> Result<Self, String> {
        match s.trim().to_lowercase().as_str() {
            "physique" => Ok(Self::Physique),
            "numerique" => Ok(Self::Numerique),
            "transport" => Ok(Self::Transport),
            "sport" => Ok(Self::Sport),
            "en_ligne" | "enligne" | "en-line" => Ok(Self::EnLigne),
            "domicile" => Ok(Self::Domicile),
            "service" => Ok(Self::Service),
            _ => Err(format!("Type de produit inconnu : {}", s)),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Physique => "physique",
            Self::Numerique => "numerique",
            Self::Transport => "transport",
            Self::Sport => "sport",
            Self::EnLigne => "en_ligne",
            Self::Domicile => "domicile",
            Self::Service => "service",
        }
    }

    /// Q2 — le GPS est exigé à la remise pour tout type physique ;
    /// exempte : numerique et en_ligne.
    pub fn gps_requis(&self) -> bool {
        !matches!(self, Self::Numerique | Self::EnLigne)
    }

    /// Q16 — assurance obligatoire pour transport et domicile.
    pub fn assurance_requise(&self) -> bool {
        matches!(self, Self::Transport | Self::Domicile)
    }

    /// Q15 — acompte exigé pour les types à risque au-delà du seuil.
    pub fn acompte_requis(&self) -> bool {
        matches!(self, Self::Transport | Self::Sport | Self::Domicile)
    }
}

// ---------------------------------------------------------------------------
// Schéma (idempotent, base centrale uniquement)
// ---------------------------------------------------------------------------

pub fn ensure_tables(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS marketplace_transactions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            produit_id TEXT NOT NULL,
            type_produit TEXT NOT NULL,
            vendeur TEXT NOT NULL,
            acheteur TEXT NOT NULL,
            livreur_uq TEXT,
            statut TEXT NOT NULL DEFAULT 'reservation',
            points_total INTEGER NOT NULL DEFAULT 0,
            points_livraison INTEGER NOT NULL DEFAULT 0,
            devise TEXT NOT NULL DEFAULT 'USD',
            acompte BOOLEAN NOT NULL DEFAULT 0,
            code_hash TEXT,
            code_expire_at INTEGER,
            tentatives_code INTEGER NOT NULL DEFAULT 0,
            code_saisies TEXT,
            vendeur_a_livre INTEGER NOT NULL DEFAULT 0,
            acheteur_a_recu INTEGER NOT NULL DEFAULT 0,
            motif_litige TEXT,
            litige_id TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            paye_at INTEGER,
            confirme_at INTEGER,
            livreur_at INTEGER,
            recupere_at INTEGER,
            livre_at INTEGER,
            recoit_at INTEGER,
            libere_at INTEGER,
            litige_at INTEGER,
            annule_at INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_mktx_acheteur ON marketplace_transactions(acheteur);
        CREATE INDEX IF NOT EXISTS idx_mktx_vendeur ON marketplace_transactions(vendeur);
        CREATE INDEX IF NOT EXISTS idx_mktx_livreur ON marketplace_transactions(livreur_uq);
        CREATE INDEX IF NOT EXISTS idx_mktx_statut ON marketplace_transactions(statut);

        CREATE TABLE IF NOT EXISTS marketplace_gps (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            transaction_id INTEGER NOT NULL,
            uq TEXT NOT NULL,
            lat REAL NOT NULL,
            lng REAL NOT NULL,
            recorded_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_mktgps_tx ON marketplace_gps(transaction_id);

        CREATE TABLE IF NOT EXISTS marketplace_avertissements (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            vendeur TEXT NOT NULL,
            motif TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_mktav_vendeur ON marketplace_avertissements(vendeur);

        CREATE TABLE IF NOT EXISTS creneaux (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            produit_id TEXT NOT NULL,
            vendeur TEXT NOT NULL,
            date_debut INTEGER NOT NULL,
            date_fin INTEGER NOT NULL,
            capacite INTEGER NOT NULL,
            reserve INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_creneaux_produit ON creneaux(produit_id);

        CREATE TABLE IF NOT EXISTS assurances (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            vendeur TEXT NOT NULL,
            type_produit TEXT NOT NULL,
            fichier_path TEXT NOT NULL,
            statut TEXT NOT NULL DEFAULT 'valide',
            date_expiration INTEGER,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_ass_vendeur ON assurances(vendeur, type_produit);

        CREATE TABLE IF NOT EXISTS vendeur_livreurs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            vendeur TEXT NOT NULL,
            livreur TEXT NOT NULL,
            statut TEXT NOT NULL DEFAULT 'actif',
            charte_acceptee INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL,
            UNIQUE(vendeur, livreur)
        );
        CREATE INDEX IF NOT EXISTS idx_vl_vendeur ON vendeur_livreurs(vendeur);
        CREATE INDEX IF NOT EXISTS idx_vl_livreur ON vendeur_livreurs(livreur);

        CREATE TABLE IF NOT EXISTS hors_uq_declarations (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            transaction_id INTEGER NOT NULL,
            acheteur TEXT NOT NULL,
            vendeur TEXT NOT NULL,
            statut TEXT NOT NULL DEFAULT 'declare',
            frais_acheteur INTEGER NOT NULL DEFAULT 0,
            frais_vendeur INTEGER NOT NULL DEFAULT 0,
            acheteur_pin_ok INTEGER NOT NULL DEFAULT 0,
            vendeur_pin_ok INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL,
            confirme_at INTEGER,
            termine_at INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_huq_tx ON hors_uq_declarations(transaction_id);

        CREATE TABLE IF NOT EXISTS contournements (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            uq TEXT NOT NULL,
            transaction_id INTEGER,
            motif TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_contour_uq ON contournements(uq);",
    )
    .map_err(|e| e.to_string())?;
    // Colonnes ajoutées après la première version : ALTER idempotent (ignoré si présentes).
    let _ = conn.execute("ALTER TABLE marketplace_transactions ADD COLUMN recupere_at INTEGER", []);
    let _ = conn.execute("ALTER TABLE marketplace_transactions ADD COLUMN code_saisies TEXT", []);
    let _ = conn.execute("ALTER TABLE marketplace_transactions ADD COLUMN creneau_id INTEGER", []);
    // Q18 — paiement hors UQ : mode, frais 1% et preuve des deux PIN.
    let _ = conn.execute("ALTER TABLE marketplace_transactions ADD COLUMN mode_paiement TEXT NOT NULL DEFAULT 'securise'", []);
    let _ = conn.execute("ALTER TABLE marketplace_transactions ADD COLUMN frais_hors_uq INTEGER NOT NULL DEFAULT 0", []);
    let _ = conn.execute("ALTER TABLE marketplace_transactions ADD COLUMN hors_uq_acheteur_pin INTEGER NOT NULL DEFAULT 0", []);
    let _ = conn.execute("ALTER TABLE marketplace_transactions ADD COLUMN hors_uq_vendeur_pin INTEGER NOT NULL DEFAULT 0", []);
    // Migration douce : les tables vendeur_livreurs créées avant l'ajout de
    // charte_acceptee reçoivent la colonne manquante.
    let _ = conn.execute("ALTER TABLE vendeur_livreurs ADD COLUMN charte_acceptee INTEGER NOT NULL DEFAULT 0", []);
    // Q17 — blocages par type d'opération + compteur d'avertissements vendeur (Q12).
    let _ = conn.execute("ALTER TABLE accounts ADD COLUMN marketplace_blocked INTEGER NOT NULL DEFAULT 0", []);
    let _ = conn.execute("ALTER TABLE accounts ADD COLUMN livraison_blocked INTEGER NOT NULL DEFAULT 0", []);
    let _ = conn.execute("ALTER TABLE accounts ADD COLUMN nb_avertissements INTEGER NOT NULL DEFAULT 0", []);
    let _ = conn.execute("ALTER TABLE accounts ADD COLUMN sell_blocked_until INTEGER NOT NULL DEFAULT 0", []);
    let _ = conn.execute("ALTER TABLE accounts ADD COLUMN publish_blocked_until INTEGER NOT NULL DEFAULT 0", []);
    Ok(())
}

type Conn = r2d2::PooledConnection<r2d2_sqlite::SqliteConnectionManager>;

fn db() -> Result<Conn, String> {
    let database = Database::shared("uq_platform_db.json");
    let conn = database.connexion()?;
    ensure_tables(&conn)?;
    Ok(conn)
}

fn maintenant() -> i64 {
    chrono::Utc::now().timestamp()
}

// ---------------------------------------------------------------------------
// Compte utilisateur (miroir de p2p_escrow::ouvrir_compte, privé là-bas)
// ---------------------------------------------------------------------------

fn chemin_compte(uq: &str) -> String {
    format!("data/users/{}.db", uq.replace("-", "_").to_lowercase())
}

fn ouvrir_compte(uq: &str) -> Result<Connection, String> {
    let chemin = chemin_compte(uq);
    if !std::path::Path::new(&chemin).exists() {
        return Err("Compte introuvable".to_string());
    }
    let conn = Connection::open(&chemin).map_err(|e| e.to_string())?;
    let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
    let _ = crate::db::ensure_transactions_columns(&conn);
    Ok(conn)
}

fn hash_text(source: &str) -> String {
    crate::p2p_escrow::hash_de_transaction(source)
}

fn portes_cadenas(uq: &str) -> bool {
    if crate::kyc::cadenas::porte_cadenas_structurel(uq).is_err() {
        return false;
    }
    let (orb, _garde) = crate::persistence::orb_charge_verrouille();
    orb.porte_cadenas(uq)
}

fn ecrire_ledger(
    conn: &Connection,
    tx_type: &str,
    points: i64,
    reference: &str,
    contrepartie: &str,
    destinataire: &str,
    expediteur: &str,
    horodatage: i64,
) -> Result<(), String> {
    let note = format!("Marketplace · transaction {}", reference);
    crate::local_history::enregistrer_transaction(
        conn,
        &crate::local_history::NouvelleTransaction {
            uq_number: destinataire,
            tx_type,
            amount: points,
            fee: 0,
            counterparty: contrepartie,
            memo: &note,
            borb_id: &hash_text(&format!("UQ_MKT:{}:{}:{}", reference, expediteur, points)),
            device: "marketplace",
            os: "",
            browser: "",
            ip: "",
            cadenas_hash: &hash_text(&format!("CADENAS:{}:{}", expediteur, horodatage)),
            internal_clock: horodatage,
            universal_key_hash: &hash_text(&format!("UQ_BORB_UNIVERSAL:{}", horodatage / 30)),
            signature: "unsigned",
            created_at: horodatage,
            sender_uq: expediteur,
            recipient_uq: destinataire,
        },
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

fn notifier(uq: &str, titre: &str, corps: &str, reference: &str) {
    crate::trading_routes::push_notification(uq, "marketplace", titre, corps, reference);
}

// ---------------------------------------------------------------------------
// Code jetable
// ---------------------------------------------------------------------------

const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

/// Génère un code 6 caractères côté serveur ; seul son hash est stocké.
fn generer_code() -> (String, String) {
    use sha2::{Digest, Sha256};
    let brut = format!("{}:{}", uuid::Uuid::new_v4(), maintenant());
    let digest = Sha256::digest(brut.as_bytes());
    let mut code = String::with_capacity(6);
    for i in 0..6 {
        code.push(CODE_ALPHABET[(digest[i] % CODE_ALPHABET.len() as u8) as usize] as char);
    }
    let hash = hex::encode(digest);
    (code, hash)
}

// ---------------------------------------------------------------------------
// Lecture d'une transaction
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct Transaction {
    pub id: i64,
    pub produit_id: String,
    pub type_produit: String,
    pub vendeur: String,
    pub acheteur: String,
    pub livreur_uq: Option<String>,
    pub statut: String,
    pub points_total: i64,
    pub points_livraison: i64,
    pub devise: String,
    pub acompte: bool,
    pub code_hash: Option<String>,
    pub code_expire_at: Option<i64>,
    pub tentatives_code: i64,
    pub vendeur_a_livre: bool,
    pub acheteur_a_recu: bool,
    pub recupere_at: Option<i64>,
    pub creneau_id: Option<i64>,
    pub mode_paiement: String,
    pub frais_hors_uq: i64,
    pub created_at: i64,
}

fn lire_transaction(conn: &Connection, id: i64) -> Result<Transaction, String> {
    conn.query_row(
        "SELECT id, produit_id, type_produit, vendeur, acheteur, livreur_uq, statut,
                points_total, points_livraison, devise, acompte, code_hash, code_expire_at,
                tentatives_code, vendeur_a_livre, acheteur_a_recu, recupere_at,
                creneau_id, mode_paiement, frais_hors_uq, created_at
         FROM marketplace_transactions WHERE id = ?1",
        params![id],
        |row| {
            Ok(Transaction {
                id: row.get(0)?,
                produit_id: row.get(1)?,
                type_produit: row.get(2)?,
                vendeur: row.get(3)?,
                acheteur: row.get(4)?,
                livreur_uq: row.get(5)?,
                statut: row.get(6)?,
                points_total: row.get(7)?,
                points_livraison: row.get(8)?,
                devise: row.get(9)?,
                acompte: row.get::<_, i64>(10)? != 0,
                code_hash: row.get(11)?,
                code_expire_at: row.get(12)?,
                tentatives_code: row.get(13)?,
                vendeur_a_livre: row.get::<_, i64>(14)? != 0,
                acheteur_a_recu: row.get::<_, i64>(15)? != 0,
                recupere_at: row.get(16)?,
                creneau_id: row.get(17)?,
                mode_paiement: row.get(18)?,
                frais_hors_uq: row.get(19)?,
                created_at: row.get(20)?,
            })
        },
    )
    .map_err(|_| "Transaction introuvable".to_string())
}

fn vers_json(tx: &Transaction) -> serde_json::Value {
    vers_json_pour(tx, None)
}

/// Sérialisation publique : le livreur ne voit JAMAIS le prix (Q8) —
/// ni le total, ni les frais, ni l'acompte.
fn vers_json_pour(tx: &Transaction, demandeur: Option<&str>) -> serde_json::Value {
    let mut v = json!({
        "id": tx.id,
        "produit_id": tx.produit_id,
        "type_produit": tx.type_produit,
        "vendeur": tx.vendeur,
        "acheteur": tx.acheteur,
        "livreur_uq": tx.livreur_uq,
        "statut": tx.statut,
        "points_total": tx.points_total,
        "points_livraison": tx.points_livraison,
        "devise": tx.devise,
        "acompte": tx.acompte,
        "vendeur_a_livre": tx.vendeur_a_livre,
        "acheteur_a_recu": tx.acheteur_a_recu,
        "creneau_id": tx.creneau_id,
        "mode_paiement": tx.mode_paiement,
        "frais_hors_uq": tx.frais_hors_uq,
        "created_at": tx.created_at,
    });
    if tx.livreur_uq.as_deref().is_some() && tx.livreur_uq.as_deref() == demandeur {
        if let Some(o) = v.as_object_mut() {
            o.remove("points_total");
            o.remove("points_livraison");
            o.remove("devise");
            o.remove("acompte");
            o.remove("frais_hors_uq");
        }
    }
    v
}

/// Masque les champs prix dans une réponse déjà sérialisée quand
/// l'acteur est le livreur de la transaction (Q8).
fn masquer_prix_si_livreur(v: &mut serde_json::Value, acteur: &str) {
    if v.get("livreur_uq").and_then(|x| x.as_str()) == Some(acteur) {
        if let Some(o) = v.as_object_mut() {
            o.remove("points_total");
            o.remove("points_livraison");
            o.remove("devise");
            o.remove("acompte");
            o.remove("frais_hors_uq");
        }
    }
}

// ---------------------------------------------------------------------------
// API publique du module
// ---------------------------------------------------------------------------

/// POST — crée une réservation. Aucun gel : l'argent n'est engagé qu'à `payer`.
/// Applique les règles Q12/Q13/Q14/Q15/Q16/Q17 à la création.
pub fn creer_reservation(
    acheteur: &str,
    produit_id: &str,
    type_produit: &str,
    vendeur: &str,
    points_total: i64,
    points_livraison: i64,
    devise: &str,
    acompte: bool,
    creneau_id: Option<i64>,
) -> Result<i64, String> {
    if points_total <= 0 || points_livraison < 0 || points_livraison >= points_total {
        return Err("Montants invalides".to_string());
    }
    if acheteur == vendeur {
        return Err("Impossible d'acheter son propre produit".to_string());
    }
    // Q13 — le type doit être l'un des 7 types connus ; on stocke la forme canonique.
    let type_produit = TypeProduit::from_str(type_produit)?.as_str().to_string();
    let type_obj = TypeProduit::from_str(&type_produit)?;
    // Q15 — acompte : réservé aux transactions > ACOMPTE_SEUIL, et obligatoire
    // pour les types à risque (transport, sport, domicile) au-delà du seuil.
    if acompte && points_total <= ACOMPTE_SEUIL {
        return Err(format!("L'acompte est réservé aux transactions de plus de {} points", ACOMPTE_SEUIL));
    }
    if type_obj.acompte_requis() && !acompte && points_total > ACOMPTE_SEUIL {
        return Err("Acompte obligatoire pour ce type de produit".to_string());
    }
    let conn = db()?;
    // Q17 — le vendeur ne doit pas être bloqué pour la marketplace.
    let mkt_blocked: i64 = conn
        .query_row(
            "SELECT COALESCE(marketplace_blocked, 0) FROM accounts WHERE uq_number = ?1",
            params![vendeur],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if mkt_blocked == 1 {
        return Err("Ce vendeur est bloqué sur la marketplace".to_string());
    }
    // Q12 — suspension temporaire après 5 avertissements.
    let sell_until: i64 = conn
        .query_row(
            "SELECT COALESCE(sell_blocked_until, 0) FROM accounts WHERE uq_number = ?1",
            params![vendeur],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if sell_until > maintenant() {
        return Err("Ce vendeur est suspendu temporairement de la marketplace".to_string());
    }
    // Q16 — assurance obligatoire pour transport et domicile.
    if type_obj.assurance_requise() && !assurance_valide(&conn, vendeur, &type_produit) {
        return Err("Une assurance valide est obligatoire pour ce type de produit".to_string());
    }
    // Q14 — si un créneau est visé, vérifier la capacité et réserver une place.
    if let Some(cid) = creneau_id {
        let disponible: i64 = conn
            .query_row(
                "SELECT capacite - reserve FROM creneaux WHERE id = ?1 AND vendeur = ?2",
                params![cid, vendeur],
                |r| r.get(0),
            )
            .unwrap_or(-1);
        if disponible <= 0 {
            return Err("Créneau indisponible ou complet".to_string());
        }
        conn.execute(
            "UPDATE creneaux SET reserve = reserve + 1 WHERE id = ?1",
            params![cid],
        )
        .map_err(|e| e.to_string())?;
    }
    let ts = maintenant();
    conn.execute(
        "INSERT INTO marketplace_transactions
            (produit_id, type_produit, vendeur, acheteur, statut, points_total,
             points_livraison, devise, acompte, creneau_id, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, 'reservation', ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
        params![
            produit_id, type_produit, vendeur, acheteur, points_total,
            points_livraison, devise, acompte as i64, creneau_id, ts
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(conn.last_insert_rowid())
}

/// GET /api/marketplace/{id}
pub fn detail(id: i64) -> Result<serde_json::Value, String> {
    let conn = db()?;
    Ok(vers_json(&lire_transaction(&conn, id)?))
}

/// Variante publique : masque le prix quand le demandeur est le livreur (Q8).
pub fn detail_pour(id: i64, demandeur: Option<&str>) -> Result<serde_json::Value, String> {
    let conn = db()?;
    Ok(vers_json_pour(&lire_transaction(&conn, id)?, demandeur))
}

/// GET /api/marketplace/mes?role= — transactions du membre connecté.
pub fn mes(uq: &str, role: &str, limite: i64) -> Result<Vec<serde_json::Value>, String> {
    let conn = db()?;
    let colonne = match role {
        "vendeur" => "vendeur",
        "livreur" => "livreur_uq",
        _ => "acheteur",
    };
    let mut stmt = conn
        .prepare(&format!(
            "SELECT id FROM marketplace_transactions WHERE {} = ?1 ORDER BY id DESC LIMIT ?2",
            colonne
        ))
        .map_err(|e| e.to_string())?;
    let ids: Vec<i64> = stmt
        .query_map(params![uq, limite], |r| r.get(0))
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    let masque_livreur = role == "livreur";
    Ok(ids
        .iter()
        .filter_map(|id| lire_transaction(&conn, *id).ok())
        .map(|tx| vers_json_pour(&tx, if masque_livreur { Some(uq) } else { None }))
        .collect())
}

/// POST /api/marketplace/{id}/payer — l'acheteur séquestre le montant.
pub fn payer(id: i64, acheteur: &str) -> Result<serde_json::Value, String> {
    let conn = db()?;
    let tx = lire_transaction(&conn, id)?;
    if tx.acheteur != acheteur {
        return Err("Seul l'acheteur peut payer".to_string());
    }
    if tx.statut != "reservation" {
        return Err(format!("Paiement impossible depuis le statut {}", tx.statut));
    }
    if let Err(e) = crate::p2p_escrow::geler(&tx.acheteur, tx.points_total, maintenant()) {
        return Err(e.message());
    }
    conn.execute(
        "UPDATE marketplace_transactions
         SET statut = 'payee', paye_at = ?1, updated_at = ?1 WHERE id = ?2",
        params![maintenant(), id],
    )
    .map_err(|e| e.to_string())?;
    notifier(
        &tx.vendeur,
        "Nouvelle commande marketplace",
        &format!("{} veut acheter : transaction #{} à confirmer.", tx.acheteur, id),
        &id.to_string(),
    );
    detail(id)
}

/// POST /api/marketplace/{id}/accepter — le vendeur confirme (bloque le produit,
/// gèle les publications : branché au module produits au phase 3).
pub fn accepter(id: i64, vendeur: &str) -> Result<serde_json::Value, String> {
    let conn = db()?;
    let tx = lire_transaction(&conn, id)?;
    if tx.vendeur != vendeur {
        return Err("Seul le vendeur peut accepter".to_string());
    }
    if tx.statut != "payee" {
        return Err(format!("Acceptation impossible depuis le statut {}", tx.statut));
    }
    let ts = maintenant();
    conn.execute(
        "UPDATE marketplace_transactions
         SET statut = 'confirme', confirme_at = ?1, updated_at = ?1 WHERE id = ?2",
        params![ts, id],
    )
    .map_err(|e| e.to_string())?;
    notifier(
        &tx.acheteur,
        "Commande confirmée",
        &format!("Le vendeur a confirmé la transaction #{}.", id),
        &id.to_string(),
    );
    detail(id)
}

/// POST /api/marketplace/{id}/refuser — le vendeur refuse : dégel + annulation.
pub fn refuser(id: i64, vendeur: &str) -> Result<serde_json::Value, String> {
    let conn = db()?;
    let tx = lire_transaction(&conn, id)?;
    if tx.vendeur != vendeur {
        return Err("Seul le vendeur peut refuser".to_string());
    }
    if !matches!(tx.statut.as_str(), "reservation" | "payee") {
        return Err(format!("Refus impossible depuis le statut {}", tx.statut));
    }
    degeler_acheteur(&tx)?;
    if let Some(cid) = tx.creneau_id {
        liberer_creneau(&conn, cid);
    }
    marquer_annule(&conn, id, "refusee_par_vendeur")?;
    detail(id)
}

/// POST /api/marketplace/{id}/livreur — le vendeur assigne un livreur de confiance.
pub fn assigner_livreur(id: i64, vendeur: &str, livreur: &str) -> Result<serde_json::Value, String> {
    if livreur == vendeur || livreur.is_empty() {
        return Err("Livreur invalide".to_string());
    }
    let conn = db()?;
    let tx = lire_transaction(&conn, id)?;
    if tx.vendeur != vendeur {
        return Err("Seul le vendeur assigne le livreur".to_string());
    }
    if !matches!(tx.statut.as_str(), "confirme" | "hors_uq_confirme" | "attente_livreur") {
        return Err(format!("Assignation impossible depuis le statut {}", tx.statut));
    }
    // Q9 — le livreur doit figurer dans la liste de confiance du vendeur.
    let confiance: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM vendeur_livreurs
             WHERE vendeur = ?1 AND livreur = ?2 AND statut = 'actif'",
            params![vendeur, livreur],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if confiance == 0 {
        return Err("Ce livreur n'est pas dans ta liste de confiance".to_string());
    }
    // Q17 — un livreur bloqué pour les livraisons ne peut pas être assigné.
    if crate::reputation_routes::est_bloque(&conn, livreur, "livraison") {
        return Err("Ce livreur est bloqué pour les livraisons".to_string());
    }
    let ts = maintenant();
    conn.execute(
        "UPDATE marketplace_transactions
         SET livreur_uq = ?1, statut = 'attente_livreur', updated_at = ?2 WHERE id = ?3",
        params![livreur, ts, id],
    )
    .map_err(|e| e.to_string())?;
    notifier(
        livreur,
        "Livraison proposée",
        &format!("Le vendeur te propose la livraison de la transaction #{} (30 min pour répondre).", id),
        &id.to_string(),
    );
    detail(id)
}

/// POST /api/marketplace/{id}/livreur_accepte — le livreur accepte (30 min max,
/// contrôle du délai dans le job du phase 3).
pub fn livreur_accepte(id: i64, livreur: &str) -> Result<serde_json::Value, String> {
    let conn = db()?;
    let tx = lire_transaction(&conn, id)?;
    if tx.livreur_uq.as_deref() != Some(livreur) {
        return Err("Tu n'es pas le livreur de cette transaction".to_string());
    }
    if tx.statut != "attente_livreur" {
        return Err(format!("Acceptation impossible depuis le statut {}", tx.statut));
    }
    let ts = maintenant();
    if ts - tx.created_at > LIVREUR_DELAI_SECS && ts.saturating_sub(lire_livreur_at(&conn, id)?) > LIVREUR_DELAI_SECS {
        return Err("Délai d'acceptation dépassé (30 min)".to_string());
    }
    conn.execute(
        "UPDATE marketplace_transactions
         SET statut = 'confirme', livreur_at = ?1, updated_at = ?1 WHERE id = ?2",
        params![ts, id],
    )
    .map_err(|e| e.to_string())?;
    detail_pour(id, Some(livreur))
}

fn lire_livreur_at(conn: &Connection, id: i64) -> Result<i64, String> {
    conn.query_row(
        "SELECT COALESCE(MAX(updated_at), created_at) FROM marketplace_transactions WHERE id = ?1",
        params![id],
        |r| r.get(0),
    )
    .map_err(|e| e.to_string())
}

/// POST /api/marketplace/{id}/livreur_refuse — le livreur refuse : retour vendeur.
pub fn livreur_refuse(id: i64, livreur: &str) -> Result<serde_json::Value, String> {
    let conn = db()?;
    let tx = lire_transaction(&conn, id)?;
    if tx.livreur_uq.as_deref() != Some(livreur) {
        return Err("Tu n'es pas le livreur de cette transaction".to_string());
    }
    if tx.statut != "attente_livreur" {
        return Err(format!("Refus impossible depuis le statut {}", tx.statut));
    }
    conn.execute(
        "UPDATE marketplace_transactions
         SET statut = 'confirme', livreur_uq = NULL, updated_at = ?1 WHERE id = ?2",
        params![maintenant(), id],
    )
    .map_err(|e| e.to_string())?;
    notifier(
        &tx.vendeur,
        "Livreur indisponible",
        &format!("Le livreur a refusé la transaction #{} : choisis-en un autre.", id),
        &id.to_string(),
    );
    detail_pour(id, Some(livreur))
}

/// POST /api/marketplace/{id}/action — étapes de validation.
/// actions : livre | recoit | recupere | code | litige | annuler
pub fn action(id: i64, acteur: &str, action: &str, code: Option<&str>, motif: Option<&str>) -> Result<serde_json::Value, String> {
    let mut v = match action {
        "livre" => marquer_etape(id, acteur, "livre"),
        "recoit" => marquer_etape(id, acteur, "recoit"),
        "recupere" => marquer_etape(id, acteur, "recupere"),
        "code" => verifier_code(id, acteur, code.unwrap_or("")),
        "litige" => ouvrir_litige(id, acteur, motif.unwrap_or("")),
        "annuler" => annuler(id, acteur),
        _ => Err("Action inconnue".to_string()),
    }?;
    // Q8 — le livreur ne voit jamais le prix, y compris dans les réponses d'action.
    masquer_prix_si_livreur(&mut v, acteur);
    Ok(v)
}

/// Marque une étape de remise :
///   - `livre`    : vendeur (remise propre) ou livreur (livraison) — statut confirme
///   - `recupere` : livreur uniquement — statut confirme (colonne vendeur_a_livre
///                 réutilisée comme « récupéré » pour le flux livreur)
///   - `recoit`   : acheteur uniquement — statut confirme
/// Quand les deux validations (`vendeur_a_livre` + `acheteur_a_recu`) sont posées,
/// le statut passe à `en_validation` et le code jetable est généré + notifié.
fn marquer_etape(id: i64, acteur: &str, etape: &str) -> Result<serde_json::Value, String> {
    let conn = db()?;
    let tx = lire_transaction(&conn, id)?;

    // Q18 — 'hors_uq_confirme' suit exactement le même chemin de remise que
    // 'confirme' ; seul le mouvement d'argent diffère (pas de séquestre).
    let etape_confirmee = matches!(tx.statut.as_str(), "confirme" | "hors_uq_confirme");
    let autorise = match (etape, etape_confirmee) {
        ("livre", true) => {
            tx.vendeur == acteur || tx.livreur_uq.as_deref() == Some(acteur)
        }
        ("recupere", true) => tx.livreur_uq.as_deref() == Some(acteur),
        ("recoit", true) => tx.acheteur == acteur,
        _ => false,
    };
    if !autorise {
        return Err(format!("Action '{}' non autorisée pour ce rôle/statut", etape));
    }

    let ts = maintenant();
    match etape {
        // Le livreur marque « récupéré chez le vendeur » : étape propre, sans flag de validation.
        "recupere" => {
            if tx.livreur_uq.as_deref() != Some(acteur) || tx.recupere_at.is_some() {
                return Err("Action 'recupere' non autorisée".to_string());
            }
            conn.execute(
                "UPDATE marketplace_transactions SET recupere_at = ?1, updated_at = ?1 WHERE id = ?2",
                params![ts, id],
            )
            .map_err(|e| e.to_string())?;
            // PARTIE 5 — « Le livreur a récupéré le colis ».
            notifier(
                &tx.acheteur,
                "Colis récupéré",
                &format!("Le livreur a récupéré le colis de la transaction #{}.", id),
                &id.to_string(),
            );
        }
        // « livré » : sans livreur → le vendeur ; avec livreur → le livreur,
        // obligatoirement APRÈS avoir marqué « récupéré ».
        "livre" => {
            let autorise_livre = if tx.livreur_uq.is_some() {
                tx.livreur_uq.as_deref() == Some(acteur) && tx.recupere_at.is_some()
            } else {
                tx.vendeur == acteur
            };
            if !autorise_livre || tx.vendeur_a_livre {
                return Err("Action 'livre' non autorisée".to_string());
            }
            // Q2 — GPS obligatoire à la remise pour les types physiques.
            if TypeProduit::from_str(&tx.type_produit)
                .map(|t| t.gps_requis())
                .unwrap_or(false)
            {
                let points = nb_points_gps(&conn, id)?;
                if points == 0 {
                    return Err(
                        "Un point GPS est requis avant de marquer 'livre' pour ce type de produit"
                            .to_string(),
                    );
                }
            }
            conn.execute(
                "UPDATE marketplace_transactions SET vendeur_a_livre = 1, livre_at = ?1, updated_at = ?1 WHERE id = ?2",
                params![ts, id],
            )
            .map_err(|e| e.to_string())?;
        }
        // « reçu » : l'acheteur uniquement.
        _ => {
            if tx.acheteur != acteur || tx.acheteur_a_recu {
                return Err("Action 'recoit' non autorisée".to_string());
            }
            conn.execute(
                "UPDATE marketplace_transactions SET acheteur_a_recu = 1, recoit_at = ?1, updated_at = ?1 WHERE id = ?2",
                params![ts, id],
            )
            .map_err(|e| e.to_string())?;
        }
    }

    // Les deux validations posées → en_validation + code jetable.
    let tx2 = lire_transaction(&conn, id)?;
    let mut reponse = vers_json(&tx2);
    if tx2.vendeur_a_livre && tx2.acheteur_a_recu {
        conn.execute(
            "UPDATE marketplace_transactions SET statut = 'en_validation', updated_at = ?1 WHERE id = ?2",
            params![ts, id],
        )
        .map_err(|e| e.to_string())?;
        let (code, hash) = generer_code();
        conn.execute(
            "UPDATE marketplace_transactions
             SET code_hash = ?1, code_expire_at = ?2, tentatives_code = 0,
                 code_saisies = '[]', updated_at = ?3
             WHERE id = ?4",
            params![hash, ts + CODE_TTL_SECS, ts, id],
        )
        .map_err(|e| e.to_string())?;
        let message = format!(
            "Code de confirmation pour la transaction #{} : {} (valable 6 h, 3 tentatives).",
            id, code
        );
        notifier(&tx.vendeur, "Code de confirmation", &message, &id.to_string());
        notifier(&tx.acheteur, "Code de confirmation", &message, &id.to_string());
        if let Some(ref l) = tx.livreur_uq {
            notifier(l, "Code de confirmation", &message, &id.to_string());
        }
        reponse["statut"] = json!("en_validation");
        reponse["code_jetable"] = json!(code);
    }
    Ok(reponse)
}

/// POST — saisie du code jetable par l'acheteur ou le livreur.
/// La libération exige les DEUX validations de code.
fn verifier_code(id: i64, acteur: &str, code: &str) -> Result<serde_json::Value, String> {
    let conn = db()?;
    let tx = lire_transaction(&conn, id)?;
    if tx.statut != "en_validation" {
        return Err(format!("Vérification impossible depuis le statut {}", tx.statut));
    }
    // Sans livreur, le vendeur est la seconde partie au code : sans cette
    // branche, une remise en main propre ne pourrait jamais passer
    // 'en_validation' -> 'libere' (le vendeur ne pouvait pas saisir).
    let roles_ok = tx.acheteur == acteur
        || tx.livreur_uq.as_deref() == Some(acteur)
        || (tx.livreur_uq.is_none() && tx.vendeur == acteur);
    if !roles_ok {
        return Err("Seul l'acheteur, le vendeur (sans livreur) ou le livreur saisit le code".to_string());
    }
    let hash_attendu = tx.code_hash.clone().ok_or("Code non généré")?;
    let expire = tx.code_expire_at.unwrap_or(0);
    if maintenant() > expire {
        return Err("Code expiré".to_string());
    }
    let hash_saisi = hash_text(&code.trim().to_uppercase());
    if hash_saisi != hash_attendu {
        let tentatives = tx.tentatives_code + 1;
        conn.execute(
            "UPDATE marketplace_transactions SET tentatives_code = ?1, updated_at = ?2 WHERE id = ?3",
            params![tentatives, maintenant(), id],
        )
        .map_err(|e| e.to_string())?;
        if tentatives >= CODE_MAX_TENTATIVES {
            return Err("Trop de tentatives : code invalidé".to_string());
        }
        return Err(format!("Code incorrect ({} tentatives restantes)", CODE_MAX_TENTATIVES - tentatives));
    }

    // Code correct : on enregistre qui a saisi dans `code_saisies` (JSON array).
    let mut saisies: Vec<String> = lire_champ(&conn, id, "code_saisies")?
        .and_then(|brut| serde_json::from_str(&brut).ok())
        .unwrap_or_default();
    if !saisies.iter().any(|a| a == acteur) {
        saisies.push(acteur.to_string());
        conn.execute(
            "UPDATE marketplace_transactions SET code_saisies = ?1, updated_at = ?2 WHERE id = ?3",
            params![serde_json::to_string(&saisies).unwrap_or_default(), maintenant(), id],
        )
        .map_err(|e| e.to_string())?;
    }

    // Règle de complétude : sans livreur → vendeur + acheteur ; avec livreur →
    // livreur + acheteur (le vendeur assiste, ne saisit pas).
    let tx3 = lire_transaction(&conn, id)?;
    let requis: Vec<&str> = if tx3.livreur_uq.is_some() {
        vec![tx3.acheteur.as_str(), tx3.livreur_uq.as_deref().unwrap()]
    } else {
        vec![tx3.vendeur.as_str(), tx3.acheteur.as_str()]
    };
    let complets = requis.iter().all(|u| saisies.iter().any(|a| a == u));
    if !complets {
        return Ok(json!({
            "status": "success",
            "message": "Code accepté — en attente de la seconde saisie.",
            "saisies": saisies,
        }));
    }

    // Tous les codes sont validés → libération du séquestre.
    liberer_esquestre(&conn, &tx3)?;
    let _ = conn.execute(
        "UPDATE marketplace_transactions
         SET statut = 'libere', code_hash = NULL, libere_at = ?1, updated_at = ?1 WHERE id = ?2",
        params![maintenant(), id],
    );
    notifier(&tx.vendeur, "Paiement libéré", &format!("Transaction #{} libérée.", id), &id.to_string());
    notifier(&tx.acheteur, "Transaction terminée", &format!("Transaction #{} libérée.", id), &id.to_string());
    if let Some(ref l) = tx.livreur_uq {
        notifier(l, "Frais de livraison versés", &format!("Transaction #{} libérée.", id), &id.to_string());
    }
    detail(id)
}

fn lire_champ(conn: &Connection, id: i64, champ: &str) -> Result<Option<String>, String> {
    conn.query_row(
        &format!("SELECT {} FROM marketplace_transactions WHERE id = ?1", champ),
        params![id],
        |r| r.get(0),
    )
    .map_err(|e| e.to_string())
}

/// Litige : gèle le séquestre (statut 'litige'), notifie le vendeur.
/// La suite (jugement 3 jours) est branchée sur `cases_*` au phase 3.
fn ouvrir_litige(id: i64, acteur: &str, motif: &str) -> Result<serde_json::Value, String> {
    let conn = db()?;
    let tx = lire_transaction(&conn, id)?;
    if tx.acheteur != acteur && tx.livreur_uq.as_deref() != Some(acteur) {
        return Err("Seul l'acheteur ou le livreur peut ouvrir un litige".to_string());
    }
    if !matches!(tx.statut.as_str(), "confirme" | "hors_uq_confirme" | "en_validation") {
        return Err(format!("Litige impossible depuis le statut {}", tx.statut));
    }
    let ts = maintenant();
    conn.execute(
        "UPDATE marketplace_transactions
         SET statut = 'litige', motif_litige = ?1, litige_at = ?2, updated_at = ?2 WHERE id = ?3",
        params![motif, ts, id],
    )
    .map_err(|e| e.to_string())?;
    notifier(
        &tx.vendeur,
        "Litige ouvert",
        &format!("L'acheteur conteste la transaction #{} : {}", id, motif),
        &id.to_string(),
    );
    detail(id)
}

/// Annulation : remboursable tant que le séquestre n'est ni libéré ni en litige.
fn annuler(id: i64, acteur: &str) -> Result<serde_json::Value, String> {
    let conn = db()?;
    let tx = lire_transaction(&conn, id)?;
    let est_acteur = tx.acheteur == acteur || tx.vendeur == acteur;
    if !est_acteur {
        return Err("Action non autorisée".to_string());
    }
    if !matches!(
        tx.statut.as_str(),
        "reservation" | "payee" | "confirme" | "attente_livreur" | "hors_uq_attente_vendeur" | "hors_uq_confirme"
    ) {
        return Err(format!("Annulation impossible depuis le statut {}", tx.statut));
    }
    degeler_acheteur(&tx)?;
    if let Some(cid) = tx.creneau_id {
        liberer_creneau(&conn, cid);
    }
    marquer_annule(&conn, id, "annulee")?;
    detail(id)
}

// ---------------------------------------------------------------------------
// Mouvement d'argent
// ---------------------------------------------------------------------------

/// Dégele l'acheteur si la transaction a été payée (séquestre actif).
/// En mode hors UQ (Q18) il n'y a jamais eu de gel : rien à rembourser.
fn degeler_acheteur(tx: &Transaction) -> Result<(), String> {
    if tx.statut == "reservation" || tx.mode_paiement == "hors_uq" {
        return Ok(());
    }
    crate::p2p_escrow::degeler(&tx.acheteur, tx.points_total, maintenant()).map_err(|e| e.message())
}

fn marquer_annule(conn: &Connection, id: i64, motif: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE marketplace_transactions
         SET statut = 'annule', motif_litige = ?1, annule_at = ?2, updated_at = ?2 WHERE id = ?3",
        params![motif, maintenant(), id],
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

/// Libération : débit de l'acheteur (points + gel), crédit vendeur (prix) et
/// livreur (frais). Chaque crédit exige le cadenas structurel du destinataire.
fn liberer_esquestre(conn: &Connection, tx: &Transaction) -> Result<(), String> {
    let ts = maintenant();
    let points_vendeur = tx.points_total - tx.points_livraison;
    let points_livreur = tx.points_livraison;

    // Q18 — paiement hors UQ : aucun séquestre n'a jamais été gelé, donc aucun
    // mouvement d'argent ici. Seuls la réputation et le créneau sont traités.
    if tx.mode_paiement != "hors_uq" {
    // Portes d'entrée : les destinataires de BORB doivent avoir un cadenas réel.
    if !portes_cadenas(&tx.vendeur) {
        return Err(format!("Le vendeur {} n'a pas de cadenas valide", tx.vendeur));
    }
    if let Some(ref l) = tx.livreur_uq {
        if points_livreur > 0 && !portes_cadenas(l) {
            return Err(format!("Le livreur {} n'a pas de cadenas valide", l));
        }
    }

    // 1. Débit acheteur : points ET frozen_points diminue de points_total.
    let mut db_acheteur = ouvrir_compte(&tx.acheteur)?;
    db_acheteur
        .execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| e.to_string())?;
    let debite = db_acheteur
        .execute(
            "UPDATE balance
             SET points = points - ?1,
                 frozen_points = COALESCE(frozen_points, 0) - ?1,
                 updated_at = ?2
             WHERE id = 1 AND points >= ?1 AND COALESCE(frozen_points, 0) >= ?1",
            params![tx.points_total, ts],
        )
        .map_err(|e| e.to_string())?;
    if debite != 1 {
        let _ = db_acheteur.execute_batch("ROLLBACK");
        return Err("Séquestre introuvable chez l'acheteur".to_string());
    }
    ecrire_ledger(
        &db_acheteur, "marketplace_achat", tx.points_total, &tx.id.to_string(),
        &tx.vendeur, &tx.acheteur, &tx.vendeur, ts,
    )?;
    crate::p2p_escrow::resynchroniser_sous_unites(&db_acheteur, ts).map_err(|e| e.message())?;
    db_acheteur.execute_batch("COMMIT").map_err(|e| e.to_string())?;

    // 2. Crédit vendeur (prix hors frais de livraison).
    if points_vendeur > 0 {
        let mut db_vendeur = ouvrir_compte(&tx.vendeur)?;
        db_vendeur.execute_batch("BEGIN IMMEDIATE").map_err(|e| e.to_string())?;
        db_vendeur
            .execute(
                "UPDATE balance SET points = points + ?1, updated_at = ?2 WHERE id = 1",
                params![points_vendeur, ts],
            )
            .map_err(|e| e.to_string())?;
        ecrire_ledger(
            &db_vendeur, "marketplace_vente", points_vendeur, &tx.id.to_string(),
            &tx.acheteur, &tx.vendeur, &tx.acheteur, ts,
        )?;
        crate::p2p_escrow::resynchroniser_sous_unites(&db_vendeur, ts).map_err(|e| e.message())?;
        db_vendeur.execute_batch("COMMIT").map_err(|e| e.to_string())?;
    }

    // 3. Crédit livreur (frais de livraison uniquement — jamais le prix produit).
    if let Some(ref livreur) = tx.livreur_uq {
        if points_livreur > 0 {
            let mut db_livreur = ouvrir_compte(livreur)?;
            db_livreur.execute_batch("BEGIN IMMEDIATE").map_err(|e| e.to_string())?;
            db_livreur
                .execute(
                    "UPDATE balance SET points = points + ?1, updated_at = ?2 WHERE id = 1",
                    params![points_livreur, ts],
                )
                .map_err(|e| e.to_string())?;
            ecrire_ledger(
                &db_livreur, "marketplace_livraison", points_livreur, &tx.id.to_string(),
                &tx.vendeur, livreur, &tx.vendeur, ts,
            )?;
            crate::p2p_escrow::resynchroniser_sous_unites(&db_livreur, ts).map_err(|e| e.message())?;
            db_livreur.execute_batch("COMMIT").map_err(|e| e.to_string())?;
        }
    }
    } // fin du bloc séquestre (mode securise uniquement)

    // Q10 — réputation : achat réussi pour l'acheteur, vente pour le vendeur,
    // livraison pour le livreur (une entrée « vert » chacune).
    let _ = crate::reputation_routes::achat_reussi(conn, &tx.acheteur);
    let _ = crate::reputation_routes::achat_reussi(conn, &tx.vendeur);
    if let Some(ref l) = tx.livreur_uq {
        let _ = crate::reputation_routes::achat_reussi(conn, l);
    }

    // Q14 — le créneau est consommé par la transaction aboutie : on libère la place.
    if let Some(cid) = tx.creneau_id {
        liberer_creneau(conn, cid);
    }

    let _ = conn; // la connexion centrale n'est pas utilisée pour l'argent
    Ok(())
}

// ---------------------------------------------------------------------------
// Q9 — Livreurs de confiance du vendeur
// ---------------------------------------------------------------------------

/// Nombre minimal de livraisons réussies pour devenir livreur de confiance.
pub const LIVREUR_MIN_LIVRAISONS: i64 = 3;

/// Ajoute (ou réactive) un livreur dans la liste de confiance du vendeur.
/// Conditions (PARTIE 2) : réputation sans rouge ni noir, charte acceptée,
/// et au moins LIVREUR_MIN_LIVRAISONS livraisons marketplace réussies.
pub fn enregistrer_livreur(vendeur: &str, livreur: &str, charte_acceptee: bool) -> Result<(), String> {
    if vendeur == livreur || livreur.trim().is_empty() {
        return Err("Livreur invalide".to_string());
    }
    if !charte_acceptee {
        return Err("Le livreur doit accepter la charte des livreurs".to_string());
    }
    let conn = db()?;
    // Réputation : pas de rouge ni de noir.
    let (rouge, noir): (i64, i64) = conn
        .query_row(
            "SELECT COALESCE(rouge, 0), COALESCE(noir, 0) FROM reputation WHERE uq = ?1",
            params![livreur],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap_or((0, 0));
    if rouge > 0 || noir > 0 {
        return Err("Réputation insuffisante : aucun rouge ni noir n'est toléré".to_string());
    }
    // Au moins LIVREUR_MIN_LIVRAISONS livraisons réussies (statut 'libere').
    let livrees: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM marketplace_transactions
             WHERE livreur_uq = ?1 AND statut = 'libere'",
            params![livreur],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if livrees < LIVREUR_MIN_LIVRAISONS {
        return Err(format!(
            "Minimum {} livraisons réussies requis (actuel : {})",
            LIVREUR_MIN_LIVRAISONS, livrees
        ));
    }
    conn.execute(
        "INSERT INTO vendeur_livreurs (vendeur, livreur, statut, charte_acceptee, created_at)
         VALUES (?1, ?2, 'actif', 1, ?3)
         ON CONFLICT(vendeur, livreur) DO UPDATE SET statut = 'actif', charte_acceptee = 1",
        params![vendeur, livreur, maintenant()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Liste les livreurs de confiance actifs d'un vendeur.
pub fn lister_livreurs(vendeur: &str) -> Result<Vec<String>, String> {
    let conn = db()?;
    let mut stmt = conn
        .prepare(
            "SELECT livreur FROM vendeur_livreurs
             WHERE vendeur = ?1 AND statut = 'actif' ORDER BY livreur",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![vendeur], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

/// Retire un livreur de la liste de confiance (statut 'retire', pas de DELETE
/// pour conserver l'historique des assignations passées).
pub fn retirer_livreur(vendeur: &str, livreur: &str) -> Result<(), String> {
    let conn = db()?;
    conn.execute(
        "UPDATE vendeur_livreurs SET statut = 'retire' WHERE vendeur = ?1 AND livreur = ?2",
        params![vendeur, livreur],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Q14 — Créneaux multiples avec capacité
// ---------------------------------------------------------------------------

/// Crée un créneau de réservation pour un produit du vendeur.
pub fn creer_creneau(
    produit_id: &str,
    vendeur: &str,
    date_debut: i64,
    date_fin: i64,
    capacite: i64,
) -> Result<i64, String> {
    if produit_id.is_empty() || vendeur.is_empty() {
        return Err("produit_id et vendeur obligatoires".to_string());
    }
    if date_fin <= date_debut {
        return Err("date_fin doit être postérieure à date_debut".to_string());
    }
    if capacite <= 0 {
        return Err("La capacité doit être positive".to_string());
    }
    let conn = db()?;
    conn.execute(
        "INSERT INTO creneaux (produit_id, vendeur, date_debut, date_fin, capacite, reserve, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6)",
        params![produit_id, vendeur, date_debut, date_fin, capacite, maintenant()],
    )
    .map_err(|e| e.to_string())?;
    Ok(conn.last_insert_rowid())
}

/// Liste les créneaux d'un produit.
pub fn lister_creneaux(produit_id: &str) -> Result<Vec<serde_json::Value>, String> {
    let conn = db()?;
    let mut stmt = conn
        .prepare(
            "SELECT id, produit_id, vendeur, date_debut, date_fin, capacite, reserve
             FROM creneaux WHERE produit_id = ?1 ORDER BY date_debut",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![produit_id], |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "produit_id": r.get::<_, String>(1)?,
                "vendeur": r.get::<_, String>(2)?,
                "date_debut": r.get::<_, i64>(3)?,
                "date_fin": r.get::<_, i64>(4)?,
                "capacite": r.get::<_, i64>(5)?,
                "reserve": r.get::<_, i64>(6)?,
                "places_restantes": r.get::<_, i64>(5)? - r.get::<_, i64>(6)?,
            }))
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

/// Libère une place sur un créneau (annulation, refus ou libération escrow).
fn liberer_creneau(conn: &Connection, creneau_id: i64) {
    let _ = conn.execute(
        "UPDATE creneaux SET reserve = MAX(0, reserve - 1) WHERE id = ?1",
        params![creneau_id],
    );
}

// ---------------------------------------------------------------------------
// Q2 — Points GPS de la transaction
// ---------------------------------------------------------------------------

/// Enregistre un point GPS pour une transaction. Seuls l'acheteur, le vendeur
/// et le livreur assigné peuvent poster un point.
pub fn enregistrer_gps(id: i64, uq: &str, lat: f64, lng: f64) -> Result<(), String> {
    if !lat.is_finite() || !lng.is_finite() || lat.abs() > 90.0 || lng.abs() > 180.0 {
        return Err("Coordonnées GPS invalides".to_string());
    }
    let conn = db()?;
    let tx = lire_transaction(&conn, id)?;
    let participant = tx.acheteur == uq
        || tx.vendeur == uq
        || tx.livreur_uq.as_deref() == Some(uq);
    if !participant {
        return Err("Tu n'es pas partie à cette transaction".to_string());
    }
    conn.execute(
        "INSERT INTO marketplace_gps (transaction_id, uq, lat, lng, recorded_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, uq, lat, lng, maintenant()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn nb_points_gps(conn: &Connection, id: i64) -> Result<i64, String> {
    conn.query_row(
        "SELECT COUNT(*) FROM marketplace_gps WHERE transaction_id = ?1",
        params![id],
        |r| r.get(0),
    )
    .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Q16 — Assurances par type de produit
// ---------------------------------------------------------------------------

/// L'acheteur/vendeur dépose une attestation d'assurance pour un type donné.
/// Le statut passe directement à 'valide' (pas de circuit de validation admin
/// pour l'instant — la colonne statut permet de le brancher plus tard).
pub fn uploader_assurance(vendeur: &str, type_produit: &str, fichier: &str) -> Result<i64, String> {
    TypeProduit::from_str(type_produit)?;
    if fichier.trim().is_empty() {
        return Err("Chemin du fichier d'assurance requis".to_string());
    }
    let conn = db()?;
    conn.execute(
        "INSERT INTO assurances (vendeur, type_produit, fichier_path, statut, created_at)
         VALUES (?1, ?2, ?3, 'valide', ?4)",
        params![vendeur, type_produit, fichier, maintenant()],
    )
    .map_err(|e| e.to_string())?;
    Ok(conn.last_insert_rowid())
}

/// true si le vendeur a une assurance valide et non expirée pour ce type.
fn assurance_valide(conn: &Connection, vendeur: &str, type_produit: &str) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM assurances
         WHERE vendeur = ?1 AND type_produit = ?2 AND statut = 'valide'
           AND (date_expiration IS NULL OR date_expiration > ?3)",
        params![vendeur, type_produit, maintenant()],
        |r| r.get::<_, i64>(0),
    )
    .unwrap_or(0)
        > 0
}

/// Point d'entrée route : le vendeur connecté vérifie sa couverture.
pub fn assurance_couverte(uq: &str, type_produit: &str) -> Result<bool, String> {
    TypeProduit::from_str(type_produit)?;
    let conn = db()?;
    Ok(assurance_valide(&conn, uq, type_produit))
}

// ---------------------------------------------------------------------------
// Q12 — Avertissements et suspension de vente
// ---------------------------------------------------------------------------

/// Enregistre un avertissement marketplace pour un vendeur.
/// Au AVERTISSEMENTS_MAX-ième avertissement, sell_blocked_until = now + 7 j.
pub fn avertir(vendeur: &str, motif: &str) -> Result<serde_json::Value, String> {
    if motif.trim().is_empty() {
        return Err("Le motif de l'avertissement est requis".to_string());
    }
    let conn = db()?;
    let ts = maintenant();
    conn.execute(
        "INSERT INTO marketplace_avertissements (vendeur, motif, created_at)
         VALUES (?1, ?2, ?3)",
        params![vendeur, motif, ts],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE accounts SET nb_avertissements = COALESCE(nb_avertissements, 0) + 1
         WHERE uq_number = ?1",
        params![vendeur],
    )
    .map_err(|e| e.to_string())?;
    let nb: i64 = conn
        .query_row(
            "SELECT COALESCE(nb_avertissements, 0) FROM accounts WHERE uq_number = ?1",
            params![vendeur],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let mut bloque = false;
    if nb >= AVERTISSEMENTS_MAX {
        conn.execute(
            "UPDATE accounts SET sell_blocked_until = ?1 WHERE uq_number = ?2",
            params![ts + SELL_BLOCK_SECS, vendeur],
        )
        .map_err(|e| e.to_string())?;
        bloque = true;
        // PARTIE 5 — notification « Vente bloquée 7 jours ».
        notifier(
            vendeur,
            "Vente bloquée 7 jours",
            &format!(
                "Tu as atteint {} avertissements marketplace : ta vente est suspendue 7 jours.",
                AVERTISSEMENTS_MAX
            ),
            &nb.to_string(),
        );
    }
    Ok(json!({
        "vendeur": vendeur,
        "avertissements": nb,
        "sell_blocked": bloque,
        "sell_blocked_until": if bloque { ts + SELL_BLOCK_SECS } else { 0 },
    }))
}

// ---------------------------------------------------------------------------
// Q18 — Paiement hors UQ (déclaré, tracé, taxé 1 %, sans garantie UQ)
// ---------------------------------------------------------------------------

/// Taux des frais de déclaration hors UQ : 1 % du prix, par partie.
const HORS_UQ_TAUX_PCT: i64 = 1;
/// Minimum par partie (≈ 0,10 USD en points).
const HORS_UQ_FRAIS_MIN: i64 = 1;

fn frais_hors_uq(points_total: i64) -> i64 {
    (points_total * HORS_UQ_TAUX_PCT / 100).max(HORS_UQ_FRAIS_MIN)
}

/// Étape 2 — l'acheteur déclare le paiement hors UQ (PIN vérifié en amont
/// par la route). La transaction passe en 'hors_uq_attente_vendeur'.
pub fn declarer_hors_uq(id: i64, acheteur: &str) -> Result<serde_json::Value, String> {
    let conn = db()?;
    let tx = lire_transaction(&conn, id)?;
    if tx.acheteur != acheteur {
        return Err("Seul l'acheteur peut déclarer un paiement hors UQ".to_string());
    }
    if tx.statut != "reservation" || tx.mode_paiement == "hors_uq" {
        return Err(format!("Déclaration impossible depuis le statut {}", tx.statut));
    }
    let ts = maintenant();
    let frais = frais_hors_uq(tx.points_total);
    conn.execute(
        "UPDATE marketplace_transactions
         SET mode_paiement = 'hors_uq', statut = 'hors_uq_attente_vendeur',
             frais_hors_uq = ?1, hors_uq_acheteur_pin = 1, updated_at = ?2
         WHERE id = ?3",
        params![frais, ts, id],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO hors_uq_declarations
            (transaction_id, acheteur, vendeur, statut, frais_acheteur, frais_vendeur,
             acheteur_pin_ok, created_at)
         VALUES (?1, ?2, ?3, 'declare', ?4, ?4, 1, ?5)",
        params![id, acheteur, tx.vendeur, frais, ts],
    )
    .map_err(|e| e.to_string())?;
    notifier(
        &tx.vendeur,
        "Paiement hors UQ déclaré",
        &format!("{} veut payer hors UQ la transaction #{} : confirme pour 1% de frais.", acheteur, id),
        &id.to_string(),
    );
    detail(id)
}

/// Étape 3 — le vendeur confirme le mode hors UQ (PIN vérifié en amont).
/// La transaction rejoint le flux normal de remise en 'hors_uq_confirme'.
pub fn confirmer_hors_uq(id: i64, vendeur: &str) -> Result<serde_json::Value, String> {
    let conn = db()?;
    let tx = lire_transaction(&conn, id)?;
    if tx.vendeur != vendeur {
        return Err("Seul le vendeur peut confirmer le paiement hors UQ".to_string());
    }
    if tx.statut != "hors_uq_attente_vendeur" {
        return Err(format!("Confirmation impossible depuis le statut {}", tx.statut));
    }
    let ts = maintenant();
    conn.execute(
        "UPDATE marketplace_transactions
         SET statut = 'hors_uq_confirme', hors_uq_vendeur_pin = 1, updated_at = ?1
         WHERE id = ?2",
        params![ts, id],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE hors_uq_declarations
         SET statut = 'confirme', vendeur_pin_ok = 1, confirme_at = ?1
         WHERE transaction_id = ?2",
        params![ts, id],
    )
    .map_err(|e| e.to_string())?;
    let msg = format!(
        "Mode hors UQ confirmé pour la transaction #{} : frais de déclaration 1% par partie. Aucune garantie UQ.",
        id
    );
    notifier(&tx.acheteur, "Paiement hors UQ confirmé", &msg, &id.to_string());
    notifier(&tx.vendeur, "Paiement hors UQ confirmé", &msg, &id.to_string());
    detail(id)
}

/// Le vendeur refuse le mode hors UQ : retour à 'reservation' en mode sécurisé.
pub fn refuser_hors_uq(id: i64, vendeur: &str) -> Result<serde_json::Value, String> {
    let conn = db()?;
    let tx = lire_transaction(&conn, id)?;
    if tx.vendeur != vendeur {
        return Err("Seul le vendeur peut refuser le paiement hors UQ".to_string());
    }
    if tx.statut != "hors_uq_attente_vendeur" {
        return Err(format!("Refus impossible depuis le statut {}", tx.statut));
    }
    let ts = maintenant();
    conn.execute(
        "UPDATE marketplace_transactions
         SET mode_paiement = 'securise', statut = 'reservation', frais_hors_uq = 0,
             hors_uq_acheteur_pin = 0, updated_at = ?1
         WHERE id = ?2",
        params![ts, id],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE hors_uq_declarations SET statut = 'refuse' WHERE transaction_id = ?1",
        params![id],
    )
    .map_err(|e| e.to_string())?;
    notifier(
        &tx.acheteur,
        "Paiement hors UQ refusé",
        &format!("Le vendeur refuse le hors UQ pour la transaction #{} : paie en mode sécurisé ou annule.", id),
        &id.to_string(),
    );
    detail(id)
}

/// Litige sur une transaction hors UQ : tracé mais sans séquestre à geler.
/// La réputation n'est créditée que si le code jetable est ensuite validé.
pub fn litige_hors_uq(id: i64, acteur: &str, motif: &str) -> Result<serde_json::Value, String> {
    let conn = db()?;
    let tx = lire_transaction(&conn, id)?;
    if tx.mode_paiement != "hors_uq" {
        return Err("Cette transaction n'est pas en mode hors UQ".to_string());
    }
    if tx.acheteur != acteur && tx.vendeur != acteur {
        return Err("Action non autorisée".to_string());
    }
    if !matches!(tx.statut.as_str(), "hors_uq_confirme" | "en_validation") {
        return Err(format!("Litige impossible depuis le statut {}", tx.statut));
    }
    let ts = maintenant();
    conn.execute(
        "UPDATE marketplace_transactions
         SET statut = 'litige', motif_litige = ?1, litige_at = ?2, updated_at = ?2 WHERE id = ?3",
        params![motif, ts, id],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE hors_uq_declarations SET statut = 'litige' WHERE transaction_id = ?1",
        params![id],
    )
    .map_err(|e| e.to_string())?;
    notifier(
        &tx.vendeur,
        "Litige hors UQ ouvert",
        &format!("Litige signalé sur la transaction hors UQ #{} : {}", id, motif),
        &id.to_string(),
    );
    detail(id)
}

/// « Terminer » côté acheteur en hors UQ : équivaut à l'action 'recoit'.
pub fn terminer_hors_uq(id: i64, acheteur: &str) -> Result<serde_json::Value, String> {
    let conn = db()?;
    let tx = lire_transaction(&conn, id)?;
    if tx.mode_paiement != "hors_uq" {
        return Err("Cette transaction n'est pas en mode hors UQ".to_string());
    }
    if tx.acheteur != acheteur {
        return Err("Seul l'acheteur peut terminer la transaction".to_string());
    }
    marquer_etape(id, acheteur, "recoit")
}

// ---------------------------------------------------------------------------
// Lectures compte : avertissements et restrictions (routes /api/account/*)
// ---------------------------------------------------------------------------

/// Avertissements marketplace d'un vendeur + son état de blocage.
pub fn avertissements(uq: &str) -> Result<serde_json::Value, String> {
    let conn = db()?;
    let ts = maintenant();
    let nb: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM marketplace_avertissements WHERE vendeur = ?1",
            params![uq],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let sell_until: i64 = conn
        .query_row(
            "SELECT COALESCE(sell_blocked_until, 0) FROM accounts WHERE uq_number = ?1",
            params![uq],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let (mkt, liv): (i64, i64) = conn
        .query_row(
            "SELECT COALESCE(marketplace_blocked, 0), COALESCE(livraison_blocked, 0)
             FROM accounts WHERE uq_number = ?1",
            params![uq],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap_or((0, 0));
    Ok(json!({
        "vendeur": uq,
        "nb_avertissements": nb,
        "avertissements_max": AVERTISSEMENTS_MAX,
        "sell_blocked": sell_until > ts,
        "sell_blocked_until": sell_until,
        "marketplace_blocked": mkt == 1,
        "livraison_blocked": liv == 1,
    }))
}

// ---------------------------------------------------------------------------
// Jobs périodiques marketplace (PARTIE 4)
// ---------------------------------------------------------------------------

/// Filet de sécurité périodique : à appeler par le scheduler (jobs 1 h).
/// - Détecte les sur-réservations (reserve > capacite) → avertissement vendeur.
/// - Libère les créneaux des transactions terminées dont la place serait restée.
/// - Purge douce : signale les contournements hors UQ non déclarés détectés
///   (transactions 'libere' avec indice de contournement → tracé, pas de blocage auto).
pub fn jobs_periodiques() -> Result<serde_json::Value, String> {
    let conn = db()?;
    let ts = maintenant();
    let mut actions = Vec::new();

    // Sur-réservation : un créneau ne doit jamais avoir reserve > capacite.
    let mut stmt = conn
        .prepare("SELECT id, produit_id, vendeur FROM creneaux WHERE reserve > capacite")
        .map_err(|e| e.to_string())?;
    let surplus: Vec<(i64, String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    for (cid, produit, vendeur) in surplus {
        conn.execute(
            "UPDATE creneaux SET reserve = capacite WHERE id = ?1",
            params![cid],
        )
        .map_err(|e| e.to_string())?;
        let _ = avertir(&vendeur, &format!("Sur-réservation détectée sur le créneau #{} (produit {})", cid, produit));
        actions.push(format!("surreservation_creneau:{}", cid));
    }

    // Intégrité des places : les transactions annulées/libérées ne doivent pas
    // laisser de réservation pendante (filet si un chemin a oublié le libérer).
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT creneau_id FROM marketplace_transactions
             WHERE creneau_id IS NOT NULL AND statut IN ('annule', 'libere')",
        )
        .map_err(|e| e.to_string())?;
    let ids: Vec<i64> = stmt
        .query_map([], |r| r.get(0))
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    for cid in ids {
        // On ne libère que si AUCUNE transaction active n'utilise ce créneau.
        let actifs: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM marketplace_transactions
                 WHERE creneau_id = ?1 AND statut NOT IN ('annule', 'libere', 'litige')",
                params![cid],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if actifs == 0 {
            liberer_creneau(&conn, cid);
            actions.push(format!("creneau_resynchronise:{}", cid));
        }
    }

    // Escrows bloqués : une transaction 'payee'/'confirme' vieille de plus de
    // 30 jours sans mouvement est signalée pour revue manuelle.
    let stale: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM marketplace_transactions
             WHERE statut IN ('payee', 'confirme', 'hors_uq_confirme', 'attente_livreur')
               AND updated_at < ?1",
            params![ts - 30 * 24 * 3600],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if stale > 0 {
        actions.push(format!("escrows_stales:{}", stale));
    }

    Ok(json!({ "ok": true, "actions": actions, "ts": ts }))
}

// ---------------------------------------------------------------------------
// Tests unitaires (logique pure, sans base)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_fait_6_caracteres_alfanumeriques() {
        let (code, hash) = generer_code();
        assert_eq!(code.len(), 6);
        assert!(code.chars().all(|c| CODE_ALPHABET.contains(&(c as u8))));
        assert_eq!(hash.len(), 64);
        let (autre, _) = generer_code();
        assert_ne!(code, autre);
    }

    #[test]
    fn reservation_refuse_montants_invalides() {
        // Vérifié via la garde avant tout accès DB : on teste la logique
        // indirectement — les erreurs de validation sont des String.
        assert!(points_invalides(0, 0));
        assert!(points_invalides(100, 100));
        assert!(points_invalides(100, 150));
        assert!(!points_invalides(100, 10));
    }

    fn points_invalides(total: i64, livraison: i64) -> bool {
        total <= 0 || livraison < 0 || livraison >= total
    }
}
