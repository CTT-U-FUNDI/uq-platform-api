// ====================
// 🚰 ROBINET — autorisation et prélèvement continu
// ====================
//
// Un robinet est un flux qui sort du coffre sans geste à chaque tick. Deux choses
// le rendent légitime : une autorisation posée par un geste PIN, et un prélèvement
// qui ressemble trait pour trait à un frais de transfert. Ce module tient les deux
// pour que chaque source de robinet — présence d'une vitrine, lecteur de contenu
// payant, revente de connexion — utilise la même plomberie plutôt qu'une copie.
//
// Les autorisations vivent dans la table `settings` de la base du compte, au même
// titre que la langue ou le thème : elles suivent le compte, pas l'appareil.

use rusqlite::Connection;

use crate::api_handlers::{chemin_base_compte, tracer_ecritures_coffre, ContexteAppel, EcritureCoffre};

/// Clé `settings` : le robinet de présence des vitrines du compte.
pub const VITRINE: &str = "robinet_vitrine";

/// Sentinelle : le coffre n'a pas suivi. Les appelants la reconnaissent pour
/// fermer le robinet au lieu de retenter un prélèvement à chaque tick.
pub const SOLDE_INSUFFISANT: &str = "SOLDE_INSUFFISANT";

/// Deux écritures du même compte ? Les bases sont nommées en minuscules avec des
/// tirets du bas, et un identifiant arrive parfois tel que tapé.
pub fn meme_compte(premier: &str, second: &str) -> bool {
    chemin_base_compte(premier) == chemin_base_compte(second)
}

/// La base du compte est-elle là ? Sert à distinguer « le vendeur a coupé son
/// robinet » de « ce compte n'existe plus » dans les messages du tick.
pub fn compte_existe(uq: &str) -> bool {
    std::path::Path::new(&chemin_base_compte(uq)).exists()
}

/// L'état d'une autorisation de robinet, lu dans la base du compte.
///
/// Une clé absente vaut « ouvert » : l'interrupteur seul ne fait jamais partir
/// d'argent, chaque source exige en plus son propre geste PIN. Une base absente
/// vaut « fermé » — on ne facture pas un compte qu'on ne peut pas ouvrir.
pub fn autorise(uq: &str, cle: &str) -> bool {
    if !compte_existe(uq) {
        return false;
    }
    let chemin = chemin_base_compte(uq);
    let valeur = Connection::open(&chemin).and_then(|conn| {
        conn.query_row(
            "SELECT value FROM settings WHERE key = ?1",
            rusqlite::params![cle],
            |row| row.get::<_, String>(0),
        )
    });
    match valeur {
        Ok(valeur) => valeur != "false",
        // Clé jamais écrite : la valeur par défaut du robinet.
        Err(_) => true,
    }
}

/// Pose une autorisation dans la base du compte. Le tick relit cette valeur à
/// chaque tour : couper ici arrête le flux, sans effacer les boutiques.
///
/// La base doit exister : écrire une autorisation dans un compte qu'on ne peut
/// pas lire créerait un fichier vide à la place d'un refus.
pub fn autoriser(uq: &str, cle: &str, valeur: bool) -> Result<(), String> {
    if !compte_existe(uq) {
        return Err("Compte inaccessible".to_string());
    }
    let conn =
        Connection::open(chemin_base_compte(uq)).map_err(|e| format!("Compte inaccessible: {}", e))?;
    conn.execute(
        "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)",
        rusqlite::params![
            cle,
            if valeur { "true" } else { "false" },
            chrono::Utc::now().timestamp()
        ],
    )
    .map_err(|e| format!("Enregistrement de l'autorisation impossible: {}", e))?;
    Ok(())
}

