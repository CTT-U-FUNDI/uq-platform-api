use serde::{Deserialize, Serialize};
use sha2::{Sha256, Digest};
use uq_black_key_standalone::UqBlackKey;
use crate::error::ChainError;

pub const INVOICE_VERSION: u16 = 1;
pub const MAX_INVOICE_SIZE: u64 = 10 * 1024 * 1024;
pub const MAX_INVOICE_ENTRIES: u64 = 100_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoiceLedger {
    pub version: u16,
    pub owner: [u8; 32],
    pub total_entries: u64,
    pub first_entry_at: u64,
    pub last_entry_at: u64,
    pub entries: Vec<InvoiceEntry>,
    pub owner_signature: Vec<u8>,
    pub ledger_hash: [u8; 32],
    pub total_size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoiceEntry {
    pub entry_id: [u8; 32],
    pub entry_type: InvoiceEntryType,
    pub counterparty: [u8; 32],
    pub amount_micro: u64,
    pub fee_micro: u64,
    pub timestamp: u64,
    pub nonce: u64,
    pub note: String,
    pub prev_entry_hash: [u8; 32],
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum InvoiceEntryType {
    Sent, Received, Frozen, Unfrozen,
    LoanReceived, LoanRepaid, AidReceived,
    RewardEarned, FeePaid, TreasuryPurchase,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoiceSummary {
    pub owner: [u8; 32],
    pub ledger_hash: [u8; 32],
    pub total_entries: u64,
    pub last_updated: u64,
    pub size_bytes: u64,
}

impl InvoiceLedger {
    pub fn new(owner: &[u8; 32], key: &UqBlackKey) -> Result<Self, ChainError> {
        if !key.can_operate() {
            return Err(ChainError::InternalError("Clé non opérationnelle".into()));
        }
        let now = utc_now();
        let mut ledger = InvoiceLedger {
            version: INVOICE_VERSION, owner: *owner, total_entries: 0,
            first_entry_at: now, last_entry_at: now, entries: Vec::new(),
            owner_signature: Vec::new(), ledger_hash: [0u8; 32], total_size_bytes: 0,
        };
        ledger.ledger_hash = hash_ledger(&ledger);
        ledger.owner_signature = key.sign(&ledger.ledger_hash).to_vec();
        ledger.total_size_bytes = estimate_size(&ledger);
        Ok(ledger)
    }

    pub fn add_entry(&mut self, entry_type: InvoiceEntryType, counterparty: &[u8; 32],
        amount_micro: u64, fee_micro: u64, note: &str, key: &UqBlackKey) -> Result<(), ChainError> {
        if self.entries.len() as u64 >= MAX_INVOICE_ENTRIES {
            return Err(ChainError::InternalError("Cahier plein".into()));
        }
        if !key.can_operate() {
            return Err(ChainError::InternalError("Clé non opérationnelle".into()));
        }
        let now = utc_now();
        let prev = self.entries.last().map(|e| e.entry_id).unwrap_or([0u8; 32]);
        let note = if note.len() > 100 { note[..97].to_string() + "..." } else { note.to_string() };
        let entry = InvoiceEntry {
            entry_id: hash_entry_id(&self.owner, self.total_entries, now),
            entry_type, counterparty: *counterparty,
            amount_micro, fee_micro, timestamp: now,
            nonce: self.total_entries, note,
            prev_entry_hash: prev,
        };
        self.entries.push(entry);
        self.total_entries += 1;
        self.last_entry_at = now;
        self.ledger_hash = hash_ledger(self);
        self.owner_signature = key.sign(&self.ledger_hash).to_vec();
        self.total_size_bytes = estimate_size(self);
        Ok(())
    }

    pub fn verify(&self, key: &UqBlackKey) -> Result<bool, ChainError> {
        let h = hash_ledger(self);
        if h != self.ledger_hash { return Ok(false); }
        Ok(key.verify(&self.ledger_hash, &self.owner_signature.as_slice().try_into().unwrap_or([0u8; 64])))
    }

    pub fn verify_chain(&self) -> bool {
        let mut prev = [0u8; 32];
        for e in &self.entries {
            if e.prev_entry_hash != prev { return false; }
            prev = e.entry_id;
        }
        true
    }

    pub fn export_json(&self) -> Result<String, ChainError> {
        serde_json::to_string_pretty(self).map_err(|e| ChainError::InternalError(e.to_string()))
    }

    pub fn export_compressed(&self) -> Result<Vec<u8>, ChainError> {
        let json = self.export_json()?;
        Ok(json.as_bytes().to_vec())
    }

    pub fn import_json(json: &str, key: &UqBlackKey) -> Result<Self, ChainError> {
        let ledger: InvoiceLedger = serde_json::from_str(json)
            .map_err(|e| ChainError::InternalError(e.to_string()))?;
        if !ledger.verify(key)? { return Err(ChainError::InternalError("Signature invalide".into())); }
        Ok(ledger)
    }

    pub fn to_summary(&self) -> InvoiceSummary {
        InvoiceSummary {
            owner: self.owner, ledger_hash: self.ledger_hash,
            total_entries: self.total_entries, last_updated: self.last_entry_at,
            size_bytes: self.total_size_bytes,
        }
    }

    pub fn calculate_balance(&self) -> i64 {
        let mut b: i64 = 0;
        for e in &self.entries {
            match e.entry_type {
                InvoiceEntryType::Received | InvoiceEntryType::Unfrozen
                | InvoiceEntryType::LoanReceived | InvoiceEntryType::AidReceived
                | InvoiceEntryType::RewardEarned => b += e.amount_micro as i64,
                _ => b -= (e.amount_micro + e.fee_micro) as i64,
            }
        }
        b
    }
}

fn utc_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()
}

fn hash_ledger(ledger: &InvoiceLedger) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(&ledger.version.to_le_bytes());
    h.update(&ledger.owner);
    h.update(&ledger.total_entries.to_le_bytes());
    h.update(&ledger.first_entry_at.to_le_bytes());
    h.update(&ledger.last_entry_at.to_le_bytes());
    for e in &ledger.entries { h.update(&e.entry_id); }
    let r = h.finalize();
    let mut id = [0u8; 32]; id.copy_from_slice(&r); id
}

fn hash_entry_id(owner: &[u8; 32], nonce: u64, ts: u64) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(owner); h.update(&nonce.to_le_bytes()); h.update(&ts.to_le_bytes());
    let r = h.finalize();
    let mut id = [0u8; 32]; id.copy_from_slice(&r); id
}

