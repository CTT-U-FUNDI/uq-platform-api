// ====================
// 📜 LOCAL HISTORY — Historique persistant dans la DB de l'utilisateur
// ====================
//
// SPÉCIFICATION :
//   - chaque utilisateur garde SON historique dans SA DB locale
//   - la table `transactions` porte : borb_id (hash unique), prev_hash
//     (chaînage) et block_number
//   - l'historique est VERROUILLÉ par le cadenas : toute modification
//     manuelle ou suppression non autorisée est détectable
//   - les lignes de plus d'un an sont auto-supprimées (auto_cleanup.rs)
//
// Le scellé (`history_lock`) contient l'empreinte de la chaîne complète.
// Si quelqu'un modifie une ligne, `verifier_scelle` échoue.
// ====================

use crate::blockchain_lib::{self, TxRecord};
use rusqlite::{params, Connection};

// ====================
// TABLES LOCALES
// ====================

pub fn init_tables(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        -- Colonnes de chaînage (ajoutées aussi par migration pour les comptes existants)
        -- Le scellé de l'historique (verrouillage par le cadenas)
        CREATE TABLE IF NOT EXISTS history_lock (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            cadenas_hash TEXT,
            tx_count INTEGER NOT NULL DEFAULT 0,
            last_borb_id TEXT,
            chain_hash TEXT NOT NULL,
            locked INTEGER NOT NULL DEFAULT 1,
            updated_at INTEGER NOT NULL
        );

        -- Fragments d'historique fournis par les voisins (récupération)
        CREATE TABLE IF NOT EXISTS history_fragments (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            request_id TEXT NOT NULL,
            provider_uq TEXT NOT NULL,
            lost_uq TEXT NOT NULL,
            borb_id TEXT NOT NULL,
            prev_hash TEXT NOT NULL,
            sender_uq TEXT NOT NULL,
            recipient_uq TEXT NOT NULL,
            amount INTEGER NOT NULL,
            fee INTEGER NOT NULL DEFAULT 0,
            tx_timestamp INTEGER NOT NULL,
            block_number INTEGER NOT NULL DEFAULT 0,
            signature TEXT NOT NULL,
            accepted INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL,
            UNIQUE(request_id, provider_uq, borb_id)
        );

        -- Demandes de récupération d'historique
        CREATE TABLE IF NOT EXISTS recovery_requests (
            id TEXT PRIMARY KEY,
            requester_uq TEXT NOT NULL,
            public_uq TEXT,
            date_from INTEGER NOT NULL DEFAULT 0,
            date_to INTEGER NOT NULL DEFAULT 0,
            reason TEXT,
            signature TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'open',
            confirmations INTEGER NOT NULL DEFAULT 0,
            reconstructed_count INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL,
            closed_at INTEGER
        );

        -- Validations apportées par les voisins (3+ = validé)
        CREATE TABLE IF NOT EXISTS recovery_validations (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            request_id TEXT NOT NULL,
            validator_uq TEXT NOT NULL,
            approved INTEGER NOT NULL DEFAULT 1,
            comment TEXT,
            signature TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            UNIQUE(request_id, validator_uq)
        );

        CREATE INDEX IF NOT EXISTS idx_fragments_request ON history_fragments(request_id);
        CREATE INDEX IF NOT EXISTS idx_validations_request ON recovery_validations(request_id);
        CREATE INDEX IF NOT EXISTS idx_tx_borb ON transactions(borb_id);

        -- Réclamations UBI : créée ICI pour que TOUS les comptes l'aient
        -- (avant, elle n'apparaissait qu'à la première réclamation, ce qui
        --  laissait 8 comptes migrés sans cette table).
        CREATE TABLE IF NOT EXISTS ubi_claims (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            amount INTEGER NOT NULL,
            claimed_at INTEGER NOT NULL
        );
        "#,
    )
    .map_err(|e| e.to_string())
}

// ====================
// LECTURE DE LA CHAÎNE LOCALE
// ====================

