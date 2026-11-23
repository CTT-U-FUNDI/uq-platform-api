// ====================
// SECURITY — Helpers de sécurité pour tous les handlers
// ====================
//
// Ce module centralise :
// - Vérification du JWT (sender = JWT, pas body)
// - Validation des montants
// - Protection contre les transferts frauduleux
//
// ====================

use actix_web::{HttpRequest, HttpResponse};
use serde_json::json;

/// Vérifie que l'utilisateur connecté est bien l'expéditeur
/// Retourne le UQ de l'utilisateur authentifié (depuis le JWT)
pub fn verify_sender(req: &HttpRequest, body_sender: Option<&str>) -> Result<String, HttpResponse> {
    let claims = crate::auth::AuthManager::require_role(req, &["user", "admin", "guardian"])?;
    let sender = claims.sub.clone();

    // Si un sender est fourni dans le body, il doit correspondre
    if let Some(bs) = body_sender {
        if !bs.is_empty() && bs != sender {
            eprintln!(
                "🚨 TENTATIVE FRAUDE : {} essaie d'envoyer au nom de {}",
                sender, bs
            );
            return Err(HttpResponse::Forbidden().json(json!({
                "status": "error",
                "code": "SENDER_MISMATCH",
                "message": "Expéditeur non autorisé"
            })));
        }
    }

    Ok(sender)
}

/// Vérifie que l'utilisateur peut voir ce compte
/// (soit c'est son compte, soit il est admin)
pub fn verify_account_access(req: &HttpRequest, account_id: &str) -> Result<(), HttpResponse> {
    let claims = crate::auth::AuthManager::require_role(req, &["user", "admin", "guardian"])?;

    // Owner et admin peuvent tout voir
    if claims.role == "admin" || claims.role == "guardian" {
        return Ok(());
    }

    // Un user ne peut voir que SON compte
    if claims.sub != account_id {
        eprintln!(
            "🚨 ACCÈS REFUSÉ : {} essaie de voir {}",
            claims.sub, account_id
        );
        return Err(HttpResponse::Forbidden().json(json!({
            "status": "error",
            "code": "ACCESS_DENIED",
            "message": "Accès à ce compte refusé"
        })));
    }

    Ok(())
}

/// Valide un montant de transfert
/// Retourne (amount, fee) validés
pub fn validate_transfer_amount(
    amount_points: u64,
    fee_points: u64,
    max_balance: i64,
) -> Result<(u64, u64), HttpResponse> {
    // 1. Montant > 0
    if amount_points == 0 {
        return Err(HttpResponse::BadRequest().json(json!({
            "status": "error",
            "code": "AMOUNT_ZERO",
            "message": "Montant ne peut pas être 0"
        })));
    }

    // 2. Fee ≤ 1% du montant (0.1% + marge)
    let max_fee = (amount_points / 100).max(1);
    if fee_points > max_fee {
        return Err(HttpResponse::BadRequest().json(json!({
            "status": "error",
            "code": "FEE_TOO_HIGH",
            "message": format!("Frais trop élevés. Max: {} points", max_fee)
        })));
    }

    // 3. Total ≤ solde
    let total = (amount_points as i64) + (fee_points as i64);
    if total > max_balance {
        return Err(HttpResponse::BadRequest().json(json!({
            "status": "error",
            "code": "INSUFFICIENT_BALANCE",
            "message": format!("Solde insuffisant. Requis: {} points, Disponible: {}", total, max_balance)
        })));
    }

    Ok((amount_points, fee_points))
}

/// Calcule les frais côté serveur (ne jamais faire confiance au client)
pub fn calculate_fee(amount_points: u64) -> u64 {
    // 0.1% du montant, minimum 1 point
    (amount_points / 1000).max(1)
}
