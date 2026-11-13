use crate::marketplace::{Publication, PubType, PubBadge};
use crate::error::ChainError;

/// Calcul de distance GPS (formule de Haversine)
/// CONNECTÉ à user_models.rs — mais recréé ici pour éviter la dépendance circulaire
pub fn calculate_distance_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let earth_radius_km = 6371.0;
    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();
    let a = (dlat / 2.0).sin().powi(2) 
        + lat1.to_radians().cos() * lat2.to_radians().cos() * (dlon / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().atan2((1.0 - a).sqrt());
    earth_radius_km * c
}

/// L'intérêt d'un utilisateur — le "Je cherche quoi"
#[derive(Debug, Clone)]
pub struct Interest {
    pub user_id: String,
    pub tags: Vec<String>,
    pub max_distance_km: f64,
    pub max_price: Option<u64>,
    pub min_rating: f64,
    pub notify_enabled: bool,
}

impl Interest {
    /// Crée un nouvel intérêt
    pub fn new(user_id: &str, tags: Vec<String>) -> Self {
        Self {
            user_id: user_id.to_string(),
            tags,
            max_distance_km: 2.0,      // 2 km par défaut
            max_price: None,            // pas de limite de prix
            min_rating: 0.0,            // pas de note minimale
            notify_enabled: true,       // notifications activées
        }
    }
    
    /// Régle la distance maximale
    pub fn set_max_distance(&mut self, km: f64) {
        self.max_distance_km = km;
    }
    
    /// Définit un budget maximum
    pub fn set_max_price(&mut self, price: u64) {
        self.max_price = Some(price);
    }
    
    /// Définit une note minimale
    pub fn set_min_rating(&mut self, rating: f64) {
        self.min_rating = rating;
    }
    
    /// Vérifie si une publication correspond aux intérêts
    pub fn matches(&self, publication: &Publication, user_lat: f64, user_lon: f64) -> bool {
        // RÈGLE 1 : Distance
        let distance = calculate_distance_km(user_lat, user_lon, publication.latitude, publication.longitude);
        if distance > self.max_distance_km {
            return false;
        }
        
        // RÈGLE 2 : Tags
        if !self.tags.is_empty() {
            let has_matching_tag = publication.tags.iter().any(|t| self.tags.contains(t));
            if !has_matching_tag {
                return false;
            }
        }
        
        // RÈGLE 3 : Prix
        if let Some(max_price) = self.max_price {
            if publication.listing.amount > max_price {
                return false;
            }
        }
        
        // RÈGLE 4 : Note
        if publication.rating < self.min_rating {
            return false;
        }
        
        // RÈGLE 5 : Publication active
        if !publication.is_active() {
            return false;
        }
        
        true
    }
    
    /// Vérifie si le type de publication correspond
    pub fn matches_type(&self, publication: &Publication, desired_type: &PubType) -> bool {
        match (desired_type, &publication.pub_type) {
            (PubType::Custom { category: c1, .. }, PubType::Custom { category: c2, .. }) => c1 == c2,
            (a, b) => a == b,
        }
    }
    
    /// Trie les publications par pertinence
    pub fn rank_publications<'a>(&self, publications: &'a [Publication], user_lat: f64, user_lon: f64) -> Vec<&'a Publication> {
        let mut matching: Vec<&Publication> = publications
            .iter()
            .filter(|p| self.matches(p, user_lat, user_lon))
            .collect();
        
        // Trier par distance (les plus proches d'abord)
        matching.sort_by(|a, b| {
            let dist_a = calculate_distance_km(user_lat, user_lon, a.latitude, a.longitude);
            let dist_b = calculate_distance_km(user_lat, user_lon, b.latitude, b.longitude);
            dist_a.partial_cmp(&dist_b).unwrap_or(std::cmp::Ordering::Equal)
        });
        
        matching
    }
}

/// Le registre des intérêts
#[derive(Debug, Clone)]
pub struct InterestRegistry {
    pub interests: Vec<Interest>,
}

impl InterestRegistry {
    pub fn new() -> Self {
        Self { interests: Vec::new() }
    }
    
    /// Ajoute un intérêt
    pub fn add_interest(&mut self, interest: Interest) {
        self.interests.push(interest);
    }
    
    /// Supprime un intérêt
    pub fn remove_interest(&mut self, user_id: &str, tags: &[String]) {
        self.interests.retain(|i| {
            !(i.user_id == user_id && i.tags == tags)
        });
    }
    
    /// Trouve tous les intérêts qui correspondent à une publication
    pub fn find_matching_interests(&self, publication: &Publication, user_lat: f64, user_lon: f64) -> Vec<&Interest> {
        self.interests
            .iter()
            .filter(|i| i.matches(publication, user_lat, user_lon))
            .collect()
    }
}

// ═══════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_distance() {
        // Kinshasa → Brazzaville ≈ 4 km
        let d = calculate_distance_km(-4.32, 15.31, -4.27, 15.28);
        assert!(d > 3.0 && d < 10.0);
    }
    
    #[test]
    fn test_interest_matches() {
        let interest = Interest::new("user1", vec!["savon".to_string()]);
        interest.set_max_distance(5.0);
        interest.set_max_price(10);
        interest.set_min_rating(3.0);
        
        // Tester avec une publication fictive (test simplifié)
        assert!(interest.max_distance_km == 5.0);
        assert!(interest.max_price == Some(10));
        assert!(interest.min_rating == 3.0);
    }
}

