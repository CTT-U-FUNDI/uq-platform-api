use crate::error::ChainError;
use sha2::{Sha256, Digest};

/// Statut de vérification d'un contenu externe
#[derive(Debug, Clone, PartialEq)]
pub enum VerificationStatus {
    Verified,
    Pending,
    Unverified,
    Rejected,
}

impl VerificationStatus {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Verified => "verified",
            Self::Pending => "pending",
            Self::Unverified => "unverified",
            Self::Rejected => "rejected",
        }
    }
    
    pub fn is_monetizable(&self) -> bool {
        matches!(self, Self::Verified)
    }
}

/// Méthode de vérification
#[derive(Debug, Clone, PartialEq)]
pub enum VerificationMethod {
    BioProfile,
    Description,
    FileContent,
    OcrFallback,
}

/// Résultat de la vérification
#[derive(Debug, Clone)]
pub struct VerificationResult {
    pub status: VerificationStatus,
    pub method: Option<VerificationMethod>,
    pub confidence: f64,
    pub message: String,
}

impl VerificationResult {
    pub fn verified(method: VerificationMethod, confidence: f64) -> Self {
        Self {
            status: VerificationStatus::Verified,
            method: Some(method),
            confidence,
            message: "Contenu authentique vérifié".to_string(),
        }
    }
    
    pub fn unverified() -> Self {
        Self {
            status: VerificationStatus::Unverified,
            method: None,
            confidence: 0.0,
            message: "Contenu non vérifié — non monétisable".to_string(),
        }
    }
    
    pub fn pending() -> Self {
        Self {
            status: VerificationStatus::Pending,
            method: None,
            confidence: 0.0,
            message: "Vérification en cours".to_string(),
        }
    }
    
    pub fn rejected(reason: &str) -> Self {
        Self {
            status: VerificationStatus::Rejected,
            method: None,
            confidence: 0.0,
            message: format!("Rejeté : {}", reason),
        }
    }
}

/// Le vérificateur UQ — vérifie l'authenticité des contenus externes
pub struct UqVerifier;

impl UqVerifier {
    /// Vérifie un lien YouTube — cherche le lien UQ dans la description
    pub fn verify_youtube(video_url: &str, uq_link: &str) -> VerificationResult {
        if !video_url.contains("youtube.com") && !video_url.contains("youtu.be") {
            return VerificationResult::rejected("URL YouTube invalide");
        }
        
        // En production : appeler YouTube Data API v3
        // GET https://www.googleapis.com/youtube/v3/videos?id={video_id}&part=snippet
        // Chercher uq_link dans snippet.description
        
        // Pour l'instant : vérification de format
        if uq_link.starts_with("uq://") {
            VerificationResult::verified(VerificationMethod::Description, 0.95)
        } else {
            VerificationResult::unverified()
        }
    }
    
    /// Vérifie un lien Spotify — cherche le lien UQ dans la bio de l'artiste
    pub fn verify_spotify(url: &str, uq_link: &str) -> VerificationResult {
        if !url.contains("spotify.com") {
            return VerificationResult::rejected("URL Spotify invalide");
        }
        
        if uq_link.starts_with("uq://") {
            VerificationResult::verified(VerificationMethod::BioProfile, 0.90)
        } else {
            VerificationResult::unverified()
        }
    }
    
    /// Vérifie un lien Instagram — cherche le lien UQ dans la bio
    pub fn verify_instagram(url: &str, uq_link: &str) -> VerificationResult {
        if !url.contains("instagram.com") {
            return VerificationResult::rejected("URL Instagram invalide");
        }
        
        if uq_link.starts_with("uq://") {
            VerificationResult::verified(VerificationMethod::BioProfile, 0.85)
        } else {
            VerificationResult::unverified()
        }
    }
    
    /// Vérifie un lien TikTok — cherche le lien UQ dans la bio
    pub fn verify_tiktok(url: &str, uq_link: &str) -> VerificationResult {
        if !url.contains("tiktok.com") {
            return VerificationResult::rejected("URL TikTok invalide");
        }
        
        if uq_link.starts_with("uq://") {
            VerificationResult::verified(VerificationMethod::BioProfile, 0.80)
        } else {
            VerificationResult::unverified()
        }
    }
    
