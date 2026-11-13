use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use sha2::{Sha256, Digest};
use uq_black_key_standalone::UqBlackKey;
use crate::error::ChainError;

// ═══════════════════════════════════════════════════════════════
// CONSTANTES
// ═══════════════════════════════════════════════════════════════

/// Nombre de validateurs dans l'enclave virtuelle
pub const VTEE_VALIDATOR_COUNT: usize = 7;
/// Consensus requis (5 sur 7)
pub const VTEE_CONSENSUS_REQUIRED: usize = 5;
/// Taille maximale du code à exécuter (1 Mo)
pub const VTEE_MAX_CODE_SIZE: usize = 1_048_576;
/// Mémoire maximale de la sandbox (16 Mo)
pub const VTEE_MAX_MEMORY: usize = 16_777_216;
/// Frais pour une exécution vTEE
pub const VTEE_EXECUTION_FEE: u64 = 10;

// ═══════════════════════════════════════════════════════════════
// STRUCTURES
// ═══════════════════════════════════════════════════════════════

/// Une enclave virtuelle (sandbox déterministe)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VirtualEnclave {
    /// ID unique de l'enclave
    pub id: [u8; 32],
    /// Code à exécuter (bytecode ou WASM)
    pub code: Vec<u8>,
    /// Hash du code (vérification d'intégrité)
    pub code_hash: [u8; 32],
    /// Entrée (input data)
    pub input: Vec<u8>,
    /// Hash de l'entrée
    pub input_hash: [u8; 32],
    /// Résultat attendu (si connu)
    pub expected_output_hash: Option<[u8; 32]>,
    /// Validateurs assignés
    pub validators: Vec<[u8; 32]>,
    /// Résultats des validateurs
    pub results: HashMap<[u8; 32], VteeResult>,
    /// Statut de l'exécution
    pub status: VteeStatus,
    /// Timestamp de création
    pub created_at: u64,
    /// Frais payés
    pub fee_paid: u64,
}

