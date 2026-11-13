// ====================
// 🩺 SANTÉ — État par source : panne, retard, immobilisme, réintégration
// ====================
//
// Une source n'est pas simplement « activable » ou « désactivable » : elle
// peut mentir sans tomber. Les signaux ci-dessous distinguent les deux.
//
//   Down      → timeout, erreur HTTP ou JSON invalide ; remplacée par les
//               autres sources (auto-réparation par redondance)
//   Retard    → pas de donnée fraîche depuis plus longtemps que sa cadence
//               normale (la BCC publie une fois par jour, ce n'est pas un retard)
//   Immuable  → même valeur pendant N ticks consécutifs : suspect, possible
//               réponse en cache côté fournisseur ou valeur en dur
//   Suspect   → blacklist temporaire ; réintégrée si elle redevient cohérente
// ====================

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EtatSource {
    Active,
    Down,
    /// Valeurs identiques trop longtemps : soupçon de cache côté fournisseur.
    Suspecte,
    Retardee,
    /// Clé requise non fournie : exclue de l'agrégat, pas une panne.
    Desactivee,
    /// Jamais interrogée avec succès : aucun verdict. Distinct de Desactivee,
    /// qui affirme une absence de clé, et de Down, qui affirme une panne.
    Inconnue,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SanteSource {
    pub nom: &'static str,
    pub categorie: &'static str,
    pub etat: EtatSource,
    pub dernier_succes: i64,
    pub echecs_consecutifs: u32,
    pub latence_moyenne_ms: u64,
    pub valeurs_identiques_de_suite: u32,
    /// Fin du blacklistage temporaire (0 = non blacklistée).
    pub suspecte_jusqu_a: i64,
    /// Raison lisible pour le statut.
    pub detail: String,
}

impl SanteSource {
    pub fn nouvelle(nom: &'static str, categorie: &'static str) -> Self {
        SanteSource {
            nom,
            categorie,
            etat: EtatSource::Inconnue,
            dernier_succes: 0,
            echecs_consecutifs: 0,
            latence_moyenne_ms: 0,
            valeurs_identiques_de_suite: 0,
            suspecte_jusqu_a: 0,
            detail: "jamais interrogée".to_string(),
        }
    }

    /// Moyenne glissante : les latences de ces API varient du simple au triple,
    /// une moyenne lissée est le seul signal exploitable pour le timeout.
    fn lat_suivante(&mut self, latence_ms: u64) {
        self.latence_moyenne_ms =
            (self.latence_moyenne_ms as f64 * 0.7 + latence_ms as f64 * 0.3).round() as u64;
    }

    pub fn marquer_succes(&mut self, latence_ms: u64, maintenant: i64) {
        self.echecs_consecutifs = 0;
        self.dernier_succes = maintenant;
        self.lat_suivante(latence_ms);
        if self.etat == EtatSource::Suspecte && maintenant >= self.suspecte_jusqu_a {
            self.etat = EtatSource::Active;
            self.detail = "réintégrée après blacklistage".to_string();
        } else if self.etat != EtatSource::Suspecte {
            self.etat = EtatSource::Active;
            self.detail = format!("{} ms", latence_ms);
        }
    }

    pub fn marquer_echec(&mut self, raison: &str, maintenant: i64) {
        self.echecs_consecutifs += 1;
        self.detail = raison.to_string();
        // Three consecutive failures and we consider the source down.
        if self.echecs_consecutifs >= 3 {
            self.etat = EtatSource::Down;
        }
        let _ = maintenant;
    }

    pub fn marquer_absence_de_cle(&mut self) {
        self.etat = EtatSource::Desactivee;
        self.detail = "clé API non fournie".to_string();
    }

    /// Valeur identique pendant `seuil` ticks → source soupçonnée de servir
    /// du cache : on l'écarte de la médiane le temps du blacklistage.
    pub fn noter_valeur(&mut self, similaire: bool, seuil: u32, blacklist_secs: i64, maintenant: i64) {
        if similaire {
            self.valeurs_identiques_de_suite += 1;
        } else {
            self.valeurs_identiques_de_suite = 0;
        }
        if self.valeurs_identiques_de_suite >= seuil && self.etat == EtatSource::Active {
            self.etat = EtatSource::Suspecte;
            self.suspecte_jusqu_a = maintenant + blacklist_secs;
            self.detail = format!(
                "valeur immobile depuis {} lectures",
                self.valeurs_identiques_de_suite
            );
        }
    }

    pub fn est_utilisable(&self, maintenant: i64, cadence_max_secs: i64) -> bool {
        match self.etat {
            EtatSource::Desactivee => false,
            EtatSource::Suspecte => maintenant >= self.suspecte_jusqu_a,
            EtatSource::Down => maintenant - self.dernier_succes < cadence_max_secs * 3,
            _ => maintenant - self.dernier_succes <= cadence_max_secs,
        }
    }
}
