// ====================
// VERIFIER — Vérification des reçus PNG
// ====================

use super::{embed, treasury_key};
use super::{ReceiptData, VerifiedReceipt};

/// Vérifie un reçu PNG (bytes)
pub fn verify_receipt_png(png_bytes: &[u8]) -> VerifiedReceipt {
    let now = chrono::Utc::now().timestamp();

    // 1. Extraire les métadonnées
    let json_data = match embed::extract_text_chunk(png_bytes, "uq_data") {
        Some(d) => d,
        None => {
            return VerifiedReceipt {
                valid: false,
                data: None,
                error: Some("uq_data non trouvé".to_string()),
                verified_at: now,
            }
        }
    };

    let stored_hash = match embed::extract_text_chunk(png_bytes, "uq_hash") {
        Some(h) => h,
        None => {
            return VerifiedReceipt {
                valid: false,
                data: None,
                error: Some("uq_hash non trouvé".to_string()),
                verified_at: now,
            }
        }
    };

    let signature_hex = match embed::extract_text_chunk(png_bytes, "uq_signature") {
        Some(s) => s,
        None => {
            return VerifiedReceipt {
                valid: false,
                data: None,
                error: Some("uq_signature non trouvé".to_string()),
                verified_at: now,
            }
        }
    };

    let pubkey_hex = match embed::extract_text_chunk(png_bytes, "uq_pubkey") {
        Some(p) => p,
        None => {
            return VerifiedReceipt {
                valid: false,
                data: None,
                error: Some("uq_pubkey non trouvé".to_string()),
                verified_at: now,
            }
        }
    };

    // 2. Vérifier le hash
    let computed_hash = treasury_key::hash_receipt_data(json_data.as_bytes());
    if computed_hash != stored_hash {
        return VerifiedReceipt {
            valid: false,
            data: None,
            error: Some("Hash ne correspond pas (falsification détectée)".to_string()),
            verified_at: now,
        };
    }

    // 3. Décoder la signature et la clé publique
    let signature_bytes = match hex::decode(&signature_hex) {
        Ok(b) => b,
        Err(_) => {
            return VerifiedReceipt {
                valid: false,
                data: None,
                error: Some("Signature invalide (hex)".to_string()),
                verified_at: now,
            }
        }
    };

    let pubkey_bytes = match hex::decode(&pubkey_hex) {
        Ok(b) => b,
        Err(_) => {
            return VerifiedReceipt {
                valid: false,
                data: None,
                error: Some("Clé publique invalide (hex)".to_string()),
                verified_at: now,
            }
        }
    };

    if signature_bytes.len() != 64 || pubkey_bytes.len() != 32 {
        return VerifiedReceipt {
            valid: false,
            data: None,
            error: Some("Taille signature/clé invalide".to_string()),
            verified_at: now,
        };
    }

    let mut sig_arr = [0u8; 64];
    sig_arr.copy_from_slice(&signature_bytes);

    let mut pub_arr = [0u8; 32];
    pub_arr.copy_from_slice(&pubkey_bytes);

    // 4. Vérifier la signature
    let valid = treasury_key::verify_with_treasury(stored_hash.as_bytes(), &sig_arr, &pub_arr);

    if !valid {
        return VerifiedReceipt {
            valid: false,
            data: None,
            error: Some("Signature Ed25519 invalide".to_string()),
            verified_at: now,
        };
    }

    // 5. Désérialiser les données
    let data: ReceiptData = match serde_json::from_str(&json_data) {
        Ok(d) => d,
        Err(e) => {
            return VerifiedReceipt {
                valid: false,
                data: None,
                error: Some(format!("JSON invalide: {}", e)),
                verified_at: now,
            }
        }
    };

    VerifiedReceipt {
        valid: true,
        data: Some(data),
        error: None,
        verified_at: now,
    }
}
