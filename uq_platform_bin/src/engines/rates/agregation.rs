// ====================
// 🧮 AGRÉGATION — Médiane multi-sources + détection d'anomalies
// ====================
//
// La médiane plutôt que la moyenne : une source manipulée ou en cache ne
// déplace pas le centre du nuage.
//
// Seuils (écart relatif à la médiane) :
//   > 15 %  → valeur rejetée, non comptée dans l'agrégat + anomalie journalisée
//   > 50 %  → alerte
//   > 100 % → suspicion de manipulation, remontée au bus de fraude
//
// Le cours officiel BCC est tenu À PART de la médiane temps réel : il est
// publié une fois par jour (08:10 UTC). Le mélanger à des ticks de seconde
// en seconde biaiserais l'agrégat ; il sert de RÉFÉRENCE officielle quand il
// est frais, et s'affiche avec son propre badge.
// ====================

use super::etat::{InformationDevise, SnapshotTaux};
use super::sante::SanteSource;
use super::sources::{Categorie, ReponseSource};
use serde_json::json;
use std::collections::HashMap;

/// Écart relatif au-delà duquel une valeur est écartée de la médiane.
pub const SEUIL_REJET: f64 = 0.15;
pub const SEUIL_ALERTE: f64 = 0.50;
pub const SEUIL_MANIPULATION: f64 = 1.00;

/// Nombre de lectures identiques consécutives faisant soupçonner un cache.
pub const SEUIL_IMMOBILITE: u32 = 6;
/// Durée du blacklistage temporaire d'une source suspecte.
pub const BLACKLIST_SECS: i64 = 900;

/// Fraîcheur maximale acceptée pour une source temps réel.
pub const CADENCE_TEMPS_REEL_SECS: i64 = 300;
/// La BCC publie quotidiennement : on tolère un peu plus qu'un cycle.
pub const CADENCE_BCC_SECS: i64 = 90_000;
/// Au-delà de cet âge, une valeur resservie n'est plus « le taux du jour » :
/// c'est le dernier taux connu, et le badge doit le dire. Ce n'est PAS une
/// fenêtre d'expiration : hors-ligne, le dernier taux réellement observé reste
/// servi jusqu'à la reconnexion, vieilli mais jamais remplacé par un nombre
/// inventé ni par une absence.
pub const CADENCE_REJEU_SECS: i64 = 86_400;

pub fn mediane(valeurs: &[f64]) -> Option<f64> {
    if valeurs.is_empty() {
        return None;
    }
    let mut v: Vec<f64> = valeurs.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let milieu = v.len() / 2;
    Some(if v.len() % 2 == 1 {
        v[milieu]
    } else {
        (v[milieu - 1] + v[milieu]) / 2.0
    })
}

/// Une valeur est-elle identique à celle du tick précédent (tolérance 1e-9 relatif) ?
fn valeur_identique(a: &HashMap<String, f64>, b: &HashMap<String, f64>) -> bool {
    let communs: Vec<&String> = a.keys().filter(|k| b.contains_key(*k)).collect();
    if communs.len() < 3 {
        return false;
    }
    communs.iter().all(|k| {
        let x = a[*k];
        let y = b[*k];
        (x - y).abs() <= x.abs() * 1e-9
    })
}

