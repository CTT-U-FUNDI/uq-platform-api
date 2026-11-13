// ====================
// ✅ DECISION HUMAINE
// ====================
//
// Le proprietaire decide : Valider, Refuser, EnAttente
// Il coche des cases pour indiquer POURQUOI.
//
// ====================

use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum HumanDecision {
    Valider,
    Refuser,
    EnAttente,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum RejectionReason {
    PhotoIllegible,
    OcrInaccurate,
    FaceMismatch,
    LivenessFailed,
    Minor,
    AgeDiscrepancy,
    DuplicateFace,
    DuplicateDocument,
    TamperingSuspected,
    RecaptureSuspected,
    Autre(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HumanReview {
    pub reviewer_id: String,
    pub decision: Option<HumanDecision>,
    pub reasons: Vec<RejectionReason>,
    pub free_text_note: Option<String>,
    pub reviewed_at: i64,
    pub cases_cochees: Vec<String>,
}
