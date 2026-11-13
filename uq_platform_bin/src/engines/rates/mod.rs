// ====================
// 🧠 RATES — Moteur de taux intelligent
// ====================
//
// Collecte multi-sources → médiane anti-manipulation → historique SQLite →
// diffusion. Le moteur ne se contente pas de stocker : il détecte les
// sources en panne, immobiles ou divergentes, écarte leurs valeurs, et ne
// sert jamais un taux qu'il n'a pas réellement observé.
//
// Flux (inversé par rapport à l'ancienne version) :
//
//   boucle de collecte ──écriture──> etat::ETAT ──lecture──> handlers API
//        (60 s)                        (Arc<RwLock>)           (non bloquant)
//          │
//          ├──> agregation (médiane, rejets, anomalies)
//          ├──> historique (rates_history + snapshot persisté)
//          └──> bus de moteurs (gossip P2P, médiane collective)
//
// Avant : chaque requête HTTP appelait open.er-api.com avec un timeout de
// 2 s, échouait (latence réelle mesurée 2,5-5,8 s) et servait la table de
// 84 taux codés en dur étiquetée « live ». CDF 2800 au lieu de 2309.
// ====================

pub mod agregation;
pub mod etat;
pub mod historique;
pub mod predictions;
pub mod routes;
pub mod sante;
pub mod sources;

use etat::SnapshotTaux;
use lazy_static::lazy_static;
use serde_json::json;
use sources::ReponseSource;
use std::collections::HashMap;
use std::sync::{Mutex, RwLock};
use std::time::Duration;

pub const TICK_SECS_DEFAULT: u64 = 60;

lazy_static! {
    /// Client HTTP partagé : la pool de connexions évite le coût d'un
    /// handshake TLS froid à chaque source, principal pourvoyeur de timeouts.
    static ref CLIENT: reqwest::Client = reqwest::Client::builder()
        .user_agent("UQ-Platform/1.0")
        .timeout(Duration::from_secs(20))
        .connect_timeout(Duration::from_secs(5))
        .pool_idle_timeout(Duration::from_secs(90))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());

    /// Dernières valeurs par source : sert à détecter l'immobilisme.
    static ref DERNIERES: RwLock<HashMap<String, HashMap<String, f64>>> = RwLock::new(HashMap::new());

    /// Horodatage de la dernière interrogation par source : pilote les rythmes
    /// propres (une source en rythme lent n'est pas une source en panne).
    static ref DERNIERE_INTERROGATION: RwLock<HashMap<String, i64>> = RwLock::new(HashMap::new());

    /// Mémoire longue : dernière observation de chaque devise, telle qu'elle est
    /// tombée dans `rates_history`. Relecture bornée dans le temps, voir
    /// `souvenir_long`.
    static ref SOUVENIR: RwLock<Souvenir> = RwLock::new(Souvenir::default());
}

#[derive(Default, Clone)]
struct Souvenir {
    horodatage: i64,
    points: Vec<historique::ObservationPensee>,
}

/// Le souvenir n'est pas une valeur : c'est une copie de travail, rechargée au
/// plus une fois par heure pour qu'un tick de 60 s ne balaie pas tout
/// l'historique à chaque cycle.
const RAFFRAICHISSEMENT_SOUVENIR_SECS: i64 = 3_600;

fn souvenir_long(maintenant: i64) -> Vec<historique::ObservationPensee> {
    if let Ok(g) = SOUVENIR.read() {
        if g.horodatage > 0 && maintenant - g.horodatage < RAFFRAICHISSEMENT_SOUVENIR_SECS {
            return g.points.clone();
        }
    }
    let points = historique::dernieres_observations();
    if let Ok(mut g) = SOUVENIR.write() {
        *g = Souvenir { horodatage: maintenant, points: points.clone() };
    }
    points
}

pub fn client() -> &'static reqwest::Client {
    &CLIENT
}

