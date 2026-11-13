// ====================
// 🌐 EXTERNAL API — Crypto / Fiat / Bourse (Mission 7.5)
// ====================
//
// Sources GRATUITES, sans clé :
//   Crypto : Binance → CoinGecko → Coinbase   (repli en cascade)
//   Fiat   : open.er-api.com                  (déjà utilisé par l'oracle)
//   Bourse : Yahoo Finance
//   Temps réel forex/or/crypto : biquote.io (sans clé, 15 000 req/min)
//   Or de référence : xaus.com (sans clé, cache 5 min)
//
// Principe : on tente la 1ʳᵉ source, on bascule sur la suivante en cas
// d'échec, et on met en cache 60 s. Si TOUT échoue (hors-ligne), on sert
// le dernier cache connu — l'interface n'est jamais bloquée.
// ====================

use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

const CACHE_FILE: &str = "data/external_cache.json";
const CACHE_SECS: i64 = 60;
const CACHE_SECS_CRYPTO: i64 = 10;
/// Prix de référence (or) : la source ne bouge pas assez vite pour justifier
/// un appel par minute.
const CACHE_SECS_OR: i64 = 300;

/// Clé de cache du spot d'or, prix de référence de la parité BORB.
pub const CLE_SPOT_OR: &str = "xaus:spot";

/// Clé de cache du prix d'un symbole crypto (`BTCUSD` → `crypto:BTCUSD`).
pub fn cle_crypto(symbole: &str) -> String {
    format!("crypto:{}", symbole.trim().to_uppercase())
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EntreeCache {
    pub cle: String,
    pub source: String,
    pub valeur: serde_json::Value,
    pub at: i64,
}

fn cache_lock() -> &'static Mutex<()> {
    static VERROU: OnceLock<Mutex<()>> = OnceLock::new();
    VERROU.get_or_init(|| Mutex::new(()))
}

/// Horodatage (unix) de la dernière capture réellement persistée pour une clé,
/// `None` si la source n'a jamais répondu. C'est ce qui survivait au redémarrage
/// du serveur sans être publié : dire « ce prix vient du cache » exige de savoir
/// de quand il date.
pub fn capture_persistee_le(cle: &str) -> Option<i64> {
    charger_cache().get(cle).map(|entree| entree.at)
}

fn charger_cache() -> HashMap<String, EntreeCache> {
    let _garde = cache_lock()
        .lock()
        .unwrap_or_else(|erreur| erreur.into_inner());
    charger_cache_sans_verrou()
}

fn charger_cache_sans_verrou() -> HashMap<String, EntreeCache> {
    std::fs::read_to_string(CACHE_FILE)
        .ok()
        .and_then(|c| serde_json::from_str::<HashMap<String, EntreeCache>>(&c).ok())
        .unwrap_or_default()
}

fn ecrire_cache(
    cle: &str,
    source: &str,
    valeur: serde_json::Value,
) -> Result<EntreeCache, std::io::Error> {
    let _garde = cache_lock()
        .lock()
        .unwrap_or_else(|erreur| erreur.into_inner());
    let entree = EntreeCache {
        cle: cle.to_string(),
        source: source.to_string(),
        valeur,
        at: chrono::Utc::now().timestamp(),
    };
    let mut c = charger_cache_sans_verrou();
    c.insert(cle.to_string(), entree.clone());
    if c.len() > 200 {
        let mut v: Vec<_> = c.values().cloned().collect();
        v.sort_by_key(|e| -e.at);
        v.truncate(200);
        let m: HashMap<String, EntreeCache> = v.into_iter().map(|e| (e.cle.clone(), e)).collect();
        ecrire_cache_disque(&m)?;
        return Ok(entree);
    }
    ecrire_cache_disque(&c)?;
    Ok(entree)
}

fn ecrire_cache_disque(cache: &HashMap<String, EntreeCache>) -> Result<(), std::io::Error> {
    if let Some(parent) = std::path::Path::new(CACHE_FILE).parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_vec(cache).map_err(std::io::Error::other)?;
    std::fs::write(CACHE_FILE, json)
}

fn capture_meta(entree: &EntreeCache, persistee: bool) -> serde_json::Value {
    let age = chrono::Utc::now()
        .timestamp()
        .saturating_sub(entree.at)
        .max(0);
    let ttl = ttl_de_la_cle(&entree.cle);
    json!({
        "captured_at": entree.at,
        "age_sec": age,
        "ttl_sec": ttl,
        "fresh": age < ttl,
        "persisted": persistee,
    })
}

