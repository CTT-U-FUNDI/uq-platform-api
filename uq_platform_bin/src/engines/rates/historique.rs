// ====================
// 🗃️ HISTORIQUE — rates_history + snapshot persisté (data/uq_engines.db)
// ====================
//
// Deux usages :
//   - alimenter les graphiques et les prédictions (NIVEAU 2 et 6)
//   - permettre un démarrage hors-ligne avec le DERNIER TAUX RÉELLEMENT
//     OBSERVÉ, étiqueté « stale » — jamais une valeur inventée
//
// La base est partagée avec le store des moteurs : le budget d'octets déjà
// appliqué au store (UQ_ENGINE_QUOTA_MB, défaut 100) reste la contrainte.
// La rétention se fait donc par âge ET par quota de lignes.
// ====================

use rusqlite::{params, Connection};
use serde_json::Value;

pub const CHEMIN_DEFAUT: &str = "data/uq_engines.db";

/// Âge maximal conservé ( défaut ≈ 13 mois, pour la demande « 1 an » ).
pub const RETENTION_JOURS: i64 = 400;
/// Plafond de lignes par (devise, source) : borne dure si la rétention
/// temporelle suffisait mal face à un tick de 60 s sur des centaines de devises.
pub const MAX_LIGNES_PAR_CUPLE: i64 = 20_000;

fn chemin_db() -> String {
    std::env::var("UQ_ENGINES_DB").unwrap_or_else(|_| CHEMIN_DEFAUT.to_string())
}

/// Ouvre la base de la plateforme, puis la variante à chemin explicite.
/// Le chemin est passé en argument aux fonctions de test pour qu'elles
/// n'aient pas à modifier une variable d'environnement partagée entre threads.
fn ouvrir() -> Result<Connection, String> {
    ouvrir_chemin(&chemin_db())
}

fn ouvrir_chemin(chemin: &str) -> Result<Connection, String> {
    if let Some(dossier) = std::path::Path::new(chemin).parent() {
        let _ = std::fs::create_dir_all(dossier);
    }
    let conn = Connection::open(chemin).map_err(|e| format!("ouverture {}: {}", chemin, e))?;
    // Le store des moteurs écrit dans le même fichier : WAL + busy_timeout
    // évitent que les deux s'excluent mutuellement.
    let _ = conn.pragma_update(None, "journal_mode", "WAL");
    conn.busy_timeout(std::time::Duration::from_millis(2000))
        .map_err(|e| format!("busy_timeout : {}", e))?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS rates_history (
            timestamp INTEGER NOT NULL,
            currency  TEXT    NOT NULL,
            rate      REAL    NOT NULL,
            source    TEXT    NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_rates_history_devise_temps
            ON rates_history (currency, timestamp);
        CREATE INDEX IF NOT EXISTS idx_rates_history_source_temps
            ON rates_history (source, timestamp);
        CREATE TABLE IF NOT EXISTS rates_snapshot (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            json TEXT NOT NULL,
            a    INTEGER NOT NULL
        );",
    )
    .map_err(|e| format!("schéma rates : {}", e))?;
    Ok(conn)
}

/// Enregistre un cycle. Un point « agregat » pour la valeur servie, puis un
/// point par observation réelle de source.
///
/// La distinction compte : journaliser sous le nom « er-api » la valeur de la
/// médiane serait exactement le mensonge qu'on corrige. Chaque source n'a que
/// ce qu'elle a vraiment dit.
///
/// `observations` : source -> (devise -> valeur observée par cette source).
pub fn enregistrer(
    horodatage: i64,
    rates: &std::collections::HashMap<String, f64>,
    observations: &std::collections::HashMap<String, std::collections::HashMap<String, f64>>,
) -> Result<usize, String> {
    enregistrer_dans(&chemin_db(), horodatage, rates, observations)
}

pub fn enregistrer_dans(
    chemin: &str,
    horodatage: i64,
    rates: &std::collections::HashMap<String, f64>,
    observations: &std::collections::HashMap<String, std::collections::HashMap<String, f64>>,
) -> Result<usize, String> {
    let conn = ouvrir_chemin(chemin)?;
    let mut inserer = |devise: &str, valeur: f64, source: &str| -> usize {
        if devise == "USD" {
            // 1 USD = 1 USD n'est pas une observation de marché.
            return 0;
        }
        match conn.execute(
            "INSERT INTO rates_history (timestamp, currency, rate, source) VALUES (?1,?2,?3,?4)",
            params![horodatage, devise, valeur, source],
        ) {
            Ok(_) => 1,
            Err(_) => 0,
        }
    };
    let mut ecrits = 0usize;
    for (devise, valeur) in rates {
        ecrits += inserer(devise, *valeur, "agregat");
    }
    for (source, taux) in observations {
        for (devise, valeur) in taux {
            ecrits += inserer(devise, *valeur, source);
        }
    }
    nettoyer(&conn);
    Ok(ecrits)
}

