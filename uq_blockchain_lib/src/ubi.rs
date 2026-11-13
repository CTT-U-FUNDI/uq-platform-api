use serde::{Serialize, Deserialize};
// use sha2::Digest;
use crate::error::ChainError;
use std::collections::HashMap;

// ═══════════════════════════════════════════════════════════════
// REVENU UNIVERSEL (UBI) - UQ CHAIN
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UbiPool {
    pub claims: HashMap<[u8; 32], UbiClaim>,
    pub total_distributed: u64,
    pub daily_amount: u64,
    pub claim_cooldown_hours: i64,
    pub min_account_age_days: u64,
    pub kyc_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UbiClaim {
    pub address: [u8; 32],
    pub last_claim: i64,
    pub total_claimed: u64,
    pub claim_count: u64,
    pub streak_days: u64,
    pub multiplier: u64,
}

impl UbiPool {
    /// Crée un nouveau pool UBI
    pub fn new(daily_amount: u64) -> Self {
        UbiPool {
            claims: HashMap::new(),
            total_distributed: 0,
            daily_amount,
            claim_cooldown_hours: 23,    // ~1 jour
            min_account_age_days: 7,       // Compte ≥ 7 jours
            kyc_required: true,
        }
    }

    /// Enregistre une nouvelle adresse dans le pool UBI
    pub fn register(&mut self, address: [u8; 32]) -> Result<(), ChainError> {
        if self.claims.contains_key(&address) {
            return Err(ChainError::InternalError("Adresse déjà enregistrée".into()));
        }

        self.claims.insert(address, UbiClaim {
            address,
            last_claim: 0,
            total_claimed: 0,
            claim_count: 0,
            streak_days: 0,
            multiplier: 100,  // 1.00x au départ
        });

        Ok(())
    }

    /// Réclame le revenu universel quotidien
    pub fn claim_ubi(
        &mut self,
        address: &[u8; 32],
        account_age_days: u64,
        is_kyc_verified: bool,
        current_time: i64,
    ) -> Result<u64, ChainError> {
        // Vérifier l'éligibilité
        if self.kyc_required && !is_kyc_verified {
            return Err(ChainError::InternalError("KYC requis pour l'UBI".into()));
        }

        if account_age_days < self.min_account_age_days {
            return Err(ChainError::InternalError(format!(
                "Compte trop récent : {} jours (min: {})",
                account_age_days, self.min_account_age_days
            )));
        }

        let claim = self.claims.get_mut(address)
            .ok_or_else(|| ChainError::InternalError("Adresse non enregistrée dans l'UBI".into()))?;

        // Vérifier le cooldown
        let hours_since_last = (current_time - claim.last_claim) / 3600;
        if claim.last_claim > 0 && hours_since_last < self.claim_cooldown_hours {
            let remaining = self.claim_cooldown_hours - hours_since_last;
            return Err(ChainError::InternalError(format!(
                "Cooldown actif : {}h restantes", remaining
            )));
        }

        // Vérifier le streak
        if claim.last_claim > 0 && hours_since_last < 48 {
            claim.streak_days = claim.streak_days.saturating_add(1);
        } else if hours_since_last >= 48 {
            claim.streak_days = 1;  // Reset du streak
        } else {
            claim.streak_days = 1;  // Première fois
        }

        // Calculer le multiplicateur de streak
        claim.multiplier = Self::calculate_multiplier(claim.streak_days);

        // Calculer le montant
        let base_amount = self.daily_amount;
        let streak_bonus = base_amount * (claim.multiplier.saturating_sub(100)) / 100;
        let total_amount = base_amount.saturating_add(streak_bonus);

        // Mettre à jour le claim
        claim.last_claim = current_time;
        claim.total_claimed = claim.total_claimed.saturating_add(total_amount);
        claim.claim_count = claim.claim_count.saturating_add(1);

        self.total_distributed = self.total_distributed.saturating_add(total_amount);

        Ok(total_amount)
    }

    /// Calcule le multiplicateur basé sur le streak
    fn calculate_multiplier(streak_days: u64) -> u64 {
        match streak_days {
            0..=6   => 100,  // 1.00x
            7..=29  => 110,  // 1.10x (+10% après 1 semaine)
            30..=89 => 125,  // 1.25x (+25% après 1 mois)
            90..=179=> 150,  // 1.50x (+50% après 3 mois)
            180..=364=>175,  // 1.75x (+75% après 6 mois)
            _       => 200,  // 2.00x (+100% après 1 an)
        }
    }

    /// Récupère les informations de claim d'une adresse
    pub fn get_claim_info(&self, address: &[u8; 32]) -> Option<&UbiClaim> {
        self.claims.get(address)
    }

    /// Vérifie si une adresse peut claim aujourd'hui
    pub fn can_claim(
        &self,
        address: &[u8; 32],
        current_time: i64,
        account_age_days: u64,
        is_kyc_verified: bool,
    ) -> Result<bool, ChainError> {
        if self.kyc_required && !is_kyc_verified {
            return Ok(false);
        }

        if account_age_days < self.min_account_age_days {
            return Ok(false);
        }

        if let Some(claim) = self.claims.get(address) {
            if claim.last_claim == 0 {
                return Ok(true);  // Jamais claim, peut claim
            }
            let hours_since = (current_time - claim.last_claim) / 3600;
            Ok(hours_since >= self.claim_cooldown_hours)
        } else {
            Ok(false)  // Pas enregistré
        }
    }

    /// Retourne le temps restant avant le prochain claim
    pub fn time_until_next_claim(&self, address: &[u8; 32], current_time: i64) -> Option<i64> {
        self.claims.get(address).map(|claim| {
            if claim.last_claim == 0 {
                return 0;
            }
            let next_claim = claim.last_claim + (self.claim_cooldown_hours * 3600);
            let remaining = next_claim - current_time;
            if remaining < 0 { 0 } else { remaining }
        })
    }

    /// Retourne les statistiques globales de l'UBI
    pub fn get_stats(&self) -> UbiStats {
        let total_users = self.claims.len() as u64;
        let active_streaks = self.claims.values()
            .filter(|c| c.streak_days >= 7)
            .count() as u64;

        UbiStats {
            total_users,
            total_distributed: self.total_distributed,
            daily_amount: self.daily_amount,
            active_streaks,
            average_streak: if total_users > 0 {
                self.claims.values().map(|c| c.streak_days).sum::<u64>() / total_users
            } else {
                0
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UbiStats {
    pub total_users: u64,
    pub total_distributed: u64,
    pub daily_amount: u64,
    pub active_streaks: u64,
    pub average_streak: u64,
}

// ═══════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    fn pool() -> UbiPool {
        let mut p = UbiPool::new(10_000_000); // 0.01 BORB par jour
        p.register([1u8; 32]).unwrap_or_default();
        p.register([2u8; 32]).unwrap_or_default();
        p.register([3u8; 32]).unwrap_or_default();
        p
    }

    #[test]
    fn test_register() {
        let p = pool();
        assert_eq!(p.claims.len(), 3);
    }

    #[test]
    fn test_first_claim() {
        let mut p = pool();
        let amount = p.claim_ubi(&[1u8; 32], 30, true, 100000).unwrap_or_default();
        assert_eq!(amount, 10_000_000);
        assert_eq!(p.total_distributed, 10_000_000);
    }

    #[test]
    fn test_cooldown() {
        let mut p = pool();
        p.claim_ubi(&[1u8; 32], 30, true, 100000).unwrap_or_default();
        // 1 heure après → trop tôt
        let result = p.claim_ubi(&[1u8; 32], 30, true, 103600);
        assert!(result.is_err());
    }

    #[test]
    fn test_cooldown_passed() {
        let mut p = pool();
        p.claim_ubi(&[1u8; 32], 30, true, 100000).unwrap_or_default();
        // 24 heures après → OK
        let result = p.claim_ubi(&[1u8; 32], 30, true, 186400);
        assert!(result.is_ok());
    }

    #[test]
    fn test_kyc_required() {
        let mut p = pool();
        let result = p.claim_ubi(&[1u8; 32], 30, false, 100000);
        assert!(result.is_err());
    }

    #[test]
    fn test_account_too_young() {
        let mut p = pool();
        let result = p.claim_ubi(&[1u8; 32], 3, true, 100000);  // 3 jours < 7
        assert!(result.is_err());
    }

    #[test]
    fn test_streak_multiplier() {
        let mut p = pool();
        let now = 100000;

        // Jour 1
        let a1 = p.claim_ubi(&[1u8; 32], 30, true, now).unwrap_or_default();
        assert_eq!(a1, 10_000_000);

        // Simuler 7 jours de streak
        for i in 1..=6 {
            p.claim_ubi(&[1u8; 32], 30, true, now + (i * 86400)).unwrap_or_default();
        }

        // Jour 7 → streak bonus +10%
        let a7 = p.claim_ubi(&[1u8; 32], 30, true, now + (7 * 86400)).unwrap_or_default();
        assert!(a7 > 10_000_000); // Avec bonus streak
    }

    #[test]
    fn test_can_claim() {
        let p = pool();
        assert!(p.can_claim(&[1u8; 32], 100000, 30, true).unwrap_or_default());
        assert!(!p.can_claim(&[1u8; 32], 100000, 3, true).unwrap_or_default()); // Trop jeune
        assert!(!p.can_claim(&[1u8; 32], 100000, 30, false).unwrap_or_default()); // Pas KYC
    }

    #[test]
    fn test_stats() {
        let mut p = pool();
        p.claim_ubi(&[1u8; 32], 30, true, 100000).unwrap_or_default();
        let stats = p.get_stats();
        assert_eq!(stats.total_users, 3);
        assert!(stats.total_distributed > 0);
    }
}





