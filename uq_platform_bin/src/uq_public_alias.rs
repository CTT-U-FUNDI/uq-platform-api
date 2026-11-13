// ====================
// UQ PUBLIC ALIAS — Pseudo + Nom affichés publiquement
// ====================
//
// RÈGLE ABSOLUE (Q1 / Q7) :
//   Le UQ number n'apparaît JAMAIS côté affichage.
//   Format public : « uq@jean (Jean M.) »
//   Le UQ reste côté DB (sender_uq, recipient_uq, borb_id, prev_hash…).
// ====================

/// Identité publique d'un compte : (pseudo, nom_court)
///   pseudo    : « uq@jean »
///   nom_court : « Jean M. » (prénom + initiale du nom), ou "" si inconnu
pub fn get_public_identity(uq: &str) -> (String, String) {
    let idx = crate::uq_index::UqIndex::shared("uq_index.db");
    let account = idx.get_account(uq);

    let mut pseudo = String::new();
    let mut display_name = String::new();

    if let Some(info) = &account {
        pseudo = info
            .get("uq_pseudo")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
    }

    let db_path = format!("data/users/{}.db", uq.replace("-", "_").to_lowercase());

    // 1. Alias public (priorité, utilisé par l'owner)
    if let Ok(conn) = rusqlite::Connection::open(&db_path) {
        let alias: Result<String, _> = conn.query_row(
            "SELECT display_name FROM uq_aliases WHERE real_uq = ?1 AND active = 1 LIMIT 1",
            rusqlite::params![uq],
            |row| row.get(0),
        );
        if let Ok(name) = alias {
            if !name.is_empty() {
                display_name = name;
            }
        }

        // 2. Sinon : l'identité du compte (table identities)
        if display_name.is_empty() {
            let identite: Result<(String, String), _> = conn.query_row(
                "SELECT COALESCE(prenom,''), COALESCE(nom,'') FROM identities
                 WHERE uq_number = ?1 LIMIT 1",
                rusqlite::params![uq],
                |row| Ok((row.get(0)?, row.get(1)?)),
            );
            if let Ok((prenom, nom)) = identite {
                display_name = format_nom_court(&prenom, &nom);
            }
        }

        // 3. Sinon : my_profile.full_name
        if display_name.is_empty() {
            let profil: Result<String, _> = conn.query_row(
                "SELECT COALESCE(full_name,'') FROM my_profile WHERE id = 1",
                [],
                |row| row.get(0),
            );
            if let Ok(full) = profil {
                if !full.trim().is_empty() {
                    let parts: Vec<&str> = full.split_whitespace().collect();
                    display_name = match parts.len() {
                        0 => String::new(),
                        1 => parts[0].to_string(),
                        _ => format!("{} {}.", parts[0], &parts[parts.len() - 1][..1]),
                    };
                }
            }
        }
    }

    if pseudo.is_empty() {
        pseudo = "Utilisateur".to_string();
    }

    (pseudo, display_name)
}

pub fn get_public_avatar_url(uq: &str) -> String {
    let uq = uq.trim().to_uppercase();
    if !is_valid_uq_number(&uq) {
        return String::new();
    }

    let db_path = format!("data/users/{}.db", uq.replace("-", "_").to_lowercase());
    if !std::path::Path::new(&db_path).is_file() {
        return String::new();
    }

    rusqlite::Connection::open(db_path)
        .and_then(|conn| {
            conn.query_row(
                "SELECT value FROM settings WHERE key = 'avatar_url'",
                [],
                |row| row.get::<_, String>(0),
            )
        })
        .unwrap_or_default()
}

fn is_valid_uq_number(uq: &str) -> bool {
    let Some(number) = uq.strip_prefix("UQ-") else {
        return false;
    };
    !number.is_empty() && number.chars().all(|character| character.is_ascii_digit())
}

/// « Jean Dupont » → « Jean D. »
fn format_nom_court(prenom: &str, nom: &str) -> String {
    let prenom = prenom.trim();
    let nom = nom.trim();
    match (prenom.is_empty(), nom.is_empty()) {
        (false, false) => format!("{} {}.", prenom, nom.chars().next().unwrap_or(' ')),
        (false, true) => prenom.to_string(),
        (true, false) => nom.to_string(),
        (true, true) => String::new(),
    }
}

