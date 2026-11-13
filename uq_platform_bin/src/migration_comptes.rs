// ------------------------------------------------------------------------
// 🔧 MIGRATION DES COMPTES EXISTANTS
// ====================
//
// Les migrations ne s'appliquaient qu'À LA CRÉATION d'un compte : les
// comptes antérieurs n'ont jamais reçu :
//   - les colonnes de chaînage  (borb_id, prev_hash, block_number)
//   - les tables v5             (history_lock, history_fragments, ubi_claims…)
//   - la synchronisation        (wallet, my_borb_units)
//   - le scellé d'historique    (history_lock)
//
// Conséquence visible : l'historique des transactions s'affichait VIDE
// (la requête échouait sur des colonnes absentes).
//
// Ce module applique tout cela à chaque compte de data/users, de façon
// idempotente. AUCUN solde n'est modifié : on ne touche qu'à la structure
// et aux tables dérivées (wallet / my_borb_units sont RECALCULÉES depuis
// balance.points, qui reste la source de vérité).
// ====================

use rusqlite::Connection;

#[derive(Debug, Clone, serde::Serialize, Default)]
pub struct RapportCompte {
    pub uq_number: String,
    pub colonnes_ajoutees: bool,
    pub tables_creees: bool,
    pub wallet_resynchronise: bool,
    pub unites_resynchronisees: bool,
    pub historique_scelle: bool,
    pub transactions_chainees: usize,
    pub transactions: i64,
    pub erreurs: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, Default)]
pub struct RapportMigration {
    pub comptes: Vec<RapportCompte>,
    pub comptes_traites: usize,
    pub comptes_modifies: usize,
    pub total_erreurs: usize,
}

/// UQ number à partir d'un nom de fichier `uq_<chiffres>.db`
fn uq_depuis_fichier(nom: &str) -> Option<String> {
    if !nom.starts_with("uq_") || !nom.ends_with(".db") {
        return None;
    }
    let brut = &nom[3..nom.len() - 3];
    if brut.is_empty() || !brut.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(format!("UQ-{}", brut))
}

/// Migre UN compte. Idempotent : relançable sans risque.
pub fn migrer_compte(uq_number: &str) -> RapportCompte {
    let mut rapport = RapportCompte {
        uq_number: uq_number.to_string(),
        ..Default::default()
    };

    let chemin = format!("data/users/{}.db", uq_number.replace("-", "_").to_lowercase());

    // 1. Migrations versionnées (v4 = colonnes, v5 = tables d'historique)
    match crate::db::init_with_migrations(&chemin) {
        Ok(_) => {}
        Err(e) => {
            rapport.erreurs.push(format!("migrations: {}", e));
            return rapport;
        }
    }

    let conn = match Connection::open(&chemin) {
        Ok(c) => c,
        Err(e) => {
            rapport.erreurs.push(format!("ouverture: {}", e));
            return rapport;
        }
    };

    // 2. Colonnes de chaînage (ceinture + bretelles)
    let avant = colonnes_transactions(&conn);
    match crate::db::ensure_transactions_columns(&conn) {
        Ok(_) => {
            let apres = colonnes_transactions(&conn);
            rapport.colonnes_ajoutees = apres.len() > avant.len();
        }
        Err(e) => rapport.erreurs.push(format!("colonnes: {}", e)),
    }

    // 3. Tables d'historique + récupération
    match crate::local_history::init_tables(&conn) {
        Ok(_) => rapport.tables_creees = true,
        Err(e) => rapport.erreurs.push(format!("tables: {}", e)),
    }

    // 4. Nombre de transactions
    rapport.transactions = conn
        .query_row("SELECT COUNT(*) FROM transactions", [], |r| r.get::<_, i64>(0))
        .unwrap_or(0);

    // 5. Backfill du chaînage : les transactions antérieures à la spécification
    //    n'ont ni borb_id ni prev_hash. On les recalcule de façon déterministe
    //    (mêmes entrées → même hash) puis on chaîne dans l'ordre chronologique.
    match backfill_chainage(&conn, uq_number) {
        Ok(n) => rapport.transactions_chainees = n,
        Err(e) => rapport.erreurs.push(format!("chainage: {}", e)),
    }

    // 6. Resynchronisation des tables DÉRIVÉES depuis balance.points
    //    (wallet et my_borb_units étaient désynchronisées : 4 285 vs 24 385)
    if let Ok(points) = conn.query_row(
        "SELECT COALESCE(points,0) FROM balance WHERE id = 1",
        [],
        |r| r.get::<_, i64>(0),
    ) {
        let n = conn
            .execute(
                "UPDATE wallet SET balance_points = ?1, balance_borb = ?2 WHERE id = 1",
                rusqlite::params![points, points as f64 / 100.0],
            )
            .unwrap_or(0);
        rapport.wallet_resynchronise = n > 0;

        let m = conn
            .execute(
                "UPDATE my_borb_units SET balance = ?1 WHERE unit = 'BORB'",
                rusqlite::params![points / 100],
            )
            .unwrap_or(0);
        rapport.unites_resynchronisees = m > 0;
    }

    // 6. Scellé de l'historique (le cadenas est embarqué dans le scellé)
    match crate::local_history::sceller(&conn, uq_number) {
        Ok(_) => rapport.historique_scelle = true,
        Err(e) => rapport.erreurs.push(format!("scelle: {}", e)),
    }

    rapport
}

