use serde::{Deserialize, Serialize};

/// Présence en ligne maximale d'une vitrine sans renouvellement (7 jours).
pub const DUREE_VITRINE_SECONDES: i64 = 7 * 86_400;
/// Le renouvellement prolonge la présence par blocs de 24 heures.
pub const DUREE_RENOUVELLEMENT_SECONDES: i64 = 86_400;
/// Frais de présence : 5 % de la valeur du catalogue, par bloc de 24 h.
pub const FRAIS_VITRINE_POURCENT: i64 = 5;
/// Une galerie de mini-site s'explore en 10 visuels au plus.
pub const GALERIE_MAX: usize = 10;
/// Un tick du robinet ne rattrape jamais plus d'une journée de présence :
/// une coupure du serveur ne se facture pas après coup.
pub const RATTRAPAGE_MAX_SECONDES: i64 = DUREE_RENOUVELLEMENT_SECONDES;

/// Une boutique utilisateur : une identité, une galerie modifiable et des
/// sections libres que le Studio assemble pour le mini-site.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Shop {
    pub id: String,
    pub owner: String,
    pub name: String,
    pub description: String,
    pub category: String,
    pub latitude: f64,
    pub longitude: f64,
    pub rating: f64,
    /// Vitrine présente dans le flux Zando.
    pub open: bool,
    pub created_at: i64,
    /// Visuel d'accroche de la vitrine (photo, à défaut de vidéo).
    #[serde(default)]
    pub presentation_photo: String,
    /// Vidéo externe de moins de 30 s (.mp4/.webm), optionnelle.
    #[serde(default)]
    pub presentation_video: String,
    /// Galerie du mini-site, 10 visuels au plus.
    #[serde(default)]
    pub gallery: Vec<String>,
    /// Sections libres du mini-site (annonces, panneaux, horaires...).
    #[serde(default)]
    pub sections: Vec<ShopSection>,
    /// Fin de présence en ligne, en secondes epoch. 0 = boutique héritée,
    /// antérieure au compteur : elle reste en ligne.
    #[serde(default)]
    pub en_ligne_jusqu_a: i64,
    /// Ligne `shop_items` qui porte la vitrine dans le flux.
    #[serde(default)]
    pub vitrine_item_id: String,
    /// Le robinet de présence : les frais coulent seconde par seconde tant que
    /// la vitrine est en ligne. Faux = boutique héritée, non comptée.
    #[serde(default)]
    pub robinet: bool,
    /// Horodatage du dernier règlement : l'horloge du robinet repart d'ici.
    #[serde(default)]
    pub dernier_reglement: i64,
    /// Secondes écoulées pas encore facturées (le débit d'un tick peut être nul
    /// quand le catalogue est petit : rien n'est perdu, rien n'est inventé).
    #[serde(default)]
    pub secondes_en_attente: i64,
    /// Total déjà prélevé du coffre pour cette présence, en points.
    #[serde(default)]
    pub regle_points: i64,
    /// Mini-site édité page par page (éditeur façon Studio). Absent des
    /// boutiques héritées : elles gardent galerie + sections.
    #[serde(default)]
    pub site: Option<MiniSite>,
    /// Studio de vitrine (nouveau modèle 22 panneaux). Conservé tel quel : le
    /// serveur ne le découpe pas, il fait foi pour l'aperçu Flux/Explorer/
    /// mini-site et pour la fiche acheteur. `site` reste la projection héritée.
    #[serde(default)]
    pub vitrine: Option<serde_json::Value>,
}

/// Une tranche de contenu du mini-site. Le type guide l'affichage (annonce,
/// panneau, liste, texte, image) mais le contenu reste libre.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShopSection {
    pub id: String,
    pub titre: String,
    #[serde(rename = "type", default)]
    pub section_type: String,
    #[serde(default)]
    pub contenu: String,
    #[serde(default)]
    pub media: String,
}

impl ShopSection {
    pub fn new(id: &str, titre: &str, section_type: &str, contenu: &str, media: &str) -> Self {
        Self {
            id: id.to_string(),
            titre: titre.trim().to_string(),
            section_type: if section_type.trim().is_empty() {
                "texte".to_string()
            } else {
                section_type.trim().to_string()
            },
            contenu: contenu.trim().to_string(),
            media: media.trim().to_string(),
        }
    }
}

/// Un mini-site édité tient en 3 pages au plus.
pub const SITE_PAGES_MAX: usize = 3;
/// Chaque page porte 2 médias au plus.
pub const SITE_MEDIAS_MAX: usize = 2;

