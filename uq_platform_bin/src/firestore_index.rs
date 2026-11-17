// ====================
// FIRESTORE INDEX — Backend Firebase Firestore (projet uqz-platform)
// Auth : compte de service via env vars FIREBASE_SA_CLIENT_EMAIL +
// FIREBASE_SA_PRIVATE_KEY (JWT RS256 -> OAuth2 access_token).
// Les methodes appelees depuis UqIndex sont SYNC : on utilise un runtime
// tokio dedie + std::thread::spawn pour eviter le panic
// "Cannot block_on within a runtime" quand l'appelant est deja async.
// ====================

use serde::Serialize;
use serde_json::{Map, Value};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const FS_BASE: &str =
    "https://firestore.googleapis.com/v1/projects/uqz-platform/databases/(default)/documents";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const SCOPE: &str = "https://www.googleapis.com/auth/datastore";

// ---------- Runtime tokio dedie ----------

fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("Failed to build tokio runtime for firestore_index")
    })
}

/// Execute une future sur le runtime dedie depuis un thread separe
/// (block_on direct panic si on est deja dans un contexte tokio/actix).
fn run_blocking<F, T>(f: F) -> Option<T>
where
    F: FnOnce() -> std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send>> + Send + 'static,
    T: Send + 'static,
{
    std::thread::spawn(move || {
        let rt = runtime();
        rt.block_on(f())
    })
    .join()
    .ok()
}

// ---------- Auth OAuth2 (JWT RS256 -> access_token, cache) ----------

static TOKEN_CACHE: Mutex<Option<(String, Instant)>> = Mutex::new(None);

#[derive(Serialize)]
struct ServiceAccountClaims {
    iss: String,
    scope: String,
    aud: String,
    iat: i64,
    exp: i64,
}

fn service_account_config() -> Result<(String, String), String> {
    let email = std::env::var("FIREBASE_SA_CLIENT_EMAIL")
        .map_err(|_| "FIREBASE_SA_CLIENT_EMAIL manquant".to_string())?;
    let key = std::env::var("FIREBASE_SA_PRIVATE_KEY")
        .map_err(|_| "FIREBASE_SA_PRIVATE_KEY manquant".to_string())?
        .replace("\\n", "\n");
    Ok((email, key))
}

async fn fetch_access_token() -> Result<String, String> {
    // Cache : reutiliser tant que le token n'expire pas (marge 60 s)
    {
        let cache = TOKEN_CACHE.lock().unwrap();
        if let Some((token, expiry)) = cache.as_ref() {
            if Instant::now() + Duration::from_secs(60) < *expiry {
                return Ok(token.clone());
            }
        }
    }

    let (email, pem) = service_account_config()?;
    let now = chrono::Utc::now().timestamp();
    let claims = ServiceAccountClaims {
        iss: email,
        scope: SCOPE.to_string(),
        aud: TOKEN_URL.to_string(),
        iat: now,
        exp: now + 3600, // +1h
    };
    let jwt = jsonwebtoken::encode(
        &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256),
        &claims,
        &jsonwebtoken::EncodingKey::from_rsa_pem(pem.as_bytes())
            .map_err(|e| format!("Clé privée invalide: {}", e))?,
    )
    .map_err(|e| format!("JWT: {}", e))?;

    let client = reqwest::Client::new();
    let resp = client
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
            ("assertion", jwt.as_str()),
        ])
        .send()
        .await
        .map_err(|e| format!("Token request: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Token endpoint {}: {}", status, body));
    }
    let json: Value = resp.json().await.map_err(|e| format!("Token JSON: {}", e))?;
    let token = json
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "access_token absent de la reponse".to_string())?
        .to_string();
    let expires_in = json
        .get("expires_in")
        .and_then(|v| v.as_u64())
        .unwrap_or(3600);

    let expiry = Instant::now() + Duration::from_secs(expires_in);
    *TOKEN_CACHE.lock().unwrap() = Some((token.clone(), expiry));
    Ok(token)
}

