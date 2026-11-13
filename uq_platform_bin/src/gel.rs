// ============================================================================
// gel.rs — Gel gradué des comptes (7 niveaux)
// ============================================================================
//
// Le gel touche les activités sensibles en premier, puis s'étend selon la
// gravité de la faute. Toujours autorisé à tous les niveaux : dépôt,
// consultation du solde et du jugement, invitations réseau, contestation.
//
//   Niveau 2 — gele_financier_leger : envoyer, retirer            (24 h)
//   Niveau 3 — gele_financier        : + trader, wallet           (3 j)
//   Niveau 4 — gele_commercial       : + vendre, acheter          (7 j)
//   Niveau 5 — gele_social           : + boutique, interagir      (14 j)
//   Niveau 6 — gele_continu          : tout sauf exceptions       (1 an)
//   Niveau 7 — gele_definitif        : tout sauf exceptions, + signalement
//
// Les blocages par réputation (est_bloque) restent évalués en plus du gel :
// un compte peut être gelé sans faute de réputation et inversement.

use rusqlite::{params, Connection};

use crate::database::Database;

// ---------------------------------------------------------------------------
// Constantes
// ---------------------------------------------------------------------------

/// Niveaux de gel applicables (1 = avertissement simple, sans gel).
pub const GEL_NIVEAUX: [i64; 6] = [2, 3, 4, 5, 6, 7];

/// Durées par défaut par niveau (secondes).
pub const DUREE_DEFAUT: [i64; 6] = [
    24 * 3600,        // 2 — 24 h
    3 * 24 * 3600,    // 3 — 3 jours
    7 * 24 * 3600,    // 4 — 7 jours
    14 * 24 * 3600,   // 5 — 14 jours
    365 * 24 * 3600,  // 6 — 1 an (gel long, contestable publiquement)
    i64::MAX / 2,     // 7 — definitif (jamais atteint)
];

// ---------------------------------------------------------------------------
// Connexion + schéma idempotent
// ---------------------------------------------------------------------------

type Conn = r2d2::PooledConnection<r2d2_sqlite::SqliteConnectionManager>;

pub fn ensure_tables(conn: &Connection) -> Result<(), String> {
    // Colonnes de gel sur accounts (ALTER idempotent : ignoré si présentes).
    for ddl in [
        "ALTER TABLE accounts ADD COLUMN statut TEXT NOT NULL DEFAULT 'actif'",
        "ALTER TABLE accounts ADD COLUMN gel_jusqu_a INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE accounts ADD COLUMN gel_motif TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE accounts ADD COLUMN gel_niveau INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE accounts ADD COLUMN gel_definitif INTEGER NOT NULL DEFAULT 0",
    ] {
        let _ = conn.execute(ddl, []);
    }
    // Le schéma accounts peut exister sans uq_number selon la version de la
    // base : on l'ajoute et on le remplit depuis `id` (qui porte l'UQ number).
    let a_uq: bool = conn
        .prepare("PRAGMA table_info(accounts)")
        .and_then(|mut s| {
            s.query_map([], |r| r.get::<_, String>(1))
                .map(|rows| rows.filter_map(|r| r.ok()).any(|c| c == "uq_number"))
        })
        .unwrap_or(false);
    if !a_uq {
        let _ = conn.execute("ALTER TABLE accounts ADD COLUMN uq_number TEXT", []);
        let _ = conn.execute(
            "UPDATE accounts SET uq_number = id WHERE uq_number IS NULL OR uq_number = ''",
            [],
        );
    }
    Ok(())
}

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
// Lecture de l'état de gel
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct EtatGel {
    pub statut: String,
    pub gel_jusqu_a: i64,
    pub gel_motif: String,
    pub gel_niveau: i64,
    pub gel_definitif: bool,
}

fn lire_etat(conn: &Connection, uq: &str) -> Result<EtatGel, String> {
    conn.query_row(
        "SELECT COALESCE(statut, 'actif'), COALESCE(gel_jusqu_a, 0),
                COALESCE(gel_motif, ''), COALESCE(gel_niveau, 0),
                COALESCE(gel_definitif, 0)
         FROM accounts WHERE uq_number = ?1",
        params![uq],
        |r| {
            Ok(EtatGel {
                statut: r.get(0)?,
                gel_jusqu_a: r.get(1)?,
                gel_motif: r.get(2)?,
                gel_niveau: r.get(3)?,
                gel_definitif: r.get::<_, i64>(4)? != 0,
            })
        },
    )
    .map_err(|_| "Compte introuvable".to_string())
}

