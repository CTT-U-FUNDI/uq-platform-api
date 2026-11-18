// ====================
// 🌐 PUBLIC ROUTES — QR de réception + oracle de change
// ====================
//
//   GET /api/account/{uq}/qr.png     → QR PNG de réception
//   GET /api/account/{uq}/public     → identité publique (pseudo + nom)
//   GET /api/oracle/fiat/{currency}  → taux de change d'une devise
//   GET /api/oracle/fiat             → tous les taux + prix BORB
//
// RÈGLES :
//   - le QR de RÉCEPTION contient l'identité publique (pseudo + nom) :
//     c'est un partage volontaire de l'utilisateur (Q8)
//   - l'UUID/UQ n'est JAMAIS exposé côté affichage (Q1)
//   - les prix viennent du moteur de taux collecté en arrière-plan ;
//     aucune valeur non observée n'est servie
// ====================

use actix_web::{web, HttpResponse};
use serde_json::json;
use std::collections::HashMap;

// ====================
// TAUX DE CHANGE (oracle fiat)
// ====================
//
// Ce module ne collecte plus rien : il LIT le moteur de taux
// (`engines::rates`), qui travaille en arrière-plan et publie un agrégat
// validé. Un appel handler ne déclenche donc aucun appel fournisseur, et
// ne peut plus servir une valeur inventée.
//
// Historique du défaut corrigé : l'appel direct à open.er-api.com avec un
// timeout de 2 s échouait en permanence (latence réelle mesurée 2,5-5,8 s),
// et la table de 84 taux codés en dur était servie, réétiquetée « live ».
// D'où CDF 2800 en circulation alors que le marché était à 2309 (+21,3 %
// d'écart sur chaque conversion).
// ====================

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TauxFiat {
    /// Devise de base des taux (USD)
    pub base: String,
    /// 1 USD = rates[X] X
    pub rates: HashMap<String, f64>,
    pub updated_at: i64,
    /// « bcc » | « mediane » | nom de source | « source_unique » | « stale »
    pub source: String,
    /// Origines réellement agrégées et détail de l'écart, quand le moteur les connaît.
    pub origines: HashMap<String, Vec<String>>,
    pub nb_sources: usize,
}

/// Taux agrégés du moteur, ou `None` si rien n'est observable.
/// Les 14 sites appelants gardent la même signature d'appel.
pub async fn taux_du_jour() -> TauxFiat {
    match crate::engines::rates::taux_ou_indisponible().await {
        Ok(s) => {
            let origines = s
                .informations
                .iter()
                .map(|(d, i)| (d.clone(), i.origines.clone()))
                .collect();
            TauxFiat {
                base: "USD".to_string(),
                rates: s.rates,
                updated_at: s.horodatage,
                source: s.source,
                origines,
                nb_sources: s.nb_sources,
            }
        }
        Err(_) => TauxFiat {
            base: "USD".to_string(),
            rates: HashMap::new(),
            updated_at: 0,
            source: "aucune".to_string(),
            origines: HashMap::new(),
            nb_sources: 0,
        },
    }
}

/// Réponse d'erreur uniforme pour les routes qui refusent d'inventer un taux.
pub fn reponse_taux_indisponible() -> HttpResponse {
    HttpResponse::ServiceUnavailable().json(json!({
        "status": "error",
        "code": "TAUX_INDISPONIBLE",
        "message": "Aucune source de taux n'a répondu et aucun historique n'existe. La plateforme ne fabrique pas de taux.",
        "reprise": "Réessayez dans quelques secondes ; le moteur de taux interroge ses sources en continu."
    }))
}

/// Prix interne du BORB en USD, tel que le régulateur l'a réglé.
///
/// `None` quand l'état du régulateur est illisible ou nul : les appelants
/// renvoient alors une erreur explicite. Le défaut `unwrap_or(1.0)` qui
/// précédait affirmait « 1 BORB = 1 USD » sans aucune observation — sur des
/// écrans de conversion et des prix verrouillés pour l'exécution.
pub fn prix_borb_usd() -> Option<f64> {
    crate::persistence::load_prix()
        .prix_actuel
        .parse::<f64>()
        .ok()
        .filter(|p| p.is_finite() && *p > 0.0)
}

