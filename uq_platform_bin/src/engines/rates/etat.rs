// ====================
// 🧭 ÉTAT — Snapshot des taux agrégés, lu par toute la plateforme
// ====================
//
// C'est le coeur de l'inversion du flux : le collecteur de fond ÉCRIT ici,
// et les handlers LISENT ici. Plus aucun appel HTTP bloquant dans une route.
//
// Le type `TauxFiat` reste celui que connaissent les 14 sites appelants
// (api_handlers, trading_routes, bus, external_routes, oracle_extensions,
// public_routes) : leur signature ne change pas.
//
// `source` ne ment jamais :
//   "bcc"           → cours officiel Banque Centrale du Congo (cadence daily)
//   "mediane"       → agrégat d'au moins 3 sources
//   "multi_sources" → plusieurs sources, aucune devise à 3 contributions
//   "source_unique" → une seule source répond
//   nom de source   → la devise vient de cette source ("er-api", "er-api+xaus")
//   "stale"         → dernier taux réellement observé, daté et marqué
// ====================

use super::sante::SanteSource;
use lazy_static::lazy_static;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::RwLock;

/// Ce qu'on sait d'un taux pour une devise à un instant donné.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InformationDevise {
    pub valeur: f64,
    /// Source retenue : "bcc", "mediane", nom(s) de source(s) réelle(s), ou "stale".
    pub source: String,
    /// Devises->sources ayant réellement nourri la valeur.
    pub origines: Vec<String>,
    pub mediane: Option<f64>,
    pub ecart_pct: Option<f64>,
    /// Cours officiel BCC quand disponible (parallel à la médiane).
    pub officiel_bcc: Option<f64>,
    /// Horodatage de l'OBSERVATION réelle de cette valeur, et non de la date du
    /// cycle : une devise portée par un cycle plus lent que les autres reste
    /// affichée avec sa vraie fraîcheur. `#[serde(default)]` pour relire les
    /// snapshots écrits avant ce champ.
    #[serde(default)]
    pub observe_a: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SnapshotTaux {
    pub rates: HashMap<String, f64>,
    pub informations: HashMap<String, InformationDevise>,
    pub horodatage: i64,
    /// "bcc" | "mediane" | "source_unique" | "stale"
    pub source: String,
    pub age_secs: i64,
    pub nb_sources: usize,
    /// Devises resservies ce cycle parce que leur source n'a pas été
    /// interrogée (ou n'a pas répondu) : leur vraie date est dans `observe_a`.
    #[serde(default)]
    pub devises_rejouees: usize,
}

lazy_static! {
    static ref ETAT: RwLock<Option<SnapshotTaux>> = RwLock::new(None);
    static ref SANTE: RwLock<HashMap<String, SanteSource>> = RwLock::new(HashMap::new());
    static ref ANOMALIES: RwLock<Vec<Value>> = RwLock::new(Vec::new());
}

pub fn publier(snapshot: SnapshotTaux) {
    if let Ok(mut e) = ETAT.write() {
        *e = Some(snapshot);
    }
}

pub fn lire_snapshot() -> Option<SnapshotTaux> {
    ETAT.read().ok().and_then(|g| g.clone())
}

pub fn sante() -> Vec<SanteSource> {
    SANTE
        .read()
        .map(|g| g.values().cloned().collect())
        .unwrap_or_default()
}

pub fn enregistrer_sante(nom: &str, maj: SanteSource) {
    if let Ok(mut s) = SANTE.write() {
        s.insert(nom.to_string(), maj);
    }
}

pub fn lire_sante_source(nom: &str) -> Option<SanteSource> {
    SANTE.read().ok().and_then(|g| g.get(nom).cloned())
}

/// Journal des anomalies du dernier cycle (rejet, alerte, manipulation,
/// source immobile, source en retard). Consulté par /api/rates/status et
/// remonté au bus de fraude.
pub fn publier_anomalies(anomalies: Vec<Value>) {
    if let Ok(mut a) = ANOMALIES.write() {
        *a = anomalies;
    }
}

pub fn anomalies() -> Vec<Value> {
    ANOMALIES.read().map(|g| g.clone()).unwrap_or_default()
}
