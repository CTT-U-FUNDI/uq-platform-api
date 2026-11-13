use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use ed25519_dalek::{Signature, Signer, Verifier, VerifyingKey, SigningKey};
use hkdf::Hkdf;
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};
use zeroize::Zeroize;

const MAGIC: &[u8; 5] = b"UQBK1";
const INFO: &[u8] = b"uq-black-key/v1";
const VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyState {
    Emerging,
    Active,
    Locked,
    Destroyed,
}

#[derive(Debug, Clone)]
pub struct UqBlackKey {
    pub public_key: [u8; 32],
    pub address: [u8; 32],
    pub complexity: usize,
    pub state: KeyState,
    pub created_at: u64,
    pub integrity_hash: [u8; 64],
    // BORB_balance SUPPRIME — le solde est dans BubbleRegistry
    pub version: u32,
    master_secret: [u8; 64],
    signing_key: [u8; 32],
    encryption_key: [u8; 32],
    mac_key: [u8; 32],
}

#[derive(Debug, Clone)]
struct DerivedKeys {
    signing_key: [u8; 32],
    encryption_key: [u8; 32],
    mac_key: [u8; 32],
}

impl UqBlackKey {
    pub fn generate() -> Self {
        let mut master_secret = [0u8; 64];
        OsRng.fill_bytes(&mut master_secret);
        Self::from_master_secret(master_secret)
    }

    fn from_master_secret(master_secret: [u8; 64]) -> Self {
        let derived = Self::derive_subkeys(&master_secret);
        let signing_key = SigningKey::from_bytes(&derived.signing_key);
        let public_key = signing_key.verifying_key().to_bytes();
        let address = Self::sha256(&public_key);
        let created_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let mut key = Self {
            public_key,
            address,
            complexity: 512,
            state: KeyState::Active,
            created_at,
            integrity_hash: [0u8; 64],
            version: VERSION,
            master_secret,
            signing_key: derived.signing_key,
            encryption_key: derived.encryption_key,
            mac_key: derived.mac_key,
        };
        key.compute_integrity();
        key
    }

    fn derive_subkeys(master_secret: &[u8; 64]) -> DerivedKeys {
        let mut okm = [0u8; 96];
        let hkdf = Hkdf::<Sha256>::new(Some(b"uq-black-key/v1"), master_secret);
        hkdf.expand(INFO, &mut okm)
            .expect("HKDF expansion should never fail with fixed-length output");

        let signing_key = okm[0..32].try_into().expect("signing key derivation");
        let encryption_key = okm[32..64].try_into().expect("encryption key derivation");
        let mac_key = okm[64..96].try_into().expect("MAC key derivation");

        DerivedKeys {
            signing_key,
            encryption_key,
            mac_key,
        }
    }

    fn sha256(data: &[u8]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(data);
        let result = hasher.finalize();
        let mut out = [0u8; 32];
        out.copy_from_slice(&result[..]);
        out
    }

    fn hmac_sha512(key: &[u8; 32], payload: &[u8]) -> [u8; 64] {
        use hmac::{Hmac, Mac};
        use sha2::Sha512;

        type HmacSha512 = Hmac<Sha512>;
        let mut mac = <HmacSha512 as Mac>::new_from_slice(key)
            .expect("HMAC-SHA512 accepts any 32-byte key");
        mac.update(payload);
        let out = mac.finalize().into_bytes();
        let mut result = [0u8; 64];
        result.copy_from_slice(&out[..]);
        result
    }

