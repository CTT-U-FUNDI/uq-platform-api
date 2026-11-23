// ============================================================================
// contestation.rs — Contestation publique des gels (niveaux 1 a 7)
// ============================================================================
//
// Un compte gelé (niveau 2-7) peut déposer des points pour ouvrir une
// contestation publique. La communauté commente et vote ; un job horaire
// réduit les sanctions selon le ratio de soutien.
//
//   Niveaux 2-5 : seuls les juges ayant un historique avec le fautif peuvent
//                 participer. Ratio = (votes 'seconde_chance' + commentaires
//                 'positif') / total signaux >= 80 % -> -1 niveau par semaine.
//   Niveaux 6-7 : les VOTANTS doivent avoir > 6 mois d'ancienneté et une
//                 réputation propre (vert>0, blanc>0, rouge=0, noir=0) ;
//                 les COMMENTATEURS sont ouverts à tous (sauf le fautif).
//                 Tous les commentaires sont comptes 'negatif'.
//                 Ratio = votes 'seconde_chance' / total votes sur 20 h.
//                 >= 50 % -> déblocage direct au niveau 5 (gele_social).
//
// Coût dynamique des commentaires (niveaux 6-7) : chaque vote
// 'gel_eternel' renforce la facture — cout = 1 + nb_gel_eternel / 10 points
// (soit 0,01 USD + 0,01 USD par tranche de 10 votes sévères). Le montant est
// prélevé sur le fonds du fautif et crédité au commentateur.
//
// Le dépôt du fautif finance les commentaires : chaque commentaire coûte
// COUT_COMMENTAIRE points au fonds et les credite au commentateur. Fonds
// épuisé -> la contestation disparaît (disparue = 1) et le gel reste.
//
// Unités : les montants sont en POINTS (balance_points du compte). Le mapping
// USD exact (0,01 USD/commentaire, dépôt max 100 USD) dépend du taux points<->USD
// de la plateforme : DEPOT_MAX_POINTS et COUT_COMMENTAIRE_POINTS sont les
// constantes à ajuster le jour où le taux est fixé.

use rusqlite::{params, Connection, OptionalExtension};

use crate::database::Database;

// ---------------------------------------------------------------------------
// Constantes
// ---------------------------------------------------------------------------

/// Dépôt maximum pour ouvrir/relancer une contestation (en points).
/// 1 point = 0,01 USD -> 10 000 points = 100 USD.
pub const DEPOT_MAX_POINTS: i64 = 10_000;
/// Coût de base d'un commentaire (niveaux 2-5, ou 6-7 sans vote sévère).
/// 1 point = 0,01 USD.
pub const COUT_COMMENTAIRE_POINTS: i64 = 1;
/// Ancienneté minimale pour voter sur une contestation de niveau 6-7.
pub const AGE_MIN_VOTE_SECS: i64 = 6 * 30 * 24 * 3600; // ~6 mois
/// Total de commentaires 'negatif' qui déclenche un gel continu 30 j.
pub const SEUIL_GEL_CONTINU_POINTS: i64 = 1000;
/// Fenêtre de calcul du ratio pour les niveaux 6-7 (secondes).
pub const FENETRE_67_SECS: i64 = 20 * 3600;
/// Seuil de soutien pour les niveaux 2-5 (pourcent).
pub const SEUIL_15: f64 = 80.0;
/// Seuil de soutien pour les niveaux 6-7 (pourcent).
pub const SEUIL_67: f64 = 50.0;
/// Délai minimum entre deux réductions de niveau (secondes) : 1 semaine.
pub const DELAI_REDUCTION_SECS: i64 = 7 * 24 * 3600;

// ---------------------------------------------------------------------------
// Connexion + schéma idempotent
// ---------------------------------------------------------------------------

type Conn = r2d2::PooledConnection<r2d2_sqlite::SqliteConnectionManager>;

