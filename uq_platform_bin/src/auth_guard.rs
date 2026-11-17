// ====================
// 🔐 AUTH GUARD — Helpers d'autorisation réutilisables
// ====================
//
// Trois niveaux, conformes à l'audit :
//
//   PUBLIC : pas d'appel (health, price, oracle, QR, pseudos…)
//   USER   : exiger_auth()          → JWT valide obligatoire
//            exiger_soi_ou_owner()  → en plus, claims.sub == cible
//   OWNER  : exiger_owner()         → UQ-996697110 uniquement
//
// Toute réponse d'échec est normalisée (401 / 403) et ne révèle rien.
// ====================

use actix_web::{HttpRequest, HttpResponse};
use serde_json::json;

use crate::auth::{AuthManager, Claims};

/// UQ du propriétaire de la plateforme
pub const OWNER_UQ: &str = "UQ-996697110";

/// Rôles acceptés pour un utilisateur normal
pub const ROLES_USER: &[&str] = &["user", "admin", "guardian", "owner"];
/// Rôles acceptés pour une opération réservée
pub const ROLES_OWNER: &[&str] = &["admin", "guardian", "owner"];

// ====================
// RÉPONSES NORMALISÉES
// ====================

pub fn non_authentifie(message: &str) -> HttpResponse {
    HttpResponse::Unauthorized().json(json!({
        "status": "error",
        "code": "NON_AUTHENTIFIE",
        "message": message
    }))
}

pub fn refus() -> HttpResponse {
    HttpResponse::Forbidden().json(json!({
        "status": "error",
        "code": "ACCES_REFUSE",
        "message": "Accès refusé — tu ne peux accéder qu'à tes propres données"
    }))
}

pub fn refuse_owner() -> HttpResponse {
    HttpResponse::Forbidden().json(json!({
        "status": "error",
        "code": "RESERVE_OWNER",
        "message": "Réservé au propriétaire"
    }))
}

// ====================
// NIVEAU 1 — AUTHENTIFICATION
// ====================

/// JWT obligatoire (n'importe quel rôle utilisateur).
pub fn exiger_auth(req: &HttpRequest) -> Result<Claims, HttpResponse> {
    AuthManager::require_role(req, ROLES_USER)
}

// ====================
// NIVEAU 2 — PROPRIÉTÉ
// ====================

pub fn est_owner(claims: &Claims) -> bool {
    claims.sub == OWNER_UQ
        || claims.role == "admin"
        || claims.role == "owner"
        || crate::owner_guard::check_is_owner(&claims.sub)
}

/// L'appelant peut-il accéder aux données de `cible` ?
pub fn acces_autorise(claims: &Claims, cible: &str) -> bool {
    if est_owner(claims) {
        return true;
    }
    claims.sub.eq_ignore_ascii_case(cible.trim())
}

/// JWT valide **et** `claims.sub == cible` (sauf owner).
pub fn exiger_soi_ou_owner(req: &HttpRequest, cible: &str) -> Result<Claims, HttpResponse> {
    let claims = exiger_auth(req)?;
    if !acces_autorise(&claims, cible) {
        eprintln!(
            "🚫 [AUTH] {} a tenté d'accéder aux données de {}",
            claims.sub, cible
        );
        return Err(refus());
    }
    Ok(claims)
}

// ====================
// NIVEAU 3 — OWNER
// ====================

pub fn exiger_owner(req: &HttpRequest) -> Result<Claims, HttpResponse> {
    let claims = match AuthManager::require_role(req, ROLES_OWNER) {
        Ok(c) => c,
        Err(_) => return Err(non_authentifie("Authentification requise")),
    };

    if !est_owner(&claims) {
        eprintln!("🚫 [AUTH] {} a tenté une opération réservée à l'owner", claims.sub);
        return Err(refuse_owner());
    }
    Ok(claims)
}

/// Un paramètre de chemin ressemble-t-il à un UQ number ?
/// (permet d'éviter de comparer un id de message avec un UQ)
pub fn ressemble_a_un_uq(valeur: &str) -> bool {
    let v = valeur.trim().to_uppercase();
    v.starts_with("UQ-") && v.len() > 3 && v[3..].chars().all(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claims(sub: &str, role: &str) -> Claims {
        Claims {
            sub: sub.to_string(),
            role: role.to_string(),
            exp: 0,
            iat: 0,
        }
    }

    #[test]
    fn uq_reconnu() {
        assert!(ressemble_a_un_uq("UQ-996697110"));
        assert!(ressemble_a_un_uq("uq-732688531"));
        assert!(!ressemble_a_un_uq("uq@jean"));
        assert!(!ressemble_a_un_uq("42"));
        assert!(!ressemble_a_un_uq(""));
    }

    #[test]
    fn acces_soi_ou_owner() {
        let moi = claims("UQ-732688531", "user");
        let autre = claims("UQ-230472511", "user");
        let owner = claims(OWNER_UQ, "owner");

        assert!(acces_autorise(&moi, "UQ-732688531"));
        assert!(acces_autorise(&moi, "uq-732688531")); // insensible à la casse
        assert!(!acces_autorise(&moi, "UQ-996697110"));
        assert!(!acces_autorise(&autre, "UQ-732688531"));
        assert!(acces_autorise(&owner, "UQ-732688531"));
    }
}