async fn auth_header() -> Result<String, String> {
    Ok(format!("Bearer {}", fetch_access_token().await?))
}

// ---------- Conversion serde_json::Value <-> Firestore Value ----------

fn fs_value_to_json(v: &Value) -> Value {
    if let Some(s) = v.get("stringValue") {
        return Value::String(s.as_str().unwrap_or_default().to_string());
    }
    if let Some(b) = v.get("booleanValue") {
        return Value::Bool(b.as_bool().unwrap_or(false));
    }
    if let Some(i) = v.get("integerValue") {
        // Firestore renvoie integerValue comme string
        if let Some(s) = i.as_str() {
            if let Ok(n) = s.parse::<i64>() {
                return Value::from(n);
            }
            if let Ok(n) = s.parse::<u64>() {
                return Value::from(n);
            }
            if let Ok(f) = s.parse::<f64>() {
                return serde_json::Number::from_f64(f)
                    .map(Value::Number)
                    .unwrap_or(Value::Null);
            }
        }
        if let Some(n) = i.as_i64() {
            return Value::from(n);
        }
        return Value::Null;
    }
    if let Some(d) = v.get("doubleValue") {
        if let Some(n) = d.as_f64() {
            return serde_json::Number::from_f64(n)
                .map(Value::Number)
                .unwrap_or(Value::Null);
        }
        return Value::Null;
    }
    if v.get("nullValue").is_some() {
        return Value::Null;
    }
    if let Some(bytes) = v.get("bytesValue") {
        return Value::String(bytes.as_str().unwrap_or_default().to_string());
    }
    if let Some(ts) = v.get("timestampValue") {
        return Value::String(ts.as_str().unwrap_or_default().to_string());
    }
    if let Some(map) = v.get("mapValue") {
        if let Some(fields) = map.get("fields").and_then(|f| f.as_object()) {
            let mut out = Map::new();
            for (k, fv) in fields {
                out.insert(k.clone(), fs_value_to_json(fv));
            }
            return Value::Object(out);
        }
        return Value::Object(Map::new());
    }
    if let Some(arr) = v.get("arrayValue") {
        if let Some(values) = arr.get("values").and_then(|x| x.as_array()) {
            return Value::Array(values.iter().map(fs_value_to_json).collect());
        }
        return Value::Array(Vec::new());
    }
    Value::Null
}

fn json_to_fs_value(v: &Value) -> Value {
    match v {
        Value::Null => serde_json::json!({ "nullValue": null }),
        Value::Bool(b) => serde_json::json!({ "booleanValue": b }),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                serde_json::json!({ "integerValue": i.to_string() })
            } else if let Some(u) = n.as_u64() {
                serde_json::json!({ "integerValue": u.to_string() })
            } else {
                serde_json::json!({ "doubleValue": n.as_f64().unwrap_or(0.0) })
            }
        }
        Value::String(s) => serde_json::json!({ "stringValue": s }),
        Value::Array(items) => serde_json::json!({
            "arrayValue": { "values": items.iter().map(json_to_fs_value).collect::<Vec<_>>() }
        }),
        Value::Object(map) => {
            let mut fields = Map::new();
            for (k, fv) in map {
                fields.insert(k.clone(), json_to_fs_value(fv));
            }
            serde_json::json!({ "mapValue": { "fields": Value::Object(fields) } })
        }
    }
}

/// Convertit la reponse Firestore d'un document en map plate {champ: json}.
fn doc_fields_to_json(doc: &Value) -> Option<Value> {
    let fields = doc.get("fields")?.as_object()?;
    let mut out = Map::new();
    for (k, fv) in fields {
        out.insert(k.clone(), fs_value_to_json(fv));
    }
    Some(Value::Object(out))
}

fn doc_id_from_name(name: &str) -> String {
    name.rsplit('/').next().unwrap_or(name).to_string()
}

// ---------- Helpers sync publics ----------

