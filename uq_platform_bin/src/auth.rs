use serde_json::json;
use std::collections::HashMap;
use std::sync::Mutex;
use chrono::{Utc, Duration};
use sha2::{Sha256, Digest};
use actix_web::{HttpRequest, HttpResponse};
use jsonwebtoken::{encode, decode, Header, Validation, EncodingKey, DecodingKey, Algorithm};
use serde::{Serialize, Deserialize};

/// Gestionnaire d'authentification
pub struct AuthManager {
    pub sessions: Mutex<HashMap<String, Session>>,
    pub users: Mutex<HashMap<String, UserCredentials>>,
}

#[derive(Debug, Clone)]
pub struct Session {
    pub token: String,
    pub user_id: String,
    pub created_at: i64,
    pub expires_at: i64,
}

#[derive(Debug, Clone)]
pub struct UserCredentials {
    pub user_id: String,
    pub password_hash: String,
    pub salt: String,
    pub role: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub role: String,
    pub exp: usize,
    pub iat: usize,
}

impl AuthManager {
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            users: Mutex::new(HashMap::new()),
        }
    }
    
    /// Hash un mot de passe avec SHA-256 + sel
    pub fn hash_password(&self, password: &str, salt: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(salt.as_bytes());
        hasher.update(password.as_bytes());
        let result = hasher.finalize();
        hex::encode(result)
    }
    
    /// Enregistre un utilisateur (role default = "user")
    pub fn register(&self, user_id: &str, password: &str) -> Result<(), String> {
        let mut users = self.users.lock().unwrap();
        if users.contains_key(user_id) {
            return Err("Utilisateur déjà enregistré".to_string());
        }
        
        let salt = hex::encode(chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0).to_le_bytes());
        let password_hash = self.hash_password(password, &salt);
        
        users.insert(user_id.to_string(), UserCredentials {
            user_id: user_id.to_string(),
            password_hash,
            salt,
            role: "user".to_string(),
        });
        Ok(())
    }
    
    /// Vérifie les identifiants
    pub fn verify_password(&self, user_id: &str, password: &str) -> bool {
        let users = self.users.lock().unwrap();
        if let Some(user) = users.get(user_id) {
            let hash = self.hash_password(password, &user.salt);
            hash == user.password_hash
        } else {
            false
        }
    }
    
    /// Crée une session (legacy in-memory) — kept for compatibility
    pub fn create_session(&self, user_id: &str) -> String {
        let mut sessions = self.sessions.lock().unwrap();
        let now = Utc::now();
        let expires = now + Duration::hours(24);
        
        // Générer un token unique
        let mut hasher = Sha256::new();
        hasher.update(user_id.as_bytes());
        hasher.update(now.timestamp().to_le_bytes());
        let token = hex::encode(hasher.finalize());
        
        sessions.insert(token.clone(), Session {
            token: token.clone(),
            user_id: user_id.to_string(),
            created_at: now.timestamp(),
            expires_at: expires.timestamp(),
        });
        
        token
    }

    /// Create a JWT for a user with a role
    pub fn create_jwt(&self, user_id: &str, role: &str, ttl_hours: i64) -> Result<String, jsonwebtoken::errors::Error> {
        let now = Utc::now();
        let exp = (now + Duration::hours(ttl_hours)).timestamp() as usize;
        let iat = now.timestamp() as usize;
        let claims = Claims { sub: user_id.to_string(), role: role.to_string(), exp, iat };
        let secret = std::env::var("JWT_SECRET").expect("JWT_SECRET must be set");
        encode(&Header::new(Algorithm::HS256), &claims, &EncodingKey::from_secret(secret.as_bytes()))
    }

    /// Verify a JWT and return claims
    pub fn verify_jwt(token: &str) -> Result<Claims, jsonwebtoken::errors::Error> {
        let secret = std::env::var("JWT_SECRET").expect("JWT_SECRET must be set");
        let token_data = decode::<Claims>(token, &DecodingKey::from_secret(secret.as_bytes()), &Validation::new(Algorithm::HS256))?;
        Ok(token_data.claims)
    }

    /// Vérifie si un token (legacy) est valide
    pub fn verify_token(&self, token: &str) -> bool {
        let sessions = self.sessions.lock().unwrap();
        if let Some(session) = sessions.get(token) {
            let now = Utc::now().timestamp();
            session.expires_at > now
        } else {
            false
        }
    }
    
    /// Déconnecte un utilisateur (legacy)
    pub fn logout(&self, token: &str) {
        let mut sessions = self.sessions.lock().unwrap();
        sessions.remove(token);
    }
    
    /// Nettoie les sessions expirées (legacy)
    pub fn cleanup_expired(&self) -> usize {
        let mut sessions = self.sessions.lock().unwrap();
        let now = Utc::now().timestamp();
        let before = sessions.len();
        sessions.retain(|_, s| s.expires_at > now);
        before - sessions.len()
    }
    
    /// Statistiques d'authentification
    pub fn get_stats(&self) -> serde_json::Value {
        let sessions = self.sessions.lock().unwrap();
        let users = self.users.lock().unwrap();
        serde_json::json!({
            "active_sessions": sessions.len(),
            "registered_users": users.len(),
            "expired_cleaned": 0
        })
    }

    /// Helper: extract JWT from Authorization header and validate role
    pub fn require_role(req: &HttpRequest, allowed: &[&str]) -> Result<Claims, HttpResponse> {
        if let Some(auth_header) = req.headers().get("Authorization") {
            if let Ok(s) = auth_header.to_str() {
                if s.starts_with("Bearer ") {
                    let token = &s[7..];
                    match AuthManager::verify_jwt(token) {
                        Ok(claims) => {
                            for role in allowed {
                                if claims.role == *role {
                                    return Ok(claims);
                                }
                            }
                            return Err(HttpResponse::Forbidden().json(serde_json::json!({"error":"insufficient_role"})))
                        }
                        Err(_) => return Err(HttpResponse::Unauthorized().json(serde_json::json!({"error":"invalid_token"}))),
                    }
                }
            }
        }
        Err(HttpResponse::Unauthorized().json(serde_json::json!({"error":"missing_authorization"})))
    }
    // ====================
    // VÉRIFICATION DU RÔLE OWNER
    // Vérifie : JWT valide + is_owner = true dans uq_index
    // ====================
    pub fn require_owner(req: &HttpRequest) -> Result<Claims, HttpResponse> {
        // 1. Vérifier le JWT
        let claims = match Self::require_role(req, &["user", "admin", "guardian"]) {
            Ok(c) => c,
            Err(resp) => return Err(resp),
        };

        // 2. Vérifier que is_owner = true dans uq_index
        let idx = crate::uq_index::UqIndex::shared("uq_index.db");
        let account = idx.get_account(&claims.sub);

        match account {
            Some(acc) => {
                let is_owner = acc.get("is_owner").and_then(|v| v.as_bool()).unwrap_or(false);
                if is_owner {
                    Ok(claims)
                } else {
                    Err(HttpResponse::Forbidden().json(json!({
                        "status": "error",
                        "message": "Accès réservé au propriétaire"
                    })))
                }
            }
            None => Err(HttpResponse::Unauthorized().json(json!({
                "status": "error",
                "message": "Compte introuvable"
            }))),
        }
    }

    // ====================
    // VÉRIFICATION DU RÔLE ADMIN
    // = owner OU role admin explicite
    // ====================
    pub fn require_admin(req: &HttpRequest) -> Result<Claims, HttpResponse> {
        let claims = match Self::require_role(req, &["user", "admin", "guardian"]) {
            Ok(c) => c,
            Err(resp) => return Err(resp),
        };

        // Si role = admin explicite → OK
        if claims.role == "admin" {
            return Ok(claims);
        }

        // Sinon, vérifier si owner
        let idx = crate::uq_index::UqIndex::shared("uq_index.db");
        let account = idx.get_account(&claims.sub);

        match account {
            Some(acc) => {
                let is_owner = acc.get("is_owner").and_then(|v| v.as_bool()).unwrap_or(false);
                if is_owner {
                    Ok(claims)
                } else {
                    Err(HttpResponse::Forbidden().json(json!({
                        "status": "error",
                        "message": "Accès administrateur requis"
                    })))
                }
            }
            None => Err(HttpResponse::Unauthorized().json(json!({
                "status": "error",
                "message": "Compte introuvable"
            }))),
        }
    }

    // ====================
    // VÉRIFICATION DU RÔLE ARBITRATOR
    // = owner OU privilege arbitration_judge
    // ====================
    pub fn require_arbitrator(req: &HttpRequest) -> Result<Claims, HttpResponse> {
        let claims = match Self::require_role(req, &["user", "admin", "guardian"]) {
            Ok(c) => c,
            Err(resp) => return Err(resp),
        };

        // Vérifier privilège dans la DB du compte
        let db_path = format!("data/users/{}.db", claims.sub.replace("-", "_").to_lowercase());
        
        if let Ok(conn) = rusqlite::Connection::open(&db_path) {
            let enabled: i64 = conn.query_row(
                "SELECT COALESCE(enabled, 0) FROM privileges WHERE privilege_name = 'arbitration_judge'",
                [],
                |row| row.get(0),
            ).unwrap_or(0);

            if enabled == 1 {
                return Ok(claims);
            }
        }

        Err(HttpResponse::Forbidden().json(json!({
            "status": "error",
            "message": "Privilège arbitrage requis"
        })))
    }

    // ====================
    // VÉRIFICATION DU RÔLE VALIDATOR
    // = owner OU privilege validator_node
    // ====================
    pub fn require_validator(req: &HttpRequest) -> Result<Claims, HttpResponse> {
        let claims = match Self::require_role(req, &["user", "admin", "guardian"]) {
            Ok(c) => c,
            Err(resp) => return Err(resp),
        };

        let db_path = format!("data/users/{}.db", claims.sub.replace("-", "_").to_lowercase());
        
        if let Ok(conn) = rusqlite::Connection::open(&db_path) {
            let enabled: i64 = conn.query_row(
                "SELECT COALESCE(enabled, 0) FROM privileges WHERE privilege_name = 'validator_node'",
                [],
                |row| row.get(0),
            ).unwrap_or(0);

            if enabled == 1 {
                return Ok(claims);
            }
        }

        Err(HttpResponse::Forbidden().json(json!({
            "status": "error",
            "message": "Privilège validateur requis"
        })))
    }
}
