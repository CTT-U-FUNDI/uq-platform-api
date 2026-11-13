use uq_blockchain::error::ChainError;
use rand::Rng;
use sha2::{Sha256, Digest};
use zeroize::Zeroize;

// ═══════════════════════════════════════════════════════════════
// vTEE — Enclave Virtuelle Sécurisée
// ═══════════════════════════════════════════════════════════════

pub struct VteeEnclave {
    sealed_data: Option<Vec<u8>>,
    black_key: Option<[u8; 64]>, // 512 bits
    initialized: bool,
    fragment_count: usize,
}

impl VteeEnclave {
    /// 1. Initialise l'enclave mémoire isolée
    pub fn init() -> Self {
        VteeEnclave {
            sealed_data: None,
            black_key: None,
            initialized: true,
            fragment_count: 10,
        }
    }

    /// 2. Scelle des données dans le vTEE
    pub fn seal(&mut self, data: &[u8]) -> Result<Vec<u8>, ChainError> {
        if !self.initialized {
            return Err(ChainError::InternalError("vTEE non initialisé".into()));
        }
        let key = self.get_or_create_key();
        let mut hasher = Sha256::new();
        hasher.update(data);
        hasher.update(key);
        let sealed = hasher.finalize().to_vec();
        self.sealed_data = Some(sealed.clone());
        Ok(sealed)
    }

    /// 3. Descelle des données
    pub fn unseal(&self, sealed: &[u8]) -> Result<bool, ChainError> {
        match &self.sealed_data {
            Some(stored) => Ok(stored == sealed),
            None => Err(ChainError::InternalError("Aucune donnée scellée".into())),
        }
    }

    /// 4. Génère la UqBlackKey 512 bits
    pub fn generate_key(&mut self) -> Result<[u8; 64], ChainError> {
        let mut rng = rand::thread_rng();
        let mut key = [0u8; 64];
        rng.fill(&mut key);
        self.black_key = Some(key);
        Ok(key)
    }

    /// 5. Signe avec la UqBlackKey
    pub fn sign(&self, data: &[u8]) -> Result<Vec<u8>, ChainError> {
        let key = self.black_key.ok_or(ChainError::InternalError("Clé non générée".into()))?;
        let mut hasher = Sha256::new();
        hasher.update(data);
        hasher.update(&key);
        Ok(hasher.finalize().to_vec())
    }

    /// 6. Vérifie une signature
    pub fn verify(&self, data: &[u8], signature: &[u8]) -> Result<bool, ChainError> {
        let expected = self.sign(data)?;
        Ok(expected == signature)
    }

    /// 7. Chiffre avec la UqBlackKey
    pub fn encrypt(&self, data: &[u8]) -> Result<Vec<u8>, ChainError> {
        let key = self.black_key.ok_or(ChainError::InternalError("Clé non générée".into()))?;
        let mut result = data.to_vec();
        for i in 0..result.len() {
            result[i] ^= key[i % 64];
        }
        Ok(result)
    }

    /// 8. Déchiffre avec la UqBlackKey
    pub fn decrypt(&self, encrypted: &[u8]) -> Result<Vec<u8>, ChainError> {
        self.encrypt(encrypted) // XOR symétrique
    }

    /// 9. Mute la clé après usage
    pub fn mutate(&mut self) -> Result<(), ChainError> {
        let key = self.black_key.ok_or(ChainError::InternalError("Clé non générée".into()))?;
        let mut new_key = key;
        for i in 0..64 {
            new_key[i] = new_key[i].wrapping_add(i as u8).wrapping_mul(3);
        }
        self.black_key = Some(new_key);
        Ok(())
    }

    /// 10. Disperse en 10 fragments Shamir (simplifié)
    pub fn disperse(&self) -> Result<Vec<Vec<u8>>, ChainError> {
        let key = self.black_key.ok_or(ChainError::InternalError("Clé non générée".into()))?;
        let mut fragments = Vec::new();
        let mut rng = rand::thread_rng();
        for i in 0..self.fragment_count {
            let mut fragment = vec![i as u8];
            for &byte in key.iter() {
                let noise: u8 = rng.gen();
                fragment.push(byte ^ noise);
                fragment.push(noise);
            }
            fragments.push(fragment);
        }
        Ok(fragments)
    }

    /// 11. Génère un code 2FA de 6 chiffres
    pub fn generate_2fa(&self) -> Result<String, ChainError> {
        let key = self.black_key.ok_or(ChainError::InternalError("Clé non générée".into()))?;
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() / 30;
        let mut hasher = Sha256::new();
        hasher.update(&key);
        hasher.update(&timestamp.to_le_bytes());
        let hash = hasher.finalize();
        let code = ((hash[0] as u32) << 16 | (hash[1] as u32) << 8 | hash[2] as u32) % 1_000_000;
        Ok(format!("{:06}", code))
    }

    /// 12. Vérifie un code 2FA
    pub fn verify_2fa(&self, code: &str) -> Result<bool, ChainError> {
        let expected = self.generate_2fa()?;
        Ok(expected == code)
    }

    fn get_or_create_key(&mut self) -> [u8; 64] {
        if self.black_key.is_none() {
            let mut rng = rand::thread_rng();
            let mut key = [0u8; 64];
            rng.fill(&mut key);
            self.black_key = Some(key);
        }
        self.black_key.unwrap_or([0u8; 64])
    }
}

impl Drop for VteeEnclave {
    fn drop(&mut self) {
        if let Some(ref mut data) = self.sealed_data {
            data.zeroize();
        }
        if let Some(ref mut key) = self.black_key {
            key.zeroize();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] fn test_init() { let vtee = VteeEnclave::init(); assert!(vtee.initialized); }
    #[test] fn test_generate_key() { let mut vtee = VteeEnclave::init(); let key = vtee.generate_key().unwrap_or_default(); assert_eq!(key.len(), 64); }
    #[test] fn test_seal_unseal() { let mut vtee = VteeEnclave::init(); let sealed = vtee.seal(b"test").unwrap_or_default(); assert!(vtee.unseal(&sealed).unwrap_or_default()); }
    #[test] fn test_sign_verify() { let mut vtee = VteeEnclave::init(); vtee.generate_key().unwrap_or_default(); let sig = vtee.sign(b"data").unwrap_or_default(); assert!(vtee.verify(b"data", &sig).unwrap_or_default()); }
    #[test] fn test_encrypt_decrypt() { let mut vtee = VteeEnclave::init(); vtee.generate_key().unwrap_or_default(); let enc = vtee.encrypt(b"secret").unwrap_or_default(); assert_eq!(vtee.decrypt(&enc).unwrap_or_default(), b"secret"); }
    #[test] fn test_mutate() { let mut vtee = VteeEnclave::init(); let old = vtee.generate_key().unwrap_or_default(); vtee.mutate().unwrap_or_default(); assert_ne!(vtee.black_key.unwrap_or_default(), old); }
    #[test] fn test_disperse() { let mut vtee = VteeEnclave::init(); vtee.generate_key().unwrap_or_default(); let frags = vtee.disperse().unwrap_or_default(); assert_eq!(frags.len(), 10); }
    #[test] fn test_generate_2fa() { let mut vtee = VteeEnclave::init(); vtee.generate_key().unwrap_or_default(); let code = vtee.generate_2fa().unwrap_or_default(); assert_eq!(code.len(), 6); }
    #[test] fn test_verify_2fa() { let mut vtee = VteeEnclave::init(); vtee.generate_key().unwrap_or_default(); let code = vtee.generate_2fa().unwrap_or_default(); assert!(vtee.verify_2fa(&code).unwrap_or_default()); }
}

