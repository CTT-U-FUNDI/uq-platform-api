// ====================
// VALIDATION — Règles centralisées pour tous les champs
// ====================

// ====================
// CONSTANTES
// ====================

pub const MIN_PRENOM_LEN: usize = 2;
pub const MAX_PRENOM_LEN: usize = 50;
pub const MIN_NOM_LEN: usize = 2;
pub const MAX_NOM_LEN: usize = 50;
pub const MIN_PASSWORD_LEN: usize = 8;
pub const MAX_PASSWORD_LEN: usize = 64;
pub const MIN_AGE: i64 = 13;
pub const MAX_AGE: i64 = 120;
pub const MIN_LIEU_LEN: usize = 2;
pub const MAX_LIEU_LEN: usize = 100;

// ====================
// RÉSULTAT DE VALIDATION
// ====================

#[derive(Debug, Clone)]
pub struct ValidationResult {
    pub valid: bool,
    pub error: Option<String>,
}

impl ValidationResult {
    pub fn ok() -> Self {
        ValidationResult {
            valid: true,
            error: None,
        }
    }

    pub fn err(msg: &str) -> Self {
        ValidationResult {
            valid: false,
            error: Some(msg.to_string()),
        }
    }

    pub fn is_ok(&self) -> bool {
        self.valid
    }

    pub fn message(&self) -> String {
        self.error.clone().unwrap_or_default()
    }
}

// ====================
// HELPERS
// ====================

/// Vérifie qu'une chaîne ne contient que des lettres (unicode), espaces, tirets, apostrophes
fn is_valid_name_chars(s: &str) -> bool {
    s.chars()
        .all(|c| c.is_alphabetic() || c.is_whitespace() || c == '-' || c == '\'' || c == '.')
}