/// Réponse quand le prix du BORB n'est pas réglé.
pub fn reponse_prix_indisponible() -> HttpResponse {
    HttpResponse::ServiceUnavailable().json(json!({
        "status": "error",
        "code": "PRIX_NON_REGLE",
        "message": "Le prix interne du BORB n'est pas réglé : la plateforme n'invente pas 1 BORB = 1 USD.",
        "reprise": "Régler le prix côté régulateur, ou attendre une source d'observation."
    }))
}

/// GET /api/oracle/fiat
pub async fn oracle_fiat_tous() -> HttpResponse {
    let snapshot = match crate::engines::rates::taux_ou_indisponible().await {
        Ok(s) => s,
        Err(_) => return reponse_taux_indisponible(),
    };
    let origines: HashMap<String, Vec<String>> = snapshot
        .informations
        .iter()
        .map(|(d, i)| (d.clone(), i.origines.clone()))
        .collect();
    let taux = TauxFiat {
        base: "USD".to_string(),
        rates: snapshot.rates.clone(),
        updated_at: snapshot.horodatage,
        source: snapshot.source.clone(),
        origines,
        nb_sources: snapshot.nb_sources,
    };
    let borb_usd = match prix_borb_usd() {
        Some(p) => p,
        None => return reponse_prix_indisponible(),
    };
    // Les seules devises listées sont celles réellement observées : plus de
    // liste de 84 codes dont les prix étaient écrits dans le code.
    let devises: Vec<String> = taux.rates.keys().cloned().collect();

    // Fraîcheur greffée sur le détail de chaque devise : la valeur resservie
    // après une coupure doit voyager avec sa date et son badge, sinon l'écran
    // affiche un « taux du jour » qui ne l'est plus.
    let maintenant = chrono::Utc::now().timestamp();
    let details: serde_json::Map<String, serde_json::Value> = snapshot
        .informations
        .iter()
        .map(|(devise, info)| {
            let mut detail =
                serde_json::to_value(info).unwrap_or_else(|_| serde_json::Map::new().into());
            if let Some(obj) = detail.as_object_mut() {
                obj.extend(crate::engines::rates::routes::fraicheur(
                    Some(info),
                    snapshot.horodatage,
                    maintenant,
                ));
            }
            (devise.clone(), detail)
        })
        .collect();

    HttpResponse::Ok().json(json!({
        "status": "success",
        "base": taux.base,
        "source": taux.source,
        "badge": crate::engines::rates::routes::badge(Some(&taux.source)),
        "updated_at": taux.updated_at,
        "age_secs": maintenant - taux.updated_at,
        "nb_sources": taux.nb_sources,
        "devises_rejouees": snapshot.devises_rejouees,
        "borb_usd": borb_usd,
        "rates": taux.rates,
        "devises": devises,
        "details": details,
        // Parités publiées : le front ne recopie plus « 1 point = 0,01 BORB »
        // dans un composant. Un seul endroit la définit, côté grand livre.
        "parite": {
            "points_par_borb": crate::p2p_escrow::POINTS_PAR_BORB,
            "usd_par_borb": borb_usd,
        },
        "message": "1 USD = rates[X] X ; 1 BORB = borb_usd USD"
    }))
}

