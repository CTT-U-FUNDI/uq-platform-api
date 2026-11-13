//! Gestion multi-wallets par compte UQ.
//! Max 10 wallets : 6 online + 4 offline.
//! Chaque wallet a sa clé publique, son type, et son état 2FA.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum WalletType {
    Online,   // Connecté à UQ, transactions quotidiennes
    Offline,  // Cold storage, USB key optionnel
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletInfo {
    pub vk_hex: String,           // Clé publique Ed25519 (64 hex chars)
    pub nom: String,              // Nom affiché (ex: "Principal", "Recovery 1")
    pub r#type: WalletType,       // Online ou Offline
    pub cree_le: u64,             // Timestamp création
    pub actif: bool,              // Wallet actif ou gelé
    pub usb_key_hash: Option<String>, // Hash de la clé USB si offline+usb
    pub derniere_connexion: Option<u64>, // Dernière fois connecté
    pub solde_cache: Option<u128>, // Solde en cache (femto-BORB)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletsCompte {
    pub uq: String,
    pub wallets: Vec<WalletInfo>, // Max 10
    pub wallet_principal: Option<String>, // vk_hex du wallet principal
    pub mis_a_jour: u64,
}

pub struct WalletsStore {
    dir: PathBuf,
    cache: Mutex<HashMap<String, WalletsCompte>>,
}

pub const MAX_WALLETS: usize = 10;
pub const MAX_ONLINE: usize = 6;
pub const MAX_OFFLINE: usize = 4;

