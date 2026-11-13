//! UQ Gene Module - Gène de Confiance Évolutif
//! Chaque nœud a un gène qui mesure sa fiabilité de 0.0 (méfiance) à 1.0 (confiance totale).
//! Le gène évolue à chaque interaction : plus un nœud est fiable, plus son score augmente.

use crate::crypto::sha256;
use crate::error::UqError;
use chrono::Utc;

/// Structure représentant le gène de confiance d'un nœud
#[derive(Debug, Clone)]
pub struct UqGene {
    /// Adresse UQ du nœud
    pub address: String,
    /// Clé publique du nœud (32 octets)
    pub public_key: [u8; 32],
    /// Version du gène (incrémentée à chaque évolution)
    pub version: u32,
    /// Score de confiance actuel (0.0 = méfiance, 1.0 = confiance totale)
    /// Score de confiance actuel (0.0 = méfiance, 1.0 = confiance totale)
    pub trust_level: f64,
    /// Nombre d'activités enregistrées (messages, relais, etc.)
    pub activity_count: u64,
    /// Disponibilité sur les dernières 24h (0.0 à 1.0)
    pub uptime_24h: f64,
    /// Réputation sur la blockchain (0.0 à 1.0)
    pub reputation_onchain: f64,
    /// Bande passante disponible (0.0 à 1.0)
    pub bandwidth_available: f64,
    /// Stockage disponible (0.0 à 1.0)
    pub storage_available: f64,
    /// Niveau de batterie (0.0 à 1.0)
    pub battery_level: f64,
    /// Timestamp de création (UNIX)
    pub created_at: i64,
    /// Timestamp de dernière mise à jour (UNIX)
    pub updated_at: i64,
}

impl UqGene {
    /// Crée un nouveau gène avec un score neutre (0.5).
    ///
    /// # Arguments
    /// * `address` - Adresse UQ du nœud
    /// * `public_key` - Clé publique Ed25519 (32 octets)
    pub fn new(address: &str, public_key: &[u8; 32]) -> Result<Self, UqError> {
        if address.is_empty() {
            return Err(UqError::InvalidAddress("Adresse vide".to_string()));
        }

        let now = Utc::now().timestamp();

        Ok(UqGene {
            address: address.to_string(),
            public_key: *public_key,
            version: 0,
            trust_level: 0.5,       // Neutre au départ
            activity_count: 0,
            uptime_24h: 0.5,        // Neutre
            reputation_onchain: 0.5, // Neutre
            bandwidth_available: 0.5,
            storage_available: 0.5,
            battery_level: 0.5,
            created_at: now,
            updated_at: now,
        })
    }

    /// Enregistre une activité (message envoyé, relais effectué, etc.).
    /// Incrémente le compteur d'activité.
    pub fn record_activity(&mut self, count: u64) {
        self.activity_count = self.activity_count.saturating_add(count);
        self.updated_at = Utc::now().timestamp();
    }

    /// Calcule le niveau de confiance actuel basé sur tous les indicateurs.
    ///
    /// # Formule
    /// ```text
    /// trust = (uptime_24h * 0.30 +
    ///          reputation_onchain * 0.25 +
    ///          bandwidth_available * 0.15 +
    ///          storage_available * 0.10 +
    ///          battery_level * 0.10 +
    ///          min(activity_count / 1000, 1.0) * 0.10) / 1.0
    /// ```
    pub fn calculate_trust(&self) -> f64 {
        let activity_score = (self.activity_count as f64 / 1000.0).min(1.0);

        let trust = 
            self.uptime_24h * 0.30 +
            self.reputation_onchain * 0.25 +
            self.bandwidth_available * 0.15 +
            self.storage_available * 0.10 +
            self.battery_level * 0.10 +
            activity_score * 0.10;

        // Clamp entre 0.0 et 1.0
        trust.clamp(0.0, 1.0)
    }

    /// Fait évoluer le gène : recalcule le score et incrémente la version.
    ///
    /// # Effets
    /// - Si trust > 0.9 : le nœud est "haute confiance" (peut devenir validateur)
    /// - Si trust < 0.3 : le nœud est "basse confiance" (limité en capacités)
    /// - Entre 0.3 et 0.9 : fonctionnement normal
    ///
    /// # Retourne
    /// La nouvelle version du gène
    pub fn evolve(&mut self) -> u32 {
        self.version += 1;
        self.trust_level = self.calculate_trust();
        self.updated_at = Utc::now().timestamp();
        self.version
    }

