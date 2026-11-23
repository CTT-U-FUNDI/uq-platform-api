// ====================
// MIGRATIONS VERSIONNÉES — UQ PLATFORM
// Auto-exécutées au démarrage, idempotentes, historisées
// ====================

use rusqlite::{Connection, Result};

// ====================
// STRUCTURE D'UNE MIGRATION
// ====================

pub struct Migration {
    pub version: i64,
    pub name: &'static str,
    pub sql: &'static str,
}

// ====================
// LISTE DES MIGRATIONS (ordre croissant de version)
// ====================

pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "create_schema_version_table",
        sql: r#"
            CREATE TABLE IF NOT EXISTS _schema_version (
                version INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                applied_at INTEGER NOT NULL
            );
        "#,
    },
    Migration {
        version: 2,
        name: "initial_schema",
        sql: "", // Sera exécuté via schema::apply_schema (car trop gros)
    },
    Migration {
        version: 3,
        name: "add_attempts_ok_to_cooldown",
        sql: r#"
            -- Ajouter les colonnes si elles n'existent pas (SQLite ne supporte pas IF NOT EXISTS sur ALTER)
            -- On utilise une approche par try (les erreurs sont ignorées si la colonne existe)
        "#,
    },
    Migration {
        version: 4,
        name: "align_transactions_columns",
        sql: "", // Exécuté via db::ensure_transactions_columns
    },
    Migration {
        version: 5,
        name: "local_history_tables",
        sql: "", // Exécuté via local_history::init_tables
    },
];

// ====================
// FONCTIONS
// ====================

/// Récupère la version actuelle du schéma
pub fn get_current_version(conn: &Connection) -> i64 {
    // Créer la table de version si elle n'existe pas
    conn.execute(
        "CREATE TABLE IF NOT EXISTS _schema_version (
            version INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            applied_at INTEGER NOT NULL
        )",
        [],
    )
    .ok();

    // Lire la version maximale
    conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM _schema_version",
        [],
        |row| row.get(0),
    )
    .unwrap_or(0)
}

/// Enregistre qu'une migration a été appliquée
pub fn record_migration(conn: &Connection, version: i64, name: &str) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT OR REPLACE INTO _schema_version (version, name, applied_at) VALUES (?1, ?2, ?3)",
        rusqlite::params![version, name, now],
    )?;
    Ok(())
}

/// Applique une migration spécifique
pub fn apply_migration(conn: &Connection, migration: &Migration) -> Result<()> {
    eprintln!("📦 Migration v{} : {}", migration.version, migration.name);

    if migration.version == 2 {
        // Migration spéciale : schéma complet via schema::apply_schema
        crate::db::schema::apply_schema(conn)?;
        eprintln!("   ✅ Schéma complet appliqué");
    } else if migration.version == 3 {
        // Migration 3 : ajouter les colonnes à security_login_cooldown_uq
        // SQLite ne supporte pas ADD COLUMN IF NOT EXISTS, on essaie et on ignore les erreurs
        let _ = conn.execute(
            "ALTER TABLE security_login_cooldown_uq ADD COLUMN attempts_ok INTEGER DEFAULT 0",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE security_login_cooldown_uq ADD COLUMN is_locked INTEGER DEFAULT 0",
            [],
        );
        let _ = conn.execute(
            "ALTER TABLE security_login_cooldown_uq ADD COLUMN locked_until INTEGER DEFAULT 0",
            [],
        );
        eprintln!("   ✅ Colonnes ajoutées (ou déjà présentes)");
    } else if migration.version == 4 {
        // Migration 4 : aligner `transactions` sur ce qu'écrit bubble_transfer
        crate::db::ensure_transactions_columns(conn).map_err(|e| {
            rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::other(e)))
        })?;
        eprintln!("   ✅ Schéma transactions aligné");
    } else if migration.version == 5 {
        // Migration 5 : tables des historiques locaux + récupération par voisins
        crate::local_history::init_tables(conn).map_err(|e| {
            rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::other(e)))
        })?;
        eprintln!("   ✅ Tables d'historique local créées");
    } else if !migration.sql.is_empty() {
        conn.execute_batch(migration.sql)?;
        eprintln!("   ✅ SQL exécuté");
    }

    // Enregistrer
    record_migration(conn, migration.version, migration.name)?;
    Ok(())
}

/// Exécute toutes les migrations non appliquées
pub fn run_all(conn: &Connection) -> Result<i64> {
    let current = get_current_version(conn);
    let mut applied = 0;

    eprintln!("🔄 Migrations : version actuelle = {}", current);

    for migration in MIGRATIONS {
        if migration.version > current {
            match apply_migration(conn, migration) {
                Ok(_) => {
                    applied += 1;
                }
                Err(e) => {
                    eprintln!("❌ Erreur migration v{} : {}", migration.version, e);
                    return Err(e);
                }
            }
        }
    }

    if applied == 0 {
        eprintln!("✅ Base à jour (version {})", current);
    } else {
        let new_version = get_current_version(conn);
        eprintln!(
            "✅ {} migration(s) appliquée(s) — version {} → {}",
            applied, current, new_version
        );
    }

    Ok(applied)
}

/// Liste l'historique des migrations appliquées
pub fn list_history(conn: &Connection) -> Vec<(i64, String, i64)> {
    let mut result = Vec::new();
    let _ = conn.execute(
        "CREATE TABLE IF NOT EXISTS _schema_version (
            version INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            applied_at INTEGER NOT NULL
        )",
        [],
    );

    if let Ok(mut stmt) =
        conn.prepare("SELECT version, name, applied_at FROM _schema_version ORDER BY version")
    {
        if let Ok(rows) = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        }) {
            for row in rows.flatten() {
                result.push(row);
            }
        }
    }

    result
}