pub fn tick_secs() -> u64 {
    std::env::var("UQ_RATES_TICK_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|v: &u64| *v >= 10)
        .unwrap_or(TICK_SECS_DEFAULT)
}

fn sauvegarder_dernieres(reponses: &[ReponseSource]) {
    if let Ok(mut d) = DERNIERES.write() {
        for r in reponses {
            d.insert(r.source.to_string(), r.taux.clone());
        }
    }
}

fn precedentes() -> HashMap<String, HashMap<String, f64>> {
    DERNIERES.read().map(|g| g.clone()).unwrap_or_default()
}

/// Santé connue avant ce cycle : elle porte le compteur d'immobilisme et les
/// blacklistages en cours, qui doivent survivre d'un tick à l'autre.
///
/// Elle est lue AVANT la collecte (le collecteur y écrit le succès et la
/// latence) puis réécrite APRÈS l'agrégation avec les qualifications de
/// cohérence. Toute source ayant répondu est fusionnée, y compris celles
/// écartées par l'agrégat : sinon un blacklistage ne serait jamais persisté.
fn sante_precedente() -> HashMap<String, sante::SanteSource> {
    etat::sante().into_iter().map(|s| (s.nom.to_string(), s)).collect()
}

/// Collecte toutes les sources actives EN PARALLÈLE. Chaque source est
/// isolée dans sa tâche avec sa propre deadline d'horloge : une API qui
/// pendouille ne peut pas retarder le cycle ni contaminer les autres.
///
/// La santé n'est marquée « succès » que pour les sources réellement
/// interrogées : une source en rythme lent n'est pas en panne.
pub async fn collecter_tout() -> (Vec<ReponseSource>, Vec<(&'static str, sources::ErreurCollecte)>) {
    let maintenant = chrono::Utc::now().timestamp();
    let mut handles = Vec::new();

    for def in sources::sources() {
        if !sources::source_active(def) {
            let mut maj = etat::lire_sante_source(def.nom).unwrap_or_else(|| {
                sante::SanteSource::nouvelle(def.nom, def.categorie.cle())
            });
            maj.marquer_absence_de_cle();
            etat::enregistrer_sante(def.nom, maj);
            continue;
        }
        if !def.activee {
            continue;
        }
        if !au_rythme(def.nom, def.rythme_secs, maintenant) {
            continue;
        }
        let def_static: &'static sources::SourceDef = def;
        let marge = def.timeout + Duration::from_millis(800);
        let h = tokio::spawn(async move {
            match tokio::time::timeout(marge, sources::collecter(&*CLIENT, def_static)).await {
                Ok(interne) => interne,
                Err(_) => Err(sources::ErreurCollecte::Reseau("deadline cycle".into())),
            }
        });
        handles.push((def.nom, h));
    }

    let mut reussies = Vec::new();
    let mut echecs = Vec::new();

    for (nom, h) in handles {
        let def = match sources::sources().iter().find(|s| s.nom == nom) {
            Some(d) => d,
            None => continue,
        };
        let marqueur = |sante: sante::SanteSource| etat::enregistrer_sante(nom, sante);
        match h.await {
            Ok(Ok(reponse)) => {
                let mut maj = etat::lire_sante_source(nom).unwrap_or_else(|| {
                    sante::SanteSource::nouvelle(nom, def.categorie.cle())
                });
                maj.marquer_succes(reponse.latence_ms, maintenant);
                marqueur(maj);
                reussies.push(reponse);
            }
            Ok(Err(erreur)) => {
                let mut maj = etat::lire_sante_source(nom).unwrap_or_else(|| {
                    sante::SanteSource::nouvelle(nom, def.categorie.cle())
                });
                maj.marquer_echec(&formater_erreur(&erreur), maintenant);
                marqueur(maj);
                if erreur.cle() != "sans_cle" {
                    echecs.push((nom, erreur));
                }
            }
            Err(_) => {
                let mut maj = etat::lire_sante_source(nom).unwrap_or_else(|| {
                    sante::SanteSource::nouvelle(nom, def.categorie.cle())
                });
                maj.marquer_echec("tâche de collecte perdue", maintenant);
                marqueur(maj);
                echecs.push((nom, sources::ErreurCollecte::Reseau("tâche perdue".into())));
            }
        }
    }

    sauvegarder_dernieres(&reussies);
    (reussies, echecs)
}

/// Source interrogeable maintenant selon son rythme propre.
fn au_rythme(nom: &str, rythme_secs: u64, maintenant: i64) -> bool {
    let rythme = rythme_secs as i64;
    if let Ok(d) = DERNIERE_INTERROGATION.read() {
        if let Some(avant) = d.get(nom) {
            if (maintenant - *avant) < rythme {
                return false;
            }
        }
    }
    if let Ok(mut d) = DERNIERE_INTERROGATION.write() {
        d.insert(nom.to_string(), maintenant);
    }
    true
}

pub fn formater_erreur(e: &sources::ErreurCollecte) -> String {
    match e {
        sources::ErreurCollecte::Reseau(m) => format!("reseau : {}", m),
        sources::ErreurCollecte::Http(s) => format!("http {}", s),
        sources::ErreurCollecte::Format(m) => format!("format : {}", m),
        sources::ErreurCollecte::SansCle => "sans cle".to_string(),
    }
}

/// Un cycle complet : collecte, agrégation, journalisation, publication.
pub async fn un_cycle() -> SnapshotTaux {
    let maintenant = chrono::Utc::now().timestamp();
    let avant = precedentes();
    let sante_avant = sante_precedente();
    let (reponses, echecs) = collecter_tout().await;

    // Les observations brutes par source, saisies avant que l'agrégation ne
    // consomme les réponses : l'historique doit garder la valeur dite par
    // chaque fournisseur, pas seulement la valeur retenue.
    let observations: HashMap<String, HashMap<String, f64>> = reponses
        .iter()
        .map(|r| (r.source.to_string(), r.taux.clone()))
        .collect();

    let (mut snapshot, sante_cycle, anomalies) =
        agregation::agreger(reponses, &avant, &sante_avant, maintenant);

    if echecs.iter().any(|(_, e)| matches!(e, sources::ErreurCollecte::Http(_))) {
        let mut annexe = anomalies.clone();
        for (nom, erreur) in &echecs {
            annexe.push(json!({
                "type": "source_en_panne",
                "source": nom,
                "erreur": formater_erreur(erreur),
                "a": maintenant,
            }));
        }
        etat::publier_anomalies(annexe);
    } else {
        etat::publier_anomalies(anomalies);
    }

    // Auto-réparation : si le cycle n'a rien OBSERVÉ, on ressert le dernier
    // taux réellement vu (mémoire, puis disque), daté et étiqueté stale.
    if sans_observation(&snapshot) {
        if let Some(rassis) =
            dernier_observe_depuis(etat::lire_snapshot(), historique::charger_snapshot(), maintenant)
        {
            snapshot = rassis;
        }
    } else {
        // Fusion santé : le collecteur est seul maître du mesuré (latence,
        // échecs, dernier succès) ; l'agrégation n'apporte que la qualification
        // de cohérence. Écraser l'un avec l'autre effacerait la latence réelle.
        for (nom, qualification) in sante_cycle {
            let fusionne = match etat::lire_sante_source(&nom) {
                Some(mut courant) => {
                    courant.valeurs_identiques_de_suite = qualification.valeurs_identiques_de_suite;
                    courant.suspecte_jusqu_a = qualification.suspecte_jusqu_a;
                    if matches!(
                        qualification.etat,
                        sante::EtatSource::Suspecte | sante::EtatSource::Retardee
                    ) {
                        courant.etat = qualification.etat;
                        courant.detail = qualification.detail;
                    }
                    courant
                }
                None => qualification,
            };
            etat::enregistrer_sante(&nom, fusionne);
        }
        purger_suspects(maintenant);
        // L'historique ne garde QUE les observations réelles de ce cycle : une
        // valeur rejouée serait un faux point d'horodatage.
        let _ = historique::enregistrer(maintenant, &snapshot.rates, &observations);

        // Cycle partiel : les devises des sources en rythme lent (ou en panne
        // ce tick) restent servies avec leur date d'observation réelle.
        let connu = etat::lire_snapshot().or_else(|| {
            historique::charger_snapshot().and_then(|v| serde_json::from_value::<SnapshotTaux>(v).ok())
        });
        if let Some(connu) = connu {
            snapshot.devises_rejouees = agregation::rejouer_connu(&mut snapshot, &connu, maintenant);
        }
    }

    // Mémoire longue : il arrive que le snapshot publié ait lui-même oublié des
    // devises — blacklist d'immobilisme, ou redémarrage sur une base plus riche
    // que la RAM. Le dernier point réellement journalisé dans `rates_history`
    // les ressert, datées de leur vraie observation. C'est ce qui garantit au
    // membre hors-ligne de continuer à voir LE dernier taux capté, devise par
    // devise, jusqu'à la reconnexion.
    let resservies = agregation::rejouer_du_passe(&mut snapshot, &souvenir_long(maintenant), maintenant);
    if resservies > 0 {
        snapshot.devises_rejouees = snapshot.devises_rejouees.saturating_add(resservies);
    }

    // Le snapshot persisté est le snapshot SERVI, réparation incluse : un
    // processus qui redémarre en pleine coupure retrouve la même valeur. Un
    // snapshot sans observation réelle n'est jamais écrit, pour ne pas écraser
    // la mémoire du disque avec un vide.
    if !sans_observation(&snapshot) {
        if let Ok(json) = serde_json::to_string(&snapshot) {
            historique::enregistrer_snapshot(&json, maintenant);
        }
    }

    etat::publier(snapshot.clone());
    snapshot
}

/// Auto-réparation : la valeur remise en service est la dernière réellement
/// observée, jamais une valeur par défaut. Mémoire d'abord (le cycle vient de
/// échouer, la RAM a ce qu'il faut), snapshot sur disque ensuite (le processus
/// a redémaré pendant la coupure). `None` si rien n'a jamais été observé : la
/// route répondra alors 503, ce qui est la réponse honnête.
fn dernier_observe_depuis(
    memoire: Option<SnapshotTaux>,
    disque: Option<serde_json::Value>,
    maintenant: i64,
) -> Option<SnapshotTaux> {
    let relu = match memoire {
        Some(s) if !s.rates.is_empty() => s,
        _ => disque.and_then(|v| serde_json::from_value::<SnapshotTaux>(v).ok())?,
    };
    Some(agregation::marquer_perime(relu, maintenant))
}

/// Un snapshot n'est exploitable que s'il repose sur une observation réelle.
/// `USD = 1` est une définition injectée par l'agrégation : seul dans la map,
/// il ne dit pas que le marché a été vu, et ne doit donc ni déclencher la
/// réparation ni la remplacer.
///
/// Le critère est la présence d'une devise réellement datée, pas le compteur de
/// sources du cycle : un snapshot 100 % resservi depuis l'historique n'a aucune
/// source vivante et reste nonetheless le dernier taux capté — c'est exactement
/// ce que le membre hors-ligne doit recevoir, avec le badge qui le dit.
fn sans_observation(snapshot: &SnapshotTaux) -> bool {
    snapshot.rates.is_empty()
        || !snapshot
            .informations
            .iter()
            .any(|(devise, info)| devise != "USD" && info.observe_a > 0)
}

/// Fin de blacklistage : une source soupçonnée d'immobilisme redevient
/// éligible dès que sa fenêtre est passée, sans attendre une nouvelle réponse.
fn purger_suspects(maintenant: i64) {
    for mut s in etat::sante() {
        if s.etat == sante::EtatSource::Suspecte && maintenant >= s.suspecte_jusqu_a {
            s.etat = sante::EtatSource::Active;
            s.valeurs_identiques_de_suite = 0;
            s.suspecte_jusqu_a = 0;
            s.detail = "blacklist expiré".to_string();
            etat::enregistrer_sante(s.nom, s);
        }
    }
}

/// Boucle du moteur : un cycle toutes les `UQ_RATES_TICK_SECS` secondes
/// (défaut 60). Démarre immédiatement pour que la première requête utilisateur
/// trouve une valeur, au lieu de payer le timeout en face de l'utilisateur.
pub async fn boucle() {
    use tokio::time::{interval, Duration as TokioDuration};
    let mut ticker = interval(TokioDuration::from_secs(tick_secs()));
    // Premier appel sans attendre : sinon la plateforme démarre sans taux.
    let snapshot = un_cycle().await;
    tracing_dernier(&snapshot);
    loop {
        ticker.tick().await;
        let snapshot = un_cycle().await;
        tracing_dernier(&snapshot);
    }
}

fn tracing_dernier(snapshot: &SnapshotTaux) {
    println!(
        "💱 Moteur de taux : {} devises agrégées depuis {} source(s) [{}]{}",
        snapshot.rates.len(),
        snapshot.nb_sources,
        snapshot.source,
        if snapshot.source == "stale" {
            format!(" — périmé depuis {} s", snapshot.age_secs)
        } else {
            String::new()
        }
    );
    if let Some(cdf) = snapshot.informations.get("CDF") {
        println!(
            "   CDF = {:.2} via {} (origines : {})",
            cdf.valeur,
            cdf.source,
            if cdf.origines.is_empty() {
                "—".to_string()
            } else {
                cdf.origines.join(", ")
            }
        );
    }
}

/// Taux disponibles, ou erreur explicite. C'est la seule porte d'entrée des
/// routes qui doivent refuser de servir une valeur inventée.
pub async fn taux_ou_indisponible() -> Result<SnapshotTaux, (u16, &'static str, String)> {
    if let Some(s) = etat::lire_snapshot() {
        if !sans_observation(&s) {
            return Ok(s);
        }
    }
    // Démarrage à froid : un cycle immédiat, borné, avant de répondre 503.
    let snapshot = un_cycle().await;
    if sans_observation(&snapshot) {
        return Err((
            503,
            "TAUX_INDISPONIBLE",
            "Aucune source de taux n'a répondu et aucun historique n'existe. Aucun taux ne sera inventé.".to_string(),
        ));
    }
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Vérification runtime, volontairement `#[ignore]` : elle sort sur le
    /// réseau. Lancée à la main, elle prouve ce que le plan promet —
    /// `cargo test -- engines::rates::tests::verifier_sur_le_reseau -- --ignored
    /// --nocapture --test-threads=1`.
    ///
    /// Elle écrit dans une base isolée du projet, jamais dans `data/uq_engines.db`.
    #[tokio::test]
    #[ignore = "collecte réelle sur les APIs externes"]
    async fn verifier_sur_le_reseau() {
        let dossier = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tmp_test");
        let _ = std::fs::create_dir_all(&dossier);
        std::env::set_var(
            "UQ_ENGINES_DB",
            dossier.join("verif_reseau.db").to_string_lossy().to_string(),
        );

        let snapshot = un_cycle().await;

        println!(
            "\n=== CYCLE : {} devises, {} source(s), source = {}, âge {} s ===",
            snapshot.rates.len(),
            snapshot.nb_sources,
            snapshot.source,
            snapshot.age_secs
        );
        for (devise, info) in snapshot.informations.iter().take(12) {
            println!(
                "  {:<5} = {:>12.4}  [{}]  origines = {}",
                devise,
                info.valeur,
                info.source,
                info.origines.join(",")
            );
        }
        println!("--- santé des sources ---");
        for s in etat::sante() {
            println!(
                "  {:<16} {:?}  latence {} ms  echecs {}  dernier succes {}  {}",
                s.nom, s.etat, s.latence_moyenne_ms, s.echecs_consecutifs, s.dernier_succes, s.detail
            );
        }
        println!("--- anomalies ---");
        for a in etat::anomalies() {
            println!("  {}", a);
        }
        println!(
            "--- historique : {} lignes, {} devises distinctes ---",
            historique::compter_lignes(),
            snapshot.rates.len()
        );

        assert!(
            snapshot.nb_sources >= 2,
            "au moins deux sources doivent répondre sur le réseau réel"
        );
        // CDF est la devise qui a servi 2800 en dur pendant des mois : elle
        // doit être là, étiquetée, et à la valeur du marché.
        let cdf = snapshot
            .informations
            .get("CDF")
            .expect("le CDF doit être observé par au moins une source");
        // Ce qu'on exige : une étiquette qui désigne une source réellement
        // interrogée. Ce qu'on refuse : un repli, un cache, une valeur en dur.
        assert!(
            !matches!(cdf.source.as_str(), "stale" | "aucune" | "fallback" | "definition"),
            "le CDF ne doit pas venir d'un repli inventé (étiquette : {})",
            cdf.source
        );
        assert!(
            !cdf.origines.is_empty(),
            "un taux observé doit nommer au moins une source (CDF : {})",
            cdf.source
        );
        assert!(
            (cdf.valeur - 2800.0).abs() / 2800.0 > 0.01,
            "le taux servi ne doit plus être l'ancienne valeur en dur 2800 (ici {})",
            cdf.valeur
        );
        println!(
            "\n>>> CDF = {:.2} via {} (origines : {}) ; officiel BCC : {:?}",
            cdf.valeur,
            cdf.source,
            cdf.origines.join(","),
            cdf.officiel_bcc
        );
    }

    /// Base isolée, nommée à la nanoseconde : un test qui panique laisse des
    /// lignes qui feraient échouer la session suivante.
    fn chemin_isole(nom: &str) -> String {
        let nonce = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let dossier = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tmp_test");
        let _ = std::fs::create_dir_all(&dossier);
        let chemin = dossier
            .join(format!("test_rates_{}_{}.db", nom, nonce))
            .to_string_lossy()
            .to_string();
        supprimer_base(&chemin);
        chemin
    }

    fn supprimer_base(chemin: &str) {
        for suffixe in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{}", chemin, suffixe));
        }
    }

    fn snapshot_test(valeurs: &[(&str, f64)], source: &str, horodatage: i64) -> SnapshotTaux {
        let informations = valeurs
            .iter()
            .map(|(devise, valeur)| {
                (
                    devise.to_string(),
                    etat::InformationDevise {
                        valeur: *valeur,
                        source: source.to_string(),
                        origines: vec![source.to_string()],
                        mediane: None,
                        ecart_pct: None,
                        officiel_bcc: None,
                        observe_a: horodatage,
                    },
                )
            })
            .collect::<HashMap<String, etat::InformationDevise>>();
        SnapshotTaux {
            rates: valeurs.iter().map(|(d, v)| (d.to_string(), *v)).collect(),
            informations,
            horodatage,
            source: source.to_string(),
            age_secs: 0,
            nb_sources: 1,
            devises_rejouees: 0,
        }
    }

    /// Prouve la règle « hors-ligne, on ressert la dernière valeur réellement
    /// observée » sans dépendre du réseau : la réparation est une fonction pure
    /// des deux mémoires (RAM du processus, snapshot du disque).
    #[test]
    fn reparation_ressert_la_valeur_de_la_memoire() {
        let maintenant = chrono::Utc::now().timestamp();
        let vecu = snapshot_test(&[("CDF", 2309.26)], "er-api", maintenant - 900);

        let rassis = dernier_observe_depuis(Some(vecu), None, maintenant).expect("un taux observé");

        assert_eq!(rassis.source, "stale");
        assert_eq!(rassis.informations["CDF"].source, "stale");
        assert_eq!(rassis.informations["CDF"].valeur, 2309.26);
        assert_eq!(rassis.age_secs, 900);
        assert_eq!(
            rassis.informations["CDF"].origines,
            vec!["er-api".to_string()],
            "la provenance d'origine doit survivre au resservage"
        );
    }

    /// Le processus redémarre pendant la coupure : la RAM est vide, seul le
    /// snapshot écrit sur disque permet de continuer à servir le dernier taux.
    #[test]
    fn redemarrage_a_chaud_recharge_le_snapshot_du_disque() {
        let chemin = chemin_isole("reparation_disque");
        let maintenant = chrono::Utc::now().timestamp();
        let capture = maintenant - 4000;
        let vecu = snapshot_test(&[("CDF", 2261.56), ("EUR", 0.9231)], "bcc", capture);

        let json = serde_json::to_string(&vecu).unwrap();
        historique::enregistrer_snapshot_dans(&chemin, &json, capture);
        let relu = historique::charger_snapshot_dans(&chemin).expect("snapshot relu du disque");

        let rassis = dernier_observe_depuis(None, Some(relu), maintenant).expect("le disque a la valeur");

        assert_eq!(rassis.informations["CDF"].valeur, 2261.56);
        assert_eq!(rassis.informations["EUR"].valeur, 0.9231);
        assert_eq!(rassis.source, "stale");
        assert_eq!(rassis.horodatage, capture, "l'âge affiché reste l'heure de capture");
        assert_eq!(rassis.age_secs, 4000);

        supprimer_base(&chemin);
    }

    /// Aucune capture connue : le moteur ne produit aucun taux, il n'invente
    /// rien et les routes répondront 503.
    #[test]
    fn sans_capture_aucun_taux_n_est_invente() {
        assert!(dernier_observe_depuis(None, None, chrono::Utc::now().timestamp()).is_none());
    }

    /// Le piège qui faisait sauter la réparation hors-ligne : l'agrégation
    /// injecte toujours `USD = 1`. Un snapshot qui ne contient que cette
    /// définition n'est pas un cycle ayant observé le marché.
    #[test]
    fn usd_seul_n_est_pas_une_observation() {
        let mut definition =
            snapshot_test(&[("USD", 1.0)], "definition", chrono::Utc::now().timestamp());
        // Ce que produit un cycle où aucune source n'a répondu : la définition
        // USD, et zéro contribution.
        definition.nb_sources = 0;
        definition.source = "aucune".to_string();
        assert!(sans_observation(&definition));

        let maintenant = chrono::Utc::now().timestamp();
        let un_peu_de_marche = snapshot_test(&[("USD", 1.0), ("CDF", 2309.26)], "er-api", maintenant);
        assert!(!sans_observation(&un_peu_de_marche));
    }

    /// Coupure réseau de bout en bout : les 11 sources échouent, le cycle
    /// ressert la dernière valeur captée, étiquetée stale, et les routes
    /// continuent de répondre. `#[ignore]` : elle détourne le trafic du
    /// processus vers un proxy mort et doit tourner seule.
    ///
    /// `cargo test -- engines::rates::tests::hors_ligne_ressert_le_dernier_taux_capte -- --ignored --nocapture --test-threads=1`
    #[tokio::test]
    #[ignore = "coupe le réseau du processus (proxy invalide)"]
    async fn hors_ligne_ressert_le_dernier_taux_capte() {
        let chemin = chemin_isole("hors_ligne");
        std::env::set_var("UQ_ENGINES_DB", &chemin);

        // Ce que le moteur avait réellement observé avant la coupure.
        let maintenant = chrono::Utc::now().timestamp();
        let capture = maintenant - 1200;
        let vecu = snapshot_test(&[("CDF", 2309.26), ("EUR", 0.9231)], "er-api", capture);
        let json = serde_json::to_string(&vecu).unwrap();
        historique::enregistrer_snapshot_dans(&chemin, &json, capture);

        // Le client HTTP n'est construit qu'au premier appel : placer le proxy
        // avant toute requête rend toutes les sources injoignables.
        std::env::set_var("HTTPS_PROXY", "http://127.0.0.1:9");
        std::env::set_var("HTTP_PROXY", "http://127.0.0.1:9");
        std::env::remove_var("NO_PROXY");

        let apres_coupure = un_cycle().await;

        println!(
            "\n=== CYCLE HORS-LIGNE : {} devises, source = {}, âge {} s ===",
            apres_coupure.rates.len(),
            apres_coupure.source,
            apres_coupure.age_secs
        );
        for (devise, info) in apres_coupure.informations.iter() {
            println!("  {:<5} = {:>12.4}  [{}]  provient de {}", devise, info.valeur, info.source, info.origines.join(","));
        }

        assert_eq!(
            apres_coupure.source, "stale",
            "un taux resservi hors-ligne doit être étiqueté comme périmé"
        );
        let cdf = apres_coupure
            .informations
            .get("CDF")
            .expect("le dernier taux capté doit être resservi, pas remplacé par 0");
        assert_eq!(cdf.valeur, 2309.26);
        assert_eq!(cdf.source, "stale");
        assert_eq!(cdf.origines, vec!["er-api".to_string()]);
        assert!(
            apres_coupure.age_secs >= 1200,
            "l'âge publié doit dire depuis quand cette valeur n'a pas été rafraîchie ({} s)",
            apres_coupure.age_secs
        );

        // Les routes lisent le même état : elle ne doivent pas tomber en 503.
        let sert = taux_ou_indisponible()
            .await
            .expect("hors-ligne, la dernière capture vaut mieux qu'une erreur");
        assert_eq!(sert.informations["CDF"].valeur, 2309.26);
        println!(">>> routes OK hors-ligne : CDF {} [{}]", sert.informations["CDF"].valeur, sert.source);

        std::env::remove_var("HTTPS_PROXY");
        std::env::remove_var("HTTP_PROXY");
        supprimer_base(&chemin);
    }
}
