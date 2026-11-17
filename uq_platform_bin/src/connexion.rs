// ====================
// 📡 CONNEXION — revendre sa connexion, téléphone à téléphone
// ====================
//
// Le vendeur publie une fiche : un nom, une photo, la nature de son lien
// (Starlink ou réseau mobile) et l'endroit où il le partage. Le serveur ne voit
// jamais son prix. Le tarif par seconde reste sur son appareil, parce que c'est
// lui qui décide, au moment où quelqu'un frappe à sa porte, ce que vaut son
// forfait : le devis se négocie de téléphone à téléphone, pas dans une base.
//
// Le serveur ne répond qu'à une question : « qui est assez proche et assez
// solide pour dépanner ? ». D'où les deux règles de ce module :
//
//   1. le classement mêle la proximité et la puissance mesurée — jamais un prix,
//      que le serveur ignore par construction ;
//   2. la réponse publique ne contient ni le UQ du vendeur, ni ses coordonnées
//      exactes : seulement la distance arrondie et la mesure qu'il a publiée.
//
// La puissance est une mesure, pas une promesse. Elle a une date, et une fiche
// mesurée il y a six heures vaut moins qu'une fiche mesurée il y a deux minutes
// — le score s'en ressent jusqu'à ce que le vendeur la re-mesure.

use actix_web::{HttpRequest, HttpResponse, web};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::database::Database;
use crate::user_models::calculate_distance_km;

/// Lien satellite : le vendeur a une antenne, le débit ne dépend pas d'une cellule.
pub const RESEAU_STARLINK: &str = "starlink";
/// Lien cellulaire (4G/5G) : la puissance dépend de l'antenne relais la plus proche.
pub const RESEAU_MOBILE: &str = "mobile";

/// Au-delà de cet âge, la mesure perd sa valeur de classement : le lien a pu
/// tomber, l'antenne a pu se saturer. La fiche reste visible, elle ne s'avance plus.
pub const MESURE_FRAICHE_SECONDES: i64 = 30 * 60;
/// Plancher de la décote : une mesure très vieille garde un quart de sa puissance
/// affichée, pour qu'aucune fiche ne disparaisse du classement sans raison lisible.
const DECOTE_MINIMALE: f64 = 0.25;
/// Région par défaut autour de l'acheteur, en kilomètres.
pub const RAYON_PAR_DEFAUT_KM: f64 = 20.0;
/// Débit auquel la note de puissance atteint son maximum.
pub const DEBIT_PLEIN_MBPS: f64 = 40.0;
/// Latence au-delà de laquelle la note de latence tombe à zéro.
pub const LATENCE_PLAFOND_MS: f64 = 300.0;
/// Le nom de la fiche reste court : il s'affiche sur une carte, pas en titre d'article.
pub const NOM_MAX: usize = 60;
/// Une photo de fiche est une data URI : le plafond des médias publiés s'applique.
pub const PHOTO_MAX_CHARS: usize = crate::database::PUBLICATION_MEDIA_MAX_CHARS;
/// Un avis reste court : il raconte une expérience de connexion, il ne remplace
/// pas la session d'échange qui, elle, n'est pas encore ouverte.
pub const AVIS_TEXTE_MAX: usize = 280;
/// Combien d'avis affichés sous la moyenne : la liste reste lisible sur un écran.
pub const AVIS_LIMITE_AFFICHEE: i64 = 8;

/// Identifiant de la boutique système.
///
/// Elle n'est pas stockée : elle est dérivée à chaque lecture. Ainsi aucun
/// Studio de vendeur ne peut l'écraser, la renommer ou la supprimer, et elle
/// ne peut pas se désynchroniser du code qui affiche les fiches.
pub const BOUTIQUE_ID: &str = "uq_connexion";
/// Titre affiché : court, il dit ce que la boutique vend sans porter le nom de
/// la plateforme ni celui d'un constructeur de terminal satellite.
pub const BOUTIQUE_NOM: &str = "Connexion à proximité";
/// La mention de confiance de la fiche : elle est portée par le compte du
/// système, pas par un vendeur privé.
pub const CONFIANCE_SYSTEME: &str = "TrustUnder";
const BOUTIQUE_DESCRIPTION: &str =
    "Tous les appareils qui vendent leur connexion, classés par proximité et par puissance mesurée.";
/// Visuels de la boutique : embarqués dans l'application, jamais hotlinks depuis
/// un site tiers. La boutique reste affichable hors ligne et ne peut pas être
/// cassée par quelqu'un d'autre que toi.
pub const BOUTIQUE_GALERIE: [&str; 3] = [
    "/assets/connexion/accueil-toits.webp",
    "/assets/connexion/accueil-lignes.webp",
    "/assets/connexion/accueil-nuit.webp",
];

