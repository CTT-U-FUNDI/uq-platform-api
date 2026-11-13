//! Binance P2P — carnet d'ordres public (lecture seule, sans clé API).
//!
//! Source : endpoint public non officiel de Binance P2P
//!   POST https://p2p.binance.com/bapi/c2c/v2/friendly/c2c/adv/search
//!
//! Ce module récupère les offres P2P (BUY/SELL) pour les paires fiat/crypto
//! les plus courantes (USD, CDF, EUR, etc.) et les expose sous forme de
//! carnet d'ordres normalisé pour l'app UQ et le wallet BORB.
//!
//! Cache : 30 secondes (les prix P2P bougent vite).
//! Aucune clé API requise — endpoint public.
//! AUCUNE exécution de trade : lecture seule du carnet d'ordres.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

const BINANCE_P2P_URL: &str =
    "https://p2p.binance.com/bapi/c2c/v2/friendly/c2c/adv/search";
const CACHE_SECS: i64 = 30;
const CACHE_FILE: &str = "data/binance_p2p_cache.json";

// ====================
// TYPES
// ====================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OffreP2P {
    pub id: String,
    pub side: String,        // "BUY" ou "SELL" (du point de vue du vendeur Binance)
    pub asset: String,       // "BTC", "ETH", etc.
    pub fiat: String,        // "USD", "CDF", "EUR", etc.
    pub price: f64,          // Prix dans la devise fiat
    pub amount: f64,         // Quantité de crypto disponible
    pub min_amount: f64,     // Montant fiat minimum
    pub max_amount: f64,     // Montant fiat maximum
    pub merchant_name: String,
    pub merchant_orders: u32,
    pub merchant_rate: f64,  // Taux de complétion (%)
    pub payment_methods: Vec<String>,
    pub online_time: String,
    pub country: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheEntry {
    pub key: String,
    pub offers: Vec<OffreP2P>,
    pub at: i64,
}

// ====================
// CACHE
// ====================

fn cache_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn charger_cache() -> HashMap<String, CacheEntry> {
    let _g = cache_lock().lock().unwrap_or_else(|e| e.into_inner());
    std::fs::read_to_string(CACHE_FILE)
        .ok()
        .and_then(|c| serde_json::from_str::<HashMap<String, CacheEntry>>(&c).ok())
        .unwrap_or_default()
}

fn ecrire_cache(cache: &HashMap<String, CacheEntry>) {
    let _g = cache_lock().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(parent) = std::path::Path::new(CACHE_FILE).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_vec(cache) {
        let _ = std::fs::write(CACHE_FILE, json);
    }
}

fn cache_frais(key: &str) -> Option<CacheEntry> {
    let c = charger_cache();
    let e = c.get(key)?;
    if chrono::Utc::now().timestamp() - e.at < CACHE_SECS {
        Some(e.clone())
    } else {
        None
    }
}

fn cache_perime(key: &str) -> Option<CacheEntry> {
    charger_cache().get(key).cloned()
}

fn enregistrer_cache(key: &str, offers: Vec<OffreP2P>) {
    let _g = cache_lock().lock().unwrap_or_else(|e| e.into_inner());
    let mut c = charger_cache();
    c.insert(
        key.to_string(),
        CacheEntry {
            key: key.to_string(),
            offers,
            at: chrono::Utc::now().timestamp(),
        },
    );
    if c.len() > 50 {
        // Garder les 50 entrées les plus récentes
        let mut v: Vec<_> = c.values().cloned().collect();
        v.sort_by_key(|e| -e.at);
        v.truncate(50);
        let m: HashMap<String, CacheEntry> =
            v.into_iter().map(|e| (e.key.clone(), e)).collect();
        ecrire_cache(&m);
    } else {
        ecrire_cache(&c);
    }
}

// ====================
// REQUÊTE BINANCE P2P
// ====================

/// Corps de la requête POST Binance P2P.
fn corps_requete(asset: &str, fiat: &str, trade_type: &str, page: u32, rows: u32) -> Value {
    json!({
        "asset": asset,
        "fiat": fiat,
        "tradeType": trade_type,
        "page": page,
        "rows": rows,
        "payTypes": [],
        "countries": [],
        "proMerchantAds": false,
        "shieldMerchantAds": false,
        "countryCode": "",
        "assetAmount": 0
    })
}

