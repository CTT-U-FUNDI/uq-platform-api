// ====================
// 🤖 IA DE VALIDATION KYC
// ====================
//
// « L'eau circule. L'IA apprend. Le proprietaire valide. »
//
// Ce module :
// 1. Pre-remplit les metriques (via les fonctions de calcul)
// 2. Propose une decision basee sur les seuils
// 3. Apprend des decisions du proprietaire
// 4. Devient autonome progressivement
//
// ====================

use crate::kyc::decision::{HumanDecision, RejectionReason};
use crate::kyc::metrics::AiMetrics;
use serde::{Deserialize, Serialize};

// ====================
// SEUILS
// ====================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Seuils {
    /// Netteté minimale pour valider
    pub min_sharpness: f32,
    /// Netteté en dessous de laquelle on refuse
    pub refuse_sharpness: f32,

    /// Similarité faciale minimale
    pub min_face_similarity: f32,
    /// Similarité en dessous de laquelle on refuse
    pub refuse_face_similarity: f32,

    /// Vivacité minimale
    pub min_liveness: f32,

    /// Âge minimum
    pub min_age: f32,

    /// Confiance OCR minimale
    pub min_ocr_confidence: f32,
}

impl Default for Seuils {
    fn default() -> Self {
        Self {
            min_sharpness: 0.70,
            refuse_sharpness: 0.40,
            min_face_similarity: 0.75,
            refuse_face_similarity: 0.60,
            min_liveness: 0.70,
            min_age: 13.0,
            min_ocr_confidence: 0.70,
        }
    }
}

// ====================
// DECISION IA
// ====================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AiDecision {
    Valider,
    Refuser,
    Incertain,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiSuggestion {
    pub decision: AiDecision,
    pub confiance: f32,
    pub raisons: Vec<RejectionReason>,
    pub corrections: Vec<String>,
}

// ====================
// IA
// ====================

pub struct KycIa {
    pub seuils: Seuils,
    pub autonomie: f32, // 0.0 = 100% humain, 1.0 = 100% auto
    pub apprentissages: Vec<Apprentissage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Apprentissage {
    pub date: i64,
    pub metrique: String,
    pub valeur: f32,
    pub decision_humaine: String,
    pub correction: String,
}

impl KycIa {
    pub fn new() -> Self {
        Self {
            seuils: Seuils::default(),
            autonomie: 0.0,
            apprentissages: Vec::new(),
        }
    }

    /// Propose une decision basee sur les metriques et les seuils
    pub fn proposer(&self, metrics: &AiMetrics) -> AiSuggestion {
        let mut raisons = Vec::new();
        let mut corrections = Vec::new();
        let mut score_positif = 0.0;
        let mut score_negatif = 0.0;

        // 1. Netteté
        if metrics.document_quality.sharpness < self.seuils.refuse_sharpness {
            raisons.push(RejectionReason::PhotoIllegible);
            corrections.push("Photo trop floue — reprends la photo".to_string());
            score_negatif += 1.0;
        } else if metrics.document_quality.sharpness < self.seuils.min_sharpness {
            corrections.push("Photo peu nette — essaie une meilleure lumière".to_string());
            score_negatif += 0.5;
        } else {
            score_positif += 1.0;
        }

        // 2. Similarité faciale
        if metrics.face_comparison.similarity_score < self.seuils.refuse_face_similarity {
            raisons.push(RejectionReason::FaceMismatch);
            corrections.push("Le visage ne correspond pas — reprends le selfie".to_string());
            score_negatif += 1.0;
        } else if metrics.face_comparison.similarity_score < self.seuils.min_face_similarity {
            corrections.push("Similarité faible — améliore l'éclairage".to_string());
            score_negatif += 0.5;
        } else {
            score_positif += 1.0;
        }

        // 3. Vivacité
        if metrics.liveness.composite_score < self.seuils.min_liveness {
            raisons.push(RejectionReason::LivenessFailed);
            corrections.push("Video non convaincante — refais la video".to_string());
            score_negatif += 1.0;
        } else {
            score_positif += 1.0;
        }

        // 4. Âge
        if let Some(age) = metrics.age_estimation.document_age {
            if age < self.seuils.min_age {
                raisons.push(RejectionReason::Minor);
                corrections.push("Âge minimum 13 ans requis".to_string());
                score_negatif += 1.0;
            } else {
                score_positif += 1.0;
            }
        }

        // 5. OCR
        if metrics.ocr_confidence.mean_char_confidence < self.seuils.min_ocr_confidence {
            corrections.push("Texte peu lisible — reprends la photo".to_string());
            score_negatif += 0.5;
        } else {
            score_positif += 1.0;
        }

        // 6. Anomalies
        if metrics.anomalies.duplicate_face_found {
            raisons.push(RejectionReason::DuplicateFace);
            corrections.push("Ce visage existe déjà — contacte le support".to_string());
            score_negatif += 2.0;
        }
        if metrics.anomalies.duplicate_document_found {
            raisons.push(RejectionReason::DuplicateDocument);
            corrections.push("Ce document existe déjà".to_string());
            score_negatif += 2.0;
        }

        // Calcul de la decision
        let total = score_positif + score_negatif;
        let confiance = if total > 0.0 {
            score_positif / total
        } else {
            0.5
        };

        let decision = if !raisons.is_empty() {
            AiDecision::Refuser
        } else if confiance > 0.85 {
            AiDecision::Valider
        } else {
            AiDecision::Incertain
        };

        AiSuggestion {
            decision,
            confiance,
            raisons,
            corrections,
        }
    }

    /// Apprend d'une decision du proprietaire
    pub fn apprendre(&mut self, metrics: &AiMetrics, decision: &str, correction: &str) {
        // Enregistrer l'apprentissage
        self.apprentissages.push(Apprentissage {
            date: chrono::Utc::now().timestamp(),
            metrique: "global".to_string(),
            valeur: metrics.overall_confidence,
            decision_humaine: decision.to_string(),
            correction: correction.to_string(),
        });

        // Ajuster les seuils si necessaire
        // (Pour l'instant, on garde les seuils par defaut)
        // TODO: Ajuster les seuils selon les corrections

        // Augmenter l'autonomie si l'IA avait vu juste
        let proposition = self.proposer(metrics);
        let ia_avait_raison = match (&proposition.decision, decision) {
            (AiDecision::Valider, "VALIDER") => true,
            (AiDecision::Refuser, "REFUSER") => true,
            (AiDecision::Incertain, _) => true,
            _ => false,
        };

        if ia_avait_raison {
            self.autonomie = (self.autonomie + 0.01).min(1.0);
        } else {
            self.autonomie = (self.autonomie - 0.02).max(0.0);
        }
    }
}
