use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use crate::bubble::BubbleRegistry;
use crate::error::ChainError;

// ═══════════════════════════════════════════════════════════════
// CONSTANTES
// ═══════════════════════════════════════════════════════════════

/// Récompense annuelle pour le gel volontaire (2% par an)
pub const FREEZE_REWARD_RATE: u64 = 2;

/// Durée minimale de gel (en secondes) — 30 jours
pub const MIN_FREEZE_DURATION: u64 = 30 * 24 * 3600;

/// Durée maximale de gel (en secondes) — 10 ans
pub const MAX_FREEZE_DURATION: u64 = 10 * 365 * 24 * 3600;

/// Pénalité de dégel anticipé (% du capital)
pub const EARLY_UNFREEZE_PENALTY: u64 = 5;

// ═══════════════════════════════════════════════════════════════
// STRUCTURES
// ═══════════════════════════════════════════════════════════════

/// Un contrat de gel volontaire
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FreezeContract {
    /// ID unique du contrat
    pub id: [u8; 32],
    /// Adresse du propriétaire
    pub owner: [u8; 32],
    /// Bulles gelées
    pub bubble_ids: Vec<[u8; 32]>,
    /// Montant total gelé (en micro-BORB)
    pub total_frozen_micro: u64,
    /// Timestamp de début
    pub started_at: u64,
    /// Timestamp de fin prévue
    pub ends_at: u64,
    /// Récompense estimée (en micro-BORB)
    pub estimated_reward_micro: u64,
    /// Statut du contrat
    pub status: FreezeStatus,
    /// Raison du gel (optionnelle)
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum FreezeStatus {
    /// Actif — les bulles sont gelées
    Active,
    /// Terminé normalement — récompense versée
    Completed { reward_paid: u64, completed_at: u64 },
    /// Dégel anticipé — pénalité appliquée
    EarlyUnfreeze { penalty_paid: u64, unfrozen_at: u64 },
    /// Annulé par le propriétaire
    Cancelled,
}

/// Statistiques de la Case 4
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FreezeStats {
    /// Nombre de contrats actifs
    pub active_contracts: usize,
    /// Nombre total de contrats
    pub total_contracts: usize,
    /// Total gelé (en BORB)
    pub total_frozen_BORB: u64,
    /// Récompenses totales distribuées
    pub total_rewards_paid: u64,
    /// Pénalités totales collectées
    pub total_penalties: u64,
}

// ═══════════════════════════════════════════════════════════════
// IMPLÉMENTATION
// ═══════════════════════════════════════════════════════════════

pub struct FreezeEngine {
    /// Contrats de gel
    pub contracts: HashMap<[u8; 32], FreezeContract>,
    /// Récompenses totales distribuées
    pub total_rewards_paid: u64,
    /// Pénalités totales collectées
    pub total_penalties: u64,
}

impl FreezeEngine {
    /// Crée un nouveau moteur de gel
    pub fn new() -> Self {
        FreezeEngine {
            contracts: HashMap::new(),
            total_rewards_paid: 0,
            total_penalties: 0,
        }
    }

    /// Crée un contrat de gel VOLONTAIRE
    pub fn create_freeze_contract(
        &mut self,
        registry: &mut BubbleRegistry,
        owner: &[u8; 32],
        bubble_ids: &[[u8; 32]],
        duration_seconds: u64,
        reason: &str,
    ) -> Result<FreezeContract, ChainError> {
        // Vérifier la durée
        if duration_seconds < MIN_FREEZE_DURATION {
            return Err(ChainError::InternalError(
                format!("Durée minimale: {} jours", MIN_FREEZE_DURATION / 86400)
            ));
        }
        if duration_seconds > MAX_FREEZE_DURATION {
            return Err(ChainError::InternalError(
                format!("Durée maximale: {} ans", MAX_FREEZE_DURATION / 31536000)
            ));
        }

        // Vérifier que les bulles appartiennent au propriétaire
        let mut total_micro: u64 = 0;
        for bubble_id in bubble_ids {
            let bubble = registry.bubbles.get(bubble_id)
                .ok_or_else(|| ChainError::InternalError("Bulle introuvable".into()))?;
            
            if bubble.owner != *owner {
                return Err(ChainError::InternalError(
                    "Vous ne pouvez geler que vos propres bulles".into()
                ));
            }
            if !bubble.is_active() {
                return Err(ChainError::InternalError(
                    "Seules les bulles actives peuvent être gelées".into()
                ));
            }
            total_micro += bubble.value_micro;
        }

        // Geler les bulles
        for bubble_id in bubble_ids {
            registry.freeze_bubble(bubble_id, reason)
                .map_err(|e| ChainError::InternalError(e))?;
        }

        // Calculer la récompense estimée (2% par an, au prorata)
        let years = duration_seconds as f64 / (365.0 * 24.0 * 3600.0);
        let estimated_reward = (total_micro as f64 * (FREEZE_REWARD_RATE as f64 / 100.0) * years) as u64;

        let now = Self::current_timestamp();
        let contract = FreezeContract {
            id: Self::generate_contract_id(owner, now),
            owner: *owner,
            bubble_ids: bubble_ids.to_vec(),
            total_frozen_micro: total_micro,
            started_at: now,
            ends_at: now + duration_seconds,
            estimated_reward_micro: estimated_reward,
            status: FreezeStatus::Active,
            reason: reason.to_string(),
        };

        self.contracts.insert(contract.id, contract.clone());
        Ok(contract)
    }

    /// Débloque un contrat arrivé à échéance (verse la récompense)
    pub fn complete_contract(
        &mut self,
        registry: &mut BubbleRegistry,
        contract_id: &[u8; 32],
    ) -> Result<u64, ChainError> {
        let contract = self.contracts.get_mut(contract_id)
            .ok_or_else(|| ChainError::InternalError("Contrat introuvable".into()))?;

        if contract.status != FreezeStatus::Active {
            return Err(ChainError::InternalError("Contrat non actif".into()));
        }

        let now = Self::current_timestamp();
        if now < contract.ends_at {
            return Err(ChainError::InternalError(
                format!("Contrat pas encore arrivé à échéance. Fin: {}", contract.ends_at)
            ));
        }

        // Dégeler les bulles
        for bubble_id in &contract.bubble_ids {
            registry.unfreeze_bubble(bubble_id)
                .map_err(|e| ChainError::InternalError(e))?;
        }

        // Verser la récompense
        let reward = contract.estimated_reward_micro;
        self.total_rewards_paid += reward;
        contract.status = FreezeStatus::Completed {
            reward_paid: reward,
            completed_at: now,
        };

        Ok(reward)
    }

    /// Dégel anticipé (avec pénalité de 5%)
    pub fn early_unfreeze(
        &mut self,
        registry: &mut BubbleRegistry,
        contract_id: &[u8; 32],
    ) -> Result<u64, ChainError> {
        let contract = self.contracts.get_mut(contract_id)
            .ok_or_else(|| ChainError::InternalError("Contrat introuvable".into()))?;

        if contract.status != FreezeStatus::Active {
            return Err(ChainError::InternalError("Contrat non actif".into()));
        }

        // Calculer la pénalité (5% du capital gelé)
        let penalty = contract.total_frozen_micro * EARLY_UNFREEZE_PENALTY / 100;
        self.total_penalties += penalty;

        // Dégeler les bulles
        for bubble_id in &contract.bubble_ids {
            registry.unfreeze_bubble(bubble_id)
                .map_err(|e| ChainError::InternalError(e))?;
        }

        let now = Self::current_timestamp();
        contract.status = FreezeStatus::EarlyUnfreeze {
            penalty_paid: penalty,
            unfrozen_at: now,
        };

        Ok(penalty)
    }

    /// Retourne les contrats actifs d'un propriétaire
    pub fn get_active_contracts(&self, owner: &[u8; 32]) -> Vec<&FreezeContract> {
        self.contracts
            .values()
            .filter(|c| c.owner == *owner && c.status == FreezeStatus::Active)
            .collect()
    }

    /// Retourne tous les contrats d'un propriétaire
    pub fn get_all_contracts(&self, owner: &[u8; 32]) -> Vec<&FreezeContract> {
        self.contracts
            .values()
            .filter(|c| c.owner == *owner)
            .collect()
    }

    /// Retourne les statistiques
    pub fn get_stats(&self) -> FreezeStats {
        let active: Vec<&FreezeContract> = self.contracts
            .values()
            .filter(|c| c.status == FreezeStatus::Active)
            .collect();

        FreezeStats {
            active_contracts: active.len(),
            total_contracts: self.contracts.len(),
            total_frozen_BORB: active.iter().map(|c| c.total_frozen_micro / 1_000_000).sum(),
            total_rewards_paid: self.total_rewards_paid,
            total_penalties: self.total_penalties,
        }
    }

    /// Calcule la récompense potentielle pour un montant et une durée
    pub fn calculate_reward(amount_micro: u64, duration_seconds: u64) -> u64 {
        let years = duration_seconds as f64 / (365.0 * 24.0 * 3600.0);
        (amount_micro as f64 * (FREEZE_REWARD_RATE as f64 / 100.0) * years) as u64
    }

    fn generate_contract_id(owner: &[u8; 32], timestamp: u64) -> [u8; 32] {
        use sha2::{Sha256, Digest};
        let mut hasher = Sha256::new();
        hasher.update(owner);
        hasher.update(&timestamp.to_le_bytes());
        let result = hasher.finalize();
        let mut id = [0u8; 32];
        id.copy_from_slice(&result);
        id
    }

    fn current_timestamp() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
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
    use crate::bubble::BubbleRegistry;

    fn sign(data: &[u8]) -> Vec<u8> {
        let key = test_key(); key.sign(data)
    }

    fn create_bubble(reg: &mut BubbleRegistry, owner: [u8; 32], value: u64) -> [u8; 32] {
        reg.create_bubble(owner, owner, value, vec![0u8; 64], sign).unwrap().id
    }

    #[test]
    fn test_create_freeze_contract() {
        let mut reg = BubbleRegistry::new();
        let mut engine = FreezeEngine::new();
        let owner = [1u8; 32];
        let bubble_id = create_bubble(&mut reg, owner, 10_000_000);

        let contract = engine.create_freeze_contract(
            &mut reg, &owner, &[bubble_id], MIN_FREEZE_DURATION, "Épargne"
        ).unwrap();

        assert_eq!(contract.owner, owner);
        assert_eq!(contract.status, FreezeStatus::Active);
        assert!(contract.estimated_reward_micro > 0);
        assert_eq!(engine.contracts.len(), 1);
    }

    #[test]
    fn test_duration_too_short() {
        let mut reg = BubbleRegistry::new();
        let mut engine = FreezeEngine::new();
        let owner = [1u8; 32];
        let bubble_id = create_bubble(&mut reg, owner, 10_000_000);

        assert!(engine.create_freeze_contract(
            &mut reg, &owner, &[bubble_id], 100, "test"
        ).is_err());
    }

    #[test]
    fn test_duration_too_long() {
        let mut reg = BubbleRegistry::new();
        let mut engine = FreezeEngine::new();
        let owner = [1u8; 32];
        let bubble_id = create_bubble(&mut reg, owner, 10_000_000);

        assert!(engine.create_freeze_contract(
            &mut reg, &owner, &[bubble_id], MAX_FREEZE_DURATION + 1, "test"
        ).is_err());
    }

    #[test]
    fn test_not_owner_cannot_freeze() {
        let mut reg = BubbleRegistry::new();
        let mut engine = FreezeEngine::new();
        let alice = [1u8; 32];
        let bob = [2u8; 32];
        let bubble_id = create_bubble(&mut reg, alice, 10_000_000);

        assert!(engine.create_freeze_contract(
            &mut reg, &bob, &[bubble_id], MIN_FREEZE_DURATION, "vol"
        ).is_err());
    }

    #[test]
    fn test_early_unfreeze_penalty() {
        let mut reg = BubbleRegistry::new();
        let mut engine = FreezeEngine::new();
        let owner = [1u8; 32];
        let bubble_id = create_bubble(&mut reg, owner, 10_000_000);

        let contract = engine.create_freeze_contract(
            &mut reg, &owner, &[bubble_id], MIN_FREEZE_DURATION, "test"
        ).unwrap();

        let penalty = engine.early_unfreeze(&mut reg, &contract.id).unwrap();
        assert!(penalty > 0);
        assert_eq!(engine.total_penalties, penalty);
    }

    #[test]
    fn test_get_active_contracts() {
        let mut reg = BubbleRegistry::new();
        let mut engine = FreezeEngine::new();
        let owner1 = [1u8; 32];
        let owner2 = [2u8; 32];
        
        // Créer 2 bulles pour 2 owners différents
        let b1 = create_bubble(&mut reg, owner1, 5_000_000);
        let b2 = create_bubble(&mut reg, owner2, 3_000_000);

        // Créer 2 contrats distincts
        engine.create_freeze_contract(&mut reg, &owner1, &[b1], MIN_FREEZE_DURATION, "c1").unwrap();
        engine.create_freeze_contract(&mut reg, &owner2, &[b2], MIN_FREEZE_DURATION, "c2").unwrap();
        
        // Vérifier le nombre total de contrats
        assert_eq!(engine.contracts.len(), 2, "Devrait avoir 2 contrats au total");
        
        // Vérifier les contrats actifs pour owner1
        let active1 = engine.get_active_contracts(&owner1);
        assert_eq!(active1.len(), 1, "Owner1 devrait avoir 1 contrat");
        
        // Vérifier les contrats actifs pour owner2
        let active2 = engine.get_active_contracts(&owner2);
        assert_eq!(active2.len(), 1, "Owner2 devrait avoir 1 contrat");
    }

    #[test]
    fn test_calculate_reward() {
        let reward = FreezeEngine::calculate_reward(1_000_000, 365 * 24 * 3600); // 1 BORB, 1 an
        assert!(reward > 0);
        assert!(reward <= 1_000_000); // max 100% (impossible avec 2%)
    }

    #[test]
    fn test_get_stats() {
        let mut reg = BubbleRegistry::new();
        let mut engine = FreezeEngine::new();
        let owner = [1u8; 32];
        let b1 = create_bubble(&mut reg, owner, 10_000_000);

        engine.create_freeze_contract(&mut reg, &owner, &[b1], MIN_FREEZE_DURATION, "test").unwrap();

        let stats = engine.get_stats();
        assert_eq!(stats.active_contracts, 1);
        assert_eq!(stats.total_contracts, 1);
        assert_eq!(stats.total_frozen_BORB, 10);
    }
}


