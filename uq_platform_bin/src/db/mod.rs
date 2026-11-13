pub mod schema;
pub mod migrations;

use rusqlite::Connection;

/// Initialise une DB avec migrations automatiques
pub fn init_with_migrations(db_path: &str) -> Result<(), String> {
    let conn = Connection::open(db_path).map_err(|e| e.to_string())?;

    // Activer les contraintes de clé étrangère
    let _ = conn.execute("PRAGMA foreign_keys = ON", []);

    // Exécuter les migrations
    migrations::run_all(&conn).map_err(|e| e.to_string())?;

    // Sécurité : garantir les colonnes de traçabilité des transactions
    ensure_transactions_columns(&conn)?;

    Ok(())
}

// ====================
// 🧬 ALIGNEMENT DU SCHÉMA `transactions`
// ====================
//
// Le handler de transfert (api_handlers::bubble_transfer) écrit :
//   device, os, browser, ip, sender_uq, recipient_uq, amount_borb,
//   amount_usd, taux_usd, public_key, status, reference, tampered,
//   signature, cadenas_hash, internal_clock, universal_key_hash
//
// Or les comptes créés par l'ancien `schema::apply_schema` n'avaient
// que : id, tx_type, amount, fee, counterparty, memo, signature,
// created_at  →  « table transactions has no column named device »
// →  HTTP 500 sur TOUT transfert.
//
// Ces colonnes sont donc ajoutées ici, de façon idempotente.

pub const TRANSACTIONS_EXTRA_COLUMNS: &[(&str, &str)] = &[
    ("device", "TEXT"),
    ("os", "TEXT"),
    ("browser", "TEXT"),
    ("ip", "TEXT"),
    ("sender_uq", "TEXT"),
    ("recipient_uq", "TEXT"),
    ("amount_borb", "REAL DEFAULT 0"),
    ("amount_usd", "REAL DEFAULT 0"),
    ("taux_usd", "TEXT"),
    ("public_key", "TEXT"),
    ("status", "TEXT DEFAULT 'success'"),
    ("reference", "TEXT"),
    ("device_hash", "TEXT"),
    ("tampered", "INTEGER DEFAULT 0"),
    ("cadenas_hash", "TEXT"),
    ("internal_clock", "INTEGER"),
    ("universal_key_hash", "TEXT"),
    // ===== SPÉCIFICATION « HISTORIQUES LOCAUX » =====
    // borb_id : hash unique de la transaction
    // prev_hash : chaînage avec la ligne précédente
    // block_number : bloc au moment de l'écriture
    ("borb_id", "TEXT"),
    ("prev_hash", "TEXT"),
    ("block_number", "INTEGER DEFAULT 0"),
];

/// Ajoute les colonnes manquantes de `transactions` (idempotent).
pub fn ensure_transactions_columns(conn: &Connection) -> Result<(), String> {
    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='transactions'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    if exists == 0 {
        // La table n'existe pas : on applique le schéma complet du compte
        schema::apply_schema(conn).map_err(|e| format!("apply_schema: {}", e))?;
        eprintln!("   ✅ Table transactions créée (schéma complet)");
        return Ok(());
    }

    // Colonnes déjà présentes
    let mut existing: Vec<String> = Vec::new();
    if let Ok(mut stmt) = conn.prepare("PRAGMA table_info(transactions)") {
        if let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(1)) {
            for row in rows.flatten() {
                existing.push(row);
            }
        }
    }

    let mut added = 0usize;
    for (name, sql_type) in TRANSACTIONS_EXTRA_COLUMNS {
        if existing.iter().any(|c| c == name) {
            continue;
        }
        let sql = format!("ALTER TABLE transactions ADD COLUMN {} {}", name, sql_type);
        conn.execute(&sql, [])
            .map_err(|e| format!("ALTER TABLE transactions ADD {} : {}", name, e))?;
        added += 1;
    }

    if added > 0 {
        eprintln!("   ✅ transactions : {} colonne(s) ajoutée(s)", added);
    }

    Ok(())
}