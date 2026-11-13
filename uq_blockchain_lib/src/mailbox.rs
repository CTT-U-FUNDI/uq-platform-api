// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// UQ CORE - mailbox.rs
// Messages offline - BoÃ®te aux lettres distribuÃ©e
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

use serde::{Serialize, Deserialize};
use sha2::{Sha256, Digest};
use crate::error::ChainError;
use std::collections::HashMap;

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// STRUCTURES
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mailbox {
    pub owner: [u8; 32],
    pub messages: HashMap<[u8; 32], StoredMessage>,
    pub max_messages: u32,
    pub max_message_size: u32,
    pub total_stored: u64,
    pub created_at: u64,
    pub last_cleanup: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredMessage {
    pub id: [u8; 32],
    pub from: [u8; 32],
    pub to: [u8; 32],
    pub encrypted_content: Vec<u8>,
    pub received_at: u64,
    pub expires_at: u64,
    pub delivered: bool,
    pub delivery_attempts: u32,
    pub priority: MessagePriority,
    pub size_bytes: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MessagePriority { Low, Normal, High, Urgent }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MailboxStats {
    pub total_messages: u32,
    pub unread_count: u32,
    pub oldest_message_age: u64,
    pub storage_used_bytes: u64,
    pub delivery_success_rate: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeliveryReceipt {
    pub message_id: [u8; 32],
    pub delivered_at: u64,
    pub delivered_to: [u8; 32],
    pub signature: Vec<u8>,
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// CONSTANTES
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

const DEFAULT_MAX_MESSAGES: u32 = 1000;
const DEFAULT_MAX_MESSAGE_SIZE: u32 = 1_048_576; // 1 Mo
const DEFAULT_MESSAGE_TTL: u64 = 604_800; // 7 jours
const MAX_DELIVERY_ATTEMPTS: u32 = 10;

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// HELPERS
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

fn get_hardbit() -> u64 { crate::hardbit::get_current_hardbit() }
fn sha256_hash(data: &[u8]) -> [u8; 32] { let mut h = Sha256::new(); h.update(data); let mut r = [0u8; 32]; r.copy_from_slice(&h.finalize()); r }

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// F1. CRÃ‰ER UNE MAILBOX
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

pub fn create_mailbox(owner: &[u8; 32]) -> Result<Mailbox, ChainError> {
    let hb = get_hardbit();
    Ok(Mailbox {
        owner: *owner,
        messages: HashMap::new(),
        max_messages: DEFAULT_MAX_MESSAGES,
        max_message_size: DEFAULT_MAX_MESSAGE_SIZE,
        total_stored: 0,
        created_at: hb,
        last_cleanup: hb,
    })
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// F2. STOCKER UN MESSAGE
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

pub fn store_message(
    mailbox: &mut Mailbox,
    from: &[u8; 32],
    to: &[u8; 32],
    encrypted_content: &[u8],
    priority: MessagePriority,
) -> Result<[u8; 32], ChainError> {
    let hb = get_hardbit();

    if mailbox.messages.len() >= mailbox.max_messages as usize {
        return Err(ChainError::InternalError("Mailbox pleine".into()));
    }

    if encrypted_content.len() > mailbox.max_message_size as usize {
        return Err(ChainError::InternalError("Message trop volumineux".into()));
    }

    let ttl = match priority {
        MessagePriority::Urgent => DEFAULT_MESSAGE_TTL * 2,
        MessagePriority::High => DEFAULT_MESSAGE_TTL,
        MessagePriority::Normal => DEFAULT_MESSAGE_TTL / 2,
        MessagePriority::Low => DEFAULT_MESSAGE_TTL / 4,
    };

    let id = sha256_hash(&[from, to, encrypted_content, &hb.to_le_bytes()].concat());

    let msg = StoredMessage {
        id,
        from: *from,
        to: *to,
        encrypted_content: encrypted_content.to_vec(),
        received_at: hb,
        expires_at: hb + ttl,
        delivered: false,
        delivery_attempts: 0,
        priority,
        size_bytes: encrypted_content.len() as u32,
    };

    mailbox.messages.insert(id, msg);
    mailbox.total_stored += 1;

    Ok(id)
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// F3. RÃ‰CUPÃ‰RER LES MESSAGES
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

pub fn get_messages<'a>(mailbox: &'a Mailbox) -> Result<Vec<&'a StoredMessage>, ChainError> {
    let mut msgs: Vec<&StoredMessage> = mailbox.messages.values()
        .filter(|m| !m.delivered)
        .collect();
    msgs.sort_by_key(|m| match m.priority {
        MessagePriority::Urgent => 0,
        MessagePriority::High => 1,
        MessagePriority::Normal => 2,
        MessagePriority::Low => 3,
    });
    Ok(msgs)
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// F4. MARQUER COMME LIVRÃ‰
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

pub fn mark_delivered(
    mailbox: &mut Mailbox,
    message_id: &[u8; 32],
) -> Result<DeliveryReceipt, ChainError> {
    let hb = get_hardbit();

    if let Some(msg) = mailbox.messages.get_mut(message_id) {
        msg.delivered = true;
        
        Ok(DeliveryReceipt {
            message_id: *message_id,
            delivered_at: hb,
            delivered_to: msg.to,
            signature: Vec::new(),
        })
    } else {
        Err(ChainError::InternalError("Message introuvable".into()))
    }
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// F5. SUPPRIMER UN MESSAGE
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

pub fn delete_message(mailbox: &mut Mailbox, message_id: &[u8; 32]) -> Result<(), ChainError> {
    if mailbox.messages.remove(message_id).is_some() {
        Ok(())
    } else {
        Err(ChainError::InternalError("Message introuvable".into()))
    }
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// F6. NETTOYER LES MESSAGES EXPIRÃ‰S
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

pub fn cleanup_expired(mailbox: &mut Mailbox) -> Result<u32, ChainError> {
    let hb = get_hardbit();
    let expired: Vec<[u8; 32]> = mailbox.messages.iter()
        .filter(|(_, m)| hb >= m.expires_at)
        .map(|(id, _)| *id)
        .collect();

    let count = expired.len() as u32;
    for id in &expired {
        mailbox.messages.remove(id);
    }

    mailbox.last_cleanup = hb;
    Ok(count)
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// F7. STATISTIQUES
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

pub fn get_stats(mailbox: &Mailbox) -> Result<MailboxStats, ChainError> {
    let _hb = get_hardbit();
    let total = mailbox.messages.len() as u32;
    let unread = mailbox.messages.values().filter(|m| !m.delivered).count() as u32;
    let delivered = mailbox.messages.values().filter(|m| m.delivered).count() as u32;
    let storage: u64 = mailbox.messages.values().map(|m| m.size_bytes as u64).sum();

    Ok(MailboxStats {
        total_messages: total,
        unread_count: unread,
        oldest_message_age: 0,
        storage_used_bytes: storage,
        delivery_success_rate: if total > 0 { delivered as f64 / total as f64 * 100.0 } else { 100.0 },
    })
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// F8. RÃ‰ESSAYER LES LIVRAISONS
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

pub fn retry_delivery(mailbox: &mut Mailbox, message_id: &[u8; 32]) -> Result<bool, ChainError> {
    if let Some(msg) = mailbox.messages.get_mut(message_id) {
        if msg.delivered { return Ok(true); }
        if msg.delivery_attempts >= MAX_DELIVERY_ATTEMPTS {
            return Ok(false);
        }
        msg.delivery_attempts += 1;
        Ok(true)
    } else {
        Err(ChainError::InternalError("Message introuvable".into()))
    }
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// F9. VIDER LA MAILBOX
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

pub fn clear_mailbox(mailbox: &mut Mailbox) -> Result<u32, ChainError> {
    let count = mailbox.messages.len() as u32;
    mailbox.messages.clear();
    mailbox.total_stored = 0;
    Ok(count)
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// F10. RECHERCHER PAR EXPÃ‰DITEUR
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

pub fn search_by_sender<'a>(mailbox: &'a Mailbox, sender: &[u8; 32]) -> Vec<&'a StoredMessage> {
    mailbox.messages.values()
        .filter(|m| m.from == *sender)
        .collect()
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// FONCTIONS SUPPLÃ‰MENTAIRES
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

pub fn message_count(mailbox: &Mailbox) -> u32 { mailbox.messages.len() as u32 }
pub fn unread_count(mailbox: &Mailbox) -> u32 { mailbox.messages.values().filter(|m| !m.delivered).count() as u32 }
pub fn is_full(mailbox: &Mailbox) -> bool { mailbox.messages.len() >= mailbox.max_messages as usize }
pub fn get_urgent_messages<'a>(mailbox: &'a Mailbox) -> Vec<&'a StoredMessage> { mailbox.messages.values().filter(|m| m.priority == MessagePriority::Urgent && !m.delivered).collect() }

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// TESTS
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

#[cfg(test)]
mod tests {
    use super::*;

    fn create_mb() -> Mailbox { create_mailbox(&[1u8; 32]).unwrap_or_default() }

    #[test] fn test_create() { let mb = create_mb(); assert_eq!(mb.owner, [1u8; 32]); assert_eq!(mb.messages.len(), 0); }
    #[test] fn test_store() { let mut mb = create_mb(); let id = store_message(&mut mb, &[2u8; 32], &[1u8; 32], b"test", MessagePriority::Normal).unwrap_or_default(); assert_eq!(mb.messages.len(), 1); assert!(mb.messages.contains_key(&id)); }
    #[test] fn test_get_messages() { let mut mb = create_mb(); store_message(&mut mb, &[2u8; 32], &[1u8; 32], b"msg1", MessagePriority::Normal).unwrap_or_default(); store_message(&mut mb, &[3u8; 32], &[1u8; 32], b"msg2", MessagePriority::Urgent).unwrap_or_default(); let msgs = get_messages(&mb).unwrap_or_default(); assert_eq!(msgs.len(), 2); }
    #[test] fn test_mark_delivered() { let mut mb = create_mb(); let id = store_message(&mut mb, &[2u8; 32], &[1u8; 32], b"test", MessagePriority::Normal).unwrap_or_default(); let receipt = mark_delivered(&mut mb, &id).unwrap_or_default(); assert_eq!(receipt.message_id, id); }
    #[test] fn test_delete() { let mut mb = create_mb(); let id = store_message(&mut mb, &[2u8; 32], &[1u8; 32], b"test", MessagePriority::Normal).unwrap_or_default(); assert!(delete_message(&mut mb, &id).is_ok()); assert_eq!(mb.messages.len(), 0); }
    #[test] fn test_cleanup() { let mut mb = create_mb(); let id = store_message(&mut mb, &[2u8; 32], &[1u8; 32], b"test", MessagePriority::Normal).unwrap_or_default(); if let Some(m) = mb.messages.get_mut(&id) { m.expires_at = 0; } let cleaned = cleanup_expired(&mut mb).unwrap_or_default(); assert_eq!(cleaned, 1); assert!(mb.messages.is_empty()); }
    #[test] fn test_stats() { let mut mb = create_mb(); store_message(&mut mb, &[2u8; 32], &[1u8; 32], b"test", MessagePriority::Normal).unwrap_or_default(); let s = get_stats(&mb).unwrap_or_default(); assert_eq!(s.total_messages, 1); assert_eq!(s.unread_count, 1); }
    #[test] fn test_retry() { let mut mb = create_mb(); let id = store_message(&mut mb, &[2u8; 32], &[1u8; 32], b"test", MessagePriority::Normal).unwrap_or_default(); assert!(retry_delivery(&mut mb, &id).unwrap_or_default()); }
    #[test] fn test_clear() { let mut mb = create_mb(); store_message(&mut mb, &[2u8; 32], &[1u8; 32], b"t1", MessagePriority::Normal).unwrap_or_default(); store_message(&mut mb, &[3u8; 32], &[1u8; 32], b"t2", MessagePriority::Normal).unwrap_or_default(); assert_eq!(clear_mailbox(&mut mb).unwrap_or_default(), 2); assert_eq!(mb.messages.len(), 0); }
    #[test] fn test_search_sender() { let mut mb = create_mb(); store_message(&mut mb, &[2u8; 32], &[1u8; 32], b"t1", MessagePriority::Normal).unwrap_or_default(); store_message(&mut mb, &[3u8; 32], &[1u8; 32], b"t2", MessagePriority::Normal).unwrap_or_default(); assert_eq!(search_by_sender(&mb, &[2u8; 32]).len(), 1); }
    #[test] fn test_is_full() { let mut mb = create_mb(); mb.max_messages = 2; store_message(&mut mb, &[2u8; 32], &[1u8; 32], b"a", MessagePriority::Normal).unwrap_or_default(); store_message(&mut mb, &[3u8; 32], &[1u8; 32], b"b", MessagePriority::Normal).unwrap_or_default(); assert!(is_full(&mb)); }
    #[test] fn test_urgent() { let mut mb = create_mb(); store_message(&mut mb, &[2u8; 32], &[1u8; 32], b"u", MessagePriority::Urgent).unwrap_or_default(); store_message(&mut mb, &[3u8; 32], &[1u8; 32], b"n", MessagePriority::Normal).unwrap_or_default(); assert_eq!(get_urgent_messages(&mb).len(), 1); }
}