fn enregistrer_capture(cle: &str, source: &str, valeur: serde_json::Value) -> serde_json::Value {
    match ecrire_cache(cle, source, valeur) {
        Ok(entree) => capture_meta(&entree, true),
        Err(erreur) => {
            log::error!("Impossible de persister la capture externe {cle}: {erreur}");
            let entree = EntreeCache {
                cle: cle.to_string(),
                source: source.to_string(),
                valeur: serde_json::Value::Null,
                at: chrono::Utc::now().timestamp(),
            };
            let mut meta = capture_meta(&entree, false);
            meta["error"] = json!("La dernière capture n'a pas pu être sauvegardée.");
            meta
        }
    }
}

fn cache_frais(cle: &str) -> Option<EntreeCache> {
    cache_frais_dans(cle, CACHE_SECS)
}

/// Variante à TTL choisi : l'or de référence n'a pas besoin d'être
/// rafraîchi aussi souvent qu'un tick temps réel.
fn cache_frais_dans(cle: &str, ttl: i64) -> Option<EntreeCache> {
    let c = charger_cache();
    let e = c.get(cle)?;
    if chrono::Utc::now().timestamp() - e.at < ttl {
        Some(e.clone())
    } else {
        None
    }
}

fn cache_perime(cle: &str) -> Option<EntreeCache> {
    charger_cache().get(cle).cloned()
}

async fn client() -> Option<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .user_agent("UQ-Platform/1.0")
        .build()
        .ok()
}

// ====================
// CRYPTO
// ====================

fn crypto_symbol_valide(symbole: &str) -> bool {
    (3..=20).contains(&symbole.len())
        && symbole
            .chars()
            .all(|caractere| caractere.is_ascii_alphanumeric())
}

fn crypto_intervalle_valide(intervalle: &str) -> bool {
    matches!(
        intervalle,
        "1s" | "1m"
            | "3m"
            | "5m"
            | "15m"
            | "30m"
            | "1h"
            | "2h"
            | "4h"
            | "6h"
            | "8h"
            | "12h"
            | "1d"
            | "3d"
            | "1w"
            | "1M"
    )
}

