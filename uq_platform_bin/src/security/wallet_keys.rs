// ====================
// WALLET KEYS — Génération + Chiffrement + Signature
// ====================
//
// Ce module gère les clés cryptographiques des wallets :
// - Génération de paires Ed25519
// - Chiffrement de la clé privée avec AES-256-GCM
// - Signature de transactions
// - Vérification de signatures
//
// ====================

use sha2::{Digest, Sha256};
use uq_core::crypto;

/// Résultat de la génération de clés
pub struct WalletKeys {
    pub public_key: [u8; 32],
    pub encrypted_private_key: Vec<u8>, // [IV(12) | ciphertext]
    pub iv: [u8; 12],
    pub salt: [u8; 32],
}

/// Dérive une clé de chiffrement depuis un mot de passe (SHA-256 simplifié)
/// En production, utiliser PBKDF2 ou Argon2
fn derive_key_from_password(password: &str, salt: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"UQ_WALLET_KEY_DERIVATION_V1:");
    hasher.update(password.as_bytes());
    hasher.update(b":");
    hasher.update(salt);
    let result = hasher.finalize();

    let mut key = [0u8; 32];
    key.copy_from_slice(&result);
    key
}

/// Génère une paire de clés + chiffre la privée avec le mot de passe
pub fn generate_wallet_keys(password: &str) -> Result<WalletKeys, String> {
    if password.is_empty() {
        return Err("Mot de passe requis pour générer les clés".to_string());
    }

    // 1. Générer la paire de clés Ed25519
    let kp = crypto::generate_keypair();

    // 2. Générer un sel aléatoire
    let salt_vec = crypto::secure_random(32);
    let mut salt = [0u8; 32];
    salt.copy_from_slice(&salt_vec);

    // 3. Dériver la clé de chiffrement depuis le mot de passe
    let encryption_key = derive_key_from_password(password, &salt);

    // 4. Chiffrer la clé privée (le résultat = [IV(12) | ciphertext])
    let encrypted = crypto::encrypt_aes_gcm(&kp.private_key, &encryption_key);

    // 5. Extraire IV
    let mut iv = [0u8; 12];
    iv.copy_from_slice(&encrypted[..12]);

    eprintln!(
        "🔐 WalletKeys généré : pubkey={}...",
        hex::encode(&kp.public_key[..8])
    );

    Ok(WalletKeys {
        public_key: kp.public_key,
        encrypted_private_key: encrypted,
        iv,
        salt,
    })
}

/// Déchiffre la clé privée avec le mot de passe
pub fn decrypt_private_key(
    encrypted_private_key: &[u8],
    salt: &[u8; 32],
    password: &str,
) -> Result<[u8; 32], String> {
    if encrypted_private_key.len() < 44 {
        return Err("Clé chiffrée trop courte".to_string());
    }

    let encryption_key = derive_key_from_password(password, salt);

    let decrypted = crypto::decrypt_aes_gcm(encrypted_private_key, &encryption_key)
        .map_err(|e| format!("Déchiffrement échoué: {}", e))?;

    if decrypted.len() != 32 {
        return Err(format!("Clé privée invalide: {} bytes", decrypted.len()));
    }

    let mut private_key = [0u8; 32];
    private_key.copy_from_slice(&decrypted);
    Ok(private_key)
}

/// Signe des données avec la clé privée
pub fn sign_data(private_key: &[u8; 32], data: &[u8]) -> [u8; 64] {
    crypto::sign_ed25519(data, private_key)
}

/// Vérifie une signature
pub fn verify_signature(public_key: &[u8; 32], data: &[u8], signature: &[u8; 64]) -> bool {
    crypto::verify_ed25519(data, signature, public_key)
}

/// Construit le message à signer pour une transaction
/// Format : uq_number:sender_public:recipient:amount_points:fee_points:timestamp:tx_hash
pub fn build_transaction_message(
    uq_number: &str,
    recipient: &str,
    amount_points: u64,
    fee_points: u64,
    timestamp: i64,
    tx_hash: &str,
) -> Vec<u8> {
    let message = format!(
        "UQ_TX_V1:{}:{}:{}:{}:{}:{}",
        uq_number, recipient, amount_points, fee_points, timestamp, tx_hash
    );
    message.into_bytes()
}

/// Hash SHA-256 (hex)
pub fn hash_hex(data: &[u8]) -> String {
    let hash = crypto::sha256(data);
    hex::encode(hash)
}
