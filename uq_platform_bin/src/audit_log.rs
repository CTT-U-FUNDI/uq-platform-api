//! Journal d'audit chaîné des émissions de comptes UQ.
//!
//! Chaque compte créé écrit une entrée dont le hash dépend du hash de
//! l'entrée précédente : modifier ou supprimer une entrée casse toute
//! la chaîne à partir de ce point — falsification détectable.
//!
//! Format (JSON Lines, un objet par ligne) :
//!   {"seq":N,"ts":...,"uq":"...","email":"...","pseudo":"...",
//!    "promo":"...","ident_hash":"...","prev":"...","hash":"..."}
//!
//! hash = SHA256(prev | ts | uq | email | pseudo | promo | ident_hash)
//!
//! Le journal ne contient JAMAIS de mot de passe ni de clé privée :
//! seulement des identifiants publics et le hash d'identité (déjà haché).

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const LOG_FILE: &str = "issued_accounts.log";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub seq: u64,
    pub ts: u64,
    pub uq: String,
    pub email: String,
    pub pseudo: String,
    pub promo: String,
    /// Hash de l'identité (nom+prénom+naissance+pays) — déjà haché en amont.
    pub ident_hash: String,
    /// Hash de l'entrée précédente (GENESIS_HASH pour la première).
    pub prev: String,
    /// Hash de cette entrée.
    pub hash: String,
}

pub struct AuditLog {
    dir: PathBuf,
    last: Mutex<(u64, String)>, // (seq, hash) de la dernière entrée
}

impl AuditLog {
    fn new(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        let _ = std::fs::create_dir_all(&dir);
        // Reprendre la chaîne là où elle s'est arrêtée (redémarrage safe)
        let (seq, hash) = Self::lire_dernier(&dir);
        Self {
            dir,
            last: Mutex::new((seq, hash)),
        }
    }

    fn path(&self) -> PathBuf {
        self.dir.join(LOG_FILE)
    }

    fn lire_dernier(dir: &PathBuf) -> (u64, String) {
        let path = dir.join(LOG_FILE);
        let Ok(content) = std::fs::read_to_string(&path) else {
            return (0, GENESIS_HASH.to_string());
        };
        let mut last_seq = 0u64;
        let mut last_hash = GENESIS_HASH.to_string();
        for line in content.lines() {
            if let Ok(e) = serde_json::from_str::<AuditEntry>(line) {
                last_seq = e.seq;
                last_hash = e.hash;
            }
        }
        (last_seq, last_hash)
    }

    /// Calcule le hash d'une entrée à partir de ses champs publics.
    fn calculer_hash(
        prev: &str,
        ts: u64,
        uq: &str,
        email: &str,
        pseudo: &str,
        promo: &str,
        ident: &str,
    ) -> String {
        let mut h = Sha256::new();
        h.update(prev.as_bytes());
        h.update(b"|");
        h.update(ts.to_string().as_bytes());
        h.update(b"|");
        h.update(uq.as_bytes());
        h.update(b"|");
        h.update(email.as_bytes());
        h.update(b"|");
        h.update(pseudo.as_bytes());
        h.update(b"|");
        h.update(promo.as_bytes());
        h.update(b"|");
        h.update(ident.as_bytes());
        hex::encode(h.finalize())
    }

    /// Enregistre l'émission d'un compte. Retourne l'entrée créée.
    pub fn record_account(
        &self,
        uq: &str,
        email: &str,
        pseudo: &str,
        promo: &str,
        ident_hash: &str,
    ) -> Result<AuditEntry, String> {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let mut guard = self.last.lock().map_err(|e| e.to_string())?;
        let (seq, prev) = (guard.0 + 1, guard.1.clone());
        let hash = Self::calculer_hash(&prev, ts, uq, email, pseudo, promo, ident_hash);

        let entry = AuditEntry {
            seq,
            ts,
            uq: uq.to_string(),
            email: email.to_string(),
            pseudo: pseudo.to_string(),
            promo: promo.to_string(),
            ident_hash: ident_hash.to_string(),
            prev,
            hash: hash.clone(),
        };

        let line = serde_json::to_string(&entry).map_err(|e| e.to_string())?;
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.path())
            .map_err(|e| e.to_string())?;
        writeln!(f, "{}", line).map_err(|e| e.to_string())?;
        f.sync_all().ok(); // durabilité : flush sur disque

