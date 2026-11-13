// ====================
// OWNER GUARD — Vérifie que l'utilisateur est owner OU propriétaire
// ====================

use actix_web::guard::GuardContext;
use actix_web::http::header;

/// Préfixes des routes réservées (owner OU propriétaire du compte)
pub const OWNER_PREFIXES: &[&str] = &[
    "/api/treasury/",
    "/api/counter/",
    "/api/cases/",
    "/api/distribution/",
    "/api/admin/",
    "/api/system/",
    "/api/dev/",
    "/api/fraud/",
    "/api/arbitration/",
    "/api/validator/",
    "/api/oracle/",
    "/api/receipt/",
    "/api/guardian/",
    "/api/borb/",
    "/api/account/",
];

/// Vérifie si un chemin est réservé
pub fn is_owner_path(path: &str) -> bool {
    OWNER_PREFIXES.iter().any(|prefix| path.starts_with(prefix))
}

/// Vérifie si l'utilisateur est owner (via uq_index)
pub fn check_is_owner(uq_number: &str) -> bool {
    let idx = crate::uq_index::UqIndex::shared("uq_index.db");
    match idx.get_account(uq_number) {
        Some(acc) => acc.get("is_owner").and_then(|v| v.as_bool()).unwrap_or(false),
        None => false,
    }
}

/// Extrait le UQ number depuis le path (ex: /api/borb/balance/UQ-00000001 → UQ-00000001)
fn extract_uq_from_path(path: &str) -> Option<String> {
    // Patterns courants :
    // - /api/borb/balance/{id}
    // - /api/account/preferences/{id}
    // - /api/account/{id}/balance
    // - /api/borb/transactions/{id}
    
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    
    for part in parts.iter().rev() {
        // Chercher un UQ-XXXXXXXX
        if part.starts_with("UQ-") || part.starts_with("uq-") {
            return Some(part.to_string());
        }
    }
    
    None
}

/// Guard actix-web : vérifie que l'utilisateur est owner OU propriétaire du compte
pub fn owner_guard() -> impl actix_web::guard::Guard {
    struct OwnerCheck;

    impl actix_web::guard::Guard for OwnerCheck {
        fn check(&self, ctx: &GuardContext<'_>) -> bool {
            let head = ctx.head();
            let path = head.uri.path();

            // 1. Si pas une route protégée → laisser passer
            if !is_owner_path(path) {
                return true;
            }

            // 2. Extraire le JWT
            let token = match head.headers.get(header::AUTHORIZATION) {
                Some(h) => match h.to_str() {
                    Ok(s) => match s.strip_prefix("Bearer ") {
                        Some(t) => t.to_string(),
                        None => {
                            eprintln!("🛡️  [OWNER-GUARD] Header malformé : {}", path);
                            return false;
                        }
                    },
                    Err(_) => {
                        eprintln!("🛡️  [OWNER-GUARD] Header invalide : {}", path);
                        return false;
                    }
                },
                None => {
                    eprintln!("🛡️  [OWNER-GUARD] Pas de token : {}", path);
                    return false;
                }
            };

            // 3. Vérifier le JWT
            let claims = match crate::auth::AuthManager::verify_jwt(&token) {
                Ok(c) => c,
                Err(_) => {
                    eprintln!("🛡️  [OWNER-GUARD] JWT invalide : {}", path);
                    return false;
                }
            };

            let user_uq = &claims.sub;

            // 4. Si l'utilisateur est OWNER → OK pour tout
            if check_is_owner(user_uq) {
                eprintln!("✅ [OWNER-GUARD] Owner OK : {} → {}", user_uq, path);
                return true;
            }

            // 5. Si la route contient un UQ number → vérifier que c'est le sien
            if let Some(target_uq) = extract_uq_from_path(path) {
                if target_uq.eq_ignore_ascii_case(user_uq) {
                    eprintln!("✅ [OWNER-GUARD] Propriétaire OK : {} → {}", user_uq, path);
                    return true;
                } else {
                    eprintln!("🚫 [OWNER-GUARD] Pas propriétaire : {} essaie d'accéder à {} ({})", user_uq, target_uq, path);
                    return false;
                }
            }

            // 6. Si la route ne contient PAS d'UQ → routes génériques (cases/stats, price/all)
            //    Accessibles à tous les utilisateurs connectés
            eprintln!("✅ [OWNER-GUARD] Route générique OK : {} → {}", user_uq, path);
            true
        }
    }

    OwnerCheck
}