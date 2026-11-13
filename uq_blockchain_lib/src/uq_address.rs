// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// UQ ADDRESS â€” GÃ©nÃ©ration des adresses UQ
// Format : UQ-[PAYS(3)][NOM+PRÃ‰NOM(5)][POSITION(1+)]
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

use crate::error::ChainError;

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// CONVERSION GSM (tÃ©lÃ©phone)
// A,B,C â†’ 2 | D,E,F â†’ 3 | G,H,I â†’ 4 | J,K,L â†’ 5
// M,N,O â†’ 6 | P,Q,R,S â†’ 7 | T,U,V â†’ 8 | W,X,Y,Z â†’ 9
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

pub fn letter_to_gsm(c: char) -> Option<u8> {
    match c.to_ascii_uppercase() {
        'A' | 'B' | 'C' => Some(2),
        'D' | 'E' | 'F' => Some(3),
        'G' | 'H' | 'I' => Some(4),
        'J' | 'K' | 'L' => Some(5),
        'M' | 'N' | 'O' => Some(6),
        'P' | 'Q' | 'R' | 'S' => Some(7),
        'T' | 'U' | 'V' => Some(8),
        'W' | 'X' | 'Y' | 'Z' => Some(9),
        _ => None,
    }
}

/// Convertit une lettre en chiffre GSM (retourne 0 si invalide)
pub fn letter_to_digit(c: char) -> u8 {
    letter_to_gsm(c).unwrap_or(0)
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// CONVERSION PAYS (3 lettres â†’ 3 chiffres)
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

pub fn country_to_gsm(country_code: &str) -> String {
    let mut result = String::new();
    for c in country_code.chars().take(3) {
        result.push_str(&letter_to_digit(c).to_string());
    }
    // Pad avec 0 si moins de 3 lettres
    while result.len() < 3 {
        result.push('0');
    }
    result
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// GÃ‰NÃ‰RATION DU NUMÃ‰RO UQ
// Format : UQ-[PAYS(3)][NOM+PRÃ‰NOM(5)][POSITION(1+)]
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

use rand::Rng;

/// Longueur du suffixe aléatoire (chiffres) après le code pays.
/// 9 chiffres → ~1 milliard de combinaisons par pays : imprévisible.
const UQ_RANDOM_DIGITS: usize = 9;

/// Génère un numéro UQ : UQ-[PAYS_GSM(3)][ALÉATOIRE(9 chiffres)].
/// Le nom/prénom ne figurent PLUS dans le numéro : seul le code pays
/// (conversion GSM des 3 lettres) est déterministe, le reste est
/// tiré d'un RNG cryptographique. Chaque appel produit une valeur
/// différente — la boucle de collision de l'appelant gère l'unicité.
pub fn generate_uq_number(country_code: &str) -> String {
    let pays = country_to_gsm(country_code);

    let mut rng = rand::rngs::OsRng;
    let mut suffix = String::with_capacity(UQ_RANDOM_DIGITS);
    // Premier chiffre : 1-9 (évite les zéros en tête trompeurs)
    suffix.push_str(&rng.gen_range(1..=9).to_string());
    for _ in 1..UQ_RANDOM_DIGITS {
        suffix.push_str(&rng.gen_range(0..=9).to_string());
    }

    format!("UQ-{}{}", pays, suffix)
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// GÃ‰NÃ‰RATION DU MAIL UQ
// Format : [prÃ©nom]@zando.uq (users) | delis@ufundi.uq (propriÃ©taire)
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

pub fn generate_uq_email(prenom: &str, existing_count: u64) -> String {
    let base = prenom.to_lowercase().replace(' ', "");
    if existing_count == 0 {
        format!("{}@zando.uq", base)
    } else {
        format!("{}{}@zando.uq", base, existing_count)
    }
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// GÃ‰NÃ‰RATION DU PSEUDO UQ
// Format : uq@[prÃ©nom]
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

/// Replie les accents courants en ASCII pour les pseudos.
fn fold_ascii(c: char) -> Option<char> {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' => Some('a'),
        'ç' | 'ć' | 'č' => Some('c'),
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ė' | 'ę' => Some('e'),
        'ì' | 'í' | 'î' | 'ï' | 'ī' => Some('i'),
        'ñ' => Some('n'),
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ō' => Some('o'),
        'ù' | 'ú' | 'û' | 'ü' | 'ū' => Some('u'),
        'ý' | 'ÿ' => Some('y'),
        'ß' => Some('s'),
        'æ' => Some('a'),
        'œ' => Some('o'),
        c if c.is_ascii_alphanumeric() => Some(c.to_ascii_lowercase()),
        _ => None,
    }
}

/// Nettoie une chaîne pour usage dans un pseudo : minuscules ASCII uniquement.
fn clean_pseudo_part(s: &str) -> String {
    s.chars().filter_map(fold_ascii).collect()
}

/// Génère le pseudo : uq@[initiale_prénom].[nom]
/// Exemples : "Jean Mutombo" → uq@j.mutombo, "Marie Kabila" → uq@m.kabila
/// En cas de collision, l'appelant réessaie avec existing_count > 0
/// qui ajoute un suffixe numérique : uq@j.mutombo1, uq@j.mutombo2...
pub fn generate_uq_pseudo(prenom: &str, nom: &str, existing_count: u64) -> String {
    let p = clean_pseudo_part(prenom);
    let n = clean_pseudo_part(nom);

    let base = if p.is_empty() && n.is_empty() {
        "uq@anonyme".to_string()
    } else if n.is_empty() {
        // Pas de nom : uq@{prénom}
        format!("uq@{}", p)
    } else {
        // Initiale du prénom + "." + nom (tronqué à 20 chars)
        let initiale: String = p.chars().take(1).collect();
        let nom_tronque: String = n.chars().take(20).collect();
        format!("uq@{}.{}", initiale, nom_tronque)
    };

    if existing_count == 0 {
        base
    } else {
        format!("{}{}", base, existing_count)
    }
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// VALIDATION
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

pub fn is_valid_uq_number(n: &str) -> bool {
    if !n.starts_with("UQ-") { return false; }
    let digits = &n[3..];
    digits.len() >= 8 && digits.len() <= 15 && digits.chars().all(|c| c.is_ascii_digit())
}

pub fn is_valid_uq_email(e: &str) -> bool {
    e.ends_with("@zando.uq") || e.ends_with("@ufundi.uq")
}

pub fn is_valid_uq_pseudo(p: &str) -> bool {
    p.starts_with("uq@") && p.len() >= 4 && p.len() <= 30
}

// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•
// TESTS
// â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•â•

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_letter_to_digit() {
        assert_eq!(letter_to_digit('A'), 2);
        assert_eq!(letter_to_digit('M'), 6);
        assert_eq!(letter_to_digit('Z'), 9);
    }

    #[test]
    fn test_country_to_gsm() {
        // "CD" (Congo) â†’ C=2, D=3, pad 0 â†’ "230"
        assert_eq!(country_to_gsm("CD"), "230");
        // "RDC" â†’ R=7, D=3, C=2 â†’ "732"
        assert_eq!(country_to_gsm("RDC"), "732");
    }

    #[test]
    fn test_generate_uq_number_jean() {
        // Nouveau format : UQ-[PAYS_GSM(3)][ALÉATOIRE(9)]
        // RDC → 732, le reste est aléatoire
        let n = generate_uq_number("RDC");
        assert!(n.starts_with("UQ-732"), "Got: {}", n);
        assert!(is_valid_uq_number(&n), "Got: {}", n);
        // Deux appels → valeurs différentes (aléatoire)
        let n2 = generate_uq_number("RDC");
        assert_ne!(n, n2, "Le générateur doit être aléatoire");
    }

    #[test]
    fn test_generate_uq_email() {
        assert_eq!(generate_uq_email("jean", 0), "jean@zando.uq");
        assert_eq!(generate_uq_email("jean", 1), "jean1@zando.uq");
    }

    #[test]
    fn test_generate_uq_pseudo() {
        // Format uq@initiale.nom
        assert_eq!(generate_uq_pseudo("jean", "mutombo", 0), "uq@j.mutombo");
        assert_eq!(generate_uq_pseudo("jean", "mutombo", 1), "uq@j.mutombo1");
        // Accents gérés
        assert_eq!(generate_uq_pseudo("josé", "mbala", 0), "uq@j.mbala");
        // Sans nom → fallback prénom
        assert_eq!(generate_uq_pseudo("jean", "", 0), "uq@jean");
    }

    #[test]
    fn test_valid_number() {
        assert!(is_valid_uq_number("UQ-266688531"));
        assert!(!is_valid_uq_number("+33 7 12 34 56 78"));
        assert!(!is_valid_uq_number("UQ-abc"));
    }
}

