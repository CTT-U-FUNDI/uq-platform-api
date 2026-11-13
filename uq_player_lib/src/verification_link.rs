use uq_blockchain::verification::{UqVerifier, VerificationStatus};

/// Le pont entre le lecteur UQ et la vérification
pub struct VerificationBridge;

impl VerificationBridge {
    /// Vérifie si un contenu peut être monétisé
    pub fn can_monetize(content_url: &str, uq_link: &str) -> bool {
        let result = UqVerifier::verify(content_url, uq_link);
        result.status.is_monetizable()
    }
    
    /// Vérifie et retourne le statut détaillé
    pub fn verify_content(content_url: &str, uq_link: &str) -> VerificationStatus {
        let result = UqVerifier::verify(content_url, uq_link);
        result.status
    }
    
    /// Vérifie un fichier directement
    pub fn verify_file_content(file: &[u8], uq_link: &str) -> bool {
        let result = UqVerifier::verify_file(file, uq_link);
        result.status.is_monetizable()
    }
}