fn colonnes_transactions(conn: &Connection) -> Vec<String> {
    let mut v = Vec::new();
    if let Ok(mut stmt) = conn.prepare("PRAGMA table_info(transactions)") {
        if let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(1)) {
            for row in rows.flatten() {
                v.push(row);
            }
        }
    }
    v
}

/// Recalcule `borb_id` / `prev_hash` / `block_number` pour les transactions
/// qui n'en ont pas (créées avant la spécification « historiques locaux »).
///
/// Le hash est déterministe : `borb_id = H(sender, recipient, amount, fee, date)`.
/// Le chaînage suit l'ordre chronologique (created_at, id).
/// Retourne le nombre de lignes traitées.
fn backfill_chainage(conn: &Connection, uq_number: &str) -> Result<usize, String> {
    let block_number = crate::persistence::load_global_orb().block_number as i64;

    // (id, type, amount, fee, date, contrepartie)
    let mut stmt = conn
        .prepare(
            "SELECT id, COALESCE(tx_type,''), COALESCE(amount,0), COALESCE(fee,0),
                    COALESCE(created_at,0), COALESCE(counterparty,''),
                    COALESCE(borb_id,'')
             FROM transactions
             ORDER BY created_at ASC, id ASC",
        )
        .map_err(|e| e.to_string())?;

    let lignes: Vec<(i64, String, i64, i64, i64, String, String)> = stmt
        .query_map([], |r| {
            Ok((
                r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .flatten()
        .collect();

    let mut precedent = String::new();
    let mut traitees = 0usize;

    for (id, tx_type, amount, fee, date, contrepartie, borb_existant) in lignes {
        if borb_existant.is_empty() {
            // Sens de l'opération : l'UQ du compte est l'expéditeur ou le destinataire
            let (sender, recipient) = if tx_type == "send" {
                (uq_number.to_string(), contrepartie.clone())
            } else {
                (contrepartie.clone(), uq_number.to_string())
            };

            let borb = crate::blockchain_lib::hash_transaction(
                &sender,
                &recipient,
                amount.max(0) as u64,
                fee.max(0) as u64,
                date,
            );

            conn.execute(
                "UPDATE transactions
                 SET borb_id = ?1, prev_hash = ?2, block_number = ?3
                 WHERE id = ?4",
                rusqlite::params![borb, precedent, block_number, id],
            )
            .map_err(|e| e.to_string())?;

            precedent = borb;
            traitees += 1;
        } else {
            // Déjà chaînée : on garde la référence pour la suite
            precedent = borb_existant;
        }
    }

    if traitees > 0 {
        eprintln!(
            "📜 Chaînage reconstitué pour {} : {} transaction(s) numérotée(s)",
            uq_number, traitees
        );
    }

    Ok(traitees)
}

/// Migre TOUS les comptes de data/users (aucun solde touché).
pub fn migrer_tous_les_comptes() -> RapportMigration {
    let mut rapport = RapportMigration::default();

    let Ok(entrees) = std::fs::read_dir("data/users") else {
        return rapport;
    };

    let mut uqs: Vec<String> = Vec::new();
    for e in entrees.flatten() {
        if let Some(uq) = uq_depuis_fichier(&e.file_name().to_string_lossy()) {
            uqs.push(uq);
        }
    }
    uqs.sort();
    uqs.dedup();

    for uq in uqs {
        let r = migrer_compte(&uq);
        let modifie = r.colonnes_ajoutees
            || r.tables_creees
            || r.wallet_resynchronise
            || r.unites_resynchronisees
            || r.historique_scelle;
        if modifie {
            rapport.comptes_modifies += 1;
        }
        rapport.total_erreurs += r.erreurs.len();
        rapport.comptes_traites += 1;
        rapport.comptes.push(r);
    }

    eprintln!(
        "🔧 Migration comptes : {} traité(s), {} modifié(s), {} erreur(s)",
        rapport.comptes_traites, rapport.comptes_modifies, rapport.total_erreurs
    );

    rapport
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nom_de_fichier_reconnu() {
        assert_eq!(uq_depuis_fichier("uq_732837813.db"), Some("UQ-732837813".to_string()));
        assert_eq!(uq_depuis_fichier("uq_xxx.db"), None);
        assert_eq!(uq_depuis_fichier("uq_00000001.db.backup"), None);
        assert_eq!(uq_depuis_fichier("block_orb_global.json"), None);
    }
}
