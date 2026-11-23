// ====================
// 📊 METRICS KYC — Les 6 mesures de l'IA
// ====================
//
// Ces metriques sont calculees (ou estimees) pour chaque dossier.
// Elles servent a l'apprentissage progressif de l'IA.
//
// ====================

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AiMetrics {
    // 1. Qualite du document
    pub document_quality: DocumentQuality,

    // 2. Confiance OCR
    pub ocr_confidence: OcrConfidence,

    // 3. Comparaison faciale
    pub face_comparison: FaceComparison,

    // 4. Vivacite
    pub liveness: Liveness,

    // 5. Age
    pub age_estimation: AgeEstimation,

    // 6. Anomalies
    pub anomalies: Anomalies,

    // Score global
    pub overall_confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DocumentQuality {
    pub sharpness: f32,       // 0-1
    pub brightness: f32,      // 0-1
    pub contrast: f32,        // 0-1
    pub skew_angle_deg: f32,  // degres
    pub glare_ratio: f32,     // 0-1
    pub noise_level: f32,     // 0-1
    pub composite_score: f32, // 0-1
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OcrConfidence {
    pub mean_char_confidence: f32,
    pub min_char_confidence: f32,
    pub composite_score: f32,
    pub ocr_used: bool, // L'OCR a-t-il ete utilise ?
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FaceComparison {
    pub face_detected_in_document: bool,
    pub face_detected_in_video: bool,
    pub similarity_score: f32,
    pub composite_score: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Liveness {
    pub challenge_completed: bool,
    pub movement_naturalness: f32,
    pub composite_score: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgeEstimation {
    pub estimated_face_age: f32,
    pub document_age: Option<f32>,
    pub age_discrepancy: f32,
    pub composite_score: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Anomalies {
    pub duplicate_face_found: bool,
    pub duplicate_document_found: bool,
    pub tampering_suspected: bool,
    pub recapture_suspected: bool,
    pub composite_score: f32,
}

impl AiMetrics {
    pub fn recompute_all(&mut self) {
        // Calcul simplifie pour l'instant
        // Les vraies valeurs seront calculees par les futurs modules

        self.document_quality.composite_score = (self.document_quality.sharpness
            + self.document_quality.brightness
            + self.document_quality.contrast)
            / 3.0;

        self.overall_confidence = self.document_quality.composite_score * 0.30
            + self.ocr_confidence.composite_score * 0.20
            + self.face_comparison.composite_score * 0.25
            + self.liveness.composite_score * 0.15
            + self.age_estimation.composite_score * 0.05
            + self.anomalies.composite_score * 0.05;
    }
}
