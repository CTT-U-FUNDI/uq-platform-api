//! UQ Preamble Module - Préambule TOTP Furtif
//! Génère un préambule changeant toutes les 5 minutes pour masquer le trafic.
//! Chaque nœud a un préambule unique dérivé de son gène de confiance.

use crate::crypto::sha256;

/// Durée d'une époque TOTP en secondes (5 minutes)
pub const TOTP_EPOCH_DURATION: u64 = 300;

/// Taille du préambule en octets
pub const PREAMBLE_SIZE: usize = 8;

/// Fenêtre de tolérance pour la validation (nombre d'époques)
pub const TOLERANCE_WINDOW: u64 = 2;

/// Structure gérant le préambule TOTP d'un nœud
#[derive(Debug, Clone)]
pub struct UqPreamble {
    /// Entropie secrète du nœud (128 octets, dérivée du gène)
    entropy: [u8; 128],
    /// Dernière époque calculée (cache)
    last_epoch: u64,
    /// Dernier préambule généré (cache)
    last_preamble: [u8; PREAMBLE_SIZE],
}

impl UqPreamble {
    /// Crée un nouveau préambule à partir de l'entropie du gène.
    ///
    /// # Arguments
    /// * `entropy` - 128 octets d'entropie provenant de UqGene::get_entropy()
    pub fn new(entropy: &[u8; 128]) -> Self {
        UqPreamble {
            entropy: *entropy,
            last_epoch: 0,
            last_preamble: [0u8; PREAMBLE_SIZE],
        }
    }

    /// Génère le préambule pour l'époque actuelle.
    ///
    /// # Algorithme
    /// ```text
    /// epoch = timestamp_unix / 300
    /// preamble = SHA256(entropy || epoch)[0..8]
    /// ```
    ///
    /// Le préambule change toutes les 5 minutes (300 secondes).
    pub fn generate_current(&mut self) -> [u8; PREAMBLE_SIZE] {
        let epoch = self.get_current_epoch();

        // Utiliser le cache si l'époque n'a pas changé
        if epoch == self.last_epoch && self.last_preamble != [0u8; PREAMBLE_SIZE] {
            return self.last_preamble;
        }

        let mut input = Vec::with_capacity(136);
        input.extend_from_slice(&self.entropy);
        input.extend_from_slice(&epoch.to_le_bytes());

        let hash = sha256(&input);
        let mut preamble = [0u8; PREAMBLE_SIZE];
        preamble.copy_from_slice(&hash[0..PREAMBLE_SIZE]);

        // Mettre en cache
        self.last_epoch = epoch;
        self.last_preamble = preamble;

        preamble
    }

    /// Valide si un préambule reçu est valide.
    ///
    /// Vérifie pour l'époque actuelle ET les 2 époques adjacentes
    /// (fenêtre de tolérance de ±10 minutes pour la dérive d'horloge).
    ///
    /// # Arguments
    /// * `received_preamble` - Le préambule reçu à vérifier
    ///
    /// # Retourne
    /// `true` si le préambule est valide pour l'époque actuelle ou adjacente
    pub fn validate(&self, received_preamble: &[u8; PREAMBLE_SIZE]) -> bool {
        let current_epoch = self.get_current_epoch();

        // Vérifier l'époque actuelle et les époques adjacentes
        for offset in 0..=TOLERANCE_WINDOW {
            let epoch_to_check = current_epoch - offset;

            let mut input = Vec::with_capacity(136);
            input.extend_from_slice(&self.entropy);
            input.extend_from_slice(&epoch_to_check.to_le_bytes());

            let hash = sha256(&input);
            let expected = &hash[0..PREAMBLE_SIZE];

            if expected == received_preamble {
                return true;
            }
        }

        // Vérifier aussi l'époque suivante (tolérance positive)
        let future_epoch = current_epoch + 1;
        let mut input = Vec::with_capacity(136);
        input.extend_from_slice(&self.entropy);
        input.extend_from_slice(&future_epoch.to_le_bytes());

        let hash = sha256(&input);
        let expected = &hash[0..PREAMBLE_SIZE];

        expected == received_preamble
    }

    /// Retourne l'époque TOTP actuelle.
    ///
    /// # Calcul
    /// ```text
    /// epoch = timestamp_unix / 300
    /// ```
    pub fn get_current_epoch(&self) -> u64 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        now / TOTP_EPOCH_DURATION
    }

    /// Retourne l'entropie utilisée (pour débogage uniquement).
    #[cfg(test)]
    pub fn get_entropy(&self) -> &[u8; 128] {
        &self.entropy
    }
}

/// ─── TESTS ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_entropy() -> [u8; 128] {
        let mut entropy = [0u8; 128];
        for i in 0..128 {
            entropy[i] = i as u8;
        }
        entropy
    }

    #[test]
    fn test_generate_current() {
        let mut preamble = UqPreamble::new(&test_entropy());
        let p1 = preamble.generate_current();
        assert_eq!(p1.len(), PREAMBLE_SIZE);
    }

    #[test]
    fn test_same_epoch_same_preamble() {
        let mut preamble = UqPreamble::new(&test_entropy());
        let p1 = preamble.generate_current();
        let p2 = preamble.generate_current();
        assert_eq!(p1, p2); // Même époque → même préambule
    }

    #[test]
    fn test_different_entropy_different_preamble() {
        let mut e1 = test_entropy();
        let mut e2 = test_entropy();
        e2[0] = 0xFF; // Modifier un octet

        let mut p1 = UqPreamble::new(&e1);
        let mut p2 = UqPreamble::new(&e2);

        assert_ne!(p1.generate_current(), p2.generate_current());
    }

    #[test]
    fn test_validate_own_preamble() {
        let mut preamble = UqPreamble::new(&test_entropy());
        let p = preamble.generate_current();
        assert!(preamble.validate(&p));
    }

    #[test]
    fn test_validate_wrong_preamble() {
        let preamble = UqPreamble::new(&test_entropy());
        let wrong = [0xFF; PREAMBLE_SIZE];
        assert!(!preamble.validate(&wrong));
    }

    #[test]
    fn test_get_current_epoch() {
        let preamble = UqPreamble::new(&test_entropy());
        let epoch = preamble.get_current_epoch();
        assert!(epoch > 0); // L'époque doit être positive
    }
}