/// Purge : par âge, puis par quota de lignes par couple (devise, source).
fn nettoyer(conn: &Connection) {
    let maintenant = chrono::Utc::now().timestamp();
    let age_max = maintenant - RETENTION_JOURS * 86_400;
    let _ = conn.execute("DELETE FROM rates_history WHERE timestamp < ?1", params![age_max]);
    let _ = conn.execute(
        "DELETE FROM rates_history WHERE rowid NOT IN (
             SELECT rowid FROM (
               SELECT rowid,
                      ROW_NUMBER() OVER (PARTITION BY currency, source ORDER BY timestamp DESC) AS n
               FROM rates_history
             ) WHERE n <= ?1
         )",
        params![MAX_LIGNES_PAR_CUPLE],
    );
}

/// Historique d'une devise sur une durée, en points chronologiques.
/// `source = None` renvoie toutes les origines (agrégat compris).
pub fn points(devise: &str, depuis_secs: i64, source: Option<&str>) -> Vec<Value> {
    points_dans(&chemin_db(), devise, depuis_secs, source)
}

pub fn points_dans(chemin: &str, devise: &str, depuis_secs: i64, source: Option<&str>) -> Vec<Value> {
    let conn = match ouvrir_chemin(chemin) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let maintenant = chrono::Utc::now().timestamp();
    let borne = maintenant - depuis_secs;

    let ligne = |l: &rusqlite::Row| -> rusqlite::Result<Value> {
        Ok(serde_json::json!({
            "timestamp": l.get::<_, i64>(0)?,
            "rate": l.get::<_, f64>(1)?,
            "source": l.get::<_, String>(2)?,
        }))
    };

    // Chaque branche consomme son propre statement : un itérateur empruntant
    // `r` ne peut pas être stocké puis renvoyé au-delà de la branche.
    match source {
        Some(s) => match conn.prepare(
            "SELECT timestamp, rate, source FROM rates_history
             WHERE currency = ?1 AND timestamp >= ?2 AND source = ?3
             ORDER BY timestamp ASC LIMIT 20000",
        ) {
            Ok(mut r) => r
                .query_map(params![devise, borne, s], ligne)
                .map(|it| it.filter_map(|l| l.ok()).collect())
                .unwrap_or_default(),
            Err(_) => Vec::new(),
        },
        None => match conn.prepare(
            "SELECT timestamp, rate, source FROM rates_history
             WHERE currency = ?1 AND timestamp >= ?2
             ORDER BY timestamp ASC LIMIT 20000",
        ) {
            Ok(mut r) => r
                .query_map(params![devise, borne], ligne)
                .map(|it| it.filter_map(|l| l.ok()).collect())
                .unwrap_or_default(),
            Err(_) => Vec::new(),
        },
    }
}

pub fn enregistrer_snapshot(json: &str, horodatage: i64) {
    enregistrer_snapshot_dans(&chemin_db(), json, horodatage);
}

pub fn enregistrer_snapshot_dans(chemin: &str, json: &str, horodatage: i64) {
    if let Ok(conn) = ouvrir_chemin(chemin) {
        let _ = conn.execute(
            "INSERT INTO rates_snapshot (id, json, a) VALUES (1, ?1, ?2)
             ON CONFLICT(id) DO UPDATE SET json = excluded.json, a = excluded.a",
            params![json, horodatage],
        );
    }
}

pub fn charger_snapshot() -> Option<Value> {
    charger_snapshot_dans(&chemin_db())
}

pub fn charger_snapshot_dans(chemin: &str) -> Option<Value> {
    let conn = ouvrir_chemin(chemin).ok()?;
    conn.query_row("SELECT json FROM rates_snapshot WHERE id = 1", [], |l| {
        l.get::<_, String>(0)
    })
    .ok()
    .and_then(|s| serde_json::from_str(&s).ok())
}

/// Dernière observation réellement journalisée pour une devise.
#[derive(Debug, Clone)]
pub struct ObservationPensee {
    pub devise: String,
    pub valeur: f64,
    pub horodatage: i64,
    pub source: String,
}

