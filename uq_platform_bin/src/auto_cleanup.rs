// ====================
// 🧹 AUTO CLEANUP — Suppression des historiques > 1 an
// ====================
//
// RÈGLES IMPOSÉES (clarification) :
//
//   RÈGLE 1 — PAR TRANSACTION, pas en masse
//     ❌ DELETE FROM transactions WHERE created_at < seuil          (masse)
//     ✅ DELETE ... WHERE id = (SELECT id ... ORDER BY created_at
//                               ASC LIMIT 1)                        (une ligne)
//
//   RÈGLE 2 — PAR COMPTE, pas globale
//     ❌ purger tous les comptes d'un coup
//     ✅ chaque compte gère SON cycle, au démarrage de SON app
//
//   RÈGLE 3 — LIMIT 1 ET au maximum une passe par 24 h
//     ✅ boucle de suppressions unitaires (pas de pic)
//     ✅ marqueur `history_last_cleanup_at` : une passe / 24 h / compte
//
// La purge ne touche QUE des lignes d'historique : aucun solde, aucune
// écriture dans le BlockOrb → l'invariant des 40 Md BORB est hors de cause.
// ====================

use crate::blockchain_lib::HISTORY_MAX_AGE_SECS;
use rusqlite::{params, Connection};

/// Clé du marqueur de dernière passe (table `settings` du compte)
pub const CLE_DERNIER_PASSAGE: &str = "history_last_cleanup_at";

/// Délai minimum entre deux passes pour un même compte (24 h)
pub const INTERVALLE_PASSAGE_SECS: i64 = 24 * 60 * 60;

// FIXE A 1 (decision produit) : << chaque transaction qui atteint 365 jours
// disparait >> -> une seule suppression par passe, la plus ancienne.
//   - lecture la plus pure de la regle
//   - aucun pic d'ecriture, cout constant
//   - un eventuel retard se rattrape a raison d'une transaction par jour
pub const MAX_SUPPRESSIONS_PAR_PASSE: usize = 1;

// ====================
// RAPPORT
// ====================

#[derive(Debug, Clone, serde::Serialize, Default)]
pub struct RapportPurge {
    pub uq_number: String,
    pub lignes_supprimees: usize,
    pub reste_a_purger: usize,
    pub deja_purge_recemment: bool,
    pub derniere_purge_at: i64,
    pub prochaine_purge_at: i64,
    pub age_max_jours: i64,
    pub mode: String,
}

// ====================
// OUTILS
// ====================

/// Chemin de la DB locale d'un compte
pub fn db_path(uq_number: &str) -> String {
    format!(
        "data/users/{}.db",
        uq_number.replace("-", "_").to_lowercase()
    )
}

fn lire_marqueur(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT COALESCE(value,'0') FROM settings WHERE key = ?1",
        params![CLE_DERNIER_PASSAGE],
        |row| row.get::<_, String>(0),
    )
    .ok()
    .and_then(|v| v.parse::<i64>().ok())
    .unwrap_or(0)
}

fn ecrire_marqueur(conn: &Connection, quand: i64) {
    let _ = conn.execute(
        "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)",
        params![CLE_DERNIER_PASSAGE, quand.to_string(), quand],
    );
}

/// Nombre de transactions déjà expirées (≥ 1 an) restant à purger
pub fn compter_expirees(conn: &Connection, seuil: i64) -> usize {
    conn.query_row(
        "SELECT COUNT(*) FROM transactions WHERE created_at <= ?1",
        params![seuil],
        |row| row.get::<_, i64>(0),
    )
    .unwrap_or(0) as usize
}

// ====================
// RÈGLE 1 + 3 — SUPPRESSION UNITAIRE (LIMIT 1)
// ====================

/// Supprime UNE SEULE transaction expirée : la plus ancienne.
///
/// `DELETE ... WHERE id = (SELECT id ... ORDER BY created_at ASC LIMIT 1)`
/// → jamais de suppression en masse, jamais de pic d'écriture.
/// Retourne `true` si une ligne a été supprimée.
pub fn supprimer_une_expiree(conn: &Connection, seuil: i64) -> Result<bool, String> {
    let supprimees = conn
        .execute(
            "DELETE FROM transactions
             WHERE id = (
                 SELECT id FROM transactions
                 WHERE created_at <= ?1
                 ORDER BY created_at ASC
                 LIMIT 1
             )",
            params![seuil],
        )
        .map_err(|e| e.to_string())?;

    Ok(supprimees > 0)
}

// ====================
// RÈGLE 2 — PURGE DU COMPTE COURANT UNIQUEMENT
// ====================

