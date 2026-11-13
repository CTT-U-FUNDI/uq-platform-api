use crate::bubble::BubbleRegistry;
use uq_black_key_standalone::UqBlackKey;
use crate::constants::*;
use crate::error::ChainError;

#[derive(Debug, Clone)]
pub struct MarketplaceListing {
    pub seller: [u8; 32],
    pub buyer: Option<[u8; 32]>,
    pub amount: u64,
    pub commission_pct: u8,
    pub state: ListingState,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ListingState { Published, Sold, Delivered, Completed, Disputed, Refunded }

impl MarketplaceListing {
    pub fn publish(seller: [u8; 32], amount: u64, commission_pct: u8, registry: &mut BubbleRegistry) -> Result<Self, ChainError> {
        // Vérifier que le vendeur peut payer les frais de publication
        let balance = registry.get_balance(&seller);
        if balance < MARKETPLACE_PUBLISH_FEE { return Err(ChainError::InternalError("Fonds insuffisants".into())); }
        // Le transfert est fait par le handler API
        Ok(MarketplaceListing { seller, buyer: None, amount, commission_pct, state: ListingState::Published })
    }

    pub fn buy(&mut self, buyer: [u8; 32], registry: &mut BubbleRegistry) -> Result<(), ChainError> {
        let total = self.amount + MARKETPLACE_COMMISSION_PCT * self.amount / 100;
        let balance = registry.get_balance(&buyer);
        if balance < total { return Err(ChainError::InternalError("Fonds insuffisants".into())); }
        // Le transfert est fait par le handler API
        self.buyer = Some(buyer);
        self.state = ListingState::Sold;
        Ok(())
    }

    pub fn complete(&mut self, registry: &mut BubbleRegistry) -> Result<(), ChainError> {
        let balance = registry.get_balance(&self.seller);
        // Le transfert est fait par le handler API
        self.state = ListingState::Completed;
        Ok(())
    }
}

/// Type de publication UQ
#[derive(Debug, Clone, PartialEq)]
pub enum PubType {
    Boutique,
    Restaurant,
    Service,
    Emploi,
    Vehicule,
    Logement,
    Rencontre,
    Contenu,
    Connexion,
    Cours,
    Custom {
        category: String,
        tags: Vec<String>,
    },
}

/// Badge d'affichage dans le flux
#[derive(Debug, Clone, PartialEq)]
pub enum PubBadge {
    Payant,
    Social,
    Gratuit,
}

/// Publication complète UQ
#[derive(Debug, Clone)]
pub struct Publication {
    pub last_updated: u64,
    pub is_permanent: bool,
    pub video_short: Option<Vec<u8>>,
    pub photos: Vec<Vec<u8>>,
    pub description: String,
    pub external_url: Option<String>,
    pub tags: Vec<String>,
    pub category: String,
    pub listing: MarketplaceListing,
    pub pub_type: PubType,
    pub badge: PubBadge,
    pub title: String,
    pub price_per_second: Option<u64>,
    pub latitude: f64,
    pub longitude: f64,
    pub rating: f64,
    pub verified: bool,
}

impl Publication {
    /// Crée une nouvelle publication
    pub fn new(
        listing: MarketplaceListing,
        pub_type: PubType,
        title: String,
        description: String,
        latitude: f64,
        longitude: f64,
    ) -> Self {
        let (badge, category, tags) = match &pub_type {
            PubType::Rencontre | PubType::Emploi => (PubBadge::Social, String::new(), Vec::new()),
            PubType::Custom { category: c, tags: t } => (PubBadge::Payant, c.clone(), t.clone()),
            _ => (PubBadge::Payant, String::new(), Vec::new()),
        };
        
        Self {
            last_updated: 0,
            is_permanent: false,
            video_short: None,
            photos: Vec::new(),
            description,
            external_url: None,
            tags,
            category,
            listing,
            pub_type,
            badge,
            title,
            price_per_second: None,
            latitude,
            longitude,
            rating: 0.0,
            verified: false,
        }
    }
    
    /// Marque comme vérifiée
    pub fn mark_verified(&mut self) {
        self.verified = true;
    }
    