pub fn ensure_tables(conn: &Connection) -> Result<(), String> {
    // Le gel et ses colonnes accounts (uq_number, statut...) sont garantis par
    // gel::ensure_tables — on l'appelle d'abord pour partager le même schéma.
    crate::gel::ensure_tables(conn)?;

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS contestations (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            uq_number TEXT NOT NULL,
            niveau_sanction_actuel INTEGER NOT NULL,
            motif TEXT NOT NULL DEFAULT '',
            fonds_restant_points INTEGER NOT NULL DEFAULT 0,
            total_negatifs_points INTEGER NOT NULL DEFAULT 0,
            ratio_soutien REAL NOT NULL DEFAULT 0,
            disparue INTEGER NOT NULL DEFAULT 0,
            statut TEXT NOT NULL DEFAULT 'en_cours',
            created_at INTEGER NOT NULL,
            derniere_reduction_at INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_contestations_uq ON contestations(uq_number, statut);
        CREATE TABLE IF NOT EXISTS contestation_commentaires (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            contestation_id INTEGER NOT NULL,
            auteur_uq TEXT NOT NULL,
            texte TEXT NOT NULL,
            type TEXT NOT NULL DEFAULT 'negatif',
            cout_points INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_contest_com ON contestation_commentaires(contestation_id);
        CREATE TABLE IF NOT EXISTS contestation_votes (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            contestation_id INTEGER NOT NULL,
            auteur_uq TEXT NOT NULL,
            vote TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            UNIQUE(contestation_id, auteur_uq)
        );
        CREATE INDEX IF NOT EXISTS idx_contest_votes ON contestation_votes(contestation_id);",
    )
    .map_err(|e| e.to_string())?;

    // Colonnes ajoutées après coup (ALTER idempotent).
    crate::database::Database::ajouter_colonne_si_absente(
        conn,
        "contestations",
        "nb_commentaires",
        "INTEGER NOT NULL DEFAULT 0",
    );
    crate::database::Database::ajouter_colonne_si_absente(
        conn,
        "contestations",
        "disparue_at",
        "INTEGER NOT NULL DEFAULT 0",
    );
    Ok(())
}

fn db() -> Result<Conn, String> {
    let conn = Database::shared("uq_platform_db.json")
        .conn()
        .map_err(|e| e.to_string())?;
    ensure_tables(&conn)?;
    Ok(conn)
}

fn maintenant() -> i64 {
    chrono::Utc::now().timestamp()
}

// ---------------------------------------------------------------------------
// Lecture
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Contestation {
    pub id: i64,
    pub uq_number: String,
    pub niveau: i64,
    pub motif: String,
    pub fonds_restant: i64,
    pub total_negatifs: i64,
    pub ratio: f64,
    pub disparue: bool,
    pub statut: String,
    pub created_at: i64,
    pub derniere_reduction_at: i64,
}

fn lire_contestation(conn: &Connection, id: i64) -> Result<Contestation, String> {
    conn.query_row(
        "SELECT id, uq_number, niveau_sanction_actuel, motif, fonds_restant_points,
                total_negatifs_points, ratio_soutien, disparue, statut, created_at,
                derniere_reduction_at
         FROM contestations WHERE id = ?1",
        params![id],
        |r| {
            Ok(Contestation {
                id: r.get(0)?,
                uq_number: r.get(1)?,
                niveau: r.get(2)?,
                motif: r.get(3)?,
                fonds_restant: r.get(4)?,
                total_negatifs: r.get(5)?,
                ratio: r.get(6)?,
                disparue: r.get::<_, i64>(7)? != 0,
                statut: r.get(8)?,
                created_at: r.get(9)?,
                derniere_reduction_at: r.get(10)?,
            })
        },
    )
    .optional()
    .map_err(|e| e.to_string())?
    .ok_or_else(|| "Contestation introuvable".to_string())
}

fn contestation_active_uq(conn: &Connection, uq: &str) -> Result<Option<i64>, String> {
    conn.query_row(
        "SELECT id FROM contestations WHERE uq_number = ?1 AND statut = 'en_cours'
           AND disparue = 0 ORDER BY id DESC LIMIT 1",
        params![uq],
        |r| r.get(0),
    )
    .optional()
    .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Ouverture (dépôt du fautif)
// ---------------------------------------------------------------------------

/// Ouvre une contestation pour `uq` (le demandeur = claims.sub, vérifié par la
/// route). `points` = montant déposé, borné à [1, DEPOT_MAX_POINTS].
/// Une seule contestation active par compte ; une contestation disparue peut
/// être relancée par un nouveau dépôt.
pub fn ouvrir(uq: &str, points: i64) -> Result<i64, String> {
    if points < 1 || points > DEPOT_MAX_POINTS {
        return Err(format!("Depot invalide : 1 a {} points", DEPOT_MAX_POINTS));
    }
    let conn = db()?;

    // Le compte doit être réellement gelé (niveau >= 2).
    let etat = crate::gel::etat(uq)?;
    let niveau = etat.get("gel_niveau").and_then(|v| v.as_i64()).unwrap_or(0);
    let gele = etat.get("gele").and_then(|v| v.as_bool()).unwrap_or(false);
    if !gele || niveau < 2 {
        return Err("Seul un compte gelé (niveau >= 2) peut contester".to_string());
    }

    if contestation_active_uq(&conn, uq)?.is_some() {
        return Err("Une contestation est deja en cours pour ce compte".to_string());
    }

    // Débit du dépôt sur balance_points (colonne du schéma accounts de base).
    let debit = conn
        .execute(
            "UPDATE accounts SET balance_points = COALESCE(balance_points, 0) - ?1
             WHERE uq_number = ?2 AND COALESCE(balance_points, 0) >= ?1",
            params![points, uq],
        )
        .map_err(|e| e.to_string())?;
    if debit == 0 {
        return Err("Solde de points insuffisant pour le depot".to_string());
    }

    let motif = crate::gel::etat(uq)?
        .get("gel_motif")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    conn.execute(
        "INSERT INTO contestations
            (uq_number, niveau_sanction_actuel, motif, fonds_restant_points,
             created_at, derniere_reduction_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 0)",
        params![uq, niveau, motif, points, maintenant()],
    )
    .map_err(|e| e.to_string())?;

    let id = conn.last_insert_rowid();
    crate::trading_routes::push_notification(
        uq,
        "contestation",
        "Contestation ouverte",
        &format!("Depot de {} points — la communaute peut juger", points),
        &format!("contestation:{}", id),
    );
    Ok(id)
}

// ---------------------------------------------------------------------------
// Éligibilité des juges
// ---------------------------------------------------------------------------

/// Historique marketplace entre deux comptes (hors annulations).
fn ont_historique(conn: &Connection, a: &str, b: &str) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM marketplace_transactions
         WHERE ((acheteur_uq = ?1 AND vendeur_uq = ?2)
             OR (acheteur_uq = ?2 AND vendeur_uq = ?1))
           AND COALESCE(statut, '') NOT IN ('annule', 'annulee', 'refuse')",
        params![a, b],
        |r| r.get::<_, i64>(0),
    )
    .map(|n| n > 0)
    .unwrap_or(false)
}

