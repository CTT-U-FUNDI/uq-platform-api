use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use crate::bubble::BubbleRegistry;
use crate::error::ChainError;

// ═══════════════════════════════════════════════════════════════
// CONSTANTES
// ═══════════════════════════════════════════════════════════════

/// Nombre maximum de transactions hors ligne en attente
pub const MAX_PENDING_OFFLINE_TX: usize = 100;

/// Durée de validité d'une transaction hors ligne (7 jours)
pub const OFFLINE_TX_VALIDITY: u64 = 7 * 24 * 3600;

/// Nombre de confirmations vTEE requises pour valider
pub const VTEE_CONFIRMATIONS: u8 = 3;

// ═══════════════════════════════════════════════════════════════
// STRUCTURES
// ═══════════════════════════════════════════════════════════════

/// Transaction signée hors ligne
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfflineTransaction {
    /// ID unique
    pub id: [u8; 32],
    /// Expéditeur
    pub from: [u8; 32],
    /// Destinataire
    pub to: [u8; 32],
    /// Montant (micro-BORB)
    pub amount_micro: u64,
    /// Nonce (évite la double dépense)
    pub nonce: u64,
    /// Timestamp de création
    pub created_at: u64,
    /// Signature de l'expéditeur
    pub signature: Vec<u8>,
    /// Statut
    pub status: OfflineTxStatus,
    /// Confirmations vTEE reçues
    pub vtee_confirmations: Vec<[u8; 32]>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum OfflineTxStatus {
    /// En attente de connexion
    Pending,
    /// Vérifié par vTEE
    VteeVerified { verified_at: u64 },
    /// Appliqué (fond reçu)
    Applied { applied_at: u64 },
    /// Rejeté
    Rejected { reason: String, rejected_at: u64 },
    /// Expiré
    Expired { expired_at: u64 },
}

/// Demande de réception hors ligne
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfflineReceiveRequest {
    /// ID unique
    pub id: [u8; 32],
    /// Expéditeur (celui qui envoie)
    pub sender: [u8; 32],
    /// Destinataire (celui qui reçoit hors ligne)
    pub recipient: [u8; 32],
    /// Montant (micro-BORB)
    pub amount_micro: u64,
    /// Timestamp
    pub created_at: u64,
    /// Signature de l'expéditeur
    pub signature: Vec<u8>,
    /// Statut
    pub status: OfflineRxStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum OfflineRxStatus {
    /// En attente (le destinataire n'est pas encore connecté)
    Pending,
    /// Reçu (le destinataire s'est connecté)
    Received { received_at: u64 },
    /// Expiré
    Expired,
}

/// Statistiques du mode hors ligne
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfflineStats {
    /// Transactions en attente
    pub pending_tx: usize,
    /// Transactions appliquées
    pub applied_tx: usize,
    /// Demandes de réception en attente
    pub pending_rx: usize,
    /// Total reçu hors ligne (BORB)
    pub total_received_offline: u64,
}

// ═══════════════════════════════════════════════════════════════
// IMPLÉMENTATION
// ═══════════════════════════════════════════════════════════════

pub struct OfflineEngine {
    /// Transactions hors ligne en attente
    pub pending_transactions: HashMap<[u8; 32], OfflineTransaction>,
    /// Demandes de réception hors ligne
    pub receive_requests: HashMap<[u8; 32], OfflineReceiveRequest>,
    /// Total reçu hors ligne (micro-BORB)
    pub total_received_micro: u64,
    /// Compteur de transactions
    pub tx_count: u64,
}

impl OfflineEngine {
    /// Crée un nouveau moteur hors ligne
    pub fn new() -> Self {
        OfflineEngine {
            pending_transactions: HashMap::new(),
            receive_requests: HashMap::new(),
            total_received_micro: 0,
            tx_count: 0,
        }
    }

    /// Crée une transaction signée hors ligne
    pub fn create_offline_transaction(
        &mut self,
        from: &[u8; 32],
        to: &[u8; 32],
        amount_micro: u64,
        nonce: u64,
        signature: &[u8],
    ) -> Result<OfflineTransaction, ChainError> {
        // Vérifier les limites
        if self.pending_transactions.len() >= MAX_PENDING_OFFLINE_TX {
            return Err(ChainError::InternalError("File d'attente pleine".into()));
        }

        if amount_micro == 0 {
            return Err(ChainError::InternalError("Montant nul".into()));
        }

        if from == to {
            return Err(ChainError::InternalError("Expéditeur = destinataire".into()));
        }

        let now = Self::current_timestamp();
        let tx = OfflineTransaction {
            id: Self::generate_id(from, nonce, now),
            from: *from,
            to: *to,
            amount_micro,
            nonce,
            created_at: now,
            signature: signature.to_vec(),
            status: OfflineTxStatus::Pending,
            vtee_confirmations: Vec::new(),
        };

        self.pending_transactions.insert(tx.id, tx.clone());
        self.tx_count += 1;
        Ok(tx)
    }

    /// Vérifie une transaction avec vTEE (simulé)
    pub fn verify_with_vtee(
        &mut self,
        tx_id: &[u8; 32],
        vtee_node: &[u8; 32],
        is_valid: bool,
    ) -> Result<bool, ChainError> {
        let tx = self.pending_transactions.get_mut(tx_id)
            .ok_or_else(|| ChainError::InternalError("Transaction introuvable".into()))?;

        if tx.status != OfflineTxStatus::Pending {
            return Err(ChainError::InternalError("Transaction déjà traitée".into()));
        }

        if is_valid {
            // Ajouter la confirmation
            if !tx.vtee_confirmations.iter().any(|c| c == vtee_node) {
                tx.vtee_confirmations.push(*vtee_node);
            }

            // Si assez de confirmations, marquer comme vérifiée
            if tx.vtee_confirmations.len() >= VTEE_CONFIRMATIONS as usize {
                tx.status = OfflineTxStatus::VteeVerified {
                    verified_at: Self::current_timestamp(),
                };
                return Ok(true);
            }
        } else {
            tx.status = OfflineTxStatus::Rejected {
                reason: "vTEE: signature invalide".into(),
                rejected_at: Self::current_timestamp(),
            };
        }

        Ok(false)
    }

    /// Applique une transaction vérifiée (crédite le destinataire)
    pub fn apply_transaction(
        &mut self,
        registry: &mut BubbleRegistry,
        tx_id: &[u8; 32],
    ) -> Result<(), ChainError> {
        let tx = self.pending_transactions.get_mut(tx_id)
            .ok_or_else(|| ChainError::InternalError("Transaction introuvable".into()))?;

        if !matches!(tx.status, OfflineTxStatus::VteeVerified { .. }) {
            return Err(ChainError::InternalError("Transaction non vérifiée".into()));
        }

        // Vérifier le solde
        let balance = registry.get_balance(&tx.from);
        if balance < tx.amount_micro {
            tx.status = OfflineTxStatus::Rejected {
                reason: "Fonds insuffisants".into(),
                rejected_at: Self::current_timestamp(),
            };
            return Err(ChainError::InsufficientFunds("Fonds insuffisants".into()));
        }

        // Appliquer (dans un système réel, on mettrait à jour les bulles)
        tx.status = OfflineTxStatus::Applied {
            applied_at: Self::current_timestamp(),
        };

        Ok(())
    }

    /// Crée une demande de réception hors ligne
    pub fn create_receive_request(
        &mut self,
        sender: &[u8; 32],
        recipient: &[u8; 32],
        amount_micro: u64,
        signature: &[u8],
    ) -> Result<OfflineReceiveRequest, ChainError> {
        if amount_micro == 0 {
            return Err(ChainError::InternalError("Montant nul".into()));
        }

        let now = Self::current_timestamp();
        let request = OfflineReceiveRequest {
            id: Self::generate_id(sender, amount_micro, now),
            sender: *sender,
            recipient: *recipient,
            amount_micro,
            created_at: now,
            signature: signature.to_vec(),
            status: OfflineRxStatus::Pending,
        };

        self.receive_requests.insert(request.id, request.clone());
        Ok(request)
    }

    /// Marque une demande comme reçue (quand le destinataire se connecte)
    pub fn mark_received(
        &mut self,
        request_id: &[u8; 32],
    ) -> Result<u64, ChainError> {
        let request = self.receive_requests.get_mut(request_id)
            .ok_or_else(|| ChainError::InternalError("Demande introuvable".into()))?;

        if request.status != OfflineRxStatus::Pending {
            return Err(ChainError::InternalError("Demande déjà traitée".into()));
        }

        request.status = OfflineRxStatus::Received {
            received_at: Self::current_timestamp(),
        };
        self.total_received_micro += request.amount_micro;

        Ok(request.amount_micro)
    }

    /// Nettoie les transactions expirées
    pub fn cleanup_expired(&mut self) -> u64 {
        let now = Self::current_timestamp();
        let mut cleaned = 0u64;

        // Nettoyer les transactions
        let expired_tx: Vec<[u8; 32]> = self.pending_transactions
            .iter()
            .filter(|(_, tx)| {
                tx.status == OfflineTxStatus::Pending &&
                now - tx.created_at > OFFLINE_TX_VALIDITY
            })
            .map(|(id, _)| *id)
            .collect();

        for id in &expired_tx {
            if let Some(tx) = self.pending_transactions.get_mut(id) {
                tx.status = OfflineTxStatus::Expired {
                    expired_at: now,
                };
                cleaned += 1;
            }
        }

        // Nettoyer les demandes de réception
        let expired_rx: Vec<[u8; 32]> = self.receive_requests
            .iter()
            .filter(|(_, req)| {
                req.status == OfflineRxStatus::Pending &&
                now - req.created_at > OFFLINE_TX_VALIDITY
            })
            .map(|(id, _)| *id)
            .collect();

        for id in &expired_rx {
            if let Some(req) = self.receive_requests.get_mut(id) {
                req.status = OfflineRxStatus::Expired;
                cleaned += 1;
            }
        }

        cleaned
    }

    /// Retourne les demandes en attente pour un destinataire
    pub fn get_pending_for_recipient(
        &self,
        recipient: &[u8; 32],
    ) -> Vec<&OfflineReceiveRequest> {
        self.receive_requests
            .values()
            .filter(|r| r.recipient == *recipient && r.status == OfflineRxStatus::Pending)
            .collect()
    }

    /// Retourne le total en attente pour un destinataire
    pub fn get_pending_total(&self, recipient: &[u8; 32]) -> u64 {
        self.get_pending_for_recipient(recipient)
            .iter()
            .map(|r| r.amount_micro)
            .sum()
    }

    /// Retourne les statistiques
    pub fn get_stats(&self) -> OfflineStats {
        let pending_tx = self.pending_transactions
            .values()
            .filter(|tx| tx.status == OfflineTxStatus::Pending)
            .count();

        let applied_tx = self.pending_transactions
            .values()
            .filter(|tx| matches!(tx.status, OfflineTxStatus::Applied { .. }))
            .count();

        let pending_rx = self.receive_requests
            .values()
            .filter(|r| r.status == OfflineRxStatus::Pending)
            .count();

        OfflineStats {
            pending_tx,
            applied_tx,
            pending_rx,
            total_received_offline: self.total_received_micro / 1_000_000,
        }
    }

    fn generate_id(data: &[u8; 32], nonce: u64, timestamp: u64) -> [u8; 32] {
        use sha2::{Sha256, Digest};
        let mut hasher = Sha256::new();
        hasher.update(data);
        hasher.update(&nonce.to_le_bytes());
        hasher.update(&timestamp.to_le_bytes());
        let result = hasher.finalize();
        let mut id = [0u8; 32];
        id.copy_from_slice(&result);
        id
    }

    fn current_timestamp() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }
}

// ═══════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
    
mod tests {
    fn test_key() -> UqBlackKey { let mut k = UqBlackKey::generate(); k.activate(); k }
    use uq_black_key_standalone::UqBlackKey;

    use super::*;

    #[test]
    fn test_new_offline_engine() {
        let engine = OfflineEngine::new();
        assert_eq!(engine.pending_transactions.len(), 0);
        assert_eq!(engine.total_received_micro, 0);
    }

    #[test]
    fn test_create_offline_transaction() {
        let mut engine = OfflineEngine::new();
        let alice = [1u8; 32];
        let bob = [2u8; 32];
        let key = test_key(); let sig = key.sign(b"test");

        let tx = engine.create_offline_transaction(&alice, &bob, 1_000_000, 0, &sig).unwrap();
        assert_eq!(tx.from, alice);
        assert_eq!(tx.to, bob);
        assert_eq!(tx.amount_micro, 1_000_000);
        assert_eq!(tx.status, OfflineTxStatus::Pending);
    }

    #[test]
    fn test_self_transfer_rejected() {
        let mut engine = OfflineEngine::new();
        let alice = [1u8; 32];
        let key = test_key(); let sig = key.sign(b"test");

        assert!(engine.create_offline_transaction(&alice, &alice, 100, 0, &sig).is_err());
    }

    #[test]
    fn test_zero_amount_rejected() {
        let mut engine = OfflineEngine::new();
        let alice = [1u8; 32];
        let bob = [2u8; 32];
        let key = test_key(); let sig = key.sign(b"test");

        assert!(engine.create_offline_transaction(&alice, &bob, 0, 0, &sig).is_err());
    }

    #[test]
    fn test_vtee_verification() {
        let mut engine = OfflineEngine::new();
        let alice = [1u8; 32];
        let bob = [2u8; 32];
        let key = test_key(); let sig = key.sign(b"test");

        let tx = engine.create_offline_transaction(&alice, &bob, 500_000, 0, &sig).unwrap();
        let tx_id = tx.id;

        // Ajouter 3 confirmations vTEE
        let verified = engine.verify_with_vtee(&tx_id, &[10u8; 32], true).unwrap();
        assert!(!verified); // Pas encore 3

        engine.verify_with_vtee(&tx_id, &[11u8; 32], true).unwrap();
        let verified = engine.verify_with_vtee(&tx_id, &[12u8; 32], true).unwrap();
        assert!(verified); // 3 confirmations !

        let tx = engine.pending_transactions.get(&tx_id).unwrap();
        assert!(matches!(tx.status, OfflineTxStatus::VteeVerified { .. }));
    }

    #[test]
    fn test_vtee_rejection() {
        let mut engine = OfflineEngine::new();
        let alice = [1u8; 32];
        let bob = [2u8; 32];
        let key = test_key(); let sig = key.sign(b"test");

        let tx = engine.create_offline_transaction(&alice, &bob, 500_000, 0, &sig).unwrap();
        engine.verify_with_vtee(&tx.id, &[10u8; 32], false).unwrap();

        let tx = engine.pending_transactions.get(&tx.id).unwrap();
        assert!(matches!(tx.status, OfflineTxStatus::Rejected { .. }));
    }

    #[test]
    fn test_cleanup_expired() {
        let mut engine = OfflineEngine::new();
        let alice = [1u8; 32];
        let bob = [2u8; 32];
        let key = test_key(); let sig = key.sign(b"test");

        let tx = engine.create_offline_transaction(&alice, &bob, 100_000, 0, &sig).unwrap();
        // Forcer l'expiration
        if let Some(t) = engine.pending_transactions.get_mut(&tx.id) {
            t.created_at = 0; // Très vieux
        }

        let cleaned = engine.cleanup_expired();
        assert!(cleaned > 0);
    }

    #[test]
    fn test_get_pending_total() {
        let mut engine = OfflineEngine::new();
        let sender = [1u8; 32];
        let recipient = [5u8; 32];
        let key = test_key(); let sig = key.sign(b"test");

        engine.create_receive_request(&sender, &recipient, 1_000_000, &sig).unwrap();
        engine.create_receive_request(&sender, &recipient, 500_000, &sig).unwrap();

        let total = engine.get_pending_total(&recipient);
        assert_eq!(total, 1_500_000);
    }
}