/// Chaîne d'affichage publique : « uq@jean (Jean M.) » ou « uq@jean »
/// Le nom officiel du compte, tel que l'opérateur mobile money l'affiche à la
/// partie adverse une fois payée. La plateforme ne le montre jamais à cette
/// partie adverse : le recopier sur la feuille de preuve est donc une preuve de
/// connaissance, pas une valeur que le vendeur peut souffler à son acheteur.
pub fn nom_enregistre(uq: &str) -> Option<String> {
    let numero = uq.trim();
    if numero.is_empty() {
        return None;
    }
    let db = format!("data/users/{}.db", numero.replace("-", "_").to_lowercase());
    if !std::path::Path::new(&db).exists() {
        return None;
    }
    let conn = match rusqlite::Connection::open(&db) {
        Ok(conn) => conn,
        Err(_) => return None,
    };
    // Trois façons d'avoir écrit son nom, dans l'ordre de fraîcheur :
    // `settings` est ce que l'éditeur de profil du compte alimente, `my_profile`
    // porte le nom déclaré à la création, `identities` ses deux moitiés.
    let edite: Vec<(String, String)> = conn
        .prepare("SELECT key, COALESCE(value, '') FROM settings WHERE key IN ('full_name', 'prenom', 'nom')")
        .and_then(|mut stmt| {
            stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
                .map(|lignes| lignes.filter_map(|l| l.ok()).collect::<Vec<_>>())
        })
        .unwrap_or_default();
    let joint = |cle: &str| {
        edite
            .iter()
            .find(|(c, _)| c == cle)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };
    let dans_settings = if !joint("full_name").trim().is_empty() {
        joint("full_name")
    } else {
        format!("{} {}", joint("prenom"), joint("nom"))
    };
    let dans_profil: String = conn
        .query_row(
            "SELECT COALESCE(full_name, '') FROM my_profile WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .unwrap_or_default();
    let dans_identite: String = conn
        .query_row(
            "SELECT TRIM(COALESCE(prenom, '') || ' ' || COALESCE(nom, ''))
             FROM identities WHERE UPPER(uq_number) = UPPER(?1) LIMIT 1",
            rusqlite::params![numero],
            |row| row.get(0),
        )
        .unwrap_or_default();

    vec![dans_settings, dans_profil, dans_identite]
        .into_iter()
        .filter_map(|candidat| nom_exploitable(&candidat))
        .next()
}

/// Un nom ne vaut un test de connaissance qu'à deux conditions : au moins deux
/// mots, et rien qui ressemble à un pseudo. Un prénom seul se devine, ne prouve
/// rien, et se trouve déjà affiché publiquement.
fn nom_exploitable(candidat: &str) -> Option<String> {
    let mots: Vec<&str> = candidat
        .split_whitespace()
        .filter(|mot| mot.chars().any(|c| c.is_alphanumeric()))
        .collect();
    if mots.len() < 2 {
        return None;
    }
    let joint = mots.join(" ");
    if joint.contains("@") {
        return None;
    }
    Some(joint)
}

pub fn get_public_display(uq: &str) -> String {
    let (pseudo, name) = get_public_identity(uq);
    if name.is_empty() {
        pseudo
    } else {
        format!("{} ({})", pseudo, name)
    }
}

/// Affiche une contrepartie STOCKÉE en base.
///   - si c'est un UQ number  → converti en « pseudo (Nom) »
///   - sinon (déjà un affichage public) → renvoyé tel quel
/// Évite qu'un UQ ne fuite vers l'interface (règle Q1).
pub fn affichage_public(stocke: &str) -> String {
    let s = stocke.trim();
    if s.is_empty() {
        return String::new();
    }
    let haut = s.to_uppercase();
    if haut.starts_with("UQ-") && s[3..].chars().all(|c| c.is_ascii_digit()) {
        get_public_display(&haut)
    } else {
        s.to_string()
    }
}

/// Résout un destinataire saisi par l'utilisateur :
///   - « UQ-732837813 »  → tel quel
///   - « uq@jean »       → UQ correspondant (via uq_index)
/// Retourne None si introuvable.
pub fn resoudre_destinataire(saisie: &str) -> Option<String> {
    let s = saisie.trim();
    if s.is_empty() {
        return None;
    }

    // Déjà un UQ number
    if s.to_uppercase().starts_with("UQ-") && s[3..].chars().all(|c| c.is_ascii_digit()) {
        return Some(s.to_uppercase());
    }

    // Pseudo (avec ou sans « uq@ »)
    let idx = crate::uq_index::UqIndex::shared("uq_index.db");
    let candidats = [
        s.to_string(),
        format!("uq@{}", s.trim_start_matches("uq@")),
    ];

    for c in candidats.iter() {
        if let Some(acc) = idx.get_account_by_pseudo(c) {
            if let Some(uq) = acc.get("uq_number").and_then(|v| v.as_str()) {
                return Some(uq.to_string());
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nom_court_prend_prenom_et_initiale() {
        assert_eq!(format_nom_court("Jean", "Dupont"), "Jean D.");
        assert_eq!(format_nom_court("Marie", ""), "Marie");
        assert_eq!(format_nom_court("", "Dupont"), "Dupont");
        assert_eq!(format_nom_court("", ""), "");
    }

    #[test]
    fn uq_number_reconnu_sans_passer_par_l_index() {
        assert_eq!(
            resoudre_destinataire("UQ-732837813"),
            Some("UQ-732837813".to_string())
        );
        assert_eq!(
            resoudre_destinataire("uq-732837813"),
            Some("UQ-732837813".to_string())
        );
        assert_eq!(resoudre_destinataire(""), None);
    }

    #[test]
    fn validation_uq_refuse_les_chemins_et_les_pseudos() {
        assert!(is_valid_uq_number("UQ-732837813"));
        assert!(!is_valid_uq_number("../data/users/UQ-1"));
        assert!(!is_valid_uq_number("uq@jean"));
    }
}