/// Construit l'agrégat à partir des réponses du cycle.
///
/// `precedentes` : les taux déjà connus par source au tick d'avant, pour
/// détecter l'immobilisme.
/// `sante_precedente` : santé connue au cycle précédent — c'est elle qui
/// porte le compteur d'immobilisme et le blacklistage en cours ; repartir de
/// zéro à chaque cycle rendrait le seuil d'immobilisme inatteignable.
///
/// La santé (succès, latence, pannes) est tenue par le collecteur, qui seul
/// connaît la latence réelle et les erreurs de transport. L'agrégation ne
/// fait que qualifier la COHÉRENCE des valeurs : immobilité, retard, écart.
pub fn agreger(
    reponses: Vec<ReponseSource>,
    precedentes: &HashMap<String, HashMap<String, f64>>,
    sante_precedente: &HashMap<String, SanteSource>,
    maintenant: i64,
) -> (SnapshotTaux, HashMap<String, SanteSource>, Vec<serde_json::Value>) {
    // Par devise : (source, valeur)
    let mut contributions: HashMap<String, Vec<(String, f64)>> = HashMap::new();
    // Instant de récolte par source : sert à dater chaque devise observée.
    let horodatage_source: HashMap<String, i64> = reponses
        .iter()
        .map(|r| (r.source.to_string(), r.recolle_a))
        .collect();
    let mut officiel_bcc: Option<f64> = None;
    let mut officiel_bcc_a: Option<i64> = None;
    let mut sante: HashMap<String, SanteSource> = HashMap::new();
    let mut anomalies: Vec<serde_json::Value> = Vec::new();

    let sante_pour = |reponse: &ReponseSource| -> SanteSource {
        if let Some(connue) = sante_precedente.get(reponse.source) {
            let mut s = connue.clone();
            s.categorie = reponse.categorie.cle();
            return s;
        }
        let mut s = SanteSource::nouvelle(reponse.source, reponse.categorie.cle());
        // La source vient de répondre : elle n'est pas « jamais interrogée ».
        s.etat = super::sante::EtatSource::Active;
        s.dernier_succes = reponse.recolle_a;
        s
    };

    // Marché fermé (week-end, jour férié) : toutes les sources redonnent la
    // dernière cotation en même temps. Ce n'est pas un cache fournisseur ; ce
    // qui soupçonne une source, c'est de rester immobile PENDANT que les other
    // bougent. Sans cette règle, le vendredi soir blacklistait tout le monde et
    // l'app n'avait plus aucun taux à afficher.
    let hors_banque_centrale: Vec<&ReponseSource> = reponses
        .iter()
        .filter(|r| r.categorie != Categorie::BanqueCentrale)
        .collect();
    let marche_immobile = hors_banque_centrale.len() >= 2
        && hors_banque_centrale.iter().all(|r| {
            precedentes
                .get(r.source)
                .map(|avant| valeur_identique(&r.taux, avant))
                .unwrap_or(false)
        });

    for reponse in &reponses {
        let etat_source = sante.entry(reponse.source.to_string()).or_insert_with(|| sante_pour(reponse));

        if reponse.categorie == Categorie::BanqueCentrale {
            // Référence officielle : hors médiane, cadence quotidienne.
            if maintenant - reponse.recolle_a <= CADENCE_BCC_SECS {
                if let Some(v) = reponse.taux.get("CDF") {
                    officiel_bcc = Some(*v);
                    officiel_bcc_a = Some(reponse.recolle_a);
                }
            }
            continue;
        }

        // Immobilisme : la source sert-elle la même valeur que la fois dernière ?
        if let Some(avant) = precedentes.get(reponse.source) {
            etat_source.noter_valeur(
                valeur_identique(&reponse.taux, avant),
                SEUIL_IMMOBILITE,
                BLACKLIST_SECS,
                maintenant,
            );
        }
        if etat_source.etat == super::sante::EtatSource::Suspecte {
            anomalies.push(json!({
                "type": "source_immeuble",
                "source": reponse.source,
                "detail": etat_source.detail,
                "a": maintenant,
            }));
            continue;
        }

        // Fraîcheur : une réponse de plus de 5 minutes n'est pas un direct.
        if maintenant - reponse.recolle_a > CADENCE_TEMPS_REEL_SECS {
            etat_source.etat = super::sante::EtatSource::Retardee;
            etat_source.detail = format!("donnée âgée de {} s", maintenant - reponse.recolle_a);
            anomalies.push(json!({
                "type": "source_en_retard",
                "source": reponse.source,
                "age_secs": maintenant - reponse.recolle_a,
                "a": maintenant,
            }));
            continue;
        }

        for (devise, valeur) in &reponse.taux {
            contributions
                .entry(devise.clone())
                .or_default()
                .push((reponse.source.to_string(), *valeur));
        }
    }

    let mut rates: HashMap<String, f64> = HashMap::new();
    let mut informations: HashMap<String, InformationDevise> = HashMap::new();

    for (devise, mut listes) in contributions {
        listes.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        let toutes: Vec<f64> = listes.iter().map(|(_, v)| *v).collect();
        let Some(med) = mediane(&toutes) else { continue };

        // Rejet des valeurs aberrantes, puis médiane recalculée sur le propre.
        let propres: Vec<(String, f64)> = listes
            .iter()
            .filter(|(_, v)| med <= 0.0 || (v - med).abs() / med <= SEUIL_REJET)
            .cloned()
            .collect();

        for (source, valeur) in &listes {
            if med > 0.0 {
                let ecart = (valeur - med).abs() / med;
                if ecart > SEUIL_REJET {
                    let niveau = if ecart > SEUIL_MANIPULATION {
                        "manipulation"
                    } else if ecart > SEUIL_ALERTE {
                        "alerte"
                    } else {
                        "rejet"
                    };
                    anomalies.push(json!({
                        "type": "valeur_aberrante",
                        "source": source,
                        "devise": devise,
                        "valeur": valeur,
                        "mediane": med,
                        "ecart_pct": ecart * 100.0,
                        "niveau": niveau,
                        "a": maintenant,
                    }));
                }
            }
        }

        if propres.is_empty() {
            continue;
        }
        let valeurs_propres: Vec<f64> = propres.iter().map(|(_, v)| *v).collect();
        let med_propre = mediane(&valeurs_propres).unwrap_or(med);
        let origines: Vec<String> = propres.iter().map(|(s, _)| s.clone()).collect();

        let (valeur, source_retenu) = if origines.len() >= 3 {
            (med_propre, "mediane".to_string())
        } else {
            // Une ou deux sources : on garde la médiane du petit échantillon,
            // mais c'est explicitement étiqueté — pas de fausse agrégation.
            (med_propre, origines.join("+"))
        };

        let ecart_pct = if origines.len() > 1 {
            let mini = valeurs_propres.iter().cloned().fold(f64::INFINITY, f64::min);
            let maxi = valeurs_propres.iter().cloned().fold(f64::MIN, f64::max);
            Some(if maxi > 0.0 { (maxi - mini) / maxi * 100.0 } else { 0.0 })
        } else {
            Some(0.0)
        };

        let bcc_pour_cette_devise = (devise == "CDF").then_some(officiel_bcc).flatten();
        let retenue = bcc_pour_cette_devise.unwrap_or(valeur);
        let source_finale = if bcc_pour_cette_devise.is_some() {
            "bcc".to_string()
        } else if origines.len() >= 3 {
            "mediane".to_string()
        } else {
            source_retenu
        };

        // Date d'observation retenue : la plus récente des sources qui ont
        // réellement nourri la valeur (la BCC pour le CDF officiel).
        let observe_a = if bcc_pour_cette_devise.is_some() {
            officiel_bcc_a.unwrap_or(maintenant)
        } else {
            origines
                .iter()
                .filter_map(|s| horodatage_source.get(s).copied())
                .max()
                .unwrap_or(maintenant)
        };

        rates.insert(devise.clone(), retenue);
        informations.insert(
            devise.clone(),
            InformationDevise {
                valeur: retenue,
                source: source_finale,
                origines,
                mediane: Some(med_propre),
                ecart_pct,
                officiel_bcc: bcc_pour_cette_devise,
                observe_a,
            },
        );
    }

    // USD par définition, pas par estimation : 1 USD = 1 USD.
    rates
        .entry("USD".to_string())
        .or_insert(1.0);
    informations.entry("USD".to_string()).or_insert(InformationDevise {
        valeur: 1.0,
        source: "definition".to_string(),
        origines: vec![],
        mediane: Some(1.0),
        ecart_pct: Some(0.0),
        officiel_bcc: None,
        observe_a: maintenant,
    });

    let nb_sources = reponses.len();
    let source_globale = if nb_sources == 0 {
        "aucune"
    } else if informations.values().any(|i| i.source == "mediane") {
        "mediane"
    } else if nb_sources >= 2 {
        // Plusieurs fournisseurs ont répondu, mais aucune devise n'a atteint
        // l'échantillon de trois : « médiane » serait un titre exagéré.
        "multi_sources"
    } else {
        "source_unique"
    };

    let snapshot = SnapshotTaux {
        rates,
        informations,
        horodatage: maintenant,
        source: source_globale.to_string(),
        age_secs: 0,
        nb_sources,
        devises_rejouees: 0,
    };

    (snapshot, sante, anomalies)
}