/// La mémoire longue du moteur : pour chaque devise, le dernier point tombé
/// dans `rates_history`. Quand le snapshot publié s'est amputé de devises
/// (blacklist d'immobilisme, redémarrage sur une base plus riche que la RAM),
/// c'est ici qu'on retrouve la dernière valeur vraiment vue au lieu de répondre
/// 404 au membre.
///
/// La requête balaye l'historique : le moteur l'appelle au plus une fois par
/// fenêtre de rejeu, pas à chaque tick.
pub fn dernieres_observations() -> Vec<ObservationPensee> {
    dernieres_observations_dans(&chemin_db())
}

pub fn dernieres_observations_dans(chemin: &str) -> Vec<ObservationPensee> {
    let conn = match ouvrir_chemin(chemin) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    // À timestamp égal, l'agrégat prime : c'est la valeur qui a été servie,
    // pas le dire d'une source déjà écartée par la médiane.
    let lignes = conn.prepare(
        "SELECT devise, rate, timestamp, source FROM (
             SELECT currency AS devise, rate, timestamp, source,
                    ROW_NUMBER() OVER (
                      PARTITION BY currency
                      ORDER BY timestamp DESC, (source = 'agregat') DESC
                    ) AS n
             FROM rates_history
         ) WHERE n = 1",
    );
    match lignes {
        Ok(mut r) => r
            .query_map([], |l| {
                Ok(ObservationPensee {
                    devise: l.get(0)?,
                    valeur: l.get(1)?,
                    horodatage: l.get(2)?,
                    source: l.get(3)?,
                })
            })
            .map(|it| it.filter_map(|p| p.ok()).collect())
            .unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

/// Le plus vieux point conservé : sert à dire honnêtement depuis quand
/// l'historique existe (il ne peut pas être reconstitué rétrospectivement).
pub fn premier_point() -> Option<i64> {
    let conn = ouvrir().ok()?;
    conn.query_row("SELECT MIN(timestamp) FROM rates_history", [], |l| {
        l.get::<_, Option<i64>>(0)
    })
    .ok()
    .flatten()
}

pub fn compter_lignes() -> i64 {
    ouvrir()
        .ok()
        .and_then(|c| c.query_row("SELECT COUNT(*) FROM rates_history", [], |l| l.get::<_, i64>(0)).ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SEQUENCE: AtomicUsize = AtomicUsize::new(0);

    /// Les tests écrivent dans le TMP du projet (contrainte d'exécution UQ)
    /// et chaque test prend son propre fichier, passé en argument : rien ne
    /// dépend d'une variable d'environnement partagée entre threads de test.
    ///
    /// Le nom est horodaté à la nanoseconde et le fichier existant est supprimé
    /// d'abord : un test qui a paniqué lors d'une session précédente laissait
    /// des lignes qui faisaient échouer la suivante.
    fn base_isolation(nom: &str) -> String {
        let n = SEQUENCE.fetch_add(1, Ordering::SeqCst);
        let nonce = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let dossier = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tmp_test");
        let _ = std::fs::create_dir_all(&dossier);
        let chemin = dossier
            .join(format!("test_rates_{}_{}_{}.db", nom, nonce, n))
            .to_string_lossy()
            .to_string();
        liberer(&chemin);
        chemin
    }

    fn liberer(chemin: &str) {
        let _ = std::fs::remove_file(chemin);
        let _ = std::fs::remove_file(format!("{}-wal", chemin));
        let _ = std::fs::remove_file(format!("{}-shm", chemin));
    }

    fn map_devise(entries: &[(&str, f64)]) -> HashMap<String, f64> {
        entries
            .iter()
            .map(|(c, v)| (c.to_string(), *v))
            .collect()
    }

    #[test]
    fn historique_relit_ce_qu_a_ete_ecrit() {
        let chemin = base_isolation("historique");
        let maintenant = chrono::Utc::now().timestamp();

        // L'agrégat sert 2309,26 ; er-api disait 2312 et la BCC 2301.
        // Les trois valeurs doivent être relisibles séparément.
        let rates = map_devise(&[("CDF", 2309.26), ("EUR", 0.895), ("USD", 1.0)]);
        let mut observations: HashMap<String, HashMap<String, f64>> = HashMap::new();
        observations.insert("er-api".to_string(), map_devise(&[("CDF", 2312.0)]));
        observations.insert("bcc".to_string(), map_devise(&[("CDF", 2301.0)]));

        let ecrits = enregistrer_dans(&chemin, maintenant, &rates, &observations).unwrap();
        assert_eq!(ecrits, 4, "USD exclu, CDF et EUR en agrégat + 2 observations");

        let points_agregat = points_dans(&chemin, "CDF", 3600, Some("agregat"));
        assert_eq!(points_agregat.len(), 1);
        assert_eq!(points_agregat[0]["rate"].as_f64().unwrap(), 2309.26);

        let tous = points_dans(&chemin, "CDF", 3600, None);
        assert_eq!(tous.len(), 3, "agregat + er-api + bcc");

        liberer(&chemin);
    }

    #[test]
    fn une_source_nenregistre_que_sa_propre_valeur() {
        // Le défaut corrigé ici : l'historique notait le nom d'une source à côté
        // de la valeur de la médiane. Une divergence par source lue là-dessus
        // serait fausse, et rien ne permettrait de le voir.
        let chemin = base_isolation("verite_source");
        let maintenant = chrono::Utc::now().timestamp();
        let rates = map_devise(&[("CDF", 2309.26)]);
        let mut observations: HashMap<String, HashMap<String, f64>> = HashMap::new();
        observations.insert("er-api".to_string(), map_devise(&[("CDF", 2312.0)]));

        enregistrer_dans(&chemin, maintenant, &rates, &observations).unwrap();

        let er_api = points_dans(&chemin, "CDF", 3600, Some("er-api"));
        assert_eq!(er_api.len(), 1);
        assert_eq!(
            er_api[0]["rate"].as_f64().unwrap(),
            2312.0,
            "la ligne er-api porte la valeur d'er-api, pas celle de l'agrégat"
        );
        liberer(&chemin);
    }

    #[test]
    fn le_point_usd_n_est_pas_enregistre() {
        // 1 USD = 1 USD n'est pas une observation de marché : l'écrire
        // fausserait les moyennes et le comptage de volume.
        let chemin = base_isolation("usd");
        let maintenant = chrono::Utc::now().timestamp();
        let rates = map_devise(&[("USD", 1.0), ("CDF", 2309.0)]);
        let mut observations: HashMap<String, HashMap<String, f64>> = HashMap::new();
        observations.insert("er-api".to_string(), map_devise(&[("USD", 1.0), ("CDF", 2309.0)]));
        enregistrer_dans(&chemin, maintenant, &rates, &observations).unwrap();
        assert!(points_dans(&chemin, "USD", 3600, None).is_empty(), "USD ne doit pas être journalisé");
        liberer(&chemin);
    }

    /// Ce que le moteur vient chercher dans la mémoire longue : UNE ligne par
    /// devise, la plus récente, et l'agrégat gagne à timestamp égal.
    #[test]
    fn la_memoire_longue_redonne_la_derniere_observation_par_devise() {
        let chemin = base_isolation("souvenir");
        let t_tardif = chrono::Utc::now().timestamp();
        let t_anctre = t_tardif - 7_200;

        let mut observations: HashMap<String, HashMap<String, f64>> = HashMap::new();
        observations.insert("er-api".to_string(), map_devise(&[("CDF", 2312.0)]));
        // Premier cycle : le forex avait répondu.
        enregistrer_dans(&chemin, t_anctre, &map_devise(&[("CDF", 2309.26)]), &observations).unwrap();
        // Cycle plus récent : seule la crypto a parlé.
        let mut crypto: HashMap<String, HashMap<String, f64>> = HashMap::new();
        crypto.insert("coingecko".to_string(), map_devise(&[("BTC", 77646.0)]));
        enregistrer_dans(chemin.as_str(), t_tardif, &map_devise(&[("BTC", 77646.0)]), &crypto).unwrap();

        let souvenir = dernieres_observations_dans(&chemin);
        let cdf = souvenir.iter().find(|p| p.devise == "CDF").expect("CDF retrouvé");
        assert_eq!(cdf.valeur, 2309.26, "la valeur servie, pas le dire d'er-api");
        assert_eq!(cdf.source, "agregat");
        assert_eq!(cdf.horodatage, t_anctre, "avec SA date, pas celle du cycle courant");

        let btc = souvenir.iter().find(|p| p.devise == "BTC").expect("BTC retrouvé");
        assert_eq!(btc.horodatage, t_tardif);

        assert!(
            !souvenir.iter().any(|p| p.devise == "USD"),
            "USD n'a jamais été journalisé : il ne peut pas ressortir d'un souvenir"
        );
        assert_eq!(souvenir.len(), 2, "une seule ligne par devise");
        liberer(&chemin);
    }

    #[test]
    fn snapshot_survit_au_processus() {
        let chemin = base_isolation("snapshot");
        enregistrer_snapshot_dans(&chemin, r#"{"rates":{"CDF":2309.26},"source":"mediane"}"#, 1_000);
        let relu = charger_snapshot_dans(&chemin).expect("snapshot relisible");
        assert_eq!(relu["rates"]["CDF"].as_f64(), Some(2309.26));
        liberer(&chemin);
    }
}
