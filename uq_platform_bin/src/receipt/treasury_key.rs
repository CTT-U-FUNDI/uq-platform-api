// ====================
// TREASURY KEY — Clé Ed25519 du Trésor (pour signer les reçus)
// ====================
//
// La clé est stockée dans uq_index.db → table "treasury"
//
// ====================

use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use uq_core::crypto;

/// Table treasury (dans uq_index.db)
pub fn init_treasury_table(conn: &Connection) -> Result<(), String> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS treasury (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            public_key TEXT NOT NULL,
            encrypted_private_key TEXT NOT NULL,
            salt TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER
        )",
        [],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Charge la clé Trésor (la crée si absente)
pub fn load_or_create_treasury_key() -> Result<([u8; 32], [u8; 32]), String> {
    let conn = Connection::open("uq_index.db").map_err(|e| format!("DB uq_index: {}", e))?;

    init_treasury_table(&conn)?;

    // Essayer de charger
    let result: Result<(String, String), _> = conn.query_row(
        "SELECT public_key, encrypted_private_key FROM treasury WHERE id = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    );

    if let Ok((pub_hex, _priv_enc)) = result {
        // Décoder la clé publique
        let pub_bytes = hex::decode(&pub_hex).map_err(|e| format!("Hex: {}", e))?;

        if pub_bytes.len() == 32 {
            let mut pub_arr = [0u8; 32];
            pub_arr.copy_from_slice(&pub_bytes);

            // ⚠️ Pour l'instant on génère une nouvelle clé privée à chaque fois
            // (à améliorer plus tard avec déchiffrement AES)
            let kp = crypto::generate_keypair();
            return Ok((kp.public_key, kp.private_key));
        }
    }

    // Créer une nouvelle clé
    let kp = crypto::generate_keypair();
    let pub_hex = hex::encode(&kp.public_key);

    // Chiffrer la privée avec un sel fixe (à améliorer)
    let salt = "uq_treasury_salt_v1";
    let priv_enc = hex::encode(&kp.private_key); // ⚠️ À améliorer : chiffrer vraiment

    let now = chrono::Utc::now().timestamp();

    conn.execute(
        "INSERT OR REPLACE INTO treasury (id, public_key, encrypted_private_key, salt, created_at, updated_at)
         VALUES (1, ?1, ?2, ?3, ?4, ?4)",
        params![pub_hex, priv_enc, salt, now],
    ).map_err(|e| e.to_string())?;

    eprintln!("🔑 Clé Trésor créée : {}", pub_hex);

    Ok((kp.public_key, kp.private_key))
}

/// Signe des données avec la clé Trésor
pub fn sign_with_treasury(data: &[u8]) -> Result<([u8; 64], [u8; 32]), String> {
    let (public_key, private_key) = load_or_create_treasury_key()?;
    let signature = crypto::sign_ed25519(data, &private_key);
    Ok((signature, public_key))
}

/// Vérifie une signature avec la clé publique
pub fn verify_with_treasury(data: &[u8], signature: &[u8; 64], public_key: &[u8; 32]) -> bool {
    crypto::verify_ed25519(data, signature, public_key)
}

/// Calcule le hash SHA-256 des données du reçu
pub fn hash_receipt_data(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}
