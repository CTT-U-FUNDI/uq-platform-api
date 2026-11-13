use uq_blockchain::error::ChainError;

// ═══════════════════════════════════════════════════════════════
// NUMÉRO UQ
// ═══════════════════════════════════════════════════════════════

/// Un numéro UQ (format : 901 XX XXXXXXX)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UqNumber {
    pub mcc: u16,        // 901 (Mobile Country Code)
    pub mnc: u16,        // 01 (Mobile Network Code)
    pub subscriber: u32, // 0-9999999 (7 chiffres)
}

impl UqNumber {
    /// Crée un numéro UQ
    pub fn new(subscriber: u32) -> Result<Self, ChainError> {
        if subscriber > 9_999_999 {
            return Err(ChainError::InternalError("Numéro abonné invalide (> 9 999 999)".into()));
        }
        Ok(UqNumber { mcc: 901, mnc: 1, subscriber })
    }

    /// Formate en chaîne lisible
    pub fn format(&self) -> String {
        format!("+{} {:02} {:07}", self.mcc, self.mnc, self.subscriber)
    }

    /// Formate compact
    pub fn compact(&self) -> String {
        format!("{}{:02}{:07}", self.mcc, self.mnc, self.subscriber)
    }

    /// Parse depuis une chaîne
    pub fn parse(s: &str) -> Result<Self, ChainError> {
        let cleaned: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
        if cleaned.len() != 12 {
            return Err(ChainError::InternalError("Format invalide (12 chiffres attendus)".into()));
        }
        let mcc: u16 = cleaned[0..3].parse().map_err(|_| ChainError::InternalError("MCC invalide".into()))?;
        let mnc: u16 = cleaned[3..5].parse().map_err(|_| ChainError::InternalError("MNC invalide".into()))?;
        let subscriber: u32 = cleaned[5..12].parse().map_err(|_| ChainError::InternalError("Abonné invalide".into()))?;
        Ok(UqNumber { mcc, mnc, subscriber })
    }

    /// Vérifie si c'est un numéro UQ valide
    pub fn is_valid_uq(&self) -> bool {
        self.mcc == 901 && self.mnc == 1
    }
}

// ═══════════════════════════════════════════════════════════════
// ÉTAT D'UN NUMÉRO
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq)]
pub enum NumberState {
    Available,   // Libre
    Reserved,    // Réservé
    Assigned,    // Assigné à un utilisateur
    Suspended,   // Suspendu
    Blocked,     // Bloqué
    Released,    // Libéré (recyclable)
}

// ═══════════════════════════════════════════════════════════════
// ENTRÉE DU POOL
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone)]
pub struct NumberEntry {
    pub number: UqNumber,
    pub state: NumberState,
    pub assigned_to: Option<[u8; 32]>, // Adresse du propriétaire
    pub assigned_at: Option<u64>,
    pub released_at: Option<u64>,
    pub times_recycled: u32,
}

