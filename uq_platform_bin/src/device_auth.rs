// ====================
// 🔐 APPAREILS — liaison compte<->appareil et paliers de connexion
// ====================
//
// Regle metier decidee par le proprietaire :
//  * moins de 24 h qu'une connexion a reussi SUR CET APPAREIL -> reprise
//    silencieuse, aucune saisie ;
//  * entre 24 et 48 h -> numero UQ + validation biometrique ;
//  * au-dela de 48 h -> numero UQ + email + mot de passe, puis biometrie ;
//  * plafond : une re-authentication complete est forcee tous les 30 jours,
//    meme si l'appareil se connecte chaque jour. Sans ce plafond, un usage
//    quotidien ferait rester le compte au palier biometrie indefiniment.
//
// L'autorite n'est PAS un identifiant fabrique par l'app. Ce qui prouve
// l'appareil, c'est la cle WebAuthn nee dans l'enclave materielle (vTEE/TPM/
// Secure Enclave) et la signature qu'elle produit. La table porte donc la
// credential comme cle primaire ; le libelle de l'appareil n'est qu'une
// etiquette affichee dans « Mes appareils ».
//
// Les horodatages sont ecrits cote serveur : un client qui reprogrammerait son
// horloge ne gagne pas de palier, il perd juste la reprise silencieuse si
// l'ecart devient impossible.

use rusqlite::{params, Connection};
use serde_json::json;

/// Reprise silencieuse possible en dessous de cet age de derniere connexion.
pub const PALIER_AUTO_SECS: i64 = 24 * 3600;
/// Au-dela, la biometrie seule ne suffit plus : email et mot de passe reviennent.
pub const PALIER_COMPLET_SECS: i64 = 48 * 3600;
/// Une re-authentication complete doit arriver au plus tard a cet age de la
/// derniere authentification complete.
pub const PLAFOND_COMPLET_SECS: i64 = 30 * 24 * 3600;