/// Vérifie qu'une chaîne ne contient que des chiffres
fn is_all_digits(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

// ====================
// VALIDATION PRÉNOM
// ====================

pub fn validate_prenom(prenom: &str) -> ValidationResult {
    let cleaned = prenom.trim();

    if cleaned.is_empty() {
        return ValidationResult::err("Entre ton prénom");
    }

    let len = cleaned.chars().count();
    if len < MIN_PRENOM_LEN {
        return ValidationResult::err(&format!("Prénom : minimum {} caractères", MIN_PRENOM_LEN));
    }
    if len > MAX_PRENOM_LEN {
        return ValidationResult::err(&format!("Prénom : maximum {} caractères", MAX_PRENOM_LEN));
    }

    if !is_valid_name_chars(cleaned) {
        return ValidationResult::err("Prénom : lettres uniquement");
    }

    ValidationResult::ok()
}

// ====================
// VALIDATION NOM
// ====================

pub fn validate_nom(nom: &str) -> ValidationResult {
    let cleaned = nom.trim();

    if cleaned.is_empty() {
        return ValidationResult::err("Entre ton nom");
    }

    let len = cleaned.chars().count();
    if len < MIN_NOM_LEN {
        return ValidationResult::err(&format!("Nom : minimum {} caractères", MIN_NOM_LEN));
    }
    if len > MAX_NOM_LEN {
        return ValidationResult::err(&format!("Nom : maximum {} caractères", MAX_NOM_LEN));
    }

    if !is_valid_name_chars(cleaned) {
        return ValidationResult::err("Nom : lettres uniquement");
    }

    ValidationResult::ok()
}

// ====================
// VALIDATION MOT DE PASSE
// ====================

pub fn validate_password(password: &str) -> ValidationResult {
    if password.is_empty() {
        return ValidationResult::err("Entre ton mot de passe");
    }

    if password.len() < MIN_PASSWORD_LEN {
        return ValidationResult::err(&format!("Minimum {} caractères", MIN_PASSWORD_LEN));
    }
    if password.len() > MAX_PASSWORD_LEN {
        return ValidationResult::err(&format!("Maximum {} caractères", MAX_PASSWORD_LEN));
    }

    if !password.chars().any(|c| c.is_ascii_uppercase()) {
        return ValidationResult::err("Au moins 1 majuscule requise");
    }
    if !password.chars().any(|c| c.is_ascii_lowercase()) {
        return ValidationResult::err("Au moins 1 minuscule requise");
    }
    if !password.chars().any(|c| c.is_ascii_digit()) {
        return ValidationResult::err("Au moins 1 chiffre requis");
    }

    ValidationResult::ok()
}

// ====================
// VALIDATION DATE DE NAISSANCE
// ====================

pub fn validate_date_naissance(date: &str) -> ValidationResult {
    if date.is_empty() {
        return ValidationResult::err("Entre ta date de naissance");
    }

    // Attendu : YYYY-MM-DD
    let parts: Vec<&str> = date.split('-').collect();
    if parts.len() != 3 {
        return ValidationResult::err("Format attendu : AAAA-MM-JJ");
    }

    let year: i32 = match parts[0].parse() {
        Ok(y) => y,
        Err(_) => return ValidationResult::err("Année invalide"),
    };
    let month: u32 = match parts[1].parse() {
        Ok(m) => m,
        Err(_) => return ValidationResult::err("Mois invalide"),
    };
    let day: u32 = match parts[2].parse() {
        Ok(d) => d,
        Err(_) => return ValidationResult::err("Jour invalide"),
    };

    if month < 1 || month > 12 {
        return ValidationResult::err("Mois invalide (1-12)");
    }
    if day < 1 || day > 31 {
        return ValidationResult::err("Jour invalide (1-31)");
    }

    // ===== Calculer l' ge =====
    let now = chrono::Utc::now();
    let current_year = now.format("%Y").to_string().parse::<i32>().unwrap_or(2026);
    let current_month = now.format("%m").to_string().parse::<u32>().unwrap_or(1);
    let current_day = now.format("%d").to_string().parse::<u32>().unwrap_or(1);

    let mut age = current_year - year;
    if current_month < month || (current_month == month && current_day < day) {
        age -= 1;
    }

    if age < MIN_AGE as i32 {
        return ValidationResult::err(&format!("Âge minimum : {} ans", MIN_AGE));
    }
    if age > MAX_AGE as i32 {
        return ValidationResult::err("Date invalide");
    }

    ValidationResult::ok()
}

// ====================
// VALIDATION LIEU DE NAISSANCE
// ====================

pub fn validate_lieu_naissance(lieu: &str) -> ValidationResult {
    let cleaned = lieu.trim();

    if cleaned.is_empty() {
        return ValidationResult::err("Entre ton lieu de naissance");
    }

    let len = cleaned.chars().count();
    if len < MIN_LIEU_LEN {
        return ValidationResult::err(&format!("Lieu : minimum {} caractères", MIN_LIEU_LEN));
    }
    if len > MAX_LIEU_LEN {
        return ValidationResult::err(&format!("Lieu : maximum {} caractères", MAX_LIEU_LEN));
    }

    ValidationResult::ok()
}

// ====================
// VALIDATION PAYS
// ====================

pub fn validate_pays(pays: &str) -> ValidationResult {
    let cleaned = pays.trim().to_uppercase();

    if cleaned.is_empty() {
        return ValidationResult::err("Entre ton pays");
    }

    let len = cleaned.chars().count();
    if len < 2 || len > 3 {
        return ValidationResult::err("Code pays : 2 à 3 lettres");
    }

    if !cleaned.chars().all(|c| c.is_ascii_alphabetic()) {
        return ValidationResult::err("Pays : lettres uniquement");
    }

    ValidationResult::ok()
}

// ====================
// VALIDATION EMAIL
// ====================

pub fn validate_email(email: &str) -> ValidationResult {
    let cleaned = email.trim().to_lowercase();

    if cleaned.is_empty() {
        return ValidationResult::err("Entre ton email UQ");
    }

    if !cleaned.ends_with("@zando.uq") && !cleaned.ends_with("@ufundi.uq") {
        return ValidationResult::err("Format : pseudo@zando.uq ou pseudo@ufundi.uq");
    }

    let local = cleaned.split('@').next().unwrap_or("");
    if local.len() < 2 || local.len() > 64 {
        return ValidationResult::err("Pseudo email : 2 à 64 caractères");
    }

    ValidationResult::ok()
}

// ====================
// VALIDATION TÉLÉPHONE
// ====================

pub fn validate_telephone(tel: &str) -> ValidationResult {
    let cleaned = tel.replace(|c: char| c.is_whitespace() || c == '.' || c == '-', "");

    if cleaned.is_empty() {
        return ValidationResult::ok(); // Optionnel
    }

    if !cleaned.starts_with('+') {
        return ValidationResult::err("Format : +XXX XXXXXXXXX");
    }

    let digits = &cleaned[1..];
    if !is_all_digits(digits) {
        return ValidationResult::err("Numéro : chiffres uniquement après +");
    }

    if digits.len() < 9 || digits.len() > 15 {
        return ValidationResult::err("Numéro : 9 à 15 chiffres");
    }

    ValidationResult::ok()
}

// ====================
// VALIDATION PSEUDO
// ====================

pub fn validate_pseudo(pseudo: &str) -> ValidationResult {
    let cleaned = pseudo.trim().to_lowercase();

    if cleaned.is_empty() {
        return ValidationResult::err("Entre ton pseudo");
    }

    if !cleaned.starts_with("uq@") {
        return ValidationResult::err("Le pseudo doit commencer par uq@");
    }

    if cleaned.len() < 4 || cleaned.len() > 30 {
        return ValidationResult::err("Pseudo : 4 à 30 caractères");
    }

    ValidationResult::ok()
}

// ====================
// VALIDATION UQ NUMBER
// ====================

pub fn validate_uq_number(uq: &str) -> ValidationResult {
    let cleaned = uq.trim().to_uppercase();

    if cleaned.is_empty() {
        return ValidationResult::err("UQ number requis");
    }

    if !cleaned.starts_with("UQ-") {
        return ValidationResult::err("Le numéro doit commencer par UQ-");
    }

    let digits = &cleaned[3..];
    if digits.len() < 8 || digits.len() > 15 {
        return ValidationResult::err("Format : UQ-XXXXXXXX (8 à 15 chiffres)");
    }

    if !is_all_digits(digits) {
        return ValidationResult::err("UQ number : chiffres uniquement");
    }

    ValidationResult::ok()
}

// ====================
// VALIDATION COMPLÈTE D'UN COMPTE
// ====================

pub struct AccountInput {
    pub prenom: String,
    pub nom: String,
    pub password: String,
    pub date_naissance: String,
    pub lieu_naissance: String,
    pub pays: String,
    pub telephone: String,
}

pub fn validate_account_input(input: &AccountInput) -> ValidationResult {
    let checks: Vec<ValidationResult> = vec![
        validate_prenom(&input.prenom),
        validate_nom(&input.nom),
        validate_password(&input.password),
        validate_date_naissance(&input.date_naissance),
        validate_lieu_naissance(&input.lieu_naissance),
        validate_pays(&input.pays),
        validate_telephone(&input.telephone),
    ];

    for check in checks {
        if !check.is_ok() {
            return check;
        }
    }

    ValidationResult::ok()
}

// ====================
// TESTS
// ====================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prenom_valide() {
        assert!(validate_prenom("Jean").is_ok());
        assert!(validate_prenom("Éliezer").is_ok());
        assert!(!validate_prenom("J").is_ok());
        assert!(!validate_prenom("").is_ok());
        assert!(!validate_prenom("Jean123").is_ok());
    }

    #[test]
    fn test_nom_valide() {
        assert!(validate_nom("Destin").is_ok());
        assert!(validate_nom("Mukendi-Kabila").is_ok());
        assert!(!validate_nom("D").is_ok());
        assert!(!validate_nom("").is_ok());
    }

    #[test]
    fn test_password_valide() {
        assert!(validate_password("Jean1234").is_ok());
        assert!(validate_password("Password1").is_ok());
        assert!(!validate_password("jean1234").is_ok()); // Pas de majuscule
        assert!(!validate_password("JEAN1234").is_ok()); // Pas de minuscule
        assert!(!validate_password("Jeanabcd").is_ok()); // Pas de chiffre
        assert!(!validate_password("Jean123").is_ok()); // Trop court
    }

    #[test]
    fn test_date_valide() {
        assert!(validate_date_naissance("1996-06-01").is_ok());
        assert!(validate_date_naissance("2000-01-15").is_ok());
        assert!(!validate_date_naissance("2020-01-01").is_ok()); // Trop jeune
        assert!(!validate_date_naissance("1800-01-01").is_ok()); // Trop vieux
        assert!(!validate_date_naissance("1996/06/01").is_ok()); // Mauvais format
    }

    #[test]
    fn test_pays_valide() {
        assert!(validate_pays("RDC").is_ok());
        assert!(validate_pays("CD").is_ok());
        assert!(!validate_pays("RDCA").is_ok());
        assert!(!validate_pays("").is_ok());
    }

    #[test]
    fn test_email_valide() {
        assert!(validate_email("jean@zando.uq").is_ok());
        assert!(validate_email("delis@ufundi.uq").is_ok());
        assert!(!validate_email("jean@gmail.com").is_ok());
    }

    #[test]
    fn test_uq_valide() {
        assert!(validate_uq_number("UQ-732688531").is_ok());
        assert!(validate_uq_number("UQ-00000001").is_ok());
        assert!(!validate_uq_number("UQ-123").is_ok());
        assert!(!validate_uq_number("732688531").is_ok());
    }

    #[test]
    fn test_account_complet() {
        let input = AccountInput {
            prenom: "Jean".to_string(),
            nom: "Destin".to_string(),
            password: "Jean1234".to_string(),
            date_naissance: "1996-06-01".to_string(),
            lieu_naissance: "Bukavu".to_string(),
            pays: "RDC".to_string(),
            telephone: "".to_string(),
        };
        assert!(validate_account_input(&input).is_ok());
    }
}
