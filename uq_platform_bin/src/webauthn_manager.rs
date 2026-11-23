// ====================
// WEBAUTHN MANAGER — Authentification par Passkey
// Face ID / Empreinte / Windows Hello
// ====================

use chrono::Utc;
use rusqlite::{params, Connection};
use serde_json::json;

// ===== Types =====
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WebAuthnCredential {
    pub credential_id: String,
    pub public_key: Vec<u8>,
    pub sign_count: u32,
    pub device_name: String,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Challenge {
    pub challenge: String,
    pub user_id: String,
    pub action: String,
    pub created_at: i64,
}

// ===== Init table =====
pub fn init_webauthn_table(conn: &Connection) -> Result<(), String> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS webauthn_credentials (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            credential_id TEXT UNIQUE NOT NULL,
            public_key BLOB NOT NULL,
            sign_count INTEGER DEFAULT 0,
            device_name TEXT DEFAULT 'Appareil inconnu',
            created_at INTEGER NOT NULL,
            last_used_at INTEGER
        )",
        [],
    )
    .map_err(|e| e.to_string())?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS webauthn_challenges (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            challenge TEXT UNIQUE NOT NULL,
            user_id TEXT NOT NULL,
            action TEXT NOT NULL,
            created_at INTEGER NOT NULL
        )",
        [],
    )
    .map_err(|e| e.to_string())?;

    Ok(())
}

// ===== Generer un challenge =====
//
// Le challenge est un vrai base64url de 32 octets aleatoires : le navigateur le
// remet tel quel dans clientDataJSON.challenge, donc le serveur peut le
// recomparer octet pour octet. Un hexadecimal ne le permettait pas : le front
// le decodait comme du base64url et envoyait des octets que le serveur ne
// savait pas retrouver.
pub fn generate_challenge() -> String {
    let aleatoire: [u8; 32] = rand::random();
    crate::webauthn_verify::encodage_base64url(&aleatoire)
}

// ===== Stocker un challenge =====
pub fn store_challenge(
    conn: &Connection,
    challenge: &str,
    user_id: &str,
    action: &str,
) -> Result<(), String> {
    // Supprimer les anciens challenges (> 5 min)
    let cutoff = Utc::now().timestamp() - 300;
    conn.execute(
        "DELETE FROM webauthn_challenges WHERE created_at < ?1",
        params![cutoff],
    )
    .map_err(|e| e.to_string())?;

    conn.execute(
        "INSERT INTO webauthn_challenges (challenge, user_id, action, created_at)
         VALUES (?1, ?2, ?3, ?4)",
        params![challenge, user_id, action, Utc::now().timestamp()],
    )
    .map_err(|e| e.to_string())?;

    Ok(())
}

// ===== Verifier un challenge =====
pub fn verify_challenge(
    conn: &Connection,
    challenge: &str,
) -> Result<Option<(String, String)>, String> {
    let cutoff = Utc::now().timestamp() - 300;
    let result: Result<(String, String), _> = conn.query_row(
        "SELECT user_id, action FROM webauthn_challenges
         WHERE challenge = ?1 AND created_at > ?2",
        params![challenge, cutoff],
        |row| Ok((row.get(0)?, row.get(1)?)),
    );

    match result {
        Ok(r) => {
            // Supprimer le challenge utilise (anti-replay)
            conn.execute(
                "DELETE FROM webauthn_challenges WHERE challenge = ?1",
                params![challenge],
            )
            .map_err(|e| e.to_string())?;
            Ok(Some(r))
        }
        Err(_) => Ok(None),
    }
}

// ===== Enregistrer une credential =====
pub fn save_credential(
    conn: &Connection,
    credential_id: &str,
    public_key: &[u8],
    device_name: &str,
) -> Result<(), String> {
    conn.execute(
        "INSERT OR REPLACE INTO webauthn_credentials
         (credential_id, public_key, sign_count, device_name, created_at)
         VALUES (?1, ?2, 0, ?3, ?4)",
        params![
            credential_id,
            public_key,
            device_name,
            Utc::now().timestamp()
        ],
    )
    .map_err(|e| e.to_string())?;

    Ok(())
}

