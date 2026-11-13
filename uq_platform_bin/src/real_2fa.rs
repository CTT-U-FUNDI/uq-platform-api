use std::sync::Mutex;
use std::collections::HashMap;
use crate::totp::TotpSecret;

/// État global 2FA avec secrets persistants
pub struct TwoFaState {
    pub secrets: Mutex<HashMap<String, TotpSecret>>,
}

impl TwoFaState {
    pub fn new() -> Self {
        Self {
            secrets: Mutex::new(HashMap::new()),
        }
    }
    
    /// Génère un nouveau secret pour un compte (au moment de la création)
    pub fn enroll(&self, account_id: &str) -> serde_json::Value {
        let secret = TotpSecret::new(account_id);
        let otpauth_uri = secret.otpauth_uri();
        let mut secrets = self.secrets.lock().unwrap();
        secrets.insert(account_id.to_string(), secret);
        
        serde_json::json!({
            "status": "success",
            "account_id": account_id,
            "otpauth_uri": otpauth_uri,
            "message": "Scannez ce QR code avec Google Authenticator"
        })
    }
    
    /// Vérifie un code 2FA
    pub fn verify(&self, account_id: &str, code: &str) -> bool {
        let secrets = self.secrets.lock().unwrap();
        if let Some(secret) = secrets.get(account_id) {
            return secret.verify(code);
        }
        false
    }
    
    /// Récupère le code actuel pour un compte (DEBUG UNIQUEMENT — ne jamais exposer en production)
    pub fn get_current_code(&self, account_id: &str) -> Option<String> {
        let secrets = self.secrets.lock().unwrap();
        if let Some(secret) = secrets.get(account_id) {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            return Some(secret.generate_code(now));
        }
        None
    }
}