/// Prix d'un symbole (ex. « BTCUSDT ») : Binance → CoinGecko → Coinbase.
pub async fn prix_crypto(symbole: &str) -> serde_json::Value {
    let symbole = symbole.trim().to_uppercase();
    if !crypto_symbol_valide(&symbole) {
        return json!({"status":"error","message":"Symbole crypto invalide"});
    }
    let cle = cle_crypto(&symbole);

    if let Some(e) = cache_frais_dans(&cle, CACHE_SECS_CRYPTO) {
        let capture = capture_meta(&e, true);
        return json!({"status":"success","symbol":symbole,"source":e.source,"cached":true,"capture":capture,"data":e.valeur});
    }

    let Some(cli) = client().await else {
        return repli(&cle, &symbole);
    };

    // 1. Binance
    if let Ok(r) = cli
        .get(format!(
            "https://api.binance.com/api/v3/ticker/24hr?symbol={}",
            symbole
        ))
        .send()
        .await
    {
        if let Ok(v) = r.json::<serde_json::Value>().await {
            if let Some(prix) = v
                .get("lastPrice")
                .and_then(|value| value.as_str())
                .and_then(|value| value.parse::<f64>().ok())
                .filter(|value| value.is_finite() && *value > 0.0)
            {
                let data = json!({
                    "price": prix,
                    "pair": symbole,
                    "currency": if symbole.ends_with("USDT") { "USDT" } else { "USD" },
                    "change_percent": v.get("priceChangePercent").and_then(|value| value.as_str()).and_then(|value| value.parse::<f64>().ok()),
                    "open": v.get("openPrice").and_then(|value| value.as_str()).and_then(|value| value.parse::<f64>().ok()),
                    "day_high": v.get("highPrice").and_then(|value| value.as_str()).and_then(|value| value.parse::<f64>().ok()),
                    "day_low": v.get("lowPrice").and_then(|value| value.as_str()).and_then(|value| value.parse::<f64>().ok()),
                    "volume": v.get("volume").and_then(|value| value.as_str()).and_then(|value| value.parse::<f64>().ok())
                });
                let capture = enregistrer_capture(&cle, "binance", data.clone());
                return json!({"status":"success","symbol":symbole,"source":"binance","cached":false,"capture":capture,"data":data});
            }
        }
    }

    // 2. CoinGecko (BTC → bitcoin)
    let base = symbole.trim_end_matches("USDT").trim_end_matches("USD");
    let id = match base {
        "BTC" => "bitcoin",
        "ETH" => "ethereum",
        "BNB" => "binancecoin",
        "SOL" => "solana",
        "XRP" => "ripple",
        "ADA" => "cardano",
        "DOGE" => "dogecoin",
        "TRX" => "tron",
        "TON" => "the-open-network",
        "AVAX" => "avalanche-2",
        "DOT" => "polkadot",
        "LINK" => "chainlink",
        "LTC" => "litecoin",
        "BCH" => "bitcoin-cash",
        "SHIB" => "shiba-inu",
        "UNI" => "uniswap",
        "ATOM" => "cosmos",
        "NEAR" => "near",
        other => other.to_lowercase().leak(),
    };
    if let Ok(r) = cli
        .get(format!(
            "https://api.coingecko.com/api/v3/simple/price?ids={}&vs_currencies=usd&include_24hr_change=true",
            id
        ))
        .send()
        .await
    {
        if let Ok(v) = r.json::<serde_json::Value>().await {
            if let Some(p) = v.get(id).and_then(|x| x.get("usd")).and_then(|x| x.as_f64()) {
                let data = json!({
                    "price": p,
                    "pair": format!("{}USD", base),
                    "currency": "USD",
                    "change_percent": v.get(id).and_then(|x| x.get("usd_24h_change")).and_then(|x| x.as_f64())
                });
                let capture = enregistrer_capture(&cle, "coingecko", data.clone());
                return json!({"status":"success","symbol":symbole,"source":"coingecko","cached":false,"capture":capture,"data":data});
            }
        }
    }

    // 3. Coinbase
    let paire = if symbole.ends_with("USDT") {
        format!("{}-USD", symbole.trim_end_matches("USDT"))
    } else {
        format!("{}-USD", symbole.trim_end_matches("USD"))
    };
    if let Ok(r) = cli
        .get(format!("https://api.coinbase.com/v2/prices/{}/spot", paire))
        .send()
        .await
    {
        if let Ok(v) = r.json::<serde_json::Value>().await {
            if let Some(p) = v
                .get("data")
                .and_then(|d| d.get("amount"))
                .and_then(|a| a.as_str())
                .and_then(|a| a.parse::<f64>().ok())
            {
                let data = json!({"price": p, "pair": paire, "currency": "USD"});
                let capture = enregistrer_capture(&cle, "coinbase", data.clone());
                return json!({"status":"success","symbol":symbole,"source":"coinbase","cached":false,"capture":capture,"data":data});
            }
        }
    }

    repli(&cle, &symbole)
}

fn repli(cle: &str, symbole: &str) -> serde_json::Value {
    match cache_perime(cle) {
        Some(e) => {
            let capture = capture_meta(&e, true);
            json!({
                "status":"success","symbol":symbole,"source":e.source,
                "cached":true,"stale":true,"capture":capture,"data":e.valeur,
                "message":"Sources injoignables — dernier prix connu servi"
            })
        }
        None => json!({
            "status":"error","symbol":symbole,
            "message":"Aucune source disponible (reseau) et aucun cache"
        }),
    }
}

