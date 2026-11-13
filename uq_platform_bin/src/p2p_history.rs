// ====================
// 📡 P2P HISTORY — Messages d'échange de fragments d'historique
// ====================
//
// SPÉCIFICATION (§6) :
//
//   Demande :
//   {
//     "type": "history_request",
//     "from": "UQ-732688531",
//     "lost_uq": "UQ-230472511",
//     "date_range": "2026-01-01 to 2026-12-31",
//     "signature": "..."
//   }
//
//   Réponse :
//   {
//     "type": "history_fragment",
//     "from": "UQ-732688531",
//     "lost_uq": "UQ-230472511",
//     "transactions": [ { "borb_id": "...", "prev_hash": "...", "amount": 5 } ],
//     "signature": "..."
//   }
//
// Ce module ne fait AUCUNE I/O : il fabrique, sérialise et valide les
// messages. Le transport (HTTP aujourd'hui, UDP/tel perdu demain) est
// branché dans history_recovery.rs et les routes /api/history/*.
// ====================

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const TYPE_REQUEST: &str = "history_request";
pub const TYPE_FRAGMENT: &str = "history_fragment";
pub const TYPE_VALIDATION: &str = "history_validation";

// ====================
// DEMANDE DE RÉCUPÉRATION
// ====================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryRequest {
    #[serde(rename = "type")]
    pub msg_type: String,
    /// Compte qui a perdu son historique (et qui demande)
    pub from: String,
    /// UQ public du compte dont l'historique est perdu
    pub lost_uq: String,
    /// « 2026-01-01 to 2026-12-31 » (format lisible de la spécification)
    pub date_range: String,
    pub signature: String,
}

impl HistoryRequest {
    pub fn new(from: &str, lost_uq: &str, date_from: i64, date_to: i64, signature: &str) -> Self {
        Self {
            msg_type: TYPE_REQUEST.to_string(),
            from: from.to_string(),
            lost_uq: lost_uq.to_string(),
            date_range: format!("{} to {}", date_lisible(date_from), date_lisible(date_to)),
            signature: signature.to_string(),
        }
    }

    /// Message canonique à signer
    pub fn message_a_signer(&self) -> String {
        format!(
            "UQ_HISTORY_REQUEST_V1:{}:{}:{}",
            self.from, self.lost_uq, self.date_range
        )
    }

    pub fn valide(&self) -> bool {
        self.msg_type == TYPE_REQUEST
            && !self.from.is_empty()
            && !self.lost_uq.is_empty()
            && !self.signature.is_empty()
    }

    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|e| e.to_string())
    }

    pub fn from_json(s: &str) -> Result<Self, String> {
        serde_json::from_str(s).map_err(|e| e.to_string())
    }
}

// ====================
// FRAGMENT FOURNI PAR UN VOISIN
// ====================

/// Une ligne d'historique telle qu'un voisin peut l'attester.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FragmentTx {
    pub borb_id: String,
    pub prev_hash: String,
    pub sender_uq: String,
    pub recipient_uq: String,
    pub amount: i64,
    pub fee: i64,
    pub timestamp: i64,
    pub block_number: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryFragment {
    #[serde(rename = "type")]
    pub msg_type: String,
    /// Le voisin qui atteste
    pub from: String,
    /// Le compte dont l'historique est perdu
    pub lost_uq: String,
    pub transactions: Vec<FragmentTx>,
    pub signature: String,
}

impl HistoryFragment {
    pub fn new(from: &str, lost_uq: &str, transactions: Vec<FragmentTx>, signature: &str) -> Self {
        Self {
            msg_type: TYPE_FRAGMENT.to_string(),
            from: from.to_string(),
            lost_uq: lost_uq.to_string(),
            transactions,
            signature: signature.to_string(),
        }
    }

    /// Message canonique : lie le fournisseur, le bénéficiaire et CHAQUE
    /// transaction → un fragment ne peut pas être rejoué pour un autre compte
    /// ni modifié sans invalider la signature.
    pub fn message_a_signer(&self) -> String {
        let mut msg = format!("UQ_HISTORY_FRAGMENT_V1:{}:{}", self.from, self.lost_uq);
        for tx in &self.transactions {
            msg.push_str(&format!(
                ":{}|{}|{}|{}|{}|{}|{}",
                tx.borb_id,
                tx.prev_hash,
                tx.sender_uq,
                tx.recipient_uq,
                tx.amount,
                tx.fee,
                tx.timestamp
            ));
        }
        msg
    }

    pub fn valide(&self) -> bool {
        self.msg_type == TYPE_FRAGMENT
            && !self.from.is_empty()
            && !self.lost_uq.is_empty()
            && !self.signature.is_empty()
            && !self.transactions.is_empty()
    }

    /// Le voisin doit être partie prenante : il a envoyé ou reçu.
    pub fn concerne(&self, tx: &FragmentTx) -> bool {
        tx.sender_uq == self.from || tx.recipient_uq == self.from
    }