/// Parse la réponse Binance P2P → Vec<OffreP2P>.
/// Format réel : {"code":"000000","data":[{"adv":{...}},...]}
fn parser_reponse(data: &Value, asset: &str, fiat: &str, side: &str) -> Vec<OffreP2P> {
    let mut offres = Vec::new();

    // data est un tableau d'objets {"adv": {...}}
    let Some(ads_array) = data.get("data").and_then(|d| d.as_array()) else {
        return offres;
    };

    for ad in ads_array {
        let Some(adv) = ad.get("adv") else { continue };
        let Some(adv_obj) = adv.as_object() else { continue };

        let id = adv_obj
            .get("advNo")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let price: f64 = adv_obj
            .get("price")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .or_else(|| adv_obj.get("price").and_then(|v| v.as_f64()))
            .unwrap_or(0.0);

        let amount: f64 = adv_obj
            .get("surplusAmount")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .or_else(|| adv_obj.get("tradableQuantity").and_then(|v| v.as_str()).and_then(|s| s.parse().ok()))
            .unwrap_or(0.0);

        let min_amount: f64 = adv_obj
            .get("minSingleTransAmount")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0);

        let max_amount: f64 = adv_obj
            .get("dynamicMaxSingleTransAmount")
            .or_else(|| adv_obj.get("maxSingleTransAmount"))
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0);

        // Méthodes de paiement
        let pay_types: Vec<String> = adv_obj
            .get("tradeMethods")
            .and_then(|t| t.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|t| {
                        t.get("tradeMethodName")
                            .and_then(|v| v.as_str())
                            .or_else(|| t.get("payType").and_then(|v| v.as_str()))
                            .map(String::from)
                    })
                    .collect()
            })
            .unwrap_or_default();

        // Temps de paiement (minutes)
        let pay_time = adv_obj
            .get("payTimeLimit")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        // Classification du marchand (profession vs ordinaire)
        let classify = adv_obj
            .get("classify")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // ID de l'annonceur (pas de nom complet dans cet endpoint)
        let advertiser_id = adv_obj
            .get("advertiser")
            .and_then(|a| a.as_str())
            .unwrap_or("")
            .to_string();

        if price > 0.0 && amount > 0.0 {
            offres.push(OffreP2P {
                id,
                side: side.to_string(),
                asset: asset.to_string(),
                fiat: fiat.to_string(),
                price,
                amount,
                min_amount,
                max_amount,
                merchant_name: if classify == "profession" {
                    format!("Marchand Pro {}", &advertiser_id[..advertiser_id.len().min(8)])
                } else {
                    format!("Particulier {}", &advertiser_id[..advertiser_id.len().min(8)])
                },
                merchant_orders: 0, // non disponible dans cet endpoint
                merchant_rate: 0.0, // non disponible dans cet endpoint
                payment_methods: pay_types,
                online_time: format!("{}min", pay_time),
                country: String::new(),
            });
        }
    }

    offres
}

// ====================
// API PUBLIQUE
// ====================

/// Récupère les offres P2P Binance pour une paire asset/fiat.
/// `trade_type` : "BUY" (acheter crypto avec fiat) ou "SELL" (vendre crypto pour fiat).
pub async fn offres_binance_p2p(
    asset: &str,
    fiat: &str,
    trade_type: &str,
    page: u32,
) -> serde_json::Value {
    let asset = asset.trim().to_uppercase();
    let fiat = fiat.trim().to_uppercase();
    let trade_type = trade_type.trim().to_uppercase();

    if asset.is_empty() || fiat.is_empty() {
        return json!({"status":"error","message":"asset et fiat requis"});
    }
    if !matches!(trade_type.as_str(), "BUY" | "SELL") {
        return json!({"status":"error","message":"tradeType doit être BUY ou SELL"});
    }

    let page = page.max(1);
    let key = format!("{}:{}:{}:{}", asset, fiat, trade_type, page);

    // Cache frais
    if let Some(e) = cache_frais(&key) {
        return json!({
            "status":"success",
            "asset": asset,
            "fiat": fiat,
            "trade_type": trade_type,
            "page": page,
            "cached": true,
            "age_sec": chrono::Utc::now().timestamp() - e.at,
            "offers": e.offers,
            "count": e.offers.len()
        });
    }

    // Requête Binance P2P — curl en subprocess (reqwest bloqué par Binance)
    let body = corps_requete(&asset, &fiat, &trade_type, page, 10);

    match requete_binance_p2p(&body).await {
        Ok(data) => {
            let mut offres = parser_reponse(&data, &asset, &fiat, &trade_type);

            // Trier par prix (BUY: plus bas d'abord, SELL: plus haut d'abord)
            if trade_type == "BUY" {
                offres.sort_by(|a, b| a.price.partial_cmp(&b.price).unwrap_or(std::cmp::Ordering::Equal));
            } else {
                offres.sort_by(|a, b| b.price.partial_cmp(&a.price).unwrap_or(std::cmp::Ordering::Equal));
            }

            if !offres.is_empty() {
                enregistrer_cache(&key, offres.clone());
            }

            json!({
                "status":"success",
                "asset": asset,
                "fiat": fiat,
                "trade_type": trade_type,
                "page": page,
                "cached": false,
                "offers": offres,
                "count": offres.len()
            })
        }
        Err(_) => repli(&key, &asset, &fiat, &trade_type, page),
    }
}

/// Requête Binance P2P via curl subprocess (contourne le blocage reqwest).
async fn requete_binance_p2p(body: &Value) -> Result<Value, String> {
    let body_str = serde_json::to_string(body).map_err(|e| e.to_string())?;

    // Windows : chemin complet curl.exe ; Unix : curl dans PATH
    let curl_bin = if cfg!(windows) {
        "C:\\Windows\\System32\\curl.exe"
    } else {
        "curl"
    };

    let output = tokio::process::Command::new(curl_bin)
        .args([
            "-s",
            "-X", "POST",
            BINANCE_P2P_URL,
            "-H", "Content-Type: application/json",
            "-H", "User-Agent: Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
            "-H", "Accept: application/json",
            "-d", &body_str,
            "--max-time", "15",
        ])
        .output()
        .await
        .map_err(|e| format!("curl failed: {}", e))?;

    if !output.status.success() {
        return Err(format!("curl exit code: {}", output.status.code().unwrap_or(-1)));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&stdout).map_err(|e| format!("JSON parse error: {}", e))
}

