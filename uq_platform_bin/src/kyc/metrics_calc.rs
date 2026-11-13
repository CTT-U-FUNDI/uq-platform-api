// ====================
// 📊 CALCUL DES METRIQUES KYC (simule pour l'instant)
// ====================
//
// Ces fonctions calculent les metriques a partir de la photo/video.
// Pour l'instant, elles SIMULENT les valeurs.
// Plus tard, on branchera les vrais calculs.
//
// ====================

use crate::kyc::metrics::{AiMetrics, DocumentQuality, OcrConfidence, FaceComparison, Liveness, AgeEstimation, Anomalies};

/// Calcule TOUTES les metriques a partir de la photo et video
pub fn calculer_metriques(photo: &[u8], video: Option<&[u8]>) -> AiMetrics {
    let mut metrics = AiMetrics::default();

    // ===== 1. Qualite du document =====
    metrics.document_quality = DocumentQuality {
        sharpness: simuler_valeur(0.3, 0.95),
        brightness: simuler_valeur(0.2, 0.9),
        contrast: simuler_valeur(0.3, 0.9),
        skew_angle_deg: simuler_valeur(0.0, 10.0),
        glare_ratio: simuler_valeur(0.0, 0.2),
        noise_level: simuler_valeur(0.0, 0.4),
        composite_score: 0.0,  // Sera calcule
    };

    // ===== 2. OCR =====
    metrics.ocr_confidence = OcrConfidence {
        mean_char_confidence: simuler_valeur(0.4, 0.95),
        min_char_confidence: simuler_valeur(0.2, 0.8),
        composite_score: 0.0,
        ocr_used: true,
    };

    // ===== 3. Visage =====
    metrics.face_comparison = FaceComparison {
        face_detected_in_document: true,
        face_detected_in_video: video.is_some(),
        similarity_score: simuler_valeur(0.5, 0.95),
        composite_score: 0.0,
    };

    // ===== 4. Vivacite =====
    metrics.liveness = Liveness {
        challenge_completed: video.is_some(),
        movement_naturalness: simuler_valeur(0.5, 0.95),
        composite_score: simuler_valeur(0.5, 0.95),
    };

    // ===== 5. Age =====
    metrics.age_estimation = AgeEstimation {
        estimated_face_age: simuler_valeur(15.0, 45.0),
        document_age: Some(simuler_valeur(15.0, 45.0)),
        age_discrepancy: simuler_valeur(0.0, 3.0),
        composite_score: 0.0,
    };

    // ===== 6. Anomalies =====
    metrics.anomalies = Anomalies {
        duplicate_face_found: false,
        duplicate_document_found: false,
        tampering_suspected: false,
        recapture_suspected: false,
        composite_score: 0.9,
    };

    // Calculer les composites
    metrics.recompute_all();

    metrics
}

/// Simule une valeur dans un intervalle
fn simuler_valeur(min: f32, max: f32) -> f32 {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    rng.gen_range(min..max)
}