/// La boutique système, annexée au compte owner.
///
/// « Annexée » veut dire exactement ceci : le porteur de la fiche est le compte
/// du système, donc personne n'achète ni ne vend au nom d'un vendeur privé, et
/// le profil s'affiche avec la mention de confiance système. Les éléments que
/// la boutique contient, ce sont les fiches des vendeurs, classées pour
/// l'acheteur qui la consulte — pas un catalogue écrit à la main.
pub fn boutique_systeme() -> serde_json::Value {
    json!({
        "id": BOUTIQUE_ID,
        "owner": crate::security_login::OWNER_UQ,
        "name": BOUTIQUE_NOM,
        "description": BOUTIQUE_DESCRIPTION,
        "category": "connexion",
        "systeme": true,
        "confiance_systeme": true,
        "confiance": CONFIANCE_SYSTEME,
        "open": true,
        // Pas d'échéance : la boutique du système ne s'achète pas le droit de
        // rester en ligne, elle est le service.
        "created_at": 0,
        "presentation_photo": BOUTIQUE_GALERIE[0],
        "gallery": BOUTIQUE_GALERIE,
        "sections": [
            {
                "id": "connexion_fonctionnement",
                "titre": "Ce que la boutique affiche",
                "type": "information",
                "contenu": "Les vendeurs de ta région, le plus proche mis en avant, les suivants derrière. Pour chacun : le nom, la photo, la nature du lien, la distance arrondie et la dernière mesure publiée.",
            },
            {
                "id": "connexion_prix_local",
                "titre": "Ce que la boutique ne sait pas",
                "type": "information",
                "contenu": "Ton tarif par seconde. Il reste sur l'appareil du vendeur et se négocie téléphone à téléphone au moment de la demande.",
            }
        ],
    })
}

/// Ce que le vendeur accepte qu'on publie de lui.
///
/// Aucun champ prix n'existe dans ce type, et c'est le cœur du contrat : le
/// serveur ne peut pas révéler, facturer ni classer ce qu'il n'a jamais reçu.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnonceConnexion {
    pub id: String,
    /// UQ du vendeur. Jamais sérialisé dans une réponse publique.
    pub owner: String,
    pub nom: String,
    #[serde(default)]
    pub photo: String,
    /// `starlink` ou `mobile`.
    pub reseau: String,
    pub latitude: f64,
    pub longitude: f64,
    /// Puissance du lien, en pourcentage (0 à 100) : barres RSSI remontées par l'app.
    #[serde(default)]
    pub signal: f64,
    /// Débit descendant mesuré par l'appareil du vendeur, en Mb/s.
    #[serde(default)]
    pub debit_mbps: f64,
    #[serde(default)]
    pub latence_ms: f64,
    /// Horodatage de la mesure : c'est ce qui la rend fraîche ou périmée.
    #[serde(default)]
    pub mesuree_a: i64,
    #[serde(default)]
    pub publiee: bool,
    #[serde(default)]
    pub creee_a: i64,
    #[serde(default)]
    pub maj_a: i64,
}

impl AnnonceConnexion {
    /// Un compte tient une seule annonce : republier met à jour la même fiche,
    /// ce qui évite un classement où le même vendeur occupe trois places.
    pub fn id_du_compte(uq: &str) -> String {
        format!("connexion_{}", uq.trim().to_lowercase().replace('-', "_"))
    }

    /// Note de puissance de 0 à 1 : le signal d'abord, le débit mesuré et la
    /// latence ensuite, le tout plafonné par la fraîcheur de la mesure.
    pub fn puissance(&self, maintenant: i64) -> f64 {
        let age = (maintenant - self.mesuree_a).max(0);
        puissance_de(self.signal, self.debit_mbps, self.latence_ms, age)
    }

    /// La fiche publique que l'acheteur reçoit : ni UQ, ni coordonnées exactes.
    pub fn vue_publique(&self, distance_km: Option<f64>, maintenant: i64) -> serde_json::Value {
        let puissance = self.puissance(maintenant);
        let distance = distance_km.map(|d| (d * 10.0).round() / 10.0);
        json!({
            "id": self.id,
            "nom": self.nom,
            "photo": self.photo,
            "reseau": self.reseau,
            "signal": self.signal,
            "debit_mbps": self.debit_mbps,
            "latence_ms": self.latence_ms,
            "mesuree_a": self.mesuree_a,
            "puissance": (puissance * 100.0).round() / 100.0,
            "distance_km": distance,
            "mesure_peu_fraiche": (maintenant - self.mesuree_a).max(0) > MESURE_FRAICHE_SECONDES,
            "prix": "sur l'appareil du vendeur"
        })
    }
}

/// Une valeur bornée à 0..=1, sans NaN qui viendrait troubler un tri.
fn borne(value: f64) -> f64 {
    if !value.is_finite() {
        0.0
    } else {
        value.clamp(0.0, 1.0)
    }
}

/// Note de puissance brute, sortie du modèle pour que les tests la vérifient
/// sans construire une fiche entière.
pub fn puissance_de(signal: f64, debit_mbps: f64, latence_ms: f64, age_secondes: i64) -> f64 {
    let signal = borne(signal / 100.0);
    let debit = borne(debit_mbps / DEBIT_PLEIN_MBPS);
    let latence = borne(1.0 - latence_ms / LATENCE_PLAFOND_MS);
    let base = 0.5 * signal + 0.3 * debit + 0.2 * latence;
    let fraicheur = if age_secondes <= MESURE_FRAICHE_SECONDES {
        1.0
    } else {
        (MESURE_FRAICHE_SECONDES as f64 / age_secondes.max(1) as f64).max(DECOTE_MINIMALE)
    };
    borne(base * fraicheur)
}

