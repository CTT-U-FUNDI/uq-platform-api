use uq_blockchain::error::ChainError;

// ═══════════════════════════════════════════════════════════════
// BANDES DE FRÉQUENCE UQ
// ═══════════════════════════════════════════════════════════════

/// Bande de fréquence supportée
#[derive(Debug, Clone, PartialEq)]
pub struct FrequencyBand {
    pub name: String,
    pub start_mhz: u32,
    pub end_mhz: u32,
    pub is_ism: bool,
    pub max_power_dbm: i8,
    pub channel_width_khz: u32,
}

impl FrequencyBand {
    /// Nombre de canaux dans cette bande
    pub fn channel_count(&self) -> u32 {
        (self.end_mhz - self.start_mhz) * 1000 / self.channel_width_khz
    }

    /// Vérifie si une fréquence est dans cette bande
    pub fn contains(&self, freq_mhz: u32) -> bool {
        freq_mhz >= self.start_mhz && freq_mhz <= self.end_mhz
    }
}

/// Bandes prédéfinies UQ
pub const UQ_BANDS: &[FrequencyBand] = &[
    FrequencyBand { name: String::new(), start_mhz: 2400, end_mhz: 2483, is_ism: true, max_power_dbm: 0, channel_width_khz: 1000 },
    // ISM 2.4 GHz
];

// Initialisation statique simplifiée
pub fn get_default_bands() -> Vec<FrequencyBand> {
    vec![
        FrequencyBand { name: "ISM 2.4 GHz".into(), start_mhz: 2400, end_mhz: 2483, is_ism: true, max_power_dbm: 0, channel_width_khz: 1000 },
        FrequencyBand { name: "ISM 5 GHz".into(), start_mhz: 5150, end_mhz: 5875, is_ism: true, max_power_dbm: 0, channel_width_khz: 20000 },
        FrequencyBand { name: "ISM 6 GHz".into(), start_mhz: 5925, end_mhz: 7125, is_ism: true, max_power_dbm: 0, channel_width_khz: 20000 },
        FrequencyBand { name: "SRD 868 MHz".into(), start_mhz: 863, end_mhz: 870, is_ism: true, max_power_dbm: 14, channel_width_khz: 200 },
        FrequencyBand { name: "ISM 915 MHz".into(), start_mhz: 902, end_mhz: 928, is_ism: true, max_power_dbm: 0, channel_width_khz: 500 },
    ]
}

// ═══════════════════════════════════════════════════════════════
// SÉQUENCE DE SAUT
// ═══════════════════════════════════════════════════════════════

/// Séquence de saut de fréquence
#[derive(Debug, Clone)]
pub struct HopSequence {
    /// Fréquences dans l'ordre de saut
    frequencies: Vec<u32>,
    /// Index actuel
    current_index: usize,
    /// Nombre de sauts effectués
    hops_count: u64,
    /// Intervalle entre sauts (ms)
    hop_interval_ms: u64,
}

impl HopSequence {
    /// Crée une séquence de saut aléatoire
    pub fn new_random(band: &FrequencyBand, count: usize, interval_ms: u64) -> Self {
        let step = (band.end_mhz - band.start_mhz) * 1000 / band.channel_width_khz;
        let mut frequencies: Vec<u32> = (0..count)
            .map(|_| band.start_mhz + (rand::random::<u32>() % step) * band.channel_width_khz / 1000)
            .collect();
        frequencies.sort();
        frequencies.dedup();
        
        // Si dedup a réduit, compléter
        while frequencies.len() < count {
            let f = band.start_mhz + (rand::random::<u32>() % step) * band.channel_width_khz / 1000;
            if !frequencies.contains(&f) {
                frequencies.push(f);
            }
        }
        frequencies.sort();
        
        HopSequence {
            frequencies,
            current_index: 0,
            hops_count: 0,
            hop_interval_ms: interval_ms,
        }
    }

    /// Crée une séquence linéaire
    pub fn new_linear(band: &FrequencyBand, interval_ms: u64) -> Self {
        let step = band.channel_width_khz / 1000;
        let frequencies: Vec<u32> = (band.start_mhz..=band.end_mhz).step_by(step as usize).collect();
        
        HopSequence {
            frequencies,
            current_index: 0,
            hops_count: 0,
            hop_interval_ms: interval_ms,
        }
    }

    /// Passe à la fréquence suivante
    pub fn next_hop(&mut self) -> u32 {
        if self.frequencies.is_empty() {
            return 2442; // Fréquence de secours
        }
        let freq = self.frequencies[self.current_index];
        self.current_index = (self.current_index + 1) % self.frequencies.len();
        self.hops_count += 1;
        freq
    }

    /// Retourne la fréquence actuelle sans avancer
    pub fn current_frequency(&self) -> u32 {
        if self.frequencies.is_empty() { 2442 }
        else { self.frequencies[self.current_index] }
    }

