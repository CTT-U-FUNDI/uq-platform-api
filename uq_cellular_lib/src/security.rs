use uq_blockchain::error::ChainError;

// ═══════════════════════════════════════════════════════════════
// TYPES DE MENACES
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq)]
pub enum ThreatLevel {
    None,
    Low,
    Medium,
    High,
    Critical,
}

impl ThreatLevel {
    pub fn name(&self) -> &str {
        match self {
            ThreatLevel::None => "Aucune",
            ThreatLevel::Low => "Faible",
            ThreatLevel::Medium => "Moyen",
            ThreatLevel::High => "Élevé",
            ThreatLevel::Critical => "Critique",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ThreatType {
    ImsiCatcher,
    Jamming,
    ManInTheMiddle,
    RogueBaseStation,
    DowngradeAttack,
    SmsSpoofing,
    Unknown,
}

impl ThreatType {
    pub fn name(&self) -> &str {
        match self {
            ThreatType::ImsiCatcher => "IMSI Catcher",
            ThreatType::Jamming => "Brouillage",
            ThreatType::ManInTheMiddle => "MITM",
            ThreatType::RogueBaseStation => "Fausse station",
            ThreatType::DowngradeAttack => "Downgrade 2G",
            ThreatType::SmsSpoofing => "SMS spoofing",
            ThreatType::Unknown => "Inconnue",
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// ALERTE DE SÉCURITÉ
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone)]
pub struct SecurityAlert {
    pub threat_type: ThreatType,
    pub level: ThreatLevel,
    pub description: String,
    pub timestamp: u64,
    pub mitigated: bool,
}

impl SecurityAlert {
    pub fn new(threat_type: ThreatType, level: ThreatLevel, description: &str) -> Self {
        SecurityAlert {
            threat_type,
            level,
            description: description.to_string(),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            mitigated: false,
        }
    }

    pub fn mitigate(&mut self) {
        self.mitigated = true;
    }
}

// ═══════════════════════════════════════════════════════════════
// GESTIONNAIRE DE SÉCURITÉ
// ═══════════════════════════════════════════════════════════════

pub struct SecurityManager {
    alerts: Vec<SecurityAlert>,
    current_threat_level: ThreatLevel,
    imsi_protection: bool,
    downgrade_protection: bool,
    sms_verification: bool,
    total_alerts: u64,
    total_mitigated: u64,
}

impl SecurityManager {
    pub fn new() -> Self {
        SecurityManager {
            alerts: Vec::new(),
            current_threat_level: ThreatLevel::None,
            imsi_protection: true,
            downgrade_protection: true,
            sms_verification: true,
            total_alerts: 0,
            total_mitigated: 0,
        }
    }

    /// Détecte un IMSI Catcher (changement suspect de LAC/CellID)
    pub fn detect_imsi_catcher(&mut self, cell_id_changed_rapidly: bool, signal_anomaly: bool) -> Option<&SecurityAlert> {
        if cell_id_changed_rapidly && signal_anomaly {
            let alert = SecurityAlert::new(
                ThreatType::ImsiCatcher,
                ThreatLevel::High,
                "Changement rapide de CellID avec anomalie de signal"
            );
            self.add_alert(alert)
        } else if cell_id_changed_rapidly {
            let alert = SecurityAlert::new(
                ThreatType::ImsiCatcher,
                ThreatLevel::Medium,
                "Changement rapide de CellID suspect"
            );
            self.add_alert(alert)
        } else {
            None
        }
    }

    /// Détecte un brouillage
    pub fn detect_jamming(&mut self, signal_lost_suddenly: bool, noise_floor_high: bool) -> Option<&SecurityAlert> {
        if signal_lost_suddenly && noise_floor_high {
            let alert = SecurityAlert::new(
                ThreatType::Jamming,
                ThreatLevel::Critical,
                "Perte brutale de signal avec bruit élevé - Brouillage probable"
            );
            self.add_alert(alert)
        } else {
            None
        }
    }

    /// Détecte une attaque de downgrade 2G
    pub fn detect_downgrade_attack(&mut self, network_type_changed_to_2g: bool, previously_on_4g: bool) -> Option<&SecurityAlert> {
        if network_type_changed_to_2g && previously_on_4g && self.downgrade_protection {
            let alert = SecurityAlert::new(
                ThreatType::DowngradeAttack,
                ThreatLevel::High,
                "Rétrogradation forcée vers 2G détectée"
            );
            self.add_alert(alert)
        } else {
            None
        }
    }

    /// Détecte un spoofing SMS
    pub fn detect_sms_spoofing(&mut self, sender_unknown: bool, contains_malicious_link: bool) -> Option<&SecurityAlert> {
        if sender_unknown && contains_malicious_link {
            let alert = SecurityAlert::new(
                ThreatType::SmsSpoofing,
                ThreatLevel::Medium,
                "SMS suspect avec lien malveillant"
            );
            self.add_alert(alert)
        } else {
            None
        }
    }

    /// Ajoute une alerte
    fn add_alert(&mut self, alert: SecurityAlert) -> Option<&SecurityAlert> {
        self.total_alerts += 1;
        self.alerts.push(alert);
        self.update_threat_level();
        self.alerts.last()
    }

    /// Mitige une alerte
    pub fn mitigate_alert(&mut self, index: usize) -> Result<(), ChainError> {
        if index >= self.alerts.len() {
            return Err(ChainError::InternalError("Index d'alerte invalide".into()));
        }
        self.alerts[index].mitigate();
        self.total_mitigated += 1;
        self.update_threat_level();
        Ok(())
    }

    /// Mitige toutes les alertes
    pub fn mitigate_all(&mut self) {
        for alert in &mut self.alerts {
            if !alert.mitigated {
                alert.mitigate();
                self.total_mitigated += 1;
            }
        }
        self.current_threat_level = ThreatLevel::None;
    }

    /// Met à jour le niveau de menace global
    fn update_threat_level(&mut self) {
        let active_alerts: Vec<&SecurityAlert> = self.alerts.iter().filter(|a| !a.mitigated).collect();

        self.current_threat_level = if active_alerts.iter().any(|a| a.level == ThreatLevel::Critical) {
            ThreatLevel::Critical
        } else if active_alerts.iter().any(|a| a.level == ThreatLevel::High) {
            ThreatLevel::High
        } else if active_alerts.iter().any(|a| a.level == ThreatLevel::Medium) {
            ThreatLevel::Medium
        } else if !active_alerts.is_empty() {
            ThreatLevel::Low
        } else {
            ThreatLevel::None
        };
    }

    /// Active/désactive la protection IMSI
    pub fn set_imsi_protection(&mut self, enabled: bool) {
        self.imsi_protection = enabled;
    }

    /// Active/désactive la protection downgrade
    pub fn set_downgrade_protection(&mut self, enabled: bool) {
        self.downgrade_protection = enabled;
    }

    /// Active/désactive la vérification SMS
    pub fn set_sms_verification(&mut self, enabled: bool) {
        self.sms_verification = enabled;
    }

    /// Retourne les alertes actives
    pub fn get_active_alerts(&self) -> Vec<&SecurityAlert> {
        self.alerts.iter().filter(|a| !a.mitigated).collect()
    }

    /// Retourne toutes les alertes
    pub fn get_all_alerts(&self) -> &[SecurityAlert] {
        &self.alerts
    }

    /// Retourne le niveau de menace actuel
    pub fn get_threat_level(&self) -> &ThreatLevel {
        &self.current_threat_level
    }

    /// Vérifie si le système est en sécurité
    pub fn is_secure(&self) -> bool {
        self.current_threat_level == ThreatLevel::None || self.current_threat_level == ThreatLevel::Low
    }

    /// Retourne les statistiques
    pub fn get_stats(&self) -> SecurityStats {
        SecurityStats {
            threat_level: self.current_threat_level.name().to_string(),
            total_alerts: self.total_alerts,
            active_alerts: self.get_active_alerts().len(),
            total_mitigated: self.total_mitigated,
            imsi_protection: self.imsi_protection,
            downgrade_protection: self.downgrade_protection,
            sms_verification: self.sms_verification,
            is_secure: self.is_secure(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SecurityStats {
    pub threat_level: String,
    pub total_alerts: u64,
    pub active_alerts: usize,
    pub total_mitigated: u64,
    pub imsi_protection: bool,
    pub downgrade_protection: bool,
    pub sms_verification: bool,
    pub is_secure: bool,
}

// ═══════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    // ─── Tests ThreatLevel ──────────────────────

    #[test] fn test_threat_level_names() {
        assert_eq!(ThreatLevel::None.name(), "Aucune");
        assert_eq!(ThreatLevel::Critical.name(), "Critique");
    }

    // ─── Tests SecurityAlert ────────────────────

    #[test] fn test_alert_new() {
        let alert = SecurityAlert::new(ThreatType::ImsiCatcher, ThreatLevel::High, "Test");
        assert_eq!(alert.threat_type, ThreatType::ImsiCatcher);
        assert!(!alert.mitigated);
    }

    #[test] fn test_alert_mitigate() {
        let mut alert = SecurityAlert::new(ThreatType::Jamming, ThreatLevel::Critical, "Test");
        alert.mitigate();
        assert!(alert.mitigated);
    }

    // ─── Tests SecurityManager ──────────────────

    #[test] fn test_manager_new() {
        let sm = SecurityManager::new();
        assert_eq!(sm.get_threat_level(), &ThreatLevel::None);
        assert!(sm.is_secure());
    }

    #[test] fn test_detect_imsi_catcher() {
        let mut sm = SecurityManager::new();
        let result = sm.detect_imsi_catcher(true, true);
        assert!(result.is_some());
        assert_eq!(sm.get_threat_level(), &ThreatLevel::High);
    }

    #[test] fn test_detect_imsi_catcher_medium() {
        let mut sm = SecurityManager::new();
        let result = sm.detect_imsi_catcher(true, false);
        assert!(result.is_some());
        assert_eq!(sm.get_threat_level(), &ThreatLevel::Medium);
    }

    #[test] fn test_detect_imsi_catcher_none() {
        let mut sm = SecurityManager::new();
        let result = sm.detect_imsi_catcher(false, false);
        assert!(result.is_none());
    }