/// Ce qui fait une tête de liste : la puissance du lien rapportée à la distance.
///
/// Un lien parfait à 5 km ne bat pas un lien moyen à 200 m : la revente de
/// connexion se fait sur place, en Wi-Fi Direct ou en Bluetooth, la proximité
/// vaut donc plus que la performance.
fn score(puissance: f64, distance_km: f64) -> f64 {
    puissance / (1.0 + distance_km.max(0.0))
}

/// Les clés de prix que le serveur refuse : le tarif vit sur l'appareil du
/// vendeur, et une requête qui en apporte un doit le savoir tout de suite.
const CLES_DE_PRIX: &[&str] = &[
    "prix",
    "price",
    "prix_par_seconde",
    "prix_per_seconde",
    "prix_per_second",
    "price_per_second",
    "price_per_sec",
    "prix_seconde",
    "price_per_mb",
    "prix_par_mb",
    "price_points",
    "prix_points",
    "tarif",
];

/// `Some(clé)` quand la requête tente de publier un prix.
pub fn cle_de_prix_presente(body: &serde_json::Value) -> Option<&'static str> {
    CLES_DE_PRIX.iter().copied().find(|cle| body.get(*cle).is_some())
}

fn texte(body: &serde_json::Value, cle: &str) -> String {
    body.get(cle)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string()
}

fn nombre(body: &serde_json::Value, cle: &str) -> Option<f64> {
    body.get(cle).and_then(|v| v.as_f64()).filter(|v| v.is_finite())
}

/// Une position exploitable : des degrés valides, et pas le (0, 0) d'un GPS qui
/// n'a jamais eu de fix. Le golfe de Guinée n'est pas une zone de revente.
fn position_valide(latitude: f64, longitude: f64) -> bool {
    latitude.is_finite()
        && longitude.is_finite()
        && latitude.abs() <= 90.0
        && longitude.abs() <= 180.0
        && !(latitude.abs() < 0.0001 && longitude.abs() < 0.0001)
}

/// Construit une fiche à partir de la requête, ou dit ce qui manque.
///
/// Les champs inconnus ne sont pas kopiés : `owner`, `id` et les horodatages
/// viennent du jeton et de l'horloge du serveur, jamais du client.
pub fn fiche_from_body(
    uq: &str,
    body: &serde_json::Value,
    maintenant: i64,
    existante: Option<&AnnonceConnexion>,
) -> Result<AnnonceConnexion, String> {
    let nom = texte(body, "nom");
    if nom.is_empty() {
        return Err("Le nom de ta fiche est obligatoire".to_string());
    }
    if nom.chars().count() > NOM_MAX {
        return Err(format!("Le nom est trop long ({} caractères au plus)", NOM_MAX));
    }

    let reseau = texte(body, "reseau").to_lowercase();
    if reseau != RESEAU_STARLINK && reseau != RESEAU_MOBILE {
        return Err("Le type de lien est « starlink » ou « mobile »".to_string());
    }

    let latitude = nombre(body, "latitude").unwrap_or(0.0);
    let longitude = nombre(body, "longitude").unwrap_or(0.0);
    if !position_valide(latitude, longitude) {
        return Err("Une position valide est obligatoire : on ne partage pas une connexion sans savoir où".to_string());
    }

    let photo = texte(body, "photo");
    if photo.chars().count() > PHOTO_MAX_CHARS {
        return Err("La photo dépasse 2 Mo".to_string());
    }

    let signal = nombre(body, "signal").unwrap_or(0.0).clamp(0.0, 100.0);
    let debit_mbps = nombre(body, "debit_mbps").unwrap_or(0.0).max(0.0);
    let latence_ms = nombre(body, "latence_ms").unwrap_or(0.0).max(0.0);
    // Une mesure sans horodatage est une mesure d'il y a cinq minutes : l'app
    // vient de la faire. Sans mesure du tout, la fiche reste quand même classée,
    // mais avec une puissance de zéro — rien n'est inventé à la place du vendeur.
    let mesuree_a = nombre(body, "mesuree_a")
        .map(|v| v as i64)
        .filter(|v| *v > 0)
        .unwrap_or(maintenant);

    let id = AnnonceConnexion::id_du_compte(uq);
    Ok(AnnonceConnexion {
        id: id.clone(),
        owner: uq.to_string(),
        nom,
        photo,
        reseau,
        latitude,
        longitude,
        signal,
        debit_mbps,
        latence_ms,
        mesuree_a,
        publiee: true,
        creee_a: existante.map(|a| a.creee_a).filter(|v| *v > 0).unwrap_or(maintenant),
        maj_a: maintenant,
    })
}

