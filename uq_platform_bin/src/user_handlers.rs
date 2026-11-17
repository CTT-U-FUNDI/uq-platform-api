use crate::database::Database;
use actix_web::{web, HttpResponse};
use serde_json::json;
use crate::user_models::*;

// ==========================================================
// HANDLERS BOUTIQUES
// ==========================================================

/// Galerie du mini-site : 10 visuels au plus, jamais de visuel vide.
fn galerie_depuis(body: &serde_json::Value) -> Vec<String> {
    body.get("gallery")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|media| media.as_str())
                .map(|media| media.trim().to_string())
                .filter(|media| !media.is_empty())
                .take(GALERIE_MAX)
                .collect()
        })
        .unwrap_or_default()
}

/// Sections libres : un titre suffit à exister, le contenu reste facultatif.
fn sections_depuis(body: &serde_json::Value) -> Vec<ShopSection> {
    body.get("sections")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .enumerate()
                .filter_map(|(index, section)| {
                    let titre = section.get("titre").and_then(|v| v.as_str()).unwrap_or("").trim();
                    if titre.is_empty() {
                        return None;
                    }
                    let id = section
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| format!("sec_{}", index + 1));
                    Some(ShopSection::new(
                        &id,
                        titre,
                        section.get("type").and_then(|v| v.as_str()).unwrap_or("texte"),
                        section.get("contenu").and_then(|v| v.as_str()).unwrap_or(""),
                        section.get("media").and_then(|v| v.as_str()).unwrap_or(""),
                    ))
                })
                .take(20)
                .collect()
        })
        .unwrap_or_default()
}

fn texte(body: &serde_json::Value, cle: &str) -> String {
    body.get(cle)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string()
}

/// Le contenu d'une boutique s'écrit à partir du corps de la requête. Les
/// champs absents restent tels quels : une modification partielle ne vide pas
/// la galerie.
fn appliquer_contenu(shop: &mut Shop, body: &serde_json::Value) {
    if body["presentation"].is_object() {
        let presentation = &body["presentation"];
        shop.presentation_photo = texte(presentation, "photo");
        shop.presentation_video = texte(presentation, "video");
    }
    if body["gallery"].is_array() {
        shop.gallery = galerie_depuis(body);
    }
    if body["sections"].is_array() {
        shop.sections = sections_depuis(body);
    }
    if let Some(v) = body.get("site") {
        if v.is_null() {
            shop.site = None;
        } else if let Ok(site) = serde_json::from_value::<MiniSite>(v.clone()) {
            shop.site = Some(site.normalise());
        }
    }
    // Le studio de vitrine (nouveau modèle) est conservé tel quel : c'est lui
    // que l'éditeur relit, et la fiche acheteur s'en sert quand il existe.
    if let Some(v) = body.get("vitrine") {
        shop.vitrine = if v.is_null() { None } else { Some(v.clone()) };
    }
}

/// Visuels de la vitrine dans le flux : l'accroche d'abord, la galerie ensuite.
/// Sans présentation, on montre les trois premiers visuels du catalogue publié.
fn medias_vitrine(db: &Database, shop: &Shop) -> String {
    let candidats = [shop.presentation_video.clone(), shop.presentation_photo.clone()]
        .into_iter()
        .chain(shop.gallery.iter().cloned())
        .map(|media| media.trim().to_string())
        .filter(|media| !media.is_empty())
        .collect::<Vec<String>>();
    let mut medias: Vec<String> = candidats.into_iter().take(4).collect();

    if medias.is_empty() {
        for item in db.get_shop_items(&shop.owner) {
            if item["type"].as_str() == Some("boutique") || item["published"].as_bool() == Some(false) {
                continue;
            }
            let brut = item["media_urls"].as_str().unwrap_or("[]");
            if let Ok(serde_json::Value::Array(tableau)) = serde_json::from_str::<serde_json::Value>(brut) {
                for media in tableau.iter().filter_map(|v| v.as_str()) {
                    if media.starts_with("data:") || media.starts_with("http") {
                        medias.push(media.to_string());
                    }
                    if medias.len() == 3 {
                        break;
                    }
                }
            }
            if medias.len() >= 3 {
                break;
            }
        }
    }

    json!(medias).to_string()
}

