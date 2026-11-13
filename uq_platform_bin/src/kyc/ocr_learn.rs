// ====================
// 🧠 OCR LEARN — Moteur d'apprentissage progressif
// ====================
//
// « L'eau circule. L'humain montre. Le moteur apprend. »
//
// 4 phases :
// - OBSERVATION  : 0-100 utilisateurs (l'humain fait tout)
// - APPRENTISSAGE: 100-500 (l'IA propose, l'humain corrige)
// - PRECISION    : 500-1000 (l'IA est precise, l'humain valide)
// - AUTONOMIE    : 1000+ (l'IA est autonome)
//
// ====================

use serde::{Serialize, Deserialize};
use std::collections::HashMap;
use crate::kyc::metrics::AiMetrics;

// ====================
// PHASES
// ====================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Phase {
    Observation,    // 0-100
    Apprentissage,  // 100-500
    Precision,      // 500-1000
    Autonomie,      // 1000+
}

impl Default for Phase {
    fn default() -> Self {
        Phase::Observation
    }
}

// ====================
// PATTERN
// ====================

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Pattern {
    /// Hash perceptuel (64 bits)
    pub image_hash: u64,

    /// Hash SHA-256 de l'image
    pub image_sha256: String,

    /// Embedding (128 dimensions)
    #[serde(default)]
    pub embedding: Vec<f32>,

    /// Texte extrait
    pub texte: String,

    /// Champs
    pub nom: String,
    pub prenom: String,
    pub date_naissance: String,
    pub numero_document: String,

    /// Metriques
    #[serde(default)]
    pub metrics: AiMetrics,

    /// Nombre de fois vu
    pub count: u32,

    /// Confiance (0-1)
    pub confiance: f32,
}

// ====================
// STATS
// ====================

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChampStats {
    pub total: u32,
    pub corrects: u32,
    pub precision: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Stats {
    pub total: u32,
    pub corrects: u32,
    pub precision: f32,
    pub par_champ: HashMap<String, ChampStats>,
}

// ====================
// PROPOSITION
// ====================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proposition {
    pub texte: String,
    pub nom: String,
    pub prenom: String,
    pub date_naissance: String,
    pub numero_document: String,
    pub confiance: f32,
    pub methode: String,  // "hash", "embedding", "exact"
}

// ====================
// MOTEUR
// ====================

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OcrLearn {
    pub patterns: Vec<Pattern>,
    pub total_users: u32,
    pub seuil_autonomie: u32,
    pub stats: Stats,
    pub phase: Phase,
}

impl OcrLearn {
    pub fn new() -> Self {
        Self {
            patterns: Vec::new(),
            total_users: 0,
            seuil_autonomie: 1000,
            stats: Stats::default(),
            phase: Phase::Observation,
        }
    }

    /// Phase 1 : Observer (0-100)
    pub fn observer(
        &mut self,
        image: &[u8],
        texte: &str,
        nom: &str,
        prenom: &str,
        date_naissance: &str,
        numero_document: &str,
        metrics: &AiMetrics,
    ) {
        let pattern = Pattern {
            image_hash: self.calculer_hash(image),
            image_sha256: self.calculer_sha256(image),
            embedding: self.calculer_embedding(image),
            texte: texte.to_string(),
            nom: nom.to_string(),
            prenom: prenom.to_string(),
            date_naissance: date_naissance.to_string(),
            numero_document: numero_document.to_string(),
            metrics: metrics.clone(),
            count: 1,
            confiance: 1.0,
        };

        self.patterns.push(pattern);
        self.total_users += 1;

        // Passer en phase suivante ?
        if self.total_users >= 100 && self.phase == Phase::Observation {
            self.phase = Phase::Apprentissage;
            eprintln!("🧠 Phase -> APPRENTISSAGE (100 utilisateurs)");
        }
    }

    /// Phase 2+ : Proposer
    pub fn proposer(&self, image: &[u8]) -> Option<Proposition> {
        if self.phase == Phase::Observation {
            return None;  // L'humain fait tout
        }

        // 1. Hash perceptuel d'abord
        let hash = self.calculer_hash(image);
        if let Some(pattern) = self.patterns.iter().find(|p| p.image_hash == hash) {
            return Some(Proposition {
                texte: pattern.texte.clone(),
                nom: pattern.nom.clone(),
                prenom: pattern.prenom.clone(),
                date_naissance: pattern.date_naissance.clone(),
                numero_document: pattern.numero_document.clone(),
                confiance: pattern.confiance,
                methode: "hash".to_string(),
            });
        }

        // 2. SHA-256
        let sha = self.calculer_sha256(image);
        if let Some(pattern) = self.patterns.iter().find(|p| p.image_sha256 == sha) {
            return Some(Proposition {
                texte: pattern.texte.clone(),
                nom: pattern.nom.clone(),
                prenom: pattern.prenom.clone(),
                date_naissance: pattern.date_naissance.clone(),
                numero_document: pattern.numero_document.clone(),
                confiance: pattern.confiance * 0.95,
                methode: "sha256".to_string(),
            });
        }

        // 3. Embedding
        let embedding = self.calculer_embedding(image);
        if !embedding.is_empty() {
            let meilleur = self.patterns.iter()
                .filter(|p| !p.embedding.is_empty())
                .map(|p| (p, distance_cosinus(&p.embedding, &embedding)))
                .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap());

            if let Some((pattern, dist)) = meilleur {
                if dist < 0.3 {
                    return Some(Proposition {
                        texte: pattern.texte.clone(),
                        nom: pattern.nom.clone(),
                        prenom: pattern.prenom.clone(),
                        date_naissance: pattern.date_naissance.clone(),
                        numero_document: pattern.numero_document.clone(),
                        confiance: (1.0 - dist) * pattern.confiance,
                        methode: "embedding".to_string(),
                    });
                }
            }
        }