/// Toutes les lignes de l'historique local, dans l'ordre chronologique.
pub fn charger_chaine(conn: &Connection) -> Result<Vec<TxRecord>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT COALESCE(borb_id,''), COALESCE(prev_hash,''),
                    COALESCE(sender_uq,''), COALESCE(recipient_uq,''),
                    amount, COALESCE(fee,0), created_at
             FROM transactions
             ORDER BY id ASC",
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map([], |row| {
            Ok(TxRecord {
                borb_id: row.get(0)?,
                prev_hash: row.get(1)?,
                sender_uq: row.get(2)?,
                recipient_uq: row.get(3)?,
                amount: row.get(4)?,
                fee: row.get(5)?,
                timestamp: row.get(6)?,
            })
        })
        .map_err(|e| e.to_string())?;

    Ok(rows.flatten().collect())
}

/// Dernier `borb_id` connu (pour le chaînage de la prochaine transaction)
pub fn dernier_borb_id(conn: &Connection) -> String {
    conn.query_row(
        "SELECT COALESCE(borb_id,'') FROM transactions
         WHERE COALESCE(borb_id,'') <> '' ORDER BY id DESC LIMIT 1",
        [],
        |row| row.get(0),
    )
    .unwrap_or_default()
}

/// Numéro de bloc courant du BlockOrb (lu depuis le disque)
fn block_number_courant() -> u64 {
    crate::persistence::load_global_orb().block_number
}

// ====================
// ÉCRITURE D'UNE TRANSACTION (avec chaînage)
// ====================

/// Paramètres d'une ligne d'historique local.
pub struct NouvelleTransaction<'a> {
    pub uq_number: &'a str,
    pub tx_type: &'a str,
    pub amount: i64,
    pub fee: i64,
    pub counterparty: &'a str,
    pub memo: &'a str,
    pub borb_id: &'a str,
    pub device: &'a str,
    pub os: &'a str,
    pub browser: &'a str,
    pub ip: &'a str,
    pub cadenas_hash: &'a str,
    pub internal_clock: i64,
    pub universal_key_hash: &'a str,
    pub signature: &'a str,
    pub created_at: i64,
    pub sender_uq: &'a str,
    pub recipient_uq: &'a str,
}

/// Insère une ligne d'historique local en calculant son `prev_hash`
/// (chaînage local) et son `block_number`.
pub fn enregistrer_transaction(
    conn: &Connection,
    tx: &NouvelleTransaction,
) -> Result<String, String> {
    init_tables(conn)?;

    let prev_hash = dernier_borb_id(conn);
    let block_number = block_number_courant();

    conn.execute(
        "INSERT INTO transactions
            (tx_type, amount, fee, counterparty, memo, created_at,
             device, os, browser, ip, sender_uq, recipient_uq,
             amount_borb, amount_usd, taux_usd, public_key, status, reference, tampered,
             signature, cadenas_hash, internal_clock, universal_key_hash,
             borb_id, prev_hash, block_number)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6,
             ?7, ?8, ?9, ?10, ?11, ?12,
             ?13, ?13, '1.0', '', 'success', ?14, 0,
             ?15, ?16, ?17, ?18,
             ?19, ?20, ?21)",
        params![
            tx.tx_type,
            tx.amount,
            tx.fee,
            tx.counterparty,
            tx.memo,
            tx.created_at,
            tx.device,
            tx.os,
            tx.browser,
            tx.ip,
            tx.sender_uq,
            tx.recipient_uq,
            tx.amount as f64 / 100.0,
            tx.memo,
            tx.signature,
            tx.cadenas_hash,
            tx.internal_clock,
            tx.universal_key_hash,
            tx.borb_id,
            prev_hash,
            block_number as i64,
        ],
    )
    .map_err(|e| e.to_string())?;

    // Le scellé doit suivre chaque écriture
    sceller(conn, tx.uq_number)?;

    Ok(prev_hash)
}

// ====================
// SCELLÉ DE L'HISTORIQUE (verrouillage par le cadenas)
// ====================

