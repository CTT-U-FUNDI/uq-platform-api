// ====================
// UTILITAIRES USERNAME UQ
// Normalisation + validation (politique Telegram)
// ====================

use crate::reserved_usernames::is_reserved;

// ====================
// NORMALISATION
// ====================

/// Normalise un nom en minuscules, sans points, tirets, underscores
/// Utilise pour comparer (eviter les collisions)
///
/// Exemples :
///   "Eliezer.Mukendi" -> "eliezermukendi"
///   "eliezer_mukendi" -> "eliezermukendi"
///   "E.L.I.E.Z.E.R"   -> "eliezer"
pub fn normalize_username(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '.')
        .collect()
}

/// Normalise un email (retire la partie @domaine)
pub fn normalize_email(email: &str) -> String {
    let local_part = email.split('@').next().unwrap_or(email);
    normalize_username(local_part)
}

/// Normalise un pseudo (retire le prefixe uq@)
pub fn normalize_pseudo(pseudo: &str) -> String {
    let name = pseudo.trim_start_matches("uq@").trim_start_matches("uq.");
    format!("uq.{}", normalize_username(name))
}

// ====================
// VALIDATION POLITIQUE TELEGRAM
// ====================

/// Verifie si un username respecte la politique Telegram :
/// - 5 a 32 caracteres
/// - a-z, 0-9, underscore uniquement
/// - Ne commence pas par un chiffre
/// - N'est pas entierement numerique
pub fn is_valid_telegram_username(username: &str) -> bool {
    let len = username.len();

    // 1. Longueur : 5-32 caracteres
    if len < 5 || len > 32 {
        return false;
    }

    // 2. Caracteres autorises : a-z, 0-9, _, .
    if !username.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.') {
        return false;
    }

    // 3. Ne commence pas par un chiffre
    if let Some(first) = username.chars().next() {
        if first.is_ascii_digit() {
            return false;
        }
    }

    // 4. Pas entierement numerique
    if username.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }

    // 5. Pas de double underscore
    if username.contains("__") {
        return false;
    }

    // 6. Ne commence/finit pas par underscore
    if username.starts_with('_') || username.ends_with('_') {
        return false;
    }

    // 7. Pas de point au debut/fin
    if username.starts_with('.') || username.ends_with('.') {
        return false;
    }

    // 8. Pas de double point
    if username.contains("..") {
        return false;
    }

    true
}

/// Valide un pseudo complet (avec prefixe uq@)
pub fn is_valid_uq_pseudo(pseudo: &str) -> bool {
    let name = if pseudo.starts_with("uq@") {
        &pseudo[3..]
    } else if pseudo.starts_with("uq.") {
        &pseudo[3..]
    } else {
        return false;
    };
    is_valid_telegram_username(name)
}

// ====================
// VALIDATION COMPLETE
// ====================

/// Verifie si un username est completement valide (format + reserve)
pub fn is_username_valid_and_available(username: &str) -> bool {
    // 1. Format Telegram valide
    if !is_valid_telegram_username(username) {
        return false;
    }

    // 2. Pas reserve
    if is_reserved(username) {
        return false;
    }

    true
}

// ====================
// GENERATION BASE
// ====================

/// Genere la base d'un username a partir du prenom/nom
/// "Eliezer", "Mukendi" -> "eliezermukendi"
pub fn generate_username_base(prenom: &str, nom: &str) -> String {
    let p = normalize_username(prenom);
    let n = normalize_username(nom);
    format!("{}{}", p, n)
}

/// Ajoute un chiffre a la fin si collision
/// "eliezer", 0 -> "eliezer"
/// "eliezer", 1 -> "eliezer1"
pub fn add_collision_suffix(base: &str, count: u64) -> String {
    if count == 0 {
        base.to_string()
    } else {
        format!("{}{}", base, count)
    }
}

// ====================
// DISTANCE DE LEVENSHTEIN (pour futures fonctions)
// ====================

/// Calcule la distance de Levenshtein entre 2 chaines
pub fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let a_len = a_chars.len();
    let b_len = b_chars.len();

    if a_len == 0 { return b_len; }
    if b_len == 0 { return a_len; }

    let mut matrix = vec![vec![0usize; b_len + 1]; a_len + 1];

    for i in 0..=a_len { matrix[i][0] = i; }
    for j in 0..=b_len { matrix[0][j] = j; }

    for i in 1..=a_len {
        for j in 1..=b_len {
            let cost = if a_chars[i - 1] == b_chars[j - 1] { 0 } else { 1 };
            matrix[i][j] = std::cmp::min(
                std::cmp::min(
                    matrix[i - 1][j] + 1,
                    matrix[i][j - 1] + 1,
                ),
                matrix[i - 1][j - 1] + cost,
            );
        }
    }

    matrix[a_len][b_len]
}

// ====================
// TESTS
// ====================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize() {
        assert_eq!(normalize_username("Eliezer.Mukendi"), "eliezermukendi");
        assert_eq!(normalize_username("eliezer_mukendi"), "eliezermukendi");
        assert_eq!(normalize_username("E.L.I.E.Z.E.R"), "eliezer");
        assert_eq!(normalize_email("Eliezer@zando.uq"), "eliezer");
        assert_eq!(normalize_pseudo("uq@Eliezer"), "eliezer");
    }

    #[test]
    fn test_telegram_valid() {
        assert!(is_valid_telegram_username("eliezer"));
        assert!(is_valid_telegram_username("eliezer_mukendi"));
        assert!(is_valid_telegram_username("eliezer123"));
        assert!(!is_valid_telegram_username("eli"));           // trop court
        assert!(!is_valid_telegram_username("123eliezer"));    // commence par chiffre
        assert!(!is_valid_telegram_username("eliezer!"));      // caractere interdit
        assert!(!is_valid_telegram_username("_eliezer"));      // commence par _
    }

    #[test]
    fn test_pseudo_uq() {
        assert!(is_valid_uq_pseudo("uq@eliezer"));
        assert!(!is_valid_uq_pseudo("eliezer"));
        assert!(!is_valid_uq_pseudo("uq@123"));
    }

    #[test]
    fn test_reserved() {
        assert!(!is_username_valid_and_available("admin"));
        assert!(!is_username_valid_and_available("support"));
        assert!(is_username_valid_and_available("eliezer"));
    }

    #[test]
    fn test_generate_base() {
        assert_eq!(generate_username_base("Eliezer", "Mukendi"), "eliezermukendi");
        assert_eq!(generate_username_base("eliezer", "kabila"), "eliezerkabila");
    }

    #[test]
    fn test_collision_suffix() {
        assert_eq!(add_collision_suffix("eliezer", 0), "eliezer");
        assert_eq!(add_collision_suffix("eliezer", 1), "eliezer1");
        assert_eq!(add_collision_suffix("eliezer", 42), "eliezer42");
    }

    #[test]
    fn test_levenshtein() {
        assert_eq!(levenshtein_distance("eliezer", "eliezer"), 0);
        assert_eq!(levenshtein_distance("eliezer", "eliez3r"), 1);
        assert_eq!(levenshtein_distance("eliezer", "eliezr"), 1);
        assert_eq!(levenshtein_distance("eliezer", "eiezer"), 1);
    }
}