/// Age rendu en francais courant, pour les messages destines au membre.
pub fn age_humain(secs: i64) -> String {
    if secs < 0 {
        return "inconnu".to_string();
    }
    if secs < 90 {
        return format!("{} seconde{}", secs, if secs > 1 { "s" } else { "" });
    }
    if secs < 36 * 3600 {
        return format!("{} minutes", ((secs + 30) / 60).max(1));
    }
    if secs < 48 * 3600 {
        return format!("{} heures", ((secs + 1800) / 3600).max(1));
    }
    let jours = ((secs + 43_200) / 86_400).max(1);
    if jours < 60 {
        return format!("{} jours", jours);
    }
    format!("{} mois", ((jours + 15) / 30).max(1))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Palier {
    /// Jeton encore valide et appareil vu il y a moins de 24 h.
    Auto,
    /// Numero UQ + signature de la cle de l'appareil.
    Biometrie,
    /// Numero UQ + email + mot de passe, puis cle de l'appareil.
    Complet,
    /// Aucun passkey enregistre pour ce compte : rien a verifier en enclave.
    AucunAppareil,
}

impl Palier {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Biometrie => "biometrie",
            Self::Complet => "complet",
            Self::AucunAppareil => "aucun_appareil",
        }
    }

    /// Explication courte montre a l'ecran : le membre doit comprendre pourquoi
    /// on lui redemande son mot de passe.
    pub fn motif(self, age_connexion: Option<i64>, age_complet: Option<i64>) -> String {
        match self {
            Self::Auto => match age_connexion {
                Some(age) => format!(
                    "Appareil reconnu : derniere connexion il y a {}.",
                    age_humain(age)
                ),
                None => "Appareil reconnu.".to_string(),
            },
            Self::Biometrie => match age_connexion {
                Some(age) => format!(
                    "Derniere connexion sur cet appareil il y a {} : la biométrie suffit.",
                    age_humain(age)
                ),
                None => "Validation biométrique requise.".to_string(),
            },
            Self::Complet => match (age_connexion, age_complet) {
                (Some(age), _) if age >= PALIER_COMPLET_SECS => format!(
                    "Plus aucune connexion sur cet appareil depuis {} : email et mot de passe exigés.",
                    age_humain(age)
                ),
                (_, Some(complet)) if complet >= PLAFOND_COMPLET_SECS => format!(
                    "La reconnexion complète est due : dernier contrôle complet il y a {}.",
                    age_humain(complet)
                ),
                _ => "Contrôle complet exigé pour ce compte.".to_string(),
            },
            Self::AucunAppareil => {
                "Aucune clé biométrique enregistrée sur ce compte.".to_string()
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct Appareil {
    pub credential_id: String,
    pub libelle: String,
    pub cree_le: i64,
    pub derniere_connexion: i64,
    pub dernier_controle_complet: i64,
    pub connexions: i64,
    pub revoque_le: Option<i64>,
}

impl Appareil {
    pub fn json(&self, maintenant: i64, activee: bool) -> serde_json::Value {
        json!({
            "credential_id": self.credential_id,
            "label": self.libelle,
            "enrolled_at": self.cree_le,
            "last_auth_at": self.derniere_connexion,
            "last_full_auth_at": self.dernier_controle_complet,
            "auth_count": self.connexions,
            "revoked_at": self.revoque_le,
            "active": activee,
            "since_last_auth_secs": if self.derniere_connexion > 0 {
                (maintenant - self.derniere_connexion).max(0)
            } else { -1 },
            "palier": palier_pour(self.derniere_connexion, self.dernier_controle_complet, maintenant).as_str(),
        })
    }
}

/// Calcul pur du palier — toute la regle 24 h / 48 h / 30 jours tient ici, pour
/// qu'elle soit testable sans base ni HTTP.
///
/// `derniere_connexion` = age de la derniere reussite de n'importe quel type ;
/// `dernier_complet` = age de la derniere reussite email + mot de passe.
pub fn palier_pour(derniere_connexion: i64, dernier_complet: i64, maintenant: i64) -> Palier {
    // Cle sans historique de connexion ET sans controle complet horodate : le
    // serveur ne peut rien garantir. Un enroulement normal horodate les deux ;
    // cette branche n'est atteinte que par une ligne heritee ou manuellement
    // inseree, et elle doit renvoyer au palier le plus strict.
    if derniere_connexion <= 0 && dernier_complet <= 0 {
        return Palier::Complet;
    }
    let age = match maintenant.checked_sub(derniere_connexion) {
        Some(age) if age >= 0 => age,
        _ => return Palier::Complet,
    };
    // Plafond de 30 jours : meme un appareil vu hier repasse au controle
    // complet si le dernier controle complet remonte trop loin.
    if dernier_complet > 0 && maintenant.saturating_sub(dernier_complet) >= PLAFOND_COMPLET_SECS {
        return Palier::Complet;
    }
    if dernier_complet <= 0 {
        return Palier::Complet;
    }
    if age < PALIER_AUTO_SECS {
        return Palier::Auto;
    }
    if age < PALIER_COMPLET_SECS {
        return Palier::Biometrie;
    }
    Palier::Complet
}

// ====================
// STOCKAGE (base du compte)
// ====================

pub fn chemin_base(uq: &str) -> String {
    format!("data/users/{}.db", uq.replace('-', "_").to_lowercase())
}

pub fn ouvrir(uq: &str) -> Result<Connection, String> {
    let chemin = chemin_base(uq);
    let conn =
        Connection::open(&chemin).map_err(|e| format!("Base du compte indisponible ({})", e))?;
    let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
    assurer_table(&conn)?;
    Ok(conn)
}

/// Table des appareils du compte. Elle vit dans la base du compte, au meme
/// endroit que `webauthn_credentials` : un compte deplace emporte ses appareils.
pub fn assurer_table(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS security_devices (
            credential_id TEXT PRIMARY KEY,
            device_label TEXT DEFAULT '',
            created_at INTEGER NOT NULL,
            last_auth_at INTEGER DEFAULT 0,
            last_full_auth_at INTEGER DEFAULT 0,
            auth_count INTEGER DEFAULT 0,
            revoked_at INTEGER
        );
        CREATE TABLE IF NOT EXISTS security_state (
            cle TEXT PRIMARY KEY,
            valeur INTEGER NOT NULL
        );",
    )
    .map_err(|e| format!("Table des appareils impossible ({})", e))
}

/// Fenetre pendant laquelle un enrolement de passkey peut se reclamer du controle
/// complet qui vient d'avoir lieu (email + mot de passe, ou PIN proprietaire).
pub const FENETRE_CONTROLE_COMPLET_SECS: i64 = 900;

const CLE_CONTROLE_COMPLET: &str = "last_full_auth_at";

/// Le serveur note qu'un controle complet vient de reussir pour CE compte.
/// Enregistrer un passkey juste apres doit beneficier de ce controle : sinon un
/// appareil tout juste enrole repartirait au palier complet a chaque connexion.
pub fn tracer_controle_complet(conn: &Connection, maintenant: i64) -> Result<(), String> {
    conn.execute(
        "INSERT INTO security_state (cle, valeur) VALUES (?1, ?2)
         ON CONFLICT(cle) DO UPDATE SET valeur = excluded.valeur",
        params![CLE_CONTROLE_COMPLET, maintenant],
    )
    .map_err(|e| format!("Horodatage du controle complet impossible ({})", e))?;
    Ok(())
}

pub fn dernier_controle_complet(conn: &Connection) -> Option<i64> {
    conn.query_row(
        "SELECT valeur FROM security_state WHERE cle = ?1",
        params![CLE_CONTROLE_COMPLET],
        |row| row.get::<_, i64>(0),
    )
    .ok()
}

/// Un controle complet vient-il de se produire ? Le client n'a aucun moyen de
/// tricher ici : la valeur est ecrite par le serveur dans finalize-*.
pub fn controle_complet_recent(conn: &Connection, maintenant: i64) -> bool {
    match dernier_controle_complet(conn) {
        Some(moment) => {
            let age = maintenant - moment;
            age >= 0 && age <= FENETRE_CONTROLE_COMPLET_SECS
        }
        None => false,
    }
}

/// Enregistrement d'une cle de l'appareil apres une verification reussie.
/// Reenregistrer la meme cle ne remet PAS les compteurs a zero : sinon effacer
/// puis rejouer l'enrolement ferait croire a un appareil fraichement complet.
pub fn enregistrer(
    conn: &Connection,
    credential_id: &str,
    libelle: &str,
    maintenant: i64,
) -> Result<(), String> {
    if credential_id.trim().is_empty() {
        return Err("Identifiant de cle absent".to_string());
    }
    conn.execute(
        "INSERT INTO security_devices (credential_id, device_label, created_at, last_auth_at, auth_count)
         VALUES (?1, ?2, ?3, ?3, 0)
         ON CONFLICT(credential_id) DO UPDATE SET device_label = COALESCE(NULLIF(?2, ''), device_label)",
        params![credential_id, libelle, maintenant],
    )
    .map_err(|e| format!("Enregistrement de l'appareil impossible ({})", e))?;
    Ok(())
}

/// Trace d'une connexion reussie. `complet = true` seulement quand email et mot
/// de passe ont ete controles (palier 3) : c'est ce qui relance l'horloge de
/// 30 jours.
pub fn tracer_connexion(
    conn: &Connection,
    credential_id: &str,
    complet: bool,
    maintenant: i64,
) -> Result<(), String> {
    if complet {
        conn.execute(
            "UPDATE security_devices
                SET last_auth_at = ?1, last_full_auth_at = ?1, auth_count = auth_count + 1,
                    revoked_at = NULL
              WHERE credential_id = ?2",
            params![maintenant, credential_id],
        )
    } else {
        conn.execute(
            "UPDATE security_devices
                SET last_auth_at = ?1, auth_count = auth_count + 1
              WHERE credential_id = ?2 AND revoked_at IS NULL",
            params![maintenant, credential_id],
        )
    }
    .map_err(|e| format!("Tracage de la connexion impossible ({})", e))?;
    Ok(())
}

pub fn lire(conn: &Connection, credential_id: &str) -> Option<Appareil> {
    conn.query_row(
        "SELECT credential_id, COALESCE(device_label,''), created_at,
                COALESCE(last_auth_at,0), COALESCE(last_full_auth_at,0),
                COALESCE(auth_count,0), revoked_at
         FROM security_devices WHERE credential_id = ?1",
        params![credential_id],
        |row| {
            Ok(Appareil {
                credential_id: row.get(0)?,
                libelle: row.get(1)?,
                cree_le: row.get(2)?,
                derniere_connexion: row.get(3)?,
                dernier_controle_complet: row.get(4)?,
                connexions: row.get(5)?,
                revoque_le: row.get::<_, Option<i64>>(6)?,
            })
        },
    )
    .ok()
}

/// Les clefs actives du compte, dans l'ordre d'enrolement le plus recent.
pub fn appareils_actifs(conn: &Connection) -> Vec<Appareil> {
    let mut attendu = Vec::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT credential_id, COALESCE(device_label,''), created_at,
                COALESCE(last_auth_at,0), COALESCE(last_full_auth_at,0),
                COALESCE(auth_count,0), revoked_at
         FROM security_devices WHERE revoked_at IS NULL ORDER BY created_at DESC",
    ) {
        if let Ok(rows) = stmt.query_map([], |row| {
            Ok(Appareil {
                credential_id: row.get(0)?,
                libelle: row.get(1)?,
                cree_le: row.get(2)?,
                derniere_connexion: row.get(3)?,
                dernier_controle_complet: row.get(4)?,
                connexions: row.get(5)?,
                revoque_le: row.get::<_, Option<i64>>(6)?,
            })
        }) {
            attendu.extend(rows.flatten());
        }
    }
    attendu
}

/// Le palier demande pour une cle donnee. `None` si la cle n'existe pas ou est
/// revoquee : dans ce cas le serveur ne peut rien signer depuis l'enclave de
/// cet appareil, il renvoie le palier complet.
pub fn palier_pour_credential(
    conn: &Connection,
    credential_id: &str,
    maintenant: i64,
) -> Option<(Appareil, Palier)> {
    let appareil = lire(conn, credential_id)?;
    if appareil.revoque_le.is_some() {
        return None;
    }
    let palier = palier_pour(
        appareil.derniere_connexion,
        appareil.dernier_controle_complet,
        maintenant,
    );
    Some((appareil, palier))
}

pub fn revoquer(conn: &Connection, credential_id: &str, maintenant: i64) -> Result<bool, String> {
    let touches = conn
        .execute(
            "UPDATE security_devices SET revoked_at = ?1
             WHERE credential_id = ?2 AND revoked_at IS NULL",
            params![maintenant, credential_id],
        )
        .map_err(|e| e.to_string())?;
    Ok(touches == 1)
}

// ====================
// TESTS
// ====================

#[cfg(test)]
mod tests {
    use super::*;

    const JOUR: i64 = 86_400;

    #[test]
    fn cle_sans_historique_redemande_le_controle_complet() {
        // Cle inconnue du serveur d'horodatage : aucune reprise douce, aucune
        // biometrie seule. Un enroulement normal horodate last_auth_at et
        // last_full_auth_at en meme temps, donc ce cas n'est que du residuel.
        assert_eq!(palier_pour(0, 0, 1_700_000_000), Palier::Complet);
        // Une cle dont la derniere connexion n'est pas horodatee reste au palier
        // strict, meme si un controle complet est connu par ailleurs.
        assert_eq!(
            palier_pour(0, 1_699_999_000, 1_700_000_000),
            Palier::Complet
        );
    }

    #[test]
    fn moins_de_24h_reprise_silencieuse() {
        let maint = 1_700_000_000;
        let dernier_complet = maint - 2 * JOUR;
        assert_eq!(
            palier_pour(maint - 60, dernier_complet, maint),
            Palier::Auto
        );
        // A 23 h 59 : toujours auto.
        assert_eq!(
            palier_pour(maint - (PALIER_AUTO_SECS - 1), dernier_complet, maint),
            Palier::Auto
        );
    }

    #[test]
    fn exactement_24h_plus_de_reprise_silencieuse() {
        let maint = 1_700_000_000;
        let complet = maint - 2 * JOUR;
        assert_eq!(
            palier_pour(maint - PALIER_AUTO_SECS, complet, maint),
            Palier::Biometrie
        );
    }

    #[test]
    fn entre_24_et_48h_biometrie_seule() {
        let maint = 1_700_000_000;
        let complet = maint - 3 * JOUR;
        assert_eq!(
            palier_pour(maint - 30 * 3600, complet, maint),
            Palier::Biometrie
        );
        assert_eq!(
            palier_pour(maint - (PALIER_COMPLET_SECS - 1), complet, maint),
            Palier::Biometrie
        );
    }

    #[test]
    fn au_dela_de_48h_email_et_mot_de_passe_reviennent() {
        let maint = 1_700_000_000;
        let complet = maint - 3 * JOUR;
        assert_eq!(
            palier_pour(maint - PALIER_COMPLET_SECS, complet, maint),
            Palier::Complet
        );
        assert_eq!(
            palier_pour(maint - 10 * JOUR, complet, maint),
            Palier::Complet
        );
    }

    #[test]
    fn plafond_30_jours_force_le_controle_complet_meme_a_chaque_jour() {
        let maint = 1_700_000_000;
        // Appareil vu il y a 1 h (donc < 24 h) mais dernier controle complet a
        // exactement 30 jours : le plafond gagne.
        assert_eq!(
            palier_pour(maint - 3600, maint - PLAFOND_COMPLET_SECS, maint),
            Palier::Complet
        );
        // A 29 jours et 23 h, la reprise silencieuse reste due.
        assert_eq!(
            palier_pour(maint - 3600, maint - (PLAFOND_COMPLET_SECS - 3600), maint),
            Palier::Auto
        );
    }

    #[test]
    fn jamais_de_controle_complet_connu_exige_l_etape_complete() {
        // Compte deplace sur un appareil enregistre avant l'arrivee du controle
        // complet : on repart du palier 3, pas de la biometrie.
        let maint = 1_700_000_000;
        assert_eq!(palier_pour(maint - 60, 0, maint), Palier::Complet);
    }

    #[test]
    fn horloge_du_client_avancee_ne_favorise_personne() {
        let maint = 1_700_000_000;
        // Connexion « future » : aucune reprise silencieuse la dessus.
        assert_eq!(
            palier_pour(maint + 500, maint + 500, maint),
            Palier::Complet
        );
    }

    #[test]
    fn table_et_enregistrement_relatif_a_la_cle() {
        let conn = Connection::open_in_memory().expect("base memoire");
        assurer_table(&conn).expect("table creee");
        let maint = 1_700_000_000;
        enregistrer(&conn, "cle-1", "PC", maint).expect("enregistre");
        tracer_connexion(&conn, "cle-1", true, maint).expect("trace");
        let (appareil, palier) = palier_pour_credential(&conn, "cle-1", maint).expect("cle connue");
        assert_eq!(palier, Palier::Auto);
        assert_eq!(appareil.dernier_controle_complet, maint);

        // Re-enrôlement de la même clé : les compteurs ne repartent pas à zéro.
        enregistrer(&conn, "cle-1", "Telephone", maint + 100).expect("re-enregistre");
        let relu = lire(&conn, "cle-1").expect("toujours la");
        assert_eq!(relu.derniere_connexion, maint);
        assert_eq!(relu.connexions, 1);
        assert_eq!(relu.libelle, "Telephone");
    }

    #[test]
    fn cle_revoquee_n_est_plus_acceptee() {
        let conn = Connection::open_in_memory().expect("base memoire");
        assurer_table(&conn).expect("table creee");
        let maint = 1_700_000_000;
        enregistrer(&conn, "cle-2", "PC", maint).expect("enregistre");
        tracer_connexion(&conn, "cle-2", true, maint).expect("trace");
        assert!(revoquer(&conn, "cle-2", maint + 10).expect("révoquée"));
        assert!(palier_pour_credential(&conn, "cle-2", maint + 20).is_none());
        // Deuxième révocation : rien de plus à faire.
        assert!(!revoquer(&conn, "cle-2", maint + 30).expect("déjà révoquée"));
        // Tracer une connexion sur une cle revoquee ne la reanime pas.
        tracer_connexion(&conn, "cle-2", false, maint + 40).expect("ignore");
        assert_eq!(lire(&conn, "cle-2").expect("la").derniere_connexion, maint);
    }

    #[test]
    fn le_controle_complet_recent_autorise_l_enroulement() {
        let conn = Connection::open_in_memory().expect("base memoire");
        assurer_table(&conn).expect("table creee");
        let maint = 1_700_000_000;

        // Rien d'horodate : l'enrolement ne peut pas se reclamer d'un controle complet.
        assert!(!controle_complet_recent(&conn, maint));
        assert_eq!(dernier_controle_complet(&conn), None);

        tracer_controle_complet(&conn, maint).expect("horodate");
        assert!(controle_complet_recent(&conn, maint));
        // Juste apres : toujours dans la fenetre.
        assert!(controle_complet_recent(
            &conn,
            maint + FENETRE_CONTROLE_COMPLET_SECS
        ));
        // Une de seconde de plus : la fenetre est fermee.
        assert!(!controle_complet_recent(
            &conn,
            maint + FENETRE_CONTROLE_COMPLET_SECS + 1
        ));

        // Un horodatage ecrit dans le futur n'est pas credité.
        tracer_controle_complet(&conn, maint + 10_000).expect("horodate futur");
        assert!(!controle_complet_recent(&conn, maint));
    }

    #[test]
    fn assurer_table_est_idempotent() {
        let conn = Connection::open_in_memory().expect("base memoire");
        assurer_table(&conn).expect("premiere creation");
        assurer_table(&conn).expect("seconde creation");
        enregistrer(&conn, "cle-3", "PC", 1_700_000_000).expect("enregistre");
        enregistrer(&conn, "cle-3", "PC", 1_700_000_001).expect("re-enregistre");
        assert_eq!(appareils_actifs(&conn).len(), 1);
    }
}
