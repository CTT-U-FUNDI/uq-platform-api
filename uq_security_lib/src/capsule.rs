// ═══════════════════════════════════════════════════════════════
// CAPSULE UQ — enveloppe opaque pour données sensibles (ex: numéro UQ)
// ═══════════════════════════════════════════════════════════════
//
// Une capsule est un blob chiffré auto-contenu :
//   "v1.<base64url nonce 12o>.<base64url ciphertext+tag>"
//
// - AES-256-GCM (authentifié) : toute altération est détectée.
// - AAD (données associées) lie la capsule à son contexte
//   (ex: "uq-number", "uq-mail:{uq}") — une capsule ne peut pas
//   être rejouée dans un autre usage.
// - La clé est dérivée de la clé maîtresse plateforme :
//   SHA-256("uq-capsule-v1|" + secret). Le secret peut être
//   surchargé par UQ_CAPSULE_KEY (hex 64) ; sinon JWT_SECRET.
// - Aucune donnée en clair ne transite : ni dans la capsule,
//   ni dans les logs.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::RngCore;
use sha2::{Digest, Sha256};
use thiserror::Error;
use zeroize::Zeroize;

const VERSION: &str = "v1";
const INFO_DERIVATION: &[u8] = b"uq-capsule-v1|";

#[derive(Debug, Error)]
pub enum CapsuleError {
    #[error("capsule malformée")]
    Malformee,
    #[error("capsule corrompue ou mot de passe/contexte incorrect")]
    Decrypt,
    #[error("clé capsule invalide")]
    CleInvalide,
}

/// Clé de capsule — 32 octets, effacée de la mémoire au drop.
pub struct CapsuleKey([u8; 32]);

impl CapsuleKey {
    /// Dérive la clé depuis le secret plateforme.
    /// `secret` = JWT_SECRET ou UQ_CAPSULE_KEY (hex décode ou brut).
    pub fn depuis_secret(secret: &str) -> Self {
        let mut h = Sha256::new();
        h.update(INFO_DERIVATION);
        h.update(secret.as_bytes());
        let out = h.finalize();
        let mut k = [0u8; 32];
        k.copy_from_slice(&out);
        CapsuleKey(k)
    }

    /// Depuis une clé hexadécimale de 64 caractères (32 octets).
    pub fn depuis_hex(hex_key: &str) -> Result<Self, CapsuleError> {
        let bytes = hex::decode(hex_key.trim()).map_err(|_| CapsuleError::CleInvalide)?;
        if bytes.len() != 32 {
            return Err(CapsuleError::CleInvalide);
        }
        let mut k = [0u8; 32];
        k.copy_from_slice(&bytes);
        Ok(CapsuleKey(k))
    }
}

impl Drop for CapsuleKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

fn cle_plateforme() -> Result<CapsuleKey, CapsuleError> {
    if let Ok(hex_key) = std::env::var("UQ_CAPSULE_KEY") {
        return CapsuleKey::depuis_hex(&hex_key);
    }
    let secret = std::env::var("JWT_SECRET")
        .map_err(|_| CapsuleError::CleInvalide)?;
    Ok(CapsuleKey::depuis_secret(&secret))
}

/// Scelle `clair` dans une capsule opaque.
/// `aad` lie la capsule à un contexte (ne jamais utiliser de secret
/// comme AAD — l'AAD n'est pas chiffré, seulement authentifié).
pub fn sceller(plaintext: &[u8], aad: &[u8]) -> Result<String, CapsuleError> {
    let key = cle_plateforme()?;
    sceller_avec(&key, plaintext, aad)
}

/// Ouvre une capsule. Échoue si altérée ou mauvais AAD.
pub fn ouvrir(capsule: &str, aad: &[u8]) -> Result<Vec<u8>, CapsuleError> {
    let key = cle_plateforme()?;
    ouvrir_avec(&key, capsule, aad)
}

