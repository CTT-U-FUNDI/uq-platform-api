// ====================
// 🪟 VITRINES — le robinet de présence d'une boutique
// ====================
//
// Le frais de présence vaut 5 % de la valeur du catalogue publié pour un bloc
// de 24 h. Le robinet ne vend pas ce bloc d'avance : il le laisse couler
// seconde par seconde, et la monnaie sort du coffre à chaque tick.
//
// Le tick est la seule écriture d'argent de ce module. Il n'a pas de PIN sous
// la main — l'autorisation continuante est donnée par le geste PIN de
// `/api/shop/{id}/renew`, qui ouvre le robinet. C'est pour ça qu'un robinet
// fermé ne facture rien : `robinet = false` vaut « le vendeur n'a jamais
// autorisé le prélèvement ».
//
// Le compte garde le dernier mot : l'interrupteur `robinet_vitrine` de
// Profil › Paramètres est relu à chaque tick et coupe le flux sur-le-champ,
// sans effacer la boutique.
//
// Un solde qui ne suit pas ne détruit rien : la vitrine passe hors ligne, son
// compteur s'arrête à maintenant, et la boutique reste dans le compte.

use serde_json::json;

use crate::database::Database;
use crate::robinet::{self, Prelevement};
use crate::user_models::{Shop, DUREE_RENOUVELLEMENT_SECONDES, FRAIS_VITRINE_POURCENT};