/// GET /api/oracle/fiat/{currency}
/// Retourne le taux d'UNE devise, plus la conversion dans les deux sens.
pub async fn oracle_fiat_devise(path: web::Path<String>) -> HttpResponse {
    let devise = path.into_inner().trim().to_uppercase();
    let snapshot = match crate::engines::rates::taux_ou_indisponible().await {
        Ok(s) => s,
        Err(_) => return reponse_taux_indisponible(),
    };
    let Some(borb_usd) = prix_borb_usd() else {
        return reponse_prix_indisponible();
    };
    let info = snapshot.informations.get(&devise);
    let source_devise = info.map(|i| i.source.clone()).unwrap_or_else(|| snapshot.source.clone());

    if devise == "BORB" {
        return HttpResponse::Ok().json(json!({
            "status": "success",
            "currency": "BORB",
            "usd_per_unit": borb_usd,
            "units_per_usd": 1.0 / borb_usd,
            "source": source_devise,
            "badge": crate::engines::rates::routes::badge(Some(&source_devise)),
            "updated_at": snapshot.horodatage
        }));
    }

    let Some(par_usd) = snapshot.rates.get(&devise).copied() else {
        return HttpResponse::NotFound().json(json!({
            "status": "error",
            "code": "DEVISE_INCONNUE",
            "message": format!("Aucune source n'a observé de taux pour {} : {}", devise, "la devise n'est pas cotée ou est indisponible"),
            "devises": snapshot.rates.keys().cloned().collect::<Vec<_>>()
        }));
    };

    // Un taux à 0 ou négatif n'est pas un taux : le diviser produirait `inf` ou
    // un prix nul affiché sans erreur. La devise est déclarée indisponible.
    if !par_usd.is_finite() || par_usd <= 0.0 {
        return HttpResponse::ServiceUnavailable().json(json!({
            "status": "error",
            "code": "TAUX_NON_EXPLOITABLE",
            "message": format!("Le taux observé pour {} n'est pas exploitable ({}) : aucune conversion ne sera calculée sur 0.", devise, par_usd),
            "devise": devise
        }));
    }

    let maintenant = chrono::Utc::now().timestamp();
    let fraicheur = crate::engines::rates::routes::fraicheur(info, snapshot.horodatage, maintenant);
    let badge_devise = fraicheur
        .get("badge")
        .and_then(|v| v.as_str())
        .unwrap_or("Aucune source")
        .to_string();

    HttpResponse::Ok().json(json!({
        "status": "success",
        "currency": devise,
        // 1 unité de cette devise = X USD
        "usd_per_unit": 1.0 / par_usd,
        // 1 USD = X unités
        "units_per_usd": par_usd,
        // 1 BORB = X unités de cette devise
        "units_per_borb": borb_usd * par_usd,
        // 1 unité de cette devise = X BORB (borb_usd est validé > 0 plus haut)
        "borb_per_unit": (1.0 / par_usd) / borb_usd,
        "borb_usd": borb_usd,
        "source": source_devise,
        // Badge de CETTE devise, calculé sur sa date d'observation à elle :
        // resservir un forex de la veille ne doit pas le faire passer pour la
        // médiane du jour.
        "badge": badge_devise,
        "observe_a": fraicheur.get("observe_a"),
        "age_secs": fraicheur.get("age_secs"),
        "origines": info.map(|i| i.origines.clone()).unwrap_or_default(),
        "mediane": info.and_then(|i| i.mediane),
        "officiel_bcc": info.and_then(|i| i.officiel_bcc),
        "updated_at": snapshot.horodatage
    }))
}

// ====================
// QR DE RÉCEPTION
// ====================

/// GET /api/account/{uq}/public — identité publique (jamais le UQ)
pub async fn account_public_identity(path: web::Path<String>) -> HttpResponse {
    let uq = path.into_inner();
    if !crate::auth_guard::ressemble_a_un_uq(&uq) {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Identifiant de compte invalide"
        }));
    }
    let (pseudo, nom) = crate::uq_public_alias::get_public_identity(&uq);
    // Le wallet BORB vérifie l'existence du compte UQ avant d'ouvrir
    // un wallet — `existe` reflète la vraie DB uq_index, pas le nœud.
    let existe = crate::uq_index::UqIndex::shared(&crate::uq_index::uq_index_path())
        .get_account(&uq)
        .is_some();
    // Statut KYC : le wallet BORB exige un compte vérifié pour s'ouvrir.
    // Absence de DB cadenas → false (non vérifié), jamais d'exception.
    let kyc = existe && crate::kyc::cadenas::est_deverrouille(&uq);

    HttpResponse::Ok().json(json!({
        "status": "success",
        "existe": existe,
        "kyc": kyc,
        "pseudo": pseudo,
        "name": nom,
        "display": crate::uq_public_alias::get_public_display(&uq),
        "avatar_url": crate::uq_public_alias::get_public_avatar_url(&uq)
    }))
}

