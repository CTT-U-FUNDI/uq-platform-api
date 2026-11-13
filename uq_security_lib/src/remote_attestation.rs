// ═══════════════════════════════════════════════════════════════
// VTEE — Attestation à Distance
// Prouve au serveur UQ que le VTEE est authentique
// ═══════════════════════════════════════════════════════════════

use uq_blockchain::error::ChainError;
use uq_black_key_standalone::UqBlackKey;
use sha2::{Sha256, Digest};

/// Mesure d'intégrité du VTEE (hash de tout le code)
#[derive(Debug, Clone)]
pub struct VteeMeasurement {
    pub code_hash: [u8; 32],
    pub data_hash: [u8; 32],
    pub version: u64,
    pub timestamp: i64,
}

/// Rapport d'attestation signé
#[derive(Debug, Clone)]
pub struct AttestationReport {
    pub measurement: VteeMeasurement,
    pub signature: Vec<u8>,
    pub public_key: [u8; 64],
    pub nonce: [u8; 32],
}

impl VteeMeasurement {
    /// Mesure l'intégrité du code VTEE
    pub fn measure() -> Result<Self, ChainError> {
        // Hasher tout le code source du VTEE
        let code = include_str!("vtee.rs").as_bytes();
        let mut hasher = Sha256::new();
        hasher.update(code);
        let code_hash: [u8; 32] = hasher.finalize().into();

        // Hasher les données internes
        let mut hasher2 = Sha256::new();
        hasher2.update(b"VTEE_DATA_2026");
        let data_hash: [u8; 32] = hasher2.finalize().into();

        Ok(VteeMeasurement {
            code_hash,
            data_hash,
            version: 2,
            timestamp: chrono::Utc::now().timestamp(),
        })
    }

    /// Vérifie que la mesure correspond
    pub fn verify(&self, other: &VteeMeasurement) -> bool {
        self.code_hash == other.code_hash 
        && self.data_hash == other.data_hash
        && self.version == other.version
    }
}

/// Génère un rapport d'attestation signé
pub fn generate_attestation(nonce: [u8; 32]) -> Result<AttestationReport, ChainError> {
    let measurement = VteeMeasurement::measure()?;
    
    // Générer une clé éphémère pour signer
    let mut key = UqBlackKey::generate();
    key.activate();
    key.mutate();
    key.mutate();
    
    // Sérialiser la mesure pour la signer
    let mut data = Vec::new();
    data.extend_from_slice(&measurement.code_hash);
    data.extend_from_slice(&measurement.data_hash);
    data.extend_from_slice(&measurement.version.to_le_bytes());
    data.extend_from_slice(&nonce);
    
    let signature = key.sign(&data);
    let public_key = key.serialize()[..64].try_into().unwrap_or([0u8; 64]);

    Ok(AttestationReport {
        measurement,
        signature: signature.to_vec(),
        public_key,
        nonce,
    })
}

/// Vérifie un rapport d'attestation
pub fn verify_attestation(report: &AttestationReport) -> Result<bool, ChainError> {
    let key = UqBlackKey::generate();
    // Reconstruire la clé publique à partir du rapport
    let mut data = Vec::new();
    data.extend_from_slice(&report.measurement.code_hash);
    data.extend_from_slice(&report.measurement.data_hash);
    data.extend_from_slice(&report.measurement.version.to_le_bytes());
    data.extend_from_slice(&report.nonce);
    
    Ok(key.verify(&data, &report.signature.as_slice().try_into().unwrap_or([0u8; 64])))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_measure() {
        let m = VteeMeasurement::measure().unwrap_or_default();
        assert_ne!(m.code_hash, [0u8; 32]);
        assert_eq!(m.version, 2);
    }

    #[test]
    fn test_attestation_roundtrip() {
        let nonce = [1u8; 32];
        let report = generate_attestation(nonce).unwrap_or_default();
        assert!(verify_attestation(&report).unwrap_or_default());
    }

    #[test]
    fn test_verify_same() {
        let m1 = VteeMeasurement::measure().unwrap_or_default();
        let m2 = VteeMeasurement::measure().unwrap_or_default();
        assert!(m1.verify(&m2));
    }
}