/// Classe les annonces pour un acheteur.
///
/// `position` absente = l'acheteur n'a pas partagé sa position : le tri se fait
/// alors sur la seule puissance, et `distance_km` reste nul dans la réponse.
/// Une fiche plus éloignée que `rayon_km` est hors région : elle n'est pas
/// proposée, parce qu'on ne dépanne pas quelqu'un à 300 km avec son forfait.
pub fn classer(
    annonces: &[AnnonceConnexion],
    position: Option<(f64, f64)>,
    rayon_km: f64,
    maintenant: i64,
) -> Vec<serde_json::Value> {
    let mut classees: Vec<(f64, f64, AnnonceConnexion)> = annonces
        .iter()
        .filter(|annonce| annonce.publiee)
        .filter_map(|annonce| {
            let distance = match position {
                Some((lat, lon)) => {
                    let d = calculate_distance_km(lat, lon, annonce.latitude, annonce.longitude);
                    if d > rayon_km {
                        return None;
                    }
                    Some(d)
                }
                None => None,
            };
            let puissance = annonce.puissance(maintenant);
            let note = distance.map(|d| score(puissance, d)).unwrap_or(puissance);
            Some((note, distance.unwrap_or(f64::MAX), annonce.clone()))
        })
        .collect();

    // Score d'abord, puis le plus proche à score égal, puis l'identifiant pour
    // que deux appels consécutifs rendent le même ordre.
    classees.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .then_with(|| a.2.id.cmp(&b.2.id))
    });

    classees
        .into_iter()
        .map(|(note, distance, annonce)| {
            let mut vue = annonce.vue_publique(
                if distance == f64::MAX { None } else { Some(distance) },
                maintenant,
            );
            vue["score"] = json!((note * 1000.0).round() / 1000.0);
            vue
        })
        .collect()
}

fn lire_annonces(db: &Database) -> Vec<AnnonceConnexion> {
    db.get_annonces_publiees()
        .into_iter()
        .filter_map(|v| serde_json::from_value::<AnnonceConnexion>(v).ok())
        .filter(|a| !a.owner.trim().is_empty() && !a.id.trim().is_empty())
        .collect()
}

fn reponse_erreur(message: &str) -> HttpResponse {
    HttpResponse::BadRequest().json(json!({ "status": "error", "message": message }))
}

/// Ce que la plateforme compte vraiment, à l'instant où elle est interrogée.
///
/// Ces chiffres remplacent toute promesse de popularité : ils ne sont ni
/// arrondis à la hausse, ni semés par le code. Un compte qui lit la boutique
/// voit le nombre de fiches publiées aujourd'hui, pas le nombre de fiches que
/// la boutique aimerait avoir. Une place vide se lit comme une place vide.
fn vitalite_boutique(annonces: &[AnnonceConnexion], maintenant: i64) -> serde_json::Value {
    let mut vendeurs: Vec<&str> = Vec::with_capacity(annonces.len());
    let mut satellite = 0usize;
    let mut mobile = 0usize;
    let mut fraiches = 0usize;
    let mut derniere = 0i64;

    for annonce in annonces {
        let compte = annonce.owner.trim();
        if !vendeurs.contains(&compte) {
            vendeurs.push(compte);
        }
        match annonce.reseau.as_str() {
            RESEAU_STARLINK => satellite += 1,
            RESEAU_MOBILE => mobile += 1,
            _ => {}
        }
        if maintenant - annonce.mesuree_a <= MESURE_FRAICHE_SECONDES {
            fraiches += 1;
        }
        derniere = derniere.max(annonce.maj_a.max(annonce.creee_a));
    }

    json!({
        "fiches": annonces.len(),
        "vendeurs": vendeurs.len(),
        "satellite": satellite,
        "mobile": mobile,
        "mesures_fraiches": fraiches,
        "derniere_fiche_a": derniere,
    })
}

// ==========================================================
// ROUTES
// ==========================================================

/// POST /api/connexion/publish — publier (ou remonter) sa fiche de revente.
pub async fn connexion_publish(
    req: HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(claims) => claims,
        Err(response) => return response,
    };
    if let Some(cle) = cle_de_prix_presente(&body) {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": format!(
                "Le prix ne se publie pas (« {cle} ») : ton tarif par seconde reste sur ton appareil \
                 et se négocie téléphone à téléphone."
            ),
            "prix_local": true
        }));
    }

    let db = Database::shared("uq_platform_db.json");
    let maintenant = chrono::Utc::now().timestamp();
    let existante = db
        .get_annonce_par_compte(&claims.sub)
        .and_then(|v| serde_json::from_value::<AnnonceConnexion>(v).ok());

    let annonce = match fiche_from_body(&claims.sub, &body, maintenant, existante.as_ref()) {
        Ok(annonce) => annonce,
        Err(message) => return reponse_erreur(&message),
    };

    let json_annonce = serde_json::to_value(&annonce).unwrap_or(json!({}));
    if let Err(erreur) = db.save_annonce(json_annonce.clone()) {
        return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("Fiche non enregistrée: {}", erreur)
        }));
    }

    HttpResponse::Ok().json(json!({
        "status": "success",
        "annonce": json_annonce,
        "puissance": (annonce.puissance(maintenant) * 100.0).round() / 100.0,
        "prix_local": true,
        "message": "Fiche publiée : ta connexion est proposée aux appareils de ta région."
    }))
}

/// GET /api/connexion/mine — relire sa propre fiche, publiée ou retirée.
pub async fn connexion_mine(req: HttpRequest) -> HttpResponse {
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(claims) => claims,
        Err(response) => return response,
    };
    let db = Database::shared("uq_platform_db.json");
    let maintenant = chrono::Utc::now().timestamp();
    let annonce = db
        .get_annonce_par_compte(&claims.sub)
        .and_then(|v| serde_json::from_value::<AnnonceConnexion>(v).ok());

    HttpResponse::Ok().json(json!({
        "status": "success",
        "annonce": annonce,
        "publiee": annonce.as_ref().map(|a| a.publiee).unwrap_or(false),
        "puissance": annonce.as_ref().map(|a| (a.puissance(maintenant) * 100.0).round() / 100.0),
        "prix_local": true
    }))
}

