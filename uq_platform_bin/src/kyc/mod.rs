// ====================
// 🪪 KYC — Verification d'identite (IA + humain)
// ====================
//
// « L'eau circule. La verite aussi. »
//
// Modules :
// - dossier  : Le dossier complet (infos + photo + video)
// - queue    : La file d'attente (1 par 1)
// - metrics  : Les 6 mesures (document, OCR, visage, vivacite, age, anomalies)
// - decision : La decision humaine
// - training : L'enregistrement d'apprentissage
// - storage  : SQLite
//
// ====================

pub mod dossier;
pub mod queue;
pub mod metrics;
pub mod decision;
pub mod training;
pub mod storage;
pub mod ia;
pub mod metrics_calc;
pub mod ocr_learn;
pub mod cadenas;

pub use dossier::{KycDossier, KycStatus};
pub use queue::KycQueue;
pub use decision::{HumanDecision, HumanReview, RejectionReason};
pub use storage::KycStore;