/// true si le compte est actuellement sous gel (actif après expiration exclu).
#[allow(dead_code)] // API publique pour les futurs guards de routes
pub fn est_gele(uq: &str) -> bool {
    let Ok(conn) = db() else { return false };
    let Ok(etat) = lire_etat(&conn, uq) else { return false };
    en_cours(&etat)
}

fn en_cours(etat: &EtatGel) -> bool {
    etat.statut != "actif"
        && (etat.gel_definitif || etat.gel_jusqu_a > maintenant())
}

/// Réactive un compte dont le gel temporaire est expiré. Retourne true si
/// une remise à zéro a été nécessaire.
fn resynchroniser_si_expire(conn: &Connection, uq: &str, etat: &EtatGel) -> Result<bool, String> {
    if etat.statut == "actif" || etat.gel_definitif {
        return Ok(false);
    }
    if etat.gel_jusqu_a > 0 && maintenant() >= etat.gel_jusqu_a {
        conn.execute(
            "UPDATE accounts SET statut = 'actif', gel_jusqu_a = 0,
                    gel_niveau = 0, gel_motif = '' WHERE uq_number = ?1",
            params![uq],
        )
        .map_err(|e| e.to_string())?;
        return Ok(true);
    }
    Ok(false)
}

// ---------------------------------------------------------------------------
// Vérification d'action — le cœur du système
// ---------------------------------------------------------------------------

/// Actions canoniques du système de gel.
const ACTIONS_TOUJOURS_AUTORISEES: &[&str] = &[
    "deposer",
    "voir_jugement",
    "voir_solde",
    "consulter",
    "contester",
    "inviter",
];

/// Bloquées à partir du niveau 2.
const N2: &[&str] = &["envoyer", "retirer"];
/// Ajoutées au niveau 3.
const N3: &[&str] = &["trader", "wallet"];
/// Ajoutées au niveau 4.
const N4: &[&str] = &["vendre", "acheter", "livraison"];
/// Ajoutées au niveau 5.
const N5: &[&str] = &["boutique", "interagir"];
/// Niveaux 6 et 7 : tout est bloqué sauf les exceptions déjà filtrées
// (y compris 'voir_flux' — le fallback `true` du match suffit).

/// Normalise les alias d'actions utilisés par les différents modules
/// (marketplace, p2p, trading, chat) vers les actions canoniques.
fn canonique(action: &str) -> &str {
    match action {
        "transfer" | "transferer" | "envoyer_argent" | "p2p_envoyer" => "envoyer",
        "withdraw" | "retrait" | "retirer_fonds" => "retirer",
        "trade" | "trading" => "trader",
        "sell" | "marketplace_vendre" | "p2p_vendre" | "p2p_creer_offre" | "zando_vendre" => "vendre",
        "buy" | "marketplace_acheter" | "p2p_acheter" | "zando_acheter" => "acheter",
        "marketplace" => "vendre",
        "shop" | "zando_publier" | "publier_boutique" => "boutique",
        "chat" | "like" | "commenter" | "interaction" => "interagir",
        // Participer à une contestation = droit de recours, autorisé à tous
        // les niveaux (y compris 6-7 qui bloquent 'interagir').
        "contestation" | "contester" | "commenter_contestation" | "voter_contestation" => "contester",
        "feed" | "flux" | "lire" => "voir_flux",
        "solde" | "balance" | "compte" => "voir_solde",
        "litige" | "dossier" | "jugement" => "voir_jugement",
        _ => action,
    }
}

fn bloque_par_niveau(niveau: i64, action: &str) -> bool {
    let action = canonique(action);
    if ACTIONS_TOUJOURS_AUTORISEES.contains(&action) {
        return false;
    }
    match niveau {
        2 => N2.contains(&action),
        3 => N2.contains(&action) || N3.contains(&action),
        4 => N2.contains(&action) || N3.contains(&action) || N4.contains(&action),
        5 => N2.contains(&action) || N3.contains(&action) || N4.contains(&action) || N5.contains(&action),
        // Niveaux 6 et 7 : tout est bloqué sauf les exceptions déjà filtrées
        // et 'voir_flux' explicitement bloqué au niveau 6+.
        6 | 7 => true,
        _ => false,
    }
}