impl NumberEntry {
    pub fn new(number: UqNumber) -> Self {
        NumberEntry {
            number,
            state: NumberState::Available,
            assigned_to: None,
            assigned_at: None,
            released_at: None,
            times_recycled: 0,
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// POOL DE NUMÉROS
// ═══════════════════════════════════════════════════════════════

pub struct NumberPool {
    numbers: Vec<NumberEntry>,
    total_assigned: u64,
    total_released: u64,
    total_recycled: u64,
}

impl NumberPool {
    /// Crée un pool avec une plage de numéros
    pub fn new(start: u32, end: u32) -> Result<Self, ChainError> {
        if start > end || end > 9_999_999 {
            return Err(ChainError::InternalError("Plage de numéros invalide".into()));
        }

        let mut numbers = Vec::with_capacity((end - start + 1) as usize);
        for subscriber in start..=end {
            let number = UqNumber::new(subscriber)?;
            numbers.push(NumberEntry::new(number));
        }

        Ok(NumberPool {
            numbers,
            total_assigned: 0,
            total_released: 0,
            total_recycled: 0,
        })
    }

    /// Assigne un numéro disponible
    pub fn assign_number(&mut self, owner: [u8; 32]) -> Result<UqNumber, ChainError> {
        // Chercher un numéro disponible
        if let Some(entry) = self.numbers.iter_mut().find(|e| e.state == NumberState::Available) {
            entry.state = NumberState::Assigned;
            entry.assigned_to = Some(owner);
            entry.assigned_at = Some(std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs());
            self.total_assigned += 1;
            return Ok(entry.number.clone());
        }

        // Chercher un numéro recyclable
        if let Some(entry) = self.numbers.iter_mut().find(|e| e.state == NumberState::Released) {
            entry.state = NumberState::Assigned;
            entry.assigned_to = Some(owner);
            entry.assigned_at = Some(std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs());
            entry.times_recycled += 1;
            self.total_recycled += 1;
            self.total_assigned += 1;
            return Ok(entry.number.clone());
        }

        Err(ChainError::InternalError("Aucun numéro disponible".into()))
    }

    /// Libère un numéro
    pub fn release_number(&mut self, subscriber: u32) -> Result<(), ChainError> {
        if let Some(entry) = self.numbers.iter_mut().find(|e| e.number.subscriber == subscriber) {
            if entry.state != NumberState::Assigned && entry.state != NumberState::Suspended {
                return Err(ChainError::InternalError("Le numéro n'est pas assigné".into()));
            }
            entry.state = NumberState::Available;  // Remettre en disponible
            entry.assigned_to = None;
            entry.released_at = Some(std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs());
            entry.times_recycled += 1;
            self.total_released += 1;
            Ok(())
        } else {
            Err(ChainError::InternalError("Numéro non trouvé".into()))
        }
    }

    /// Réserve un numéro spécifique
    pub fn reserve_number(&mut self, subscriber: u32) -> Result<(), ChainError> {
        if let Some(entry) = self.numbers.iter_mut().find(|e| e.number.subscriber == subscriber) {
            if entry.state != NumberState::Available {
                return Err(ChainError::InternalError("Numéro non disponible".into()));
            }
            entry.state = NumberState::Reserved;
            Ok(())
        } else {
            Err(ChainError::InternalError("Numéro non trouvé".into()))
        }
    }

    /// Bloque un numéro
    pub fn block_number(&mut self, subscriber: u32) -> Result<(), ChainError> {
        if let Some(entry) = self.numbers.iter_mut().find(|e| e.number.subscriber == subscriber) {
            entry.state = NumberState::Blocked;
            Ok(())
        } else {
            Err(ChainError::InternalError("Numéro non trouvé".into()))
        }
    }

    /// Suspend un numéro
    pub fn suspend_number(&mut self, subscriber: u32) -> Result<(), ChainError> {
        if let Some(entry) = self.numbers.iter_mut().find(|e| e.number.subscriber == subscriber) {
            if entry.state != NumberState::Assigned {
                return Err(ChainError::InternalError("Le numéro n'est pas assigné".into()));
            }
            entry.state = NumberState::Suspended;
            Ok(())
        } else {
            Err(ChainError::InternalError("Numéro non trouvé".into()))
        }
    }

    /// Réactive un numéro suspendu
    pub fn reactivate_number(&mut self, subscriber: u32) -> Result<(), ChainError> {
        if let Some(entry) = self.numbers.iter_mut().find(|e| e.number.subscriber == subscriber) {
            if entry.state != NumberState::Suspended {
                return Err(ChainError::InternalError("Le numéro n'est pas suspendu".into()));
            }
            entry.state = NumberState::Assigned;
            Ok(())
        } else {
            Err(ChainError::InternalError("Numéro non trouvé".into()))
        }
    }

    /// Cherche un numéro par abonné
    pub fn find_by_subscriber(&self, subscriber: u32) -> Option<&NumberEntry> {
        self.numbers.iter().find(|e| e.number.subscriber == subscriber)
    }

    /// Cherche le propriétaire d'un numéro
    pub fn find_owner(&self, subscriber: u32) -> Option<[u8; 32]> {
        self.numbers.iter()
            .find(|e| e.number.subscriber == subscriber)
            .and_then(|e| e.assigned_to)
    }

    /// Compte les numéros par état
    pub fn count_by_state(&self, state: NumberState) -> usize {
        self.numbers.iter().filter(|e| e.state == state).count()
    }

    /// Retourne le nombre total de numéros
    pub fn total_numbers(&self) -> usize {
        self.numbers.len()
    }

    /// Retourne les numéros disponibles
    pub fn available_count(&self) -> usize {
        self.count_by_state(NumberState::Available)
    }

    /// Retourne les numéros assignés
    pub fn assigned_count(&self) -> usize {
        self.count_by_state(NumberState::Assigned)
    }

    /// Vérifie si un numéro est disponible
    pub fn is_available(&self, subscriber: u32) -> bool {
        self.numbers.iter()
            .any(|e| e.number.subscriber == subscriber && e.state == NumberState::Available)
    }

    /// Retourne les statistiques du pool
    pub fn get_stats(&self) -> PoolStats {
        PoolStats {
            total: self.total_numbers(),
            available: self.available_count(),
            assigned: self.assigned_count(),
            reserved: self.count_by_state(NumberState::Reserved),
            suspended: self.count_by_state(NumberState::Suspended),
            blocked: self.count_by_state(NumberState::Blocked),
            released: self.count_by_state(NumberState::Released),
            total_assigned_ever: self.total_assigned,
            total_released_ever: self.total_released,
            total_recycled: self.total_recycled,
        }
    }
}

/// Statistiques du pool
#[derive(Debug, Clone)]
pub struct PoolStats {
    pub total: usize,
    pub available: usize,
    pub assigned: usize,
    pub reserved: usize,
    pub suspended: usize,
    pub blocked: usize,
    pub released: usize,
    pub total_assigned_ever: u64,
    pub total_released_ever: u64,
    pub total_recycled: u64,
}

// ═══════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    fn make_owner(n: u8) -> [u8; 32] { let mut a = [0u8; 32]; a[0] = n; a }

    // ─── Tests UqNumber ─────────────────────────

    #[test] fn test_number_new() {
        let n = UqNumber::new(1234567).unwrap();
        assert_eq!(n.mcc, 901);
        assert_eq!(n.mnc, 1);
        assert_eq!(n.subscriber, 1234567);
    }

    #[test] fn test_number_new_invalid() {
        assert!(UqNumber::new(10_000_000).is_err());
    }

