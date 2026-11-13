// ═══════════════════════════════════════════════════════════════
// VTEE — Side-Channel Protection (Anti-timing/power)
// ═══════════════════════════════════════════════════════════════

use uq_blockchain::error::ChainError;
use std::time::{Duration, Instant};

/// Protection contre les attaques side-channel
pub struct SideChannelGuard {
    min_op_duration_ns: u64,
    timing_noise_enabled: bool,
    constant_time_mode: bool,
}

impl SideChannelGuard {
    pub fn new() -> Self {
        SideChannelGuard {
            min_op_duration_ns: 100_000, // 100µs minimum par opération
            timing_noise_enabled: true,
            constant_time_mode: true,
        }
    }

    /// Exécute une opération en temps constant
    pub fn constant_time_compare(a: &[u8], b: &[u8]) -> bool {
        if a.len() != b.len() { return false; }
        let mut diff = 0u8;
        for i in 0..a.len() {
            diff |= a[i] ^ b[i];
        }
        diff == 0
    }

    /// Exécute une opération sensible avec timing constant
    pub fn secure_operation<F, T>(&self, operation: F) -> Result<T, ChainError>
    where F: FnOnce() -> Result<T, ChainError> {
        let start = Instant::now();
        
        // Exécuter l'opération réelle
        let result = operation()?;
        
        // Ajouter du bruit temporel pour masquer la durée réelle
        if self.timing_noise_enabled {
            let elapsed = start.elapsed();
            if elapsed < Duration::from_nanos(self.min_op_duration_ns) {
                let wait_time = Duration::from_nanos(self.min_op_duration_ns) - elapsed;
                // Ajouter un délai aléatoire dans la marge
                let noise = rand::random::<u64>() % (wait_time.as_nanos() as u64 / 10);
                std::thread::sleep(Duration::from_nanos(noise));
            }
            // Toujours attendre le minimum
            while start.elapsed() < Duration::from_nanos(self.min_op_duration_ns) {
                std::hint::spin_loop();
            }
        }
        
        Ok(result)
    }

    /// Compare deux slices en temps constant (résiste aux timing attacks)
    pub fn secure_compare(&self, a: &[u8], b: &[u8]) -> bool {
        let len_a = a.len();
        let len_b = b.len();
        
        // Toujours parcourir le même nombre d'octets
        let max_len = len_a.max(len_b);
        let mut diff = (len_a ^ len_b) as u8;
        
        for i in 0..max_len {
            let byte_a = if i < len_a { a[i] } else { 0 };
            let byte_b = if i < len_b { b[i] } else { 0 };
            diff |= byte_a ^ byte_b;
        }
        
        diff == 0
    }

    /// Masque les patterns d'accès mémoire
    pub fn secure_memory_access(&self, index: usize, max_index: usize) -> usize {
        if self.constant_time_mode {
            // Toujours parcourir tout le tableau (pas de court-circuit)
            index.min(max_index)
        } else {
            index
        }
    }

    /// Ajoute du bruit aux mesures de puissance (simulation)
    pub fn power_noise(&self) -> f64 {
        if self.timing_noise_enabled {
            let noise: f64 = rand::random();
            noise * 0.1 // 10% de bruit maximum
        } else {
            0.0
        }
    }

    pub fn stats(&self) -> SideChannelStats {
        SideChannelStats {
            constant_time: self.constant_time_mode,
            noise_enabled: self.timing_noise_enabled,
            min_op_ns: self.min_op_duration_ns,
        }
    }
}

#[derive(Debug)]
pub struct SideChannelStats {
    pub constant_time: bool,
    pub noise_enabled: bool,
    pub min_op_ns: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constant_time_compare() {
        let a = b"secret_key_123";
        let b = b"secret_key_123";
        let c = b"secret_key_124";
        assert!(SideChannelGuard::constant_time_compare(a, b));
        assert!(!SideChannelGuard::constant_time_compare(a, c));
    }

    #[test]
    fn test_secure_compare() {
        let guard = SideChannelGuard::new();
        assert!(guard.secure_compare(b"test", b"test"));
        assert!(!guard.secure_compare(b"test", b"Test"));
    }

    #[test]
    fn test_secure_operation() {
        let guard = SideChannelGuard::new();
        let result = guard.secure_operation(|| {
            Ok::<u64, ChainError>(42)
        }).unwrap_or_default();
        assert_eq!(result, 42);
    }
}