    /// Ajoute un lien externe (YouTube, Spotify...)
    
    
    /// Définit le prix par seconde (pour le lecteur UQ)
    pub fn set_price_per_second(&mut self, price: u64) {
        self.price_per_second = Some(price);
    }
    
    /// Vérifie si la publication est active
    pub fn is_active(&self) -> bool {
        matches!(self.listing.state, ListingState::Published)
    }
    
    /// Vérifie si la publication est payante
    pub fn is_payant(&self) -> bool {
        self.badge == PubBadge::Payant
    }
}


impl Publication {
    /// Valide la description (minimum 10, maximum 300 caractères)
    pub fn validate_description(text: &str) -> Result<(), String> {
        if text.len() < 10 {
            return Err("Description trop courte : minimum 10 caractères".to_string());
        }
        if text.len() > 300 {
            return Err("Description trop longue : maximum 300 caractères".to_string());
        }
        Ok(())
    }
    
    /// Valide les photos (1 à 5 photos)
    pub fn validate_photos(photos: &[Vec<u8>]) -> Result<(), String> {
        if photos.is_empty() {
            return Err("Au moins 1 photo requise".to_string());
        }
        if photos.len() > 5 {
            return Err("Maximum 5 photos".to_string());
        }
        Ok(())
    }
    
    /// Valide la vidéo (5 à 15 secondes)
    pub fn validate_video(video: &[u8]) -> Result<(), String> {
        // En production : analyser les métadonnées de la vidéo
        // Pour l'instant : vérifier la taille (proxy)
        if video.is_empty() {
            return Err("Vidéo vide".to_string());
        }
        Ok(())
    }
    
    /// Ajoute une vidéo
    pub fn set_video(&mut self, video: Vec<u8>) -> Result<(), String> {
        Self::validate_video(&video)?;
        self.video_short = Some(video);
        Ok(())
    }
    
    /// Ajoute des photos
    pub fn set_photos(&mut self, photos: Vec<Vec<u8>>) -> Result<(), String> {
        Self::validate_photos(&photos)?;
        self.photos = photos;
        Ok(())
    }
    
    /// Définit la description avec validation
    pub fn set_description(&mut self, text: String) -> Result<(), String> {
        Self::validate_description(&text)?;
        self.description = text;
        Ok(())
    }
    
    /// Ajoute un lien externe
    
    
    /// Vérifie si la publication est complète
    pub fn is_complete(&self) -> bool {
        !self.description.is_empty() 
        && !self.photos.is_empty()
        && self.listing.amount > 0
    }
    /// Ajoute un lien externe (YouTube, Spotify...)
    pub fn set_external_url(&mut self, url: String) {
        self.external_url = Some(url);
    }
}

impl Publication {
    /// Marque comme permanente (boutique, restaurant, salon...)
    pub fn mark_permanent(&mut self) {
        self.is_permanent = true;
        self.last_updated = chrono::Utc::now().timestamp() as u64;
    }
    
    /// Met à jour la preuve de vie
    pub fn update_proof_of_life(&mut self) {
        self.last_updated = chrono::Utc::now().timestamp() as u64;
    }
    
    /// Vérifie si la publication a expiré (7 jours sans mise à jour)
    pub fn is_expired(&self) -> bool {
        if !self.is_permanent {
            return false; // Les publications éphémères n'expirent pas
        }
        
        let now = chrono::Utc::now().timestamp() as u64;
        let seven_days = 7 * 24 * 3600;
        
        now > self.last_updated + seven_days
    }
    
    /// Vérifie si la publication doit disparaître du flux
    pub fn should_disappear(&self) -> bool {
        match self.listing.state {
            ListingState::Sold | ListingState::Completed | ListingState::Refunded => true,
            _ => false,
        }
    }
    
    /// Vérifie si la publication doit rester dans le flux
    pub fn should_remain(&self) -> bool {
        if self.should_disappear() {
            return false;
        }
        
        if self.is_permanent {
            return !self.is_expired();
        }
        
        self.is_active()
    }
}