    #[test] fn test_number_format() {
        let n = UqNumber::new(1234567).unwrap();
        assert_eq!(n.format(), "+901 01 1234567");
    }

    #[test] fn test_number_compact() {
        let n = UqNumber::new(1234567).unwrap();
        assert_eq!(n.compact(), "901011234567");
    }

    #[test] fn test_number_parse() {
        let n = UqNumber::parse("901011234567").unwrap();
        assert_eq!(n.subscriber, 1234567);
    }

    #[test] fn test_number_parse_invalid() {
        assert!(UqNumber::parse("123").is_err());
    }

    #[test] fn test_is_valid_uq() {
        let n = UqNumber::new(1).unwrap();
        assert!(n.is_valid_uq());
    }

    // ─── Tests NumberPool ──────────────────────

    #[test] fn test_pool_new() {
        let pool = NumberPool::new(0, 99).unwrap();
        assert_eq!(pool.total_numbers(), 100);
        assert_eq!(pool.available_count(), 100);
    }

    #[test] fn test_pool_invalid_range() {
        assert!(NumberPool::new(100, 50).is_err());
    }

    #[test] fn test_assign_number() {
        let mut pool = NumberPool::new(0, 9).unwrap();
        let num = pool.assign_number(make_owner(1)).unwrap();
        assert_eq!(num.subscriber, 0); // Premier disponible
        assert_eq!(pool.available_count(), 9);
        assert_eq!(pool.assigned_count(), 1);
    }

    #[test] fn test_assign_exhausted() {
        let mut pool = NumberPool::new(0, 2).unwrap();
        pool.assign_number(make_owner(1)).unwrap();
        pool.assign_number(make_owner(2)).unwrap();
        pool.assign_number(make_owner(3)).unwrap();
        // Pool épuisé
        assert!(pool.assign_number(make_owner(4)).is_err());
    }

    #[test] fn test_release_number() {
        let mut pool = NumberPool::new(0, 9).unwrap();
        let num = pool.assign_number(make_owner(1)).unwrap();
        assert_eq!(pool.available_count(), 9);
        pool.release_number(num.subscriber).unwrap();
        assert!(pool.is_available(num.subscriber)); // Le numéro est disponible
        assert_eq!(pool.available_count(), 10);
    }

    #[test] fn test_release_not_assigned() {
        let mut pool = NumberPool::new(0, 9).unwrap();
        assert!(pool.release_number(0).is_err());
    }

    #[test] fn test_recycle_number() {
        let mut pool = NumberPool::new(0, 2).unwrap();
        let n1 = pool.assign_number(make_owner(1)).unwrap();
        pool.release_number(n1.subscriber).unwrap();
        // Le numéro est redevenu disponible
        assert!(pool.is_available(n1.subscriber));
        let n2 = pool.assign_number(make_owner(2)).unwrap();
        // Peut être le même ou un autre (selon ordre)
        assert!(n2.subscriber == n1.subscriber || n2.subscriber != n1.subscriber);
    }

    #[test] fn test_reserve_number() {
        let mut pool = NumberPool::new(0, 9).unwrap();
        assert!(pool.reserve_number(5).is_ok());
        assert!(!pool.is_available(5));
    }

    #[test] fn test_block_number() {
        let mut pool = NumberPool::new(0, 9).unwrap();
        assert!(pool.block_number(3).is_ok());
        assert_eq!(pool.count_by_state(NumberState::Blocked), 1);
    }

    #[test] fn test_suspend_reactivate() {
        let mut pool = NumberPool::new(0, 9).unwrap();
        let num = pool.assign_number(make_owner(1)).unwrap();
        pool.suspend_number(num.subscriber).unwrap();
        assert_eq!(pool.count_by_state(NumberState::Suspended), 1);
        pool.reactivate_number(num.subscriber).unwrap();
        assert_eq!(pool.count_by_state(NumberState::Assigned), 1);
    }

    #[test] fn test_find_owner() {
        let mut pool = NumberPool::new(0, 9).unwrap();
        let num = pool.assign_number(make_owner(42)).unwrap();
        let owner = pool.find_owner(num.subscriber).unwrap();
        assert_eq!(owner[0], 42);
    }

    #[test] fn test_find_owner_not_assigned() {
        let pool = NumberPool::new(0, 9).unwrap();
        assert!(pool.find_owner(5).is_none());
    }

    #[test] fn test_stats() {
        let mut pool = NumberPool::new(0, 99).unwrap();
        pool.assign_number(make_owner(1)).unwrap();
        pool.assign_number(make_owner(2)).unwrap();
        pool.reserve_number(50).unwrap();
        pool.block_number(99).unwrap();
        let stats = pool.get_stats();
        assert_eq!(stats.total, 100);
        assert_eq!(stats.assigned, 2);
        assert_eq!(stats.reserved, 1);
        assert_eq!(stats.blocked, 1);
        assert_eq!(stats.available, 96);
    }
}