    /// Génère 128 octets d'entropie déterministe basés sur le gène.
    /// Utilisé par UqPreamble pour le TOTP furtif.
    ///
    /// # Algorithme
    /// SHA256(version || trust_level || address || updated_at)
    /// répété 4 fois pour obtenir 128 octets.
    pub fn get_entropy(&self) -> [u8; 128] {
        let seed = format!(
            "{}{}{}{}",
            self.version,
            self.trust_level,
            self.address,
            self.updated_at
        );

        let mut entropy = [0u8; 128];
        let hash = sha256(seed.as_bytes());
        entropy[0..32].copy_from_slice(&hash);
        entropy[32..64].copy_from_slice(&hash);
        entropy[64..96].copy_from_slice(&hash);
        entropy[96..128].copy_from_slice(&hash);
        entropy
    }

    /// Met à jour les métriques du nœud.
    pub fn update_metrics(
        &mut self,
        uptime: f64,
        reputation: f64,
        bandwidth: f64,
        storage: f64,
        battery: f64,
    ) {
        self.uptime_24h = uptime.clamp(0.0, 1.0);
        self.reputation_onchain = reputation.clamp(0.0, 1.0);
        self.bandwidth_available = bandwidth.clamp(0.0, 1.0);
        self.storage_available = storage.clamp(0.0, 1.0);
        self.battery_level = battery.clamp(0.0, 1.0);
        self.updated_at = Utc::now().timestamp();
    }

    /// Retourne le niveau de confiance actuel (sans recalculer).
    pub fn get_trust_level(&self) -> f64 {
        self.trust_level
    }

    /// Retourne la version actuelle du gène.
    pub fn get_version(&self) -> u32 {
        self.version
    }

    /// Retourne l'adresse UQ associée.
    pub fn get_address(&self) -> &str {
        &self.address
    }

    /// Vérifie si le nœud est considéré comme "haute confiance".
    pub fn is_high_trust(&self) -> bool {
        self.trust_level >= 0.9
    }

    /// Vérifie si le nœud est considéré comme "basse confiance".
    pub fn is_low_trust(&self) -> bool {
        self.trust_level < 0.3
    }
}

/// ─── TESTS ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::generate_keypair;

    fn test_gene() -> UqGene {
        let kp = generate_keypair();
        UqGene::new("uq:test", &kp.public_key).unwrap_or_default()
    }

    #[test]
    fn test_new_gene_defaults() {
        let gene = test_gene();
        assert_eq!(gene.version, 0);
        assert_eq!(gene.trust_level, 0.5);
        assert_eq!(gene.activity_count, 0);
    }

    #[test]
    fn test_record_activity() {
        let mut gene = test_gene();
        gene.record_activity(100);
        assert_eq!(gene.activity_count, 100);
        gene.record_activity(50);
        assert_eq!(gene.activity_count, 150);
    }

    #[test]
    fn test_calculate_trust_neutral() {
        let gene = test_gene();
        let trust = gene.calculate_trust();
        // Tous les scores à 0.5 → 0.5 * (0.30+0.25+0.15+0.10+0.10) + 0 * 0.10 = 0.45
        assert!(trust >= 0.4 && trust <= 0.6);
    }

    #[test]
    fn test_calculate_trust_high() {
        let mut gene = test_gene();
        gene.update_metrics(1.0, 1.0, 1.0, 1.0, 1.0);
        gene.activity_count = 2000;
        let trust = gene.calculate_trust();
        assert!(trust > 0.9);
    }

    #[test]
    fn test_calculate_trust_low() {
        let mut gene = test_gene();
        gene.update_metrics(0.0, 0.0, 0.0, 0.0, 0.0);
        gene.activity_count = 0;
        let trust = gene.calculate_trust();
        assert!(trust < 0.1);
    }

    #[test]
    fn test_evolve_increments_version() {
        let mut gene = test_gene();
        let v1 = gene.evolve();
        assert_eq!(v1, 1);
        let v2 = gene.evolve();
        assert_eq!(v2, 2);
    }

    #[test]
    fn test_get_entropy_length() {
        let gene = test_gene();
        let entropy = gene.get_entropy();
        assert_eq!(entropy.len(), 128);
    }

    #[test]
    fn test_get_entropy_deterministic() {
        let gene = test_gene();
        let e1 = gene.get_entropy();
        let e2 = gene.get_entropy();
        assert_eq!(e1, e2); // Même gène → même entropie
    }

    #[test]
    fn test_high_trust_detection() {
        let mut gene = test_gene();
        gene.update_metrics(1.0, 1.0, 1.0, 1.0, 1.0);
        gene.activity_count = 2000;
        gene.evolve();
        assert!(gene.is_high_trust());
    }

    #[test]
    fn test_low_trust_detection() {
        let mut gene = test_gene();
        gene.update_metrics(0.0, 0.0, 0.0, 0.0, 0.0);
        gene.activity_count = 0;
        gene.evolve();
        assert!(gene.is_low_trust());
    }

    #[test]
    fn test_different_genes_different_entropy() {
        let mut gene1 = test_gene();
        let mut gene2 = test_gene();
        gene2.record_activity(1);
        gene2.evolve();
        assert_ne!(gene1.get_entropy(), gene2.get_entropy());
    }
}

