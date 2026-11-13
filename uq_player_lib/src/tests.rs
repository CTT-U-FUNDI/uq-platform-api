#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::UqPlayer;
    use crate::cache::ContentCache;
    use crate::payment::PaymentSplitter;
    
    #[test]
    fn test_payment_split_95_5() {
        let split = PaymentSplitter::split(100);
        assert_eq!(split.creator_amount, 95);
        assert_eq!(split.uq_amount, 5);
    }
    
    #[test]
    fn test_player_full_flow() {
        let mut player = UqPlayer::new();
        let creator = [1u8; 32];
        
        // Démarrer : 0.01 BORB/seconde, max 10 minutes
        player.start(creator, 1, 600).unwrap();
        
        // Simuler 3 minutes de lecture (180 secondes)
        for _ in 0..180 {
            player.tick().unwrap();
        }
        
        // Arrêter
        let total = player.stop().unwrap();
        
        // 180 secondes × 1 point = 180 points
        assert_eq!(total, 180);
    }
    
    #[test]
    fn test_cache_system() {
        let mut cache = ContentCache::new();
        let url = "https://youtube.com/watch?v=abc123";
        
        // Pas encore payé
        assert!(!cache.is_paid(url));
        
        // Marquer comme payé
        cache.mark_paid(url);
        
        // Maintenant payé
        assert!(cache.is_paid(url));
        assert_eq!(cache.cache_size(), 1);
    }
    
    #[test]
    fn test_cache_hash_unique() {
        let hash1 = ContentCache::hash_content("video1");
        let hash2 = ContentCache::hash_content("video2");
        assert_ne!(hash1, hash2);
    }
}