/// POST /api/connexion/unpublish — retirer sa fiche du flux, sans l'effacer.
///
/// Le vendeur garde sa fiche dans le compte : la republier ne demande pas de
/// ressaisir un nom ou de renvoyer une photo.
///
/// Couper un flux ne demande pas de PIN : le geste PIN autorise un prélèvement,
/// il n'est jamais nécessaire pour arrêter de vendre.
pub async fn connexion_unpublish(req: HttpRequest) -> HttpResponse {
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(claims) => claims,
        Err(response) => return response,
    };
    let db = Database::shared("uq_platform_db.json");
    let maintenant = chrono::Utc::now().timestamp();

    let Some(mut annonce) = db
        .get_annonce_par_compte(&claims.sub)
        .and_then(|v| serde_json::from_value::<AnnonceConnexion>(v).ok())
    else {
        return HttpResponse::NotFound().json(json!({
            "status": "error", "message": "Aucune fiche de connexion sur ce compte"
        }));
    };

    annonce.publiee = false;
    annonce.maj_a = maintenant;
    let json_annonce = serde_json::to_value(&annonce).unwrap_or(json!({}));
    if let Err(erreur) = db.save_annonce(json_annonce.clone()) {
        return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("Retrait non enregistré: {}", erreur)
        }));
    }
    let _ = db.depublier_annonce(&annonce.id);

    HttpResponse::Ok().json(json!({
        "status": "success",
        "annonce": json_annonce,
        "publiee": false,
        "message": "Fiche retirée : elle reste dans ton compte et tu peux la republier."
    }))
}

/// GET /api/connexion/nearby — la liste classée pour l'acheteur.
///
/// Le premier de la liste est celui que la boutique système met en avant ; les
/// suivants remplissent le bouton « Explorer ».
pub async fn connexion_nearby(
    req: HttpRequest,
    query: web::Query<std::collections::HashMap<String, String>>,
) -> HttpResponse {
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(claims) => claims,
        Err(response) => return response,
    };
    let db = Database::shared("uq_platform_db.json");
    let maintenant = chrono::Utc::now().timestamp();

    let latitude = query.get("latitude").and_then(|v| v.parse::<f64>().ok());
    let longitude = query.get("longitude").and_then(|v| v.parse::<f64>().ok());
    let position = match (latitude, longitude) {
        (Some(lat), Some(lon)) if position_valide(lat, lon) => Some((lat, lon)),
        _ => None,
    };
    let rayon_km = query
        .get("rayon_km")
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| *v > 0.0)
        .unwrap_or(RAYON_PAR_DEFAUT_KM);
    let reseau = query.get("reseau").map(|v| v.trim().to_lowercase());

    let toutes = lire_annonces(&db);
    let annonces: Vec<AnnonceConnexion> = toutes
        .iter()
        .cloned()
        .filter(|a| !crate::robinet::meme_compte(&a.owner, &claims.sub))
        .filter(|a| match &reseau {
            Some(filtre) if filtre == RESEAU_STARLINK || filtre == RESEAU_MOBILE => {
                a.reseau == *filtre
            }
            _ => true,
        })
        .collect();

    let vitalite = vitalite_boutique(&toutes, maintenant);
    let avis = db.get_avis_par_fiche();
    let classment: Vec<serde_json::Value> = classer(&annonces, position, rayon_km, maintenant)
        .into_iter()
        .map(|vue| avec_avis(&avis, vue))
        .collect();
    let en_avant = classment.first().cloned();
    let suivants: Vec<serde_json::Value> = classment.iter().skip(1).cloned().collect();

    HttpResponse::Ok().json(json!({
        "status": "success",
        "trie_par": if position.is_some() { "proximite_et_puissance" } else { "puissance" },
        "rayon_km": rayon_km,
        "position_demandee": position.is_some(),
        "total": classment.len(),
        "en_avant": en_avant,
        "suivants": suivants,
        "vitalite": vitalite,
        "boutique": {
            "id": BOUTIQUE_ID,
            "nom": BOUTIQUE_NOM,
            "description": BOUTIQUE_DESCRIPTION,
            "confiance_systeme": true,
            "confiance": CONFIANCE_SYSTEME,
            "visuels": BOUTIQUE_GALERIE,
        },
        "prix": "Le tarif par seconde s'affiche à la demande, côté vendeur."
    }))
}

/// Coller l'agrégat d'avis sur la fiche publique : le nombre de voix et la
/// moyenne, pas les voix elles-mêmes. Une fiche sans avis affiche zéro, et
/// zéro se lit comme tel.
fn avec_avis(
    avis: &std::collections::HashMap<String, (f64, i64)>,
    mut vue: serde_json::Value,
) -> serde_json::Value {
    let id = vue
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    match avis.get(&id) {
        Some((moyenne, total)) if *total > 0 => {
            vue["avis_total"] = json!(total);
            vue["avis_moyenne"] = json!((moyenne * 10.0).round() / 10.0);
        }
        _ => {
            vue["avis_total"] = json!(0);
            vue["avis_moyenne"] = json!(null);
        }
    }
    vue
}