/// Le compte peut-il faire `action` ? Réactive le gel s'il est expiré.
/// `operation` peut aussi être une opération legacy ('p2p_vendre',
/// 'zando_publier', 'retrait', 'marketplace', 'livraison') : elle est
/// normalisée puis croisée avec `est_bloque` (réputation).
pub fn peut_faire(uq: &str, action: &str) -> Result<bool, String> {
    let conn = db()?;
    let etat = lire_etat(&conn, uq)?;
    resynchroniser_si_expire(&conn, uq, &etat)?;

    // 1. Gel gradué : si un gel est en cours, la table fait foi ; sinon tout
    // passe (un statut 'actif' ou un gel expiré déjà resynchronisé).
    if en_cours(&etat) && bloque_par_niveau(etat.gel_niveau, canonique(action)) {
        return Ok(false);
    }

    // 2. Blocages legacy de réputation (coexistent avec le gel).
    if crate::reputation_routes::est_bloque(&conn, uq, action) {
        return Ok(false);
    }

    Ok(true)
}

/// Variante qui renvoie un message d'erreur exploitable par les routes.
pub fn exiger(uq: &str, action: &str) -> Result<(), String> {
    if peut_faire(uq, action)? {
        Ok(())
    } else {
        Err(format!("Action '{}' bloquee pour ce compte (gel ou reputation)", action))
    }
}

// ---------------------------------------------------------------------------
// Application / levée du gel
// ---------------------------------------------------------------------------

fn statut_pour(niveau: i64) -> Result<&'static str, String> {
    Ok(match niveau {
        2 => "gele_financier_leger",
        3 => "gele_financier",
        4 => "gele_commercial",
        5 => "gele_social",
        6 => "gele_continu",
        7 => "gele_definitif",
        _ => return Err(format!("Niveau de gel invalide : {}", niveau)),
    })
}

/// Applique un gel de `niveau` (2-7) pour `duree_secs` (0 = durée par défaut
/// du niveau ; niveau 7 = définitif). `motif` est conservé pour le jugement.
pub fn appliquer_gel(uq: &str, niveau: i64, motif: &str, duree_secs: i64) -> Result<(), String> {
    let statut = statut_pour(niveau)?;
    let conn = db()?;
    ensure_tables(&conn)?;

    let ts = maintenant();
    let definitif = niveau == 7;
    let gel_jusqu_a = if definitif {
        i64::MAX / 2 // jamais atteint : pas de réactivation automatique
    } else {
        let duree = if duree_secs > 0 {
            duree_secs
        } else {
            DUREE_DEFAUT
                .iter()
                .zip(GEL_NIVEAUX.iter())
                .find(|(_, n)| **n == niveau)
                .map(|(d, _)| *d)
                .unwrap_or(24 * 3600)
        };
        ts + duree
    };

    let modifie = conn
        .execute(
            "UPDATE accounts SET statut = ?1, gel_jusqu_a = ?2, gel_motif = ?3,
                    gel_niveau = ?4, gel_definitif = ?5
             WHERE uq_number = ?6",
            params![statut, gel_jusqu_a, motif, niveau, definitif as i64, uq],
        )
        .map_err(|e| e.to_string())?;
    if modifie == 0 {
        return Err("Compte introuvable".to_string());
    }

    // Notification + journalisation légère.
    crate::trading_routes::push_notification(
        uq,
        "gel",
        &format!("Gel niveau {} appliqué", niveau),
        motif,
        &format!("gel:{}", ts),
    );
    Ok(())
}

/// Applique le niveau suivant après une récidive (2→3→…→7). Retourne le
/// niveau appliqué, ou 0 si le compte n'était pas gelé.
pub fn escalader(uq: &str, motif: &str) -> Result<i64, String> {
    let conn = db()?;
    let etat = lire_etat(&conn, uq)?;
    if etat.statut == "actif" {
        return Ok(0);
    }
    let suivant = (etat.gel_niveau + 1).min(7);
    if suivant == etat.gel_niveau {
        return Ok(etat.gel_niveau);
    }
    appliquer_gel(uq, suivant, motif, 0)?;
    Ok(suivant)
}

