// ====================
// 🤝 HISTORY RECOVERY — Récupération d'historique par les voisins
// ====================
//
// SPÉCIFICATION (§4) :
//   Étape 1 — A publie « j'ai perdu mon historique, voici mon UQ public »
//   Étape 2 — les voisins B, C, D cherchent dans LEUR historique les
//             transactions où ils ont envoyé/reçu de A
//   Étape 3 — A collecte les fragments, vérifie borb_id + prev_hash,
//             reconstruit son historique
//   Étape 4 — 3 voisins ou plus confirment → validé ; sinon alerte
//
// Tables (créées dans CHAQUE DB locale par local_history::init_tables) :
//   recovery_requests     — la demande publiée
//   history_fragments     — les fragments (chez le demandeur ET le voisin)
//   recovery_validations  — les confirmations de voisins
//
// INVARIANT : la récupération ne touche JAMAIS aux soldes. On ne
// reconstruit que des LIGNES D'HISTORIQUE, jamais de la monnaie.
// ====================

use rusqlite::{Connection, params};
use crate::blockchain_lib::RECOVERY_MIN_VALIDATORS;
use crate::p2p_history::{FragmentTx, HistoryFragment, HistoryRequest};

// ====================
// OUTILS
// ====================

pub fn db_path(uq_number: &str) -> String {
    format!("data/users/{}.db", uq_number.replace("-", "_").to_lowercase())
}

pub fn compte_existe(uq_number: &str) -> bool {
    std::path::Path::new(&db_path(uq_number)).exists()
}

/// Signature locale d'un message (le client peut fournir la sienne)
pub fn signature_par_defaut(message: &str) -> String {
    let secret = std::env::var("JWT_SECRET")
        .unwrap_or_else(|_| "uq_history_default_secret".to_string());
    crate::p2p_history::signature_locale(&secret, message)
}

// ====================
// ÉTAPE 1 — PUBLIER UNE DEMANDE DE RÉCUPÉRATION
// ====================

#[derive(Debug, Clone, serde::Serialize)]
pub struct DemandeCreee {
    pub request_id: String,
    pub requester_uq: String,
    pub lost_uq: String,
    pub date_range: String,
    pub signature: String,
    pub status: String,
}

/// Crée la demande dans la DB du demandeur.
pub fn creer_demande(
    conn: &Connection,
    requester_uq: &str,
    lost_uq: &str,
    date_from: i64,
    date_to: i64,
    raison: &str,
    signature_client: Option<&str>,
) -> Result<DemandeCreee, String> {
    crate::local_history::init_tables(conn)?;

    if lost_uq.is_empty() {
        return Err("lost_uq requis".to_string());
    }
    if !compte_existe(lost_uq) {
        return Err(format!("Compte inconnu : {}", lost_uq));
    }

    let maintenant = chrono::Utc::now().timestamp();
    let request = HistoryRequest::new(requester_uq, lost_uq, date_from, date_to, "");
    let signature = signature_client
        .map(|s| s.to_string())
        .unwrap_or_else(|| signature_par_defaut(&request.message_a_signer()));

    let request_id = crate::blockchain_lib::sha256_hex(
        format!("{}:{}:{}:{}", lost_uq, requester_uq, date_from, maintenant).as_bytes(),
    );

    conn.execute(
        "INSERT OR REPLACE INTO recovery_requests
            (id, requester_uq, public_uq, date_from, date_to, reason, signature,
             status, confirmations, reconstructed_count, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'open', 0, 0, ?8)",
        params![
            &request_id,
            requester_uq,
            lost_uq,
            date_from,
            date_to,
            raison,
            signature,
            maintenant
        ],
    )
    .map_err(|e| e.to_string())?;

    eprintln!(
        "📣 Demande de récupération {} : {} demande l'historique de {}",
        &request_id[..12.min(request_id.len())],
        requester_uq,
        lost_uq
    );

    Ok(DemandeCreee {
        request_id,
        requester_uq: requester_uq.to_string(),
        lost_uq: lost_uq.to_string(),
        date_range: request.date_range,
        signature,
        status: "open".to_string(),
    })
}

