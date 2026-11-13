// ====================
// 🎭 DÉFI P2P — nonce visuel exigé sur la preuve
// ====================
//
// Un reçu recyclé ne peut pas porter ce qui n'existait pas encore au moment où
// il a été photographié. La plateforme tire donc, à la création de la commande,
// un code court et une consigne de signe, montrés aux deux parties. Le vendeur
// peut exiger un nouveau tirage : la preuve précédente devient caduque.
//
// Règles :
//  * le code se recopie depuis une photo, l'alphabet exclut toute ambiguïté ;
//  * le signe s'apprécie à l'œil humain, le moteur ne le regarde pas ;
//  * rien ici ne libère un solde : le défi conditionne l'analyse, jamais la caisse.

use rand::Rng;
use rusqlite::{params, Connection};
use serde_json::json;

/// 32 signes dont aucun ne se confond avec un autre à l'écran (pas de I, O, 0, 1).
const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

/// Assez court pour être recopié sans erreur depuis une photo, assez long pour
/// qu'un reçu déjà en circulation ne le porte pas par hasard.
pub const LONGUEUR_CODE: usize = 4;

/// Consignes de signe. Volontairement tenues : toutes se tiennent d'une main,
/// devant un téléphone tenu de l'autre.
pub const SIGNES: [&str; 6] = [
    "pouce levé",
    "index pointé vers le haut",
    "deux doigts en V",
    "trois doigts levés",
    "main ouverte, paume face à l'écran",
    "poing fermé",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Defi {
    pub code: String,
    pub signe: String,
    pub pose_le: i64,
}

impl Defi {
    pub fn as_json(&self) -> serde_json::Value {
        json!({ "code": self.code, "signe": self.signe, "pose_le": self.pose_le })
    }
}

fn tirer_code() -> String {
    let mut rng = rand::thread_rng();
    let mut code = String::with_capacity(LONGUEUR_CODE);
    for _ in 0..LONGUEUR_CODE {
        let index: usize = rng.gen_range(0..ALPHABET.len());
        code.push(ALPHABET[index] as char);
    }
    code
}

pub fn generer(maintenant: i64) -> Defi {
    let mut rng = rand::thread_rng();
    Defi {
        code: tirer_code(),
        signe: SIGNES[rng.gen_range(0..SIGNES.len())].to_string(),
        pose_le: maintenant,
    }
}

/// Le texte que les deux parties lisent sur la commande.
pub fn consigne(defi: &Defi) -> String {
    format!("Code {}, et à l'écran : {}", defi.code, defi.signe)
}

/// Colonne vide = aucun défi posé encore, on en pose un. Relire ne régénère
/// jamais : le vendeur doit voir le même code que l'acheteur.
pub fn en_cours(conn: &Connection, commande: i64, maintenant: i64) -> Result<Defi, String> {
    let lu = conn.query_row(
        "SELECT COALESCE(defi_code, ''), COALESCE(defi_signe, ''), COALESCE(defi_pose_le, 0)
         FROM p2p_orders WHERE id = ?1",
        params![commande],
        |row| {
            Ok(Defi {
                code: row.get(0)?,
                signe: row.get(1)?,
                pose_le: row.get(2)?,
            })
        },
    );
    match lu {
        Ok(defi) if !defi.code.is_empty() => Ok(defi),
        Ok(_) => poser(conn, commande, &generer(maintenant)),
        Err(e) => Err(format!("Défi P2P : lecture impossible ({})", e)),
    }
}

pub fn poser(conn: &Connection, commande: i64, defi: &Defi) -> Result<Defi, String> {
    conn.execute(
        "UPDATE p2p_orders
         SET defi_code = ?1, defi_signe = ?2, defi_pose_le = ?3, updated_at = ?3
         WHERE id = ?4",
        params![defi.code, defi.signe, defi.pose_le, commande],
    )
    .map_err(|e| format!("Défi P2P : pose impossible ({})", e))?;
    Ok(defi.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn table() -> Connection {
        let conn = Connection::open_in_memory().expect("base memoire");
        conn.execute_batch(
            "CREATE TABLE p2p_orders (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                defi_code TEXT DEFAULT '',
                defi_signe TEXT DEFAULT '',
                defi_pose_le INTEGER DEFAULT 0,
                updated_at INTEGER DEFAULT 0
            );
            INSERT INTO p2p_orders (id) VALUES (7);",
        )
        .expect("migration test");
        conn
    }

    #[test]
    fn code_tire_reste_lisible_et_dans_l_alphabet() {
        let mut vus = std::collections::HashSet::new();
        for _ in 0..200 {
            let defi = generer(1);
            assert_eq!(defi.code.chars().count(), LONGUEUR_CODE);
            for c in defi.code.chars() {
                assert!(ALPHABET.contains(&c as u8), "signe hors alphabet : {}", c);
            }
            assert!(SIGNES.contains(&defi.signe.as_str()));
            vus.insert(defi.code);
        }
        // Un tirage qui répéterait presque tout son espace n'est pas un nonce.
        assert!(vus.len() > 150, "tirages trop répétés : {}", vus.len());
    }

    #[test]
    fn defi_se_pose_puis_se_regener() {
        let conn = table();
        let premier = en_cours(&conn, 7, 11).expect("premier defi");
        assert!(!premier.code.is_empty());
        assert_eq!(premier.pose_le, 11);
        assert_eq!(
            en_cours(&conn, 7, 999).expect("relecture").code,
            premier.code,
            "relire ne doit pas changer le defi"
        );

        let nouveau = poser(&conn, 7, &generer(42)).expect("regenerer");
        assert_eq!(nouveau.code, en_cours(&conn, 7, 0).expect("apres regen").code);
        assert_eq!(en_cours(&conn, 7, 0).expect("apres regen").pose_le, 42);
        assert_eq!(
            conn.query_row::<i64, _, _>(
                "SELECT updated_at FROM p2p_orders WHERE id = 7",
                [],
                |row| row.get(0)
            )
            .expect("horodatage"),
            42
        );
    }

    #[test]
    fn consigne_porte_le_code_et_le_signe() {
        let defi = Defi {
            code: "K7QM".to_string(),
            signe: "pouce levé".to_string(),
            pose_le: 9,
        };
        let texte = consigne(&defi);
        assert!(texte.contains("K7QM"), "{}", texte);
        assert!(texte.contains("pouce"), "{}", texte);
    }
}