/// Intervalle du tick, en secondes (`UQ_ROBINET_VITRINE_SECS`).
pub fn tick_secs() -> u64 {
    std::env::var("UQ_ROBINET_VITRINE_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|v: &u64| *v > 0)
        .unwrap_or(60)
}

/// Ce que le robinet doit, en points, pour une valeur de catalogue et un nombre
/// de secondes de présence. Les fractions de point ne sont pas arrondies
/// tick par tick : elles restent dans `secondes_en_attente` jusqu'à ce qu'elles
/// fassent un point entier.
pub fn points_dus(catalogue_points: i64, secondes: i64) -> i64 {
    if catalogue_points <= 0 || secondes <= 0 {
        return 0;
    }
    let dus = catalogue_points as i128 * FRAIS_VITRINE_POURCENT as i128 * secondes as i128
        / (100 * DUREE_RENOUVELLEMENT_SECONDES as i128);
    dus.max(0).min(i64::MAX as i128) as i64
}

/// Le taux de présence affiché au vendeur : points par seconde, pour qu'il
/// voie exactement ce que le robinet laisse couler.
pub fn tarif_par_seconde(catalogue_points: i64) -> f64 {
    points_dus(catalogue_points, DUREE_RENOUVELLEMENT_SECONDES) as f64
        / DUREE_RENOUVELLEMENT_SECONDES as f64
}

#[derive(Debug, Clone)]
pub struct ReglageVitrine {
    pub boutique: String,
    pub compte: String,
    pub secondes: i64,
    pub points: i64,
    /// La vitrine est passée hors ligne faute de solde.
    pub hors_ligne: bool,
    pub message: String,
}

/// Une vitrine dont le robinet est ouvert et la présence courante.
fn boutiques_en_robinet(db: &Database) -> Vec<Shop> {
    db.get_all_shops()
        .into_iter()
        .filter(|value| {
            value
                .get("robinet")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        })
        .filter_map(|value| serde_json::from_value::<Shop>(value).ok())
        .filter(|shop| !shop.owner.trim().is_empty() && !shop.id.trim().is_empty())
        .collect()
}

/// Ferme le robinet d'une vitrine sans rien purger : la présence s'arrête à
/// `maintenant`, la boutique reste dans le compte, et le flux ne reprend que par
/// un geste PIN. Sert quand le solde ne suit pas et quand le compte coupe son
/// robinet depuis Profil › Paramètres.
fn suspendre_vitrine(
    db: &Database,
    shop: &Shop,
    maintenant: i64,
    secondes: i64,
    points: i64,
    message: &str,
) -> ReglageVitrine {
    ecrire_compteurs(
        db,
        shop,
        &[
            ("en_ligne_jusqu_a", json!(maintenant)),
            ("robinet", json!(false)),
            ("dernier_reglement", json!(maintenant)),
            ("secondes_en_attente", json!(0)),
        ],
    );
    if !shop.vitrine_item_id.is_empty() {
        let _ = db.depublier_shop_item(&shop.vitrine_item_id, &shop.owner);
    }
    ReglageVitrine {
        boutique: shop.id.clone(),
        compte: shop.owner.clone(),
        secondes,
        points,
        hors_ligne: true,
        message: message.to_string(),
    }
}

/// Recale les compteurs du robinet sur la ligne réelle de la boutique : les
/// champs que le modèle ne connaît pas (boutique héritée, extensions du Studio)
/// restent en place.
fn ecrire_compteurs(db: &Database, shop: &Shop, reglages: &[(&str, serde_json::Value)]) {
    let mut boutique = db
        .get_shop_by_id(&shop.id)
        .unwrap_or_else(|| to_value(shop));
    for (cle, valeur) in reglages {
        boutique[cle] = valeur.clone();
    }
    db.save_publication(boutique);
}

/// Règle la présence d'une boutique. `None` quand il n'y a rien à faire :
/// robinet fermé, vitrine hors ligne, catalogue sans prix, ou secondes trop
/// courtes pour un point.
pub fn regler_boutique(db: &Database, shop: &Shop, maintenant: i64) -> Option<ReglageVitrine> {
    let secondes = shop.secondes_a_facturer(maintenant);
    if secondes <= 0 {
        return None;
    }

    // L'interrupteur du compte prime sur le robinet de la vitrine : le couper
    // depuis Profil › Paramètres arrête le flux sur-le-champ, boutique gardée.
    if !robinet::autorise(&shop.owner, robinet::VITRINE) {
        let message = if robinet::compte_existe(&shop.owner) {
            "Robinet coupé dans Profil › Paramètres"
        } else {
            "Compte introuvable : robinet suspendu"
        };
        return Some(suspendre_vitrine(
            db, shop, maintenant, secondes, 0, message,
        ));
    }

    let catalogue_points = db.valeur_catalogue(&shop.owner).max(0).saturating_mul(100);
    if catalogue_points <= 0 {
        // Catalogue sans prix = présence gratuite (école, église, agence).
        // Rien à accumuler, rien à écrire : le robinet tourne à vide.
        return None;
    }
    let secondes_cumulees = shop.secondes_en_attente.max(0).saturating_add(secondes);
    let points = points_dus(catalogue_points, secondes_cumulees);

    // Rien à prélever : on garde les secondes au compteur et on recale
    // l'horloge, pour que le tick suivant reparte d'ici et pas de plus loin.
    if points <= 0 {
        ecrire_compteurs(
            db,
            shop,
            &[
                ("dernier_reglement", json!(maintenant)),
                ("secondes_en_attente", json!(secondes_cumulees)),
            ],
        );
        return None;
    }

    let reference = format!("UQ-ROBINET-{}", maintenant);
    if let Err(erreur) = robinet::prelever(&Prelevement {
        uq: &shop.owner,
        points,
        maintenant,
        source: "boutique",
        tx_type: "fee",
        contrepartie: "Plateforme UQ",
        destinataire: "UQ-TRESOR-BOUTIQUE",
        note: &format!(
            "Robinet de présence · {} s · {} % du catalogue",
            secondes_cumulees, FRAIS_VITRINE_POURCENT
        ),
        reference: &reference,
        device: robinet::VITRINE,
    }) {
        if erreur != robinet::SOLDE_INSUFFISANT {
            // Une panne de base n'est pas un solde vide : l'horloge ne repart
            // pas, le tick suivant retentera ces mêmes secondes.
            return Some(ReglageVitrine {
                boutique: shop.id.clone(),
                compte: shop.owner.clone(),
                secondes,
                points,
                hors_ligne: false,
                message: erreur,
            });
        }
        // Le solde ne suit plus : la présence s'arrête là, sans purge. Le
        // robinet se ferme — le vendeur le rouvrira par un geste PIN, solde
        // rechargé. La fraction de seconde non payable n'est pas reportée.
        return Some(suspendre_vitrine(
            db,
            shop,
            maintenant,
            secondes,
            points,
            "Solde BORB insuffisant : vitrine passée hors ligne",
        ));
    }

    // Écriture validée : le compteur de la boutique repart du tick en cours.
    ecrire_compteurs(
        db,
        shop,
        &[
            ("dernier_reglement", json!(maintenant)),
            ("secondes_en_attente", json!(0)),
            (
                "regle_points",
                json!(shop.regle_points.saturating_add(points)),
            ),
        ],
    );

    Some(ReglageVitrine {
        boutique: shop.id.clone(),
        compte: shop.owner.clone(),
        secondes,
        points,
        hors_ligne: false,
        message: String::new(),
    })
}

/// Règle toutes les vitrines dont le robinet est ouvert. Retourne les vitrines
/// réellement prélevées ou passées hors ligne.
pub fn regler_vitrines(db: &Database, maintenant: i64) -> Vec<ReglageVitrine> {
    boutiques_en_robinet(db)
        .iter()
        .filter_map(|shop| regler_boutique(db, shop, maintenant))
        .collect()
}

/// Le vendeur vient de couper son robinet dans Profil › Paramètres : ses vitrines
/// en ligne s'arrêtent à `maintenant`, aucune purge, aucune donnée effacée. La
/// reprise passera par un geste PIN sur `/api/shop/{id}/renew`. Retourne les
/// boutiques réellement suspendues, pour que la réponse dise ce qui s'est passé.
pub fn suspendre_compte(db: &Database, uq: &str, maintenant: i64) -> Vec<String> {
    boutiques_en_robinet(db)
        .iter()
        .filter(|shop| robinet::meme_compte(&shop.owner, uq))
        .filter(|shop| shop.est_en_ligne(maintenant))
        .map(|shop| {
            suspendre_vitrine(
                db,
                shop,
                maintenant,
                shop.secondes_a_facturer(maintenant),
                0,
                "Robinet coupé dans Profil › Paramètres",
            )
            .boutique
        })
        .collect()
}

fn to_value(shop: &Shop) -> serde_json::Value {
    serde_json::to_value(shop).unwrap_or_else(|_| json!({}))
}

/// La boucle du robinet : un tick, un règlement, et on dort.
pub async fn boucle_robinet() {
    use tokio::time::{interval, Duration};
    let db = Database::shared("uq_platform_db.json");
    let mut ticker = interval(Duration::from_secs(tick_secs()));
    ticker.tick().await;
    loop {
        ticker.tick().await;
        let maintenant = chrono::Utc::now().timestamp();
        db.synchroniser_vitrines_expirees();
        for reglage in regler_vitrines(&db, maintenant) {
            if reglage.hors_ligne {
                eprintln!(
                    "🪟 robinet vitrine {} : {} points non payés → hors ligne ({})",
                    reglage.boutique, reglage.points, reglage.compte
                );
            } else if !reglage.message.is_empty() {
                eprintln!(
                    "⚠️  robinet vitrine {} : {} ({} points, {} s)",
                    reglage.boutique, reglage.message, reglage.points, reglage.secondes
                );
            } else {
                eprintln!(
                    "🪟 robinet vitrine {} : {} points prélevés pour {} s",
                    reglage.boutique, reglage.points, reglage.secondes
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::user_models::DUREE_VITRINE_SECONDES;

    #[test]
    fn cinq_pct_du_catalogue_pour_vingt_quatre_heures() {
        // 1 000 BORB de catalogue = 100 000 points ; 5 % = 5 000 points par jour.
        assert_eq!(points_dus(100_000, DUREE_RENOUVELLEMENT_SECONDES), 5_000);
        assert_eq!(points_dus(100_000, 43_200), 2_500);
    }

    #[test]
    fn un_catalogue_vide_ne_facture_rien() {
        assert_eq!(points_dus(0, DUREE_RENOUVELLEMENT_SECONDES), 0);
        assert_eq!(points_dus(-500, 60), 0);
        assert_eq!(points_dus(100_000, 0), 0);
    }

    #[test]
    fn le_tarif_par_seconde_est_le_frais_du_jour_divise() {
        let par_seconde = tarif_par_seconde(100_000);
        assert!((par_seconde - 0.05787).abs() < 1e-4);
    }

    #[test]
    fn secondes_facturees_cadrent_la_presence_reelle() {
        let mut shop = Shop::new("UQ-0000-0000-0000", "Atelier", "", "general", 0.0, 0.0);
        let maintenant = 1_800_000_000;
        shop.created_at = maintenant;
        shop.en_ligne_jusqu_a = maintenant + DUREE_VITRINE_SECONDES;

        // Robinet fermé : aucune seconde facturée.
        assert_eq!(shop.secondes_a_facturer(maintenant + 300), 0);

        shop.ouvrir_robinet(maintenant);
        assert_eq!(shop.secondes_a_facturer(maintenant + 300), 300);

        // Un rattrapage ne dépasse jamais une journée.
        assert_eq!(shop.secondes_a_facturer(maintenant + 3 * 86_400), 86_400);

        // Ni la présence qui n'a pas encore eu lieu : 90 s se sont écoulées
        // depuis le dernier règlement, mais la fenêtre se ferme à 30 s.
        shop.dernier_reglement = maintenant + DUREE_VITRINE_SECONDES - 120;
        shop.en_ligne_jusqu_a = maintenant + DUREE_VITRINE_SECONDES;
        assert_eq!(
            shop.secondes_a_facturer(maintenant + DUREE_VITRINE_SECONDES - 30),
            30
        );

        // Hors ligne : plus rien.
        shop.en_ligne_jusqu_a = maintenant;
        assert_eq!(shop.secondes_a_facturer(maintenant + 60), 0);
    }
}
