use uq_blockchain::error::ChainError;

// ═══════════════════════════════════════════════════════════════
// TYPES DE PUISSANCE
// ═══════════════════════════════════════════════════════════════

/// Niveaux de puissance prédéfinis
#[derive(Debug, Clone, PartialEq)]
pub enum PowerLevel {
    /// Puissance minimale (-80 dBm) — Très courte portée
    Minimal,
    /// Puissance basse (-60 dBm) — Courte portée
    Low,
    /// Puissance normale (-40 dBm) — Portée standard
    Normal,
    /// Puissance élevée (-20 dBm) — Longue portée
    High,
    /// Puissance maximale (0 dBm) — Portée maximale
    Maximum,
    /// Puissance d'urgence (0 dBm) — Mode urgence uniquement
    Emergency,
    /// Puissance personnalisée (dBm)
    Custom(i8),
}

impl PowerLevel {
    /// Convertit en dBm
    pub fn to_dbm(&self) -> i8 {
        match self {
            PowerLevel::Minimal => -80,
            PowerLevel::Low => -60,
            PowerLevel::Normal => -40,
            PowerLevel::High => -20,
            PowerLevel::Maximum => 0,
            PowerLevel::Emergency => 0,
            PowerLevel::Custom(dbm) => *dbm,
        }
    }

    /// Convertit en milliwatts (mW)
    pub fn to_mw(&self) -> f64 {
        10.0_f64.powf(self.to_dbm() as f64 / 10.0)
    }

    /// Nom lisible
    pub fn name(&self) -> String {
        match self {
            PowerLevel::Minimal => "Minimal (-80 dBm)".into(),
            PowerLevel::Low => "Bas (-60 dBm)".into(),
            PowerLevel::Normal => "Normal (-40 dBm)".into(),
            PowerLevel::High => "Élevé (-20 dBm)".into(),
            PowerLevel::Maximum => "Maximum (0 dBm)".into(),
            PowerLevel::Emergency => "URGENCE (0 dBm)".into(),
            PowerLevel::Custom(dbm) => format!("Custom ({} dBm)", dbm),
        }
    }

    /// Est-ce un niveau d'urgence ?
    pub fn is_emergency(&self) -> bool {
        matches!(self, PowerLevel::Emergency)
    }

    /// Portée estimée en mètres (espace libre)
    pub fn estimated_range_m(&self) -> f64 {
        match self {
            PowerLevel::Minimal => 5.0,
            PowerLevel::Low => 25.0,
            PowerLevel::Normal => 100.0,
            PowerLevel::High => 500.0,
            PowerLevel::Maximum => 2000.0,
            PowerLevel::Emergency => 2000.0,
            PowerLevel::Custom(dbm) => 10.0_f64.powf((*dbm as f64 + 40.0) / 20.0),
        }
    }