/// Last-known encore présentable : la valeur réellement observée, datée.
pub fn marquer_perime(mut snapshot: SnapshotTaux, maintenant: i64) -> SnapshotTaux {
    snapshot.source = "stale".to_string();
    snapshot.age_secs = maintenant - snapshot.horodatage;
    for info in snapshot.informations.values_mut() {
        info.source = "stale".to_string();
    }
    snapshot
}

/// Une valeur peut être resservie dès lors qu'elle a été RÉELLEMENT observée :
/// sa date d'observation est la preuve. Il n'existe aucune fenêtre après
/// laquelle on jetterait le dernier taux connu — un membre hors-ligne depuis
/// trois jours doit voir le taux d'avant la coupure, pas une absence ni un 1.0
/// par défaut. Ce qui borne le mensonge, c'est le badge et l'âge renvoyés.
pub fn encore_rejouable(info: &InformationDevise, _maintenant: i64) -> bool {
    info.observe_a > 0
}

/// Fenêtre au-delà de laquelle la valeur ne peut plus être présentée comme le
/// taux du jour. La BCC ne publie qu'une fois par jour : une devise qui ne
/// tient que de cette source a droit à sa cadence, les autres à la cadence
/// temps réel.
pub fn fenetre_de_fraicheur(info: &InformationDevise) -> i64 {
    if info.origines.len() == 1 && info.origines[0] == "bcc" {
        CADENCE_BCC_SECS
    } else {
        CADENCE_REJEU_SECS
    }
}

