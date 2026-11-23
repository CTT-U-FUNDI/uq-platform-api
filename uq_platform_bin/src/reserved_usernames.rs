// ====================
// NOMS RESERVES UQ
// Politique : Interdiction d'utiliser ces noms comme email/pseudo
// ====================

/// Liste complete des noms reserves (email et pseudo)
pub const RESERVED_USERNAMES: &[&str] = &[
    // ===== ADMINISTRATION =====
    "admin",
    "administrator",
    "root",
    "system",
    "official",
    "moderator",
    "mod",
    "staff",
    "team",
    "operator",
    // ===== SUPPORT =====
    "support",
    "help",
    "contact",
    "info",
    "publicite",
    "trading",
    // ===== SECURITE =====
    "security",
    "abuse",
    "postmaster",
    "hostmaster",
    "webmaster",
    "noreply",
    "no-reply",
    "mailer",
    "daemon",
    // ===== UQ SPECIFIQUE =====
    "uq",
    "ufundi",
    "zando",
    "delis",
    "owner",
    "guardian",
    "gardien",
    // ===== IA =====
    "ai",
    "bot",
    "assistant",
    "fixo",
    "uqai",
    "ia",
    "robot",
    // ===== SYSTEME =====
    "system",
    "server",
    "api",
    "database",
    "dns",
    "mail",
    "ftp",
    "smtp",
    "pop",
    "imap",
    "http",
    "https",
    "www",
    "web",
    "app",
    "mobile",
    "desktop",
    // ===== FINANCE =====
    "bank",
    "banking",
    "wallet",
    "coffre",
    "money",
    "pay",
    "payment",
    "billing",
    "invoice",
    "finance",
    "treasury",
    "tresor",
    // ===== LEGAL =====
    "legal",
    "privacy",
    "terms",
    "conditions",
    "policy",
    "reglement",
    "loi",
    "law",
    // ===== DIVERS =====
    "test",
    "demo",
    "example",
    "sample",
    "temp",
    "temporary",
    "null",
    "undefined",
    "void",
    "none",
    "anonymous",
    "anonyme",
    "guest",
    "invite",
    "user",
    "utilisateur",
    "nouveau",
    "new",
];

/// Verifie si un nom (email ou pseudo) est reserve
pub fn is_reserved(name: &str) -> bool {
    let normalized = name.to_lowercase();
    RESERVED_USERNAMES.contains(&normalized.as_str())
}

/// Verifie si une adresse email est reservee
/// Extrait la partie avant @ et verifie
pub fn is_reserved_email(email: &str) -> bool {
    let local_part = email.split('@').next().unwrap_or("").to_lowercase();
    is_reserved(&local_part)
}

/// Verifie si un pseudo est reserve
/// Retire le prefixe uq@ et verifie
pub fn is_reserved_pseudo(pseudo: &str) -> bool {
    let name = pseudo.trim_start_matches("uq@").to_lowercase();
    is_reserved(&name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_admin_reserve() {
        assert!(is_reserved("admin"));
        assert!(is_reserved("ADMIN"));
        assert!(is_reserved("Admin"));
    }

    #[test]
    fn test_support_reserve() {
        assert!(is_reserved("support"));
        assert!(is_reserved_email("support@zando.uq"));
        assert!(is_reserved_pseudo("uq@support"));
    }

    #[test]
    fn test_eliezer_non_reserve() {
        assert!(!is_reserved("eliezer"));
        assert!(!is_reserved_email("eliezer@zando.uq"));
        assert!(!is_reserved_pseudo("uq@eliezer"));
    }
}