/// Recalcule et enregistre le scellé de l'historique local.
/// Le cadenas du compte est embarqué : impossible de re-sceller sans lui.
pub fn sceller(conn: &Connection, uq_number: &str) -> Result<String, String> {
    init_tables(conn)?;

    let chaine = charger_chaine(conn)?;
    let hash = blockchain_lib::chain_hash(&chaine);
    let dernier = chaine.last().map(|t| t.borb_id.clone()).unwrap_or_default();
    // Le cadenas vient de la DB DU COMPTE (cadenas_verification), dans la
    // meme connexion : pas d'ouverture de uq_index.db, donc pas de deadlock.
    // Sans cadenas, le scellé reste calculé mais non lié → compte fantôme.
    let cadenas: String = conn
        .query_row(
            "SELECT hash_transforme FROM cadenas_verification WHERE id = 1",
            [],
            |row| row.get::<_, Option<String>>(0),
        )
        .ok()
        .flatten()
        .unwrap_or_default();
    let now = chrono::Utc::now().timestamp();

    conn.execute(
        "INSERT OR REPLACE INTO history_lock
            (id, cadenas_hash, tx_count, last_borb_id, chain_hash, locked, updated_at)
         VALUES (1, ?1, ?2, ?3, ?4, 1, ?5)",
        params![cadenas, chaine.len() as i64, dernier, hash, now],
    )
    .map_err(|e| e.to_string())?;

    Ok(hash)
}

/// Résultat de la vérification du scellé
#[derive(Debug, Clone, serde::Serialize)]
pub struct EtatHistorique {
    pub verrouille: bool,
    pub integre: bool,
    pub chaine_valide: bool,
    pub tx_count: i64,
    pub chain_hash: String,
    pub chain_hash_attendu: Option<String>,
    pub dernier_borb_id: Option<String>,
    pub updated_at: i64,
    pub cadenas_present: bool,
}

/// Vérifie le scellé : toute modification/suppression manuelle est détectée.
pub fn verifier_scelle(conn: &Connection, uq_number: &str) -> Result<EtatHistorique, String> {
    init_tables(conn)?;

    let chaine = charger_chaine(conn)?;
    let recalcule = blockchain_lib::chain_hash(&chaine);
    let chaine_valide = blockchain_lib::verify_chain(&chaine);

    let scelle: Option<(String, i64, String, Option<String>, i64, i64)> = conn
        .query_row(
            "SELECT COALESCE(chain_hash,''), tx_count, COALESCE(last_borb_id,''),
                    cadenas_hash, locked, updated_at
             FROM history_lock WHERE id = 1",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .ok();

    let cadenas_actuel = crate::kyc::cadenas::cadenas_stocke(uq_number);

    match scelle {
        Some((hash_attendu, count, dernier, cadenas_scelle, locked, updated_at)) => {
            let integre = hash_attendu == recalcule && count == chaine.len() as i64;
            let cadenas_ok = cadenas_actuel.is_some() && cadenas_scelle == cadenas_actuel;

            Ok(EtatHistorique {
                verrouille: locked == 1,
                integre: integre && cadenas_ok,
                chaine_valide,
                tx_count: chaine.len() as i64,
                chain_hash: recalcule,
                chain_hash_attendu: Some(hash_attendu),
                dernier_borb_id: Some(dernier),
                updated_at,
                cadenas_present: cadenas_actuel.is_some(),
            })
        }
        None => Ok(EtatHistorique {
            verrouille: false,
            integre: false,
            chaine_valide,
            tx_count: chaine.len() as i64,
            chain_hash: recalcule,
            chain_hash_attendu: None,
            dernier_borb_id: None,
            updated_at: 0,
            cadenas_present: cadenas_actuel.is_some(),
        }),
    }
}

// ====================
// SUPPRESSION AUTORISÉE (nécessite un cadenas valide)
// ====================

/// Supprime des lignes d'historique APRÈS vérification du cadenas.
/// C'est le seul chemin de suppression non automatique autorisé par la
/// spécification (« l'utilisateur demande explicitement la suppression »).
pub fn supprimer_autorise(
    conn: &Connection,
    uq_number: &str,
    avant_timestamp: i64,
) -> Result<usize, String> {
    // 1. Le cadenas doit être valide
    crate::kyc::cadenas::verifier_cadenas(uq_number)?;

    // 2. Suppression
    let supprimes = conn
        .execute(
            "DELETE FROM transactions WHERE created_at < ?1",
            params![avant_timestamp],
        )
        .map_err(|e| e.to_string())?;

    // 3. Re-scellement (l'historique restant devient la nouvelle référence)
    sceller(conn, uq_number)?;

    Ok(supprimes)
}