        *guard = (seq, hash);
        Ok(entry)
    }

    /// Vérifie l'intégrité complète de la chaîne.
    /// Retourne (ok, nb_entrees, premiere_rupture_seq).
    pub fn verifier(&self) -> (bool, u64, Option<u64>) {
        let path = self.path();
        let Ok(content) = std::fs::read_to_string(&path) else {
            return (true, 0, None); // journal vide = valide
        };

        let mut prev = GENESIS_HASH.to_string();
        let mut count = 0u64;
        for (i, line) in content.lines().enumerate() {
            let Ok(e) = serde_json::from_str::<AuditEntry>(line) else {
                return (false, count, Some(i as u64 + 1));
            };
            if e.seq != count + 1 || e.prev != prev {
                return (false, count, Some(e.seq));
            }
            let attendu = Self::calculer_hash(
                &e.prev,
                e.ts,
                &e.uq,
                &e.email,
                &e.pseudo,
                &e.promo,
                &e.ident_hash,
            );
            if attendu != e.hash {
                return (false, count, Some(e.seq));
            }
            prev = e.hash;
            count += 1;
        }
        (true, count, None)
    }

    /// Nombre d'entrées enregistrées.
    pub fn count(&self) -> u64 {
        self.last.lock().map(|g| g.0).unwrap_or(0)
    }
}

static AUDIT: OnceLock<AuditLog> = OnceLock::new();

/// Instance globale du journal d'audit.
pub fn shared() -> &'static AuditLog {
    AUDIT.get_or_init(|| AuditLog::new("data/audit"))
}

/// Raccourci : enregistre une émission de compte (ne fait jamais échouer
/// la création — l'appelant traite l'erreur en warning).
pub fn record_account_issued(uq: &str, email: &str, pseudo: &str, promo: &str, ident_hash: &str) {
    if let Err(e) = shared().record_account(uq, email, pseudo, promo, ident_hash) {
        eprintln!("⚠️  [audit] Échec enregistrement {} : {}", uq, e);
    } else {
        eprintln!(
            "📝 [audit] Émission enregistrée : {} (seq={})",
            uq,
            shared().count()
        );
    }
}

// ============================================================================
// CHAÎNE D'ÉVÉNEMENTS SÉCURITÉ — 2FA, appareils, rotations
// ============================================================================
// Chaque événement sensible (enrôlement 2FA, liaison wallet, rotation)
// écrit une entrée chaînée : la preuve que l'intégralité du compte est
// traçable depuis sa création. Ne stocke JAMAIS de secret — seulement
// kind + uq + detail public (vk tronqué, etc.).
// Fichier : security_events.log — chaîne indépendante de issued_accounts.

const SEC_LOG_FILE: &str = "security_events.log";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityEvent {
    pub seq: u64,
    pub ts: u64,
    /// TWOFA_ENROLLED | TWOFA_ROTATED | DEVICE_LINKED | DEVICE_REVOKED | ...
    pub kind: String,
    pub uq: String,
    /// Détail public non sensible (ex: vk tronqué à 16 chars).
    pub detail: String,
    pub prev: String,
    pub hash: String,
}

pub struct SecurityAudit {
    dir: PathBuf,
    last: Mutex<(u64, String)>,
}

impl SecurityAudit {
    fn new(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        let _ = std::fs::create_dir_all(&dir);
        let (seq, hash) = Self::lire_dernier(&dir);
        Self {
            dir,
            last: Mutex::new((seq, hash)),
        }
    }

    fn path(&self) -> PathBuf {
        self.dir.join(SEC_LOG_FILE)
    }