/// Bougies OHLCV (graphiques candlestick) — Binance klines, cache 60 s.
pub async fn bougies(symbole: &str, intervalle: &str, limite: u32) -> serde_json::Value {
    let symbole = symbole.trim().to_uppercase();
    let intervalle = intervalle.trim().to_string();
    if !crypto_symbol_valide(&symbole) {
        return json!({"status":"error","message":"Symbole crypto invalide"});
    }
    if !crypto_intervalle_valide(&intervalle) {
        return json!({"status":"error","message":"Intervalle crypto invalide"});
    }
    let limite = limite.clamp(1, 1000);
    let cle = format!("klines:{}:{}:{}", symbole, intervalle, limite);

    if let Some(e) = cache_frais_dans(&cle, CACHE_SECS_CRYPTO) {
        let capture = capture_meta(&e, true);
        return json!({"status":"success","symbol":symbole,"interval":intervalle,"source":e.source,"cached":true,"capture":capture,"candles":e.valeur});
    }

    if let Some(cli) = client().await {
        if let Ok(r) = cli
            .get(format!(
                "https://api.binance.com/api/v3/klines?symbol={}&interval={}&limit={}",
                symbole, intervalle, limite
            ))
            .send()
            .await
        {
            if let Ok(v) = r.json::<serde_json::Value>().await {
                if let Some(arr) = v.as_array() {
                    let bougies: Vec<serde_json::Value> = arr
                        .iter()
                        .filter_map(|k| {
                            let a = k.as_array()?;
                            Some(json!({
                                "time": a.get(0)?.as_i64()? / 1000,
                                "open": a.get(1)?.as_str()?.parse::<f64>().ok()?,
                                "high": a.get(2)?.as_str()?.parse::<f64>().ok()?,
                                "low": a.get(3)?.as_str()?.parse::<f64>().ok()?,
                                "close": a.get(4)?.as_str()?.parse::<f64>().ok()?,
                                "volume": a.get(5)?.as_str()?.parse::<f64>().ok()?
                            }))
                        })
                        .collect();
                    let data = json!(bougies);
                    let capture = enregistrer_capture(&cle, "binance", data.clone());
                    return json!({"status":"success","symbol":symbole,"interval":intervalle,"source":"binance","cached":false,"capture":capture,"candles":data});
                }
            }
        }
    }

    match cache_perime(&cle) {
        Some(e) => {
            let capture = capture_meta(&e, true);
            json!({"status":"success","symbol":symbole,"interval":intervalle,"source":e.source,"cached":true,"stale":true,"capture":capture,"candles":e.valeur})
        }
        None => json!({"status":"error","message":"Bougies indisponibles (reseau) et aucun cache"}),
    }
}

// ====================
// BOURSE (Yahoo Finance — sans clé)
// ====================

/// Périodes acceptées → (range Yahoo, intervalle des points). « 1w » est
/// normalisé en 5d : Yahoo ne connaît pas 1w.
fn periode_boursiere(brut: &str) -> Option<(&'static str, &'static str)> {
    match brut.trim().to_ascii_lowercase().as_str() {
        "1d" => Some(("1d", "1m")),
        "5d" | "1w" => Some(("5d", "15m")),
        "1mo" => Some(("1mo", "1h")),
        "3mo" => Some(("3mo", "1d")),
        "6mo" => Some(("6mo", "1d")),
        "1y" => Some(("1y", "1d")),
        "2y" => Some(("2y", "1wk")),
        "5y" => Some(("5y", "1wk")),
        "10y" => Some(("10y", "1mo")),
        "ytd" => Some(("ytd", "1d")),
        "max" => Some(("max", "1mo")),
        _ => None,
    }
}

/// Un ticker Yahoo n'est que des lettres, chiffres, point et tiret (BRK-B,
/// BF.B). Valider avant d'interpoler dans l'URL appelante.
fn symbole_boursier_valide(brut: &str) -> Option<String> {
    let s = brut.trim().to_uppercase();
    if s.len() < 1
        || s.len() > 12
        || !s
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
    {
        return None;
    }
    Some(s)
}

fn yahoo_meta_number(meta: &serde_json::Value, cle: &str) -> Option<f64> {
    meta.get(cle)
        .and_then(|value| value.as_f64())
        .filter(|value| value.is_finite() && *value >= 0.0)
}

