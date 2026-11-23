// ====================
// 🧠 TRAINING — Enregistrement d'apprentissage
// ====================
//
// Chaque dossier validé genere un enregistrement :
// - Ce que l'IA a vu (metriques)
// - Ce que l'humain a decide
// - Les raisons
//
// ====================

use crate::kyc::decision::{HumanDecision, HumanReview};
use crate::kyc::metrics::AiMetrics;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TrainingRecord {
    pub record_id: String,
    pub application_id: String,
    pub created_at: i64,

    /// Metriques IA avant decision
    pub ai_metrics: AiMetrics,
    pub ai_suggested_decision: Option<HumanDecision>,
    pub ai_confidence: f32,

    /// Decision humaine
    pub human_review: HumanReview,

    /// L'IA avait-elle vu juste ?
    pub ai_was_correct: bool,
}
