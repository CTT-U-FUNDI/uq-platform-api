use crate::constants::*;
use sha2::{Sha256, Digest};

// ═══════════════════════════════════════════════════════════════
// BORB — L'UNITÉ MONÉTAIRE UNIQUE (bulles = décimales)
// ═══════════════════════════════════════════════════════════════

/// Le BORB avec solde potentiellement NÉGATIF (dette)
#[derive(Debug, Clone)]
pub struct Borb {
    /// Solde total en FEMTO (peut être négatif)
    pub total_femto: i128,
}

impl Borb {
    /// Crée à partir de femto
    pub fn from_femto(femto: i128) -> Self {
        Self { total_femto: femto }
    }
    
    /// Crée à partir de BORB entiers + bulles
    pub fn from_borb_and_bulles(borb: u128, bulles: u128) -> Self {
        let total = (borb as i128) * FEMTO_PER_BORB as i128 
                  + (bulles as i128) * FEMTO_PER_BULLE as i128;
        Self { total_femto: total }
    }
    
    /// Convertit en BORB (f64 pour affichage)
    pub fn to_borb(&self) -> f64 {
        self.total_femto as f64 / FEMTO_PER_BORB as f64
    }
    
    /// Convertit en bulles
    pub fn to_bulles(&self) -> f64 {
        self.total_femto as f64 / FEMTO_PER_BULLE as f64
    }
    
    /// Affiche proprement : "3.2500 BORB" ou "32 500 bulles"
    pub fn display(&self) -> String {
        let fx = self.total_femto / FEMTO_PER_BORB as i128;
        let rem = self.total_femto % FEMTO_PER_BORB as i128;
        let bulles = rem / FEMTO_PER_BULLE as i128;
        format!("{}.{:04} BORB ({} bulles)", fx, bulles, bulles)
    }
    
    /// Addition
    pub fn add(&self, other: &Borb) -> Borb {
        Borb::from_femto(self.total_femto + other.total_femto)
    }
    
    /// Soustraction (avec vérification)
    pub fn sub(&self, other: &Borb) -> Result<Borb, String> {
        if self.total_femto < other.total_femto {
            return Err("Solde insuffisant".to_string());
        }
        Ok(Borb::from_femto(self.total_femto - other.total_femto))
    }
    
    /// Vérifie si le solde est en DETTE
    pub fn is_in_debt(&self) -> bool {
        self.total_femto < 0
    }
    
    /// Montant de la dette
    pub fn debt_amount(&self) -> u128 {
        if self.total_femto < 0 {
            (-self.total_femto) as u128
        } else {
            0
        }
    }
    
    /// PRÉLÈVEMENT AUTOMATIQUE de la dette sur les revenus
    pub fn auto_collect_debt(&mut self, incoming_amount: u128) -> u128 {
        if self.is_in_debt() {
            let debt = self.debt_amount();
            let collected = incoming_amount.min(debt);
            self.total_femto += collected as i128;
            return incoming_amount - collected;
        }
        incoming_amount
    }
    
    /// Calcule l'ID unique
    pub fn compute_id(owner: &[u8; 32]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(owner);
        hasher.update(b"UQ_BORB_V1");
        let result = hasher.finalize();
        let mut id = [0u8; 32];
        id.copy_from_slice(&result);
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_from_borb_and_bulles() {
        let f = Borb::from_borb_and_bulles(3, 2500);
        assert_eq!(f.to_borb(), 3.25);
        assert_eq!(f.to_bulles(), 32500.0);
    }
    
    #[test]
    fn test_negative_balance() {
        let mut f = Borb::from_femto(-5000000000);  // -50 BORB en dettes
        assert!(f.is_in_debt());
        assert_eq!(f.debt_amount(), 5000000000);
        
        // Revenu de 100 BORB
        let rest = f.auto_collect_debt(100_000_000_000);
        assert_eq!(rest, 50_000_000_000);  // 50 BORB restants
        assert!(!f.is_in_debt());
    }
    
    #[test]
    fn test_display() {
        let f = Borb::from_borb_and_bulles(3, 2500);
        assert_eq!(f.display(), "3.2500 BORB (2500 bulles)");
    }
}
