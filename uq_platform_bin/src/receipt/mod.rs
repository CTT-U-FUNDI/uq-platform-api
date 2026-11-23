// ====================
// MODULE RECEIPT — Reçus PNG signés
// ====================
//
// Génère des reçus PNG infalsifiables avec :
// - Signature Ed25519 embarquée (chunks PNG)
// - QR Code vers /verify/{hash}
// - Métadonnées signées
//
// ====================

pub mod embed;
pub mod generator;
pub mod qr;
pub mod routes;
pub mod treasury_key;
pub mod verifier;

use serde::{Deserialize, Serialize};

/// Données d'une transaction pour le reçu
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiptData {
    pub reference: String,
    pub tx_hash: String,
    pub sender_uq: String,
    pub sender_display: String,
    pub sender_cadenas: String,
    pub sender_ip: String,
    pub recipient_uq: String,
    pub recipient_display: String,
    pub amount_borb: f64,
    /// Contre-valeur USD au prix réellement réglé par le régulateur.
    /// `None` quand aucun prix n'est réglé : le reçu n'affiche alors aucune
    /// équivalence en dollars plutôt qu'un « 1 BORB = 1 USD » inventé.
    pub amount_usd: Option<f64>,
    pub fee_borb: f64,
    pub taux_usd: String,
    pub device: String,
    pub os: String,
    pub browser: String,
    pub created_at: i64,
    pub signature: String,
    pub public_key: String,
    pub status: String,
}

/// Reçu vérifié
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifiedReceipt {
    pub valid: bool,
    pub data: Option<ReceiptData>,
    pub error: Option<String>,
    pub verified_at: i64,
}
