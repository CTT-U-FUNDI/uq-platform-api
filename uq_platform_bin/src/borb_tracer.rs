// ====================
// BORB TRACER — Stabilisateur / Traceur de BORB
// ====================
//
// Chaque BORB créé (sortie du Trésor) reçoit :
// - Une CLÉ DE TRAÇAGE unique (hash cadenas + timestamp + ...)
// - Une SIGNATURE Ed25519 (owner)
// - Un ENREGISTREMENT dans la table borb_origins
//
// → Impossible de créer un BORB sans trace
// → Vérifiable par n'importe quel nœud
//
// ====================

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Origine d'un BORB
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BorbOrigin {
    /// Clé de traçage unique (SHA-256 hex)
    pub trace_key: String,
    /// Timestamp exact de création
    pub timestamp: i64,
    /// Cadenas du créateur (Trésor)
    pub cadenas_owner: String,
    /// UQ du destinataire
    pub recipient_uq: String,
    /// Montant créé (en points)
    pub amount_points: u64,
    /// Solde du Trésor avant
    pub tresor_before: u64,
    /// Solde du Trésor après
    pub tresor_after: u64,
    /// Signature Ed25519 (hex)
    pub signature: String,
    /// UQ du Trésor (créateur)
    pub treasury_uq: String,
}

/// Calcule la clé de traçage
pub fn calculer_trace_key(
    cadenas_owner: &str,
    timestamp: i64,
    recipient_uq: &str,
    amount_points: u64,
    tresor_before: u64,
    tresor_after: u64,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"UQ_BORB_TRACE_V1:");
    hasher.update(cadenas_owner.as_bytes());
    hasher.update(b":");
    hasher.update(timestamp.to_le_bytes());
    hasher.update(b":");
    hasher.update(recipient_uq.as_bytes());
    hasher.update(b":");
    hasher.update(amount_points.to_le_bytes());
    hasher.update(b":");
    hasher.update(tresor_before.to_le_bytes());
    hasher.update(b":");
    hasher.update(tresor_after.to_le_bytes());

    format!("{:x}", hasher.finalize())
}

/// Enregistre l'origine d'un BORB
pub fn enregistrer_origine(conn: &Connection, origin: &BorbOrigin) -> Result<(), String> {
    conn.execute(
        "INSERT INTO borb_origins 
         (trace_key, timestamp, cadenas_owner, recipient_uq, amount_points, 
          tresor_before, tresor_after, signature, treasury_uq, verified)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0)",
        params![
            &origin.trace_key,
            origin.timestamp,
            &origin.cadenas_owner,
            &origin.recipient_uq,
            origin.amount_points as i64,
            origin.tresor_before as i64,
            origin.tresor_after as i64,
            &origin.signature,
            &origin.treasury_uq,
        ],
    )
    .map_err(|e| e.to_string())?;

    Ok(())
}

/// Vérifie l'origine d'un BORB
pub fn verifier_origine(conn: &Connection, trace_key: &str) -> Result<BorbOrigin, String> {
    let result = conn
        .query_row(
            "SELECT trace_key, timestamp, cadenas_owner, recipient_uq, amount_points,
                tresor_before, tresor_after, signature, treasury_uq
         FROM borb_origins WHERE trace_key = ?1",
            params![trace_key],
            |row| {
                Ok(BorbOrigin {
                    trace_key: row.get(0)?,
                    timestamp: row.get(1)?,
                    cadenas_owner: row.get(2)?,
                    recipient_uq: row.get(3)?,
                    amount_points: row.get::<_, i64>(4)? as u64,
                    tresor_before: row.get::<_, i64>(5)? as u64,
                    tresor_after: row.get::<_, i64>(6)? as u64,
                    signature: row.get(7)?,
                    treasury_uq: row.get(8)?,
                })
            },
        )
        .map_err(|e| format!("Origine introuvable: {}", e))?;

    Ok(result)
}

/// Crée la table borb_origins si elle n'existe pas
pub fn init_borb_origins_table(conn: &Connection) -> Result<(), String> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS borb_origins (
            trace_key TEXT PRIMARY KEY,
            timestamp INTEGER NOT NULL,
            cadenas_owner TEXT NOT NULL,
            recipient_uq TEXT NOT NULL,
            amount_points INTEGER NOT NULL,
            tresor_before INTEGER NOT NULL,
            tresor_after INTEGER NOT NULL,
            signature TEXT NOT NULL,
            treasury_uq TEXT NOT NULL,
            verified INTEGER DEFAULT 0,
            created_at INTEGER NOT NULL
        )",
        [],
    )
    .map_err(|e| e.to_string())?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_borb_origins_recipient ON borb_origins(recipient_uq)",
        [],
    )
    .map_err(|e| e.to_string())?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_borb_origins_timestamp ON borb_origins(timestamp)",
        [],
    )
    .map_err(|e| e.to_string())?;

    Ok(())
}

