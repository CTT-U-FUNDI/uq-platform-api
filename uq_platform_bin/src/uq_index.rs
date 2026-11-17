// ====================
// UQ INDEX — Base de verification P2P
// Contient SEULEMENT les preuves (pas de donnees privees)
// ====================

use rusqlite::{Connection, params};
use std::sync::{Arc, Mutex, OnceLock};
use serde_json::{json, Value};
use sha2::{Sha256, Digest};

static INDEX_INSTANCE: OnceLock<Arc<UqIndex>> = OnceLock::new();

pub struct UqIndex {
    conn: Arc<Mutex<Connection>>,
    db_path: String,
}

impl UqIndex {
    /// Singleton pour l'index
    pub fn shared(db_path: &str) -> Arc<UqIndex> {
        INDEX_INSTANCE.get_or_init(|| {
            Arc::new(UqIndex::new(db_path))
        }).clone()
    }

    /// Cree ou ouvre l'index
    pub fn new(db_path: &str) -> Self {
        let conn = Connection::open(db_path).expect("Failed to open uq_index.db");
        let idx = Self {
            conn: Arc::new(Mutex::new(conn)),
            db_path: db_path.to_string(),
        };
        idx.init_tables().expect("Failed to init tables");
        idx
    }

    /// Cree les tables si elles n'existent pas
    fn init_tables(&self) -> Result<(), rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(r#"
            CREATE TABLE IF NOT EXISTS uq_index (
                uq_number       TEXT PRIMARY KEY,
                uq_email        TEXT UNIQUE NOT NULL,
                uq_pseudo       TEXT UNIQUE NOT NULL,
                hash_identity   TEXT NOT NULL,
                hash_password   TEXT NOT NULL,
                salt            TEXT NOT NULL,
                public_key      BLOB NOT NULL,
                is_owner        INTEGER DEFAULT 0,
                created_at      INTEGER NOT NULL,
                db_path         TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_email ON uq_index(uq_email);
            CREATE INDEX IF NOT EXISTS idx_pseudo ON uq_index(uq_pseudo);"#)?;

        conn.execute_batch(r#"
            CREATE TABLE IF NOT EXISTS chat_groups (
                group_id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                type TEXT NOT NULL CHECK (type IN ('group', 'channel', 'system')),
                visibility TEXT NOT NULL CHECK (visibility IN ('public', 'private')),
                description TEXT NOT NULL DEFAULT '',
                created_by TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS chat_group_members (
                group_id TEXT NOT NULL,
                member_id TEXT NOT NULL,
                role TEXT NOT NULL DEFAULT 'member',
                joined_at INTEGER NOT NULL,
                PRIMARY KEY (group_id, member_id),
                FOREIGN KEY (group_id) REFERENCES chat_groups(group_id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_chat_group_members_user
                ON chat_group_members(member_id, group_id);
            CREATE TABLE IF NOT EXISTS chat_group_messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                group_id TEXT NOT NULL,
                sender_id TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                FOREIGN KEY (group_id) REFERENCES chat_groups(group_id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_chat_group_messages_group
                ON chat_group_messages(group_id, id);
        "#)?;

        // MIGRATION PROFIL COMPLET (12 Oct 2026)
        let migrations_profil = [
            "ALTER TABLE uq_index ADD COLUMN sexe TEXT DEFAULT ''",
            "ALTER TABLE uq_index ADD COLUMN ville_provenance TEXT DEFAULT ''",
            "ALTER TABLE uq_index ADD COLUMN ville_actuelle TEXT DEFAULT ''",
            "ALTER TABLE uq_index ADD COLUMN statut_marital TEXT DEFAULT ''",
            "ALTER TABLE uq_index ADD COLUMN photo_url TEXT DEFAULT ''",
            "ALTER TABLE uq_index ADD COLUMN description TEXT DEFAULT ''",
            "ALTER TABLE uq_index ADD COLUMN emploi TEXT DEFAULT ''",
            "ALTER TABLE uq_index ADD COLUMN mobile_money_number TEXT DEFAULT ''",
            "ALTER TABLE uq_index ADD COLUMN mobile_money_operator TEXT DEFAULT ''",
            "ALTER TABLE uq_index ADD COLUMN date_naissance INTEGER DEFAULT 0",
            "ALTER TABLE uq_index ADD COLUMN kyc_verified INTEGER DEFAULT 0",
            // Contacts de recuperation (page de creation, ecran 5).
            "ALTER TABLE uq_index ADD COLUMN recovery_email TEXT DEFAULT ''",
            "ALTER TABLE uq_index ADD COLUMN recovery_phone TEXT DEFAULT ''",
            "ALTER TABLE uq_index ADD COLUMN phone_verified INTEGER DEFAULT 0",
            "ALTER TABLE uq_index ADD COLUMN email_verified INTEGER DEFAULT 0",
        ];
        for sql in migrations_profil.iter() {
            let _ = conn.execute(sql, []);
        }
        eprintln!("[OK] Migration profil : {} colonnes ajoutees (ou deja presentes)", migrations_profil.len());

        Ok(())
    }

    /// Met a jour les champs profil a la creation (sexe, villes, statut)
    pub fn update_profile_fields(
        &self,
        uq_number: &str,
        sexe: &str,
        ville_provenance: &str,
        ville_actuelle: &str,
        statut_marital: &str,
    ) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE uq_index SET 
                sexe = COALESCE(NULLIF(?2, ''), sexe),
                ville_provenance = COALESCE(NULLIF(?3, ''), ville_provenance),
                ville_actuelle = COALESCE(NULLIF(?4, ''), ville_actuelle),
                statut_marital = COALESCE(NULLIF(?5, ''), statut_marital)
             WHERE uq_number = ?1",
            params![uq_number, sexe, ville_provenance, ville_actuelle, statut_marital],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Contacts enregistres a l'inscription : profession (colonne emploi) et
    /// moyens de recuperation. Un vide laisse la valeur en place.
    pub fn update_creation_profil(
        &self,
        uq_number: &str,
        emploi: &str,
        recovery_email: &str,
        recovery_phone: &str,
        phone_verified: bool,
        email_verified: bool,
    ) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE uq_index SET 
                emploi = COALESCE(NULLIF(?2, ''), emploi),
                recovery_email = COALESCE(NULLIF(?3, ''), recovery_email),
                recovery_phone = COALESCE(NULLIF(?4, ''), recovery_phone),
                phone_verified = CASE WHEN ?4 <> '' THEN ?5 ELSE phone_verified END,
                email_verified = CASE WHEN ?3 <> '' THEN ?6 ELSE email_verified END
             WHERE uq_number = ?1",
            params![
                uq_number,
                emploi,
                recovery_email,
                recovery_phone,
                phone_verified as i64,
                email_verified as i64
            ],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Lecture des contacts de recuperation d'un compte.
    pub fn get_recovery_contacts(&self, uq_number: &str) -> Option<serde_json::Value> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT COALESCE(recovery_email,''), COALESCE(recovery_phone,''),
                    COALESCE(phone_verified,0), COALESCE(email_verified,0)
             FROM uq_index WHERE uq_number = ?1",
            rusqlite::params![uq_number],
            |row| {
                Ok(serde_json::json!({
                    "recovery_email": row.get::<_, String>(0)?,
                    "recovery_phone": row.get::<_, String>(1)?,
                    "phone_verified": row.get::<_, i64>(2)? == 1,
                    "email_verified": row.get::<_, i64>(3)? == 1,
                }))
            },
        ).ok()
    }

    /// Mise a jour depuis les reglages du profil (mail externe, telephone).
    pub fn set_recovery_contacts(
        &self,
        uq_number: &str,
        recovery_email: Option<&str>,
        recovery_phone: Option<&str>,
        phone_verified: Option<bool>,
        email_verified: Option<bool>,
    ) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        if let Some(v) = recovery_email {
            conn.execute(
                "UPDATE uq_index SET recovery_email = ?1 WHERE uq_number = ?2",
                params![v, uq_number],
            ).map_err(|e| e.to_string())?;
        }
        if let Some(v) = recovery_phone {
            conn.execute(
                "UPDATE uq_index SET recovery_phone = ?1 WHERE uq_number = ?2",
                params![v, uq_number],
            ).map_err(|e| e.to_string())?;
        }
        if let Some(v) = phone_verified {
            conn.execute(
                "UPDATE uq_index SET phone_verified = ?1 WHERE uq_number = ?2",
                params![v as i64, uq_number],
            ).map_err(|e| e.to_string())?;
        }
        if let Some(v) = email_verified {
            conn.execute(
                "UPDATE uq_index SET email_verified = ?1 WHERE uq_number = ?2",
                params![v as i64, uq_number],
            ).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Met a jour les champs modifiables du profil (depuis ProfilePage)
    pub fn update_profile_full(
        &self,
        uq_number: &str,
        ville_provenance: Option<&str>,
        ville_actuelle: Option<&str>,
        statut_marital: Option<&str>,
        photo_url: Option<&str>,
        description: Option<&str>,
        emploi: Option<&str>,
        mobile_money_number: Option<&str>,
        mobile_money_operator: Option<&str>,
    ) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        if let Some(v) = ville_provenance {
            conn.execute("UPDATE uq_index SET ville_provenance = ?1 WHERE uq_number = ?2", params![v, uq_number]).ok();
        }
        if let Some(v) = ville_actuelle {
            conn.execute("UPDATE uq_index SET ville_actuelle = ?1 WHERE uq_number = ?2", params![v, uq_number]).ok();
        }
        if let Some(v) = statut_marital {
            conn.execute("UPDATE uq_index SET statut_marital = ?1 WHERE uq_number = ?2", params![v, uq_number]).ok();
        }
        if let Some(v) = photo_url {
            conn.execute("UPDATE uq_index SET photo_url = ?1 WHERE uq_number = ?2", params![v, uq_number]).ok();
        }
        if let Some(v) = description {
            conn.execute("UPDATE uq_index SET description = ?1 WHERE uq_number = ?2", params![v, uq_number]).ok();
        }
        if let Some(v) = emploi {
            conn.execute("UPDATE uq_index SET emploi = ?1 WHERE uq_number = ?2", params![v, uq_number]).ok();
        }
        if let Some(v) = mobile_money_number {
            conn.execute("UPDATE uq_index SET mobile_money_number = ?1 WHERE uq_number = ?2", params![v, uq_number]).ok();
        }
        if let Some(v) = mobile_money_operator {
            conn.execute("UPDATE uq_index SET mobile_money_operator = ?1 WHERE uq_number = ?2", params![v, uq_number]).ok();
        }
        Ok(())
    }

    /// Hash SHA-256 avec iterations (1000) avec iterations (1000)
    pub fn hash_password(password: &str, salt: &str) -> String {
        let mut result = format!("{}{}", password, salt).into_bytes();
        for _ in 0..1000 {
            let mut hasher = Sha256::new();
            hasher.update(&result);
            result = hasher.finalize().to_vec();
        }
        hex::encode(result)
    }

    /// Hash identite (nom+prenom+pays+position)
    pub fn hash_identity(nom: &str, prenom: &str, pays: &str, position: u64) -> String {
        let data = format!("{}|{}|{}|{}", nom, prenom, pays, position);
        let mut hasher = Sha256::new();
        hasher.update(data.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// Ajoute un compte dans l'index
    pub fn insert_account(
        &self,
        uq_number: &str,
        uq_email: &str,
        uq_pseudo: &str,
        hash_identity: &str,
        hash_password: &str,
        salt: &str,
        public_key: &[u8],
        is_owner: bool,
        db_path: &str,
    ) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();

        // Normaliser email et pseudo pour les collisions
        let normalized_email = uq_email.to_lowercase()
            .replace(".", "")
            .replace("-", "")
            .replace("_", "");
        let normalized_pseudo = uq_pseudo.to_lowercase()
            .replace(".", "")
            .replace("-", "")
            .replace("_", "")
            .replace("uq@", "");

        conn.execute(
            "INSERT OR REPLACE INTO uq_index
             (uq_number, uq_email, uq_pseudo, hash_identity, hash_password, salt, public_key, is_owner, created_at, db_path, normalized_email, normalized_pseudo)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                uq_number,
                uq_email,
                uq_pseudo,
                hash_identity,
                hash_password,
                salt,
                public_key,
                if is_owner { 1 } else { 0 },
                chrono::Utc::now().timestamp(),
                db_path,
                normalized_email,
                normalized_pseudo,
            ],

        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Verifie les credentials d'un compte
    /// Retourne Some(json) si OK, None sinon
    pub fn verify_credentials(
        &self,
        uq_number: &str,
        uq_email: &str,
        password: &str,
    ) -> Option<Value> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT uq_number, uq_email, uq_pseudo, hash_password, salt, is_owner, db_path
             FROM uq_index WHERE uq_number = ?1 AND uq_email = ?2"
        ).ok()?;

        let row = stmt.query_row(params![uq_number, uq_email], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, String>(6)?,
            ))
        }).ok()?;

        let (number, email, pseudo, stored_hash, salt, is_owner, db_path) = row;

        // Recalculer le hash du mot de passe fourni
        let computed = Self::hash_password(password, &salt);

        if computed != stored_hash {
            return None;
        }

        Some(json!({
            "uq_number": number,
            "uq_email": email,
            "uq_pseudo": pseudo,
            "is_owner": is_owner == 1,
            "db_path": db_path,
            "kyc": crate::kyc::cadenas::est_deverrouille(&number),
            "next_step": if is_owner == 1 { "unlock" } else { "2fa" }
        }))
    }

    /// Vérifie les identifiants avec SEULEMENT l'email UQ + password.
    /// Retourne le même JSON que verify_credentials (uq_number résolu en interne).
    /// Utilisé pour le login « email seul » : l'utilisateur ne tape plus son UQ.
    pub fn verify_credentials_by_email(
        &self,
        uq_email: &str,
        password: &str,
    ) -> Option<Value> {
        let conn = self.conn.lock().unwrap();

        // Normaliser l'email comme get_account_by_email
        let normalized = uq_email.to_lowercase()
            .replace(".", "")
            .replace("-", "")
            .replace("_", "");

        let mut stmt = conn.prepare(
            "SELECT uq_number, uq_email, uq_pseudo, hash_password, salt, is_owner, db_path
             FROM uq_index
             WHERE normalized_email = ?1 OR LOWER(uq_email) = ?2
             LIMIT 1"
        ).ok()?;

        let row = stmt.query_row(
            rusqlite::params![normalized, uq_email.to_lowercase()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, String>(6)?,
                ))
            },
        ).ok()?;