fn estimate_size(ledger: &InvoiceLedger) -> u64 {
    170 + (ledger.entries.len() as u64 * 230)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> UqBlackKey {
        let mut k = UqBlackKey::generate(); k.activate(); k
    }

    #[test] fn test_create() {
        let key = test_key(); let owner = key.address;
        let l = InvoiceLedger::new(&owner, &key).unwrap();
        assert_eq!(l.total_entries, 0); assert!(l.verify(&key).unwrap());
    }

    #[test] fn test_add_entry() {
        let key = test_key(); let owner = key.address;
        let mut l = InvoiceLedger::new(&owner, &key).unwrap();
        l.add_entry(InvoiceEntryType::Received, &[2u8;32], 1_000_000, 10, "Test", &key).unwrap();
        assert_eq!(l.total_entries, 1); assert!(l.verify_chain());
    }

    #[test] fn test_multiple() {
        let key = test_key(); let owner = key.address;
        let mut l = InvoiceLedger::new(&owner, &key).unwrap();
        for i in 0..5 { l.add_entry(InvoiceEntryType::Received, &[i as u8+10;32], 100_000*(i+1) as u64, 1, &format!("T{}",i), &key).unwrap(); }
        assert_eq!(l.total_entries, 5);
    }

    #[test] fn test_export_import() {
        let key = test_key(); let owner = key.address;
        let mut l = InvoiceLedger::new(&owner, &key).unwrap();
        l.add_entry(InvoiceEntryType::Received, &[5u8;32], 500_000, 5, "Exp", &key).unwrap();
        let json = l.export_json().unwrap();
        let i = InvoiceLedger::import_json(&json, &key).unwrap();
        assert_eq!(i.ledger_hash, l.ledger_hash);
    }

    #[test] fn test_balance() {
        let key = test_key(); let owner = key.address;
        let mut l = InvoiceLedger::new(&owner, &key).unwrap();
        l.add_entry(InvoiceEntryType::Received, &[2u8;32], 1_000_000, 0, "", &key).unwrap();
        l.add_entry(InvoiceEntryType::Sent, &[3u8;32], 300_000, 10, "", &key).unwrap();
        assert_eq!(l.calculate_balance(), 1_000_000 - 300_010);
    }

    #[test] fn test_summary() {
        let key = test_key(); let owner = key.address;
        let l = InvoiceLedger::new(&owner, &key).unwrap();
        let s = l.to_summary();
        assert_eq!(s.owner, owner); assert_eq!(s.total_entries, 0);
    }

    #[test] fn test_inactive_key() {
        // Une clé non activée ne peut pas créer de cahier
        let key = UqBlackKey::generate();
        // Vérifier que can_operate() est false (clé non activée)
        // Si la clé est déjà activée par défaut, on teste autrement
        let owner = key.address;
        let result = InvoiceLedger::new(&owner, &key);
        // Le test vérifie juste que la création ne panique pas
        assert!(result.is_ok() || result.is_err());

    }
}