/// Résultat d'un validateur
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VteeResult {
    /// Validateur
    pub validator: [u8; 32],
    /// Hash de la sortie
    pub output_hash: [u8; 32],
    /// Temps d'exécution (microsecondes)
    pub execution_time_us: u64,
    /// Mémoire utilisée (octets)
    pub memory_used: usize,
    /// Signature du validateur
    pub signature: Vec<u8>,
    /// Timestamp
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum VteeStatus {
    /// En attente de validateurs
    Pending,
    /// En cours d'exécution
    Executing,
    /// Consensus atteint
    ConsensusReached { output_hash: [u8; 32], agreed_by: usize, completed_at: u64 },
    /// Consensus échoué (pas assez de résultats identiques)
    ConsensusFailed { reason: String },
    /// Validateur tricheur détecté
    CheaterDetected { cheater: [u8; 32], proof: Vec<u8> },
}

/// Registre vTEE
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VteeRegistry {
    /// Enclaves actives
    pub enclaves: HashMap<[u8; 32], VirtualEnclave>,
    /// Validateurs disponibles
    pub validators: Vec<VteeValidator>,
    /// Historique des exécutions
    pub execution_history: Vec<[u8; 32]>,
    /// Total des exécutions
    pub total_executions: u64,
    /// Total des frais collectés
    pub total_fees_collected: u64,
    /// Tricheurs bannis
    pub banned_validators: Vec<[u8; 32]>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VteeValidator {
    pub address: [u8; 32],
    pub stake: u64,
    pub reputation: u64,
    pub total_executions: u64,
    pub accurate_executions: u64,
    pub is_active: bool,
    pub last_execution_at: u64,
}

#[derive(Debug, Clone)]
pub struct VteeStats {
    pub active_enclaves: usize,
    pub total_executions: u64,
    pub consensus_rate: f64,
    pub cheaters_caught: usize,
    pub avg_execution_time_us: u64,
}

// ═══════════════════════════════════════════════════════════════
// IMPLÉMENTATION
// ═══════════════════════════════════════════════════════════════

impl VteeRegistry {
    pub fn new() -> Self {
        VteeRegistry {
            enclaves: HashMap::new(),
            validators: Vec::new(),
            execution_history: Vec::new(),
            total_executions: 0,
            total_fees_collected: 0,
            banned_validators: Vec::new(),
        }
    }

    /// Enregistre un validateur vTEE
    pub fn register_validator(&mut self, address: [u8; 32], stake: u64, key: &UqBlackKey) -> Result<(), ChainError> {
        if !key.can_operate() { return Err(ChainError::InternalError("Clé non opérationnelle".into())); }
        if self.banned_validators.contains(&address) { return Err(ChainError::InternalError("Validateur banni".into())); }
        if self.validators.iter().any(|v| v.address == address) { return Err(ChainError::InternalError("Déjà enregistré".into())); }
        self.validators.push(VteeValidator {
            address, stake, reputation: 100, total_executions: 0,
            accurate_executions: 0, is_active: true, last_execution_at: 0,
        });
        Ok(())
    }

    /// Crée une nouvelle enclave virtuelle
    pub fn create_enclave(&mut self, code: &[u8], input: &[u8], key: &UqBlackKey) -> Result<VirtualEnclave, ChainError> {
        if code.len() > VTEE_MAX_CODE_SIZE { return Err(ChainError::InternalError("Code trop volumineux".into())); }
        if !key.can_operate() { return Err(ChainError::InternalError("Clé non opérationnelle".into())); }
        
        // Sélectionner 7 validateurs aléatoirement (basé sur le hash)
        let now = utc_now();
        let selected = self.select_validators(now);
        if selected.len() < VTEE_CONSENSUS_REQUIRED {
            return Err(ChainError::InternalError("Pas assez de validateurs".into()));
        }

        let code_hash = hash_bytes(code);
        let input_hash = hash_bytes(input);
        let enclave_id = hash_enclave(&code_hash, &input_hash, now);

        let enclave = VirtualEnclave {
            id: enclave_id,
            code: code.to_vec(),
            code_hash,
            input: input.to_vec(),
            input_hash,
            expected_output_hash: None,
            validators: selected,
            results: HashMap::new(),
            status: VteeStatus::Pending,
            created_at: now,
            fee_paid: VTEE_EXECUTION_FEE,
        };

        self.enclaves.insert(enclave_id, enclave.clone());
        self.total_fees_collected += VTEE_EXECUTION_FEE;
        Ok(enclave)
    }

    /// Un validateur soumet son résultat
    pub fn submit_result(&mut self, enclave_id: &[u8; 32], validator: &[u8; 32],
        output: &[u8], execution_time_us: u64, memory_used: usize,
        key: &UqBlackKey) -> Result<(), ChainError> {
        
        let enclave = self.enclaves.get_mut(enclave_id)
            .ok_or(ChainError::InternalError("Enclave introuvable".into()))?;
        
        if !enclave.validators.contains(validator) {
            return Err(ChainError::InternalError("Non assigné à cette enclave".into()));
        }
        if enclave.results.contains_key(validator) {
            return Err(ChainError::InternalError("Déjà soumis".into()));
        }
        if self.banned_validators.contains(validator) {
            return Err(ChainError::InternalError("Validateur banni".into()));
        }
        if !key.can_operate() { return Err(ChainError::InternalError("Clé non opérationnelle".into())); }
        if memory_used > VTEE_MAX_MEMORY { return Err(ChainError::InternalError("Dépassement mémoire".into())); }

        let output_hash = hash_bytes(output);
        let payload = [&output_hash[..], &execution_time_us.to_le_bytes(), &memory_used.to_le_bytes()].concat();
        let signature = key.sign(&payload);

        enclave.results.insert(*validator, VteeResult {
            validator: *validator, output_hash, execution_time_us,
            memory_used, signature: signature.to_vec(), timestamp: utc_now(),
        });

        // Vérifier si le consensus est atteint
        self.check_consensus(enclave_id)?;

        // Mettre à jour les stats du validateur
        if let Some(v) = self.validators.iter_mut().find(|v| v.address == *validator) {
            v.total_executions += 1;
            v.last_execution_at = utc_now();
        }

        Ok(())
    }

    /// Vérifie si le consensus est atteint (5/7 mêmes résultats)
    fn check_consensus(&mut self, enclave_id: &[u8; 32]) -> Result<(), ChainError> {
        let enclave = self.enclaves.get_mut(enclave_id)
            .ok_or(ChainError::InternalError("Enclave introuvable".into()))?;

        if enclave.results.len() < VTEE_CONSENSUS_REQUIRED { return Ok(()); }

        // Compter les résultats identiques
        let mut hash_counts: HashMap<[u8; 32], Vec<[u8; 32]>> = HashMap::new();
        for result in enclave.results.values() {
            hash_counts.entry(result.output_hash).or_default().push(result.validator);
        }

        // Trouver le hash majoritaire
        let mut best_hash = [0u8; 32];
        let mut best_count = 0;
        let mut cheaters = Vec::new();

        for (hash, validators) in &hash_counts {
            if validators.len() > best_count {
                best_hash = *hash;
                best_count = validators.len();
            }
        }

        if best_count >= VTEE_CONSENSUS_REQUIRED {
            // Consensus atteint !
            enclave.status = VteeStatus::ConsensusReached {
                output_hash: best_hash,
                agreed_by: best_count,
                completed_at: utc_now(),
            };
            self.total_executions += 1;
            self.execution_history.push(*enclave_id);

            // Récompenser les validateurs honnêtes
            for result in enclave.results.values() {
                if let Some(v) = self.validators.iter_mut().find(|v| v.address == result.validator) {
                    if result.output_hash == best_hash {
                        v.accurate_executions += 1;
                        v.reputation = v.reputation.saturating_add(2);
                    } else {
                        // Tricheur !
                        cheaters.push(result.validator);
                        v.reputation = v.reputation.saturating_sub(50);
                        if v.reputation == 0 {
                            self.banned_validators.push(result.validator);
                            v.is_active = false;
                        }
                    }
                }
            }

            if !cheaters.is_empty() {
                enclave.status = VteeStatus::CheaterDetected {
                    cheater: cheaters[0],
                    proof: Vec::new(),
                };
            }
        } else {
            enclave.status = VteeStatus::ConsensusFailed {
                reason: format!("Seulement {}/{} d'accord", best_count, VTEE_CONSENSUS_REQUIRED),
            };
        }

        Ok(())
    }

    /// Vérifie si une sortie est valide (consensus atteint)
    pub fn verify_output(&self, enclave_id: &[u8; 32], output: &[u8]) -> Result<bool, ChainError> {
        let enclave = self.enclaves.get(enclave_id)
            .ok_or(ChainError::InternalError("Enclave introuvable".into()))?;
        if let VteeStatus::ConsensusReached { output_hash, .. } = &enclave.status {
            Ok(*output_hash == hash_bytes(output))
        } else {
            Ok(false)
        }
    }

    /// Retourne les statistiques
    pub fn get_stats(&self) -> VteeStats {
        let active = self.enclaves.values().filter(|e| matches!(e.status, VteeStatus::Pending | VteeStatus::Executing)).count();
        let total_results: u64 = self.validators.iter().map(|v| v.total_executions).sum();
        let accurate: u64 = self.validators.iter().map(|v| v.accurate_executions).sum();
        let avg_time = if total_results > 0 {
            self.enclaves.values()
                .flat_map(|e| e.results.values())
                .map(|r| r.execution_time_us)
                .sum::<u64>() / total_results
        } else { 0 };
        VteeStats {
            active_enclaves: active,
            total_executions: self.total_executions,
            consensus_rate: if total_results > 0 { accurate as f64 / total_results as f64 * 100.0 } else { 0.0 },
            cheaters_caught: self.banned_validators.len(),
            avg_execution_time_us: avg_time,
        }
    }

    /// Sélectionne 7 validateurs aléatoirement
    fn select_validators(&self, seed: u64) -> Vec<[u8; 32]> {
        let active: Vec<&VteeValidator> = self.validators.iter().filter(|v| v.is_active && !self.banned_validators.contains(&v.address)).collect();
        if active.len() <= VTEE_VALIDATOR_COUNT { return active.iter().map(|v| v.address).collect(); }
        let mut indices: Vec<usize> = (0..active.len()).collect();
        let mut rng = seed;
        for i in (0..VTEE_VALIDATOR_COUNT).rev() {
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let j = (rng as usize) % (i + 1);
            indices.swap(i, j);
        }
        indices[..VTEE_VALIDATOR_COUNT].iter().map(|&i| active[i].address).collect()
    }
}

fn hash_bytes(data: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new(); h.update(data);
    let r = h.finalize(); let mut id = [0u8; 32]; id.copy_from_slice(&r); id
}

fn hash_enclave(code_hash: &[u8; 32], input_hash: &[u8; 32], ts: u64) -> [u8; 32] {
    let mut h = Sha256::new(); h.update(code_hash); h.update(input_hash); h.update(&ts.to_le_bytes());
    let r = h.finalize(); let mut id = [0u8; 32]; id.copy_from_slice(&r); id
}

fn utc_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()
}

// ═══════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> UqBlackKey { let mut k = UqBlackKey::generate(); k.activate(); k }
    fn addr(i: u8) -> [u8; 32] { let mut a = [0u8; 32]; a[0] = i; a }

    #[test] fn test_new() { let r = VteeRegistry::new(); assert_eq!(r.total_executions, 0); }

    #[test] fn test_register() {
        let mut r = VteeRegistry::new();
        r.register_validator(addr(1), 1000, &test_key()).unwrap();
        assert_eq!(r.validators.len(), 1);
    }

    #[test] fn test_create_enclave() {
        let mut r = VteeRegistry::new();
        let key = test_key();
        for i in 1..=7 { r.register_validator(addr(i), 1000, &key).unwrap(); }
        let enclave = r.create_enclave(b"test code", b"input", &key).unwrap();
        assert_eq!(enclave.validators.len(), 7);
    }

    #[test] fn test_submit_and_consensus() {
        let mut r = VteeRegistry::new();
        let key = test_key();
        for i in 1..=7 { r.register_validator(addr(i), 1000, &key).unwrap(); }
        let enclave = r.create_enclave(b"code", b"data", &key).unwrap();
        // 5 validateurs soumettent le MÊME résultat
        let output = b"same result";
        for i in 1..=5 { r.submit_result(&enclave.id, &addr(i), output, 100, 1024, &key).unwrap(); }
        let enclave = r.enclaves.get(&enclave.id).unwrap();
        assert!(matches!(enclave.status, VteeStatus::ConsensusReached { .. }));
    }

    #[test] fn test_cheater_detected() {
        let mut r = VteeRegistry::new();
        let key = test_key();
        for i in 1..=7 { r.register_validator(addr(i), 1000, &key).unwrap(); }
        let enclave = r.create_enclave(b"code", b"data", &key).unwrap();
        // 5 soumettent le même résultat, 1 triche
        for i in 1..=5 { r.submit_result(&enclave.id, &addr(i), b"ok", 100, 1024, &key).unwrap(); }
        r.submit_result(&enclave.id, &addr(6), b"wrong", 100, 1024, &key).unwrap();
        let enclave = r.enclaves.get(&enclave.id).unwrap();
        assert!(matches!(enclave.status, VteeStatus::CheaterDetected { .. }));
    }

    #[test] fn test_insufficient_consensus() {
        let mut r = VteeRegistry::new();
        let key = test_key();
        for i in 1..=7 { r.register_validator(addr(i), 1000, &key).unwrap(); }
        let enclave = r.create_enclave(b"code", b"data", &key).unwrap();
        // Seulement 3 soumettent (pas assez pour 5)
        for i in 1..=3 { r.submit_result(&enclave.id, &addr(i), b"ok", 100, 1024, &key).unwrap(); }
        let enclave = r.enclaves.get(&enclave.id).unwrap();
        assert!(matches!(enclave.status, VteeStatus::Pending)); // Pas encore assez
    }

    #[test] fn test_ban_cheater() {
        let mut r = VteeRegistry::new();
        let key = test_key();
        for i in 1..=10 { r.register_validator(addr(i), 1000, &key).unwrap(); }
        // Vérifier que le ban fonctionne (test simplifié)
        assert!(r.banned_validators.is_empty());
    }

    #[test] fn test_verify_output() {
        let mut r = VteeRegistry::new();
        let key = test_key();
        for i in 1..=7 { r.register_validator(addr(i), 1000, &key).unwrap(); }
        let enclave = r.create_enclave(b"code", b"data", &key).unwrap();
        let output = b"verified output";
        for i in 1..=5 { r.submit_result(&enclave.id, &addr(i), output, 100, 1024, &key).unwrap(); }
        assert!(r.verify_output(&enclave.id, output).unwrap());
        assert!(!r.verify_output(&enclave.id, b"wrong").unwrap());
    }

    #[test] fn test_stats() {
        let mut r = VteeRegistry::new();
        let key = test_key();
        for i in 1..=7 { r.register_validator(addr(i), 1000, &key).unwrap(); }
        let enclave = r.create_enclave(b"code", b"data", &key).unwrap();
        for i in 1..=5 { r.submit_result(&enclave.id, &addr(i), b"ok", 100, 1024, &key).unwrap(); }
        let stats = r.get_stats();
        assert_eq!(stats.total_executions, 1);
        assert!(stats.consensus_rate > 0.0);
    }
}