/// Activité minimale pour les COMMENTATEURS des niveaux 6-7 : au moins une
/// trace publique (réputation ou transaction marketplace). Évite les comptes
/// jetables sans empêcher « tout le monde » de commenter.
fn a_activite(conn: &Connection, uq: &str) -> bool {
    let rep: i64 = conn
        .query_row(
            "SELECT COALESCE(vert,0)+COALESCE(blanc,0)+COALESCE(orange,0)
                  +COALESCE(rouge,0)+COALESCE(noir,0)
             FROM reputation WHERE uq = ?1",
            params![uq],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if rep > 0 {
        return true;
    }
    conn.query_row(
        "SELECT COUNT(*) FROM marketplace_transactions
         WHERE acheteur_uq = ?1 OR vendeur_uq = ?1",
        params![uq],
        |r| r.get::<_, i64>(0),
    )
    .map(|n| n > 0)
    .unwrap_or(false)
}

/// Éligibilité VOTEUR pour les niveaux 6-7 : plus de 6 mois d'ancienneté ET
/// réputation propre (vert > 0, blanc > 0, rouge = 0, noir = 0).
/// `accounts.created_at` peut être un timestamp entier ou une date texte
/// selon le schéma de la base : lecture tolérante.
fn vote_eligible_67(conn: &Connection, uq: &str) -> bool {
    // Ancienneté : created_at en secondes (entier) ou parsé si texte ISO.
    let cree: Option<rusqlite::types::Value> = conn
        .query_row(
            "SELECT created_at FROM accounts WHERE uq_number = ?1",
            params![uq],
            |r| r.get(0),
        )
        .ok();
    let age_ok = match cree {
        Some(rusqlite::types::Value::Integer(ts)) => {
            // Some DBs stockent des millisecondes.
            let ts = if ts > 10_000_000_000 { ts / 1000 } else { ts };
            maintenant() - ts >= AGE_MIN_VOTE_SECS
        }
        Some(rusqlite::types::Value::Text(s)) => chrono::DateTime::parse_from_rfc3339(&s)
            .or_else(|_| {
                chrono::NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S")
                    .map(|n| n.and_utc().fixed_offset())
            })
            .map(|dt| maintenant() - dt.timestamp() >= AGE_MIN_VOTE_SECS)
            .unwrap_or(false),
        _ => false,
    };
    if !age_ok {
        return false;
    }
    // Réputation propre : au moins un vert et un blanc, aucun rouge ni noir.
    let propre: bool = conn
        .query_row(
            "SELECT COALESCE(vert,0) > 0 AND COALESCE(blanc,0) > 0
                    AND COALESCE(rouge,0) = 0 AND COALESCE(noir,0) = 0
             FROM reputation WHERE uq = ?1",
            params![uq],
            |r| r.get(0),
        )
        .unwrap_or(false);
    propre
}

/// Le compte `auteur` peut-il COMMENTER la contestation `c` ?
/// - Niveaux 6-7 : ouvert à tous les comptes avec une activité minimale.
/// - Niveaux 2-5 : historique marketplace avec le fautif requis.
pub fn peut_commenter(conn: &Connection, c: &Contestation, auteur: &str) -> bool {
    if auteur.eq_ignore_ascii_case(&c.uq_number) {
        return false; // le fautif ne commente pas sa propre contestation
    }
    if c.niveau >= 6 {
        a_activite(conn, auteur)
    } else {
        ont_historique(conn, auteur, &c.uq_number)
    }
}

/// Le compte `auteur` peut-il VOTER sur la contestation `c` ?
/// - Niveaux 6-7 : > 6 mois d'ancienneté + réputation propre.
/// - Niveaux 2-5 : historique marketplace avec le fautif requis.
pub fn peut_voter(conn: &Connection, c: &Contestation, auteur: &str) -> bool {
    if auteur.eq_ignore_ascii_case(&c.uq_number) {
        return false;
    }
    if c.niveau >= 6 {
        vote_eligible_67(conn, auteur)
    } else {
        ont_historique(conn, auteur, &c.uq_number)
    }
}

// ---------------------------------------------------------------------------
// Ratio de soutien
// ---------------------------------------------------------------------------

/// Calcule et persiste `ratio_soutien`.
/// - Niveaux 2-5 : (pour + commentaires positifs) / total signaux, tout temps.
/// - Niveaux 6-7 : votes 'pour' / total votes sur les 20 dernières heures.
fn recalculer_ratio(conn: &Connection, c: &Contestation) -> Result<f64, String> {
    let ratio = if c.niveau >= 6 {
        let depuis = maintenant() - FENETRE_67_SECS;
        let (pour, total): (i64, i64) = conn
            .query_row(
                "SELECT COALESCE(SUM(CASE WHEN vote='seconde_chance' THEN 1 ELSE 0 END),0), COUNT(*)
                 FROM contestation_votes
                 WHERE contestation_id = ?1 AND created_at >= ?2",
                params![c.id, depuis],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap_or((0, 0));
        if total > 0 {
            (pour as f64 / total as f64) * 100.0
        } else {
            0.0
        }
    } else {
        // Signaux = votes + commentaires. 'positif' penche pour le fautif.
        let (pour, contre): (i64, i64) = conn
            .query_row(
                "SELECT
                    (SELECT COUNT(*) FROM contestation_votes
                      WHERE contestation_id = ?1 AND vote = 'seconde_chance')
                  + (SELECT COUNT(*) FROM contestation_commentaires
                      WHERE contestation_id = ?1 AND type = 'positif'),
                    (SELECT COUNT(*) FROM contestation_votes
                      WHERE contestation_id = ?1 AND vote = 'gel_eternel')
                  + (SELECT COUNT(*) FROM contestation_commentaires
                      WHERE contestation_id = ?1 AND type = 'negatif')",
                params![c.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap_or((0, 0));
        let total = pour + contre;
        if total > 0 {
            (pour as f64 / total as f64) * 100.0
        } else {
            0.0
        }
    };

    conn.execute(
        "UPDATE contestations SET ratio_soutien = ?1 WHERE id = ?2",
        params![ratio, c.id],
    )
    .map_err(|e| e.to_string())?;
    Ok(ratio)
}

// ---------------------------------------------------------------------------
// Commentaires
// ---------------------------------------------------------------------------

/// Coût d'un commentaire en points. Niveaux 6-7 : chaque vote 'gel_eternel'
/// renforce la facture — cout = 1 + nb_gel_eternel / 10 (≈ +0,01 USD par
/// tranche de 10 votes sévères). Niveaux 2-5 : coût fixe.
fn cout_commentaire(conn: &Connection, c: &Contestation) -> i64 {
    if c.niveau < 6 {
        return COUT_COMMENTAIRE_POINTS;
    }
    let nb_gel: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM contestation_votes
             WHERE contestation_id = ?1 AND vote = 'gel_eternel'",
            params![c.id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    COUT_COMMENTAIRE_POINTS + nb_gel / 10
}

/// Ajoute un commentaire. Le coût est prélevé sur le fonds du fautif et
/// crédité au commentateur. Niveaux 6-7 : type forcé 'negatif' et coût
/// dynamique indexé sur les votes 'gel_eternel'.
pub fn commenter(c: &Contestation, auteur: &str, texte: &str) -> Result<(), String> {
    let texte = texte.trim();
    if texte.is_empty() || texte.len() > 280 {
        return Err("Commentaire vide ou trop long (280 caracteres max)".to_string());
    }
    let conn = db()?;
    let c = lire_contestation(&conn, c.id)?;
    if c.disparue || c.statut != "en_cours" {
        return Err("Contestation close".to_string());
    }
    if !peut_commenter(&conn, &c, auteur) {
        return Err("Tu ne peux pas commenter cette contestation".to_string());
    }
    let cout = cout_commentaire(&conn, &c);
    if c.fonds_restant < cout {
        // Fonds insuffisant : la contestation disparaît, le gel reste en cours.
        conn.execute(
            "UPDATE contestations SET disparue = 1, disparue_at = ?1 WHERE id = ?2",
            params![maintenant(), c.id],
        )
        .map_err(|e| e.to_string())?;
        return Err("Fonds de contestation epuise : la contestation disparait".to_string());
    }

    // Type effectif : 'negatif' forcé pour les niveaux 6-7 (présumé coupable).
    // Pour les niveaux 2-5, un préfixe '[+] ' marque un commentaire positif ;
    // sinon le commentaire est négatif par défaut.
    let type_com = if c.niveau < 6 && texte.starts_with("[+]") {
        "positif"
    } else {
        "negatif"
    };
    let texte_propre = texte.trim_start_matches("[+]").trim().to_string();

    // Débit du fonds du fautif + crédit du commentateur (balance_points).
    // Le solde du fautif n'est PAS redébité : le dépôt initial l'a déjà été.
    conn.execute(
        "UPDATE contestations SET fonds_restant_points = fonds_restant_points - ?1,
                total_negatifs_points = total_negatifs_points + ?2,
                nb_commentaires = nb_commentaires + 1
         WHERE id = ?3",
        params![cout, if type_com == "negatif" { cout } else { 0 }, c.id],
    )
    .map_err(|e| e.to_string())?;
    let _ = conn.execute(
        "UPDATE accounts SET balance_points = COALESCE(balance_points, 0) + ?1
         WHERE uq_number = ?2",
        params![cout, auteur],
    );

    conn.execute(
        "INSERT INTO contestation_commentaires
            (contestation_id, auteur_uq, texte, type, cout_points, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![c.id, auteur, texte_propre, type_com, cout, maintenant()],
    )
    .map_err(|e| e.to_string())?;

    // Fonds à zéro après ce commentaire -> disparition immédiate.
    let fonds: i64 = conn
        .query_row(
            "SELECT fonds_restant_points FROM contestations WHERE id = ?1",
            params![c.id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if fonds <= 0 {
        let _ = conn.execute(
            "UPDATE contestations SET disparue = 1, disparue_at = ?1 WHERE id = ?2",
            params![maintenant(), c.id],
        );
    }

    // Trop de commentaires négatifs financés -> gel continu (durée par défaut
    // du niveau 6 = 1 an).
    let total_neg: i64 = conn
        .query_row(
            "SELECT total_negatifs_points FROM contestations WHERE id = ?1",
            params![c.id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if total_neg >= SEUIL_GEL_CONTINU_POINTS && c.niveau < 6 {
        let _ = crate::gel::appliquer_gel(
            &c.uq_number,
            6,
            "Fonds de contestation epuise par les commentaires negatifs",
            0,
        );
    }

    let c2 = lire_contestation(&conn, c.id)?;
    recalculer_ratio(&conn, &c2)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Votes
// ---------------------------------------------------------------------------

/// Vote 'seconde_chance' (2ᵉ chance) ou 'gel_eternel'. Accepte aussi les
/// alias historiques 'pour'/'contre'. Un seul vote par auteur (modifiable).
/// Niveaux 6-7 : chaque 'gel_eternel' augmente le coût des commentaires.
pub fn voter(c: &Contestation, auteur: &str, vote: &str) -> Result<(), String> {
    let vote = match vote {
        "seconde_chance" | "pour" => "seconde_chance",
        "gel_eternel" | "contre" => "gel_eternel",
        _ => return Err("Vote invalide ('seconde_chance' ou 'gel_eternel')".to_string()),
    };
    let conn = db()?;
    let c = lire_contestation(&conn, c.id)?;
    if c.disparue || c.statut != "en_cours" {
        return Err("Contestation close".to_string());
    }
    if !peut_voter(&conn, &c, auteur) {
        return Err("Tu ne peux pas voter sur cette contestation".to_string());
    }
    conn.execute(
        "INSERT INTO contestation_votes (contestation_id, auteur_uq, vote, created_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(contestation_id, auteur_uq) DO UPDATE SET vote = ?3, created_at = ?4",
        params![c.id, auteur, vote, maintenant()],
    )
    .map_err(|e| e.to_string())?;
    recalculer_ratio(&conn, &c)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Job de réduction des sanctions
// ---------------------------------------------------------------------------

/// Réduction progressive des gels via les contestations actives.
/// Appelé par le job horaire (main.rs) et /api/marketplace/jobs.
/// Retourne la liste des actions effectuées.
pub fn reduire_sanctions() -> Result<Vec<String>, String> {
    let conn = db()?;
    let now = maintenant();

    let mut stmt = conn
        .prepare(
            "SELECT id FROM contestations
             WHERE statut = 'en_cours' AND disparue = 0",
        )
        .map_err(|e| e.to_string())?;
    let ids: Vec<i64> = stmt
        .query_map([], |r| r.get(0))
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();
    drop(stmt);

    let mut actions = Vec::new();

    for id in ids {
        let Ok(c) = lire_contestation(&conn, id) else {
            continue;
        };
        // Le niveau réel du compte fait foi (il a pu changer depuis l'ouverture).
        let etat = crate::gel::etat(&c.uq_number)
            .unwrap_or_else(|_| serde_json::json!({"gel_niveau": c.niveau, "gele": !c.disparue}));
        let niveau_reel = etat
            .get("gel_niveau")
            .and_then(|v| v.as_i64())
            .unwrap_or(c.niveau);
        let gele = etat.get("gele").and_then(|v| v.as_bool()).unwrap_or(false);

        if !gele {
            // Plus gelé : la contestation est close.
            let _ = conn.execute(
                "UPDATE contestations SET statut = 'clos' WHERE id = ?1",
                params![id],
            );
            actions.push(format!("contestation {} close (plus gelee)", id));
            continue;
        }

        let ratio = recalculer_ratio(&conn, &c)?;

        if niveau_reel >= 6 {
            // Niveaux 6-7 : fenêtre 20 h écoulée + ratio >= 50 % -> niveau 5.
            if c.created_at + FENETRE_67_SECS <= now && ratio >= SEUIL_67 {
                match crate::gel::appliquer_gel(
                    &c.uq_number,
                    5,
                    "Contestation : soutien communautaire >= 50%",
                    0,
                ) {
                    Ok(()) => {
                        let _ = conn.execute(
                            "UPDATE contestations SET niveau_sanction_actuel = 5,
                                    derniere_reduction_at = ?1, ratio_soutien = ?2
                             WHERE id = ?3",
                            params![now, ratio, id],
                        );
                        crate::trading_routes::push_notification(
                            &c.uq_number,
                            "contestation",
                            "Contestation reussie",
                            "Soutien >= 50% : gel reduit au niveau 5",
                            &format!("contestation:{}", id),
                        );
                        actions.push(format!("contestation {} : niveau 6/7 -> 5", id));
                    }
                    Err(e) => actions.push(format!("contestation {} : erreur gel5 ({})", id, e)),
                }
            }
        } else {
            // Niveaux 2-5 : ratio >= 80 % et 1 semaine écoulée -> -1 niveau.
            let reduction_ok = c.derniere_reduction_at == 0
                || now - c.derniere_reduction_at >= DELAI_REDUCTION_SECS;
            if ratio >= SEUIL_15 && reduction_ok {
                let suivant = (niveau_reel - 1).max(1);
                if suivant < 2 {
                    // Plus de gel : déblocage complet.
                    match crate::gel::degeler(&c.uq_number) {
                        Ok(()) => {
                            let _ = conn.execute(
                                "UPDATE contestations SET statut = 'clos',
                                        niveau_sanction_actuel = 0,
                                        derniere_reduction_at = ?1, ratio_soutien = ?2
                                 WHERE id = ?3",
                                params![now, ratio, id],
                            );
                            crate::trading_routes::push_notification(
                                &c.uq_number,
                                "contestation",
                                "Compte debloque",
                                "Soutien >= 80% : gel leve",
                                &format!("contestation:{}", id),
                            );
                            actions.push(format!("contestation {} : debloque", id));
                        }
                        Err(e) => {
                            actions.push(format!("contestation {} : erreur degel ({})", id, e))
                        }
                    }
                } else {
                    match crate::gel::appliquer_gel(
                        &c.uq_number,
                        suivant,
                        "Contestation : soutien communautaire >= 80%",
                        0,
                    ) {
                        Ok(()) => {
                            let _ = conn.execute(
                                "UPDATE contestations SET niveau_sanction_actuel = ?1,
                                        derniere_reduction_at = ?2, ratio_soutien = ?3
                                 WHERE id = ?4",
                                params![suivant, now, ratio, id],
                            );
                            actions.push(format!(
                                "contestation {} : niveau {} -> {}",
                                id, niveau_reel, suivant
                            ));
                        }
                        Err(e) => actions.push(format!("contestation {} : erreur gel ({})", id, e)),
                    }
                }
            }
        }
    }

    Ok(actions)
}

// ---------------------------------------------------------------------------
// Consultation publique
// ---------------------------------------------------------------------------

/// Récupère une contestation par id (pour les routes).
pub fn par_id(id: i64) -> Result<Contestation, String> {
    let conn = db()?;
    lire_contestation(&conn, id)
}

/// Détail JSON d'une contestation (pour GET /api/contestation/{id}).
pub fn detail(id: i64) -> Result<serde_json::Value, String> {
    let conn = db()?;
    let c = lire_contestation(&conn, id)?;

    let commentaires: Vec<serde_json::Value> = conn
        .prepare(
            "SELECT auteur_uq, texte, type, created_at FROM contestation_commentaires
             WHERE contestation_id = ?1 ORDER BY created_at ASC LIMIT 100",
        )
        .and_then(|mut s| {
            s.query_map(params![id], |r| {
                Ok(serde_json::json!({
                    "auteur": r.get::<_, String>(0)?,
                    "texte": r.get::<_, String>(1)?,
                    "type": r.get::<_, String>(2)?,
                    "created_at": r.get::<_, i64>(3)?,
                }))
            })
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
        })
        .unwrap_or_default();

    let (votes_pour, votes_contre, votes_total): (i64, i64, i64) = conn
        .query_row(
            "SELECT COALESCE(SUM(CASE WHEN vote='seconde_chance' THEN 1 ELSE 0 END),0),
                    COALESCE(SUM(CASE WHEN vote='gel_eternel' THEN 1 ELSE 0 END),0),
                    COUNT(*)
             FROM contestation_votes WHERE contestation_id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap_or((0, 0, 0));

    let _ = recalculer_ratio(&conn, &c);
    let cout_actuel = cout_commentaire(&conn, &c);

    Ok(serde_json::json!({
        "id": c.id,
        "uq_number": c.uq_number,
        "niveau_sanction": c.niveau,
        "motif": c.motif,
        "fonds_restant_points": c.fonds_restant,
        "total_negatifs_points": c.total_negatifs,
        "nb_commentaires": conn.query_row(
            "SELECT nb_commentaires FROM contestations WHERE id = ?1",
            params![id],
            |r| r.get::<_, i64>(0),
        ).unwrap_or(0),
        "cout_commentaire_points": cout_actuel,
        "ratio_soutien": c.ratio,
        "disparue": c.disparue,
        "statut": c.statut,
        "created_at": c.created_at,
        "votes": {
            "seconde_chance": votes_pour,
            "gel_eternel": votes_contre,
            "total": votes_total
        },
        "commentaires": commentaires,
        "regles": if c.niveau >= 6 {
            "Niveaux 6-7 : votants > 6 mois + reputation propre ; commentaires ouverts, tous negatifs ; chaque vote gel_eternel augmente le cout des commentaires ; ratio seconde_chance/total sur 20h ; debloquage >= 50% vers niveau 5"
        } else {
            "Niveaux 2-5 : historique requis, ratio >= 80%, -1 niveau par semaine"
        }
    }))
}

/// Liste des contestations actives (pour GET /api/contestation).
pub fn liste() -> Result<Vec<serde_json::Value>, String> {
    let conn = db()?;
    let mut stmt = conn
        .prepare(
            "SELECT id, uq_number, niveau_sanction_actuel, motif, ratio_soutien,
                    created_at
             FROM contestations
             WHERE statut = 'en_cours' AND disparue = 0
             ORDER BY created_at DESC LIMIT 50",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(serde_json::json!({
                "id": r.get::<_, i64>(0)?,
                "uq_number": r.get::<_, String>(1)?,
                "niveau_sanction": r.get::<_, i64>(2)?,
                "motif": r.get::<_, String>(3)?,
                "ratio_soutien": r.get::<_, f64>(4)?,
                "created_at": r.get::<_, i64>(5)?,
            }))
        })
        .map_err(|e| e.to_string())?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// Contestation active de l'utilisateur connecté (ou None).
pub fn ma_contestation(uq: &str) -> Result<Option<i64>, String> {
    let conn = db()?;
    contestation_active_uq(&conn, uq)
}
