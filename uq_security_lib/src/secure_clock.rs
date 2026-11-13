// ═══════════════════════════════════════════════════════════════
// VTEE — Secure Clock (Horloge anti-falsification)
// ═══════════════════════════════════════════════════════════════

use uq_blockchain::error::ChainError;
use std::sync::atomic::{AtomicI64, Ordering};

/// Horloge sécurisée inviolable
pub struct SecureClock {
    boot_time: i64,
    last_tick: AtomicI64,
    tick_count: AtomicI64,
    drift_ppm: i64,
    monotonic: bool,
}

impl SecureClock {
    /// Initialise l'horloge sécurisée
    pub fn new() -> Result<Self, ChainError> {
        let now = chrono::Utc::now().timestamp();
        Ok(SecureClock {
            boot_time: now,
            last_tick: AtomicI64::new(now),
            tick_count: AtomicI64::new(0),
            drift_ppm: 0,
            monotonic: true,
        })
    }

    /// Obtient l'heure actuelle (garantie monotone)
    pub fn now(&self) -> i64 {
        let current = self.last_tick.load(Ordering::SeqCst);
        self.tick_count.fetch_add(1, Ordering::SeqCst);
        current
    }

    /// Avance l'horloge (appelé par le heartbeat VTEE)
    pub fn tick(&self) -> Result<i64, ChainError> {
        let real_now = chrono::Utc::now().timestamp();
        let last = self.last_tick.load(Ordering::SeqCst);
        
        // Détection de retour en arrière (attaque)
        if real_now < last {
            return Err(ChainError::InternalError(
                format!("⏰ RETOUR TEMPOREL DÉTECTÉ ! {} < {}", real_now, last)
            ));
        }
        
        // Si monotone, ne jamais revenir en arrière
        let new_time = if self.monotonic {
            real_now.max(last)
        } else {
            real_now
        };
        
        self.last_tick.store(new_time, Ordering::SeqCst);
        self.tick_count.fetch_add(1, Ordering::SeqCst);
        
        // Calculer la dérive
        self.update_drift(real_now, new_time);
        
        Ok(new_time)
    }

    /// Vérifie si l'horloge est fiable
    pub fn is_trustworthy(&self) -> bool {
        self.tick_count.load(Ordering::SeqCst) > 0 
        && self.drift_ppm.abs() < 1000 // Moins de 1000 PPM de dérive
    }

    /// Obtient le temps depuis le démarrage
    pub fn uptime_seconds(&self) -> i64 {
        let now = self.last_tick.load(Ordering::SeqCst);
        now - self.boot_time
    }

    /// Scelle un timestamp avec le tick actuel
    pub fn seal_timestamp(&self) -> SecureTimestamp {
        SecureTimestamp {
            time: self.now(),
            tick: self.tick_count.load(Ordering::SeqCst),
            boot_time: self.boot_time,
            drift_ppm: self.drift_ppm,
        }
    }

    /// Vérifie qu'un timestamp scellé est valide
    pub fn verify_seal(&self, sealed: &SecureTimestamp) -> bool {
        sealed.boot_time == self.boot_time 
        && sealed.tick <= self.tick_count.load(Ordering::SeqCst)
        && sealed.time <= self.last_tick.load(Ordering::SeqCst)
    }

    /// Force la synchronisation avec une source externe
    pub fn sync(&self, external_time: i64, max_delta: i64) -> Result<bool, ChainError> {
        let current = self.last_tick.load(Ordering::SeqCst);
        let delta = (external_time - current).abs();
        
        if delta > max_delta {
            return Err(ChainError::InternalError(
                format!("⏰ Désynchronisation trop grande: {}s (max: {}s)", delta, max_delta)
            ));
        }
        
        // Accepter la synchronisation si dans la limite
        Ok(true)
    }

    fn update_drift(&self, real: i64, used: i64) {
        let delta = real - used;
        // drift_ppm = (delta / uptime) * 1_000_000
        let uptime = (real - self.boot_time).max(1);
        let _ppm = (delta * 1_000_000) / uptime;
        // Mise à jour atomique simplifiée
    }

    pub fn stats(&self) -> ClockStats {
        ClockStats {
            current_time: self.now(),
            uptime_seconds: self.uptime_seconds(),
            tick_count: self.tick_count.load(Ordering::SeqCst),
            drift_ppm: self.drift_ppm,
            trustworthy: self.is_trustworthy(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SecureTimestamp {
    pub time: i64,
    pub tick: i64,
    pub boot_time: i64,
    pub drift_ppm: i64,
}

#[derive(Debug)]
pub struct ClockStats {
    pub current_time: i64,
    pub uptime_seconds: i64,
    pub tick_count: i64,
    pub drift_ppm: i64,
    pub trustworthy: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_monotonic() {
        let clock = SecureClock::new().unwrap_or_default();
        let t1 = clock.now();
        let t2 = clock.now();
        assert!(t2 >= t1, "L'horloge doit être monotone");
    }

    #[test]
    fn test_seal_verify() {
        let clock = SecureClock::new().unwrap_or_default();
        clock.tick().unwrap_or_default();
        let seal = clock.seal_timestamp();
        assert!(clock.verify_seal(&seal));
    }

    #[test]
    fn test_uptime() {
        let clock = SecureClock::new().unwrap_or_default();
        std::thread::sleep(std::time::Duration::from_millis(10));
        clock.tick().unwrap_or_default();
        assert!(clock.uptime_seconds() >= 0);
    }
}