    #[test] fn test_detect_jamming() {
        let mut sm = SecurityManager::new();
        let result = sm.detect_jamming(true, true);
        assert!(result.is_some());
        assert_eq!(sm.get_threat_level(), &ThreatLevel::Critical);
    }

    #[test] fn test_detect_jamming_none() {
        let mut sm = SecurityManager::new();
        let result = sm.detect_jamming(false, false);
        assert!(result.is_none());
    }

    #[test] fn test_detect_downgrade() {
        let mut sm = SecurityManager::new();
        let result = sm.detect_downgrade_attack(true, true);
        assert!(result.is_some());
        assert_eq!(sm.get_threat_level(), &ThreatLevel::High);
    }

    #[test] fn test_detect_downgrade_protection_off() {
        let mut sm = SecurityManager::new();
        sm.set_downgrade_protection(false);
        let result = sm.detect_downgrade_attack(true, true);
        assert!(result.is_none());
    }

    #[test] fn test_detect_sms_spoofing() {
        let mut sm = SecurityManager::new();
        let result = sm.detect_sms_spoofing(true, true);
        assert!(result.is_some());
    }

    #[test] fn test_mitigate_alert() {
        let mut sm = SecurityManager::new();
        sm.detect_imsi_catcher(true, true);
        assert!(sm.mitigate_alert(0).is_ok());
        assert_eq!(sm.get_threat_level(), &ThreatLevel::None);
    }

    #[test] fn test_mitigate_invalid() {
        let mut sm = SecurityManager::new();
        assert!(sm.mitigate_alert(99).is_err());
    }

    #[test] fn test_mitigate_all() {
        let mut sm = SecurityManager::new();
        sm.detect_imsi_catcher(true, true);
        sm.detect_jamming(true, true);
        sm.mitigate_all();
        assert!(sm.is_secure());
    }

    #[test] fn test_multiple_threats() {
        let mut sm = SecurityManager::new();
        sm.detect_jamming(true, true); // Critical
        sm.detect_imsi_catcher(true, true); // High
        // Le niveau doit être Critical (le plus élevé)
        assert_eq!(sm.get_threat_level(), &ThreatLevel::Critical);
    }

    #[test] fn test_active_alerts() {
        let mut sm = SecurityManager::new();
        sm.detect_imsi_catcher(true, true);
        sm.detect_jamming(true, true);
        assert_eq!(sm.get_active_alerts().len(), 2);
        sm.mitigate_alert(0).unwrap();
        assert_eq!(sm.get_active_alerts().len(), 1);
    }

    #[test] fn test_stats() {
        let mut sm = SecurityManager::new();
        sm.detect_imsi_catcher(true, true);
        let stats = sm.get_stats();
        assert_eq!(stats.total_alerts, 1);
        assert_eq!(stats.active_alerts, 1);
        assert!(stats.imsi_protection);
    }

    #[test] fn test_protection_toggles() {
        let mut sm = SecurityManager::new();
        sm.set_imsi_protection(false);
        sm.set_sms_verification(false);
        let stats = sm.get_stats();
        assert!(!stats.imsi_protection);
        assert!(!stats.sms_verification);
    }
}