/// POST /api/connexion/avis — dire ce que la fiche d'un autre a valu.
///
/// Une seule ligne par couple fiche-auditeur : reposer un avis le remplace, il
/// ne s'additionne pas. Le serveur refuse l'avis sur sa propre fiche et celui
/// qui tente d'atteindre une fiche qui n'est pas dans le flux — un compteur
/// que l'on peut faire grossir à volonté n'est plus une information.
pub async fn connexion_avis_publier(
    req: HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(claims) => claims,
        Err(response) => return response,
    };
    let Some(id) = body.get("annonce_id").and_then(|v| v.as_str()).map(str::trim) else {
        return reponse_erreur("Indique la fiche notée (« annonce_id »).");
    };
    let Some(note) = body.get("note").and_then(|v| v.as_i64()) else {
        return reponse_erreur("La note est un entier de 1 à 5.");
    };
    if !(1..=5).contains(&note) {
        return reponse_erreur("La note est un entier de 1 à 5.");
    }
    let texte = body.get("texte").and_then(|v| v.as_str()).unwrap_or("").trim();
    if texte.chars().count() > AVIS_TEXTE_MAX {
        return reponse_erreur(&format!(
            "Un avis tient en {} caractères : il raconte une expérience, pas une facture.",
            AVIS_TEXTE_MAX
        ));
    }

    let db = Database::shared("uq_platform_db.json");
    let maintenant = chrono::Utc::now().timestamp();
    let Some(annonce) = lire_annonces(&db).into_iter().find(|a| a.id == id) else {
        return HttpResponse::NotFound().json(json!({
            "status": "error",
            "message": "Aucune fiche publiée de cet identifiant dans le flux."
        }));
    };
    if crate::robinet::meme_compte(&annonce.owner, &claims.sub) {
        return HttpResponse::Forbidden().json(json!({
            "status": "error",
            "message": "On ne note pas sa propre fiche."
        }));
    }

    if let Err(erreur) = db.save_annonce_avis(&annonce.id, &claims.sub, note, texte, maintenant) {
        return HttpResponse::InternalServerError().json(json!({
            "status": "error", "message": erreur
        }));
    }

    let (moyenne, total) = db
        .get_avis_par_fiche()
        .get(&annonce.id)
        .cloned()
        .unwrap_or((0.0, 0));

    HttpResponse::Ok().json(json!({
        "status": "success",
        "annonce_id": annonce.id,
        "note": note,
        "avis_total": total,
        "avis_moyenne": (moyenne * 10.0).round() / 10.0,
        "message": "Avis enregistré : il compte pour une voix, la tienne."
    }))
}