        None
    }

    /// Phase 2+ : Apprendre d'une correction
    pub fn apprendre(&mut self, image: &[u8], proposition: &str, correction: &str) {
        let hash = self.calculer_hash(image);

        if let Some(pattern) = self.patterns.iter_mut().find(|p| p.image_hash == hash) {
            if pattern.texte != correction {
                pattern.texte = correction.to_string();
                pattern.confiance = (pattern.confiance - 0.1).max(0.0);
            } else {
                pattern.confiance = (pattern.confiance + 0.05).min(1.0);
            }
            pattern.count += 1;
        }

        // Stats
        self.stats.total += 1;
        if proposition == correction {
            self.stats.corrects += 1;
        }
        self.stats.precision = if self.stats.total > 0 {
            self.stats.corrects as f32 / self.stats.total as f32
        } else {
            0.0
        };

        // Passer en phase suivante ?
        if self.total_users >= 500 && self.phase == Phase::Apprentissage {
            self.phase = Phase::Precision;
            eprintln!("🧠 Phase -> PRECISION (500 utilisateurs)");
        }
        if self.total_users >= 1000 && self.phase == Phase::Precision {
            self.phase = Phase::Autonomie;
            eprintln!("🧠 Phase -> AUTONOMIE (1000 utilisateurs)");
        }
    }

    /// Est autonome ?
    pub fn est_autonome(&self) -> bool {
        self.total_users >= self.seuil_autonomie && self.stats.precision >= 0.95
    }

    /// Hash perceptuel (8x8 → 64 bits)
    fn calculer_hash(&self, image: &[u8]) -> u64 {
        // Version simplifiee : hash des bytes
        // En production : réduire l'image a 8x8, puis comparer a la moyenne
        let mut hash: u64 = 0;
        for (i, &byte) in image.iter().take(64).enumerate() {
            if byte > 128 {
                hash |= 1 << i;
            }
        }
        hash
    }

    /// SHA-256 de l'image
    fn calculer_sha256(&self, image: &[u8]) -> String {
        use sha2::{Sha256, Digest};
        let mut hasher = Sha256::new();
        hasher.update(image);
        format!("{:x}", hasher.finalize())
    }

    /// Embedding (128 dimensions) — CNN simple
    fn calculer_embedding(&self, image: &[u8]) -> Vec<f32> {
        // Version simplifiee : features basiques
        // En production : vrai CNN
        let mut embedding = vec![0.0f32; 128];

        // Feature 1-64 : distribution des bytes
        for (i, &byte) in image.iter().take(64).enumerate() {
            embedding[i] = byte as f32 / 255.0;
        }

        // Feature 65-128 : gradients
        for i in 1..64 {
            if i < image.len() {
                embedding[64 + i] = (image[i] as f32 - image[i-1] as f32) / 255.0;
            }
        }

        embedding
    }
}

/// Distance cosinus entre deux vecteurs
fn distance_cosinus(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 1.0;
    }

    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

    if norm_a == 0.0 || norm_b == 0.0 {
        return 1.0;
    }

    1.0 - (dot / (norm_a * norm_b))
}

// ====================
// QUESTIONS A L'IA
// ====================

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QuestionIa {
    pub id: String,
    pub question: String,
    pub champs: Vec<String>,
    pub reponse_attendue: String,
}

pub fn questions_predefinies() -> Vec<QuestionIa> {
    vec![
        QuestionIa {
            id: "nom_correct".to_string(),
            question: "Le nom est-il correct ?".to_string(),
            champs: vec!["nom".to_string()],
            reponse_attendue: "oui".to_string(),
        },
        QuestionIa {
            id: "prenom_correct".to_string(),
            question: "Le prenom est-il correct ?".to_string(),
            champs: vec!["prenom".to_string()],
            reponse_attendue: "oui".to_string(),
        },
        QuestionIa {
            id: "date_lisible".to_string(),
            question: "La date de naissance est-elle lisible ?".to_string(),
            champs: vec!["date_naissance".to_string()],
            reponse_attendue: "oui".to_string(),
        },
        QuestionIa {
            id: "numero_valide".to_string(),
            question: "Le numero de document est-il valide ?".to_string(),
            champs: vec!["numero_document".to_string()],
            reponse_attendue: "oui".to_string(),
        },
        QuestionIa {
            id: "photo_lisible".to_string(),
            question: "La photo est-elle lisible ?".to_string(),
            champs: vec!["photo".to_string()],
            reponse_attendue: "oui".to_string(),
        },
        QuestionIa {
            id: "visage_correspond".to_string(),
            question: "Le visage correspond-il ?".to_string(),
            champs: vec!["face_comparison".to_string()],
            reponse_attendue: "oui".to_string(),
        },
        QuestionIa {
            id: "age_valide".to_string(),
            question: "L'age est-il >= 13 ans ?".to_string(),
            champs: vec!["age".to_string()],
            reponse_attendue: "oui".to_string(),
        },
        QuestionIa {
            id: "pas_doublon".to_string(),
            question: "Aucun doublon detecte ?".to_string(),
            champs: vec!["anomalies".to_string()],
            reponse_attendue: "oui".to_string(),
        },
    ]
}