/// La ligne `shop_items` qui porte la boutique dans le flux Zando. Créée à la
/// première publication, mise à jour ensuite : une seule vitrine par boutique.
fn ecrire_vitrine(db: &Database, shop: &mut Shop, en_ligne: bool) -> Result<(), String> {
    let medias = medias_vitrine(db, shop);
    if shop.vitrine_item_id.is_empty() {
        let cree = db.create_shop_item(
            &shop.owner, "boutique", &medias, &shop.name, "", &shop.description,
            0, 0.0, &shop.category, false, 0,
        )?;
        shop.vitrine_item_id = cree;
    } else {
        db.update_shop_item(
            &shop.vitrine_item_id, &shop.owner, "boutique", &medias,
            &shop.name, "", &shop.description, 0, 0.0, &shop.category, false, 0,
        )?;
    }

    if en_ligne {
        db.publish_shop_item(&shop.vitrine_item_id, &shop.owner)?;
    } else {
        db.depublier_shop_item(&shop.vitrine_item_id, &shop.owner)?;
    }
    Ok(())
}

/// POST /api/shop/create — une identité, une galerie, des sections, et la
/// présence en ligne est décomptée d'office (7 jours).
pub async fn shop_create(req: actix_web::HttpRequest, body: web::Json<serde_json::Value>) -> HttpResponse {
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(claims) => claims,
        Err(response) => return response,
    };
    let db = Database::shared("uq_platform_db.json");

    let name = texte(&body, "name");
    if name.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Le nom de la boutique est obligatoire"
        }));
    }
    let category = {
        let value = texte(&body, "category");
        if value.is_empty() { "general".to_string() } else { value }
    };
    let lat = body.get("latitude").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let lon = body.get("longitude").and_then(|v| v.as_f64()).unwrap_or(0.0);

    // ====================
    // REGLES BOUTIQUE (politique UQ)
    // - Max 2 boutiques gratuites ; la 3e et suivantes coutent 10 BORB.
    // - Le mode 24/7 exige un compte de +6 mois.
    // - Le mode horaires (ex: 8h-17h) reste gratuit et illimite.
    // ====================
    let boutiques_existantes = db.get_all_shops().iter()
        .filter(|sh| sh.get("owner").and_then(|v| v.as_str()) == Some(claims.sub.as_str()))
        .count() as u64;

    let mode_24_7 = body.get("mode_24_7").and_then(|v| v.as_bool()).unwrap_or(false)
        || body.get("horaire").is_none();

    if mode_24_7 {
        // Regle des 6 mois : pas de 24/7 pour les comptes recents.
        let idx = crate::uq_index::UqIndex::shared("uq_index.db");
        let age_ok = idx.get_account(&claims.sub)
            .and_then(|a| a.get("created_at").and_then(|v| v.as_i64()))
            .map(|ts| chrono::Utc::now().timestamp() - ts >= 183 * 86_400)
            .unwrap_or(false);
        if !age_ok {
            return HttpResponse::Forbidden().json(json!({
                "status":"error","code":"COMPTE_TROP_RECENT",
                "message":"Le mode 24/7 est reserve aux comptes de plus de 6 mois. Choisis des horaires d'ouverture."
            }));
        }
    }

    if boutiques_existantes >= 2 {
        // 3e boutique et + : 10 BORB (1000 points) debites du wallet.
        const PRIX_BOUTIQUE_POINTS: u64 = 1000; // 10 BORB
        let (_b, points) = db.get_balance(&claims.sub);
        if points < PRIX_BOUTIQUE_POINTS {
            return HttpResponse::PaymentRequired().json(json!({
                "status":"error","code":"FRAIS_BOUTIQUE_REQUIS",
                "message":"A partir de la 3e boutique, 10 BORB sont requis. Recharge ton wallet.",
                "prix_borb": 10, "solde_points": points
            }));
        }
        db.record_transaction(&claims.sub, "TREASURY", 0, PRIX_BOUTIQUE_POINTS);
    }

    let mut shop = Shop::new(&claims.sub, &name, &texte(&body, "description"), &category, lat, lon);
    appliquer_contenu(&mut shop, &body);
    shop.open = body.get("en_ligne").and_then(|v| v.as_bool()).unwrap_or(true);
    let en_ligne = shop.open;

    if let Err(error) = ecrire_vitrine(&db, &mut shop, en_ligne) {
        return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("Vitrine impossible à publier: {}", error)
        }));
    }
    db.save_publication(serde_json::to_value(&shop).unwrap_or(json!({})));

    HttpResponse::Ok().json(json!({
        "status": "success",
        "shop": shop,
        "frais": apercu_frais(&db, &shop)
    }))
}