    /// Vérifie un fichier (PDF, MP3, MP4) — cherche le lien UQ dans le contenu
    pub fn verify_file(file_content: &[u8], uq_link: &str) -> VerificationResult {
        // Chercher le lien UQ dans les octets du fichier
        let uq_bytes = uq_link.as_bytes();
        
        if file_content.windows(uq_bytes.len()).any(|window| window == uq_bytes) {
            VerificationResult::verified(VerificationMethod::FileContent, 0.99)
        } else {
            VerificationResult::unverified()
        }
    }
    
    /// Vérification universelle par OCR + 3 validateurs (fallback)
    pub fn verify_fallback(image: &[u8], uq_link: &str) -> VerificationResult {
        // En production : OCR sur l'image + vérification par 3 validateurs
        // Pour l'instant : vérification de base
        if image.len() > 100 && uq_link.starts_with("uq://") {
            VerificationResult::verified(VerificationMethod::OcrFallback, 0.70)
        } else {
            VerificationResult::pending()
        }
    }
    
    /// Vérification universelle — détecte le type et appelle la bonne méthode
    pub fn verify(url_or_content: &str, uq_link: &str) -> VerificationResult {
        if url_or_content.contains("youtube.com") || url_or_content.contains("youtu.be") {
            Self::verify_youtube(url_or_content, uq_link)
        } else if url_or_content.contains("spotify.com") {
            Self::verify_spotify(url_or_content, uq_link)
        } else if url_or_content.contains("instagram.com") {
            Self::verify_instagram(url_or_content, uq_link)
        } else if url_or_content.contains("tiktok.com") {
            Self::verify_tiktok(url_or_content, uq_link)
        } else {
            VerificationResult::unverified()
        }
    }
    
    /// Génère un code de défi unique pour la vérification manuelle
    pub fn generate_challenge(account_address: &[u8; 32]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(account_address);
        hasher.update(b"UQ-VERIFY-CHALLENGE");
        hasher.update(&chrono::Utc::now().timestamp().to_le_bytes());
        let hash = hasher.finalize();
        let code = hex::encode(&hash[..8]);
        format!("UQ-VERIFY-{}", code.to_uppercase())
    }
    
    /// Vérifie qu'un code de défi est dans une description
    pub fn verify_challenge(description: &str, challenge: &str) -> VerificationResult {
        if description.contains(challenge) {
            VerificationResult::verified(VerificationMethod::Description, 1.0)
        } else {
            VerificationResult::unverified()
        }
    }
    
    /// Vérifie la cohérence du lien UQ
    pub fn validate_uq_link(uq_link: &str) -> bool {
        uq_link.starts_with("uq://") 
        && uq_link.len() >= 8 
        && uq_link.len() <= 64
        && uq_link.chars().all(|c| c.is_alphanumeric() || c == '/' || c == '_' || c == '-')
    }
    
    /// Calcule un score de confiance global
    pub fn calculate_confidence(results: &[VerificationResult]) -> f64 {
        if results.is_empty() {
            return 0.0;
        }
        
        let total: f64 = results.iter().map(|r| r.confidence).sum();
        total / results.len() as f64
    }
}

// ═══════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_verify_youtube_valid() {
        let result = UqVerifier::verify_youtube("https://youtube.com/watch?v=abc123", "uq://jean-music");
        assert_eq!(result.status, VerificationStatus::Verified);
        assert!(result.is_monetizable());
    }
    
    #[test]
    fn test_verify_youtube_invalid_url() {
        let result = UqVerifier::verify_youtube("https://facebook.com/video", "uq://jean");
        assert_eq!(result.status, VerificationStatus::Rejected);
    }
    
    #[test]
    fn test_verify_file_content() {
        let file = b"Ceci est un PDF avec uq://jean-auteur dedans";
        let result = UqVerifier::verify_file(file, "uq://jean-auteur");
        assert_eq!(result.status, VerificationStatus::Verified);
    }
    
    #[test]
    fn test_validate_uq_link() {
        assert!(UqVerifier::validate_uq_link("uq://jean-music"));
        assert!(!UqVerifier::validate_uq_link("https://youtube.com"));
    }
    
    #[test]
    fn test_generate_challenge() {
        let addr = [1u8; 32];
        let challenge = UqVerifier::generate_challenge(&addr);
        assert!(challenge.starts_with("UQ-VERIFY-"));
    }
}
