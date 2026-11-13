// ====================
// ORACLE EXTENSIONS — Rate Lock + Détection Géo
// ====================

use actix_web::{web, HttpResponse};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Durée du rate lock (30 secondes)
const LOCK_SECS: i64 = 30;

/// Structure d'un lock de taux
#[derive(Clone, Debug)]
pub struct TauxLock {
    pub taux: f64,
    pub expire_at: i64,
}

/// Map globale des locks actifs
lazy_static::lazy_static! {
    static ref LOCKS_TAUX: Mutex<HashMap<String, TauxLock>> = Mutex::new(HashMap::new());
}

/// Retourne le timestamp actuel
fn maintenant() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// POST /api/oracle/lock
/// Verrouille le taux d'une devise pour 30 secondes
pub async fn oracle_lock(body: web::Json<serde_json::Value>) -> HttpResponse {
    let devise = body
        .get("devise")
        .and_then(|v| v.as_str())
        .unwrap_or("CDF")
        .trim()
        .to_uppercase();

    let taux = crate::public_routes::taux_du_jour().await;

    // Un verrou fixe un prix d'EXECUTION : jamais de taux par défaut. Une
    // devise non observée, ou un prix interne non réglé, renvoient une erreur
    // explicite au lieu d'un « 1 pour 1 » qui serait exécuté.
    let taux_devise = if devise == "BORB" || devise == "BORB" {
        let prix = crate::persistence::load_prix();
        match prix.prix_actuel.parse::<f64>() {
            Ok(p) if p.is_finite() && p > 0.0 => p,
            _ => {
                return HttpResponse::ServiceUnavailable().json(json!({
                    "status": "error",
                    "code": "PRIX_NON_REGLE",
                    "devise": devise,
                    "message": "Le prix interne du BORB n'est pas réglé : aucun taux ne sera verrouillé."
                }))
            }
        }
    } else if let Some(v) = taux
        .rates
        .get(&devise)
        .copied()
        .filter(|v| v.is_finite() && *v > 0.0)
    {
        v
    } else {
        return HttpResponse::ServiceUnavailable().json(json!({
            "status": "error",
            "code": "TAUX_INDISPONIBLE",
            "devise": devise,
            "message": "Aucune source n'a observé de taux pour cette devise : aucun verrou ne sera posé.",
            "source": taux.source,
        }));
    };

    let now = maintenant();
    let expire_at = now + LOCK_SECS;

    {
        let mut locks = LOCKS_TAUX.lock().unwrap();
        locks.insert(devise.clone(), TauxLock {
            taux: taux_devise,
            expire_at,
        });
    }

    HttpResponse::Ok().json(json!({
        "status": "success",
        "devise": devise,
        "taux_usd": taux_devise,
        "lock_expire_at": expire_at,
        "duree_s": LOCK_SECS,
        "source": taux.source
    }))
}

/// GET /api/oracle/locked/{devise}
/// Retourne le taux verrouillé s'il existe
pub async fn oracle_locked(path: web::Path<String>) -> HttpResponse {
    let devise = path.into_inner().trim().to_uppercase();
    let now = maintenant();

    let locks = LOCKS_TAUX.lock().unwrap();

    match locks.get(&devise) {
        Some(lock) if lock.expire_at > now => {
            HttpResponse::Ok().json(json!({
                "status": "success",
                "devise": devise,
                "taux_usd": lock.taux,
                "expire_dans": lock.expire_at - now,
                "locked": true
            }))
        }
        Some(_) => {
            // Lock expiré
            HttpResponse::Ok().json(json!({
                "status": "expired",
                "devise": devise,
                "locked": false,
                "message": "Lock expiré"
            }))
        }
        None => {
            HttpResponse::Ok().json(json!({
                "status": "not_found",
                "devise": devise,
                "locked": false,
                "message": "Aucun lock actif pour cette devise"
            }))
        }
    }
}

