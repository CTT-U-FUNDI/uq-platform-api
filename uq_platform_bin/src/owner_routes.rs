// ====================
// OWNER ROUTES — Détection automatique des routes owner
// ====================
//
// AJOUTER UN PRÉFIXE ICI = PROTECTION AUTOMATIQUE
// ====================

/// Préfixes des routes réservées au propriétaire
pub const OWNER_PREFIXES: &[&str] = &[
    // Trésorerie
    "/api/treasury/",
    "/api/counter/",
    "/api/cases/",
    "/api/distribution/",
    "/api/invoices/",

    // Admin
    "/api/admin/",
    "/api/system/",
    "/api/dev/",

    // Sécurité avancée
    "/api/fraud/",

    // Governance
    "/api/arbitration/",
    "/api/validator/",
    "/api/oracle/",

    // Reçus
    "/api/receipt/",

    // Guardian
    "/api/guardian/",
];

/// Vérifie si un chemin est réservé au propriétaire
pub fn is_owner_path(path: &str) -> bool {
    OWNER_PREFIXES.iter().any(|prefix| path.starts_with(prefix))
}

/// Retourne la liste des préfixes (pour debug)
pub fn get_owner_prefixes() -> &'static [&'static str] {
    OWNER_PREFIXES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_owner_path() {
        assert!(is_owner_path("/api/cases/stats"));
        assert!(is_owner_path("/api/treasury/wallets"));
        assert!(is_owner_path("/api/fraud/report"));
        assert!(!is_owner_path("/api/borb/balance/UQ-123"));
        assert!(!is_owner_path("/api/account/me"));
        assert!(!is_owner_path("/api/health"));
    }
}