/// Réactive complètement un compte (jugement, contestation acceptée, admin).
pub fn degeler(uq: &str) -> Result<(), String> {
    let conn = db()?;
    ensure_tables(&conn)?;
    let modifie = conn
        .execute(
            "UPDATE accounts SET statut = 'actif', gel_jusqu_a = 0,
                    gel_motif = '', gel_niveau = 0, gel_definitif = 0
             WHERE uq_number = ?1",
            params![uq],
        )
        .map_err(|e| e.to_string())?;
    if modifie == 0 {
        return Err("Compte introuvable".to_string());
    }
    Ok(())
}

/// État courant pour les routes admin/compte.
pub fn etat(uq: &str) -> Result<serde_json::Value, String> {
    let conn = db()?;
    let etat = lire_etat(&conn, uq)?;
    let expire_bientot = !etat.gel_definitif
        && etat.gel_jusqu_a > 0
        && etat.gel_jusqu_a - maintenant() < 24 * 3600;
    Ok(serde_json::json!({
        "uq": uq,
        "statut": etat.statut,
        "gele": en_cours(&etat),
        "gel_niveau": etat.gel_niveau,
        "gel_motif": etat.gel_motif,
        "gel_jusqu_a": if etat.gel_definitif { serde_json::Value::Null } else { serde_json::json!(etat.gel_jusqu_a) },
        "gel_definitif": etat.gel_definitif,
        "expire_bientot": expire_bientot,
    }))
}

// ---------------------------------------------------------------------------
// Tests unitaires (logique pure)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toujours_autorise_aux_niveaux_hauts() {
        for niveau in [2, 3, 4, 5, 6, 7] {
            for action in ["deposer", "voir_jugement", "voir_solde", "consulter", "contester", "inviter"] {
                assert!(!bloque_par_niveau(niveau, action), "{} ne doit pas bloquer {}", niveau, action);
            }
        }
    }

    #[test]
    fn gradation_des_blocages() {
        // Niveau 2 : seules envoyer/retirer bloquées.
        assert!(bloque_par_niveau(2, "envoyer"));
        assert!(bloque_par_niveau(2, "retirer"));
        assert!(!bloque_par_niveau(2, "trader"));
        assert!(!bloque_par_niveau(2, "vendre"));
        assert!(!bloque_par_niveau(2, "interagir"));
        assert!(!bloque_par_niveau(2, "voir_flux"));

        // Niveau 3 : + trader/wallet.
        assert!(bloque_par_niveau(3, "trader"));
        assert!(bloque_par_niveau(3, "wallet"));
        assert!(!bloque_par_niveau(3, "vendre"));
        assert!(!bloque_par_niveau(3, "interagir"));

        // Niveau 4 : + vendre/acheter/livraison.
        assert!(bloque_par_niveau(4, "vendre"));
        assert!(bloque_par_niveau(4, "acheter"));
        assert!(!bloque_par_niveau(4, "interagir"));
        assert!(!bloque_par_niveau(4, "voir_flux"));

        // Niveau 5 : + boutique/interagir.
        assert!(bloque_par_niveau(5, "interagir"));
        assert!(bloque_par_niveau(5, "boutique"));
        assert!(!bloque_par_niveau(5, "voir_flux"));

        // Niveau 6-7 : tout bloqué sauf exceptions (voir_flux inclus).
        for niveau in [6, 7] {
            assert!(bloque_par_niveau(niveau, "voir_flux"));
            assert!(bloque_par_niveau(niveau, "envoyer"));
            assert!(bloque_par_niveau(niveau, "interagir"));
        }
    }

    #[test]
    fn alias_normalises() {
        assert_eq!(canonique("p2p_vendre"), "vendre");
        assert_eq!(canonique("marketplace_acheter"), "acheter");
        assert_eq!(canonique("retrait"), "retirer");
        assert_eq!(canonique("chat"), "interagir");
        assert_eq!(canonique("envoyer"), "envoyer");
    }
}
