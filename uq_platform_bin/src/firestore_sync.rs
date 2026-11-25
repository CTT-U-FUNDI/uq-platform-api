// ============================================================
// firestore_sync — projection backend → Firestore (lecture seule publique)
//
// Source de vérité = SQLite backend. Firestore n'est qu'une vitrine :
// les règles interdisent l'écriture client sur les collections
// projetées, seul ce module (Admin SDK) écrit — via l'API REST
// Firestore avec un jeton OAuth2 de compte de service.
//
// Config : FIREBASE_SERVICE_ACCOUNT = JSON du compte de service
// (console Firebase → Paramètres → Comptes de service → Générer).
// Sans la variable, toutes les fonctions sont des no-op silencieuses.
// ============================================================

use std::sync::Mutex;
use serde_json::{json, Value};

const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const SCOPE: &str = "https://www.googleapis.com/auth/datastore";

struct CompteService {
    client_email: String,
    private_key_pem: String,
    project_id: String,
}

static COMPTE: Mutex<Option<Option<CompteService>>> = Mutex::new(None);
static TOKEN: Mutex<Option<(String, std::time::Instant)>> = Mutex::new(None);

fn compte() -> Option<CompteService> {
    let mut guard = COMPTE.lock().unwrap();
    if guard.is_none() {
        let lu = std::env::var("FIREBASE_SERVICE_ACCOUNT").ok().and_then(|brut| {
            serde_json::from_str::<Value>(&brut).ok().map(|v| CompteService {
                client_email: v["client_email"].as_str().unwrap_or("").to_string(),
                private_key_pem: v["private_key"].as_str().unwrap_or("").to_string(),
                project_id: v["project_id"].as_str().unwrap_or("").to_string(),
            })
        });
        *guard = Some(lu);
    }
    // On ne peut pas cloner le Mutex guard ; on reconstruit à chaque appel
    // (le JSON est petit, la lecture env est déjà cachée).
    std::env::var("FIREBASE_SERVICE_ACCOUNT")
        .ok()
        .and_then(|brut| serde_json::from_str::<Value>(&brut).ok())
        .map(|v| CompteService {
            client_email: v["client_email"].as_str().unwrap_or("").to_string(),
            private_key_pem: v["private_key"].as_str().unwrap_or("").to_string(),
            project_id: v["project_id"].as_str().unwrap_or("").to_string(),
        })
}

fn access_token() -> Option<String> {
    {
        let guard = TOKEN.lock().unwrap();
        if let Some((t, quand)) = guard.as_ref() {
            // Les jetons durent ~1h ; on renouvelle à 50 min.
            if quand.elapsed() < std::time::Duration::from_secs(3000) {
                return Some(t.clone());
            }
        }
    }
    let compte = compte()?;
    if compte.client_email.is_empty() || compte.private_key_pem.is_empty() {
        return None;
    }
    let maintenant = chrono::Utc::now().timestamp();
    let claims = json!({
        "iss": compte.client_email,
        "scope": SCOPE,
        "aud": TOKEN_URL,
        "iat": maintenant,
        "exp": maintenant + 3600,
    });
    let cle = jsonwebtoken::EncodingKey::from_rsa_pem(compte.private_key_pem.as_bytes()).ok()?;
    let jwt = jsonwebtoken::encode(
        &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256),
        &claims,
        &cle,
    )
    .ok()?;
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .ok()?;
    let reponse = client
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
            ("assertion", jwt.as_str()),
        ])
        .send()
        .ok()?;
    let corps: Value = reponse.json().ok()?;
    let token = corps["access_token"].as_str()?.to_string();
    *TOKEN.lock().unwrap() = Some((token.clone(), std::time::Instant::now()));
    Some(token)
}

fn valeur_firestore(v: &Value) -> Value {
    match v {
        Value::Null => json!({"nullValue": null}),
        Value::Bool(b) => json!({"booleanValue": b}),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                json!({"integerValue": i.to_string()})
            } else {
                json!({"doubleValue": n.as_f64().unwrap_or(0.0)})
            }
        }
        Value::String(s) => json!({"stringValue": s}),
        Value::Array(a) => json!({
            "arrayValue": {"values": a.iter().map(valeur_firestore).collect::<Vec<_>>()}
        }),
        Value::Object(o) => json!({
            "mapValue": {"fields": o.iter().map(|(k, v)| (k.clone(), valeur_firestore(v))).collect::<serde_json::Map<_, _>>()}
        }),
    }
}

/// Écrit (ou fusionne) un document Firestore. No-op sans compte de service.
/// `document` = chemin relatif, ex: "zando_items/abc" ou "reputation/UQ-1".
pub fn ecrire(document: &str, champs: &Value) -> bool {
    let Some(compte) = compte() else { return false };
    let Some(token) = access_token() else { return false };
    let url = format!(
        "https://firestore.googleapis.com/v1/projects/{}/databases/(default)/documents/{}",
        compte.project_id, document
    );
    let fields: serde_json::Map<String, Value> = champs
        .as_object()
        .map(|o| {
            o.iter()
                .map(|(k, v)| (k.clone(), valeur_firestore(v)))
                .collect()
        })
        .unwrap_or_default();
    let corps = json!({
        "fields": fields,
    });
    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };
    // PATCH fusionne les champs sans écraser le document entier.
    client
        .patch(&url)
        .bearer_auth(&token)
        .json(&corps)
        .send()
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

/// Supprime un document projeté (no-op sans compte de service).
pub fn supprimer(document: &str) -> bool {
    let Some(compte) = compte() else { return false };
    let Some(token) = access_token() else { return false };
    let url = format!(
        "https://firestore.googleapis.com/v1/projects/{}/databases/(default)/documents/{}",
        compte.project_id, document
    );
    reqwest::blocking::Client::new()
        .delete(&url)
        .bearer_auth(&token)
        .send()
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}