    /// Consommation estimée (mA)
    pub fn current_ma(&self) -> f64 {
        match self {
            PowerLevel::Minimal => 50.0,
            PowerLevel::Low => 100.0,
            PowerLevel::Normal => 200.0,
            PowerLevel::High => 400.0,
            PowerLevel::Maximum => 800.0,
            PowerLevel::Emergency => 1000.0,
            PowerLevel::Custom(dbm) => 10.0_f64.powf((*dbm as f64 + 80.0) / 20.0),
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// GESTIONNAIRE DE PUISSANCE
// ═══════════════════════════════════════════════════════════════

pub struct PowerController {
    /// Niveau actuel
    current_level: PowerLevel,
    /// Batterie restante (%)
    battery_pct: u8,
    /// Température (°C)
    temperature_c: f32,
    /// Mode économie d'énergie
    eco_mode: bool,
    /// Verrouillage urgence
    emergency_lock: bool,
    /// Historique des changements
    changes_count: u64,
    /// Temps total d'émission (secondes)
    total_tx_time_s: u64,
    /// Énergie totale consommée (mAh)
    total_energy_mah: f64,
}

impl PowerController {
    /// Crée un nouveau contrôleur de puissance
    pub fn new(battery_pct: u8) -> Self {
        PowerController {
            current_level: PowerLevel::Normal,
            battery_pct: battery_pct.min(100),
            temperature_c: 25.0,
            eco_mode: false,
            emergency_lock: false,
            changes_count: 0,
            total_tx_time_s: 0,
            total_energy_mah: 0.0,
        }
    }

    /// Définit le niveau de puissance
    pub fn set_power(&mut self, level: PowerLevel) -> Result<(), ChainError> {
        // Vérifier le verrouillage urgence
        if self.emergency_lock && !level.is_emergency() {
            return Err(ChainError::InternalError(
                "Verrouillage urgence actif. Seul le niveau Emergency est autorisé.".into()
            ));
        }

        // Vérifier la batterie
        if self.battery_pct < 5 && level.to_dbm() > -60 {
            return Err(ChainError::InternalError(
                "Batterie trop faible pour ce niveau de puissance".into()
            ));
        }

        // Vérifier la température
        if self.temperature_c > 80.0 && level.to_dbm() > -40 {
            return Err(ChainError::InternalError(
                "Température trop élevée pour ce niveau de puissance".into()
            ));
        }

        // Vérifier les limites
        let dbm = level.to_dbm();
        if dbm < -80 || dbm > 0 {
            return Err(ChainError::InternalError(
                format!("Niveau de puissance hors limites : {} dBm", dbm)
            ));
        }

        self.current_level = level;
        self.changes_count += 1;
        Ok(())
    }

    /// Active le mode économie d'énergie
    pub fn enable_eco_mode(&mut self) {
        self.eco_mode = true;
        if self.current_level.to_dbm() > -60 {
            let _ = self.set_power(PowerLevel::Low);
        }
    }

    /// Désactive le mode économie d'énergie
    pub fn disable_eco_mode(&mut self) {
        self.eco_mode = false;
    }

    /// Active le verrouillage urgence
    pub fn lock_emergency(&mut self) -> Result<(), ChainError> {
        self.emergency_lock = true;
        self.set_power(PowerLevel::Emergency)
    }

    /// Désactive le verrouillage urgence
    pub fn unlock_emergency(&mut self) {
        self.emergency_lock = false;
    }

    /// Met à jour le niveau de batterie
    pub fn update_battery(&mut self, pct: u8) {
        self.battery_pct = pct.min(100);
        
        // Si batterie très faible, forcer l'éco mode
        if self.battery_pct < 10 && !self.eco_mode {
            self.enable_eco_mode();
        }
    }

    /// Met à jour la température
    pub fn update_temperature(&mut self, temp_c: f32) {
        self.temperature_c = temp_c;
        
        // Si trop chaud, réduire la puissance
        if self.temperature_c > 85.0 && self.current_level.to_dbm() > -60 {
            let _ = self.set_power(PowerLevel::Low);
        }
    }

    /// Enregistre du temps d'émission
    pub fn record_tx_time(&mut self, seconds: u64) {
        self.total_tx_time_s += seconds;
        self.total_energy_mah += self.current_level.current_ma() * (seconds as f64 / 3600.0);
    }

    /// Retourne le niveau actuel
    pub fn get_level(&self) -> &PowerLevel {
        &self.current_level
    }

    /// Retourne la puissance en dBm
    pub fn get_dbm(&self) -> i8 {
        self.current_level.to_dbm()
    }

    /// Retourne la puissance en mW
    pub fn get_mw(&self) -> f64 {
        self.current_level.to_mw()
    }

    /// Portée estimée actuelle
    pub fn get_range_m(&self) -> f64 {
        self.current_level.estimated_range_m()
    }

    /// Vérifie si le mode éco est actif
    pub fn is_eco_mode(&self) -> bool {
        self.eco_mode
    }

    /// Vérifie si le verrouillage urgence est actif
    pub fn is_emergency_locked(&self) -> bool {
        self.emergency_lock
    }

    /// Retourne la batterie restante
    pub fn get_battery(&self) -> u8 {
        self.battery_pct
    }

    /// Retourne la température
    pub fn get_temperature(&self) -> f32 {
        self.temperature_c
    }

    /// Calcule l'autonomie restante (heures) au niveau actuel
    pub fn estimated_remaining_hours(&self) -> f64 {
        if self.battery_pct == 0 { return 0.0; }
        let battery_capacity_mah = 3000.0; // Batterie typique 3000 mAh
        let available = battery_capacity_mah * (self.battery_pct as f64 / 100.0);
        available / self.current_level.current_ma() / 1000.0
    }

    /// Retourne les statistiques
    pub fn get_stats(&self) -> PowerStats {
        PowerStats {
            current_level: self.current_level.name(),
            dbm: self.get_dbm(),
            mw: self.get_mw(),
            range_m: self.get_range_m(),
            battery_pct: self.battery_pct,
            temperature_c: self.temperature_c,
            eco_mode: self.eco_mode,
            emergency_lock: self.emergency_lock,
            changes_count: self.changes_count,
            total_tx_time_s: self.total_tx_time_s,
            total_energy_mah: self.total_energy_mah,
            estimated_hours: self.estimated_remaining_hours(),
        }
    }
}

/// Statistiques de puissance
#[derive(Debug, Clone)]
pub struct PowerStats {
    pub current_level: String,
    pub dbm: i8,
    pub mw: f64,
    pub range_m: f64,
    pub battery_pct: u8,
    pub temperature_c: f32,
    pub eco_mode: bool,
    pub emergency_lock: bool,
    pub changes_count: u64,
    pub total_tx_time_s: u64,
    pub total_energy_mah: f64,
    pub estimated_hours: f64,
}

// ═══════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    // ─── Tests PowerLevel ──────────────────────

    #[test] fn test_power_to_dbm() {
        assert_eq!(PowerLevel::Minimal.to_dbm(), -80);
        assert_eq!(PowerLevel::Low.to_dbm(), -60);
        assert_eq!(PowerLevel::Normal.to_dbm(), -40);
        assert_eq!(PowerLevel::High.to_dbm(), -20);
        assert_eq!(PowerLevel::Maximum.to_dbm(), 0);
        assert_eq!(PowerLevel::Emergency.to_dbm(), 0);
        assert_eq!(PowerLevel::Custom(-30).to_dbm(), -30);
    }

    #[test] fn test_power_to_mw() {
        let mw = PowerLevel::Normal.to_mw();
        assert!(mw > 0.0);
        assert!(mw < 1.0); // -40 dBm = 0.0001 mW
    }