impl WalletsStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        let _ = std::fs::create_dir_all(&dir);
        Self { dir, cache: Mutex::new(HashMap::new()) }
    }

    fn path(&self, uq: &str) -> PathBuf {
        self.dir.join(format!("wallets_{}.json", uq.replace(|c: char| !c.is_ascii_alphanumeric() && c != '-', "_")))
    }

    fn charger(&self, uq: &str) -> Option<WalletsCompte> {
        if let Some(c) = self.cache.lock().unwrap().get(uq) {
            return Some(c.clone());
        }
        let p = self.path(uq);
        let brut = std::fs::read_to_string(&p).ok()?;
        let c: WalletsCompte = serde_json::from_str(&brut).ok()?;
        self.cache.lock().unwrap().insert(uq.to_string(), c.clone());
        Some(c)
    }

    fn sauver(&self, uq: &str, c: &WalletsCompte) {
        let _ = std::fs::write(self.path(uq), serde_json::to_string(c).unwrap_or_default());
        self.cache.lock().unwrap().insert(uq.to_string(), c.clone());
    }

    /// Liste tous les wallets d'un compte
    pub fn lister(&self, uq: &str) -> serde_json::Value {
        match self.charger(uq) {
            Some(c) => serde_json::json!({
                "uq": uq,
                "wallets": c.wallets.iter().map(|w| serde_json::json!({
                    "vk": w.vk_hex,
                    "nom": w.nom,
                    "type": w.r#type,
                    "actif": w.actif,
                    "cree_le": w.cree_le,
                    "usb_key": w.usb_key_hash.is_some(),
                    "derniere_connexion": w.derniere_connexion,
                    "principal": c.wallet_principal.as_ref() == Some(&w.vk_hex),
                })).collect::<Vec<_>>(),
                "total": c.wallets.len(),
                "max": MAX_WALLETS,
                "online": c.wallets.iter().filter(|w| w.r#type == WalletType::Online).count(),
                "offline": c.wallets.iter().filter(|w| w.r#type == WalletType::Offline).count(),
            }),
            None => serde_json::json!({
                "uq": uq,
                "wallets": [],
                "total": 0,
                "max": MAX_WALLETS,
            }),
        }
    }

    /// Ajoute un wallet (max 10, vérifie limites online/offline)
    pub fn ajouter(&self, uq: &str, vk_hex: &str, nom: &str, r#type: WalletType) -> Result<WalletInfo, String> {
        let mut c = self.charger(uq).unwrap_or_else(|| WalletsCompte {
            uq: uq.to_string(),
            wallets: Vec::new(),
            wallet_principal: None,
            mis_a_jour: now_ts(),
        });

        // Vérifier doublon
        if c.wallets.iter().any(|w| w.vk_hex == vk_hex) {
            return Err("WALLET_DEJA_EXISTE".to_string());
        }

        // Vérifier limite totale
        if c.wallets.len() >= MAX_WALLETS {
            return Err(format!("MAX_WALLETS: {} max", MAX_WALLETS));
        }

        // Vérifier limite par type
        let count_type = c.wallets.iter().filter(|w| w.r#type == r#type).count();
        let max_type = match r#type {
            WalletType::Online => MAX_ONLINE,
            WalletType::Offline => MAX_OFFLINE,
        };
        if count_type >= max_type {
            return Err(format!("MAX_{}: {} max", format!("{:?}", r#type).to_uppercase(), max_type));
        }

        let wallet = WalletInfo {
            vk_hex: vk_hex.to_string(),
            nom: nom.to_string(),
            r#type: r#type.clone(),
            cree_le: now_ts(),
            actif: true,
            usb_key_hash: None,
            derniere_connexion: None,
            solde_cache: None,
        };

        // Premier wallet = principal
        if c.wallet_principal.is_none() {
            c.wallet_principal = Some(vk_hex.to_string());
        }

        c.wallets.push(wallet.clone());
        c.mis_a_jour = now_ts();
        self.sauver(uq, &c);
        Ok(wallet)
    }

    /// Récupère un wallet par sa clé publique
    pub fn trouver(&self, uq: &str, vk_hex: &str) -> Option<WalletInfo> {
        self.charger(uq)?.wallets.into_iter().find(|w| w.vk_hex == vk_hex)
    }

    /// Met à jour le statut d'un wallet
    pub fn maj_statut(&self, uq: &str, vk_hex: &str, actif: bool) -> Result<(), String> {
        let mut c = self.charger(uq).ok_or("COMPTE_INCONNU")?;
        let w = c.wallets.iter_mut().find(|w| w.vk_hex == vk_hex).ok_or("WALLET_INCONNU")?;
        w.actif = actif;
        c.mis_a_jour = now_ts();
        self.sauver(uq, &c);
        Ok(())
    }

    /// Enregistre une clé USB pour un wallet offline
    pub fn lier_usb(&self, uq: &str, vk_hex: &str, usb_hash: &str) -> Result<(), String> {
        let mut c = self.charger(uq).ok_or("COMPTE_INCONNU")?;
        let w = c.wallets.iter_mut().find(|w| w.vk_hex == vk_hex).ok_or("WALLET_INCONNU")?;
        if w.r#type != WalletType::Offline {
            return Err("USB_SEULEMENT_OFFLINE".to_string());
        }
        w.usb_key_hash = Some(usb_hash.to_string());
        c.mis_a_jour = now_ts();
        self.sauver(uq, &c);
        Ok(())
    }

    /// Vérifie si un wallet offline a une clé USB
    pub fn a_usb_key(&self, uq: &str, vk_hex: &str) -> bool {
        self.trouver(uq, vk_hex).map(|w| w.usb_key_hash.is_some()).unwrap_or(false)
    }

    /// Compte les wallets par type
    pub fn compter_par_type(&self, uq: &str) -> (usize, usize) {
        match self.charger(uq) {
            Some(c) => (
                c.wallets.iter().filter(|w| w.r#type == WalletType::Online).count(),
                c.wallets.iter().filter(|w| w.r#type == WalletType::Offline).count(),
            ),
            None => (0, 0),
        }
    }

    /// Wallet principal du compte
    pub fn principal(&self, uq: &str) -> Option<String> {
        self.charger(uq)?.wallet_principal
    }

    /// Change le wallet principal
    pub fn definir_principal(&self, uq: &str, vk_hex: &str) -> Result<(), String> {
        let mut c = self.charger(uq).ok_or("COMPTE_INCONNU")?;
        if !c.wallets.iter().any(|w| w.vk_hex == vk_hex) {
            return Err("WALLET_INCONNU".to_string());
        }
        c.wallet_principal = Some(vk_hex.to_string());
        c.mis_a_jour = now_ts();
        self.sauver(uq, &c);
        Ok(())
    }
}

fn now_ts() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