pub async fn action(brut_symbole: &str, brute_periode: &str) -> serde_json::Value {
    let Some(symbole) = symbole_boursier_valide(brut_symbole) else {
        return json!({"status":"error","message":"Symbole boursier invalide"});
    };
    let Some((periode, intervalle)) = periode_boursiere(brute_periode) else {
        return json!({"status":"error","message":"Période boursière invalide"});
    };
    let cle = format!("stock:{}:{}", symbole, periode);

    if let Some(e) = cache_frais(&cle) {
        let capture = capture_meta(&e, true);
        return json!({"status":"success","symbol":symbole,"source":e.source,"cached":true,"capture":capture,"data":e.valeur});
    }

    if let Some(cli) = client().await {
        if let Ok(r) = cli
            .get(format!(
                "https://query1.finance.yahoo.com/v8/finance/chart/{}?range={}&interval={}",
                symbole, periode, intervalle
            ))
            .send()
            .await
        {
            if let Ok(v) = r.json::<serde_json::Value>().await {
                let result = v
                    .get("chart")
                    .and_then(|c| c.get("result"))
                    .and_then(|r| r.get(0));
                let meta = result.and_then(|r| r.get("meta"));
                if let Some(m) = meta {
                    let prix = m
                        .get("regularMarketPrice")
                        .and_then(|p| p.as_f64())
                        .unwrap_or(0.0);
                    let devise = m.get("currency").and_then(|c| c.as_str()).unwrap_or("USD");
                    let previous_close = m
                        .get("chartPreviousClose")
                        .or_else(|| m.get("previousClose"))
                        .and_then(|value| value.as_f64())
                        .filter(|value| *value > 0.0);
                    let change_percent = previous_close.map(|close| (prix - close) / close * 100.0);
                    let timestamps = result
                        .and_then(|r| r.get("timestamp"))
                        .and_then(|values| values.as_array());
                    let indicators = result
                        .and_then(|r| r.get("indicators"))
                        .and_then(|value| value.get("quote"))
                        .and_then(|values| values.get(0));
                    let closes = indicators
                        .and_then(|value| value.get("close"))
                        .and_then(|values| values.as_array());
                    let volumes = indicators
                        .and_then(|value| value.get("volume"))
                        .and_then(|values| values.as_array());
                    let mut history = Vec::new();
                    let mut volume_sum = 0.0;
                    let mut volume_count = 0usize;
                    if let Some(timestamps) = timestamps {
                        for (index, timestamp) in timestamps.iter().enumerate() {
                            let Some(time) = timestamp.as_i64() else {
                                continue;
                            };
                            let Some(close) = closes
                                .and_then(|values| values.get(index))
                                .and_then(|value| value.as_f64())
                            else {
                                continue;
                            };
                            let volume = volumes
                                .and_then(|values| values.get(index))
                                .and_then(|value| value.as_f64());
                            if let Some(volume) = volume {
                                volume_sum += volume;
                                volume_count += 1;
                            }
                            history.push(json!({
                                "time": time,
                                "close": close,
                                "volume": volume
                            }));
                        }
                    }
                    let data = json!({
                        "price": prix,
                        "currency": devise,
                        "symbol": symbole,
                        "previous_close": previous_close,
                        "change_percent": change_percent,
                        "history": history,
                        "average_volume": (volume_count > 0).then(|| volume_sum / volume_count as f64),
                        "open": yahoo_meta_number(m, "regularMarketOpen"),
                        "day_high": yahoo_meta_number(m, "regularMarketDayHigh"),
                        "day_low": yahoo_meta_number(m, "regularMarketDayLow"),
                        "volume": yahoo_meta_number(m, "regularMarketVolume")
                    });
                    let capture = enregistrer_capture(&cle, "yahoo", data.clone());
                    return json!({"status":"success","symbol":symbole,"source":"yahoo","cached":false,"capture":capture,"data":data});
                }
            }
        }
    }

    match cache_perime(&cle) {
        Some(e) => {
            let capture = capture_meta(&e, true);
            json!({"status":"success","symbol":symbole,"source":e.source,"cached":true,"stale":true,"capture":capture,"data":e.valeur})
        }
        None => json!({"status":"error","message":"Cours indisponible (reseau) et aucun cache"}),
    }
}

// ====================
// TEMPS RÉEL (biquote.io — sans clé)
// ====================

/// Un symbole de marché n'est que des lettres et des chiffres (XAUUSD,
/// EURUSD, BTCUSD). Valider avant d'interpoler dans l'URL appelante.
fn symbole_valide(brut: &str) -> Option<String> {
    let s = brut.trim().to_uppercase();
    if s.len() < 3 || s.len() > 12 || !s.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    Some(s)
}