/// Trace une émission de BORB (appelée par le Trésor)
pub fn tracer_emission(
    conn: &Connection,
    cadenas_owner: &str,
    recipient_uq: &str,
    amount_points: u64,
    tresor_before: u64,
    tresor_after: u64,
    signature_hex: &str,
    treasury_uq: &str,
) -> Result<String, String> {
    let timestamp = chrono::Utc::now().timestamp();

    let trace_key = calculer_trace_key(
        cadenas_owner,
        timestamp,
        recipient_uq,
        amount_points,
        tresor_before,
        tresor_after,
    );

    let origin = BorbOrigin {
        trace_key: trace_key.clone(),
        timestamp,
        cadenas_owner: cadenas_owner.to_string(),
        recipient_uq: recipient_uq.to_string(),
        amount_points,
        tresor_before,
        tresor_after,
        signature: signature_hex.to_string(),
        treasury_uq: treasury_uq.to_string(),
    };

    enregistrer_origine(conn, &origin)?;

    eprintln!(
        "🔐 BORB tracé : {} ({} points → {})",
        &trace_key[..16],
        amount_points,
        recipient_uq
    );

    Ok(trace_key)
}

// ====================
// CONSTANTES USD — Valeurs FIXES (indépendantes du prix BORB)
// ====================

/// Valeur du code promo en USD (FIXE)
pub const PROMO_VALUE_USD: f64 = 1.0;

/// Valeur de l'UBI quotidien en USD (FIXE)
pub const UBI_VALUE_USD: f64 = 0.2;

/// Stock initial du Trésor pour l'owner (FIXE)
pub const TRESOR_OWNER_BORB: u64 = 1_000_000;

/// Pourcentage du Profil 1 vers Profil 2
pub const PROFILE2_CLAIM_PCT: u64 = 30;

/// Seuil bas du Profil 2 (déclenche réclamation)
pub const PROFILE2_THRESHOLD_BORB: u64 = 5_000;

/// Calcule les points selon la valeur USD et le prix
/// Formule : points = (valeur_USD / prix_BORB) × 100
pub fn usd_to_points(amount_usd: f64, prix_borb_usd: f64) -> u64 {
    if prix_borb_usd <= 0.0 {
        return 0;
    }
    let amount_borb = amount_usd / prix_borb_usd;
    (amount_borb * 100.0).round() as u64
}

/// Calcule les BORB selon la valeur USD et le prix
pub fn usd_to_borb(amount_usd: f64, prix_borb_usd: f64) -> f64 {
    if prix_borb_usd <= 0.0 {
        return 0.0;
    }
    amount_usd / prix_borb_usd
}

/// Tracer une émission avec valeur USD fixe
/// Utilisé pour : UBI, Code promo
pub fn trace_with_usd_value(
    conn: &Connection,
    source_type: &str,
    source_uq: &str,
    source_cadenas: &str,
    recipient_uq: &str,
    amount_usd: f64,
    prix_borb_usd: f64,
    signature_hex: &str,
) -> Result<String, String> {
    let amount_points = usd_to_points(amount_usd, prix_borb_usd);
    let timestamp = chrono::Utc::now().timestamp();

    let trace_key =
        calculer_trace_key(source_cadenas, timestamp, recipient_uq, amount_points, 0, 0);

    conn.execute(
        "INSERT INTO borb_origins 
         (trace_key, timestamp, cadenas_owner, recipient_uq, amount_points,
          amount_usd, prix_borb_at_emission, source_type, source_uq,
          tresor_before, tresor_after, signature, treasury_uq, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, 0, ?10, ?11, ?2)",
        params![
            &trace_key,
            timestamp,
            source_cadenas,
            recipient_uq,
            amount_points as i64,
            amount_usd,
            prix_borb_usd,
            source_type,
            source_uq,
            signature_hex,
            source_uq,
        ],
    )
    .map_err(|e| e.to_string())?;

    eprintln!(
        "🎁 BORB tracé [{}] : {} USD = {} points → {}",
        source_type, amount_usd, amount_points, recipient_uq
    );

    Ok(trace_key)
}