    fn lire_dernier(dir: &PathBuf) -> (u64, String) {
        let path = dir.join(SEC_LOG_FILE);
        let Ok(content) = std::fs::read_to_string(&path) else {
            return (0, GENESIS_HASH.to_string());
        };
        let mut last_seq = 0u64;
        let mut last_hash = GENESIS_HASH.to_string();
        for line in content.lines() {
            if let Ok(e) = serde_json::from_str::<SecurityEvent>(line) {
                last_seq = e.seq;
                last_hash = e.hash;
            }
        }
        (last_seq, last_hash)
    }

    fn calculer_hash(prev: &str, ts: u64, kind: &str, uq: &str, detail: &str) -> String {
        let mut h = Sha256::new();
        h.update(prev.as_bytes());
        h.update(b"|");
        h.update(ts.to_string().as_bytes());
        h.update(b"|");
        h.update(kind.as_bytes());
        h.update(b"|");
        h.update(uq.as_bytes());
        h.update(b"|");
        h.update(detail.as_bytes());
        hex::encode(h.finalize())
    }

    /// Enregistre un événement sécurité. Ne fait jamais échouer l'appelant.
    pub fn record(&self, kind: &str, uq: &str, detail: &str) -> Result<SecurityEvent, String> {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut guard = self.last.lock().map_err(|e| e.to_string())?;
        let (seq, prev) = (guard.0 + 1, guard.1.clone());
        let hash = Self::calculer_hash(&prev, ts, kind, uq, detail);
        let entry = SecurityEvent {
            seq,
            ts,
            kind: kind.to_string(),
            uq: uq.to_string(),
            detail: detail.to_string(),
            prev,
            hash: hash.clone(),
        };
        let line = serde_json::to_string(&entry).map_err(|e| e.to_string())?;
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.path())
            .map_err(|e| e.to_string())?;
        writeln!(f, "{}", line).map_err(|e| e.to_string())?;
        f.sync_all().ok();
        *guard = (seq, hash);
        Ok(entry)
    }

    /// Vérifie l'intégrité de la chaîne événements. (ok, count, rupture_seq)
    pub fn verifier(&self) -> (bool, u64, Option<u64>) {
        let path = self.path();
        let Ok(content) = std::fs::read_to_string(&path) else {
            return (true, 0, None);
        };
        let mut prev = GENESIS_HASH.to_string();
        let mut count = 0u64;
        for line in content.lines() {
            let Ok(e) = serde_json::from_str::<SecurityEvent>(line) else {
                return (false, count, Some(count + 1));
            };
            if e.seq != count + 1 || e.prev != prev {
                return (false, count, Some(e.seq));
            }
            let attendu = Self::calculer_hash(&e.prev, e.ts, &e.kind, &e.uq, &e.detail);
            if attendu != e.hash {
                return (false, count, Some(e.seq));
            }
            prev = e.hash;
            count += 1;
        }
        (true, count, None)
    }

    /// Événements d'un compte (filtres, sans secrets).
    pub fn evenements_de(&self, uq: &str) -> Vec<SecurityEvent> {
        let Ok(content) = std::fs::read_to_string(self.path()) else {
            return Vec::new();
        };
        content
            .lines()
            .filter_map(|l| serde_json::from_str::<SecurityEvent>(l).ok())
            .filter(|e| e.uq == uq)
            .collect()
    }
}

static SEC_AUDIT: OnceLock<SecurityAudit> = OnceLock::new();

/// Instance globale de la chaîne événements sécurité.
pub fn security() -> &'static SecurityAudit {
    SEC_AUDIT.get_or_init(|| SecurityAudit::new("data/audit"))
}

/// Raccourci : enregistre un événement sécurité (warning seulement).
pub fn record_security_event(kind: &str, uq: &str, detail: &str) {
    if let Err(e) = security().record(kind, uq, detail) {
        eprintln!("⚠️  [audit-sec] Échec {} {} : {}", kind, uq, e);
    }
}