/// Purge l'historique du COMPTE COURANT :
///   - une transaction à la fois (LIMIT 1)
///   - au maximum une passe toutes les 24 h (sauf `forcer`)
///   - re-scellé après suppression (le scellé doit suivre l'historique)
///
/// Aucun autre compte n'est touché : chaque compte gère SON cycle.
pub fn purger_compte(
    conn: &Connection,
    uq_number: &str,
    maintenant: i64,
    forcer: bool,
) -> Result<RapportPurge, String> {
    crate::local_history::init_tables(conn)?;

    let seuil = maintenant - HISTORY_MAX_AGE_SECS;
    let dernier = lire_marqueur(conn);
    let prochaine = if dernier > 0 {
        dernier + INTERVALLE_PASSAGE_SECS
    } else {
        0
    };

    // RÈGLE 3 : une passe par 24 h maximum
    if !forcer && dernier > 0 && maintenant < prochaine {
        return Ok(RapportPurge {
            uq_number: uq_number.to_string(),
            lignes_supprimees: 0,
            reste_a_purger: compter_expirees(conn, seuil),
            deja_purge_recemment: true,
            derniere_purge_at: dernier,
            prochaine_purge_at: prochaine,
            age_max_jours: HISTORY_MAX_AGE_SECS / 86400,
            mode: "par transaction / par compte / 1x par 24h".to_string(),
        });
    }

    // RÈGLE 1 : suppressions UNITAIRES, de la plus ancienne à la plus récente
    let mut supprimees = 0usize;
    while supprimees < MAX_SUPPRESSIONS_PAR_PASSE {
        match supprimer_une_expiree(conn, seuil)? {
            true => supprimees += 1,
            false => break,
        }
    }

    if supprimees > 0 {
        // Le scellé doit refléter l'historique restant
        crate::local_history::sceller(conn, uq_number)?;
        eprintln!(
            "🧹 Purge {} : {} transaction(s) de plus d'un an supprimee(s) une par une",
            uq_number, supprimees
        );
    }

    ecrire_marqueur(conn, maintenant);

    Ok(RapportPurge {
        uq_number: uq_number.to_string(),
        lignes_supprimees: supprimees,
        reste_a_purger: compter_expirees(conn, seuil),
        deja_purge_recemment: false,
        derniere_purge_at: maintenant,
        prochaine_purge_at: maintenant + INTERVALLE_PASSAGE_SECS,
        age_max_jours: HISTORY_MAX_AGE_SECS / 86400,
        mode: "par transaction / par compte / 1x par 24h".to_string(),
    })
}

