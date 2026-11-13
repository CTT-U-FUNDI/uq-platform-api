use ed25519_dalek::{SigningKey, VerifyingKey, Signature, Signer, Verifier};
use aes_gcm::{Aes256Gcm, KeyInit, aead::Aead, Nonce};
use sha2::{Sha256, Digest};
use rand::RngCore;

/// Paire de clés Ed25519
pub struct KeyPair {
    pub private_key: [u8; 32],  // ed25519-dalek 2.x utilise 32 octets
    pub public_key: [u8; 32],
}

/// Génère une paire de clés Ed25519
pub fn generate_keypair() -> KeyPair {
    // ✅ VRAI aléatoire cryptographique
    let mut rng = rand::thread_rng();
    let mut private_bytes = [0u8; 32];
    rng.fill_bytes(&mut private_bytes);
    
    let signing_key = SigningKey::from_bytes(&private_bytes);
    let verifying_key = signing_key.verifying_key();
    
    KeyPair {
        private_key: signing_key.to_bytes(),
        public_key: verifying_key.to_bytes(),
    }
}

/// Signe un message avec Ed25519
pub fn sign_ed25519(message: &[u8], private_key: &[u8; 32]) -> [u8; 64] {
    let signing_key = SigningKey::from_bytes(private_key);
    let signature = signing_key.sign(message);
    signature.to_bytes()
}

/// Vérifie une signature Ed25519
pub fn verify_ed25519(message: &[u8], signature: &[u8; 64], public_key: &[u8; 32]) -> bool {
    if let Ok(verifying_key) = VerifyingKey::from_bytes(public_key) {
        if let Ok(sig) = Signature::from_slice(signature) {
            return verifying_key.verify(message, &sig).is_ok();
        }
    }
    false
}

/// Chiffre avec AES-256-GCM
pub fn encrypt_aes_gcm(plaintext: &[u8], key: &[u8; 32]) -> Vec<u8> {
    let cipher = Aes256Gcm::new_from_slice(key).expect("Clé invalide");
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher.encrypt(nonce, plaintext).expect("Chiffrement échoué");
    let mut result = Vec::with_capacity(12 + ciphertext.len());
    result.extend_from_slice(&nonce_bytes);
    result.extend_from_slice(&ciphertext);
    result
}

/// Déchiffre AES-256-GCM
pub fn decrypt_aes_gcm(ciphertext: &[u8], key: &[u8; 32]) -> Result<Vec<u8>, &'static str> {
    if ciphertext.len() < 28 {
        return Err("Ciphertext trop court");
    }
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| "Clé invalide")?;
    let (nonce_bytes, encrypted) = ciphertext.split_at(12);
    let nonce = Nonce::from_slice(nonce_bytes);
    cipher.decrypt(nonce, encrypted).map_err(|_| "Déchiffrement échoué")
}

/// Calcule SHA-256
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&result);
    hash
}

/// Nombres aléatoires sécurisés
pub fn secure_random(length: usize) -> Vec<u8> {
    let mut bytes = vec![0u8; length];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_keypair() {
        let kp = generate_keypair();
        assert_eq!(kp.private_key.len(), 32);
        assert_eq!(kp.public_key.len(), 32);
    }

    #[test]
    fn test_sign_and_verify() {
        let kp = generate_keypair();
        let message = b"UQ Protocol";
        let signature = sign_ed25519(message, &kp.private_key);
        assert!(verify_ed25519(message, &signature, &kp.public_key));
    }

    #[test]
    fn test_encrypt_decrypt() {
        let key = sha256(b"test key");
        let plaintext = b"Message secret";
        let ciphertext = encrypt_aes_gcm(plaintext, &key);
        let decrypted = decrypt_aes_gcm(&ciphertext, &key).unwrap_or_default();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_sha256() {
        let hash = sha256(b"UQ Protocol");
        assert_eq!(hash.len(), 32);
    }
}