fn repli(key: &str, asset: &str, fiat: &str, trade_type: &str, page: u32) -> serde_json::Value {
    match cache_perime(key) {
        Some(e) => json!({
            "status":"success",
            "asset": asset,
            "fiat": fiat,
            "trade_type": trade_type,
            "page": page,
            "cached": true,
            "stale": true,
            "age_sec": chrono::Utc::now().timestamp() - e.at,
            "offers": e.offers,
            "count": e.offers.len(),
            "message":"Binance P2P injoignable — dernières offres connues servies"
        }),
        None => json!({
            "status":"error",
            "asset": asset,
            "fiat": fiat,
            "trade_type": trade_type,
            "message":"Binance P2P indisponible et aucun cache"
        }),
    }
}

/// Résumé multi-paires : BTC, ETH, USDT, BNB pour USD et CDF.
pub async fn resume_binance_p2p() -> serde_json::Value {
    let paires = [
        ("BTC", "USD"),
        ("BTC", "CDF"),
        ("ETH", "USD"),
        ("ETH", "CDF"),
        ("USDT", "CDF"),
        ("BNB", "USD"),
    ];

    let mut resultats = Vec::new();
    for (asset, fiat) in paires {
        // Pour chaque paire, on prend le meilleur prix BUY et SELL
        let buy_key = format!("{}:{}:BUY:1", asset, fiat);
        let sell_key = format!("{}:{}:SELL:1", asset, fiat);

        let buy_offres = cache_perime(&buy_key).map(|e| e.offers).unwrap_or_default();
        let sell_offres = cache_perime(&sell_key).map(|e| e.offers).unwrap_or_default();

        let best_buy = buy_offres.first().map(|o| o.price);
        let best_sell = sell_offres.first().map(|o| o.price);

        resultats.push(json!({
            "asset": asset,
            "fiat": fiat,
            "best_buy_price": best_buy,
            "best_sell_price": best_sell,
            "buy_count": buy_offres.len(),
            "sell_count": sell_offres.len(),
            "buy_merchant": buy_offres.first().map(|o| &o.merchant_name),
            "sell_merchant": sell_offres.first().map(|o| &o.merchant_name),
        }));
    }

    json!({
        "status": "success",
        "timestamp": chrono::Utc::now().timestamp(),
        "pairs": resultats
    })
}

/// État du cache Binance P2P.
pub fn etat_cache() -> serde_json::Value {
    let c = charger_cache();
    let maintenant = chrono::Utc::now().timestamp();
    let mut entrees: Vec<serde_json::Value> = c
        .values()
        .map(|e| {
            json!({
                "key": e.key,
                "offers": e.offers.len(),
                "age_sec": maintenant - e.at,
                "frais": maintenant - e.at < CACHE_SECS
            })
        })
        .collect();
    entrees.sort_by(|a, b| a["key"].as_str().cmp(&b["key"].as_str()));

    json!({
        "status": "success",
        "cache_secondes": CACHE_SECS,
        "entrees": entrees.len(),
        "detail": entrees
    })
}

// ====================
// HANDLERS HTTP (actix-web)
// ====================

use actix_web::{web, HttpRequest, HttpResponse};

/// GET /api/binance-p2p/offers?asset=BTC&fiat=USD&side=BUY&page=1
pub async fn offres_binance_p2p_handler(
    _req: HttpRequest,
    query: web::Query<HashMap<String, String>>,
) -> HttpResponse {
    let asset = query.get("asset").cloned().unwrap_or_default();
    let fiat = query.get("fiat").cloned().unwrap_or_default();
    let side = query.get("side").or_else(|| query.get("trade_type")).cloned().unwrap_or_else(|| "BUY".to_string());
    let page = query.get("page").and_then(|p| p.parse().ok()).unwrap_or(1);

    if asset.is_empty() || fiat.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status":"error","message":"asset et fiat requis (ex: ?asset=BTC&fiat=USD&side=BUY)"
        }));
    }

    let result = offres_binance_p2p(&asset, &fiat, &side, page).await;
    if result.get("status").and_then(|s| s.as_str()) == Some("error") {
        HttpResponse::ServiceUnavailable().json(result)
    } else {
        HttpResponse::Ok().json(result)
    }
}

/// GET /api/binance-p2p/resume
pub async fn resume_binance_p2p_handler(_req: HttpRequest) -> HttpResponse {
    HttpResponse::Ok().json(resume_binance_p2p().await)
}

/// GET /api/binance-p2p/cache
pub async fn etat_cache_handler(_req: HttpRequest) -> HttpResponse {
    HttpResponse::Ok().json(etat_cache())
}