    /// Retourne le nombre de fréquences dans la séquence
    pub fn len(&self) -> usize {
        self.frequencies.len()
    }

    /// Nombre total de sauts effectués
    pub fn get_hops(&self) -> u64 {
        self.hops_count
    }

    /// Intervalle entre sauts
    pub fn get_interval_ms(&self) -> u64 {
        self.hop_interval_ms
    }

    /// Réinitialise la séquence
    pub fn reset(&mut self) {
        self.current_index = 0;
        self.hops_count = 0;
    }
}

// ═══════════════════════════════════════════════════════════════
// GESTIONNAIRE DE SAUT DE FRÉQUENCE
// ═══════════════════════════════════════════════════════════════

pub struct FrequencyHopper {
    /// Bandes disponibles
    bands: Vec<FrequencyBand>,
    /// Bande active
    active_band_index: usize,
    /// Séquence de saut active
    hop_sequence: Option<HopSequence>,
    /// Fréquence actuelle
    current_frequency_mhz: u32,
    /// Mode saut activé
    hopping_enabled: bool,
    /// Temps total d'émission
    total_tx_time_ms: u64,
}

impl FrequencyHopper {
    /// Crée un nouveau gestionnaire
    pub fn new() -> Self {
        let bands = get_default_bands();
        FrequencyHopper {
            bands,
            active_band_index: 0,
            hop_sequence: None,
            current_frequency_mhz: 2442,
            hopping_enabled: false,
            total_tx_time_ms: 0,
        }
    }

    /// Active le saut de fréquence
    pub fn enable_hopping(&mut self, band_index: usize, hop_count: usize, interval_ms: u64) -> Result<(), ChainError> {
        if band_index >= self.bands.len() {
            return Err(ChainError::InternalError("Index de bande invalide".into()));
        }
        
        let band = &self.bands[band_index];
        let sequence = HopSequence::new_random(band, hop_count, interval_ms);
        
        self.hop_sequence = Some(sequence);
        self.active_band_index = band_index;
        self.hopping_enabled = true;
        self.current_frequency_mhz = self.hop_sequence.as_ref().unwrap().current_frequency();
        
        Ok(())
    }

    /// Désactive le saut de fréquence
    pub fn disable_hopping(&mut self) {
        self.hopping_enabled = false;
        self.hop_sequence = None;
    }

    /// Exécute un saut
    pub fn hop(&mut self) -> Result<u32, ChainError> {
        if !self.hopping_enabled || self.hop_sequence.is_none() {
            return Err(ChainError::InternalError("Saut de fréquence non activé".into()));
        }
        
        let seq = self.hop_sequence.as_mut().unwrap();
        let freq = seq.next_hop();
        self.current_frequency_mhz = freq;
        self.total_tx_time_ms += seq.get_interval_ms();
        
        Ok(freq)
    }

    /// Change de bande
    pub fn switch_band(&mut self, band_index: usize, hop_count: usize, interval_ms: u64) -> Result<(), ChainError> {
        self.enable_hopping(band_index, hop_count, interval_ms)
    }

    /// Retourne la fréquence actuelle
    pub fn get_frequency(&self) -> u32 {
        self.current_frequency_mhz
    }

    /// Retourne la bande active
    pub fn get_active_band(&self) -> Option<&FrequencyBand> {
        if self.active_band_index < self.bands.len() {
            Some(&self.bands[self.active_band_index])
        } else {
            None
        }
    }

    /// Vérifie si le saut est activé
    pub fn is_hopping(&self) -> bool {
        self.hopping_enabled
    }

    /// Retourne les bandes disponibles
    pub fn get_bands(&self) -> &[FrequencyBand] {
        &self.bands
    }

    /// Retourne les statistiques
    pub fn get_stats(&self) -> HopStats {
        HopStats {
            hopping_enabled: self.hopping_enabled,
            current_frequency_mhz: self.current_frequency_mhz,
            active_band: self.get_active_band().map(|b| b.name.clone()).unwrap(),
            total_hops: self.hop_sequence.as_ref().map(|s| s.get_hops()).unwrap_or(0),
            sequence_length: self.hop_sequence.as_ref().map(|s| s.len()).unwrap_or(0),
            total_tx_time_ms: self.total_tx_time_ms,
            available_bands: self.bands.len(),
        }
    }
}

/// Statistiques de saut
#[derive(Debug, Clone)]
pub struct HopStats {
    pub hopping_enabled: bool,
    pub current_frequency_mhz: u32,
    pub active_band: String,
    pub total_hops: u64,
    pub sequence_length: usize,
    pub total_tx_time_ms: u64,
    pub available_bands: usize,
}