/// La devise a-t-elle encore une chance de décrire le marché d'aujourd'hui ?
pub fn est_fraiche(info: &InformationDevise, maintenant: i64) -> bool {
    info.observe_a > 0 && (maintenant - info.observe_a) <= fenetre_de_fraicheur(info)
}

/// Complète un cycle partiel avec les valeurs réellement observées plus tôt.
///
/// Une source en rythme lent (frankfurter 60 s, xaus 300 s, bcc 3600 s) ne
/// répond pas à tous les ticks : sans rejeu, le tick suivant publierait un
/// snapshot amnésique qui ne contient plus que les cryptos. Ce n'est pas « le
/// marché qui a disparu », c'est le moteur qui a oublié : la valeur doit
/// rester celle qui a été vue, avec sa date d'observation.
///
/// Ne rejoue que ce que le cycle courant n'a PAS observé : une valeur
/// réellement vue à l'instant gagne toujours contre une valeur resservie.
pub fn rejouer_connu(
    snapshot: &mut SnapshotTaux,
    connu: &SnapshotTaux,
    maintenant: i64,
) -> usize {
    let mut rejouees = 0usize;
    for (devise, info) in &connu.informations {
        if snapshot.rates.contains_key(devise) {
            continue;
        }
        if !encore_rejouable(info, maintenant) {
            continue;
        }
        snapshot.rates.insert(devise.clone(), info.valeur);
        snapshot.informations.insert(devise.clone(), info.clone());
        rejouees += 1;
    }
    // L'horodatage du snapshot reste la date de la dernière OBSERVATION du
    // cycle courant ; l'âge de chaque devise se lit dans `observe_a`.
    snapshot.age_secs = (maintenant - snapshot.horodatage).max(0);
    rejouees
}