        let (number, email, pseudo, stored_hash, salt, is_owner, db_path) = row;

        let computed = Self::hash_password(password, &salt);
        if computed != stored_hash {
            return None;
        }

        Some(json!({
            "uq_number": number,
            "uq_email": email,
            "uq_pseudo": pseudo,
            "is_owner": is_owner == 1,
            "db_path": db_path,
            "kyc": crate::kyc::cadenas::est_deverrouille(&number),
            "next_step": if is_owner == 1 { "unlock" } else { "2fa" }
        }))
    }

    /// Retrouve le uq_number à partir d'un email UQ (pour le step1 du login).
    pub fn get_uq_by_email(&self, uq_email: &str) -> Option<String> {
        self.get_account_by_email(uq_email)
            .and_then(|v| v.get("uq_number").and_then(|n| n.as_str()).map(|s| s.to_string()))
    }

    /// Résout un localpart d'adresse mail réelle (ex: "jean" de jean@mail.domaine)
    /// vers le uq_number du compte dont uq_email = "jean@zando.uq".
    /// Utilisé par /api/mail/in pour router le courrier entrant.
    pub fn get_uq_by_email_localpart(&self, localpart: &str) -> Option<String> {
        let conn = self.conn.lock().ok()?;
        let local = localpart.to_lowercase();
        // uq_email = "localpart@domaine_uq" — on matche le localpart exact
        conn.query_row(
            "SELECT uq_number FROM uq_index
             WHERE LOWER(SUBSTR(uq_email, 1, INSTR(uq_email, '@') - 1)) = ?1
             LIMIT 1",
            params![local],
            |row| row.get::<_, String>(0),
        ).ok()
    }

    /// Recupere un compte par uq_number
    /// Récupère le code promo d'un utilisateur
    pub fn get_code_promo(&self, uq_number: &str) -> Option<String> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT code FROM code_promos WHERE uq_number = ?1",
            params![uq_number],
            |row| row.get::<_, String>(0),
        ).ok()
    }

    /// Retrouve le parrain (uq_number) à partir d'un code promo.
    /// Utilisé à la création de compte pour enregistrer le parrainage.
    pub fn get_uq_by_code_promo(&self, code: &str) -> Option<String> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT uq_number FROM code_promos WHERE UPPER(code) = UPPER(?1)",
            params![code.trim()],
            |row| row.get::<_, String>(0),
        ).ok()
    }

    /// Insère un code promo dans la table code_promos
    pub fn insert_code_promo(&self, code: &str, uq_number: &str, prenom: &str, position: u64) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT OR REPLACE INTO code_promos (code, uq_number, prenom, position, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![code, uq_number, prenom, position as i64, now],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn get_account(&self, uq_number: &str) -> Option<Value> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT uq_number, uq_email, uq_pseudo, is_owner, db_path FROM uq_index WHERE uq_number = ?1"
        ).ok()?;

        stmt.query_row(params![uq_number], |row| {
            Ok(json!({
                "uq_number": row.get::<_, String>(0)?,
                "uq_email": row.get::<_, String>(1)?,
                "uq_pseudo": row.get::<_, String>(2)?,
                "is_owner": row.get::<_, i64>(3)? == 1,
                "db_path": row.get::<_, String>(4)?
            }))
        }).ok()
    }


    /// Clé publique Ed25519 du compte (pour vérif signature d'autorisation).
    /// Email associe a un numero UQ — resolution interne pour le login
    /// « UQ + mot de passe » (l'utilisateur ne saisit jamais son email).
    pub fn get_email(&self, uq_number: &str) -> Option<String> {
        let conn = self.conn.lock().ok()?;
        conn.query_row(
            "SELECT uq_email FROM uq_index WHERE uq_number = ?1",
            params![uq_number],
            |row| row.get::<_, String>(0),
        ).ok()
    }

    pub fn get_public_key(&self, uq_number: &str) -> Option<Vec<u8>> {
        let conn = self.conn.lock().ok()?;
        conn.query_row(
            "SELECT public_key FROM uq_index WHERE uq_number = ?1",
            params![uq_number],
            |row| row.get::<_, Vec<u8>>(0),
        ).ok()
    }

    /// Migration : Ajouter les colonnes normalized_email et normalized_pseudo
    /// a la table uq_index si elles n'existent pas
    pub fn migrate_uq_index(&self) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;

        // Verifier si les colonnes existent deja
        let columns: Vec<String> = conn
            .prepare("PRAGMA table_info(uq_index)")
            .map_err(|e| e.to_string())?
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();

        // Ajouter normalized_email si absente
        if !columns.contains(&"normalized_email".to_string()) {
            conn.execute(
                "ALTER TABLE uq_index ADD COLUMN normalized_email TEXT DEFAULT ''",
                [],
            ).map_err(|e| e.to_string())?;
            eprintln!("✅ Colonne normalized_email ajoutee");
        }

        // Ajouter normalized_pseudo si absente
        if !columns.contains(&"normalized_pseudo".to_string()) {
            conn.execute(
                "ALTER TABLE uq_index ADD COLUMN normalized_pseudo TEXT DEFAULT ''",
                [],
            ).map_err(|e| e.to_string())?;
            eprintln!("✅ Colonne normalized_pseudo ajoutee");
        }

        // Remplir les colonnes vides avec la version normalisee
        conn.execute(
            "UPDATE uq_index
             SET normalized_email = LOWER(REPLACE(REPLACE(REPLACE(uq_email, '.', ''), '-', ''), '_', ''))
             WHERE normalized_email = '' OR normalized_email IS NULL",
            [],
        ).map_err(|e| e.to_string())?;

        conn.execute(
            "UPDATE uq_index
             SET normalized_pseudo = LOWER(REPLACE(REPLACE(REPLACE(uq_pseudo, '.', ''), '-', ''), '_', ''))
             WHERE normalized_pseudo = '' OR normalized_pseudo IS NULL",
            [],
        ).map_err(|e| e.to_string())?;

        Ok(())
    }
    /// Compte total d'utilisateurs
    // ====================
    // ADDITIONNEUR INFINI — next_position()
    // Jamais de soustraction. Persiste. Ne recule jamais.
    // ====================
    pub fn next_position(&self) -> u64 {
        let conn = self.conn.lock().unwrap();
        let now = chrono::Utc::now().timestamp();

        // Lire la position actuelle
        let current: i64 = conn.query_row(
            "SELECT COALESCE(last_position, 0) FROM uq_counter WHERE id = 1",
            [],
            |row| row.get(0),
        ).unwrap_or(0);

        // ADDITIONNER (jamais soustraire)
        let new_position = (current + 1) as u64;

        // Persister
        let _ = conn.execute(
            "UPDATE uq_counter SET last_position = ?1, updated_at = ?2 WHERE id = 1",
            params![new_position as i64, now],
        );

        new_position
    }

    /// Lire la position actuelle sans l'incrémenter
    pub fn current_position(&self) -> u64 {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT COALESCE(last_position, 0) FROM uq_counter WHERE id = 1",
            [],
            |row| row.get::<_, i64>(0),
        ).map(|v| v as u64).unwrap_or(0)
    }

    // ====================
    // VÉRIFICATEUR DE PATTERNS — pattern_exists()
    // ====================
    pub fn pattern_exists(&self, pattern_type: &str, pattern_value: &str) -> bool {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM generated_patterns 
             WHERE pattern_type = ?1 AND pattern_value = ?2",
            params![pattern_type, pattern_value],
            |row| row.get(0),
        ).unwrap_or(0);
        count > 0
    }

    // ====================
    // ENREGISTREUR DE PATTERNS — record_pattern()
    // ====================
    pub fn record_pattern(
        &self,
        pattern_type: &str,
        pattern_value: &str,
        position: u64,
        uq_number: &str,
    ) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT OR IGNORE INTO generated_patterns 
             (pattern_type, pattern_value, position, uq_number, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![pattern_type, pattern_value, position as i64, uq_number, now],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn count(&self) -> u64 {
        let conn = self.conn.lock().unwrap();
        conn.query_row("SELECT COUNT(*) FROM uq_index", [], |r| r.get(0)).unwrap_or(0)
    }

    /// Stats de l'index
    pub fn stats(&self) -> Value {
        json!({
            "total_users": self.count(),
            "db_path": self.db_path,
            "storage": "uq_index.db"
        })
    }

    pub fn create_chat_group(
        &self,
        name: &str,
        description: &str,
        group_type: &str,
        visibility: &str,
        owner_id: &str,
    ) -> Result<Value, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let created_at = chrono::Utc::now().timestamp();
        let group_id = format!(
            "chat_{}_{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
            group_type
        );
        let transaction = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        transaction.execute(
            "INSERT INTO chat_groups
             (group_id, name, type, visibility, description, created_by, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![group_id, name, group_type, visibility, description, owner_id, created_at],
        ).map_err(|e| e.to_string())?;
        transaction.execute(
            "INSERT INTO chat_group_members (group_id, member_id, role, joined_at)
             VALUES (?1, ?2, 'admin', ?3)",
            params![group_id, owner_id, created_at],
        ).map_err(|e| e.to_string())?;
        transaction.commit().map_err(|e| e.to_string())?;
        Ok(json!({
            "group_id": group_id,
            "name": name,
            "type": group_type,
            "visibility": visibility,
            "description": description,
            "created_by": owner_id,
            "created_at": created_at,
            "member_count": 1,
            "role": "admin",
            "is_member": true
        }))
    }

    pub fn list_chat_groups(&self, user_id: &str) -> Result<Vec<Value>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut statement = conn.prepare(
            "SELECT g.group_id, g.name, g.type, g.visibility, g.description, g.created_by,
                    g.created_at, COUNT(m.member_id), mine.role
             FROM chat_groups g
             LEFT JOIN chat_group_members m ON m.group_id = g.group_id
             LEFT JOIN chat_group_members mine
                    ON mine.group_id = g.group_id AND mine.member_id = ?1
             WHERE g.visibility = 'public' OR mine.member_id IS NOT NULL
             GROUP BY g.group_id
             ORDER BY g.type = 'system' DESC, g.created_at DESC",
        ).map_err(|e| e.to_string())?;
        let rows = statement.query_map(params![user_id], |row| {
            let role: Option<String> = row.get(8)?;
            let is_member = role.is_some();
            Ok(json!({
                "group_id": row.get::<_, String>(0)?,
                "name": row.get::<_, String>(1)?,
                "type": row.get::<_, String>(2)?,
                "visibility": row.get::<_, String>(3)?,
                "description": row.get::<_, String>(4)?,
                "created_by": row.get::<_, String>(5)?,
                "created_at": row.get::<_, i64>(6)?,
                "member_count": row.get::<_, i64>(7)?,
                "role": role,
                "is_member": is_member
            }))
        }).map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
    }

    pub fn join_chat_group(&self, group_id: &str, user_id: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let visibility: String = conn.query_row(
            "SELECT visibility FROM chat_groups WHERE group_id = ?1",
            params![group_id],
            |row| row.get(0),
        ).map_err(|e| e.to_string())?;
        if visibility != "public" {
            return Err("Ce canal est privé et nécessite une invitation.".to_string());
        }
        conn.execute(
            "INSERT OR IGNORE INTO chat_group_members (group_id, member_id, role, joined_at)
             VALUES (?1, ?2, 'member', ?3)",
            params![group_id, user_id, chrono::Utc::now().timestamp()],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn invite_chat_group_member(
        &self,
        group_id: &str,
        admin_id: &str,
        member_id: &str,
    ) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let role: String = conn.query_row(
            "SELECT role FROM chat_group_members
             WHERE group_id = ?1 AND member_id = ?2",
            params![group_id, admin_id],
            |row| row.get(0),
        ).map_err(|e| e.to_string())?;
        if role != "admin" {
            return Err("Seuls les administrateurs peuvent inviter des membres.".to_string());
        }
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM uq_index WHERE uq_number = ?1)",
            params![member_id],
            |row| row.get(0),
        ).map_err(|e| e.to_string())?;
        if !exists {
            return Err("Le membre UQ demandé n’existe pas.".to_string());
        }
        conn.execute(
            "INSERT OR IGNORE INTO chat_group_members (group_id, member_id, role, joined_at)
             VALUES (?1, ?2, 'member', ?3)",
            params![group_id, member_id, chrono::Utc::now().timestamp()],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn leave_chat_group(&self, group_id: &str, user_id: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let role: String = conn.query_row(
            "SELECT role FROM chat_group_members
             WHERE group_id = ?1 AND member_id = ?2",
            params![group_id, user_id],
            |row| row.get(0),
        ).map_err(|e| e.to_string())?;
        if role == "admin" {
            let admins: i64 = conn.query_row(
                "SELECT COUNT(*) FROM chat_group_members
                 WHERE group_id = ?1 AND role = 'admin'",
                params![group_id],
                |row| row.get(0),
            ).map_err(|e| e.to_string())?;
            if admins <= 1 {
                return Err("Nommez un autre administrateur avant de quitter.".to_string());
            }
        }
        conn.execute(
            "DELETE FROM chat_group_members WHERE group_id = ?1 AND member_id = ?2",
            params![group_id, user_id],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn delete_chat_group(&self, group_id: &str, user_id: &str) -> Result<(), String> {
        let mut conn = self.conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let role: String = tx.query_row(
            "SELECT role FROM chat_group_members
             WHERE group_id = ?1 AND member_id = ?2",
            params![group_id, user_id],
            |row| row.get(0),
        ).map_err(|e| e.to_string())?;
        if role != "admin" {
            return Err("Seuls les administrateurs peuvent supprimer ce groupe.".to_string());
        }
        tx.execute("DELETE FROM chat_group_messages WHERE group_id = ?1", params![group_id])
            .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM chat_group_members WHERE group_id = ?1", params![group_id])
            .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM chat_groups WHERE group_id = ?1", params![group_id])
            .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    }

    pub fn chat_group_messages(
        &self,
        group_id: &str,
        user_id: &str,
    ) -> Result<Vec<Value>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let member: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM chat_group_members
             WHERE group_id = ?1 AND member_id = ?2)",
            params![group_id, user_id],
            |row| row.get(0),
        ).map_err(|e| e.to_string())?;
        if !member {
            return Err("Rejoignez le canal pour consulter ses messages.".to_string());
        }
        // Messages de groupe ephemeres : purge au-dela de 72 h.
        let cutoff = chrono::Utc::now().timestamp() - 72 * 3600;
        let _ = conn.execute(
            "DELETE FROM chat_group_messages WHERE created_at < ?1",
            params![cutoff],
        );
        let mut statement = conn.prepare(
            "SELECT id, sender_id, content, created_at
             FROM chat_group_messages WHERE group_id = ?1
             ORDER BY id DESC LIMIT 100",
        ).map_err(|e| e.to_string())?;
        let rows = statement.query_map(params![group_id], |row| {
            Ok(json!({
                "id": row.get::<_, i64>(0)?.to_string(),
                "sender": row.get::<_, String>(1)?,
                "content": row.get::<_, String>(2)?,
                "timestamp": row.get::<_, i64>(3)?
            }))
        }).map_err(|e| e.to_string())?;
        let mut messages = rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
        messages.reverse();
        Ok(messages)
    }

    pub fn send_chat_group_message(
        &self,
        group_id: &str,
        sender_id: &str,
        content: &str,
    ) -> Result<i64, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let role: String = conn.query_row(
            "SELECT m.role FROM chat_group_members m
             WHERE m.group_id = ?1 AND m.member_id = ?2",
            params![group_id, sender_id],
            |row| row.get(0),
        ).map_err(|e| e.to_string())?;
        let group_type: String = conn.query_row(
            "SELECT type FROM chat_groups WHERE group_id = ?1",
            params![group_id],
            |row| row.get(0),
        ).map_err(|e| e.to_string())?;
        if group_type == "channel" && role != "admin" {
            return Err("Seuls les administrateurs peuvent publier dans ce canal.".to_string());
        }
        let cutoff = chrono::Utc::now().timestamp() - 72 * 3600;
        let _ = conn.execute(
            "DELETE FROM chat_group_messages WHERE created_at < ?1",
            params![cutoff],
        );
        conn.execute(
            "INSERT INTO chat_group_messages (group_id, sender_id, content, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![group_id, sender_id, content, chrono::Utc::now().timestamp()],
        ).map_err(|e| e.to_string())?;
        Ok(conn.last_insert_rowid())
    }

    /// Cree les 13 tables standards dans une base utilisateur
    pub fn init_user_db(db_path: &str) -> Result<(), String> {
        let conn = Connection::open(db_path).map_err(|e| e.to_string())?;

        conn.execute_batch(r#"
            -- 1. SOLDE
            CREATE TABLE IF NOT EXISTS balance (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                bubbles INTEGER DEFAULT 0,
                points INTEGER DEFAULT 0,
                frozen_points INTEGER DEFAULT 0,
                updated_at INTEGER NOT NULL
            );
            INSERT OR IGNORE INTO balance (id, bubbles, points, frozen_points, updated_at)
            VALUES (1, 0, 0, 0, strftime('%s', 'now'));

            -- 2. TRANSACTIONS
            CREATE TABLE IF NOT EXISTS transactions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                tx_type TEXT NOT NULL,
                amount INTEGER NOT NULL,
                fee INTEGER DEFAULT 0,
                counterparty TEXT,
                memo TEXT,
                signature BLOB,
                created_at INTEGER NOT NULL
            );

            -- 3. ITEMS ZANDO
            CREATE TABLE IF NOT EXISTS my_items (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                description TEXT,
                price_borb INTEGER DEFAULT 0,
                category TEXT,
                media_urls TEXT,
                published INTEGER DEFAULT 0,
                sold INTEGER DEFAULT 0,
                expires_at INTEGER,
                created_at INTEGER NOT NULL
            );

            -- 4. UBI CLAIMS
            CREATE TABLE IF NOT EXISTS ubi_claims (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                amount INTEGER NOT NULL,
                streak_days INTEGER DEFAULT 0,
                claimed_at INTEGER NOT NULL
            );

            -- 5. FREEZE CONTRACTS (Epargne 2%/an)
            CREATE TABLE IF NOT EXISTS freeze_contracts (
                id TEXT PRIMARY KEY,
                amount INTEGER NOT NULL,
                rate_percent REAL DEFAULT 2.0,
                locked_until INTEGER NOT NULL,
                status TEXT DEFAULT 'active',
                reward_earned INTEGER DEFAULT 0,
                created_at INTEGER NOT NULL
            );

            -- 6. LOANS (Prets)
            CREATE TABLE IF NOT EXISTS loans (
                id TEXT PRIMARY KEY,
                direction TEXT NOT NULL,
                amount INTEGER NOT NULL,
                counterparty TEXT,
                rate_bps INTEGER DEFAULT 0,
                due_at INTEGER,
                status TEXT DEFAULT 'pending',
                repaid INTEGER DEFAULT 0,
                created_at INTEGER NOT NULL
            );

            -- 7. INSURANCE
            CREATE TABLE IF NOT EXISTS insurance (
                id TEXT PRIMARY KEY,
                contribution INTEGER NOT NULL,
                claim_amount INTEGER DEFAULT 0,
                claim_reason TEXT,
                status TEXT DEFAULT 'contributed',
                created_at INTEGER NOT NULL
            );

            -- 8. MESSAGES (Chat)
            CREATE TABLE IF NOT EXISTS messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                conversation_id TEXT NOT NULL,
                sender TEXT NOT NULL,
                recipient TEXT NOT NULL,
                content TEXT NOT NULL,
                encrypted INTEGER DEFAULT 1,
                read INTEGER DEFAULT 0,
                created_at INTEGER NOT NULL
            );

            -- 9. NOTIFICATIONS
            CREATE TABLE IF NOT EXISTS notifications (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                title TEXT NOT NULL,
                content TEXT,
                notif_type TEXT,
                read INTEGER DEFAULT 0,
                created_at INTEGER NOT NULL
            );

            -- 10. SECURITY LOG
            CREATE TABLE IF NOT EXISTS security_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                event_type TEXT NOT NULL,
                ip TEXT,
                device TEXT,
                success INTEGER DEFAULT 1,
                details TEXT,
                created_at INTEGER NOT NULL
            );

            -- 11. REPUTATION HISTORY
            CREATE TABLE IF NOT EXISTS reputation_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                old_score INTEGER NOT NULL,
                new_score INTEGER NOT NULL,
                reason TEXT,
                created_at INTEGER NOT NULL
            );

            -- 12. REFERRALS
            CREATE TABLE IF NOT EXISTS referrals (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                referrer_id TEXT NOT NULL,
                referred_id TEXT NOT NULL,
                commission_earned INTEGER DEFAULT 0,
                created_at INTEGER NOT NULL
            );

            -- 13. SETTINGS
            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT,
                updated_at INTEGER NOT NULL
            );
            INSERT OR IGNORE INTO settings (key, value, updated_at)
            VALUES ('language', 'fr', strftime('%s', 'now'));

            -- 14. MY PROFILE (nom, adresse, KYC)
            CREATE TABLE IF NOT EXISTS my_profile (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                full_name TEXT,
                phone TEXT,
                address TEXT,
                city TEXT,
                country TEXT,
                kyc_status TEXT DEFAULT 'pending',
                kyc_verified_at INTEGER,
                photo_url TEXT,
                bio TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER
            );
            INSERT OR IGNORE INTO my_profile (id, kyc_status, created_at)
            VALUES (1, 'pending', strftime('%s', 'now'));

            -- 15. MY PRIVATE KEY (cle privee chiffree 512 bits)
            CREATE TABLE IF NOT EXISTS my_private_key (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                encrypted_key BLOB NOT NULL,
                iv BLOB NOT NULL,
                salt BLOB NOT NULL,
                algorithm TEXT DEFAULT 'AES-256-GCM',
                created_at INTEGER NOT NULL,
                rotated_at INTEGER
            );

            -- 16. MY CONTACTS (demandes de contact Zando)
            CREATE TABLE IF NOT EXISTS my_contacts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                from_user TEXT NOT NULL,
                to_user TEXT NOT NULL,
                item_id TEXT,
                message TEXT,
                status TEXT DEFAULT 'pending',
                created_at INTEGER NOT NULL,
                responded_at INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_contacts_to_user ON my_contacts(to_user);
            CREATE INDEX IF NOT EXISTS idx_contacts_status ON my_contacts(status);

            -- 17. MY SAVINGS (contrats d'epargne separes)
            CREATE TABLE IF NOT EXISTS my_savings (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                amount_points INTEGER NOT NULL,
                locked_until INTEGER NOT NULL,
                yield_rate REAL DEFAULT 2.0,
                status TEXT DEFAULT 'active',
                created_at INTEGER NOT NULL,
                completed_at INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_savings_status ON my_savings(status);

            -- 18. MY PURCHASES (historique achats Zando)
            CREATE TABLE IF NOT EXISTS my_purchases (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                item_id TEXT NOT NULL,
                seller_id TEXT NOT NULL,
                price_paid INTEGER NOT NULL,
                status TEXT DEFAULT 'completed',
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_purchases_item ON my_purchases(item_id);

            -- 19. MY SALES (historique ventes Zando)
            CREATE TABLE IF NOT EXISTS my_sales (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                item_id TEXT NOT NULL,
                buyer_id TEXT NOT NULL,
                price_received INTEGER NOT NULL,
                fee_paid INTEGER DEFAULT 0,
                status TEXT DEFAULT 'completed',
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_sales_item ON my_sales(item_id);

            -- 20. MY REVIEWS (avis sur les items)
            CREATE TABLE IF NOT EXISTS my_reviews (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                item_id TEXT NOT NULL,
                target_user TEXT NOT NULL,
                rating INTEGER CHECK (rating >= 1 AND rating <= 5),
                comment TEXT,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_reviews_item ON my_reviews(item_id);

            -- 21. MY TRADES (trades en cours)
            CREATE TABLE IF NOT EXISTS my_trades (
                id TEXT PRIMARY KEY,
                trade_type TEXT NOT NULL,
                from_currency TEXT NOT NULL,
                to_currency TEXT NOT NULL,
                amount_from INTEGER NOT NULL,
                amount_to INTEGER NOT NULL,
                rate REAL NOT NULL,
                counterparty TEXT,
                status TEXT DEFAULT 'pending',
                created_at INTEGER NOT NULL,
                completed_at INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_trades_status ON my_trades(status);

            -- 22. MY TRADE HISTORY (historique trades)
            CREATE TABLE IF NOT EXISTS my_trade_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                trade_id TEXT NOT NULL,
                trade_type TEXT NOT NULL,
                amount INTEGER NOT NULL,
                price REAL NOT NULL,
                fee INTEGER DEFAULT 0,
                counterparty TEXT,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_trade_history_trade ON my_trade_history(trade_id);

            -- 23. MY P2P ORDERS (ordres P2P)
            CREATE TABLE IF NOT EXISTS my_p2p_orders (
                id TEXT PRIMARY KEY,
                direction TEXT NOT NULL,
                currency TEXT NOT NULL,
                amount INTEGER NOT NULL,
                price_per_unit REAL NOT NULL,
                min_amount INTEGER,
                max_amount INTEGER,
                payment_method TEXT,
                status TEXT DEFAULT 'active',
                created_at INTEGER NOT NULL,
                expires_at INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_p2p_status ON my_p2p_orders(status);

            -- 24. MY BORB CONVERSIONS (historique conversions)
            CREATE TABLE IF NOT EXISTS my_borb_conversions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                from_unit TEXT NOT NULL,
                to_unit TEXT NOT NULL,
                amount_from INTEGER NOT NULL,
                amount_to INTEGER NOT NULL,
                rate REAL NOT NULL,
                created_at INTEGER NOT NULL
            );

            -- 25. MY BORB UNITS (mes unités fractales)
            CREATE TABLE IF NOT EXISTS my_borb_units (
                unit TEXT PRIMARY KEY,
                balance INTEGER DEFAULT 0,
                created_at INTEGER NOT NULL,
                updated_at INTEGER
            );
            INSERT OR IGNORE INTO my_borb_units (unit, balance, created_at) VALUES
                ('BORB', 0, strftime('%s', 'now')),
                ('borb', 0, strftime('%s', 'now')),
                ('mborb', 0, strftime('%s', 'now')),
                ('nborb', 0, strftime('%s', 'now')),
                ('pborb', 0, strftime('%s', 'now')),
                ('fborb', 0, strftime('%s', 'now'));

            -- 26. NOTIFICATIONS PUSH (queue)
            CREATE TABLE IF NOT EXISTS my_notification_queue (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                type TEXT NOT NULL,
                title TEXT NOT NULL,
                body TEXT NOT NULL,
                data TEXT,
                status TEXT DEFAULT 'pending',
                created_at INTEGER NOT NULL,
                sent_at INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_notif_status ON my_notification_queue(status);

            -- 27. NOTIFICATIONS PREFERENCES
            CREATE TABLE IF NOT EXISTS my_notification_prefs (
                type TEXT PRIMARY KEY,
                enabled INTEGER DEFAULT 1,
                sound INTEGER DEFAULT 1,
                vibration INTEGER DEFAULT 1
            );

            -- 28. PUSH TOKENS
            CREATE TABLE IF NOT EXISTS my_push_tokens (
                token TEXT PRIMARY KEY,
                platform TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                last_used INTEGER
            );

            -- 29. CONTACT REQUESTS SENT
            CREATE TABLE IF NOT EXISTS my_contact_requests_sent (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                to_user TEXT NOT NULL,
                item_id TEXT,
                status TEXT DEFAULT 'pending',
                created_at INTEGER NOT NULL,
                responded_at INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_req_sent_status ON my_contact_requests_sent(status);

            -- 30. CONTACT REQUESTS RECEIVED
            CREATE TABLE IF NOT EXISTS my_contact_requests_received (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                from_user TEXT NOT NULL,
                item_id TEXT,
                status TEXT DEFAULT 'pending',
                created_at INTEGER NOT NULL,
                responded_at INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_req_recv_status ON my_contact_requests_received(status);

            -- 31. SHARED CONTACTS
            CREATE TABLE IF NOT EXISTS my_shared_contacts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                contact_user TEXT NOT NULL,
                shared_at INTEGER NOT NULL,
                fields TEXT
            );

            -- 32. BLOCKED CONTACTS
            CREATE TABLE IF NOT EXISTS my_blocked_contacts (
                user_id TEXT PRIMARY KEY,
                reason TEXT,
                blocked_at INTEGER NOT NULL
            );

            -- 33. GROUPS
            CREATE TABLE IF NOT EXISTS my_groups (
                group_id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                type TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );

            -- 34. GROUP MEMBERS
            CREATE TABLE IF NOT EXISTS my_group_members (
                group_id TEXT NOT NULL,
                member_id TEXT NOT NULL,
                role TEXT DEFAULT 'member',
                joined_at INTEGER NOT NULL,
                PRIMARY KEY (group_id, member_id)
            );

            -- 35. GROUP MESSAGES
            CREATE TABLE IF NOT EXISTS my_group_messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                group_id TEXT NOT NULL,
                sender_id TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_group_msg_group ON my_group_messages(group_id);

            -- 36. PRESENCE
            CREATE TABLE IF NOT EXISTS my_presence (
                user_id TEXT PRIMARY KEY,
                is_online INTEGER DEFAULT 0,
                last_seen INTEGER,
                updated_at INTEGER NOT NULL
            );

            -- 37. CONTACTS ONLINE
            CREATE TABLE IF NOT EXISTS my_contacts_online (
                contact_id TEXT PRIMARY KEY,
                is_online INTEGER DEFAULT 0,
                last_seen INTEGER,
                updated_at INTEGER NOT NULL
            );

            -- 38. FAVORITES
            CREATE TABLE IF NOT EXISTS my_favorites (
                item_id TEXT PRIMARY KEY,
                added_at INTEGER NOT NULL
            );

            -- 39. BOOKMARKS
            CREATE TABLE IF NOT EXISTS my_bookmarks (
                publication_id TEXT PRIMARY KEY,
                added_at INTEGER NOT NULL
            );

            -- 40. ITEM VIEWS
            CREATE TABLE IF NOT EXISTS my_item_views (
                item_id TEXT PRIMARY KEY,
                view_count INTEGER DEFAULT 0,
                last_view INTEGER
            );

            -- 41. PROFILE VIEWS
            CREATE TABLE IF NOT EXISTS my_profile_views (
                viewer_id TEXT PRIMARY KEY,
                viewed_at INTEGER NOT NULL
            );

            -- 42. LIKES
            CREATE TABLE IF NOT EXISTS my_likes (
                item_id TEXT PRIMARY KEY,
                liked_at INTEGER NOT NULL
            );

            -- 43. COMMENTS
            CREATE TABLE IF NOT EXISTS my_comments (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                item_id TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_comments_item ON my_comments(item_id);

            -- 44. RECEIVED COMMENTS
            CREATE TABLE IF NOT EXISTS my_received_comments (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                item_id TEXT NOT NULL,
                from_user TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );

            -- 45. RECEIVED LIKES
            CREATE TABLE IF NOT EXISTS my_received_likes (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                item_id TEXT NOT NULL,
                from_user TEXT NOT NULL,
                liked_at INTEGER NOT NULL
            );

            -- ═══════════════════════════════════════════════════════════════
            -- 18 NOUVELLES TABLES — TREASURY + COUNTER + 7 CASES + INVOICES
            -- ═══════════════════════════════════════════════════════════════

            -- ─── TREASURY SYSTEM ───

            -- Compte propriétaire
            CREATE TABLE IF NOT EXISTS owner_account (
                id INTEGER PRIMARY KEY,
                uq_number TEXT UNIQUE NOT NULL,
                email TEXT UNIQUE NOT NULL,
                created_at INTEGER DEFAULT (strftime('%s', 'now'))
            );

            -- Profil 1 : TREASURY SYSTEM
            CREATE TABLE IF NOT EXISTS treasury_system (
                id INTEGER PRIMARY KEY,
                owner_id INTEGER,
                balance_borb REAL DEFAULT 0,
                balance_usd REAL DEFAULT 0,
                percentage REAL DEFAULT 70,
                auto_sell_enabled INTEGER DEFAULT 1,
                last_sale_at INTEGER,
                created_at INTEGER DEFAULT (strftime('%s', 'now')),
                FOREIGN KEY (owner_id) REFERENCES owner_account(id)
            );

            -- Profil 2 : DELIS-DEV
            CREATE TABLE IF NOT EXISTS dev_profile (
                id INTEGER PRIMARY KEY,
                owner_id INTEGER,
                balance_borb REAL DEFAULT 0,
                balance_usd REAL DEFAULT 0,
                percentage REAL DEFAULT 30,
                can_manipulate_70 INTEGER DEFAULT 0,
                can_buy_from_system INTEGER DEFAULT 0,
                created_at INTEGER DEFAULT (strftime('%s', 'now')),
                FOREIGN KEY (owner_id) REFERENCES owner_account(id)
            );

            -- 6 noms système du Profil 1
            CREATE TABLE IF NOT EXISTS system_profiles (
                id INTEGER PRIMARY KEY,
                code TEXT UNIQUE NOT NULL,
                display_name TEXT NOT NULL,
                icon TEXT,
                description TEXT,
                is_active INTEGER DEFAULT 1,
                created_at INTEGER DEFAULT (strftime('%s', 'now'))
            );

            -- Ventes automatiques BORB
            CREATE TABLE IF NOT EXISTS treasury_system_sales (
                id INTEGER PRIMARY KEY,
                amount_borb REAL NOT NULL,
                crypto_type TEXT NOT NULL,
                crypto_amount REAL NOT NULL,
                rate_at_sale REAL NOT NULL,
                rate_source TEXT DEFAULT 'blockchain',
                destination_wallet TEXT,
                is_automatic INTEGER DEFAULT 1,
                status TEXT DEFAULT 'pending',
                created_at INTEGER DEFAULT (strftime('%s', 'now')),
                completed_at INTEGER
            );

            -- Annonces personnalisées Profil 2
            CREATE TABLE IF NOT EXISTS dev_ads (
                id INTEGER PRIMARY KEY,
                title TEXT NOT NULL,
                description TEXT,
                amount_borb REAL NOT NULL,
                price_usd REAL NOT NULL,
                payment_methods TEXT,
                status TEXT DEFAULT 'active',
                created_at INTEGER DEFAULT (strftime('%s', 'now'))
            );

            -- Revenus 70/30
            CREATE TABLE IF NOT EXISTS treasury_revenue (
                id INTEGER PRIMARY KEY,
                sale_id INTEGER,
                profile_name TEXT,
                amount_borb REAL,
                amount_usd REAL,
                percentage REAL,
                created_at INTEGER DEFAULT (strftime('%s', 'now')),
                FOREIGN KEY (sale_id) REFERENCES treasury_system_sales(id)
            );

            -- Wallets crypto du propriétaire
            CREATE TABLE IF NOT EXISTS treasury_wallets (
                id INTEGER PRIMARY KEY,
                crypto_type TEXT NOT NULL,
                wallet_address TEXT NOT NULL,
                label TEXT,
                is_default INTEGER DEFAULT 0,
                created_at INTEGER DEFAULT (strftime('%s', 'now'))
            );

            -- Messages système
            CREATE TABLE IF NOT EXISTS system_messages (
                id INTEGER PRIMARY KEY,
                profile_code TEXT NOT NULL,
                title TEXT NOT NULL,
                content TEXT NOT NULL,
                type TEXT DEFAULT 'info',
                target_audience TEXT DEFAULT 'all',
                priority INTEGER DEFAULT 0,
                is_active INTEGER DEFAULT 1,
                expires_at INTEGER,
                created_at INTEGER DEFAULT (strftime('%s', 'now')),
                FOREIGN KEY (profile_code) REFERENCES system_profiles(code)
            );

            -- Messages par utilisateur
            CREATE TABLE IF NOT EXISTS user_system_messages (
                id INTEGER PRIMARY KEY,
                user_id TEXT NOT NULL,
                message_id INTEGER NOT NULL,
                is_read INTEGER DEFAULT 0,
                read_at INTEGER,
                created_at INTEGER DEFAULT (strftime('%s', 'now')),
                FOREIGN KEY (message_id) REFERENCES system_messages(id)
            );

            -- ─── COMPTEUR & CALCULATEUR ───

            CREATE TABLE IF NOT EXISTS counter_calculator (
                id INTEGER PRIMARY KEY,
                block_number INTEGER UNIQUE,
                timestamp INTEGER,
                total_users INTEGER,
                total_borb REAL,
                active_users INTEGER,
                inactive_users INTEGER,
                total_fborb TEXT,
                precision_level INTEGER DEFAULT 12,
                flux_in REAL,
                flux_out REAL,
                total_fees REAL,
                fees_tresor REAL,
                fees_frais REAL,
                fees_recompenses REAL,
                fees_epargne REAL,
                fees_credit REAL,
                fees_bouclier REAL,
                distribution_tresor REAL,
                distribution_frais REAL,
                distribution_recompenses REAL,
                distribution_epargne REAL,
                distribution_credit REAL,
                distribution_bouclier REAL,
                sum_6_cases REAL,
                counter_value REAL,
                total REAL,
                invariant_valid INTEGER DEFAULT 1,
                bridges_active TEXT,
                block_hash TEXT,
                previous_hash TEXT
            );

            CREATE TABLE IF NOT EXISTS counter_transactions (
                id INTEGER PRIMARY KEY,
                block_number INTEGER,
                tx_id TEXT,
                from_user TEXT,
                to_user TEXT,
                amount_borb REAL,
                amount_fborb TEXT,
                fee_borb REAL,
                fee_type TEXT,
                to_tresor REAL,
                to_frais REAL,
                to_recompenses REAL,
                to_epargne REAL,
                to_credit REAL,
                to_bouclier REAL,
                invariant_before REAL,
                invariant_after REAL,
                invariant_valid INTEGER,
                created_at INTEGER DEFAULT (strftime('%s', 'now'))
            );

            CREATE TABLE IF NOT EXISTS counter_case_rights (
                id INTEGER PRIMARY KEY,
                case_name TEXT UNIQUE,
                right_percentage REAL,
                total_received REAL DEFAULT 0,
                last_received_at INTEGER,
                updated_at INTEGER DEFAULT (strftime('%s', 'now'))
            );

            -- ─── 7 CASES ───

            CREATE TABLE IF NOT EXISTS seven_cases (
                id INTEGER PRIMARY KEY,
                block_number INTEGER UNIQUE,
                timestamp INTEGER,
                case_1_tresor REAL,
                case_2_frais REAL,
                case_3_recompenses REAL,
                case_4_epargne REAL,
                case_5_credit REAL,
                case_6_bouclier REAL,
                case_7_comptes REAL,
                case_7_count INTEGER,
                sum_6_cases REAL,
                total_7_cases REAL,
                invariant_valid INTEGER DEFAULT 1,
                total_fborb TEXT,
                bridges_active TEXT,
                block_hash TEXT,
                previous_hash TEXT
            );

            CREATE TABLE IF NOT EXISTS bridge_actions (
                id INTEGER PRIMARY KEY,
                block_number INTEGER,
                from_case TEXT,
                to_case TEXT,
                amount REAL,
                reason TEXT,
                timestamp INTEGER DEFAULT (strftime('%s', 'now'))
            );

            CREATE TABLE IF NOT EXISTS anomalies (
                id INTEGER PRIMARY KEY,
                block_number INTEGER,
                tx_id TEXT,
                anomaly_type TEXT,
                severity TEXT,
                details TEXT,
                resolved INTEGER DEFAULT 0,
                created_at INTEGER DEFAULT (strftime('%s', 'now'))
            );

            CREATE TABLE IF NOT EXISTS optimizations (
                id INTEGER PRIMARY KEY,
                block_number INTEGER,
                action_type TEXT,
                details TEXT,
                applied INTEGER DEFAULT 1,
                created_at INTEGER DEFAULT (strftime('%s', 'now'))
            );

            -- ─── FACTURES ───

            CREATE TABLE IF NOT EXISTS invoices (
                id INTEGER PRIMARY KEY,
                invoice_id TEXT UNIQUE NOT NULL,
                tx_id TEXT NOT NULL,
                created_at INTEGER DEFAULT (strftime('%s', 'now')),
                confirmed_at INTEGER,
                signature TEXT,
                tx_hash TEXT,
                block_number INTEGER,
                fee_borb REAL DEFAULT 0,
                fee_usd REAL DEFAULT 0,
                fee_type TEXT,
                rate_at_tx REAL NOT NULL,
                rate_source TEXT DEFAULT 'blockchain',
                amount_borb REAL NOT NULL,
                amount_usd REAL NOT NULL,
                total_debited_borb REAL,
                total_received_borb REAL,
                type TEXT NOT NULL,
                description TEXT,
                from_user TEXT,
                to_user TEXT,
                from_name TEXT,
                to_name TEXT,
                pdf_path TEXT,
                pdf_hash TEXT,
                status TEXT DEFAULT 'confirmed'
            );

            -- Index
            CREATE INDEX IF NOT EXISTS idx_invoices_tx_id ON invoices(tx_id);
            CREATE INDEX IF NOT EXISTS idx_invoices_from_user ON invoices(from_user);
            CREATE INDEX IF NOT EXISTS idx_invoices_to_user ON invoices(to_user);
            CREATE INDEX IF NOT EXISTS idx_invoices_type ON invoices(type);
            CREATE INDEX IF NOT EXISTS idx_invoices_created_at ON invoices(created_at);
            CREATE INDEX IF NOT EXISTS idx_counter_tx_block ON counter_transactions(block_number);
            CREATE INDEX IF NOT EXISTS idx_bridges_block ON bridge_actions(block_number);
            CREATE INDEX IF NOT EXISTS idx_anomalies_block ON anomalies(block_number);
            CREATE INDEX IF NOT EXISTS idx_optimizations_block ON optimizations(block_number);
            -- ═══════════════════════════════════════════════════════════════
            -- CADENAS DE VERIFICATION (anti-piratage)
            -- Stocke le hash de validation du compte
            -- Genere a la creation, valide apres KYC
            -- ═══════════════════════════════════════════════════════════════

            CREATE TABLE IF NOT EXISTS cadenas_verification (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                uq_number TEXT NOT NULL,
                hash_brut TEXT NOT NULL,
                hash_transforme TEXT NOT NULL,
                nom TEXT NOT NULL,
                prenom TEXT NOT NULL,
                date_naissance TEXT NOT NULL,
                lieu_naissance TEXT NOT NULL,
                pays TEXT NOT NULL,
                adresse TEXT,
                telephone TEXT,
                created_at INTEGER NOT NULL,
                verifie_at INTEGER,
                est_verifie INTEGER DEFAULT 0,
                verrouille INTEGER DEFAULT 1,
                est_actif INTEGER DEFAULT 1
            );
            CREATE INDEX IF NOT EXISTS idx_cadenas_uq ON cadenas_verification(uq_number);
        "#).map_err(|e| e.to_string())?;

        Ok(())
    }
    // ====================
    // RECHERCHE PAR EMAIL / PSEUDO (pour collisions)
    // ====================

    // ====================
    // RECHERCHE PAR EMAIL / PSEUDO (pour collisions)
    // ====================

    pub fn get_account_by_email(&self, email: &str) -> Option<Value> {
        let conn = self.conn.lock().ok()?;

        // Normaliser l'email
        let normalized = email.to_lowercase()
            .replace(".", "")
            .replace("-", "")
            .replace("_", "");

        let mut stmt = conn.prepare(
            "SELECT uq_number, uq_email, uq_pseudo FROM uq_index
             WHERE normalized_email = ?1 OR LOWER(uq_email) = ?2 LIMIT 1"
        ).ok()?;
        let mut rows = stmt.query(rusqlite::params![normalized, email.to_lowercase()]).ok()?;

        if let Some(row) = rows.next().ok()? {
            return Some(serde_json::json!({
                "uq_number": row.get::<_, String>(0).ok()?,
                "email": row.get::<_, String>(1).ok()?,
                "pseudo": row.get::<_, String>(2).ok()?,
            }));
        }
        None
    }


    /// Liste tous les comptes de l'index
    pub fn list_all(&self) -> Vec<Value> {
        let conn = self.conn.lock().unwrap();
        let mut result = Vec::new();

        if let Ok(mut stmt) = conn.prepare(
            "SELECT uq_number, uq_email, uq_pseudo, is_owner, db_path, created_at
             FROM uq_index ORDER BY created_at ASC"
        ) {
            if let Ok(rows) = stmt.query_map([], |row| {
                Ok(json!({
                    "uq_number": row.get::<_, String>(0)?,
                    "uq_email": row.get::<_, String>(1)?,
                    "uq_pseudo": row.get::<_, String>(2)?,
                    "is_owner": row.get::<_, i64>(3)? == 1,
                    "db_path": row.get::<_, String>(4)?,
                    "created_at": row.get::<_, i64>(5).unwrap_or(0),
                }))
            }) {
                for row in rows.flatten() {
                    result.push(row);
                }
            }
        }

        result
    }
    pub fn get_account_by_pseudo(&self, pseudo: &str) -> Option<Value> {
        let conn = self.conn.lock().ok()?;

        // Normaliser le pseudo
        let normalized = pseudo.to_lowercase()
            .replace(".", "")
            .replace("-", "")
            .replace("_", "")
            .replace("uq@", "");

        let mut stmt = conn.prepare(
            "SELECT uq_number, uq_email, uq_pseudo FROM uq_index
             WHERE normalized_pseudo = ?1 OR LOWER(uq_pseudo) = ?2 LIMIT 1"
        ).ok()?;
        let mut rows = stmt.query(rusqlite::params![normalized, pseudo.to_lowercase()]).ok()?;

        if let Some(row) = rows.next().ok()? {
            return Some(serde_json::json!({
                "uq_number": row.get::<_, String>(0).ok()?,
                "email": row.get::<_, String>(1).ok()?,
                "pseudo": row.get::<_, String>(2).ok()?,
            }));
        }
        None
    }

    /// Met a jour le mot de passe d'un compte
    /// Lit les 10 champs du profil complet pour un UQ
    pub fn get_full_profile(&self, uq_number: &str) -> std::collections::HashMap<String, String> {
        let mut result = std::collections::HashMap::new();
        let conn = self.conn.lock().unwrap();
        let r = conn.query_row(
            "SELECT COALESCE(sexe,''), COALESCE(ville_provenance,''), COALESCE(ville_actuelle,''),
                    COALESCE(statut_marital,''), COALESCE(photo_url,''), COALESCE(description,''),
                    COALESCE(emploi,''), COALESCE(mobile_money_number,''), COALESCE(mobile_money_operator,''),
                    COALESCE(date_naissance,0), COALESCE(display_currency,'')
             FROM uq_index WHERE uq_number = ?1",
            rusqlite::params![uq_number],
            |row| Ok((
                row.get::<_, String>(0).unwrap_or_default(),
                row.get::<_, String>(1).unwrap_or_default(),
                row.get::<_, String>(2).unwrap_or_default(),
                row.get::<_, String>(3).unwrap_or_default(),
                row.get::<_, String>(4).unwrap_or_default(),
                row.get::<_, String>(5).unwrap_or_default(),
                row.get::<_, String>(6).unwrap_or_default(),
                row.get::<_, String>(7).unwrap_or_default(),
                row.get::<_, String>(8).unwrap_or_default(),
                row.get::<_, i64>(9).unwrap_or(0),
                row.get::<_, String>(10).unwrap_or_default(),
            )),
        );
        if let Ok((sexe, vp, va, sm, ph, de, em, mm_n, mm_o, dn, devise)) = r {
            result.insert("sexe".into(), sexe);
            result.insert("ville_provenance".into(), vp);
            result.insert("ville_actuelle".into(), va);
            result.insert("statut_marital".into(), sm);
            result.insert("photo_url".into(), ph);
            result.insert("description".into(), de);
            result.insert("emploi".into(), em);
            result.insert("mobile_money_number".into(), mm_n);
            result.insert("mobile_money_operator".into(), mm_o);
            result.insert("date_naissance".into(), if dn > 0 { dn.to_string() } else { String::new() });
            result.insert("display_currency".into(), devise);
        }
        result
    }

    pub fn update_password(&self, uq_number: &str, new_password: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;

        // Generer un nouveau salt
        let salt = format!("uq_{}_salt_{}", uq_number.to_lowercase().replace("-", "_"), chrono::Utc::now().timestamp());
        let new_hash = Self::hash_password(new_password, &salt);

        conn.execute(
            "UPDATE uq_index SET hash_password = ?1, salt = ?2 WHERE uq_number = ?3",
            rusqlite::params![new_hash, salt, uq_number],
        ).map_err(|e| e.to_string())?;

        Ok(())
    }

    /// Recherche de pseudos (auto-complétion)
    pub fn search_pseudos(&self, query: &str, limite: i64) -> Vec<serde_json::Value> {
        let pattern = format!("%{}%", query);
        
        // 1. Requete SQL enrichie - lock relache apres collecte
        #[derive(Debug)]
        struct PseudoRow {
            pseudo: String,
            uq_number: String,
            sexe: String,
            ville_prov: String,
            ville_act: String,
            statut_m: String,
            photo: String,
            desc: String,
            emploi: String,
            mm_num: String,
            mm_op: String,
            kyc: i64,
        }
        
        let sql = format!(
            "SELECT uq_pseudo, uq_number,
                    COALESCE(sexe, ''),
                    COALESCE(ville_provenance, ''),
                    COALESCE(ville_actuelle, ''),
                    COALESCE(statut_marital, ''),
                    COALESCE(photo_url, ''),
                    COALESCE(description, ''),
                    COALESCE(emploi, ''),
                    COALESCE(mobile_money_number, ''),
                    COALESCE(mobile_money_operator, ''),
                    COALESCE(kyc_verified, 0)
             FROM uq_index
             WHERE LOWER(uq_pseudo) LIKE ?1
               AND COALESCE(uq_pseudo, '') != ''
             LIMIT {}",
            limite.clamp(1, 200)
        );
        let rows: Vec<PseudoRow> = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = match conn.prepare(&sql) {
                Ok(s) => s,
                Err(_) => return vec![],
            };
            
            let iter = stmt.query_map(rusqlite::params![pattern], |row| {
                Ok(PseudoRow {
                    pseudo: row.get(0)?,
                    uq_number: row.get(1)?,
                    sexe: row.get(2).unwrap_or_default(),
                    ville_prov: row.get(3).unwrap_or_default(),
                    ville_act: row.get(4).unwrap_or_default(),
                    statut_m: row.get(5).unwrap_or_default(),
                    photo: row.get(6).unwrap_or_default(),
                    desc: row.get(7).unwrap_or_default(),
                    emploi: row.get(8).unwrap_or_default(),
                    mm_num: row.get(9).unwrap_or_default(),
                    mm_op: row.get(10).unwrap_or_default(),
                    kyc: row.get(11).unwrap_or(0),
                })
            });
            
            let mut collected = Vec::new();
            if let Ok(r) = iter {
                for row in r {
                    if let Ok(pr) = row {
                        collected.push(pr);
                    }
                }
            }
            collected
        }; // LOCK RELACHE ICI
        
        // 2. Appels externes HORS du lock
        let mut results = Vec::new();
        for r in rows {
            let (_, display_name) = crate::uq_public_alias::get_public_identity(&r.uq_number);
            let (prenom, nom) = if display_name.contains(' ') {
                let parts: Vec<&str> = display_name.splitn(2, ' ').collect();
                (parts[0].to_string(), parts.get(1).unwrap_or(&"").to_string())
            } else {
                ("".to_string(), display_name.clone())
            };
            
            results.push(json!({
                "pseudo": r.pseudo,
                "nom": nom,
                "prenom": prenom,
                "display": if display_name.is_empty() { r.pseudo.clone() } else { format!("{} ({})", r.pseudo, display_name) },
                "sexe": r.sexe,
                "ville_provenance": r.ville_prov,
                "ville_actuelle": r.ville_act,
                "statut_marital": r.statut_m,
                "photo_url": r.photo,
                "description": r.desc,
                "emploi": r.emploi,
                "mobile_money_number": r.mm_num,
                "mobile_money_operator": r.mm_op,
                "verified": r.kyc == 1
            }));
        }
        
        results
    }

/// Ouvre uq_index.db avec la clé maître (SQLCipher)
pub fn open_encrypted_index() -> Result<rusqlite::Connection, String> {
    let master = crate::security::master_key::get_master_key()
        .ok_or_else(|| "Système verrouillé".to_string())?;
    let key_hex = crate::security::master_key::key_to_hex(&master);
    
    let conn = rusqlite::Connection::open("uq_index.db")
        .map_err(|e| format!("Ouverture uq_index échouée: {}", e))?;
    
    conn.pragma_update(None, "key", &key_hex)
        .map_err(|e| format!("Application clé échouée: {}", e))?;
    
    // Test
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(()))
        .map_err(|_| "Déchiffrement uq_index échoué".to_string())?;
    
    eprintln!("🔓 uq_index.db déverrouillée");
    Ok(conn)
}
}