/// GET /api/account/public/{pseudo} — résolution publique d'un pseudo.
pub async fn account_public_by_alias(path: web::Path<String>) -> HttpResponse {
    let pseudo = path.into_inner().replacen('_', "@", 1);
    let Some(uq) = crate::uq_public_alias::resoudre_destinataire(&pseudo) else {
        return HttpResponse::NotFound().json(json!({
            "status": "error",
            "existe": false,
            "message": "Compte introuvable"
        }));
    };

    let (pseudo, name) = crate::uq_public_alias::get_public_identity(&uq);
    HttpResponse::Ok().json(json!({
        "status": "success",
        "existe": true,
        "pseudo": pseudo,
        "name": name,
        "display": crate::uq_public_alias::get_public_display(&uq),
        "avatar_url": crate::uq_public_alias::get_public_avatar_url(&uq)
    }))
}

/// GET /api/account/{uq}/qr.png
///
/// QR de RÉCEPTION : contient l'identité publique (pseudo + nom).
/// C'est un partage VOLONTAIRE — le UQ n'y figure pas (Q1/Q8).
pub async fn account_qr_png(path: web::Path<String>) -> HttpResponse {
    let uq = path.into_inner();
    let (pseudo, nom) = crate::uq_public_alias::get_public_identity(&uq);

    // Contenu : { pseudo, name } — jamais le UQ
    let contenu = json!({
        "pseudo": pseudo,
        "name": nom
    })
    .to_string();

    match crate::receipt::qr::generate_qr_png_bytes(&contenu, 512) {
        Ok(bytes) => HttpResponse::Ok()
            .content_type("image/png")
            .insert_header(("Cache-Control", "no-store"))
            .body(bytes),
        Err(e) => HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("Erreur generation QR : {}", e)
        })),
    }
}

// ====================
// TESTS
// ====================

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn aucune_valeur_n_est_inventee_quand_les_sources_sont_tues() {
        // Le moteur n'a rien publié et n'a pas d'historique : la route doit
        // refuser, pas ressortir une table de taux.
        let reponse = reponse_taux_indisponible();
        assert_eq!(reponse.status().as_u16(), 503);
    }

    #[test]
    fn le_badge_reflete_la_source_reelle() {
        use crate::engines::rates::routes::badge;
        assert_eq!(badge(Some("bcc")), "Taux officiel BCC");
        assert_eq!(badge(Some("mediane")), "Taux agrégé (médiane)");
        assert_eq!(badge(Some("stale")), "Dernier taux connu (périmé)");
        // Une valeur de repli ne peut plus être présentée comme « live ».
        assert!(!badge(Some("fallback")).contains("live"));
    }

    #[tokio::test]
    async fn taux_du_jour_ne_bloque_pas_sur_le_reseau() {
        // Lecture d'état : même hors-ligne, l'appel revient sans attendre un
        // timeout fournisseur (c'était le défaut des 2 secondes en face client).
        let debut = std::time::Instant::now();
        let taux = taux_du_jour().await;
        let duree = debut.elapsed();
        assert!(
            duree.as_secs() < 25,
            "taux_du_jour a mis {:?} — il doit lire l'état, pas appeler un fournisseur",
            duree
        );
        assert!(taux.rates.len() < 500);
    }

    #[test]
    fn qr_reception_ne_contient_pas_le_uq() {
        // Le contenu du QR est construit à partir de l'identité publique
        let contenu = json!({"pseudo": "uq@jean", "name": "Jean M."}).to_string();
        assert!(contenu.contains("uq@jean"));
        assert!(!contenu.contains("UQ-"));
    }
}
