// ====================
// 📁 DOSSIER KYC
// ====================
//
// Un dossier = les infos du compte + photo identite + video selfie
//
// ====================

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum KycStatus {
    EnAttente,
    Verifie,
    Refuse,
}

impl Default for KycStatus {
    fn default() -> Self {
        KycStatus::EnAttente
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KycDossier {
    pub uq_number: String,
    pub nom: String,
    pub prenom: String,
    pub date_naissance: String,
    pub pays: String,
    pub adresse: String,
    pub telephone: String,
    pub document_number: String,

    /// Chemins des fichiers
    pub photo_identite_path: Option<String>,
    pub video_selfie_path: Option<String>,

    /// Selfie guide produit par l'assistant KYC (photo, pas un film)
    #[serde(default)]
    pub selfie_path: Option<String>,

    /// Bilan du controle de vivacite (nonce retrouve, visages, mouvement)
    #[serde(default)]
    pub vivacite: Option<serde_json::Value>,

    /// Statut
    pub statut: KycStatus,

    /// Dates
    pub soumis_at: i64,
    pub decision_at: Option<i64>,

    /// Decision
    pub decision_par: Option<String>, // "proprietaire" ou "ia"
    pub cases_cochees: Vec<String>,
    pub raison_refus: Option<String>,

    /// Metriques IA calculees
    #[serde(default)]
    pub ai_metrics: crate::kyc::metrics::AiMetrics,

    /// Suggestion de l'IA
    #[serde(default)]
    pub ai_suggestion: Option<String>,

    /// Confiance de l'IA
    #[serde(default)]
    pub ai_confidence: Option<f32>,
}

impl KycDossier {
    pub fn new(uq_number: String) -> Self {
        Self {
            uq_number,
            statut: KycStatus::EnAttente,
            soumis_at: chrono::Utc::now().timestamp(),
            ..Default::default()
        }
    }

    /// Anonymise le dossier après décision : purge TOUTES les données
    /// personnelles (nom, document, photos, vidéo, vivacité). Il ne
    /// reste que le UQ, le statut, les dates et la décision — la
    /// preuve de vérification vit dans l'index sous forme de hash.
    pub fn anonymiser(&mut self) {
        self.nom.clear();
        self.prenom.clear();
        self.date_naissance.clear();
        self.pays.clear();
        self.adresse.clear();
        self.telephone.clear();
        self.document_number.clear();
        self.photo_identite_path = None;
        self.video_selfie_path = None;
        self.selfie_path = None;
        self.vivacite = None;
        self.ai_metrics = Default::default();
        self.ai_suggestion = None;
        self.ai_confidence = None;
        self.cases_cochees.clear();
    }

    pub fn age(&self) -> Option<u32> {
        // Calculer l'age a partir de date_naissance
        if self.date_naissance.is_empty() {
            return None;
        }
        let birth = chrono::NaiveDate::parse_from_str(&self.date_naissance, "%Y-%m-%d").ok()?;
        let today = chrono::Utc::now().date_naive();
        let age = today.signed_duration_since(birth).num_days() / 365;
        Some(age as u32)
    }
}