    fn payload_for_integrity(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&self.version.to_be_bytes());
        out.extend_from_slice(&self.created_at.to_be_bytes());
        out.push(match self.state {
            KeyState::Emerging => 0,
            KeyState::Active => 1,
            KeyState::Locked => 2,
            KeyState::Destroyed => 3,
        });
        out.extend_from_slice(&self.complexity.to_be_bytes());
        out.extend_from_slice(&self.public_key);
        out.extend_from_slice(&self.address);
        // BORB_balance retire du hash d'integrite
        out
    }

    pub fn compute_integrity(&mut self) {
        self.integrity_hash = Self::hmac_sha512(&self.mac_key, &self.payload_for_integrity());
    }

    pub fn verify_integrity(&self) -> bool {
        Self::hmac_sha512(&self.mac_key, &self.payload_for_integrity()) == self.integrity_hash
    }

    pub fn can_operate(&self) -> bool {
        self.state == KeyState::Active
    }

    pub fn activate(&mut self) {
        self.state = KeyState::Active;
        self.compute_integrity();
    }

    pub fn sign(&self, data: &[u8]) -> [u8; 64] {
        let signer = SigningKey::from_bytes(&self.signing_key);
        signer.sign(data).to_bytes()
    }

    pub fn verify(&self, data: &[u8], signature: &[u8; 64]) -> bool {
        let signature = match Signature::from_slice(signature) {
            Ok(sig) => sig,
            Err(_) => return false,
        };
        let verifying_key = match VerifyingKey::from_bytes(&self.public_key) {
            Ok(vk) => vk,
            Err(_) => return false,
        };
        verifying_key.verify(data, &signature).is_ok()
    }

    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, String> {
        let cipher = Aes256Gcm::new_from_slice(&self.encryption_key)
            .map_err(|_| "invalid AES key length".to_string())?;
        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(nonce, plaintext)
            .map_err(|_| "AES-GCM encryption failed".to_string())?;
        let mut output = Vec::with_capacity(12 + ciphertext.len());
        output.extend_from_slice(&nonce_bytes);
        output.extend_from_slice(&ciphertext);
        Ok(output)
    }

    pub fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, String> {
        if ciphertext.len() < 12 + 16 {
            return Err("ciphertext too short".to_string());
        }
        let cipher = Aes256Gcm::new_from_slice(&self.encryption_key)
            .map_err(|_| "invalid AES key length".to_string())?;
        let (nonce_bytes, encrypted) = ciphertext.split_at(12);
        let nonce = Nonce::from_slice(nonce_bytes);
        cipher
            .decrypt(nonce, encrypted)
            .map_err(|_| "AES-GCM decryption failed".to_string())
    }

    // balance() SUPPRIMEE — utilisez BubbleRegistry::get_balance() a la place
    // La cle ne stocke plus de solde

    // update_balance() SUPPRIMEE — utilisez BubbleRegistry a la place

    pub fn mutate(&mut self) {
        // Rotation du master_secret via SHA-256
        let hash = Self::sha256(&self.master_secret);
        for i in 0..64 {
            self.master_secret[i] ^= hash[i % 32];
        }
        
        // Re-dÃ©river les sous-clÃ©s
        let derived = Self::derive_subkeys(&self.master_secret);
        self.signing_key = derived.signing_key;
        self.encryption_key = derived.encryption_key;
        self.mac_key = derived.mac_key;
        
        // Recalculer la clÃ© publique
        let signer = SigningKey::from_bytes(&self.signing_key);
        self.public_key = signer.verifying_key().to_bytes();
        
        // Recalculer l'intÃ©gritÃ©
        self.compute_integrity();
    }

    pub fn rotate(&self) -> Result<Self, String> {
        let mut master_secret = [0u8; 64];
        OsRng.fill_bytes(&mut master_secret);
        let mut rotated = Self::from_master_secret(master_secret);
        rotated.state = self.state;
        rotated.created_at = self.created_at;
        rotated.compute_integrity();
        Ok(rotated)
    }

    pub fn serialize(&self) -> Vec<u8> {
        self.payload_for_integrity()
    }

    pub fn deserialize(data: &[u8]) -> Result<Self, String> {
        if data.len() < 162 {
            return Err("serialized context is too short".to_string());
        }
        if &data[0..5] != MAGIC {
            return Err("invalid UQBK magic".to_string());
        }
        let mut master_secret = [0u8; 64];
        OsRng.fill_bytes(&mut master_secret);
        Ok(Self::from_master_secret(master_secret))
    }

    pub fn serialize_secure(&self, passphrase: &[u8]) -> Result<Vec<u8>, String> {
        let mut salt = [0u8; 16];
        OsRng.fill_bytes(&mut salt);
        let mut key_material = [0u8; 32];
        let hkdf = Hkdf::<Sha256>::new(Some(&salt), passphrase);
        hkdf.expand(b"uq-black-key/store/v1", &mut key_material)
            .map_err(|_| "HKDF key derivation failed".to_string())?;

        let mut payload = self.payload_for_integrity();
        payload.extend_from_slice(&self.master_secret);

        let cipher = Aes256Gcm::new_from_slice(&key_material)
            .map_err(|_| "invalid secure storage key".to_string())?;
        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let encrypted = cipher
            .encrypt(nonce, payload.as_ref())
            .map_err(|_| "secure storage encryption failed".to_string())?;

        let mut output = Vec::new();
        output.extend_from_slice(b"UQBK2");
        output.extend_from_slice(&salt);
        output.extend_from_slice(&nonce_bytes);
        output.extend_from_slice(&encrypted);
        Ok(output)
    }

    pub fn deserialize_secure(data: &[u8], passphrase: &[u8]) -> Result<Self, String> {
        if data.len() < 5 + 16 + 12 + 16 {
            return Err("stored key is too short".to_string());
        }
        if &data[0..5] != b"UQBK2" {
            return Err("invalid secure key format".to_string());
        }

        let salt: [u8; 16] = data[5..21].try_into().unwrap();
        let nonce_bytes: [u8; 12] = data[21..33].try_into().unwrap();
        let encrypted = &data[33..];

        let mut key_material = [0u8; 32];
        let hkdf = Hkdf::<Sha256>::new(Some(&salt), passphrase);
        hkdf.expand(b"uq-black-key/store/v1", &mut key_material)
            .map_err(|_| "HKDF derivation for secure key failed".to_string())?;

        let cipher = Aes256Gcm::new_from_slice(&key_material)
            .map_err(|_| "invalid secure storage key".to_string())?;
        let nonce = Nonce::from_slice(&nonce_bytes);
        let plaintext = cipher
            .decrypt(nonce, encrypted)
            .map_err(|_| "invalid passphrase or corrupted key".to_string())?;

        let mut master_secret = [0u8; 64];
        if plaintext.len() >= 226 {
            master_secret.copy_from_slice(&plaintext[162..226]);
        }
        Ok(Self::from_master_secret(master_secret))
    }
}

impl Drop for UqBlackKey {
    fn drop(&mut self) {
        self.master_secret.zeroize();
        self.signing_key.zeroize();
        self.encryption_key.zeroize();
        self.mac_key.zeroize();
    }
}
