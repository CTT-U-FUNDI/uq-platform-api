// ====================
// 📡 ROUTES — Lecture du moteur par les clients
// ====================
//
//   GET /api/rates/live     → taux agrégés du dernier cycle + détail par devise
//   GET /api/rates/status   → santé des 11 sources, anomalies, volumétrie
//
// Principe : une route ne déclenche JAMAIS un appel fournisseur — elle lit
// l'état publié par le moteur. Et si le moteur n'a rien d'observable, elle
// répond 503 plutôt que d'inventer un nombre.
// ====================

use actix_web::{web, HttpResponse};
use serde_json::json;
use std::collections::HashMap;

use super::{etat, historique};

/// GET /api/rates/live?devises=CDF,EUR,XAU,BTC
pub async fn live(query: web::Query<HashMap<String, String>>) -> HttpResponse {
    let snapshot = match super::taux_ou_indisponible().await {
        Ok(s) => s,
        Err((code, statut, message)) => {
            return HttpResponse::build(
                actix_web::http::StatusCode::from_u16(code)
                    .unwrap_or(actix_web::http::StatusCode::SERVICE_UNAVAILABLE),
            )
            .json(json!({"status": "error", "code": statut, "message": message}));
        }
    };

    let filtrees: Vec<String> = query
        .get("devises")
        .map(|v| {
            v.split(',')
                .map(|s| s.trim().to_uppercase())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let maintenant = chrono::Utc::now().timestamp();
    let taux: serde_json::Map<String, serde_json::Value> = snapshot
        .rates
        .iter()
        .filter(|(devise, _)| filtrees.is_empty() || filtrees.contains(devise))
        .map(|(devise, valeur)| {
            let info = snapshot.informations.get(devise);
            let mut detail = json!({
                "taux": valeur,
                "source": info.map(|i| i.source.clone()).unwrap_or_default(),
                "origines": info.map(|i| i.origines.clone()).unwrap_or_default(),
                "mediane": info.and_then(|i| i.mediane),
                "ecart_pct": info.and_then(|i| i.ecart_pct),
                "officiel_bcc": info.and_then(|i| i.officiel_bcc),
            });
            // Date de l'observation réelle de CETTE devise : un cycle partiel
            // ressert le forex sans rajeunir sa date.
            if let Some(obj) = detail.as_object_mut() {
                obj.extend(fraicheur(info, snapshot.horodatage, maintenant));
            }
            (devise.clone(), detail)
        })
        .collect();

    HttpResponse::Ok().json(json!({
        "status": "success",
        "base": "USD",
        "source": snapshot.source,
        "badge": badge(Some(&snapshot.source)),
        "updated_at": snapshot.horodatage,
        "age_secs": snapshot.age_secs,
        "nb_sources": snapshot.nb_sources,
        // Devises resservées ce cycle depuis leur dernière observation : c'est
        // le nombre que le membre regarde pour savoir ce qui est « du jour ».
        "devises_rejouees": snapshot.devises_rejouees,
        "tick_secs": super::tick_secs(),
        "rates": taux,
    }))
}

/// GET /api/rates/status
pub async fn status() -> HttpResponse {
    let snapshot = etat::lire_snapshot();
    let sante = etat::sante();
    let anomalies = etat::anomalies();

    // Liste exhaustive des sources du moteur, y compris celles qu'aucune clé
    // n'a encore activées : l'admin doit pouvoir distinguer « mort » de « non configuré ».
    let declarees: Vec<&str> = super::sources::sources().iter().map(|s| s.nom).collect();
    let vues: Vec<serde_json::Value> = sante
        .iter()
        .map(|s| {
            json!({
                "source": s.nom,
                "categorie": s.categorie,
                "etat": s.etat,
                "dernier_succes": s.dernier_succes,
                "age_dernier_succes_secs": chrono::Utc::now().timestamp() - s.dernier_succes,
                "echecs_consecutifs": s.echecs_consecutifs,
                "latence_moyenne_ms": s.latence_moyenne_ms,
                "valeurs_identiques_de_suite": s.valeurs_identiques_de_suite,
                "suspecte_jusqu_a": s.suspecte_jusqu_a,
                "detail": s.detail,
            })
        })
        .collect();

    let manquantes: Vec<&str> = declarees
        .into_iter()
        .filter(|nom| !sante.iter().any(|s| s.nom == *nom))
        .collect();

    let indemne = sante
        .iter()
        .any(|s| s.etat == super::sante::EtatSource::Active);

    HttpResponse::Ok().json(json!({
        "status": if snapshot.as_ref().map(|s| !s.rates.is_empty()).unwrap_or(false) && indemne {
            "healthy"
        } else {
            "degrade"
        },
        "moteur": {
            "tick_secs": super::tick_secs(),
            "nb_devises": snapshot.as_ref().map(|s| s.rates.len()).unwrap_or(0),
            "source": snapshot.as_ref().map(|s| s.source.clone()).unwrap_or_default(),
            "horodatage": snapshot.as_ref().map(|s| s.horodatage).unwrap_or(0),
            "age_secs": snapshot.as_ref().map(|s| chrono::Utc::now().timestamp() - s.horodatage).unwrap_or(-1),
            "devises_rejouees": snapshot.as_ref().map(|s| s.devises_rejouees).unwrap_or(0),
            "devises_perimees": snapshot
                .as_ref()
                .map(|s| {
                    let maintenant = chrono::Utc::now().timestamp();
                    s.informations
                        .iter()
                        .filter(|(_, i)| i.observe_a > 0 && !super::agregation::est_fraiche(i, maintenant))
                        .count()
                })
                .unwrap_or(0),
        },
        "sources": vues,
        "sources_jamais_interrogees": manquantes,
        "anomalies": anomalies,
        "historique": {
            "lignes": historique::compter_lignes(),
            "premier_point": historique::premier_point(),
            "retention_jours": historique::RETENTION_JOURS,
        },
    }))
}

/// Badge de la source, tel que le moteur l'a étiquetée.
pub fn badge(source: Option<&str>) -> &'static str {
    match source {
        Some("bcc") => "Taux officiel BCC",
        Some("mediane") => "Taux agrégé (médiane)",
        Some("multi_sources") => "Taux agrégé (faible échantillon)",
        Some("definition") => "Par définition",
        Some("stale") => "Dernier taux connu (périmé)",
        Some("aucune") => "Aucune source",
        Some(nom) => {
            if nom.contains('+') {
                "Taux agrégé (faible échantillon)"
            } else {
                "Taux d'une source unique"
            }
        }
        None => "Aucune source",
    }
}

/// Badge d'UNE seule devise. La provenance dit d'où vient le nombre ; son âge
/// propre dit s'il décrit encore le marché.
///
/// Indispensable depuis que le moteur ressert le dernier taux capté : sans lui,
/// un CDF observé il y a trois jours porterait le même « Taux agrégé » qu'une
/// médiane calculée à la seconde, et le membre croirait acheter au cours du jour.
pub fn badge_de_fraiche(info: Option<&etat::InformationDevise>, maintenant: i64) -> &'static str {
    let Some(info) = info else {
        return "Aucune source";
    };
    // Étiquettes déjà portées par le moteur : il les connaît mieux que nous.
    if matches!(info.source.as_str(), "stale" | "aucune" | "definition") {
        return badge(Some(&info.source));
    }
    if info.observe_a <= 0 {
        return badge(Some(&info.source));
    }
    if maintenant - info.observe_a <= super::agregation::CADENCE_TEMPS_REEL_SECS {
        return badge(Some(&info.source));
    }
    if super::agregation::est_fraiche(info, maintenant) {
        "Dernier taux connu"
    } else {
        "Dernier taux connu (périmé)"
    }
}

/// Les trois champs de fraîcheur d'une devise, au même format pour toutes les
/// routes (`/api/rates/live` comme `/api/oracle/fiat`) : un front qui calcule
/// son propre âge ailleurs risquerait de dater une valeur du jour de la
/// réponse HTTP plutôt que de l'observation.
pub fn fraicheur(
    info: Option<&etat::InformationDevise>,
    horodatage_snapshot: i64,
    maintenant: i64,
) -> serde_json::Map<String, serde_json::Value> {
    let observe_a = info.map(|i| i.observe_a).unwrap_or(horodatage_snapshot);
    let mut champs = serde_json::Map::new();
    champs.insert("observe_a".to_string(), json!(observe_a));
    champs.insert(
        "age_secs".to_string(),
        json!((maintenant - observe_a).max(0)),
    );
    champs.insert(
        "badge".to_string(),
        json!(badge_de_fraiche(info, maintenant)),
    );
    champs
}