/// GET document -> map plate des champs, None si absent/erreur.
pub fn fs_get_doc(collection: &str, id: &str) -> Option<Value> {
    let coll = collection.to_string();
    let doc_id = id.to_string();
    run_blocking(move || {
        Box::pin(async move {
            let auth = auth_header().await.ok()?;
            let client = reqwest::Client::new();
            let url = format!("{}/{}/{}", FS_BASE, coll, doc_id);
            let resp = client
                .get(&url)
                .header("Authorization", auth)
                .send()
                .await
                .ok()?;
            if resp.status().as_u16() == 404 {
                return None;
            }
            if !resp.status().is_success() {
                eprintln!("[firestore] GET {}/{} -> {}", coll, doc_id, resp.status());
                return None;
            }
            let doc: Value = resp.json().await.ok()?;
            doc_fields_to_json(&doc)
        })
    })
    .flatten()
}

/// PATCH (upsert) document. `fields` = map plate serde_json.
pub fn fs_set_doc(collection: &str, id: &str, fields: &Value) -> Result<(), String> {
    let coll = collection.to_string();
    let doc_id = id.to_string();
    let fields = fields.clone();
    run_blocking(move || {
        Box::pin(async move {
            let auth = auth_header().await?;
            let map = fields.as_object().cloned().unwrap_or_default();
            let mut fs_fields = Map::new();
            let mut mask = String::new();
            for (k, v) in &map {
                fs_fields.insert(k.clone(), json_to_fs_value(v));
                if !mask.is_empty() {
                    mask.push('&');
                }
                mask.push_str(&format!("updateMask.fieldPaths={}", k));
            }
            let url = format!("{}/{}/{}?{}", FS_BASE, coll, doc_id, mask);
            let client = reqwest::Client::new();
            let resp = client
                .patch(&url)
                .header("Authorization", auth)
                .json(&serde_json::json!({ "fields": Value::Object(fs_fields) }))
                .send()
                .await
                .map_err(|e| format!("PATCH {}: {}", url, e))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(format!("PATCH {} -> {}: {}", url, status, body));
            }
            Ok(())
        })
    })
    .ok_or_else(|| "thread firestore panic".to_string())?
}

/// DELETE document.
#[allow(dead_code)] // prevu pour les prochaines etapes (purge comptes)
pub fn fs_delete_doc(collection: &str, id: &str) -> Result<(), String> {
    let coll = collection.to_string();
    let doc_id = id.to_string();
    run_blocking(move || {
        Box::pin(async move {
            let auth = auth_header().await?;
            let client = reqwest::Client::new();
            let url = format!("{}/{}/{}", FS_BASE, coll, doc_id);
            let resp = client
                .delete(&url)
                .header("Authorization", auth)
                .send()
                .await
                .map_err(|e| format!("DELETE {}: {}", url, e))?;
            if !resp.status().is_success() && resp.status().as_u16() != 404 {
                return Err(format!("DELETE {} -> {}", url, resp.status()));
            }
            Ok(())
        })
    })
    .ok_or_else(|| "thread firestore panic".to_string())?
}

/// Liste tous les documents d'une collection -> Vec<(doc_id, fields)>.
pub fn fs_list_docs(collection: &str) -> Vec<(String, Value)> {
    let coll = collection.to_string();
    run_blocking(move || {
        Box::pin(async move {
            let auth = auth_header().await.ok()?;
            let client = reqwest::Client::new();
            let url = format!("{}/{}", FS_BASE, coll);
            let resp = client
                .get(&url)
                .header("Authorization", auth)
                .send()
                .await
                .ok()?;
            if !resp.status().is_success() {
                eprintln!("[firestore] LIST {} -> {}", coll, resp.status());
                return None;
            }
            let json: Value = resp.json().await.ok()?;
            let mut out = Vec::new();
            if let Some(docs) = json.get("documents").and_then(|d| d.as_array()) {
                for doc in docs {
                    let name = doc.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    if let Some(fields) = doc_fields_to_json(doc) {
                        out.push((doc_id_from_name(name), fields));
                    }
                }
            }
            Some(out)
        })
    })
    .flatten()
    .unwrap_or_default()
}
