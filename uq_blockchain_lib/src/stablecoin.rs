// ═══════════════════════════════════════════════════════════════
// UQ CHAIN - UQ Stable (UST) - Stablecoin interne
// ═══════════════════════════════════════════════════════════════

use serde::{Serialize, Deserialize};
use sha2::{Sha256, Digest};
use crate::error::ChainError;
use std::collections::HashMap;

/// Coffre de collatéral individuel
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollateralVault {
    pub owner: [u8; 32],
    pub BORB_locked: u64,       // BORB bloqués en garantie
    pub ust_minted: u64,         // UST créés
    pub created_at: i64,
    pub updated_at: i64,
    pub status: VaultStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum VaultStatus {
    Active,      // Coffre actif
    Liquidated,  // Liquidé (sous-collatéralisé)
    Closed,      // Fermé par l'utilisateur
}

/// Pool global UST
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UstPool {
    pub total_ust_minted: u64,          // Total UST en circulation
    pub total_BORB_locked: u64,        // Total BORB bloqués
    pub vaults: HashMap<[u8; 32], CollateralVault>,
    pub collateral_ratio: u64,          // Ratio de collatéral minimum (150% = 150)
    pub liquidation_ratio: u64,         // Ratio de liquidation (120% = 120)
    pub stability_fee: u64,            // Frais annuels (0.5% = 50 basis points)
}

impl UstPool {
    pub fn new() -> Self {
        UstPool {
            total_ust_minted: 0,
            total_BORB_locked: 0,
            vaults: HashMap::new(),
            collateral_ratio: 150,      // 150% de collatéral requis
            liquidation_ratio: 120,     // Liquidation si < 120%
            stability_fee: 50,          // 0.5% par an
        }
    }
    
    /// Crée des UST en bloquant des BORB
    pub fn mint_ust(
        &mut self,
        owner: [u8; 32],
        BORB_amount: u64,
        ust_amount: u64,
    ) -> Result<[u8; 32], ChainError> {
        if BORB_amount == 0 || ust_amount == 0 {
            return Err(ChainError::InsufficientFunds("Montant nul".into()));
        }
        
        // Vérifier le ratio de collatéral
        let BORB_value_usd = BORB_amount * crate::constants::BORB_PRICE_USD;
        let ust_value_usd = ust_amount;
        let ratio = (BORB_value_usd * 100) / ust_value_usd;
        
        if ratio < self.collateral_ratio {
            return Err(ChainError::InsufficientFunds(
                format!("Ratio de collatéral insuffisant: {}% (min: {}%)", ratio, self.collateral_ratio)
            ));
        }
        
        let now = chrono::Utc::now().timestamp();
        let vault = CollateralVault {
            owner,
            BORB_locked: BORB_amount,
            ust_minted: ust_amount,
            created_at: now,
            updated_at: now,
            status: VaultStatus::Active,
        };
        
        let vault_id = Self::compute_vault_id(&vault);
        self.vaults.insert(vault_id, vault);
        self.total_ust_minted += ust_amount;
        self.total_BORB_locked += BORB_amount;
        
        Ok(vault_id)
    }
    
    /// Brûle des UST et récupère les BORB
    pub fn burn_ust(
        &mut self,
        vault_id: &[u8; 32],
        ust_amount: u64,
    ) -> Result<u64, ChainError> {
        let vault = self.vaults.get_mut(vault_id)
            .ok_or_else(|| ChainError::InternalError("Coffre introuvable".into()))?;
        
        if vault.status != VaultStatus::Active {
            return Err(ChainError::InternalError("Coffre non actif".into()));
        }
        
        if ust_amount > vault.ust_minted {
            return Err(ChainError::InsufficientFunds("UST insuffisants".into()));
        }
        
        // Calculer le ratio de remboursement
        let BORB_to_return = (vault.BORB_locked * ust_amount) / vault.ust_minted;
        
        vault.ust_minted -= ust_amount;
        vault.BORB_locked -= BORB_to_return;
        vault.updated_at = chrono::Utc::now().timestamp();
        
        self.total_ust_minted -= ust_amount;
        self.total_BORB_locked -= BORB_to_return;
        
        // Nettoyer si vide
        if vault.ust_minted == 0 {
            vault.status = VaultStatus::Closed;
        }
        
        Ok(BORB_to_return)
    }
    
    /// Vérifie si un coffre est sous-collatéralisé
    pub fn is_undercollateralized(&self, vault_id: &[u8; 32]) -> Result<bool, ChainError> {
        let vault = self.vaults.get(vault_id)
            .ok_or_else(|| ChainError::InternalError("Coffre introuvable".into()))?;
        
        if vault.ust_minted == 0 {
            return Ok(false);
        }
        
        let BORB_value = vault.BORB_locked * crate::constants::BORB_PRICE_USD;
        let ratio = (BORB_value * 100) / vault.ust_minted;
        
        Ok(ratio < self.liquidation_ratio)
    }
    
    /// Liquide un coffre sous-collatéralisé
    pub fn liquidate(&mut self, vault_id: &[u8; 32]) -> Result<(u64, u64), ChainError> {
        let vault = self.vaults.get_mut(vault_id)
            .ok_or_else(|| ChainError::InternalError("Coffre introuvable".into()))?;
        
        if vault.status != VaultStatus::Active {
            return Err(ChainError::InternalError("Coffre non actif".into()));
        }
        
        let BORB_seized = vault.BORB_locked;
        let ust_burned = vault.ust_minted;
        
        vault.status = VaultStatus::Liquidated;
        vault.BORB_locked = 0;
        vault.ust_minted = 0;
        vault.updated_at = chrono::Utc::now().timestamp();
        
        self.total_ust_minted -= ust_burned;
        self.total_BORB_locked -= BORB_seized;
        
        Ok((BORB_seized, ust_burned))
    }
    
    /// Retourne le ratio de collatéral d'un coffre
    pub fn get_collateral_ratio(&self, vault_id: &[u8; 32]) -> Result<u64, ChainError> {
        let vault = self.vaults.get(vault_id)
            .ok_or_else(|| ChainError::InternalError("Coffre introuvable".into()))?;
        
        if vault.ust_minted == 0 {
            return Ok(0);
        }
        
        let BORB_value = vault.BORB_locked * crate::constants::BORB_PRICE_USD;
        Ok((BORB_value * 100) / vault.ust_minted)
    }
    
    fn compute_vault_id(vault: &CollateralVault) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(&vault.owner);
        hasher.update(&vault.BORB_locked.to_le_bytes());
        hasher.update(&vault.created_at.to_le_bytes());
        let result = hasher.finalize();
        let mut id = [0u8; 32];
        id.copy_from_slice(&result);
        id
    }
    
    pub fn get_total_ust(&self) -> u64 { self.total_ust_minted }
    pub fn get_total_locked(&self) -> u64 { self.total_BORB_locked }
}

