//! Protection des binaires propriétaires — clés de déchiffrement côté serveur.
//! Le binaire chiffré ne peut pas tourner sans autorisation UQ owner.
//! Chaque accès est loggé et nécessite une signature Ed25519 fraîche.

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

/// Une clé de déchiffrement pour un binaire protégé.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleBinaire {
    pub id: String,              // Identifiant du binaire (ex: "borb_node", "uq_platform")
    pub nom: String,             // Nom affiché
    pub cle_chiffrement: String, // Clé AES-256 en hex (64 chars)
    pub algorithme: String,      // "AES-256-GCM"
    pub created_at: u64,
    pub last_used: Option<u64>,
    pub use_count: u64,
    pub actif: bool,
}

/// Requête de déchiffrement.
#[derive(Debug, Deserialize)]
pub struct DemandeDecrypt {
    pub binaire_id: String,      // "borb_node", "uq_platform", etc.
    pub uq_owner: String,        // Doit être UQ-996697110
    pub ts: u64,                 // Timestamp anti-rejeu
    pub signature: String,       // Signature Ed25519 du owner
}

/// Store des clés de binaires.
pub struct BinaireStore {
    dir: PathBuf,
    cache: Mutex<HashMap<String, CleBinaire>>,
}

impl BinaireStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        let _ = std::fs::create_dir_all(&dir);
        Self { dir, cache: Mutex::new(HashMap::new()) }
    }

    fn path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("bin_{}.json", id.replace(|c: char| !c.is_ascii_alphanumeric() && c != '-', "_")))
    }

    fn charger(&self, id: &str) -> Option<CleBinaire> {
        if let Some(c) = self.cache.lock().unwrap().get(id) {
            return Some(c.clone());
        }
        let p = self.path(id);
        let brut = std::fs::read_to_string(&p).ok()?;
        let c: CleBinaire = serde_json::from_str(&brut).ok()?;
        self.cache.lock().unwrap().insert(id.to_string(), c.clone());
        Some(c)
    }

    fn sauver(&self, id: &str, c: &CleBinaire) {
        let _ = std::fs::write(self.path(id), serde_json::to_string(c).unwrap_or_default());
        self.cache.lock().unwrap().insert(id.to_string(), c.clone());
    }

    /// Crée ou met à jour une clé de binaire.
    pub fn definir_cle(&self, id: &str, nom: &str, cle_hex: &str) -> Result<(), String> {
        // Valider la clé : 64 hex chars = 32 bytes = AES-256
        if cle_hex.len() != 64 || !cle_hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err("CLE_INVALIDE: 64 hex chars requis (AES-256)".to_string());
        }

        let mut c = self.charger(id).unwrap_or_else(|| CleBinaire {
            id: id.to_string(),
            nom: nom.to_string(),
            cle_chiffrement: String::new(),
            algorithme: "AES-256-GCM".to_string(),
            created_at: now_ts(),
            last_used: None,
            use_count: 0,
            actif: true,
        });

        c.cle_chiffrement = cle_hex.to_string();
        c.nom = nom.to_string();
        c.actif = true;
        self.sauver(id, &c);
        Ok(())
    }

    /// Récupère une clé si autorisée.
    pub fn obtenir_cle(&self, id: &str) -> Option<String> {
        let mut c = self.charger(id)?;
        if !c.actif {
            return None;
        }
        c.last_used = Some(now_ts());
        c.use_count += 1;
        self.sauver(id, &c);
        Some(c.cle_chiffrement.clone())
    }

    /// Vérifie si un binaire a une clé configurée.
    pub fn existe(&self, id: &str) -> bool {
        self.charger(id).map(|c| c.actif).unwrap_or(false)
    }

    /// Liste tous les binaires protégés.
    pub fn lister(&self) -> Vec<serde_json::Value> {
        let cache = self.cache.lock().unwrap();
        cache.values().map(|c| serde_json::json!({
            "id": c.id,
            "nom": c.nom,
            "actif": c.actif,
            "created_at": c.created_at,
            "last_used": c.last_used,
            "use_count": c.use_count,
        })).collect()
    }

    /// Révoque une clé (binaire ne peut plus démarrer).
    pub fn revoquer(&self, id: &str) -> Result<(), String> {
        let mut c = self.charger(id).ok_or("BINAIRE_INCONNU")?;
        c.actif = false;
        self.sauver(id, &c);
        Ok(())
    }
}

/// Vérifie la signature Ed25519 du owner.
/// Message : UQ_BIN_V1|{binaire_id}|{uq}|{ts}
pub fn signature_owner_valide(binaire_id: &str, uq: &str, ts: u64, signature_hex: &str, owner_vk_hex: &str) -> bool {
    // Anti-rejeu : 5 minutes max
    if ts.abs_diff(now_ts()) > 300 {
        return false;
    }

    // Vérifier que c'est bien le owner
    if uq != "UQ-996697110" {
        return false;
    }

    // Vérifier la signature
    let vk_bytes = match hex::decode(owner_vk_hex) {
        Ok(b) if b.len() == 32 => b,
        _ => return false,
    };
    let sig_bytes = match hex::decode(signature_hex) {
        Ok(b) if b.len() == 64 => b,
        _ => return false,
    };

    let vk_arr: [u8; 32] = vk_bytes.try_into().unwrap();
    let vk = match VerifyingKey::from_bytes(&vk_arr) {
        Ok(k) => k,
        Err(_) => return false,
    };

    let msg = format!("UQ_BIN_V1|{}|{}|{}", binaire_id, uq, ts);
    let sig = Signature::from_slice(&sig_bytes).unwrap();
    vk.verify(msg.as_bytes(), &sig).is_ok()
}

fn now_ts() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