fn cent() -> f64 {
    100.0
}
fn un() -> f64 {
    1.0
}
fn photo_ty() -> String {
    "photo".to_string()
}

/// Réglages colorimétriques d'un média, appliqués en direct côté client.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaAdjust {
    #[serde(default = "cent")]
    pub brightness: f64,
    #[serde(default = "cent")]
    pub contrast: f64,
    #[serde(default = "cent")]
    pub saturation: f64,
    #[serde(default)]
    pub highlights: f64,
    #[serde(default)]
    pub shadows: f64,
    #[serde(default)]
    pub sharp: f64,
    #[serde(default)]
    pub warmth: f64,
    #[serde(default)]
    pub hue: f64,
    #[serde(default)]
    pub vignette: f64,
}

impl Default for MediaAdjust {
    fn default() -> Self {
        Self {
            brightness: 100.0,
            contrast: 100.0,
            saturation: 100.0,
            highlights: 0.0,
            shadows: 0.0,
            sharp: 0.0,
            warmth: 0.0,
            hue: 0.0,
            vignette: 0.0,
        }
    }
}

/// Un visuel importé par-dessus le média (photo ou vidéo), en % du cadre.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteOverlay {
    #[serde(rename = "type", default = "photo_ty")]
    pub kind: String,
    #[serde(default)]
    pub src: String,
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
    #[serde(default)]
    pub w: f64,
    #[serde(default)]
    pub h: f64,
}

/// Un son calé sur la durée de la vidéo (start / len en secondes).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteSound {
    #[serde(default)]
    pub track: String,
    #[serde(default)]
    pub src: String,
    #[serde(default)]
    pub start: f64,
    #[serde(default)]
    pub len: f64,
}

/// Un morceau de la timeline après coupe (start / end en secondes source).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteSeg {
    #[serde(default)]
    pub start: f64,
    #[serde(default)]
    pub end: f64,
}

/// Un média d'une page : photo ou vidéo, ses effets et son montage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteMedia {
    #[serde(rename = "type", default = "photo_ty")]
    pub media_type: String,
    #[serde(default)]
    pub src: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub dur: f64,
    #[serde(default)]
    pub adj: MediaAdjust,
    #[serde(default = "un")]
    pub speed: f64,
    #[serde(default)]
    pub crop: f64,
    #[serde(default)]
    pub overlays: Vec<SiteOverlay>,
    #[serde(default)]
    pub sound: Option<SiteSound>,
    #[serde(default)]
    pub segs: Vec<SiteSeg>,
}

/// Un appel à action de la page (contact, achat, lien). `url` ne sert que pour
/// un lien externe ; contact et achat ouvrent le chat et le panier de la boutique.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteButton {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub url: String,
}

/// Un bouton de réseau social en bas de page : son nom et son lien réel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteSocial {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub url: String,
}

/// Une vidéo externe (YouTube ou autre) affichée dans la page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteVideo {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub src: String,
}

/// Une page du mini-site : 2 médias, des boutons, une vidéo, des réseaux.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SitePage {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub medias: Vec<SiteMedia>,
    #[serde(default)]
    pub buttons: Vec<SiteButton>,
    #[serde(default)]
    pub video: Option<SiteVideo>,
    #[serde(default)]
    pub social: Vec<SiteSocial>,
}

/// Le mini-site édité : au plus 3 pages, naviguées horizontalement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MiniSite {
    #[serde(default)]
    pub pages: Vec<SitePage>,
}

impl MiniSite {
    /// Ramène le mini-site aux limites produit : 3 pages, 2 médias par page.
    pub fn normalise(mut self) -> Self {
        self.pages.truncate(SITE_PAGES_MAX);
        for page in &mut self.pages {
            page.medias.truncate(SITE_MEDIAS_MAX);
        }
        self
    }
}

/// Un produit dans une boutique
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Product {
    pub id: String,
    pub shop_id: String,
    pub name: String,
    pub description: String,
    pub price_points: u64,
    pub stock: u64,
    pub category: String,
    pub active: bool,
}

/// Résultat d'une recherche
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub shops: Vec<Shop>,
    pub products: Vec<Product>,
    pub sellers: Vec<DataSeller>,
    pub total: u64,
}

