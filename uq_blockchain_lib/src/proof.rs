use serde::{Serialize, Deserialize};
use sha2::{Sha256, Digest};
use crate::error::ChainError;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ProofType { Screenshot, SmsConfirmation, PushNotification, ManualConfirm }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofMetadata {
    pub gps_latitude: f64, pub gps_longitude: f64,
    pub device_id: [u8; 32], pub app_version: String,
    pub os_type: String, pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentProof {
    pub id: [u8; 32], pub trade_id: [u8; 32],
    pub proof_type: ProofType, pub image_hash: [u8; 32],
    pub metadata: ProofMetadata,
    pub ocr_text: Option<String>, pub ocr_amount: Option<u64>,
    pub ocr_recipient: Option<String>,
    pub verified: bool, pub verified_by: Option<[u8; 32]>,
    pub trust_score: f64, pub created_at: i64,
}

impl PaymentProof {
    pub fn new(trade_id: [u8; 32], proof_type: ProofType, image_data: &[u8], metadata: ProofMetadata) -> Result<Self, ChainError> {
        if image_data.is_empty() && proof_type != ProofType::ManualConfirm {
            return Err(ChainError::InvalidProof("Image requise".into()));
        }
        let image_hash = if !image_data.is_empty() {
            let mut h = Sha256::new(); h.update(image_data);
            let r = h.finalize(); let mut hash = [0u8; 32]; hash.copy_from_slice(&r); hash
        } else { [0u8; 32] };
        let mut proof = PaymentProof {
            id: [0u8; 32], trade_id, proof_type, image_hash, metadata,
            ocr_text: None, ocr_amount: None, ocr_recipient: None,
            verified: false, verified_by: None, trust_score: 0.0,
            created_at: chrono::Utc::now().timestamp(),
        };
        proof.id = proof.compute_id();
        proof.trust_score = proof.compute_trust_score();
        Ok(proof)
    }
    
    fn compute_id(&self) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(&self.trade_id); h.update(&self.image_hash); h.update(&self.created_at.to_le_bytes());
        let r = h.finalize(); let mut id = [0u8; 32]; id.copy_from_slice(&r); id
    }
    
    pub fn compute_trust_score(&self) -> f64 {
        let mut score: f64 = 0.0;
        if self.image_hash != [0u8; 32] { score += 0.3; }
        if self.metadata.gps_latitude != 0.0 && self.metadata.gps_longitude != 0.0 { score += 0.2; }
        if self.ocr_amount.is_some() && self.ocr_recipient.is_some() { score += 0.3; }
        let now = chrono::Utc::now().timestamp();
        if self.metadata.timestamp <= now && (now - self.metadata.timestamp) < 3600 { score += 0.2; }
        if score > 1.0 { 1.0 } else if score < 0.0 { 0.0 } else { score }
    }
    
    pub fn is_valid(&self) -> bool { self.trust_score >= 0.5 }
    
    pub fn verify(&mut self, validator: [u8; 32]) -> Result<(), ChainError> {
        if self.verified { return Err(ChainError::InvalidProof("Déjà vérifiée".into())); }
        self.verified = true; self.verified_by = Some(validator); Ok(())
    }
    
    pub fn set_ocr(&mut self, amount: u64, recipient: String, raw_text: String) {
        self.ocr_amount = Some(amount); self.ocr_recipient = Some(recipient);
        self.ocr_text = Some(raw_text); self.trust_score = self.compute_trust_score();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn meta() -> ProofMetadata {
        ProofMetadata { gps_latitude: -4.32, gps_longitude: 15.31, device_id: [1u8;32],
            app_version: "1.0".into(), os_type: "android".into(), timestamp: chrono::Utc::now().timestamp() }
    }
    
    #[test]
    fn test_create() {
        let p = PaymentProof::new([2u8;32], ProofType::Screenshot, b"img", meta()).unwrap_or_default();
        assert!(p.trust_score > 0.0);
    }
    
    #[test]
    fn test_ocr() {
        let mut p = PaymentProof::new([3u8;32], ProofType::SmsConfirmation, b"img", meta()).unwrap_or_default();
        p.set_ocr(50000, "Bob".into(), "OK".into());
        assert!(p.is_valid());
    }
}