/// Un prélèvement de robinet : ce qu'on sort, d'où il sort, et ce que la
/// comptabilité doit en dire.
pub struct Prelevement<'a> {
    pub uq: &'a str,
    pub points: i64,
    pub maintenant: i64,
    /// La catégorie de frais dans l'index de la plateforme : « boutique »,
    /// « lecture », « connexion ».
    pub source: &'a str,
    /// La ligne posée dans le grand livre du compte.
    pub tx_type: &'a str,
    pub contrepartie: &'a str,
    pub destinataire: &'a str,
    pub note: &'a str,
    pub reference: &'a str,
    /// Le robinet n'est jamais un geste humain : il signe avec le nom de sa
    /// boucle, pas avec un appareil.
    pub device: &'a str,
}

/// Débète le coffre, enregistre le frais dans l'index, puis pose le miroir
/// BlockOrb et la ligne de grand livre du compte — la chaîne exacte d'un frais de
/// transfert. Un robinet n'invente pas sa propre comptabilité.
pub fn prelever(p: &Prelevement) -> Result<(), String> {
    let chemin = chemin_base_compte(p.uq);
    let mut conn =
        Connection::open(crate::uq_index::uq_index_path()).map_err(|e| format!("Index inaccessible: {}", e))?;
    if conn
        .execute("ATTACH DATABASE ?1 AS user_account", rusqlite::params![chemin])
        .is_err()
    {
        return Err("Compte inaccessible".to_string());
    }

    let resultat = (|| -> Result<(), String> {
        crate::frais_plateforme::assurer_tables(&conn)
            .map_err(|e| format!("Initialisation des frais impossible: {}", e))?;
        let transaction = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| format!("Transaction impossible: {}", e))?;

        let change = transaction
            .execute(
                "UPDATE user_account.balance
                 SET points = points - ?1, updated_at = ?2
                 WHERE id = 1
                   AND points - COALESCE(frozen_points, 0) >= ?1",
                rusqlite::params![p.points, p.maintenant],
            )
            .map_err(|e| format!("Débit BORB impossible: {}", e))?;
        if change != 1 {
            // Aucun commit : le solde n'a pas bougé et rien n'a été tracé.
            return Err(SOLDE_INSUFFISANT.to_string());
        }

        crate::frais_plateforme::enregistrer_tx(
            &transaction,
            p.source,
            p.reference,
            p.uq,
            "BORB",
            p.points as f64 / 100.0,
            p.maintenant,
        )
        .map_err(|e| format!("Enregistrement des frais impossible: {}", e))?;

        transaction.commit().map_err(|e| e.to_string())
    })();

    let _ = conn.execute("DETACH DATABASE user_account", []);
    resultat?;

    // Les deux suites ne se posent qu'après le commit : un échec laisserait un
    // mouvement non tracé, jamais un mouvement inventé.
    if let Err(e) = crate::persistence::mirroir_frais(p.uq, p.points as u128) {
        eprintln!("⚠️  Miroir BlockOrb du robinet {} : {}", p.uq, e);
    }
    tracer_ecritures_coffre(
        &[EcritureCoffre {
            uq: p.uq,
            chemin_db: &chemin,
            tx_type: p.tx_type,
            points: p.points,
            frais_points: 0,
            contrepartie: p.contrepartie,
            note: p.note,
            reference: p.reference,
            emetteur: p.uq,
            destinataire: p.destinataire,
        }],
        p.maintenant,
        &ContexteAppel {
            device: p.device,
            ..Default::default()
        },
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn un_compte_sans_base_ne_facture_rien() {
        // Aucune base lue = aucune autorisation vérifiable : le robinet reste
        // fermé plutôt que de prélever à l'aveugle.
        assert!(!autorise("UQ-0000-0000-9999", VITRINE));
    }

    #[test]
    fn une_autorisation_ne_s_ecrit_pas_dans_un_compte_inexistant() {
        // La coupure depuis Profil › Paramètres doit rendre une erreur lisible
        // plutôt que de créer une base vide au passage.
        let erreur = autoriser("UQ-0000-0000-9999", VITRINE, false);
        assert_eq!(erreur.err().as_deref(), Some("Compte inaccessible"));
        assert!(!std::path::Path::new("data/users/uq_0000_0000_9999.db").exists());
    }
}