/// Tick temps réel : `GET https://biquote.io/api/{SYMBOL}`.
/// `mid` fait foi (le doc l'impose) ; bid/ask sont conservés pour affichage.
pub async fn tick_temps_reel(brut: &str) -> serde_json::Value {
    let Some(symbole) = symbole_valide(brut) else {
        return json!({
            "status": "error", "symbol": brut,
            "message": "Symbole invalide — lettres et chiffres uniquement (ex. XAUUSD)"
        });
    };
    let cle = format!("biquote:{}", symbole);

    if let Some(e) = cache_frais(&cle) {
        let capture = capture_meta(&e, true);
        return json!({"status":"success","symbol":symbole,"source":e.source,"cached":true,"capture":capture,"data":e.valeur});
    }

    let Some(cli) = client().await else {
        return repli(&cle, &symbole);
    };
    if let Ok(r) = cli
        .get(format!("https://biquote.io/api/{}", symbole))
        .send()
        .await
    {
        if let Ok(v) = r.json::<serde_json::Value>().await {
            if let Some(mid) = v.get("mid").and_then(|m| m.as_f64()) {
                if mid > 0.0 {
                    let data = json!({
                        "price": mid,
                        "bid": v.get("bid").and_then(|x| x.as_f64()).unwrap_or(0.0),
                        "ask": v.get("ask").and_then(|x| x.as_f64()).unwrap_or(0.0),
                        "spread": v.get("spread").and_then(|x| x.as_f64()).unwrap_or(0.0),
                        "high_24h": v.get("high").and_then(|x| x.as_f64()).unwrap_or(0.0),
                        "low_24h": v.get("low").and_then(|x| x.as_f64()).unwrap_or(0.0),
                        "change_pct_24h": v.get("dayDiffPercent").and_then(|x| x.as_f64()).unwrap_or(0.0),
                        "direction": v.get("direction").and_then(|x| x.as_str()).unwrap_or(""),
                        "market_state": v.get("marketState").and_then(|x| x.as_str()).unwrap_or(""),
                        "quote_at": v.get("timestamp").and_then(|x| x.as_str()).unwrap_or(""),
                        "feed": v.get("source").and_then(|x| x.as_str()).unwrap_or(""),
                        "exchange": v.get("exchange").and_then(|x| x.as_str()).unwrap_or(""),
                        "stale": v.get("stale").and_then(|x| x.as_bool()).unwrap_or(false),
                    });
                    let capture = enregistrer_capture(&cle, "biquote", data.clone());
                    return json!({"status":"success","symbol":symbole,"source":"biquote","cached":false,"capture":capture,"data":data});
                }
            }
        }
    }

    repli(&cle, &symbole)
}

/// Bougies temps réel : `GET https://biquote.io/api/{SYMBOL}/ohlc?interval=&limit=`.
/// Intervalles acceptés par la source : 1m, 5m, 15m, 30m, 1h, 4h, 1d.
pub async fn bougies_temps_reel(brut: &str, intervalle: &str, limite: u32) -> serde_json::Value {
    let Some(symbole) = symbole_valide(brut) else {
        return json!({
            "status": "error", "symbol": brut,
            "message": "Symbole invalide — lettres et chiffres uniquement (ex. XAUUSD)"
        });
    };
    let intervalle = intervalle.trim().to_lowercase();
    if !matches!(
        intervalle.as_str(),
        "1m" | "5m" | "15m" | "30m" | "1h" | "4h" | "1d"
    ) {
        return json!({
            "status": "error", "symbol": symbole, "interval": intervalle,
            "message": "Intervalle invalide — 1m, 5m, 15m, 30m, 1h, 4h ou 1d"
        });
    }
    let limite = limite.clamp(1, 500);
    let cle = format!("biquote:ohlc:{}:{}", symbole, intervalle);

    if let Some(e) = cache_frais(&cle) {
        let capture = capture_meta(&e, true);
        return json!({"status":"success","symbol":symbole,"interval":intervalle,"source":e.source,"cached":true,"capture":capture,"candles":e.valeur});
    }

    let Some(cli) = client().await else {
        return repli_bougies(&cle, &symbole, &intervalle);
    };
    if let Ok(r) = cli
        .get(format!(
            "https://biquote.io/api/{}/ohlc?interval={}&limit={}",
            symbole, intervalle, limite
        ))
        .send()
        .await
    {
        if let Ok(v) = r.json::<serde_json::Value>().await {
            if let Some(bars) = v.get("bars").and_then(|b| b.as_array()) {
                let bougies: Vec<serde_json::Value> = bars
                    .iter()
                    .filter_map(|b| {
                        Some(json!({
                            "time": b.get("openTime")?.as_str()?,
                            "open": b.get("open")?.as_f64()?,
                            "high": b.get("high")?.as_f64()?,
                            "low": b.get("low")?.as_f64()?,
                            "close": b.get("close")?.as_f64()?,
                            "volume": b.get("tickVolume").and_then(|t| t.as_f64()).unwrap_or(0.0),
                            "is_open": b.get("isOpen").and_then(|o| o.as_bool()).unwrap_or(false),
                        }))
                    })
                    .collect();
                if !bougies.is_empty() {
                    let data = json!(bougies);
                    let capture = enregistrer_capture(&cle, "biquote", data.clone());
                    return json!({"status":"success","symbol":symbole,"interval":intervalle,"source":"biquote","cached":false,"capture":capture,"candles":data});
                }
            }
        }
    }

    repli_bougies(&cle, &symbole, &intervalle)
}