/// ─── TESTS ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_mint_ust() {
        let mut pool = UstPool::new();
        let owner = [1u8; 32];
        
        // Bloquer 10 BORB pour créer 500 000 UST (ratio 200%)
        let vault_id = pool.mint_ust(owner, 10, 500_000).unwrap_or_default();
        
        assert_eq!(pool.get_total_ust(), 500_000);
        assert_eq!(pool.get_total_locked(), 10);
        
        let ratio = pool.get_collateral_ratio(&vault_id).unwrap_or_default();
        assert!(ratio >= 150); // Au-dessus du minimum
    }
    
    #[test]
    fn test_burn_ust() {
        let mut pool = UstPool::new();
        let owner = [2u8; 32];
        
        let vault_id = pool.mint_ust(owner, 10, 500_000).unwrap_or_default();
        let returned = pool.burn_ust(&vault_id, 250_000).unwrap_or_default();
        
        assert!(returned > 0);
        assert_eq!(pool.get_total_ust(), 250_000);
    }
    
    #[test]
    fn test_undercollateralized_rejected() {
        let mut pool = UstPool::new();
        let owner = [3u8; 32];
        
        // Essayer avec trop peu de collatéral (ratio < 150%)
        let result = pool.mint_ust(owner, 1, 500_000);
        assert!(result.is_err());
    }
    
    #[test]
    fn test_liquidation() {
        let mut pool = UstPool::new();
        let owner = [4u8; 32];
        
        let vault_id = pool.mint_ust(owner, 10, 500_000).unwrap_or_default();
        
        // Simuler une baisse de prix (le ratio devient critique)
        // Normalement déclenché par l'oracle
        let (seized, burned) = pool.liquidate(&vault_id).unwrap_or_default();
        assert!(seized > 0);
        assert!(burned > 0);
    }
}