    /// Vérifie que le fragment ne contient que des transactions où le
    /// fournisseur est impliqué, et que le compte perdu l'est aussi.
    pub fn coherent(&self) -> bool {
        self.transactions.iter().all(|tx| {
            self.concerne(tx)
                && (tx.sender_uq == self.lost_uq || tx.recipient_uq == self.lost_uq)
                && tx.borb_id.len() == 64
        })
    }

    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|e| e.to_string())
    }

    pub fn from_json(s: &str) -> Result<Self, String> {
        serde_json::from_str(s).map_err(|e| e.to_string())
    }
}

// ====================
// VALIDATION PAR UN VOISIN
// ====================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryValidation {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub from: String,
    pub lost_uq: String,
    pub request_id: String,
    pub approved: bool,
    pub signature: String,
}

impl HistoryValidation {
    pub fn new(from: &str, lost_uq: &str, request_id: &str, approved: bool, signature: &str) -> Self {
        Self {
            msg_type: TYPE_VALIDATION.to_string(),
            from: from.to_string(),
            lost_uq: lost_uq.to_string(),
            request_id: request_id.to_string(),
            approved,
            signature: signature.to_string(),
        }
    }

    pub fn message_a_signer(&self) -> String {
        format!(
            "UQ_HISTORY_VALIDATION_V1:{}:{}:{}:{}",
            self.from, self.lost_uq, self.request_id, self.approved
        )
    }

    pub fn valide(&self) -> bool {
        self.msg_type == TYPE_VALIDATION
            && !self.from.is_empty()
            && !self.request_id.is_empty()
            && !self.signature.is_empty()
    }

    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|e| e.to_string())
    }

    pub fn from_json(s: &str) -> Result<Self, String> {
        serde_json::from_str(s).map_err(|e| e.to_string())
    }
}

// ====================
// OUTILS
// ====================

/// Signature locale (HMAC-like) quand le client n'a pas de clé Ed25519.
/// Le serveur ne fait que vérifier la cohérence ; la vraie signature
/// Ed25519 est produite par le téléphone (champ `signature` du client).
pub fn signature_locale(secret: &str, message: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(secret.as_bytes());
    hasher.update(b"|");
    hasher.update(message.as_bytes());
    hex::encode(hasher.finalize())
}

/// « 2026-01-01 » à partir d'un timestamp (format de la spécification)
pub fn date_lisible(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "1970-01-01".to_string())
}

/// Convertit « 2026-01-01 to 2026-12-31 » en (from, to)
pub fn parser_date_range(range: &str) -> Option<(i64, i64)> {
    let (a, b) = range.split_once(" to ")?;
    let from = chrono::NaiveDate::parse_from_str(a.trim(), "%Y-%m-%d").ok()?;
    let to = chrono::NaiveDate::parse_from_str(b.trim(), "%Y-%m-%d").ok()?;
    Some((
        from.and_hms_opt(0, 0, 0)?.and_utc().timestamp(),
        to.and_hms_opt(23, 59, 59)?.and_utc().timestamp(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tx(from: &str, to: &str) -> FragmentTx {
        FragmentTx {
            borb_id: "a".repeat(64),
            prev_hash: String::new(),
            sender_uq: from.to_string(),
            recipient_uq: to.to_string(),
            amount: 500,
            fee: 5,
            timestamp: 1_800_000_000,
            block_number: 3,
        }
    }

    #[test]
    fn message_request_contient_le_type_attendu() {
        let r = HistoryRequest::new("UQ-1", "UQ-2", 1_767_225_600, 1_798_761_599, "sig");
        let json = r.to_json().unwrap();
        assert!(json.contains("\"type\":\"history_request\""));
        assert!(json.contains("\"lost_uq\":\"UQ-2\""));
        assert!(json.contains("2026-01-01"));
        assert!(r.valide());
        assert_eq!(HistoryRequest::from_json(&json).unwrap().from, "UQ-1");
    }

    #[test]
    fn fragment_refuse_une_transaction_tierce() {
        // Le voisin UQ-9 atteste une transaction entre UQ-1 et UQ-2 :
        // il n'est pas partie prenante → incohérent.
        let frag = HistoryFragment::new("UQ-9", "UQ-1", vec![tx("UQ-1", "UQ-2")], "sig");
        assert!(!frag.coherent());

        // Un vrai voisin (UQ-2) est partie prenante → cohérent
        let frag2 = HistoryFragment::new("UQ-2", "UQ-1", vec![tx("UQ-1", "UQ-2")], "sig");
        assert!(frag2.coherent());
        assert!(frag2.valide());
    }

    #[test]
    fn date_range_aller_retour() {
        let (from, to) = parser_date_range("2026-01-01 to 2026-12-31").unwrap();
        assert_eq!(date_lisible(from), "2026-01-01");
        assert_eq!(date_lisible(to), "2026-12-31");
        assert!(parser_date_range("n'importe quoi").is_none());
    }

    #[test]
    fn signature_change_avec_le_contenu() {
        let a = signature_locale("secret", "message");
        let b = signature_locale("secret", "message2");
        assert_ne!(a, b);
        assert_eq!(a.len(), 64);
    }
}