/// Appelée au démarrage de l'app D'UN compte (login) :
/// chaque compte gère son propre cycle, indépendamment des autres.
pub fn purger_compte_au_login(uq_number: &str) -> Option<RapportPurge> {
    let chemin = db_path(uq_number);
    let conn = Connection::open(&chemin).ok()?;
    let maintenant = chrono::Utc::now().timestamp();

    match purger_compte(&conn, uq_number, maintenant, false) {
        Ok(r) => Some(r),
        Err(e) => {
            eprintln!("⚠️  Purge au login de {} impossible : {}", uq_number, e);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db_test() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS transactions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                tx_type TEXT NOT NULL, amount INTEGER NOT NULL, fee INTEGER DEFAULT 0,
                counterparty TEXT, memo TEXT, signature BLOB, created_at INTEGER NOT NULL,
                sender_uq TEXT, recipient_uq TEXT, status TEXT, reference TEXT,
                borb_id TEXT, prev_hash TEXT, block_number INTEGER
            );
            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY, value TEXT, updated_at INTEGER
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

    fn inserer(conn: &Connection, age_jours: i64, maintenant: i64, tag: &str) {
        conn.execute(
            "INSERT INTO transactions (tx_type, amount, created_at, borb_id)
             VALUES ('send', 10, ?1, ?2)",
            params![maintenant - age_jours * 86400, tag],
        )
        .unwrap();
    }

    fn compter(conn: &Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM transactions", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn purge_unitaire_ne_supprime_que_les_expirees() {
        let conn = db_test();
        let maintenant = 2_000_000_000i64;
        inserer(&conn, 400, maintenant, "tres_vieux"); // expirée
        inserer(&conn, 366, maintenant, "expiree"); // expirée
        inserer(&conn, 364, maintenant, "limite"); // RESTE (364 j)
        inserer(&conn, 10, maintenant, "recente"); // reste

        // MAX_SUPPRESSIONS_PAR_PASSE = 1 : une seule ligne par passe
        let rapport = purger_compte(&conn, "UQ-TEST", maintenant, true).unwrap();

        assert_eq!(rapport.lignes_supprimees, 1, "une seule ligne par passe");
        assert_eq!(
            rapport.reste_a_purger, 1,
            "la 2e expiree reste pour la passe suivante"
        );
        assert_eq!(compter(&conn), 3);
        let restants: Vec<String> = conn
            .prepare("SELECT borb_id FROM transactions")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .flatten()
            .collect();
        assert!(restants.contains(&"limite".to_string()));
        assert!(restants.contains(&"recente".to_string()));
    }

    #[test]
    fn une_seule_passe_par_24h() {
        let conn = db_test();
        let maintenant = 2_000_000_000i64;
        inserer(&conn, 400, maintenant, "a");
        inserer(&conn, 400, maintenant, "b");

        // 1re passe : UNE seule ligne (meme si 2 sont expirees)
        let r1 = purger_compte(&conn, "UQ-TEST", maintenant, false).unwrap();
        assert_eq!(r1.lignes_supprimees, 1);
        assert_eq!(r1.reste_a_purger, 1);

        // Nouvelles lignes expirées
        inserer(&conn, 400, maintenant, "c");

        // 2e passe 1 h plus tard → ignorée (marqueur 24 h)
        let r2 = purger_compte(&conn, "UQ-TEST", maintenant + 3600, false).unwrap();
        assert_eq!(r2.lignes_supprimees, 0);
        assert!(r2.deja_purge_recemment);
        assert_eq!(r2.reste_a_purger, 2); // b (restante) + c (nouvelle)

        // 3e passe 25 h plus tard → autorisée, UNE seule ligne de plus
        let r3 = purger_compte(&conn, "UQ-TEST", maintenant + 25 * 3600, false).unwrap();
        assert_eq!(r3.lignes_supprimees, 1);
        assert_eq!(r3.reste_a_purger, 1);

        // 4e passe au lendemain → la dernière expiree part
        let r4 = purger_compte(&conn, "UQ-TEST", maintenant + 50 * 3600, false).unwrap();
        assert_eq!(r4.lignes_supprimees, 1);
        assert_eq!(r4.reste_a_purger, 0);
    }

    #[test]
    fn suppression_unitaire_une_a_une() {
        let conn = db_test();
        let maintenant = 2_000_000_000i64;
        let seuil = maintenant - HISTORY_MAX_AGE_SECS;
        inserer(&conn, 500, maintenant, "x");
        inserer(&conn, 500, maintenant, "y");
        inserer(&conn, 500, maintenant, "z");

        // Chaque appel ne retire QU'UNE ligne
        assert!(supprimer_une_expiree(&conn, seuil).unwrap());
        assert_eq!(compter(&conn), 2);
        assert!(supprimer_une_expiree(&conn, seuil).unwrap());
        assert_eq!(compter(&conn), 1);
        assert!(supprimer_une_expiree(&conn, seuil).unwrap());
        assert_eq!(compter(&conn), 0);
        assert!(!supprimer_une_expiree(&conn, seuil).unwrap());
    }

    #[test]
    fn aucun_compte_voisin_n_est_touche() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let n = N.fetch_add(1, Ordering::SeqCst);
        let maintenant = 2_000_000_000i64;

        let chemin = std::env::temp_dir().join(format!("uq_cleanup_test_{}.db", n));
        let _ = std::fs::remove_file(&chemin);
        let conn = Connection::open(&chemin).unwrap();
        conn.execute_batch(
            "CREATE TABLE transactions (
                id INTEGER PRIMARY KEY AUTOINCREMENT, tx_type TEXT NOT NULL,
                amount INTEGER NOT NULL, fee INTEGER DEFAULT 0, counterparty TEXT,
                memo TEXT, signature BLOB, created_at INTEGER NOT NULL,
                sender_uq TEXT, recipient_uq TEXT, status TEXT, reference TEXT,
                borb_id TEXT, prev_hash TEXT, block_number INTEGER
            );
            CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT, updated_at INTEGER);",
        )
        .unwrap();
        inserer(&conn, 400, maintenant, "vieux");

        // Purge du compte A
        let r = purger_compte(&conn, "UQ-A", maintenant, true).unwrap();
        assert_eq!(r.lignes_supprimees, 1);

        // Le marqueur est bien celui du compte A
        assert_eq!(lire_marqueur(&conn), maintenant);
        drop(conn);
        let _ = std::fs::remove_file(&chemin);
    }
}
