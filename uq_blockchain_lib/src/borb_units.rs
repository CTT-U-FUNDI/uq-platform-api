use crate::constants::*;

// ═══════════════════════════════════════════════════════════════
// EXTENSION FRACTALE — UNITÉS AUTO-GÉNÉRÉES
// ═══════════════════════════════════════════════════════════════

pub struct UnitRegistry {
    pub units: Vec<Unit>,
}

#[derive(Debug, Clone)]
pub struct Unit {
    pub name: String,
    pub symbol: String,
    pub factor: u128,
    pub children: Vec<Unit>,
}

impl UnitRegistry {
    /// Initialise les 6 unités de base
    pub fn new() -> Self {
        let mut units = Vec::new();
        
        let femto_unit = Unit {
            name: "FEMTO-FIXO".to_string(),
            symbol: "fborb".to_string(),
            factor: 1,
            children: Vec::new(),
        };
        
        let pico_unit = Unit {
            name: "PICO-FIXO".to_string(),
            symbol: "pborb".to_string(),
            factor: SUB_UNIT_FACTOR as u128,
            children: vec![femto_unit],
        };
        
        let nano_unit = Unit {
            name: "NANO-FIXO".to_string(),
            symbol: "nborb".to_string(),
            factor: SUB_UNIT_FACTOR as u128 * SUB_UNIT_FACTOR as u128,
            children: vec![pico_unit],
        };
        
        let micro_unit = Unit {
            name: "MICRO-FIXO".to_string(),
            symbol: "mborb".to_string(),
            factor: SUB_UNIT_FACTOR as u128 * SUB_UNIT_FACTOR as u128 * SUB_UNIT_FACTOR as u128,
            children: vec![nano_unit],
        };
        
        let fixo_unit = Unit {
            name: "FIXO".to_string(),
            symbol: "borb".to_string(),
            factor: FEMTO_PER_BULLE as u128,
            children: vec![micro_unit],
        };
        
        let borb_unit = Unit {
            name: "BORB".to_string(),
            symbol: "BORB".to_string(),
            factor: FEMTO_PER_BORB as u128,
            children: vec![fixo_unit],
        };
        
        units.push(borb_unit);
        Self { units }
    }
    
    /// VÉRIFIE SI L'EXTENSION EST NÉCESSAIRE
    pub fn check_extension(&mut self, borb_price_usd: f64) -> bool {
        let smallest_value = borb_price_usd as f64 / FEMTO_PER_BORB as f64 as f64;
        
        if smallest_value > EXTENSION_THRESHOLD_USD as f64 {
            self.extend_smallest_unit();
            return true;
        }
        false
    }
    
    /// CRÉE 3 SOUS-UNITÉS (a, b, c) sous la plus petite unité
    pub fn extend_smallest_unit(&mut self) {
        let prefixes = ["a", "b", "c"];
        
        // Trouver la plus petite unité (récursivement)
        let smallest = Self::find_smallest_mut(&mut self.units);
        
        for prefix in prefixes {
            smallest.children.push(Unit {
                name: format!("{}-FIXO", prefix.to_uppercase()),
                symbol: format!("{}fborb", prefix),
                factor: SUB_UNIT_FACTOR as u128,
                children: Vec::new(),
            });
        }
        
        log::info!("🔄 Extension fractale : 3 nouvelles sous-unités créées");
    }
    
    /// Trouve itérativement la plus petite unité
    fn find_smallest_mut(units: &mut Vec<Unit>) -> &mut Unit {
        let mut current = units;
        
        loop {
            // Vérifier si le premier élément a des enfants
            let has_children = !current[0].children.is_empty();
            
            if has_children {
                // Descendre dans les enfants du premier élément
                let child = &mut current[0].children;
                current = child;
            } else {
                // Pas d'enfants = c'est la plus petite
                return &mut current[0];
            }
        }
    }
    
    /// LISTE des unités actives pour l'API
    pub fn get_active_units(&self) -> Vec<String> {
        let mut result = Vec::new();
        Self::collect_units(&self.units, &mut result);
        result
    }
    
    /// Collecte récursivement tous les symboles
    fn collect_units(units: &[Unit], result: &mut Vec<String>) {
        for unit in units {
            result.push(unit.symbol.clone());
            if !unit.children.is_empty() {
                Self::collect_units(&unit.children, result);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_initial_units() {
        let registry = UnitRegistry::new();
        let units = registry.get_active_units();
        assert_eq!(units.len(), 6);
        assert_eq!(units[0], "BORB");
        assert_eq!(units[1], "borb");
    }
    
    #[test]
    fn test_extension_triggered() {
        let mut registry = UnitRegistry::new();
        
        // Simuler un BORB très cher (or très haut)
        let triggered = registry.check_extension(1000.0);
        assert!(triggered);  // fborb > 0.001 USD → extension
    }
    
    #[test]
    fn test_no_extension() {
        let mut registry = UnitRegistry::new();
        
        // BORB normal (1 USD)
        let triggered = registry.check_extension(1.0);
        assert!(!triggered);  // fborb < 0.001 USD → pas d'extension
    }
}



