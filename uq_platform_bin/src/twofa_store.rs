//! 2FA par compte UQ — store fichier data/2fa/{uq}.json.
//! Secret compte + wallets liés par clé publique (vk).
//! Le wallet n'a PAS besoin de JWT : il prouve son vk par signature
//! Ed25519 sur le message canonique 2FA (vérifié dans les handlers).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::totp;

#[derive(Serialize, Deserialize, Clone)]
pub struct TwoFaCompte {
    pub secret_b32: String, // secret du COMPTE (jamais exposé)
    pub created_at: u64,
    pub rotated_at: u64,
    /// vk_hex -> true : wallets liés (le code TOTP dépend du vk)
    pub wallets: HashMap<String, bool>,
    /// hash sha256 des codes de secours restants
    pub backup: Vec<String>,
}

pub struct TwoFaStore {
    dir: PathBuf,
    cache: Mutex<HashMap<String, TwoFaCompte>>,
}

impl TwoFaStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        let _ = std::fs::create_dir_all(&dir);
        Self {
            dir,
            cache: Mutex::new(HashMap::new()),
        }
    }

    fn path(&self, uq: &str) -> PathBuf {
        // uq est validé en amont (ressemble_a_un_uq) — pas de traversal
        self.dir.join(format!(
            "{}.json",
            uq.replace(|c: char| !c.is_ascii_alphanumeric() && c != '-', "_")
        ))
    }

    fn charger(&self, uq: &str) -> Option<TwoFaCompte> {
        if let Some(c) = self.cache.lock().unwrap().get(uq) {
            return Some(c.clone());
        }
        let p = self.path(uq);
        let brut = std::fs::read_to_string(&p).ok()?;
        let c: TwoFaCompte = serde_json::from_str(&brut).ok()?;
        self.cache.lock().unwrap().insert(uq.to_string(), c.clone());
        Some(c)
    }

    fn sauver(&self, uq: &str, c: &TwoFaCompte) {
        let _ = std::fs::write(self.path(uq), serde_json::to_string(c).unwrap_or_default());
        self.cache.lock().unwrap().insert(uq.to_string(), c.clone());
    }

    pub fn statut(&self, uq: &str) -> serde_json::Value {
        match self.charger(uq) {
            Some(c) => serde_json::json!({
                "actif": true,
                "created_at": c.created_at,
                "rotated_at": c.rotated_at,
                "wallets": c.wallets.keys().collect::<Vec<_>>(),
                "backup_restants": c.backup.len(),
            }),
            None => serde_json::json!({ "actif": false }),
        }
    }

    /// Active (ou retourne l'existant). Renvoie le secret compte UNE SEULE FOIS.
    pub fn enroll(&self, uq: &str) -> serde_json::Value {
        if let Some(c) = self.charger(uq) {
            return serde_json::json!({
                "status": "deja_actif",
                "message": "2FA déjà active — utilisez /api/2fa/rotate pour regénérer",
            });
        }
        let now = now_ts();
        let secret = totp::nouveau_secret_compte();
        let backup = backup_codes();
        let c = TwoFaCompte {
            secret_b32: secret.clone(),
            created_at: now,
            rotated_at: now,
            wallets: HashMap::new(),
            backup: backup.iter().map(|b| sha256_hex(b)).collect(),
        };
        self.sauver(uq, &c);
        serde_json::json!({
            "status": "success",
            "secret": secret,
            "otpauth_compte": format!("otpauth://totp/UQ:{}?secret={}&issuer=UQ&digits=6&period=30", uq, secret),
            "backup_codes": backup,
            "message": "NOTEZ les codes de secours — affichés une seule fois",
        })
    }

    /// Rotation : nouveau secret → tous les codes changent (y compris wallets).
    pub fn rotate(&self, uq: &str) -> serde_json::Value {
        let mut c = match self.charger(uq) {
            Some(c) => c,
            None => {
                return serde_json::json!({"status":"error","message":"2FA non activée — enroll d'abord"})
            }
        };
        let now = now_ts();
        c.secret_b32 = totp::nouveau_secret_compte();
        c.rotated_at = now;
        c.wallets.clear();
        let backup = backup_codes();
        c.backup = backup.iter().map(|b| sha256_hex(b)).collect();
        self.sauver(uq, &c);
        serde_json::json!({
            "status": "success",
            "secret": c.secret_b32,
            "backup_codes": backup,
            "message": "Secret regénéré — tous les codes (compte + wallets) ont changé. Re-liez vos wallets.",
        })
    }

    /// Lie un wallet (vk) au compte — appelé par l'app UQ (JWT requis en amont).
    pub fn lier_wallet(&self, uq: &str, vk_hex: &str) -> serde_json::Value {
        let mut c = match self.charger(uq) {
            Some(c) => c,
            None => return serde_json::json!({"status":"error","message":"2FA non activée"}),
        };
        if c.wallets.len() >= 20 {
            return serde_json::json!({"status":"error","message":"Trop de wallets liés (max 20)"});
        }
        c.wallets.insert(vk_hex.to_string(), true);
        self.sauver(uq, &c);
        serde_json::json!({
            "status": "success",
            "vk": vk_hex,
            "otpauth": totp::otpauth_uri(&c.secret_b32, uq, vk_hex),
            "code_actuel": totp::code_pour(&totp::secret_wallet(&c.secret_b32, vk_hex), now_ts()),
        })
    }

    /// Code courant pour un wallet — affiché par l'app UQ.
    pub fn code_pour_wallet(&self, uq: &str, vk_hex: &str) -> Option<String> {
        let c = self.charger(uq)?;
        if !c.wallets.contains_key(vk_hex) {
            return None;
        }
        Some(totp::code_pour(
            &totp::secret_wallet(&c.secret_b32, vk_hex),
            now_ts(),
        ))
    }

    /// Vérifie code TOTP OU code de secours pour un wallet lié.
    /// `sig_valide` doit être true (signature Ed25519 vérifiée par l'appelant).
    pub fn verifier(&self, uq: &str, vk_hex: &str, code: &str) -> bool {
        let mut c = match self.charger(uq) {
            Some(c) => c,
            None => return false,
        };
        if !c.wallets.contains_key(vk_hex) {
            return false;
        }
        let sub = totp::secret_wallet(&c.secret_b32, vk_hex);
        if totp::verifier(&sub, code) {
            return true;
        }
        // code de secours ?
        let h = sha256_hex(&code.replace('-', "").to_lowercase());
        if let Some(i) = c.backup.iter().position(|b| *b == h) {
            c.backup.remove(i);
            self.sauver(uq, &c);
            return true;
        }
        false
    }

    /// Le wallet récupère le secret de SON sous-secret (auth par signature vk).
    /// ⚠️ Le sous-secret seul ne compromet pas le secret compte.
    pub fn secret_pour_wallet(&self, uq: &str, vk_hex: &str) -> Option<String> {
        let c = self.charger(uq)?;
        if !c.wallets.contains_key(vk_hex) {
            return None;
        }
        Some(totp::secret_wallet(&c.secret_b32, vk_hex))
    }
}

/// Instance globale partagée — OBLIGATOIRE : une instance par appel
/// créerait un cache périmé (codes de secours consommés réutilisables).
pub fn shared() -> &'static TwoFaStore {
    static STORE: std::sync::OnceLock<TwoFaStore> = std::sync::OnceLock::new();
    STORE.get_or_init(|| TwoFaStore::new("data/2fa"))
}

fn now_ts() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn sha256_hex(s: &str) -> String {
    use sha2::{Digest, Sha256};
    let h = Sha256::digest(s.as_bytes());
    h.iter().map(|b| format!("{:02x}", b)).collect()
}

fn backup_codes() -> Vec<String> {
    use rand::RngCore;
    let chars: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut rng = rand::thread_rng();
    (0..8)
        .map(|_| {
            let mut c = String::with_capacity(9);
            for i in 0..8 {
                if i == 4 {
                    c.push('-');
                }
                c.push(chars[(rng.next_u32() as usize) % chars.len()] as char);
            }
            c
        })
        .collect()
}