/// GET /api/connexion/avis?annonce_id=… — lire les avis d'une fiche.
///
/// Ce qui sort : la moyenne, le nombre de voix, et les quelques derniers
/// textes. Ni l'identifiant de celui qui note, ni le UQ du compte noté : la
/// fiche publique parle d'un lien, pas des personnes qui l'ont emprunté.
pub async fn connexion_avis_liste(
    req: HttpRequest,
    query: web::Query<std::collections::HashMap<String, String>>,
) -> HttpResponse {
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(claims) => claims,
        Err(response) => return response,
    };
    let id = query.get("annonce_id").map(|v| v.trim()).unwrap_or("");
    if id.is_empty() {
        return reponse_erreur("Indique la fiche lue (« annonce_id »).");
    }
    let db = Database::shared("uq_platform_db.json");
    let annonce = lire_annonces(&db).into_iter().find(|a| a.id == id);
    let Some(annonce) = annonce else {
        return HttpResponse::NotFound().json(json!({
            "status": "error", "message": "Aucune fiche publiée de cet identifiant."
        }));
    };
    let (moyenne, total) = db
        .get_avis_par_fiche()
        .get(id)
        .cloned()
        .unwrap_or((0.0, 0));

    HttpResponse::Ok().json(json!({
        "status": "success",
        "annonce_id": annonce.id,
        "avis_total": total,
        "avis_moyenne": if total > 0 { (moyenne * 10.0).round() / 10.0 } else { 0.0 },
        "avis": db.get_annonce_avis(id, AVIS_LIMITE_AFFICHEE),
        "limite": "Un avis par compte et par fiche ; le noteur n'est pas publié."
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fiche(id: &str, reseau: &str, signal: f64, debit: f64, latence: f64) -> AnnonceConnexion {
        AnnonceConnexion {
            id: id.to_string(),
            owner: format!("UQ-0000000{}", id.chars().next_back().unwrap_or('1')),
            nom: "Fiche".to_string(),
            photo: String::new(),
            reseau: reseau.to_string(),
            latitude: 4.0,
            longitude: 11.0,
            signal,
            debit_mbps: debit,
            latence_ms: latence,
            mesuree_a: 1_000_000,
            publiee: true,
            creee_a: 1_000_000,
            maj_a: 1_000_000,
        }
    }

    #[test]
    fn un_lien_fort_val_mieux_qu_un_lien_faible() {
        let maintenant = 1_000_000 + 60;
        let starlink = puissance_de(95.0, 45.0, 40.0, 60);
        let mobile = puissance_de(30.0, 3.0, 200.0, 60);
        assert!(starlink > mobile + 0.3, "{} vs {}", starlink, mobile);
    }

    #[test]
    fn une_mesure_perimee_ne_s_avance_plus() {
        let fraiche = puissance_de(80.0, 20.0, 50.0, 60);
        let perimee = puissance_de(80.0, 20.0, 50.0, 24 * 60 * 60);
        assert!(perimee < fraiche * 0.3, "{} vs {}", perimee, fraiche);
        // Décote plafonnée : la fiche reste classable, elle ne vaut plus rien.
        assert!(perimee > 0.0);
    }

    #[test]
    fn le_plus_proche_prime_sur_le_plus_performant() {
        let maintenant = 2_000_000;
        let voisin = {
            let mut a = fiche("connexion_voisin", "mobile", 55.0, 8.0, 120.0);
            a.latitude = 4.002;
            a.longitude = 11.002;
            a.mesuree_a = maintenant - 60;
            a
        };
        let lointain = {
            let mut a = fiche("connexion_lointain", "starlink", 98.0, 60.0, 30.0);
            a.latitude = 4.05;
            a.mesuree_a = maintenant - 60;
            a
        };
        let classement = classer(&[lointain, voisin], Some((4.0, 11.0)), 20.0, maintenant);
        assert_eq!(classement.len(), 2);
        assert_eq!(classement[0]["id"], json!("connexion_voisin"));
        assert!(classement[0]["distance_km"].as_f64().unwrap() < 1.0);
        // Le Starlink reste dans la région, il est simplement second : à
        // 5,5 km, sa puissance ne rattrape pas le voisin du coin de la rue.
        assert!(classement[1]["puissance"].as_f64().unwrap() > classement[0]["puissance"].as_f64().unwrap());
    }

    #[test]
    fn a_position_egale_le_plus_puissant_gagne() {
        let maintenant = 3_000_000;
        let solide = {
            let mut a = fiche("connexion_solide", "starlink", 90.0, 40.0, 40.0);
            a.latitude = 4.01;
            a.mesuree_a = maintenant - 30;
            a
        };
        let chancelant = {
            let mut a = fiche("connexion_chancelant", "mobile", 25.0, 2.0, 250.0);
            a.latitude = 4.01;
            a.mesuree_a = maintenant - 30;
            a
        };
        let classement = classer(&[chancelant, solide], Some((4.0, 11.0)), 20.0, maintenant);
        assert_eq!(classement[0]["id"], json!("connexion_solide"));
    }

    #[test]
    fn hors_region_n_est_propose() {
        let maintenant = 4_000_000;
        let excentre = {
            let mut a = fiche("connexion_excentree", "starlink", 99.0, 50.0, 20.0);
            a.latitude = 12.0;
            a.mesuree_a = maintenant;
            a
        };
        assert!(classer(&[excentre], Some((4.0, 11.0)), 20.0, maintenant).is_empty());
    }

    #[test]
    fn sans_position_acheteur_la_distance_reste_nulle() {
        let maintenant = 5_000_000;
        let classement = classer(
            &[fiche("connexion_sans_position_acheteur", "mobile", 60.0, 10.0, 80.0)],
            None,
            20.0,
            maintenant,
        );
        assert_eq!(classement.len(), 1);
        assert!(classement[0].get("distance_km").unwrap().is_null());
        assert_eq!(classement[0]["reseau"], json!("mobile"));
    }

    #[test]
    fn la_fiche_publique_ne_dit_ni_le_uq_ni_la_position() {
        let annonce = fiche("connexion_privee", "starlink", 70.0, 20.0, 60.0);
        let vue = annonce.vue_publique(Some(1.234), annonce.mesuree_a + 60);
        assert!(vue.get("owner").is_none());
        assert!(vue.get("latitude").is_none());
        assert!(vue.get("longitude").is_none());
        assert_eq!(vue["distance_km"], json!(1.2));
    }

    #[test]
    fn le_prix_ne_passe_pas_par_le_serveur() {
        let body = json!({ "nom": "Ma box", "reseau": "mobile", "price_per_mb": 10 });
        assert_eq!(cle_de_prix_presente(&body), Some("price_per_mb"));
        let net = json!({ "nom": "Ma box", "reseau": "mobile" });
        assert_eq!(cle_de_prix_presente(&net), None);
    }

    #[test]
    fn une_fiche_sans_type_de_lien_est_refusee() {
        let body = json!({ "nom": "Forfait partagé", "latitude": 4.0, "longitude": 11.0 });
        let erreur = fiche_from_body("UQ-11111111", &body, 1_000, None).err();
        assert_eq!(
            erreur.as_deref(),
            Some("Le type de lien est « starlink » ou « mobile »")
        );
    }

    #[test]
    fn un_gps_sans_fix_ne_publie_rien() {
        let body = json!({ "nom": "Box", "reseau": "starlink", "latitude": 0.0, "longitude": 0.0 });
        assert!(fiche_from_body("UQ-11111111", &body, 1_000, None).is_err());
    }

    #[test]
    fn le_prix_n_est_jamais_ecrit_dans_la_fiche() {
        let body = json!({
            "nom": "Box du quartier", "reseau": "starlink",
            "latitude": 4.05, "longitude": 11.5, "signal": 80, "debit_mbps": 30
        });
        let annonce = fiche_from_body("UQ-11111111", &body, 1_000, None).unwrap();
        let brut = serde_json::to_string(&annonce).unwrap();
        assert!(!brut.contains("prix"));
        assert!(!brut.contains("price"));
        assert_eq!(annonce.id, "connexion_uq_11111111");
        assert!(annonce.publiee);
    }

    #[test]
    fn un_compte_tient_une_seule_fiche() {
        assert_eq!(
            AnnonceConnexion::id_du_compte("UQ-2303-5436"),
            AnnonceConnexion::id_du_compte("uq-2303-5436")
        );
        assert_eq!(AnnonceConnexion::id_du_compte("UQ-230354360"), "connexion_uq_230354360");
    }

    #[test]
    fn une_fiche_retiree_sort_du_classement() {
        let maintenant = 6_000_000;
        let retires = {
            let mut a = fiche("connexion_retiree", "starlink", 99.0, 50.0, 10.0);
            a.publiee = false;
            a
        };
        let en_ligne = fiche("connexion_en_ligne", "mobile", 40.0, 5.0, 150.0);
        let classement = classer(&[retires, en_ligne], Some((4.0, 11.0)), 20.0, maintenant);
        assert_eq!(classement.len(), 1);
        assert_eq!(classement[0]["id"], json!("connexion_en_ligne"));
    }

    #[test]
    fn un_signal_hors_bornes_ne_fausse_rien() {
        // Un client cassé peut envoyer 5 000 % : la note reste bornée à 1.
        assert!((puissance_de(5000.0, 9000.0, -400.0, 10) - 1.0).abs() < 1e-9);
        assert_eq!(puissance_de(f64::NAN, f64::NAN, f64::NAN, 10), 0.0);
    }

    #[test]
    fn la_boutique_systeme_est_annexee_au_compte_owner() {
        let boutique = boutique_systeme();
        assert_eq!(boutique["id"], json!(BOUTIQUE_ID));
        assert_eq!(boutique["owner"], json!(crate::security_login::OWNER_UQ));
        assert_eq!(boutique["name"], json!(BOUTIQUE_NOM));
        assert_eq!(boutique["confiance"], json!(CONFIANCE_SYSTEME));
        assert_eq!(boutique["confiance_systeme"], json!(true));
        assert_eq!(boutique["systeme"], json!(true));
        // Le titre dit le service, pas la plateforme : « UQ » n'y apparaît pas.
        assert!(!BOUTIQUE_NOM.to_lowercase().contains("uq"), "le titre porte la plateforme");
    }

    #[test]
    fn la_boutique_systeme_ne_porte_ni_prix_ni_position() {
        let boutique = boutique_systeme();
        assert_eq!(cle_de_prix_presente(&boutique), None);
        let brut = boutique.to_string();
        for interdit in ["latitude", "longitude", "\"owner\":\"UQ-1", "prix_par_seconde"] {
            assert!(!brut.contains(interdit), "la fiche système fuit {}", interdit);
        }
        // Ses visuels sont embarqués dans l'application : aucun dépend d'un site tiers.
        for visuel in BOUTIQUE_GALERIE {
            assert!(visuel.starts_with("/assets/connexion/"), "visuel externe {}", visuel);
        }
    }

    #[test]
    fn les_agregats_comptent_le_flux_sans_l_inventer() {
        let maintenant = 1_000_000 + 60;
        let mut perimee = fiche("c4", RESEAU_MOBILE, 60.0, 8.0, 80.0);
        perimee.mesuree_a = maintenant - 24 * 60 * 60;
        let fiches = vec![
            fiche("c1", RESEAU_STARLINK, 90.0, 40.0, 40.0),
            // Deux fiches du même compte (même clef d'identifiant) : la seconde
            // ne double pas le compteur de vendeurs.
            fiche("x1", RESEAU_MOBILE, 50.0, 10.0, 90.0),
            perimee,
        ];

        let vitalite = vitalite_boutique(&fiches, maintenant);
        assert_eq!(vitalite["fiches"], json!(3));
        assert_eq!(vitalite["vendeurs"], json!(2));
        assert_eq!(vitalite["satellite"], json!(1));
        assert_eq!(vitalite["mobile"], json!(2));
        assert_eq!(vitalite["mesures_fraiches"], json!(2));
        assert!(vitalite["derniere_fiche_a"].as_i64().unwrap() > 0);
    }

    #[test]
    fn un_flux_vide_affiche_zero_et_non_une_popularite_prete() {
        let vitalite = vitalite_boutique(&[], 1_000_000);
        assert_eq!(vitalite["fiches"], json!(0));
        assert_eq!(vitalite["vendeurs"], json!(0));
        assert_eq!(vitalite["derniere_fiche_a"], json!(0));
    }

    #[test]
    fn une_fiche_sans_avis_n_affiche_aucune_moyenne() {
        let vue = json!({ "id": "connexion_uq_996697110", "nom": "Fiche" });

        let vu = avec_avis(&Default::default(), vue.clone());
        assert_eq!(vu["avis_total"], json!(0));
        assert_eq!(vu["avis_moyenne"], json!(null));

        let mut agregats = std::collections::HashMap::new();
        agregats.insert("connexion_uq_996697110".to_string(), (4.333_3, 3i64));
        let vu = avec_avis(&agregats, vue);
        assert_eq!(vu["avis_total"], json!(3));
        assert_eq!(vu["avis_moyenne"], json!(4.3));
    }
}
