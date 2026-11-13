// ====================
// MASTER KEY — Gestion de la clé maître de l'owner
// ====================
//
// La clé maître dérive du mot de passe owner (Argon2 ou PBKDF2).
// Elle permet de déverrouiller uq_index.db et toutes les DB users.
//
// ====================

use sha2::{Sha256, Digest};
use std::sync::{Arc, Mutex, OnceLock};

/// Clé maître globale (chargée au démarrage du backend)
static MASTER_KEY: OnceLock<Arc<Mutex<Option<[u8; 32]>>>> = OnceLock::new();

/// Nombre d'itérations pour PBKDF2
const PBKDF2_ITERATIONS: u32 = 100_000;

/// Salt global (stocké dans .env ou fixe)
const MASTER_SALT: &[u8] = b"UQ_MASTER_SALT_V1_2026";

/// Dérive la clé maître depuis le mot de passe owner
/// Utilise PBKDF2-HMAC-SHA256 simplifié (extensible à Argon2)
pub fn derive_master_key(owner_password: &str) -> [u8; 32] {
    let mut key = owner_password.as_bytes().to_vec();
    key.extend_from_slice(MASTER_SALT);
    
    // PBKDF2 simplifié : 100k itérations SHA-256
    let mut hasher = Sha256::new();
    hasher.update(&key);
    let mut result = hasher.finalize();
    
    for _ in 0..PBKDF2_ITERATIONS {
        let mut h = Sha256::new();
        h.update(&result);
        h.update(MASTER_SALT);
        result = h.finalize();
    }
    
    let mut master = [0u8; 32];
    master.copy_from_slice(&result);
    master
}

/// Encode la clé en hex (format SQLCipher)
pub fn key_to_hex(key: &[u8; 32]) -> String {
    hex::encode(key)
}

/// Initialise la clé maître au démarrage du backend
pub fn init_master_key(owner_password: &str) -> Result<(), String> {
    let key = derive_master_key(owner_password);
    
    let storage = MASTER_KEY.get_or_init(|| Arc::new(Mutex::new(None)));
    let mut guard = storage.lock().map_err(|_| "Lock error")?;
    *guard = Some(key);
    
    eprintln!("🔐 Clé maître initialisée (dérivée du mot de passe owner)");
    Ok(())
}

/// Récupère la clé maître (pour ouvrir les DB chiffrées)
pub fn get_master_key() -> Option<[u8; 32]> {
    let storage = MASTER_KEY.get_or_init(|| Arc::new(Mutex::new(None)));
    let guard = storage.lock().ok()?;
    *guard
}

/// Vérifie si la clé maître est chargée
pub fn is_unlocked() -> bool {
    get_master_key().is_some()
}

/// Verrouille le système (retire la clé de la mémoire)
pub fn lock_system() {
    let storage = MASTER_KEY.get_or_init(|| Arc::new(Mutex::new(None)));
    if let Ok(mut guard) = storage.lock() {
        *guard = None;
    }
    eprintln!("🔒 Système verrouillé");
}

/// Ouvre une DB chiffrée avec la clé maître
pub fn open_encrypted_db(path: &str) -> Result<rusqlite::Connection, String> {
    let master = get_master_key().ok_or_else(|| "Système verrouillé — clé maître manquante".to_string())?;
    let key_hex = key_to_hex(&master);
    
    let conn = rusqlite::Connection::open(path)
        .map_err(|e| format!("Ouverture DB échouée: {}", e))?;
    
    // Appliquer la clé SQLCipher
    conn.pragma_update(None, "key", &key_hex)
        .map_err(|e| format!("Application clé échouée: {}", e))?;
    
    // Test de déchiffrement
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(()))
        .map_err(|_| "Déchiffrement échoué — clé invalide".to_string())?;
    
    Ok(conn)
}

/// Chiffre une DB en clair (migration)
pub fn encrypt_plain_db(plain_path: &str, output_path: &str, master_key: &[u8; 32]) -> Result<(), String> {
    let key_hex = key_to_hex(master_key);
    
    // Ouvrir la DB en clair
    let source = rusqlite::Connection::open(plain_path)
        .map_err(|e| format!("Ouverture source échouée: {}", e))?;
    
    // Créer la DB chiffrée
    let dest = rusqlite::Connection::open(output_path)
        .map_err(|e| format!("Création dest échouée: {}", e))?;
    dest.pragma_update(None, "key", &key_hex)
        .map_err(|e| format!("Application clé échouée: {}", e))?;
    
    // Attach + copie complète
    source.execute(
        &format!("ATTACH DATABASE '{}' AS encrypted KEY '{}'", output_path, key_hex),
        []
    ).map_err(|e| format!("ATTACH échoué: {}", e))?;
    
    source.execute("SELECT sqlcipher_export('encrypted')", [])
        .map_err(|e| format!("Export échoué: {}", e))?;
    
    source.execute("DETACH DATABASE encrypted", [])
        .map_err(|e| format!("DETACH échoué: {}", e))?;
    
    Ok(())
}