/// Lire la cle publique SEC1 et le compteur d'une credential.
///
/// La table etant dans la base du compte, retrouver la credential prouve deja
/// qu'elle appartient a ce numero UQ.
pub fn lire_credential(
    conn: &Connection,
    credential_id: &str,
) -> Result<Option<(Vec<u8>, u32)>, String> {
    let resultat: Result<(Vec<u8>, i64), _> = conn.query_row(
        "SELECT public_key, sign_count FROM webauthn_credentials WHERE credential_id = ?1",
        params![credential_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    );
    match resultat {
        Ok((cle, compteur)) => Ok(Some((cle, compteur.max(0) as u32))),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// Meme chose en listant tout (pour allowCredentials du login/begin).
pub fn credentials_autorisees(conn: &Connection) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("SELECT credential_id FROM webauthn_credentials")
        .map_err(|e| e.to_string())?;
    let lignes = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    let mut ids = Vec::new();
    for ligne in lignes {
        if let Ok(id) = ligne {
            ids.push(id);
        }
    }
    Ok(ids)
}

/// Met a jour le compteur de signature apres une assertion verifiee.
///
/// Le compteur ne monte jamais a l'envers : c'est le signal de clonage de
/// credential (le meme passkey physiquement duplique sur deux enclaves).
pub fn marquer_usage(conn: &Connection, credential_id: &str, compteur: u32) -> Result<(), String> {
    conn.execute(
        "UPDATE webauthn_credentials
         SET sign_count = ?1, last_used_at = ?2
         WHERE credential_id = ?3 AND sign_count <= ?1",
        params![compteur as i64, Utc::now().timestamp(), credential_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ===== Lister les credentials d'un utilisateur =====
pub fn list_credentials(conn: &Connection) -> Result<Vec<serde_json::Value>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT credential_id, device_name, created_at, last_used_at, sign_count
         FROM webauthn_credentials ORDER BY created_at DESC",
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map([], |row| {
            Ok(json!({
                "credential_id": row.get::<_, String>(0)?,
                "device_name": row.get::<_, String>(1)?,
                "created_at": row.get::<_, i64>(2)?,
                "last_used_at": row.get::<_, Option<i64>>(3)?,
                "sign_count": row.get::<_, i64>(4)?,
            }))
        })
        .map_err(|e| e.to_string())?;

    let mut result = Vec::new();
    for row in rows {
        if let Ok(v) = row {
            result.push(v);
        }
    }

    Ok(result)
}

// ===== Compter les credentials =====
pub fn count_credentials(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM webauthn_credentials", [], |row| {
        row.get(0)
    })
    .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_challenge_generation() {
        let c1 = generate_challenge();
        let c2 = generate_challenge();
        assert_ne!(c1, c2);
        // 32 octets aleatoires en base64url sans padding
        assert_eq!(c1.len(), 43, "challenge: {}", c1);
        assert!(
            c1.chars()
                .all(|car| car.is_ascii_alphanumeric() || car == '-' || car == '_'),
            "challenge non base64url: {}",
            c1
        );
        // le round-trip doit retrouver les 32 octets : c'est ce que compare
        // la verification d'assertion
        assert_eq!(
            crate::webauthn_verify::decodage_base64url(&c1)
                .unwrap()
                .len(),
            32
        );
    }

    #[test]
    fn counter_ne_regrade_pas() {
        let conn = Connection::open_in_memory().unwrap();
        init_webauthn_table(&conn).unwrap();
        save_credential(&conn, "cred-1", &[0x04, 0x01], "PC").unwrap();
        marquer_usage(&conn, "cred-1", 12).unwrap();
        assert_eq!(
            lire_credential(&conn, "cred-1").unwrap(),
            Some((vec![0x04u8, 0x01], 12))
        );
        // un compteur plus petit est ignore (la clause sign_count <= ?1)
        marquer_usage(&conn, "cred-1", 5).unwrap();
        assert_eq!(lire_credential(&conn, "cred-1").unwrap().unwrap().1, 12);
        assert_eq!(
            credentials_autorisees(&conn).unwrap(),
            vec!["cred-1".to_string()]
        );
        assert_eq!(lire_credential(&conn, "inconnue").unwrap(), None);
    }

    #[test]
    fn challenge_est_a_usage_unique() {
        let conn = Connection::open_in_memory().unwrap();
        init_webauthn_table(&conn).unwrap();
        let c = generate_challenge();
        store_challenge(&conn, &c, "UQ-00000001", "login").unwrap();
        let premier = verify_challenge(&conn, &c).unwrap();
        assert_eq!(
            premier,
            Some(("UQ-00000001".to_string(), "login".to_string()))
        );
        // rejoue : le challenge a ete supprime
        assert_eq!(verify_challenge(&conn, &c).unwrap(), None);
    }
}