/// Mémoire longue : les devises que le snapshot lui-même a perdues — parce
/// qu'une blacklist d'immobilisme les a fait tomber du dernier cycle publié, ou
/// parce que le moteur a redémarré sur une base plus riche que sa RAM — sont
/// resservies depuis leur dernière observation journalisée dans `rates_history`.
///
/// La valeur et la date viennent du disque, jamais d'une table de taux : une
/// devise que le moteur n'a jamais vue n'apparaît pas ici et la route fera 404.
pub fn rejouer_du_passe(
    snapshot: &mut SnapshotTaux,
    souvenir: &[super::historique::ObservationPensee],
    maintenant: i64,
) -> usize {
    let mut rejouees = 0usize;
    for point in souvenir {
        if snapshot.rates.contains_key(&point.devise) {
            continue;
        }
        let provenance = if point.source == "agregat" { "mediane" } else { &point.source };
        snapshot.rates.insert(point.devise.clone(), point.valeur);
        snapshot.informations.insert(
            point.devise.clone(),
            InformationDevise {
                valeur: point.valeur,
                // La provenance de la source qui a parlé en dernier est
                // conservée : « er-api » est plus informatif qu'un « stale »
                // collé sur toute la map, et l'âge dit déjà la vérité.
                source: provenance.to_string(),
                origines: vec![provenance.to_string()],
                mediane: None,
                ecart_pct: None,
                officiel_bcc: if point.source == "bcc" { Some(point.valeur) } else { None },
                observe_a: point.horodatage,
            },
        );
        rejouees += 1;
    }
    snapshot.age_secs = (maintenant - snapshot.horodatage).max(0);
    rejouees
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engines::rates::sources::Categorie;

    fn reponse(nom: &'static str, taux: &[(&str, f64)], age: i64) -> ReponseSource {
        ReponseSource {
            source: nom,
            categorie: Categorie::Forex,
            taux: taux.iter().map(|(c, v)| (c.to_string(), *v)).collect(),
            latence_ms: 100,
            recolle_a: chrono::Utc::now().timestamp() - age,
        }
    }

    #[test]
    fn mediane_ignore_une_source_manipulee() {
        let maintenant = chrono::Utc::now().timestamp();
        let reponses = vec![
            reponse("er-api", &[("CDF", 2300.0)], 0),
            reponse("frankfurter", &[("CDF", 2310.0)], 0),
            reponse("currencyapi", &[("CDF", 2305.0)], 0),
            reponse("fixer", &[("CDF", 9000.0)], 0), // aberrante
        ];
        let (snapshot, _, anomalies) = agreger(reponses, &HashMap::new(), &HashMap::new(), maintenant);
        let cdf = snapshot.rates.get("CDF").copied().expect("CDF présent");
        assert!((cdf - 2305.0).abs() < 20.0, "médiane attendue ~2305, obtenu {}", cdf);
        assert!(
            anomalies.iter().any(|a| a["type"] == "valeur_aberrante" && a["source"] == "fixer"),
            "la source aberrante doit être journalisée"
        );
    }

    /// L'immobilisme ne se déclare pas sur un cycle isolé : il faut N lectures
    /// consécutives, et la santé doit survivre d'un cycle à l'autre.
    #[test]
    fn source_immobile_est_blacklistee_apres_seuil_de_lectures() {
        let maintenant = chrono::Utc::now().timestamp();
        let taux_fixe: &[(&str, f64)] = &[("CDF", 2300.0), ("EUR", 0.9), ("XOF", 590.0)];

        let mut precedentes: HashMap<String, HashMap<String, f64>> = HashMap::new();
        let mut sante: HashMap<String, SanteSource> = HashMap::new();
        let mut dernier = SnapshotTaux {
            rates: HashMap::new(),
            informations: HashMap::new(),
            horodatage: maintenant,
            source: "aucune".into(),
            age_secs: 0,
            nb_sources: 0,
            devises_rejouees: 0,
        };

        for tick in 1..=(SEUIL_IMMOBILITE as usize + 1) {
            let reponses = vec![reponse("er-api", taux_fixe, 0)];
            let (snapshot, sante_cycle, _) = agreger(reponses.clone(), &precedentes, &sante, maintenant);
            for r in reponses {
                precedentes.insert(r.source.to_string(), r.taux);
            }
            for (n, m) in sante_cycle {
                sante.insert(n, m);
            }
            dernier = snapshot;

            if tick <= SEUIL_IMMOBILITE as usize {
                assert!(
                    dernier.rates.contains_key("CDF"),
                    "la source doit rester éligible tant que le seuil n'est pas atteint (tick {})",
                    tick
                );
            }
        }

        assert_eq!(
            sante.get("er-api").map(|s| s.etat),
            Some(super::super::sante::EtatSource::Suspecte),
            "seuil atteint : la source est soupçonnée de servir un cache"
        );
        assert!(
            !dernier.rates.contains_key("CDF"),
            "une source suspecte ne doit plus nourrir l'agrégat"
        );
    }

    #[test]
    fn une_source_mobile_reste_eligible() {
        let maintenant = chrono::Utc::now().timestamp();
        let mut precedentes: HashMap<String, HashMap<String, f64>> = HashMap::new();
        let mut sante: HashMap<String, SanteSource> = HashMap::new();
        let valeur = |v: f64| -> Vec<ReponseSource> { vec![reponse("er-api", &[("CDF", v)], 0)] };

        // Six cycles, mais des valeurs qui bougent : aucune suspicion.
        for i in 0..6 {
            let reponses = valeur(2300.0 + i as f64 * 2.0);
            let (snapshot, sante_cycle, _) = agreger(reponses.clone(), &precedentes, &sante, maintenant);
            for r in reponses {
                precedentes.insert(r.source.to_string(), r.taux);
            }
            for (n, m) in sante_cycle {
                sante.insert(n, m);
            }
            assert!(snapshot.rates.contains_key("CDF"), "un taux mouvant reste retenu (cycle {})", i);
        }
        assert_eq!(
            sante.get("er-api").map(|s| s.etat),
            Some(super::super::sante::EtatSource::Active)
        );
    }

    #[test]
    fn le_bcc_ne_se_melange_pas_a_la_mediane() {
        let maintenant = chrono::Utc::now().timestamp();
        let mut bcc = reponse("bcc", &[("CDF", 2261.56)], 0);
        bcc.categorie = Categorie::BanqueCentrale;
        let (snapshot, _, _) = agreger(
            vec![
                bcc,
                reponse("er-api", &[("CDF", 2309.0)], 0),
                reponse("frankfurter", &[("CDF", 2311.0)], 0),
                reponse("xaus", &[("CDF", 2310.0)], 0),
            ],
            &HashMap::new(),
            &HashMap::new(),
            maintenant,
        );
        let info = snapshot.informations.get("CDF").expect("CDF");
        // Le cours officiel est retenu pour CDF, la médiane temps réel reste visible.
        assert_eq!(info.source, "bcc");
        assert_eq!(info.officiel_bcc, Some(2261.56));
        assert!(
            (info.mediane.expect("mediane") - 2310.0).abs() < 1.0,
            "la médiane doit ignorer la BCC"
        );
    }

    #[test]
    fn une_seule_source_nest_pas_presentee_comme_agregat() {
        let maintenant = chrono::Utc::now().timestamp();
        let (snapshot, _, _) = agreger(
            vec![reponse("er-api", &[("CDF", 2309.0)], 0)],
            &HashMap::new(),
            &HashMap::new(),
            maintenant,
        );
        let info = snapshot.informations.get("CDF").expect("CDF");
        assert_eq!(info.source, "er-api");
        assert_ne!(snapshot.source, "mediane");
    }

    #[test]
    fn donnee_trop_ancienne_est_rejetee() {
        let maintenant = chrono::Utc::now().timestamp();
        let (snapshot, sante, anomalies) = agreger(
            vec![reponse("er-api", &[("CDF", 2309.0)], CADENCE_TEMPS_REEL_SECS + 10)],
            &HashMap::new(),
            &HashMap::new(),
            maintenant,
        );
        assert!(!snapshot.rates.contains_key("CDF"));
        assert_eq!(
            sante.get("er-api").map(|s| s.etat),
            Some(super::super::sante::EtatSource::Retardee)
        );
        assert!(anomalies.iter().any(|a| a["type"] == "source_en_retard"));
    }

    /// Ce qui s'est réellement passé sur la machine en production : le cycle
    /// 14:13 publie 166 devises, le cycle 14:14 n'a que les cryptos (les
    /// sources forex ne sont pas au rythme) et publie donc 17 devises. Le
    /// membre perdait le CDF sans que le marché ait disparu.
    #[test]
    fn cycle_partiel_ne_supprime_pas_les_devises_des_sources_lentes() {
        let maintenant = chrono::Utc::now().timestamp();
        // Cycle précédent : le forex a répondu il y a 2 minutes.
        let (vecu, _, _) = agreger(
            vec![reponse("er-api", &[("CDF", 2309.26), ("EUR", 0.9231)], 120)],
            &HashMap::new(),
            &HashMap::new(),
            maintenant - 120,
        );
        // Cycle courant : seule la crypto a répondu.
        let (mut neuf, _, _) = agreger(
            vec![reponse("coingecko", &[("BTC", 77646.0)], 0)],
            &HashMap::new(),
            &HashMap::new(),
            maintenant,
        );
        assert!(!neuf.rates.contains_key("CDF"), "sans rejeu le CDF est perdu");

        let rejouees = rejouer_connu(&mut neuf, &vecu, maintenant);

        assert_eq!(rejouees, 2, "CDF et EUR doivent être resservis");
        assert_eq!(neuf.rates.get("CDF").copied(), Some(2309.26));
        assert_eq!(
            neuf.informations["CDF"].observe_a,
            maintenant - 120,
            "la devise rejouée garde sa vraie date d'observation"
        );
        assert_eq!(
            neuf.informations["BTC"].observe_a,
            maintenant,
            "une valeur vue à l'instant ne doit pas être datée du cycle précédent"
        );
    }

    /// Le rejeu ne remplace jamais une observation neuve : la fraîcheur gagne.
    #[test]
    fn le_rejeu_necrase_jamais_une_vue_neuve() {
        let maintenant = chrono::Utc::now().timestamp();
        let (vecu, _, _) = agreger(
            vec![reponse("er-api", &[("CDF", 2309.26)], 0)],
            &HashMap::new(),
            &HashMap::new(),
            maintenant - 600,
        );
        let (mut neuf, _, _) = agreger(
            vec![reponse("frankfurter", &[("CDF", 2312.75)], 0)],
            &HashMap::new(),
            &HashMap::new(),
            maintenant,
        );

        rejouer_connu(&mut neuf, &vecu, maintenant);

        assert_eq!(neuf.rates.get("CDF").copied(), Some(2312.75));
        assert_eq!(neuf.informations["CDF"].observe_a, maintenant);
    }

    /// Le rejeu ne s'arrête jamais : hors-ligne, le dernier taux capté reste
    /// servi. Ce qui change avec l'âge, c'est le badge, pas la présence.
    #[test]
    fn au_dela_de_la_fenetre_on_rejoue_mais_le_badge_le_dis() {
        let maintenant = chrono::Utc::now().timestamp();
        // Le dernier cycle qui a vraiment parlé : le forex a répondu.
        let (vecu, _, _) = agreger(
            vec![reponse("er-api", &[("CDF", 2309.26)], 0)],
            &HashMap::new(),
            &HashMap::new(),
            maintenant,
        );
        let observe = vecu.informations["CDF"].observe_a;
        // Tick suivant, un jour et une minute plus tard : plus que la crypto.
        let apres = maintenant + CADENCE_REJEU_SECS + 60;
        let (mut neuf, _, _) = agreger(
            vec![reponse("coingecko", &[("BTC", 77646.0)], 0)],
            &HashMap::new(),
            &HashMap::new(),
            maintenant,
        );

        assert_eq!(rejouer_connu(&mut neuf, &vecu, apres), 1);
        assert_eq!(
            neuf.rates.get("CDF").copied(),
            Some(2309.26),
            "le dernier taux capté reste servi, même au-delà d'un jour"
        );
        assert_eq!(
            neuf.informations["CDF"].observe_a, observe,
            "et il reste daté de sa vraie observation"
        );
        assert!(
            est_fraiche(&neuf.informations["CDF"], maintenant),
            "le jour même, c'était le taux du marché"
        );
        assert!(
            !est_fraiche(&neuf.informations["CDF"], apres),
            "un jour plus tard, le badge doit dire « dernier taux connu (périmé) »"
        );
    }

    /// Le trou de production : la blacklist d'immobilisme a fait disparaître le
    /// fiat du dernier snapshot publié. La mémoire longue (rates_history) doit
    /// le resservir, daté, sans jamais en inventer un.
    #[test]
    fn la_memoire_longue_ressert_une_devise_oubliee_du_snapshot() {
        let maintenant = chrono::Utc::now().timestamp();
        let (mut neuf, _, _) = agreger(
            vec![reponse("coingecko", &[("BTC", 77646.0)], 0)],
            &HashMap::new(),
            &HashMap::new(),
            maintenant,
        );
        let souvenir = vec![
            super::super::historique::ObservationPensee {
                devise: "CDF".into(),
                valeur: 2314.48,
                horodatage: maintenant - 7_200,
                source: "agregat".into(),
            },
            super::super::historique::ObservationPensee {
                devise: "BTC".into(),
                valeur: 1.0,
                horodatage: maintenant - 86_400,
                source: "agregat".into(),
            },
        ];

        let rejouees = rejouer_du_passe(&mut neuf, &souvenir, maintenant);

        assert_eq!(rejouees, 1, "une seule devise manquante : CDF");
        assert_eq!(neuf.rates["CDF"], 2314.48);
        assert_eq!(
            neuf.informations["CDF"].observe_a,
            maintenant - 7_200,
            "la devise retrouve SA date, pas l'heure du cycle"
        );
        assert_eq!(neuf.rates["BTC"], 77646.0, "l'observation neuve gagne");
        assert_eq!(
            neuf.informations["CDF"].source, "mediane",
            "l'agrégat journalisé est la valeur servie, pas le dire d'une source"
        );
    }

    /// Une devise que le moteur n'a jamais vue n'existe pas dans la mémoire
    /// longue : elle ne peut donc pas réapparaître par magie.
    #[test]
    fn la_memoire_longue_n_invente_aucune_devise() {
        let maintenant = chrono::Utc::now().timestamp();
        let (mut neuf, _, _) = agreger(
            vec![reponse("coingecko", &[("BTC", 77646.0)], 0)],
            &HashMap::new(),
            &HashMap::new(),
            maintenant,
        );

        let rejouees = rejouer_du_passe(&mut neuf, &[], maintenant);

        assert_eq!(rejouees, 0);
        assert!(!neuf.rates.contains_key("CDF"));
        assert_eq!(neuf.rates.len(), 2, "USD par définition + BTC observé");
    }
}