/// Le frais de présence tel que le robinet le prélève : 5 % du catalogue pour
/// 24 heures, et le débit seconde par seconde qui en découle.
fn apercu_frais(db: &Database, shop: &Shop) -> serde_json::Value {
    // Les `price_borb` du catalogue sont en BORB ; les champs rendus ici sont en
    // points (le centième de BORB débité dans le coffre), comme au tick.
    let catalogue_points = db.valeur_catalogue(&shop.owner).max(0).saturating_mul(100);
    json!({
        "pourcent": FRAIS_VITRINE_POURCENT,
        "catalogue_points": catalogue_points,
        "frais_24h_points": crate::vitrines::points_dus(catalogue_points, DUREE_RENOUVELLEMENT_SECONDES),
        "par_seconde_points": crate::vitrines::tarif_par_seconde(catalogue_points),
        "duree_secondes": DUREE_RENOUVELLEMENT_SECONDES,
        "robinet_ouvert": shop.robinet,
        "regle_points": shop.regle_points,
        "secondes_en_attente": shop.secondes_en_attente
    })
}

/// POST /api/shop/{id}/update — la galerie reste modifiable, l'abonnement de
/// présence ne se prolonge pas pour autant.
pub async fn shop_update(
    req: actix_web::HttpRequest,
    path: web::Path<String>,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(claims) => claims,
        Err(response) => return response,
    };
    let db = Database::shared("uq_platform_db.json");
    let shop_id = path.into_inner();

    // La boutique du système est dérivée, pas stockée : même le compte owner
    // ne peut pas la renommer ni vider sa galerie depuis un Studio.
    if shop_id == crate::connexion::BOUTIQUE_ID {
        return HttpResponse::Forbidden().json(json!({
            "status": "error",
            "message": "La boutique système n'est pas modifiable : elle se remplit des fiches publiées par les vendeurs."
        }));
    }

    let mut shop = match lire_boutique(&db, &shop_id) {
        Some(shop) => shop,
        None => return HttpResponse::NotFound().json(json!({
            "status": "error", "message": "Boutique introuvable"
        })),
    };
    if shop.owner != claims.sub {
        return HttpResponse::Forbidden().json(json!({
            "status": "error", "message": "Seul le propriétaire modifie sa boutique"
        }));
    }

    let nom = texte(&body, "name");
    if !nom.is_empty() { shop.name = nom; }
    let description = texte(&body, "description");
    if body.get("description").is_some() { shop.description = description; }
    let category = texte(&body, "category");
    if !category.is_empty() { shop.category = category; }
    appliquer_contenu(&mut shop, &body);
    if let Some(ligne) = body.get("en_ligne").and_then(|v| v.as_bool()) {
        shop.open = ligne;
    }
    // Redescendre `en_ligne` sur une vitrine dont le compteur est épuisé ne
    // rend pas le flux gratuit : la présence se prolonge par le geste PIN de
    // `/api/shop/{id}/renew`, pas par une modification de la galerie.
    let maintenant = chrono::Utc::now().timestamp();
    let en_ligne = shop.open && shop.est_en_ligne(maintenant);

    if let Err(error) = ecrire_vitrine(&db, &mut shop, en_ligne) {
        return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("Mise à jour de la vitrine impossible: {}", error)
        }));
    }

    db.save_publication(serde_json::to_value(&shop).unwrap_or(json!({})));

    HttpResponse::Ok().json(json!({
        "status": "success",
        "shop": shop,
        "frais": apercu_frais(&db, &shop)
    }))
}