/// Versions avec clé explicite (tests, dérivations par compte).
pub fn sceller_avec(key: &CapsuleKey, plaintext: &[u8], aad: &[u8]) -> Result<String, CapsuleError> {
    let cipher = Aes256Gcm::new_from_slice(&key.0).map_err(|_| CapsuleError::CleInvalide)?;
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct = cipher
        .encrypt(nonce, Payload { msg: plaintext, aad })
        .map_err(|_| CapsuleError::Decrypt)?;
    Ok(format!(
        "{}.{}.{}",
        VERSION,
        URL_SAFE_NO_PAD.encode(nonce_bytes),
        URL_SAFE_NO_PAD.encode(ct)
    ))
}

pub fn ouvrir_avec(key: &CapsuleKey, capsule: &str, aad: &[u8]) -> Result<Vec<u8>, CapsuleError> {
    let mut parts = capsule.split('.');
    let (v, n, c) = match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(v), Some(n), Some(c), None) => (v, n, c),
        _ => return Err(CapsuleError::Malformee),
    };
    if v != VERSION {
        return Err(CapsuleError::Malformee);
    }
    let nonce_bytes = URL_SAFE_NO_PAD.decode(n).map_err(|_| CapsuleError::Malformee)?;
    let ct = URL_SAFE_NO_PAD.decode(c).map_err(|_| CapsuleError::Malformee)?;
    if nonce_bytes.len() != 12 {
        return Err(CapsuleError::Malformee);
    }
    let cipher = Aes256Gcm::new_from_slice(&key.0).map_err(|_| CapsuleError::CleInvalide)?;
    cipher
        .decrypt(Nonce::from_slice(&nonce_bytes), Payload { msg: &ct, aad })
        .map_err(|_| CapsuleError::Decrypt)
}

// ---------- Helpers métier : numéro UQ ----------

pub const AAD_NUMERO_UQ: &[u8] = b"uq-number";

/// Scelle un numéro UQ → capsule opaque (base64url).
pub fn sceller_uq(uq: &str) -> Result<String, CapsuleError> {
    sceller(uq.as_bytes(), AAD_NUMERO_UQ)
}

/// Ouvre une capsule de numéro UQ → "UQ-XXXXXXXX".
pub fn ouvrir_uq(capsule: &str) -> Result<String, CapsuleError> {
    let brut = ouvrir(capsule, AAD_NUMERO_UQ)?;
    String::from_utf8(brut).map_err(|_| CapsuleError::Decrypt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aller_retour() {
        let key = CapsuleKey::depuis_secret("secret-test");
        let cap = sceller_avec(&key, b"UQ-12345678", AAD_NUMERO_UQ).unwrap();
        assert!(cap.starts_with("v1."));
        assert!(!cap.contains("UQ-"));
        let out = ouvrir_avec(&key, &cap, AAD_NUMERO_UQ).unwrap();
        assert_eq!(out, b"UQ-12345678");
    }

    #[test]
    fn nonce_aleatoire() {
        let key = CapsuleKey::depuis_secret("s");
        let a = sceller_avec(&key, b"UQ-1", AAD_NUMERO_UQ).unwrap();
        let b = sceller_avec(&key, b"UQ-1", AAD_NUMERO_UQ).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn mauvais_aad_rejete() {
        let key = CapsuleKey::depuis_secret("s");
        let cap = sceller_avec(&key, b"UQ-1", AAD_NUMERO_UQ).unwrap();
        assert!(ouvrir_avec(&key, &cap, b"autre-contexte").is_err());
    }

    #[test]
    fn alteration_rejetee() {
        let key = CapsuleKey::depuis_secret("s");
        let cap = sceller_avec(&key, b"UQ-1", AAD_NUMERO_UQ).unwrap();
        let mut chars: Vec<char> = cap.chars().collect();
        let i = chars.len() - 3;
        chars[i] = if chars[i] == 'A' { 'B' } else { 'A' };
        let cap2: String = chars.into_iter().collect();
        assert!(ouvrir_avec(&key, &cap2, AAD_NUMERO_UQ).is_err());
    }

    #[test]
    fn mauvaise_cle_rejetee() {
        let k1 = CapsuleKey::depuis_secret("s1");
        let k2 = CapsuleKey::depuis_secret("s2");
        let cap = sceller_avec(&k1, b"UQ-1", AAD_NUMERO_UQ).unwrap();
        assert!(ouvrir_avec(&k2, &cap, AAD_NUMERO_UQ).is_err());
    }
}