fn repli_bougies(cle: &str, symbole: &str, intervalle: &str) -> serde_json::Value {
    match cache_perime(cle) {
        Some(e) => {
            let capture = capture_meta(&e, true);
            json!({
                "status":"success","symbol":symbole,"interval":intervalle,"source":e.source,
                "cached":true,"stale":true,"capture":capture,"candles":e.valeur,
                "message":"Source injoignable — dernières bougies connues servies"
            })
        }
        None => json!({
            "status":"error","symbol":symbole,"interval":intervalle,
            "message":"Aucune bougie disponible (reseau) et aucun cache"
        }),
    }
}

// ====================
// OR DE RÉFÉRENCE (xaus.com — sans clé, cache 5 min)
// ====================

/// Spot or : `GET https://xaus.com/api/v1/spot` (`spot_usd_oz` en USD/once).
pub async fn spot_or() -> serde_json::Value {
    let cle = CLE_SPOT_OR;

    if let Some(e) = cache_frais_dans(cle, CACHE_SECS_OR) {
        let capture = capture_meta(&e, true);
        return json!({"status":"success","symbol":"XAUUSD","source":e.source,"cached":true,"cache_secondes":CACHE_SECS_OR,"capture":capture,"data":e.valeur});
    }

    if let Some(cli) = client().await {
        if let Ok(r) = cli.get("https://xaus.com/api/v1/spot").send().await {
            if let Ok(v) = r.json::<serde_json::Value>().await {
                if let Some(once) = v.get("spot_usd_oz").and_then(|x| x.as_f64()) {
                    if once > 0.0 {
                        let data = json!({
                            "price_usd_per_ounce": once,
                            "price_usd_per_gram": v.get("per_gram_usd").and_then(|x| x.as_f64()).unwrap_or(0.0),
                            "price_usd_per_kg": v.get("per_kg_usd").and_then(|x| x.as_f64()).unwrap_or(0.0),
                            "silver_usd_per_ounce": v.get("silver_usd_oz").and_then(|x| x.as_f64()).unwrap_or(0.0),
                            "gold_silver_ratio": v.get("gold_silver_ratio").and_then(|x| x.as_f64()).unwrap_or(0.0),
                            "data_state": v.get("data_state").and_then(|x| x.as_str()).unwrap_or(""),
                        });
                        let capture = enregistrer_capture(cle, "xaus", data.clone());
                        return json!({"status":"success","symbol":"XAUUSD","source":"xaus","cached":false,"cache_secondes":CACHE_SECS_OR,"capture":capture,"data":data});
                    }
                }
            }
        }
    }

    match cache_perime(cle) {
        Some(e) => {
            let capture = capture_meta(&e, true);
            json!({
                "status":"success","symbol":"XAUUSD","source":e.source,"cached":true,"stale":true,
                "cache_secondes":CACHE_SECS_OR,"capture":capture,"data":e.valeur,
                "message":"Source injoignable — dernier spot or connu"
            })
        }
        None => json!({
            "status":"error","symbol":"XAUUSD",
            "message":"Aucun prix de l'or disponible (reseau) et aucun cache"
        }),
    }
}

/// Le prix de l'or en USD/once, ou `None` si ni la source ni le cache ne répondent.
pub async fn once_or_usd() -> Option<f64> {
    spot_or()
        .await
        .get("data")?
        .get("price_usd_per_ounce")?
        .as_f64()
        .filter(|p| *p > 0.0)
}

// ====================
// ÉTAT DES SOURCES
// ====================