// ═══════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    // ─── Tests FrequencyBand ───────────────────

    #[test] fn test_band_channel_count() {
        let band = FrequencyBand { name: "Test".into(), start_mhz: 2400, end_mhz: 2483, is_ism: true, max_power_dbm: 0, channel_width_khz: 1000 };
        assert_eq!(band.channel_count(), 83);
    }

    #[test] fn test_band_contains() {
        let band = FrequencyBand { name: "Test".into(), start_mhz: 2400, end_mhz: 2483, is_ism: true, max_power_dbm: 0, channel_width_khz: 1000 };
        assert!(band.contains(2442));
        assert!(!band.contains(5000));
    }

    #[test] fn test_default_bands() {
        let bands = get_default_bands();
        assert_eq!(bands.len(), 5);
        assert!(bands.iter().all(|b| b.is_ism));
    }

    // ─── Tests HopSequence ─────────────────────

    #[test] fn test_sequence_new_random() {
        let band = FrequencyBand { name: "Test".into(), start_mhz: 2400, end_mhz: 2483, is_ism: true, max_power_dbm: 0, channel_width_khz: 1000 };
        let seq = HopSequence::new_random(&band, 10, 100);
        assert_eq!(seq.len(), 10, "La séquence doit avoir exactement 10 fréquences");
        assert_eq!(seq.get_interval_ms(), 100);
    }

    #[test] fn test_sequence_new_linear() {
        let band = FrequencyBand { name: "Test".into(), start_mhz: 2400, end_mhz: 2403, is_ism: true, max_power_dbm: 0, channel_width_khz: 1000 };
        let seq = HopSequence::new_linear(&band, 100);
        assert_eq!(seq.len(), 4); // 2400, 2401, 2402, 2403
    }

    #[test] fn test_sequence_next_hop() {
        let band = FrequencyBand { name: "Test".into(), start_mhz: 2400, end_mhz: 2483, is_ism: true, max_power_dbm: 0, channel_width_khz: 1000 };
        let mut seq = HopSequence::new_random(&band, 3, 100);
        let f1 = seq.next_hop();
        seq.next_hop();
        seq.next_hop();
        let f4 = seq.next_hop(); // Retour au début
        assert_eq!(f1, f4);
        assert_eq!(seq.get_hops(), 4);
    }

    #[test] fn test_sequence_current() {
        let band = FrequencyBand { name: "Test".into(), start_mhz: 2400, end_mhz: 2483, is_ism: true, max_power_dbm: 0, channel_width_khz: 1000 };
        let mut seq = HopSequence::new_random(&band, 5, 100);
        let current = seq.current_frequency();
        assert_eq!(seq.next_hop(), current); // Le premier hop retourne la fréquence actuelle
    }

    #[test] fn test_sequence_reset() {
        let band = FrequencyBand { name: "Test".into(), start_mhz: 2400, end_mhz: 2483, is_ism: true, max_power_dbm: 0, channel_width_khz: 1000 };
        let mut seq = HopSequence::new_random(&band, 3, 100);
        seq.next_hop();
        seq.next_hop();
        seq.reset();
        assert_eq!(seq.get_hops(), 0);
    }

    // ─── Tests FrequencyHopper ─────────────────

    #[test] fn test_hopper_new() {
        let fh = FrequencyHopper::new();
        assert!(!fh.is_hopping());
        assert_eq!(fh.get_frequency(), 2442);
        assert_eq!(fh.get_bands().len(), 5);
    }

    #[test] fn test_enable_hopping() {
        let mut fh = FrequencyHopper::new();
        assert!(fh.enable_hopping(0, 10, 100).is_ok());
        assert!(fh.is_hopping());
    }

    #[test] fn test_enable_invalid_band() {
        let mut fh = FrequencyHopper::new();
        assert!(fh.enable_hopping(99, 10, 100).is_err());
    }

    #[test] fn test_hop() {
        let mut fh = FrequencyHopper::new();
        fh.enable_hopping(0, 10, 100).unwrap();
        let freq = fh.hop().unwrap();
        assert!(freq >= 2400 && freq <= 2483);
    }

    #[test] fn test_hop_without_enabling() {
        let mut fh = FrequencyHopper::new();
        assert!(fh.hop().is_err());
    }

    #[test] fn test_disable_hopping() {
        let mut fh = FrequencyHopper::new();
        fh.enable_hopping(0, 10, 100).unwrap();
        fh.disable_hopping();
        assert!(!fh.is_hopping());
    }

    #[test] fn test_switch_band() {
        let mut fh = FrequencyHopper::new();
        fh.switch_band(1, 10, 100).unwrap(); // ISM 5 GHz
        let band = fh.get_active_band().unwrap();
        assert!(band.name.contains("5 GHz"));
    }

    #[test] fn test_stats() {
        let mut fh = FrequencyHopper::new();
        fh.enable_hopping(0, 10, 100).unwrap();
        fh.hop().unwrap();
        let stats = fh.get_stats();
        assert!(stats.hopping_enabled);
        assert_eq!(stats.available_bands, 5);
        assert!(stats.total_hops > 0);
    }
}