    #[test] fn test_power_range() {
        assert!(PowerLevel::Normal.estimated_range_m() > PowerLevel::Low.estimated_range_m());
        assert!(PowerLevel::High.estimated_range_m() > PowerLevel::Normal.estimated_range_m());
    }

    #[test] fn test_power_current() {
        assert!(PowerLevel::High.current_ma() > PowerLevel::Normal.current_ma());
        assert!(PowerLevel::Emergency.current_ma() > PowerLevel::High.current_ma());
    }

    #[test] fn test_power_is_emergency() {
        assert!(PowerLevel::Emergency.is_emergency());
        assert!(!PowerLevel::Normal.is_emergency());
    }

    #[test] fn test_power_name() {
        assert!(PowerLevel::Normal.name().contains("-40"));
        assert!(PowerLevel::Emergency.name().contains("URGENCE"));
    }

    // ─── Tests PowerController ─────────────────

    #[test] fn test_controller_new() {
        let pc = PowerController::new(80);
        assert_eq!(pc.get_level(), &PowerLevel::Normal);
        assert_eq!(pc.get_battery(), 80);
        assert!(!pc.is_eco_mode());
    }

    #[test] fn test_set_power_valid() {
        let mut pc = PowerController::new(80);
        assert!(pc.set_power(PowerLevel::Low).is_ok());
        assert_eq!(pc.get_level(), &PowerLevel::Low);
    }

    #[test] fn test_set_power_custom() {
        let mut pc = PowerController::new(80);
        assert!(pc.set_power(PowerLevel::Custom(-50)).is_ok());
        assert_eq!(pc.get_dbm(), -50);
    }

    #[test] fn test_set_power_out_of_range() {
        let mut pc = PowerController::new(80);
        assert!(pc.set_power(PowerLevel::Custom(-90)).is_err());
        assert!(pc.set_power(PowerLevel::Custom(10)).is_err());
    }

    #[test] fn test_low_battery_restriction() {
        let mut pc = PowerController::new(3); // 3% batterie
        assert!(pc.set_power(PowerLevel::High).is_err());
        assert!(pc.set_power(PowerLevel::Low).is_ok());
    }

    #[test] fn test_high_temperature_restriction() {
        let mut pc = PowerController::new(80);
        pc.update_temperature(85.0);
        assert!(pc.set_power(PowerLevel::High).is_err());
        assert!(pc.set_power(PowerLevel::Low).is_ok());
    }

    #[test] fn test_eco_mode() {
        let mut pc = PowerController::new(80);
        pc.set_power(PowerLevel::High).unwrap();
        pc.enable_eco_mode();
        assert!(pc.is_eco_mode());
        assert_eq!(pc.get_level(), &PowerLevel::Low); // Forcé en Low
    }

    #[test] fn test_emergency_lock() {
        let mut pc = PowerController::new(80);
        pc.lock_emergency().unwrap();
        assert!(pc.is_emergency_locked());
        assert_eq!(pc.get_level(), &PowerLevel::Emergency);
        // Ne peut pas changer
        assert!(pc.set_power(PowerLevel::Normal).is_err());
    }

    #[test] fn test_unlock_emergency() {
        let mut pc = PowerController::new(80);
        pc.lock_emergency().unwrap();
        pc.unlock_emergency();
        assert!(!pc.is_emergency_locked());
        assert!(pc.set_power(PowerLevel::Normal).is_ok());
    }

    #[test] fn test_battery_update_triggers_eco() {
        let mut pc = PowerController::new(80);
        pc.set_power(PowerLevel::High).unwrap();
        pc.update_battery(8); // < 10%
        assert!(pc.is_eco_mode());
    }

    #[test] fn test_temperature_update_reduces_power() {
        let mut pc = PowerController::new(80);
        pc.set_power(PowerLevel::High).unwrap();
        pc.update_temperature(90.0); // > 85°C
        assert_eq!(pc.get_level(), &PowerLevel::Low);
    }

    #[test] fn test_record_tx_time() {
        let mut pc = PowerController::new(80);
        pc.record_tx_time(3600); // 1 heure
        assert_eq!(pc.total_tx_time_s, 3600);
        assert!(pc.total_energy_mah > 0.0);
    }

    #[test] fn test_estimated_hours() {
        let pc = PowerController::new(100);
        let hours = pc.estimated_remaining_hours();
        assert!(hours > 0.0);
    }

    #[test] fn test_estimated_hours_empty() {
        let pc = PowerController::new(0);
        assert_eq!(pc.estimated_remaining_hours(), 0.0);
    }

    #[test] fn test_stats() {
        let mut pc = PowerController::new(80);
        pc.set_power(PowerLevel::Low).unwrap();
        pc.record_tx_time(1800); // 30 minutes
        let stats = pc.get_stats();
        assert_eq!(stats.dbm, -60);
        assert_eq!(stats.battery_pct, 80);
        assert!(stats.total_tx_time_s > 0);
    }

    #[test] fn test_changes_count() {
        let mut pc = PowerController::new(80);
        pc.set_power(PowerLevel::Low).unwrap();
        pc.set_power(PowerLevel::Normal).unwrap();
        pc.set_power(PowerLevel::High).unwrap();
        assert_eq!(pc.changes_count, 3);
    }
}