/// Toutes les demandes ouvertes visibles depuis la DB d'un voisin.
///
/// Le nœud central (ce backend) stocke les demandes dans la DB du
/// demandeur ; un voisin les découvre via cette fonction, appelée par
/// la route GET /api/history/pending-requests.
pub fn demandes_ouvertes(conn: &Connection) -> Result<Vec<serde_json::Value>, String> {
    crate::local_history::init_tables(conn)?;

    let mut stmt = conn
        .prepare(
            "SELECT id, requester_uq, COALESCE(public_uq,''), date_from, date_to,
                    COALESCE(reason,''), signature, status, confirmations, created_at
             FROM recovery_requests
             WHERE status = 'open'
             ORDER BY created_at DESC LIMIT 50",
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map([], |row| {
            Ok(serde_json::json!({
                "request_id": row.get::<_, String>(0)?,
                "requester_uq": row.get::<_, String>(1)?,
                "lost_uq": row.get::<_, String>(2)?,
                "date_from": row.get::<_, i64>(3)?,
                "date_to": row.get::<_, i64>(4)?,
                "date_range": format!(
                    "{} to {}",
                    crate::p2p_history::date_lisible(row.get::<_, i64>(3)?),
                    crate::p2p_history::date_lisible(row.get::<_, i64>(4)?)
                ),
                "reason": row.get::<_, String>(5)?,
                "signature": row.get::<_, String>(6)?,
                "status": row.get::<_, String>(7)?,
                "confirmations": row.get::<_, i64>(8)?,
                "created_at": row.get::<_, i64>(9)?,
            }))
        })
        .map_err(|e| e.to_string())?;

    Ok(rows.flatten().collect())
}

// ====================
// ÉTAPE 2 — UN VOISIN CHERCHE DANS SON HISTORIQUE
// ====================

/// Recherche dans l'historique LOCAL du voisin les transactions qui
/// concernent `lost_uq` (envoyées ou reçues).
pub fn chercher_fragments(
    conn_voisin: &Connection,
    provider_uq: &str,
    lost_uq: &str,
    date_from: i64,
    date_to: i64,
) -> Result<Vec<FragmentTx>, String> {
    let mut stmt = conn_voisin
        .prepare(
            "SELECT COALESCE(borb_id,''), COALESCE(prev_hash,''),
                    COALESCE(sender_uq,''), COALESCE(recipient_uq,''),
                    amount, COALESCE(fee,0), created_at, COALESCE(block_number,0)
             FROM transactions
             WHERE (sender_uq = ?1 OR recipient_uq = ?1)
               AND (created_at BETWEEN ?2 AND ?3)
               AND COALESCE(borb_id,'') <> ''
             ORDER BY id ASC",
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map(params![lost_uq, date_from, date_to], |row| {
            Ok(FragmentTx {
                borb_id: row.get(0)?,
                prev_hash: row.get(1)?,
                sender_uq: row.get(2)?,
                recipient_uq: row.get(3)?,
                amount: row.get(4)?,
                fee: row.get(5)?,
                timestamp: row.get(6)?,
                block_number: row.get::<_, i64>(7)? as u64,
            })
        })
        .map_err(|e| e.to_string())?;

    // Le voisin ne peut attester que ce qui le concerne réellement
    let fragments: Vec<FragmentTx> = rows
        .flatten()
        .filter(|tx| tx.sender_uq == provider_uq || tx.recipient_uq == provider_uq)
        .collect();

    Ok(fragments)
}

/// Enregistre un fragment chez le fournisseur (sa propre trace de l'aide
/// apportée) et le renvoie prêt à être transmis.
pub fn enregistrer_fragment_chez_voisin(
    conn_voisin: &Connection,
    request_id: &str,
    provider_uq: &str,
    lost_uq: &str,
    transactions: &[FragmentTx],
    signature_client: Option<&str>,
) -> Result<HistoryFragment, String> {
    crate::local_history::init_tables(conn_voisin)?;

    let fragment = HistoryFragment::new(provider_uq, lost_uq, transactions.to_vec(), "");
    if !fragment.coherent() {
        return Err(
            "Fragment incoherent : le voisin doit etre partie prenante de chaque transaction"
                .to_string(),
        );
    }

    let signature = signature_client
        .map(|s| s.to_string())
        .unwrap_or_else(|| signature_par_defaut(&fragment.message_a_signer()));

    let maintenant = chrono::Utc::now().timestamp();
    for tx in transactions {
        conn_voisin
            .execute(
                "INSERT OR IGNORE INTO history_fragments
                    (request_id, provider_uq, lost_uq, borb_id, prev_hash, sender_uq,
                     recipient_uq, amount, fee, tx_timestamp, block_number, signature,
                     accepted, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 0, ?13)",
                params![
                    request_id,
                    provider_uq,
                    lost_uq,
                    tx.borb_id,
                    tx.prev_hash,
                    tx.sender_uq,
                    tx.recipient_uq,
                    tx.amount,
                    tx.fee,
                    tx.timestamp,
                    tx.block_number as i64,
                    signature,
                    maintenant
                ],
            )
            .map_err(|e| e.to_string())?;
    }

    Ok(HistoryFragment::new(provider_uq, lost_uq, transactions.to_vec(), &signature))
}

/// Dépose un fragment dans la DB du DEMANDEUR (c'est là que se fait la
/// reconstitution). Rejette les fragments incohérents ou non signés.
pub fn deposer_fragment_chez_demandeur(
    conn_demandeur: &Connection,
    request_id: &str,
    fragment: &HistoryFragment,
) -> Result<usize, String> {
    crate::local_history::init_tables(conn_demandeur)?;

    if !fragment.valide() {
        return Err("Fragment invalide (type, champs ou signature manquants)".to_string());
    }
    if !fragment.coherent() {
        return Err("Fragment incoherent avec son fournisseur".to_string());
    }

    let maintenant = chrono::Utc::now().timestamp();
    let mut ajoutes = 0usize;

    for tx in &fragment.transactions {
        let n = conn_demandeur
            .execute(
                "INSERT OR IGNORE INTO history_fragments
                    (request_id, provider_uq, lost_uq, borb_id, prev_hash, sender_uq,
                     recipient_uq, amount, fee, tx_timestamp, block_number, signature,
                     accepted, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 0, ?13)",
                params![
                    request_id,
                    fragment.from,
                    fragment.lost_uq,
                    tx.borb_id,
                    tx.prev_hash,
                    tx.sender_uq,
                    tx.recipient_uq,
                    tx.amount,
                    tx.fee,
                    tx.timestamp,
                    tx.block_number as i64,
                    fragment.signature,
                    maintenant
                ],
            )
            .map_err(|e| e.to_string())?;
        ajoutes += n;
    }

    Ok(ajoutes)
}

/// Applique les fragments à l'historique local du compte qui a perdu ses
/// données. Les lignes sont insérées dans l'ordre chronologique en
/// reconstruisant le chaînage ; les soldes ne sont JAMAIS touchés.
pub fn reconstruire_historique(
    conn: &Connection,
    uq_number: &str,
    request_id: &str,
) -> Result<usize, String> {
    crate::local_history::init_tables(conn)?;

    let mut stmt = conn
        .prepare(
            "SELECT borb_id, prev_hash, sender_uq, recipient_uq, amount, fee,
                    tx_timestamp, block_number, signature, provider_uq
             FROM history_fragments
             WHERE request_id = ?1
             ORDER BY tx_timestamp ASC, id ASC",
        )
        .map_err(|e| e.to_string())?;

    let lignes: Vec<(FragmentTx, String, String)> = stmt
        .query_map(params![request_id], |row| {
            Ok((
                FragmentTx {
                    borb_id: row.get(0)?,
                    prev_hash: row.get(1)?,
                    sender_uq: row.get(2)?,
                    recipient_uq: row.get(3)?,
                    amount: row.get(4)?,
                    fee: row.get(5)?,
                    timestamp: row.get(6)?,
                    block_number: row.get::<_, i64>(7)? as u64,
                },
                row.get::<_, String>(8)?,
                row.get::<_, String>(9)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .flatten()
        .collect();

    if lignes.is_empty() {
        return Err("Aucun fragment disponible pour cette demande".to_string());
    }

    // Déduplication par borb_id (plusieurs voisins peuvent attester la même)
    let mut vus: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut reconstruites = 0usize;

    for (tx, _signature, provider) in &lignes {
        if !vus.insert(tx.borb_id.clone()) {
            continue;
        }

        // Le voisin doit être partie prenante
        if tx.sender_uq != *provider && tx.recipient_uq != *provider {
            continue;
        }

        let type_tx = if tx.sender_uq == uq_number { "send" } else { "receive" };
        let contrepartie = if tx.sender_uq == uq_number {
            tx.recipient_uq.clone()
        } else {
            tx.sender_uq.clone()
        };

        let n = conn
            .execute(
                "INSERT OR IGNORE INTO transactions
                    (tx_type, amount, fee, counterparty, memo, created_at,
                     sender_uq, recipient_uq, status, reference,
                     borb_id, prev_hash, block_number)
                 VALUES (?1, ?2, ?3, ?4, 'recupere par voisins', ?5,
                     ?6, ?7, 'recovered', ?8,
                     ?9, ?10, ?11)",
                params![
                    type_tx,
                    tx.amount,
                    tx.fee,
                    contrepartie,
                    tx.timestamp,
                    tx.sender_uq,
                    tx.recipient_uq,
                    format!("UQ-RECOV-{}", &request_id[..12.min(request_id.len())]),
                    tx.borb_id,
                    tx.prev_hash,
                    tx.block_number as i64
                ],
            )
            .map_err(|e| e.to_string())?;

        reconstruites += n;
    }

    // Marquer les fragments acceptés + re-sceller l'historique reconstruit
    let _ = conn.execute(
        "UPDATE history_fragments SET accepted = 1 WHERE request_id = ?1",
        params![request_id],
    );

    if reconstruites > 0 {
        crate::local_history::sceller(conn, uq_number)?;
    }

    conn.execute(
        "UPDATE recovery_requests SET reconstructed_count = ?1 WHERE id = ?2",
        params![reconstruites as i64, request_id],
    )
    .ok();

    eprintln!(
        "🧩 Historique reconstruit pour {} : {} ligne(s) (demande {})",
        uq_number,
        reconstruites,
        &request_id[..12.min(request_id.len())]
    );

    Ok(reconstruites)
}

// ====================
// ÉTAPE 4 — VALIDATION PAR LES VOISINS (3+)
// ====================

#[derive(Debug, Clone, serde::Serialize)]
pub struct EtatValidation {
    pub request_id: String,
    pub confirmations: i64,
    pub minimum_requis: usize,
    pub valide: bool,
    pub statut: String,
    pub alerte: Option<String>,
}

/// Enregistre la validation d'un voisin et met à jour le statut.
pub fn valider_recuperation(
    conn_demandeur: &Connection,
    request_id: &str,
    validator_uq: &str,
    approuve: bool,
    commentaire: &str,
    signature_client: Option<&str>,
) -> Result<EtatValidation, String> {
    crate::local_history::init_tables(conn_demandeur)?;

    let (lost_uq, statut_actuel): (String, String) = conn_demandeur
        .query_row(
            "SELECT COALESCE(public_uq,''), status FROM recovery_requests WHERE id = ?1",
            params![request_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|_| format!("Demande introuvable : {}", request_id))?;

    if statut_actuel == "validated" {
        return Err("Demande deja validee".to_string());
    }

    let signature = signature_client
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            let msg = crate::p2p_history::HistoryValidation::new(
                validator_uq, &lost_uq, request_id, approuve, "",
            )
            .message_a_signer();
            signature_par_defaut(&msg)
        });

    let maintenant = chrono::Utc::now().timestamp();
    conn_demandeur
        .execute(
            "INSERT OR REPLACE INTO recovery_validations
                (request_id, validator_uq, approved, comment, signature, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                request_id,
                validator_uq,
                if approuve { 1 } else { 0 },
                commentaire,
                signature,
                maintenant
            ],
        )
        .map_err(|e| e.to_string())?;

    // Les refus comptent comme une alerte
    let refus: i64 = conn_demandeur
        .query_row(
            "SELECT COUNT(*) FROM recovery_validations WHERE request_id = ?1 AND approved = 0",
            params![request_id],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let confirmations: i64 = conn_demandeur
        .query_row(
            "SELECT COUNT(*) FROM recovery_validations WHERE request_id = ?1 AND approved = 1",
            params![request_id],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let valide = confirmations >= RECOVERY_MIN_VALIDATORS as i64;
    let nouveau_statut = if valide {
        "validated"
    } else if refus > confirmations {
        "suspect"
    } else {
        "open"
    };

    conn_demandeur
        .execute(
            "UPDATE recovery_requests
             SET confirmations = ?1, status = ?2, closed_at = ?3
             WHERE id = ?4",
            params![
                confirmations,
                nouveau_statut,
                if nouveau_statut == "open" { 0 } else { maintenant },
                request_id
            ],
        )
        .map_err(|e| e.to_string())?;

    Ok(EtatValidation {
        request_id: request_id.to_string(),
        confirmations,
        minimum_requis: RECOVERY_MIN_VALIDATORS,
        valide,
        statut: nouveau_statut.to_string(),
        alerte: if nouveau_statut == "suspect" {
            Some(format!(
                "Possible fraude : {} refus contre {} confirmation(s)",
                refus, confirmations
            ))
        } else {
            None
        },
    })
}

/// État courant d'une demande
pub fn etat_demande(conn: &Connection, request_id: &str) -> Result<serde_json::Value, String> {
    crate::local_history::init_tables(conn)?;

    let (requester, lost_uq, statut, confirmations, reconstruites, created_at): (
        String,
        String,
        String,
        i64,
        i64,
        i64,
    ) = conn
        .query_row(
            "SELECT requester_uq, COALESCE(public_uq,''), status, confirmations,
                    reconstructed_count, created_at
             FROM recovery_requests WHERE id = ?1",
            params![request_id],
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
        .map_err(|_| format!("Demande introuvable : {}", request_id))?;

    let fragments: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM history_fragments WHERE request_id = ?1",
            params![request_id],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let fournisseurs: i64 = conn
        .query_row(
            "SELECT COUNT(DISTINCT provider_uq) FROM history_fragments WHERE request_id = ?1",
            params![request_id],
            |r| r.get(0),
        )
        .unwrap_or(0);

    Ok(serde_json::json!({
        "request_id": request_id,
        "requester_uq": requester,
        "lost_uq": lost_uq,
        "status": statut,
        "confirmations": confirmations,
        "minimum_requis": RECOVERY_MIN_VALIDATORS,
        "valide": confirmations >= RECOVERY_MIN_VALIDATORS as i64,
        "fragments_recus": fragments,
        "voisins_participants": fournisseurs,
        "transactions_reconstruites": reconstruites,
        "created_at": created_at,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::p2p_history::FragmentTx;

    fn conn_test() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS transactions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                tx_type TEXT NOT NULL, amount INTEGER NOT NULL, fee INTEGER DEFAULT 0,
                counterparty TEXT, memo TEXT, signature BLOB, created_at INTEGER NOT NULL,
                sender_uq TEXT, recipient_uq TEXT, status TEXT, reference TEXT,
                borb_id TEXT, prev_hash TEXT, block_number INTEGER, device TEXT, os TEXT,
                browser TEXT, ip TEXT, amount_borb REAL, amount_usd REAL, taux_usd TEXT,
                public_key TEXT, tampered INTEGER DEFAULT 0, cadenas_hash TEXT,
                internal_clock INTEGER, universal_key_hash TEXT
            );
            CREATE TABLE IF NOT EXISTS history_lock (
                id INTEGER PRIMARY KEY CHECK (id = 1), cadenas_hash TEXT,
                tx_count INTEGER NOT NULL DEFAULT 0, last_borb_id TEXT,
                chain_hash TEXT NOT NULL, locked INTEGER NOT NULL DEFAULT 1,
                updated_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS history_fragments (
                id INTEGER PRIMARY KEY AUTOINCREMENT, request_id TEXT NOT NULL,
                provider_uq TEXT NOT NULL, lost_uq TEXT NOT NULL, borb_id TEXT NOT NULL,
                prev_hash TEXT NOT NULL, sender_uq TEXT NOT NULL, recipient_uq TEXT NOT NULL,
                amount INTEGER NOT NULL, fee INTEGER NOT NULL DEFAULT 0,
                tx_timestamp INTEGER NOT NULL, block_number INTEGER NOT NULL DEFAULT 0,
                signature TEXT NOT NULL, accepted INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL, UNIQUE(request_id, provider_uq, borb_id)
            );
            CREATE TABLE IF NOT EXISTS recovery_requests (
                id TEXT PRIMARY KEY, requester_uq TEXT NOT NULL, public_uq TEXT,
                date_from INTEGER NOT NULL DEFAULT 0, date_to INTEGER NOT NULL DEFAULT 0,
                reason TEXT, signature TEXT NOT NULL, status TEXT NOT NULL DEFAULT 'open',
                confirmations INTEGER NOT NULL DEFAULT 0,
                reconstructed_count INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL, closed_at INTEGER
            );
            CREATE TABLE IF NOT EXISTS recovery_validations (
                id INTEGER PRIMARY KEY AUTOINCREMENT, request_id TEXT NOT NULL,
                validator_uq TEXT NOT NULL, approved INTEGER NOT NULL DEFAULT 1,
                comment TEXT, signature TEXT NOT NULL, created_at INTEGER NOT NULL,
                UNIQUE(request_id, validator_uq)
            );",
        )
        .unwrap();
        conn
    }

    fn borb(seed: &str) -> String {
        crate::blockchain_lib::sha256_hex(seed.as_bytes())
    }

    #[test]
    fn validation_exige_trois_voisins() {
        let demandeur = conn_test();
        demandeur
            .execute(
                "INSERT INTO recovery_requests (id, requester_uq, public_uq, signature, created_at)
                 VALUES ('req1', 'UQ-1', 'UQ-1', 'sig', 100)",
                [],
            )
            .unwrap();

        let e1 = valider_recuperation(&demandeur, "req1", "UQ-2", true, "", None).unwrap();
        assert!(!e1.valide);
        assert_eq!(e1.confirmations, 1);

        valider_recuperation(&demandeur, "req1", "UQ-3", true, "", None).unwrap();
        let e3 = valider_recuperation(&demandeur, "req1", "UQ-4", true, "", None).unwrap();
        assert!(e3.valide);
        assert_eq!(e3.statut, "validated");
        assert_eq!(e3.confirmations, 3);
    }

    #[test]
    fn fragment_tierce_est_refuse() {
        let voisin = conn_test();
        let tx = FragmentTx {
            borb_id: borb("t1"),
            prev_hash: String::new(),
            sender_uq: "UQ-1".to_string(),
            recipient_uq: "UQ-2".to_string(),
            amount: 500,
            fee: 5,
            timestamp: 1_800_000_000,
            block_number: 1,
        };
        // UQ-9 n'est ni l'expéditeur ni le destinataire
        let res = enregistrer_fragment_chez_voisin(&voisin, "req1", "UQ-9", "UQ-1", &[tx], None);
        assert!(res.is_err());
    }
}