/// Un vendeur de connexion internet
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataSeller {
    pub id: String,
    pub name: String,
    pub price_per_mb: u64,
    pub signal_strength: i8,
    pub network_type: String,
    pub latitude: f64,
    pub longitude: f64,
    pub distance_km: f64,
    pub rating: f64,
}

/// Une notification utilisateur
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub id: String,
    pub user_id: String,
    pub notification_type: String,
    pub title: String,
    pub content: String,
    pub read: bool,
    pub created_at: i64,
}

/// Statistiques du tableau de bord
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardStats {
    pub balance_bubbles: u64,
    pub balance_points: u64,
    pub value_usd: f64,
    pub ubi_claimed_today: bool,
    pub ubi_total_claimed: u64,
    pub referral_count: u64,
    pub referral_earnings: u64,
    pub transactions_count: u64,
    pub messages_unread: u64,
    pub offers_active: u64,
}

impl Shop {
    pub fn new(
        owner: &str,
        name: &str,
        description: &str,
        category: &str,
        lat: f64,
        lon: f64,
    ) -> Self {
        Self {
            id: format!(
                "shop_{}",
                chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
            ),
            owner: owner.to_string(),
            name: name.to_string(),
            description: description.to_string(),
            category: category.to_string(),
            latitude: lat,
            longitude: lon,
            rating: 0.0,
            open: true,
            created_at: chrono::Utc::now().timestamp(),
            presentation_photo: String::new(),
            presentation_video: String::new(),
            gallery: Vec::new(),
            sections: Vec::new(),
            site: None,
            vitrine: None,
            en_ligne_jusqu_a: chrono::Utc::now().timestamp() + DUREE_VITRINE_SECONDES,
            vitrine_item_id: String::new(),
            // Le prélèvement continu part d'un coffre : il n'est ouvert que par
            // un geste PIN (prolonger la présence). La première fenêtre de
            // 7 jours reste gratuite.
            robinet: false,
            dernier_reglement: 0,
            secondes_en_attente: 0,
            regle_points: 0,
        }
    }

    /// Une boutique héritée (JSON sans compteur) reste en ligne : le compteur
    /// ne s'applique qu'aux vitrines créées depuis l'atelier.
    pub fn est_en_ligne(&self, maintenant: i64) -> bool {
        self.open && (self.en_ligne_jusqu_a <= 0 || self.en_ligne_jusqu_a > maintenant)
    }

    /// Secondes de présence restantes ; 0 quand le compteur est épuisé.
    pub fn secondes_restantes(&self, maintenant: i64) -> i64 {
        if self.en_ligne_jusqu_a <= 0 {
            return 0;
        }
        (self.en_ligne_jusqu_a - maintenant).max(0)
    }

    /// Rouvre le robinet et remet l'horloge à zéro : les secondes déjà écoulées
    /// ne sont pas facturées deux fois après une prolongation.
    pub fn ouvrir_robinet(&mut self, maintenant: i64) {
        self.robinet = true;
        self.dernier_reglement = maintenant;
        self.secondes_en_attente = 0;
    }

    /// Le nombre de secondes que ce règlement doit facturer : uniquement du
    /// temps de présence réelle, jamais la période déjà payée, jamais plus
    /// d'une journée de rattrapage.
    pub fn secondes_a_facturer(&self, maintenant: i64) -> i64 {
        if !self.robinet || !self.est_en_ligne(maintenant) {
            return 0;
        }
        let ecoulee = (maintenant - self.dernier_reglement)
            .max(0)
            .min(RATTRAPAGE_MAX_SECONDES);
        // On ne facture pas la présence qui n'a pas encore eu lieu.
        if self.en_ligne_jusqu_a > 0 {
            ecoulee.min((self.en_ligne_jusqu_a - maintenant).max(0))
        } else {
            ecoulee
        }
    }
}

impl Product {
    pub fn new(shop_id: &str, name: &str, price_points: u64, stock: u64) -> Self {
        Self {
            id: format!(
                "prod_{}",
                chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
            ),
            shop_id: shop_id.to_string(),
            name: name.to_string(),
            description: String::new(),
            price_points,
            stock,
            category: String::new(),
            active: true,
        }
    }
}

/// Calcule la distance entre deux points GPS (formule de Haversine)
pub fn calculate_distance_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let earth_radius_km = 6371.0;
    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();
    let a = (dlat / 2.0).sin().powi(2)
        + lat1.to_radians().cos() * lat2.to_radians().cos() * (dlon / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().atan2((1.0 - a).sqrt());
    earth_radius_km * c
}
