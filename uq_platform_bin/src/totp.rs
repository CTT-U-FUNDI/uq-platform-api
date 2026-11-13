//! TOTP RFC 6238 réel — HMAC-SHA1, fenêtre 30s, compatible
//! Google Authenticator / Aegis / 2FAS.
//! Chaque wallet a un sous-secret dérivé de la clé publique
//! (vk) → codes différents par wallet, régénérés à la rotation.

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha1::Sha1;
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

type HmacSha1 = Hmac<Sha1>;
type HmacSha256 = Hmac<Sha256>;

pub const STEP: u64 = 30;
pub const DIGITS: u32 = 1_000_000;

/// Alphabet base32 RFC 4648 (sans padding)
const B32: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

pub fn base32_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len() * 8 / 5 + 1);
    let mut bits: u32 = 0;
    let mut nbits = 0u32;
    for &b in data {
        bits = (bits << 8) | b as u32;
        nbits += 8;
        while nbits >= 5 {
            nbits -= 5;
            out.push(B32[((bits >> nbits) & 31) as usize] as char);
        }
    }
    if nbits > 0 {
        out.push(B32[((bits << (5 - nbits)) & 31) as usize] as char);
    }
    out
}

pub fn base32_decode(s: &str) -> Option<Vec<u8>> {
    let clean: String = s.chars().filter(|c| !c.is_whitespace() && *c != '=').collect();
    let mut bits: u32 = 0;
    let mut nbits = 0u32;
    let mut out = Vec::with_capacity(clean.len() * 5 / 8);
    for c in clean.chars() {
        let v = B32.iter().position(|&b| b as char == c.to_ascii_uppercase())? as u32;
        bits = (bits << 5) | v;
        nbits += 5;
        if nbits >= 8 {
            nbits -= 8;
            out.push((bits >> nbits) as u8);
        }
    }
    Some(out)
}

/// Secret TOTP du COMPTE (20 octets aléatoires, base32).
pub fn nouveau_secret_compte() -> String {
    let mut b = [0u8; 20];
    rand::thread_rng().fill_bytes(&mut b);
    base32_encode(&b)
}

/// Sous-secret d'un wallet : HMAC-SHA256(secret_compte, "uq-wallet|"+vk).
/// → code différent par clé publique ; change si le secret compte tourne.
pub fn secret_wallet(secret_compte_b32: &str, vk_hex: &str) -> String {
    let raw = base32_decode(secret_compte_b32).unwrap_or_default();
    let mut mac = HmacSha256::new_from_slice(&raw).expect("hmac");
    mac.update(b"uq-wallet|");
    mac.update(vk_hex.as_bytes());
    base32_encode(&mac.finalize().into_bytes()[..20])
}

/// Code TOTP 6 chiffres pour un timestamp (secondes Unix).
pub fn code_pour(secret_b32: &str, ts: u64) -> String {
    let raw = base32_decode(secret_b32).unwrap_or_default();
    let counter = ts / STEP;
    let mut mac = HmacSha1::new_from_slice(&raw).expect("hmac");
    mac.update(&counter.to_be_bytes());
    let h = mac.finalize().into_bytes();
    let off = (h[19] & 0x0f) as usize;
    let code = (((h[off] as u32 & 0x7f) << 24)
        | ((h[off + 1] as u32) << 16)
        | ((h[off + 2] as u32) << 8)
        | (h[off + 3] as u32))
        % DIGITS;
    format!("{:06}", code)
}

/// Vérifie un code : fenêtre ±1 pas de 30s (tolérance 90s).
pub fn verifier(secret_b32: &str, code: &str) -> bool {
    if code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    for w in -1i64..=1 {
        let ts = (now as i64 + w * STEP as i64).max(0) as u64;
        if code_pour(secret_b32, ts) == code {
            return true;
        }
    }
    false
}

/// URI otpauth:// pour apps tierces (secret = sous-secret wallet).
pub fn otpauth_uri(secret_b32: &str, compte: &str, vk_hex: &str) -> String {
    let sub = secret_wallet(secret_b32, vk_hex);
    format!(
        "otpauth://totp/UQ:{}?secret={}&issuer=UQ&digits=6&period=30",
        compte, sub
    )
}

/// Secret compte encodé pour stockage JSON (base64 du b32 — simple).
pub fn secret_pour_json(secret_b32: &str) -> String {
    B64.encode(secret_b32.as_bytes())
}

pub fn secret_depuis_json(s: &str) -> String {
    B64.decode(s)
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .unwrap_or_default()
}