fn lire_boutique(db: &Database, shop_id: &str) -> Option<Shop> {
    db.get_shop_by_id(shop_id)
        .and_then(|value| serde_json::from_value::<Shop>(value).ok())
}

/// GET /api/shop/{id} — la boutique réelle, son compteur et le frais à venir.
pub async fn shop_get(path: web::Path<String>) -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.synchroniser_vitrines_expirees();
    let shop_id = path.into_inner();

    // La boutique système n'existe pas en base : elle est construite à la
    // lecture, sans compteur de présence ni frais — elle ne s'achète rien.
    if shop_id == crate::connexion::BOUTIQUE_ID {
        return HttpResponse::Ok().json(json!({
            "status": "success",
            "shop": crate::connexion::boutique_systeme(),
            "en_ligne": true,
            "systeme": true
        }));
    }

    match lire_boutique(&db, &shop_id) {
        Some(shop) => {
            let maintenant = chrono::Utc::now().timestamp();
            HttpResponse::Ok().json(json!({
                "status": "success",
                "shop": shop,
                "en_ligne": shop.est_en_ligne(maintenant),
                "secondes_restantes": shop.secondes_restantes(maintenant),
                "frais": apercu_frais(&db, &shop)
            }))
        }
        None => HttpResponse::NotFound().json(json!({
            "status": "error", "message": "Boutique introuvable"
        })),
    }
}

pub async fn shop_list() -> HttpResponse {
    let db = Database::shared("uq_platform_db.json");
    db.synchroniser_vitrines_expirees();
    let shops = db.get_all_shops();
    HttpResponse::Ok().json(json!({"shops": shops, "total": shops.len()}))
}

pub async fn product_add(body: web::Json<serde_json::Value>) -> HttpResponse {
    let shop_id = body.get("shop_id").and_then(|v| v.as_str()).unwrap_or("");
    let name = body.get("name").and_then(|v| v.as_str()).unwrap_or("Produit");
    let price = body.get("price_points").and_then(|v| v.as_u64()).unwrap_or(0);
    let stock = body.get("stock").and_then(|v| v.as_u64()).unwrap_or(1);

    let product = Product::new(shop_id, name, price, stock);

    // SAUVEGARDER DANS LA BD RÉELLE
    let db = Database::shared("uq_platform_db.json");
    db.save_publication(json!(product));

    HttpResponse::Ok().json(json!({
        "status": "success",
        "product": product
    }))
}

pub async fn product_buy(body: web::Json<serde_json::Value>) -> HttpResponse {
    HttpResponse::Ok().json(json!({"status": "success", "message": "Produit acheté", "escrow_created": true}))
}

// ==========================================================
// HANDLERS RECHERCHE + GÉOLOCALISATION
// ==========================================================

pub async fn search(query: web::Query<std::collections::HashMap<String, String>>) -> HttpResponse {
    let q = query.get("q").cloned().unwrap_or_default();
    let search_type = query.get("type").cloned().unwrap_or_else(|| "all".to_string());
    let max_distance = query.get("distance").and_then(|d| d.parse::<f64>().ok()).unwrap_or(10.0);
    let min_price = query.get("price_min").and_then(|p| p.parse::<u64>().ok()).unwrap_or(0);
    let max_price = query.get("price_max").and_then(|p| p.parse::<u64>().ok()).unwrap_or(u64::MAX);

    let db = Database::shared("uq_platform_db.json");
    let offers = db.get_all_offers();
    let mut results = Vec::new();

    for offer in &offers {
        if offer.description.contains(&q) || q.is_empty() {
            results.push(json!({
                "type": "offer",
                "id": offer.id,
                "seller": offer.seller,
                "description": offer.description,
                "price_per_mb": offer.price_per_mb
            }));
        }
    }

    HttpResponse::Ok().json(json!({
        "query": q,
        "type": search_type,
        "max_distance_km": max_distance,
        "price_range": [min_price, max_price],
        "results": results,
        "total": results.len()
    }))
}