pub fn etat_sources() -> serde_json::Value {
    let c = charger_cache();
    let maintenant = chrono::Utc::now().timestamp();
    let mut entrees: Vec<serde_json::Value> = c
        .values()
        .map(|e| {
            let ttl = ttl_de_la_cle(&e.cle);
            json!({
                "cle": e.cle, "source": e.source,
                "age_sec": maintenant - e.at,
                "captured_at": e.at,
                "ttl_sec": ttl,
                "frais": maintenant - e.at < ttl
            })
        })
        .collect();
    entrees.sort_by(|a, b| a["cle"].as_str().cmp(&b["cle"].as_str()));

    json!({
        "status": "success",
        "cache_secondes": CACHE_SECS,
        "cache_secondes_crypto": CACHE_SECS_CRYPTO,
        "cache_secondes_or": CACHE_SECS_OR,
        "entrees": entrees.len(),
        "detail": entrees,
        "sources": {
            "crypto": ["binance", "coingecko", "coinbase"],
            "fiat": ["open.er-api.com"],
            "bourse": ["yahoo"],
            "temps_reel": ["biquote"],
            "or": ["xaus"]
        }
    })
}

/// TTL réellement appliqué à une clé de cache.
fn ttl_de_la_cle(cle: &str) -> i64 {
    if cle.starts_with("xaus:") {
        CACHE_SECS_OR
    } else if cle.starts_with("crypto:") || cle.starts_with("klines:") {
        CACHE_SECS_CRYPTO
    } else {
        CACHE_SECS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_ecrit_et_relit() {
        let cle = "test:crypto:XYZ";
        ecrire_cache(cle, "test", json!({"price": 42.0})).unwrap();
        let c = charger_cache();
        assert!(c.contains_key(cle));
        assert_eq!(c[cle].source, "test");
        assert!(cache_frais(cle).is_some());
    }

    #[test]
    fn symbole_accepte_et_rejete() {
        assert_eq!(symbole_valide("xauusd").as_deref(), Some("XAUUSD"));
        assert_eq!(symbole_valide(" EURUSD ").as_deref(), Some("EURUSD"));
        // injections et symboles vides refusées
        assert_eq!(symbole_valide("XAUUSD?fmt=html"), None);
        assert_eq!(symbole_valide("EUR/USD"), None);
        assert_eq!(symbole_valide(""), None);
        assert_eq!(symbole_valide("AB"), None);
    }

    #[test]
    fn ttl_de_l_or_est_de_cinq_minutes() {
        assert_eq!(ttl_de_la_cle("xaus:spot"), CACHE_SECS_OR);
        assert_eq!(ttl_de_la_cle("biquote:XAUUSD"), CACHE_SECS);
    }

    #[test]
    fn capture_meta_expose_horodatage_fraicheur_et_persistance() {
        let entree = EntreeCache {
            cle: "crypto:BTCUSDT".to_string(),
            source: "binance".to_string(),
            valeur: json!({"price": 42.0}),
            at: chrono::Utc::now().timestamp() - 3,
        };
        let capture = capture_meta(&entree, true);
        assert_eq!(capture["captured_at"], json!(entree.at));
        assert_eq!(capture["ttl_sec"], json!(CACHE_SECS_CRYPTO));
        assert_eq!(capture["fresh"], json!(true));
        assert_eq!(capture["persisted"], json!(true));
    }

    #[test]
    fn symboles_crypto_et_intervalles_sont_valides() {
        assert!(crypto_symbol_valide("BTCUSDT"));
        assert!(crypto_symbol_valide("NEARUSDT"));
        assert!(!crypto_symbol_valide("BTC/USDT"));
        assert!(!crypto_symbol_valide("BTCUSDT?limit=1"));
        assert!(!crypto_symbol_valide(""));

        assert!(crypto_intervalle_valide("1s"));
        assert!(crypto_intervalle_valide("1m"));
        assert!(crypto_intervalle_valide("1M"));
        assert!(!crypto_intervalle_valide("1m&limit=1"));
        assert!(!crypto_intervalle_valide("1y"));

        assert_eq!(ttl_de_la_cle("crypto:BTCUSDT"), CACHE_SECS_CRYPTO);
        assert_eq!(ttl_de_la_cle("klines:BTCUSDT:1s:200"), CACHE_SECS_CRYPTO);
    }
}