/// POST /api/oracle/detect
/// Détecte la devise en fonction de la géolocalisation ou de l'IP
pub async fn oracle_detect(body: web::Json<serde_json::Value>) -> HttpResponse {
    let ip = body.get("ip").and_then(|v| v.as_str()).unwrap_or("");
    let lat = body.get("lat").and_then(|v| v.as_f64());
    let lng = body.get("lng").and_then(|v| v.as_f64());
    let pays_manuel = body.get("pays").and_then(|v| v.as_str()).unwrap_or("");

    // 1. Si le pays est fourni manuellement
    if !pays_manuel.is_empty() {
        let (devise, langue) = pays_vers_devise(pays_manuel);
        return HttpResponse::Ok().json(json!({
            "status": "success",
            "source": "manual",
            "pays": pays_manuel,
            "devise": devise,
            "langue": langue
        }));
    }

    // 2. Si GPS est fourni
    if let (Some(lat), Some(lng)) = (lat, lng) {
        let pays = detecter_pays_gps(lat, lng);
        let (devise, langue) = pays_vers_devise(&pays);
        return HttpResponse::Ok().json(json!({
            "status": "success",
            "source": "gps",
            "pays": pays,
            "devise": devise,
            "langue": langue,
            "coords": { "lat": lat, "lng": lng }
        }));
    }

    // 3. Si IP est fourni
    if !ip.is_empty() {
        let pays = detecter_pays_ip(ip);
        let (devise, langue) = pays_vers_devise(&pays);
        return HttpResponse::Ok().json(json!({
            "status": "success",
            "source": "ip",
            "pays": pays,
            "devise": devise,
            "langue": langue,
            "ip": ip
        }));
    }

    // 4. Fallback
    HttpResponse::Ok().json(json!({
        "status": "success",
        "source": "default",
        "pays": "RDC",
        "devise": "CDF",
        "langue": "fr",
        "message": "Aucune donnée de localisation fournie, valeur par défaut RDC"
    }))
}

/// Détecte le pays à partir des coordonnées GPS (approximatif)
fn detecter_pays_gps(lat: f64, lng: f64) -> String {
    // Afrique Centrale (RDC)
    if lat >= -13.5 && lat <= 5.5 && lng >= 12.0 && lng <= 31.5 {
        return "RDC".to_string();
    }
    // Cameroun
    if lat >= 1.6 && lat <= 13.1 && lng >= 8.4 && lng <= 16.2 {
        return "Cameroun".to_string();
    }
    // Sénégal
    if lat >= 12.2 && lat <= 16.7 && lng >= -17.6 && lng <= -11.3 {
        return "Sénégal".to_string();
    }
    // Nigeria
    if lat >= 4.2 && lat <= 13.9 && lng >= 2.6 && lng <= 14.7 {
        return "Nigeria".to_string();
    }
    // Kenya
    if lat >= -4.7 && lat <= 5.0 && lng >= 33.9 && lng <= 41.9 {
        return "Kenya".to_string();
    }
    // Maroc
    if lat >= 27.6 && lat <= 35.9 && lng >= -13.2 && lng <= -1.0 {
        return "Maroc".to_string();
    }
    // Europe (France)
    if lat >= 41.3 && lat <= 51.1 && lng >= -5.2 && lng <= 9.6 {
        return "France".to_string();
    }
    // Fallback
    "RDC".to_string()
}

/// Détecte le pays à partir de l'IP (approximatif - premiers octets)
fn detecter_pays_ip(ip: &str) -> String {
    // IPs congolaises typiques : 41.x, 154.x, 196.x
    if ip.starts_with("41.") || ip.starts_with("154.12") || ip.starts_with("196.") {
        return "RDC".to_string();
    }
    // Cameroun : 41.x, 154.x
    // Sénégal : 41.x, 197.x
    // Nigeria : 105.x, 197.x
    // Fallback
    "RDC".to_string()
}

/// Mappe un pays vers une devise et une langue
fn pays_vers_devise(pays: &str) -> (&'static str, &'static str) {
    match pays {
        "RDC" | "CD" | "Congo-Kinshasa" => ("CDF", "fr"),
        "Cameroun" | "CM" => ("XAF", "fr"),
        "Sénégal" | "SN" => ("XOF", "fr"),
        "Côte d'Ivoire" | "CI" => ("XOF", "fr"),
        "Nigeria" | "NG" => ("NGN", "en"),
        "Kenya" | "KE" => ("KES", "sw"),
        "Maroc" | "MA" => ("MAD", "ar"),
        "Ghana" | "GH" => ("GHS", "en"),
        "Ouganda" | "UG" => ("UGX", "en"),
        "Tanzanie" | "TZ" => ("TZS", "sw"),
        "Égypte" | "EG" => ("EGP", "ar"),
        "Rwanda" | "RW" => ("RWF", "fr"),
        "Burundi" | "BI" => ("BIF", "fr"),
        "France" | "FR" => ("EUR", "fr"),
        "USA" | "US" => ("USD", "en"),
        "Canada" | "CA" => ("CAD", "en"),
        "Suisse" | "CH" => ("CHF", "fr"),
        "Chine" | "CN" => ("CNY", "zh"),
        "Japon" | "JP" => ("JPY", "ja"),
        _ => ("USD", "en"),
    }
}