pub async fn search_nearby(query: web::Query<std::collections::HashMap<String, String>>) -> HttpResponse {
    HttpResponse::Ok().json(json!({"results": [], "total": 0}))
}

pub async fn location_update(body: web::Json<serde_json::Value>) -> HttpResponse {
    HttpResponse::Ok().json(json!({"status": "success", "location_updated": true}))
}

pub async fn location_distance(body: web::Json<serde_json::Value>) -> HttpResponse {
    HttpResponse::Ok().json(json!({"status": "success", "distance_km": 0.0}))
}

// ==========================================================
// HANDLERS CONNEXION + PARRAINAGE
// ==========================================================

pub async fn connect_sell(body: web::Json<serde_json::Value>) -> HttpResponse {
    let seller = body.get("seller").and_then(|v| v.as_str()).unwrap_or("anonymous");
    let network_type = body.get("type").and_then(|v| v.as_str()).unwrap_or("4G");
    let price_per_mb = body.get("price_per_mb").and_then(|v| v.as_u64()).unwrap_or(10);

    HttpResponse::Ok().json(json!({
        "status": "success",
        "seller": seller,
        "network_type": network_type,
        "price_per_mb": price_per_mb,
        "active": true
    }))
}

pub async fn connect_buy(body: web::Json<serde_json::Value>) -> HttpResponse {
    HttpResponse::Ok().json(json!({"status": "success", "connection_purchased": true}))
}

pub async fn connect_sellers() -> HttpResponse {
    HttpResponse::Ok().json(json!({"sellers": [], "total": 0}))
}

pub async fn referral_create(body: web::Json<serde_json::Value>) -> HttpResponse {
    HttpResponse::Ok().json(json!({"status": "success", "referral_link": "https://uq/ref/ABC123"}))
}

pub async fn referral_stats(path: web::Path<String>) -> HttpResponse {
    HttpResponse::Ok().json(json!({"user_id": path.into_inner(), "referrals": 0, "earnings": 0}))
}

pub async fn referral_claim(body: web::Json<serde_json::Value>) -> HttpResponse {
    HttpResponse::Ok().json(json!({"status": "success", "claimed": true}))
}

// ==========================================================
// HANDLERS NOTIFICATIONS + DASHBOARD
// ==========================================================

pub async fn notifications_list(req: actix_web::HttpRequest, path: web::Path<String>) -> HttpResponse {
    // 🔐 SÉCURITÉ (vague 1) — USER : accès à SES données (ou owner)
    let __cible = path.to_string();
    if let Err(r) = crate::auth_guard::exiger_soi_ou_owner(&req, &__cible) { return r; }

    let uid = path.into_inner();
    let db = Database::shared("uq_platform_db.json");
    let notifs = db.get_notifications(&uid);
    HttpResponse::Ok().json(json!({"user_id": uid, "notifications": notifs}))
}

pub async fn notifications_read(body: web::Json<serde_json::Value>) -> HttpResponse {
    HttpResponse::Ok().json(json!({"status": "success", "marked_read": true}))
}

pub async fn dashboard(req: actix_web::HttpRequest, path: web::Path<String>) -> HttpResponse {
    // 🔐 SÉCURITÉ (vague 1) — USER : accès à SES données (ou owner)
    let __cible = path.to_string();
    if let Err(r) = crate::auth_guard::exiger_soi_ou_owner(&req, &__cible) { return r; }

    let user_id = path.into_inner();
    let db = Database::shared("uq_platform_db.json");
    let stats = db.get_dashboard(&user_id);
    HttpResponse::Ok().json(json!({
        "user_id": user_id,
        "stats": {
            "balance_bubbles": 0,
            "balance_points": 0,
            "value_usd": 0.0,
            "transactions_count": 0,
            "messages_unread": 0,
            "offers_active": 0,
            "referral_count": 0,
            "referral_earnings": 0,
            "ubi_claimed_today": false,
            "ubi_total_claimed": 0
        }
    }))
}