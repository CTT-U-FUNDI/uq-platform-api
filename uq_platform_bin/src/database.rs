use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock, atomic::{AtomicU64, Ordering}};
use std::thread;
use std::time::Duration;
use serde::{Serialize, Deserialize};
use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::SqliteConnectionManager;
use uq_blockchain::constants::TOTAL_BORB;
/// Base de donnees persistante JSON
pub struct Database {
    data: Arc<Mutex<DbData>>,
    file_path: String,
    sqlite_path: String,
    sqlite_pool: Arc<Pool<SqliteConnectionManager>>,
    persist_json: bool,
    // WAL instance for durable append-only logging
    pub wal: Arc<crate::wal::Wal>,
    // transactions counter since last snapshot
    pub tx_since_snapshot: Arc<AtomicU64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbData {
    pub publications: Vec<serde_json::Value>,
    pub interests: Vec<serde_json::Value>,
    pub claims: Vec<serde_json::Value>,
    pub loans: Vec<serde_json::Value>,
    pub exchange_orders: Vec<serde_json::Value>,
    pub sanctions: Vec<serde_json::Value>,
    pub marketers: Vec<serde_json::Value>,
    pub kyc_records: Vec<serde_json::Value>,
    pub offline_txs: Vec<serde_json::Value>,
    pub fragments: Vec<serde_json::Value>,
    pub player_sessions: Vec<serde_json::Value>,
    pub connections: Vec<serde_json::Value>,
    pub bubbles: Vec<serde_json::Value>,
    pub fees: Vec<serde_json::Value>,
    pub votes: Vec<serde_json::Value>,
    pub accounts: Vec<Account>,
    pub transactions: Vec<TransactionRecord>,
    pub six_cases: Vec<CaseBalance>,
    pub ubi_claims: Vec<UbiClaim>,
    pub proposals: Vec<Proposal>,
    pub messages: Vec<Message>,
    pub offers: Vec<Offer>,
    pub sessions: Vec<Session>,
    pub notifications: Vec<serde_json::Value>,
    pub savings_contracts: Vec<serde_json::Value>,
    pub nanties: Vec<serde_json::Value>,
    pub debts: Vec<serde_json::Value>,
    pub creator_prices: Vec<serde_json::Value>,
    pub total_streamed: u128,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub public_key: Vec<u8>,
    pub balance_bubbles: u64,
    pub balance_points: u64,
    pub kyc_verified: bool,
    pub created_at: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionRecord {
    pub id: String,
    pub sender: String,
    pub recipient: String,
    pub amount_points: u64,
    pub fee_points: u64,
    pub timestamp: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaseBalance {
    pub name: String,
    pub balance: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UbiClaim {
    pub account_id: String,
    pub last_claim_at: i64,
    pub streak_days: u64,
    pub total_claimed: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proposal {
    pub id: String,
    pub title: String,
    pub description: String,
    pub proposer: String,
    pub votes_for: u64,
    pub votes_against: u64,
    pub status: String,
    pub created_at: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub sender: String,
    pub recipient: String,
    pub content: String,
    /// Photo jointe (data URL) stockee localement, jamais sur un cloud.
    #[serde(default)]
    pub image: Option<String>,
    pub delivered: bool,
    pub timestamp: i64,
    /// Fin de vie du message. `None` = persistant : c est le defaut depuis que
    /// l ephemere est une option par conversation, plus une regle generale.
    #[serde(default)]
    pub expires_at: Option<i64>,
}

/// Duree de vie d'une conversation en mode ephemere : 72 h apres l'envoi.
/// Une conversation qui n'a pas active le mode garde ses messages.
pub const MESSAGE_TTL_SECS: i64 = 72 * 60 * 60;

/// Duree de vie d'une publication (annonce, conference, post) : 7 jours.
pub const PUBLICATION_TTL_SECS: i64 = 7 * 24 * 60 * 60;

/// Duree de vie d'un statut (story publiee depuis le flux Zando) : 24 heures.
pub const STATUT_TTL_SECS: i64 = 24 * 60 * 60;

/// Duree de vie appliquee selon le type de publication.
pub fn ttl_publication(kind: &str) -> i64 {
    if kind == "status" {
        STATUT_TTL_SECS
    } else {
        PUBLICATION_TTL_SECS
    }
}

/// Plafond du media joint a une publication, aligne sur le media Zando : la
/// photo voyage en data URI dans le JSON, et le serveur refuse au-dela de 2 Mo.
pub const PUBLICATION_MEDIA_MAX_CHARS: usize = 1_500_000;

/// Lecture du drapeau de certification dans un publication déjà enrichie.
fn certifie_du_json(publication: &serde_json::Value) -> bool {
    publication
        .get("author_certifie")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicationComment {
    pub author: String,
    pub pseudo: String,
    pub content: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicationRecord {
    pub id: String,
    pub author: String,
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub price: Option<f64>,
    /// Media joint (URL ou data URI) : vide pour une publication de texte.
    #[serde(default)]
    pub media: String,
    /// Element Zando que le statut met en avant. La publication n'en est qu'un
    /// lien : `shop_items` reste la source du produit.
    #[serde(default)]
    pub zando_item_id: String,
    pub created_at: i64,
    #[serde(default)]
    pub likes: Vec<String>,
    #[serde(default)]
    pub comments: Vec<PublicationComment>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Offer {
    pub id: String,
    pub seller: String,
    pub description: String,
    pub price_per_mb: u64,
    pub total_size_mb: u64,
    pub active: bool,
    pub created_at: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub account_id: String,
    pub token: String,
    pub created_at: i64,
    pub expires_at: i64,
}


impl Database {
    /// Acces public au pool SQLite (pour les modules externes).
    pub fn conn(&self) -> Result<r2d2::PooledConnection<r2d2_sqlite::SqliteConnectionManager>, r2d2::Error> {
        self.sqlite_pool.get()
    }
}

impl Database {
    pub fn shared(file_path: &str) -> Arc<Database> {
        static DB_INSTANCE: OnceLock<Arc<Database>> = OnceLock::new();
        DB_INSTANCE
            .get_or_init(|| Arc::new(Database::new(file_path)))
            .clone()
    }

    /// Crée ou ouvre la base de données (JSON + SQLite)
    pub fn new(file_path: &str) -> Self {
        let initial_data = if Path::new(file_path).exists() {
            let content = fs::read_to_string(file_path).unwrap_or_default();
            serde_json::from_str(&content).unwrap_or_else(|_| Database::default_data())
        } else {
            Database::default_data()
        };

        let sqlite_path = if file_path.ends_with(".json") {
            file_path.trim_end_matches(".json").to_string() + ".db"
        } else {
            file_path.to_string() + ".db"
        };

        // create r2d2 pool for sqlite
        let manager = SqliteConnectionManager::file(&sqlite_path);
        let pool = Pool::builder()
            .max_size(15)
            .build(manager)
            .expect("Failed to create sqlite r2d2 pool");

        let persist_json = std::env::var("UQ_KEEP_JSON").map(|v| {
            let vl = v.to_lowercase();
            !(vl == "0" || vl == "false")
        }).unwrap_or(false);

        // Open or create WAL (use env WAL_DIR or default)
        let wal_dir = std::env::var("WAL_DIR").unwrap_or_else(|_| "./data/wal".to_string());
        let wal = match crate::wal::Wal::open(wal_dir) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("Failed to open WAL: {}", e);
                // As a fallback create a dummy in-memory wal by opening default dir
                Arc::new(crate::wal::Wal::open("./data/wal").expect("Failed to create fallback WAL"))
            }
        };

        let db = Self {
            data: Arc::new(Mutex::new(initial_data)),
            file_path: file_path.to_string(),
            sqlite_path: sqlite_path.clone(),
            sqlite_pool: Arc::new(pool),
            persist_json,
            wal: wal.clone(),
            tx_since_snapshot: Arc::new(AtomicU64::new(0)),
        };

        // initialiser sqlite (création des tables)
        if let Err(e) = db.init_sqlite() {
            eprintln!("Failed to init sqlite: {}", e);
        }

        // Migrer les données JSON existantes vers SQLite si la base est encore vide
        if let Err(e) = db.migrate_json_to_sqlite() {
            eprintln!("Failed to migrate json to sqlite: {}", e);
        }

        // L'historique du chat revient de SQLite : la vue memoire seule ferait
        // table rase a chaque redemarrage du serveur.
        db.charger_messages_sqlite();

        db.save();

        // Replay WAL entries since last snapshot into in-memory state (idempotent by op_id)
        {
            if let Ok(last_meta) = db.wal.read_meta() {
                let last_snapshot_seq = last_meta.unwrap_or(0u64);
                if let Ok(entries) = db.wal.read_all() {
                    for entry in entries.into_iter().filter(|e| e.seq > last_snapshot_seq) {
                        if entry.op_type == "tx" {
                            let sender = entry.payload.get("sender").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            let recipient = entry.payload.get("recipient").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            let amount = entry.payload.get("amount_points").and_then(|v| v.as_u64()).unwrap_or(0);
                            let fee = entry.payload.get("fee_points").and_then(|v| v.as_u64()).unwrap_or(0);
                            let timestamp = entry.payload.get("timestamp").and_then(|v| v.as_i64()).unwrap_or(chrono::Utc::now().timestamp());

                            // idempotency: skip if transaction already present in memory
                            let exists = {
                                let d = db.data.lock().unwrap();
                                d.transactions.iter().any(|t| t.id == entry.op_id)
                            };
                            if exists { continue; }

                            // apply to in-memory state
                            {
                                let mut d = db.data.lock().unwrap();
                                d.transactions.push(TransactionRecord {
                                    id: entry.op_id.clone(),
                                    sender: sender.clone(),
                                    recipient: recipient.clone(),
                                    amount_points: amount as u64,
                                    fee_points: fee as u64,
                                    timestamp,
                                });
                                if let Some(s) = d.accounts.iter_mut().find(|a| a.id == sender) {
                                    s.balance_points = s.balance_points.saturating_sub(amount as u64);
                                }
                                if let Some(r) = d.accounts.iter_mut().find(|a| a.id == recipient) {
                                    r.balance_points = r.balance_points.saturating_add(amount as u64);
                                }
                            }

                            // persist to sqlite best-effort
                            if let Ok(conn) = db.sqlite_pool.get() {
                                let _ = conn.execute(
                                    "INSERT OR IGNORE INTO transactions (id, sender, recipient, amount_points, fee_points, timestamp) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                                    rusqlite::params![entry.op_id, sender, recipient, amount as i64, fee as i64, timestamp]
                                );
                            }
                        }
                    }
                }
            }
        }

        // Spawn snapshot worker: performs periodic snapshots (in a background thread)
        {
            let wal_clone = db.wal.clone();
            let pool_clone = db.sqlite_pool.clone();
            let data_clone = db.data.clone();
            let tx_counter = db.tx_since_snapshot.clone();

            // config from env
            let snapshot_interval_secs = std::env::var("SNAPSHOT_INTERVAL_SECS").ok().and_then(|s| s.parse::<u64>().ok()).unwrap_or(10u64);
            let snapshot_tx_threshold = std::env::var("SNAPSHOT_TX_THRESHOLD").ok().and_then(|s| s.parse::<u64>().ok()).unwrap_or(5000u64);

            thread::Builder::new().name("snapshot-worker".into()).spawn(move || {
                loop {
                    // wait up to the configured interval, but wake earlier if tx threshold reached
                    for _ in 0..snapshot_interval_secs {
                        thread::sleep(Duration::from_secs(1));
                        let c = tx_counter.load(Ordering::SeqCst);
                        if c >= snapshot_tx_threshold {
                            break; // proceed to snapshot immediately
                        }
                    }

                    // build snapshot from in-memory state (short lock)
                    let snapshot_data = {
                        match data_clone.lock() {
                            Ok(d) => serde_json::to_string(&*d).ok(),
                            Err(_) => None,
                        }
                    };
                    if snapshot_data.is_none() {
                        continue;
                    }

                    // determine last wal seq
                    let last_seq = {
                        let entries_result = wal_clone.read_all();
// [LOG-QUIET]                         eprintln!("SNAPSHOT DEBUG: read_all() a retourné {:?}", entries_result.as_ref().map(|e| e.len()));
                        match entries_result {
                            Ok(entries) => entries.last().map(|e| e.seq).unwrap_or(0),
                            Err(_) => 0,
                        }
                    };

                    // persist snapshot into sqlite atomically
                    if let Ok(mut conn) = pool_clone.get() {
                        if let Ok(tx) = conn.transaction() {
                            // Replace accounts
                            if let Ok(d) = data_clone.lock() {
                                let _ = tx.execute("DELETE FROM accounts", []);
                                for acc in d.accounts.iter() {
                                    let _ = tx.execute(
                                        "INSERT OR REPLACE INTO accounts (id, public_key, balance_bubbles, balance_points, kyc_verified, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                                        rusqlite::params![acc.id, acc.public_key.as_slice(), acc.balance_bubbles as i64, acc.balance_points as i64, if acc.kyc_verified {1i64} else {0i64}, acc.created_at]
                                    );
                                }

                                // Replace transactions
                                let _ = tx.execute("DELETE FROM transactions", []);
                                for tr in d.transactions.iter() {
                                    let _ = tx.execute(
                                        "INSERT OR REPLACE INTO transactions (id, sender, recipient, amount_points, fee_points, timestamp) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                                        rusqlite::params![tr.id, tr.sender, tr.recipient, tr.amount_points as i64, tr.fee_points as i64, tr.timestamp]
                                    );
                                }

                                // Optionally persist other tables as needed (offers, sessions, ubi_claims)
                            }

                            if tx.commit().is_ok() {
                                // write wal meta then rotate wal up to last_seq
                                match wal_clone.write_meta(last_seq) {
    Ok(()) => eprintln!("SNAPSHOT OK: wal.meta mis à jour avec seq = {}", last_seq),
    Err(e) => eprintln!("SNAPSHOT ERROR: write_meta a échoué: {}", e),
}
                                let _ = wal_clone.rotate_after(last_seq);
                                // reset counter
                                tx_counter.store(0, Ordering::SeqCst);
                            }
                        }
                    }
                }
            }).ok();
        }

        db
    }
    /// Donnees par defaut avec les six coffres
    fn default_data() -> DbData {
        DbData {
            accounts: Vec::new(),
            publications: Vec::new(),
            interests: Vec::new(),
            claims: Vec::new(),
            loans: Vec::new(),
            exchange_orders: Vec::new(),
            sanctions: Vec::new(),
            marketers: Vec::new(),
            kyc_records: Vec::new(),
            offline_txs: Vec::new(),
            fragments: Vec::new(),
            player_sessions: Vec::new(),
            notifications: Vec::new(),
            savings_contracts: Vec::new(),
            nanties: Vec::new(),
            debts: Vec::new(),
            creator_prices: Vec::new(),
            total_streamed: 0,
            connections: Vec::new(),
            bubbles: Vec::new(),
            fees: Vec::new(),
            votes: Vec::new(),
            transactions: Vec::new(),
            six_cases: vec![
                CaseBalance { name: "Treasury".to_string(), balance: 31_120_000_000 },
                CaseBalance { name: "Fees".to_string(), balance: 2_240_000_000 },
                CaseBalance { name: "Rewards".to_string(), balance: 2_240_000_000 },
                CaseBalance { name: "Savings".to_string(), balance: 2_240_000_000 },
                CaseBalance { name: "Credit".to_string(), balance: 1_320_000_000 },
                CaseBalance { name: "Shield".to_string(), balance: 840_000_000 },
            ],
            ubi_claims: Vec::new(),
            proposals: Vec::new(),
            messages: Vec::new(),
            offers: Vec::new(),
            sessions: Vec::new(),
        }
    }
    /// Sauvegarde sur disque (JSON) "” désactivée par défaut, contrôlée par UQ_KEEP_JSON env
    fn save(&self) {
        if !self.persist_json { return; }
        if let Ok(data) = self.data.lock() {
            if let Ok(json_str) = serde_json::to_string_pretty(&*data) {
                let _ = fs::write(&self.file_path, json_str);
            }
        }
    }

    /// Initialise la base SQLite et crée les tables si elles n'existent pas
    fn init_sqlite(&self) -> Result<(), rusqlite::Error> {
        // use pooled connection
        let conn = self.sqlite_pool.get().map_err(|_e| rusqlite::Error::InvalidQuery)?;
        conn.execute_batch("PRAGMA journal_mode = WAL;
            CREATE TABLE IF NOT EXISTS accounts (
                id TEXT PRIMARY KEY,
                public_key BLOB,
                balance_bubbles INTEGER,
                balance_points INTEGER,
                kyc_verified INTEGER,
                created_at INTEGER
            );
            CREATE TABLE IF NOT EXISTS transactions (
                id TEXT PRIMARY KEY,
                sender TEXT,
                recipient TEXT,
                amount_points INTEGER,
                fee_points INTEGER,
                timestamp INTEGER
            );
            CREATE TABLE IF NOT EXISTS publications (
                id TEXT PRIMARY KEY,
                json TEXT
            );
            CREATE TABLE IF NOT EXISTS offers (
                id TEXT PRIMARY KEY,
                seller TEXT,
                description TEXT,
                price_per_mb INTEGER,
                total_size_mb INTEGER,
                active INTEGER,
                created_at INTEGER
            );
            CREATE TABLE IF NOT EXISTS ubi_claims (
                account_id TEXT PRIMARY KEY,
                last_claim_at INTEGER,
                streak_days INTEGER,
                total_claimed INTEGER
            );
            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                account_id TEXT,
                token TEXT,
                created_at INTEGER,
                expires_at INTEGER
            );

            CREATE TABLE IF NOT EXISTS journals (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id TEXT NOT NULL,
                tx_type TEXT NOT NULL,
                amount_points INTEGER NOT NULL,
                fee_points INTEGER DEFAULT 0,
                counterparty TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX IF NOT EXISTS idx_journals_account ON journals (account_id);

        CREATE TABLE IF NOT EXISTS shop_items (
            id TEXT PRIMARY KEY,
            shop_id TEXT NOT NULL,
            item_type TEXT NOT NULL,
            media_urls TEXT NOT NULL DEFAULT '[]',
            title TEXT NOT NULL,
            subtitle TEXT DEFAULT '',
            description TEXT DEFAULT '',
            price_borb INTEGER NOT NULL DEFAULT 0,
            price_usd REAL NOT NULL DEFAULT 0.0,
            category TEXT DEFAULT 'general',
            commission_enabled INTEGER NOT NULL DEFAULT 0,
            commission_rate INTEGER NOT NULL DEFAULT 0,
            rating REAL NOT NULL DEFAULT 0.0,
            review_count INTEGER NOT NULL DEFAULT 0,
            comment_count INTEGER NOT NULL DEFAULT 0,
            share_count INTEGER NOT NULL DEFAULT 0,
            published INTEGER NOT NULL DEFAULT 0,
            sold INTEGER NOT NULL DEFAULT 0,
            sold_at TEXT,
            edit_count INTEGER NOT NULL DEFAULT 0,
            edited_at TEXT,
            expires_at TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            published_at TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_shop_items_shop ON shop_items(shop_id);
        CREATE INDEX IF NOT EXISTS idx_shop_items_published ON shop_items(published);

        CREATE TABLE IF NOT EXISTS item_agents (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            item_id TEXT NOT NULL,
            user_id TEXT NOT NULL,
            commission_rate INTEGER NOT NULL DEFAULT 0,
            sales_count INTEGER NOT NULL DEFAULT 0,
            total_earned INTEGER NOT NULL DEFAULT 0,
            joined_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(item_id, user_id)
        );

        CREATE INDEX IF NOT EXISTS idx_item_agents_item ON item_agents(item_id);
        CREATE INDEX IF NOT EXISTS idx_item_agents_user ON item_agents(user_id);

        CREATE TABLE IF NOT EXISTS item_interactions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            item_id TEXT NOT NULL,
            user_id TEXT NOT NULL,
            type TEXT NOT NULL,
            data TEXT DEFAULT '{}',
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE INDEX IF NOT EXISTS idx_item_interactions_item ON item_interactions(item_id);
        CREATE INDEX IF NOT EXISTS idx_item_interactions_user ON item_interactions(user_id);

        CREATE TABLE IF NOT EXISTS publication_likes (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            publication_id TEXT NOT NULL,
            uq_liker TEXT NOT NULL,
            uq_auteur TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            UNIQUE(publication_id, uq_liker)
        );
        CREATE INDEX IF NOT EXISTS idx_publikes_auteur ON publication_likes(uq_auteur);

        CREATE TABLE IF NOT EXISTS publication_views (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            publication_id TEXT NOT NULL,
            uq_viewer TEXT,
            uq_auteur TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_pubviews_auteur ON publication_views(uq_auteur);

        CREATE TABLE IF NOT EXISTS followers (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            uq_follower TEXT NOT NULL,
            uq_followed TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            UNIQUE(uq_follower, uq_followed)
        );
        CREATE INDEX IF NOT EXISTS idx_followers_followed ON followers(uq_followed);
        CREATE INDEX IF NOT EXISTS idx_followers_follower ON followers(uq_follower);

        CREATE TABLE IF NOT EXISTS profile_ratings (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            uq_note TEXT NOT NULL,
            uq_noteur TEXT NOT NULL,
            note INTEGER NOT NULL CHECK(note BETWEEN 1 AND 5),
            created_at INTEGER NOT NULL,
            UNIQUE(uq_note, uq_noteur)
        );
        CREATE INDEX IF NOT EXISTS idx_ratings_note ON profile_ratings(uq_note);


        CREATE TABLE IF NOT EXISTS contact_requests (
            id TEXT PRIMARY KEY,
            requester_id TEXT NOT NULL,
            target_id TEXT NOT NULL,
            field TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'pending',
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            responded_at TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_contact_requests_target ON contact_requests(target_id);
        CREATE INDEX IF NOT EXISTS idx_contact_requests_requester ON contact_requests(requester_id);

        CREATE TABLE IF NOT EXISTS shared_contacts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            owner_id TEXT NOT NULL,
            shared_with_id TEXT NOT NULL,
            field TEXT NOT NULL,
            shared_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(owner_id, shared_with_id, field)
        );

        CREATE INDEX IF NOT EXISTS idx_shared_contacts_owner ON shared_contacts(owner_id);
        CREATE INDEX IF NOT EXISTS idx_shared_contacts_target ON shared_contacts(shared_with_id);


        CREATE TABLE IF NOT EXISTS drafts (
            id TEXT PRIMARY KEY,
            user_id TEXT NOT NULL,
            draft_type TEXT NOT NULL,
            data TEXT NOT NULL DEFAULT '{}',
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS custom_types (
            id TEXT PRIMARY KEY,
            owner_id TEXT NOT NULL,
            name TEXT NOT NULL,
            emoji TEXT DEFAULT '',
            color TEXT DEFAULT '#4A9EFF',
            description TEXT DEFAULT '',
            fields TEXT DEFAULT '[]',
            created_at INTEGER NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_custom_types_owner ON custom_types(owner_id);

        CREATE TABLE IF NOT EXISTS chat_messages (
            id TEXT PRIMARY KEY,
            sender TEXT NOT NULL,
            recipient TEXT NOT NULL,
            content TEXT NOT NULL DEFAULT '',
            image TEXT,
            delivered INTEGER NOT NULL DEFAULT 0,
            timestamp INTEGER NOT NULL,
            expires_at INTEGER
        );

        CREATE INDEX IF NOT EXISTS idx_chat_messages_conversation
            ON chat_messages (sender, recipient, timestamp);

        -- Une annonce de revente : identite, position, mesure du lien. Aucun
        -- prix n'y est jamais ecrit — le tarif par seconde reste sur l'appareil
        -- du vendeur.
        CREATE TABLE IF NOT EXISTS connexion_offers (
            id TEXT PRIMARY KEY,
            owner TEXT NOT NULL,
            json TEXT NOT NULL,
            published INTEGER NOT NULL DEFAULT 0,
            updated_at INTEGER NOT NULL DEFAULT 0
        );

        CREATE INDEX IF NOT EXISTS idx_connexion_offers_owner
            ON connexion_offers (owner);
        CREATE INDEX IF NOT EXISTS idx_connexion_offers_published
            ON connexion_offers (published);

        -- Un avis sur une fiche de connexion. La clef est double : un compte
        -- ne dit qu'une fois ce qu'il pense d'une fiche, et il peut la
        -- reprendre sans empiler un second avis. Le compteur est donc pose
        -- compte par compte, et non gonfle par repetition.
        CREATE TABLE IF NOT EXISTS connexion_avis (
            annonce_id TEXT NOT NULL,
            uq_noteur TEXT NOT NULL,
            note INTEGER NOT NULL CHECK(note BETWEEN 1 AND 5),
            texte TEXT NOT NULL DEFAULT '',
            created_at INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (annonce_id, uq_noteur)
        );

        CREATE INDEX IF NOT EXISTS idx_connexion_avis_annonce
            ON connexion_avis (annonce_id);

        CREATE INDEX IF NOT EXISTS idx_drafts_user ON drafts(user_id);
            CREATE INDEX IF NOT EXISTS idx_accounts_id ON accounts (id);
            CREATE INDEX IF NOT EXISTS idx_transactions_sender ON transactions (sender);
            CREATE INDEX IF NOT EXISTS idx_transactions_recipient ON transactions (recipient);
            CREATE INDEX IF NOT EXISTS idx_offers_created_at ON offers (created_at);
        ")?;
        Ok(())
    }

    /// Importe les données JSON existantes dans SQLite si les tables sont vides
    fn migrate_json_to_sqlite(&self) -> Result<(), Box<dyn std::error::Error>> {
        // use pooled connection to avoid opening raw connections repeatedly
        let mut conn = match self.sqlite_pool.get() {
            Ok(c) => c,
            Err(e) => return Err(Box::new(e)),
        };

        // Charger le JSON en mémoire
        let data = self.data.lock().unwrap().clone();

        let tx = conn.transaction()?;
        // accounts: importer si table vide
        let accounts_count: i64 = tx.query_row("SELECT COUNT(1) FROM accounts", [], |r| r.get(0)).unwrap_or(0);
        if accounts_count == 0 {
            for acc in data.accounts.iter() {
                let _ = tx.execute(
                    "INSERT OR IGNORE INTO accounts (id, public_key, balance_bubbles, balance_points, kyc_verified, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    rusqlite::params![acc.id, acc.public_key.as_slice(), acc.balance_bubbles as i64, acc.balance_points as i64, if acc.kyc_verified {1i64} else {0i64}, acc.created_at]
                );
            }
        }
        // transactions: importer si table vide
        let tx_count: i64 = tx.query_row("SELECT COUNT(1) FROM transactions", [], |r| r.get(0)).unwrap_or(0);
        if tx_count == 0 {
            for tr in data.transactions.iter() {
                let _ = tx.execute(
                    "INSERT OR IGNORE INTO transactions (id, sender, recipient, amount_points, fee_points, timestamp) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    rusqlite::params![tr.id, tr.sender, tr.recipient, tr.amount_points as i64, tr.fee_points as i64, tr.timestamp]
                );
            }
        }
        // publications: importer si table vide
        let pub_count: i64 = tx.query_row("SELECT COUNT(1) FROM publications", [], |r| r.get(0)).unwrap_or(0);
        if pub_count == 0 {
            for p in data.publications.iter() {
                let id = p.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()).unwrap_or_else(|| format!("pub_{}", chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)));
                let json_str = serde_json::to_string(p)?;
                let _ = tx.execute(
                    "INSERT OR IGNORE INTO publications (id, json) VALUES (?1, ?2)",
                    rusqlite::params![id, json_str]
                );
            }
        }
        // offers: importer si table vide
        let offers_count: i64 = tx.query_row("SELECT COUNT(1) FROM offers", [], |r| r.get(0)).unwrap_or(0);
        if offers_count == 0 {
            for o in data.offers.iter() {
                let _ = tx.execute(
                    "INSERT OR IGNORE INTO offers (id, seller, description, price_per_mb, total_size_mb, active, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    rusqlite::params![o.id, o.seller, o.description, o.price_per_mb as i64, o.total_size_mb as i64, if o.active {1i64} else {0i64}, o.created_at]
                );
            }
        }
        // ubi_claims: importer si table vide
        let ubi_count: i64 = tx.query_row("SELECT COUNT(1) FROM ubi_claims", [], |r| r.get(0)).unwrap_or(0);
        if ubi_count == 0 {
            for u in data.ubi_claims.iter() {
                let _ = tx.execute(
                    "INSERT OR IGNORE INTO ubi_claims (account_id, last_claim_at, streak_days, total_claimed) VALUES (?1, ?2, ?3, ?4)",
                    rusqlite::params![u.account_id, u.last_claim_at, u.streak_days as i64, u.total_claimed as i64]
                );
            }
        }
        // sessions: importer si table vide
        let sess_count: i64 = tx.query_row("SELECT COUNT(1) FROM sessions", [], |r| r.get(0)).unwrap_or(0);
        if sess_count == 0 {
            for s in data.sessions.iter() {
                let _ = tx.execute(
                    "INSERT OR IGNORE INTO sessions (id, account_id, token, created_at, expires_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![s.id, s.account_id, s.token, s.created_at, s.expires_at]
                );
            }
        }
        // chat_messages: importer si table vide (historique d'un JSON local)
        let messages_count: i64 = tx.query_row("SELECT COUNT(1) FROM chat_messages", [], |r| r.get(0)).unwrap_or(0);
        if messages_count == 0 {
            for m in data.messages.iter() {
                let _ = tx.execute(
                    "INSERT OR IGNORE INTO chat_messages
                     (id, sender, recipient, content, image, delivered, timestamp, expires_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    rusqlite::params![m.id, m.sender, m.recipient, m.content, m.image,
                        if m.delivered { 1i64 } else { 0i64 }, m.timestamp, m.expires_at]
                );
            }
        }
        tx.commit()?;
        Ok(())
    }
    /// Crée un compte (JSON + SQLite)
    pub fn create_account(&self, id: &str, public_key: &[u8]) {
        // Insert into JSON in-memory structure
        {
            let mut data = self.data.lock().unwrap();
            if !data.accounts.iter().any(|a| a.id == id) {
                data.accounts.push(Account {
                    id: id.to_string(),
                    public_key: public_key.to_vec(),
                    balance_bubbles: 0,
                    balance_points: 0,
                    kyc_verified: false,
                    created_at: chrono::Utc::now().timestamp(),
                });
            }
        }
        // Persist to sqlite via pool
        if let Ok(conn) = self.sqlite_pool.get() {
            let _ = conn.execute(
                "INSERT OR IGNORE INTO accounts (id, public_key, balance_bubbles, balance_points, kyc_verified, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![id, public_key, 0i64, 0i64, 0i64, chrono::Utc::now().timestamp()]
            );
        }
        self.save();
    }
    /// Récupère le solde d'un compte (préférence SQLite)
    pub fn get_balance(&self, id: &str) -> (u64, u64) {
        if let Ok(conn) = self.sqlite_pool.get() {
            let mut stmt = match conn.prepare("SELECT balance_bubbles, balance_points FROM accounts WHERE id = ?1") {
                Ok(s) => s,
                Err(_) => return (0,0),
            };
            let mut rows = stmt.query(rusqlite::params![id]).unwrap();
            if let Ok(Some(row)) = rows.next() {
                let b: i64 = row.get(0).unwrap_or(0);
                let p: i64 = row.get(1).unwrap_or(0);
                return (b as u64, p as u64);
            }
        }
        // Fallback to in-memory JSON
        let data = self.data.lock().unwrap();
        data.accounts.iter()
            .find(|a| a.id == id)
            .map(|a| (a.balance_bubbles, a.balance_points))
            .unwrap_or((0, 0))
    }
    /// Enregistre une transaction (JSON + SQLite)
    pub fn record_transaction(&self, sender: &str, recipient: &str, amount_points: u64, fee_points: u64) {
        // First, append to WAL (durable append-only source)
        let payload = serde_json::json!({
            "sender": sender,
            "recipient": recipient,
            "amount_points": amount_points,
            "fee_points": fee_points,
            "timestamp": chrono::Utc::now().timestamp(),
        });
        match self.wal.append("tx", payload.clone()) {
            Ok(_entry) => {
                // proceed to update in-memory state
                let mut data = self.data.lock().unwrap();
                let id = format!("tx_{}", chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0));
                data.transactions.push(TransactionRecord {
                    id: id.clone(),
                    sender: sender.to_string(),
                    recipient: recipient.to_string(),
                    amount_points,
                    fee_points,
                    timestamp: chrono::Utc::now().timestamp(),
                });
                // update balances in memory (simple deduct/credit if accounts exist)
                if let Some(s) = data.accounts.iter_mut().find(|a| a.id == sender) {
                    s.balance_points = s.balance_points.saturating_sub(amount_points);
                }
                if let Some(r) = data.accounts.iter_mut().find(|a| a.id == recipient) {
                    r.balance_points = r.balance_points.saturating_add(amount_points);
                }
                drop(data);

                // Also persist to SQLite (best-effort, checkpointing async via snapshots)
                if let Ok(conn) = self.sqlite_pool.get() {
                    let id = format!("tx_{}", chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0));
                    let _ = conn.execute(
                        "INSERT INTO transactions (id, sender, recipient, amount_points, fee_points, timestamp) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                        rusqlite::params![id, sender, recipient, amount_points as i64, fee_points as i64, chrono::Utc::now().timestamp()],



                    );
                }

                // Optionally persist JSON file
                self.save();
                // increment counter used by snapshot worker
                self.tx_since_snapshot.fetch_add(1, Ordering::SeqCst);
            }
            Err(e) => {
                eprintln!("Failed to append to WAL: {}", e);
                // Do not apply transaction to memory if WAL append fails "” WAL is source of truth
            }
        }
    }
    /// Recupere les statistiques des six coffres
    pub fn get_cases_stats(&self) -> Vec<(String, u64)> {
        let data = self.data.lock().unwrap();
        data.six_cases.iter().map(|c| (c.name.clone(), c.balance)).collect()
    }
    /// Enregistre la reclamation UBI
    pub fn claim_ubi(&self, account_id: &str, amount_points: u64) {
        {
            let mut data = self.data.lock().unwrap();
            let now = chrono::Utc::now().timestamp();
            if let Some(claim) = data.ubi_claims.iter_mut().find(|c| c.account_id == account_id) {
                claim.last_claim_at = now;
                claim.streak_days += 1;
                claim.total_claimed += amount_points;
            } else {
                data.ubi_claims.push(UbiClaim {
                    account_id: account_id.to_string(),
                    last_claim_at: now,
                    streak_days: 1,
                    total_claimed: amount_points,
                });
            }
        }
        self.save();
    }
    /// Cree une proposition DAO
    pub fn create_proposal(&self, title: &str, description: &str, proposer: &str) {
        {
            let mut data = self.data.lock().unwrap();
            let id = format!("prop_{}", chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0));
            data.proposals.push(Proposal {
                id,
                title: title.to_string(),
                description: description.to_string(),
                proposer: proposer.to_string(),
                votes_for: 0,
                votes_against: 0,
                status: "open".to_string(),
                created_at: chrono::Utc::now().timestamp(),
            });
        }
        self.save();
    }
    /// Envoie un message
    pub fn send_message(&self, sender: &str, recipient: &str, content: &str) -> String {
        self.send_message_avec_image(sender, recipient, content, None, false)
    }

    /// Envoie un message texte et/ou une photo (data URL gardee en local).
    /// `ephemere` = la conversation est en mode auto-destruction : le message
    /// porte sa date de fin. Sans ce mode, rien n'est purge.
    pub fn send_message_avec_image(
        &self,
        sender: &str,
        recipient: &str,
        content: &str,
        image: Option<&str>,
        ephemere: bool,
    ) -> String {
        self.purger_messages_expires();
        let id = format!("msg_{}", chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0));
        let maintenant = chrono::Utc::now().timestamp();
        let message = Message {
            id: id.clone(),
            sender: sender.to_string(),
            recipient: recipient.to_string(),
            content: content.to_string(),
            image: image.map(|value| value.to_string()),
            delivered: false,
            timestamp: maintenant,
            expires_at: ephemere.then_some(maintenant + MESSAGE_TTL_SECS),
        };
        {
            let mut data = self.data.lock().unwrap();
            data.messages.push(message.clone());
        }
        self.save();
        // Miroir SQLite : la liste JSON n'est pas ecrite sur disque quand
        // UQ_KEEP_JSON est faux, sans quoi l'historique mourrait au prochain
        // redemarrage du serveur.
        if let Ok(conn) = self.sqlite_pool.get() {
            let _ = conn.execute(
                "INSERT OR REPLACE INTO chat_messages
                 (id, sender, recipient, content, image, delivered, timestamp, expires_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![
                    message.id,
                    message.sender,
                    message.recipient,
                    message.content,
                    message.image,
                    message.delivered as i64,
                    message.timestamp,
                    message.expires_at
                ],
            );
        }
        id
    }

    /// Recharge l'historique depuis SQLite au demarrage. La memoire ne repart
    /// pas de zero : les fils, les non-lus et les accusés de lecture survivent
    /// au redemarrage du process.
    pub fn charger_messages_sqlite(&self) {
        let Ok(conn) = self.sqlite_pool.get() else { return };
        let sortie = conn.prepare(
            "SELECT id, sender, recipient, content, image, delivered, timestamp, expires_at
             FROM chat_messages WHERE expires_at IS NULL OR expires_at > ?1
             ORDER BY timestamp",
        ).and_then(|mut stmt| {
            stmt.query_map([chrono::Utc::now().timestamp()], |row| {
                Ok(Message {
                    id: row.get(0)?,
                    sender: row.get(1)?,
                    recipient: row.get(2)?,
                    content: row.get(3)?,
                    image: row.get(4)?,
                    delivered: row.get::<_, i64>(5)? != 0,
                    timestamp: row.get(6)?,
                    expires_at: row.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
        });
        let Ok(recharges) = sortie else { return };
        if recharges.is_empty() {
            return;
        }
        let mut data = self.data.lock().unwrap();
        // Le JSON de depart peut deja porter des messages : on ne double pas.
        let deja_vus = data.messages.iter().map(|m| m.id.clone()).collect::<Vec<_>>();
        for message in recharges {
            if !deja_vus.contains(&message.id) {
                data.messages.push(message);
            }
        }
    }

    /// Supprime les messages dont la fin de vie est passee. Les messages sans
    /// `expires_at` sont persistants : la purge ne les touche pas.
    pub fn purger_messages_expires(&self) {
        let maintenant = chrono::Utc::now().timestamp();
        let supprimes = {
            let mut data = self.data.lock().unwrap();
            let avant = data.messages.len();
            data
                .messages
                .retain(|m| m.expires_at.map_or(true, |fin| fin > maintenant));
            avant - data.messages.len()
        };
        if supprimes > 0 {
            self.save();
            if let Ok(conn) = self.sqlite_pool.get() {
                let _ = conn.execute(
                    "DELETE FROM chat_messages WHERE expires_at IS NOT NULL AND expires_at <= ?1",
                    [maintenant],
                );
            }
            eprintln!("[CHAT] {} message(s) ephemere(s) supprime(s)", supprimes);
        }
    }

    /// Messages recus et pas encore relus par leur destinataire.
    pub fn compter_non_lus(&self, user_id: &str) -> usize {
        let maintenant = chrono::Utc::now().timestamp();
        let data = self.data.lock().unwrap();
        data.messages
            .iter()
            .filter(|m| {
                m.recipient == user_id
                    && !m.delivered
                    && m.expires_at.map_or(true, |fin| fin > maintenant)
            })
            .count()
    }

    /// Paires (interlocuteur, message) encore en attente de lecture, pour le
    /// bandeau d'etat du chat.
    pub fn conversations_non_lues(&self, user_id: &str) -> usize {
        self.non_lus_par_pair(user_id).len()
    }

    /// Messages non lus par interlocuteur : l'inbox en a besoin pour son badge.
    pub fn non_lus_par_pair(&self, user_id: &str) -> std::collections::HashMap<String, usize> {
        let maintenant = chrono::Utc::now().timestamp();
        let data = self.data.lock().unwrap();
        let mut sortie = std::collections::HashMap::new();
        for m in data.messages.iter() {
            if m.recipient == user_id
                && !m.delivered
                && m.expires_at.map_or(true, |fin| fin > maintenant)
            {
                *sortie.entry(m.sender.clone()).or_insert(0) += 1;
            }
        }
        sortie
    }

    /// Marque comme lus les messages recus de `peer` par `lecteur`.
    pub fn marquer_lus(&self, lecteur: &str, peer: &str) -> usize {
        let modifies = {
            let mut data = self.data.lock().unwrap();
            let mut count = 0;
            for m in data.messages.iter_mut() {
                if m.sender == peer && m.recipient == lecteur && !m.delivered {
                    m.delivered = true;
                    count += 1;
                }
            }
            count
        };
        if modifies > 0 {
            self.save();
            if let Ok(conn) = self.sqlite_pool.get() {
                let _ = conn.execute(
                    "UPDATE chat_messages SET delivered = 1 WHERE sender = ?1 AND recipient = ?2",
                    rusqlite::params![peer, lecteur],
                );
            }
        }
        modifies
    }

    /// Messages entre deux utilisateurs (ordre chronologique)
    pub fn get_messages_between(&self, user_a: &str, user_b: &str) -> Vec<Message> {
        self.purger_messages_expires();
        let data = self.data.lock().unwrap();
        let mut msgs: Vec<Message> = data
            .messages
            .iter()
            .filter(|m| {
                (m.sender == user_a && m.recipient == user_b)
                    || (m.sender == user_b && m.recipient == user_a)
            })
            .cloned()
            .collect();
        msgs.sort_by_key(|m| m.timestamp);
        msgs
    }

    /// Liste des conversations (dernier message par interlocuteur)
    pub fn get_chat_inbox(&self, user_id: &str) -> Vec<serde_json::Value> {
        self.purger_messages_expires();
        let data = self.data.lock().unwrap();
        use std::collections::HashMap;
        let mut by_peer: HashMap<String, Message> = HashMap::new();

        for m in &data.messages {
            if m.sender != user_id && m.recipient != user_id {
                continue;
            }
            let peer = if m.sender == user_id {
                m.recipient.clone()
            } else {
                m.sender.clone()
            };
            let replace = match by_peer.get(&peer) {
                Some(prev) => m.timestamp > prev.timestamp,
                None => true,
            };
            if replace {
                by_peer.insert(peer, m.clone());
            }
        }

        let mut out: Vec<(i64, serde_json::Value)> = by_peer
            .into_iter()
            .map(|(peer, msg)| {
                // Une photo seule n'a pas de texte : l'apercu reste lisible.
                let apercu = if msg.content.trim().is_empty() && msg.image.is_some() {
                    "Photo".to_string()
                } else {
                    msg.content.clone()
                };
                (
                    msg.timestamp,
                    serde_json::json!({
                        "peer_id": peer,
                        "last_message": apercu,
                        "timestamp": msg.timestamp,
                        "unread": !msg.delivered && msg.recipient == user_id,
                    }),
                )
            })
            .collect();
        out.sort_by(|a, b| b.0.cmp(&a.0));
        out.into_iter().map(|(_, v)| v).collect()
    }
    /// Publie une offre (JSON + SQLite)
    pub fn publish_offer(&self, seller: &str, description: &str, price_per_mb: u64, total_size_mb: u64) {
        let id = format!("offer_{}", chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0));
        // JSON
        {
            let mut data = self.data.lock().unwrap();
            data.offers.push(Offer {
                id: id.clone(),
                seller: seller.to_string(),
                description: description.to_string(),
                price_per_mb,
                total_size_mb,
                active: true,
                created_at: chrono::Utc::now().timestamp(),
            });
        }
        // SQLite via pool
        if let Ok(conn) = self.sqlite_pool.get() {
            let _ = conn.execute(
                "INSERT OR REPLACE INTO offers (id, seller, description, price_per_mb, total_size_mb, active, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![id, seller, description, price_per_mb as i64, total_size_mb as i64, 1i64, chrono::Utc::now().timestamp()]
            );
        }
        self.save();
    }
    /// Crée une session
    pub fn create_session(&self, account_id: &str, token: &str, expires_at: i64) {
        {
            let mut data = self.data.lock().unwrap();
            let id = format!("sess_{}", chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0));
            data.sessions.push(Session {
                id,
                account_id: account_id.to_string(),
                token: token.to_string(),
                created_at: chrono::Utc::now().timestamp(),
                expires_at,
            });
        }
        // also persist session to sqlite via pool
        if let Ok(conn) = self.sqlite_pool.get() {
            let id = format!("sess_{}", chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0));
            let _ = conn.execute(
                "INSERT OR REPLACE INTO sessions (id, account_id, token, created_at, expires_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![id, account_id, token, chrono::Utc::now().timestamp(), expires_at]
            );
        }
        self.save();
    }
    /// Récupère les statistiques générales
    pub fn get_stats(&self) -> serde_json::Value {
        let data = self.data.lock().unwrap();
        serde_json::json!({
            "accounts": data.accounts.len(),
            "transactions": data.transactions.len(),
            "proposals": data.proposals.len(),
            "messages": data.messages.len(),
            "offers": data.offers.len(),
            "sessions": data.sessions.len(),
            "ubi_claims": data.ubi_claims.len()
        })
    }
    /// Vérifie que les 6 cases totalisent bien TOTAL_BORB
    pub fn verify_total_invariant(&self) -> bool {
        let data = self.data.lock().unwrap();
        let total: u128 = data.six_cases.iter().map(|c| c.balance as u128).sum();
        total == TOTAL_BORB as u128
    }
    // === MÉTHODES DE PERSISTANCE ===
    /// Sauvegarde une publication (JSON + SQLite)
    pub fn save_publication(&self, pub_data: serde_json::Value) {
        // Add to in-memory JSON
        let id = pub_data.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()).unwrap_or_else(|| format!("pub_{}", chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)));
        {
            let mut data = self.data.lock().unwrap();
            data.publications.push(pub_data.clone());
        }
        // Persist in sqlite via pool
        if let Ok(conn) = self.sqlite_pool.get() {
            let _ = conn.execute(
                "INSERT OR REPLACE INTO publications (id, json) VALUES (?1, ?2)",
                rusqlite::params![&id, serde_json::to_string(&pub_data).unwrap_or_default()]
            );
        }
        self.save();
    }
    /// Sauvegarde un interet
    pub fn save_interest(&self, interest_data: serde_json::Value) {
        {
            let mut data = self.data.lock().unwrap();
            data.interests.push(interest_data);
        }
        self.save();
    }
    /// Sauvegarde une reclamation d'assurance
    pub fn save_claim(&self, claim_data: serde_json::Value) {
        {
            let mut data = self.data.lock().unwrap();
            data.claims.push(claim_data);
        }
        self.save();
    }
    /// Sauvegarde un pret
    pub fn save_loan(&self, loan_data: serde_json::Value) {
        {
            let mut data = self.data.lock().unwrap();
            data.loans.push(loan_data);
        }
        self.save();
    }
    /// Sauvegarde un ordre d'echange
    pub fn save_exchange_order(&self, order_data: serde_json::Value) {
        {
            let mut data = self.data.lock().unwrap();
            data.exchange_orders.push(order_data);
        }
        self.save();
    }
    /// Sauvegarde une sanction
    pub fn save_sanction(&self, sanction_data: serde_json::Value) {
        {
            let mut data = self.data.lock().unwrap();
            data.sanctions.push(sanction_data);
        }
        self.save();
    }
    /// Sauvegarde un Marketer
    pub fn save_marketer(&self, marketer_data: serde_json::Value) {
        {
            let mut data = self.data.lock().unwrap();
            data.marketers.push(marketer_data);
        }
        self.save();
    }
    /// Sauvegarde un enregistrement KYC
    pub fn save_kyc(&self, kyc_data: serde_json::Value) {
        {
            let mut data = self.data.lock().unwrap();
            data.kyc_records.push(kyc_data);
        }
        self.save();
    }
    /// Sauvegarde une transaction hors ligne
    pub fn save_offline_tx(&self, tx_data: serde_json::Value) {
        {
            let mut data = self.data.lock().unwrap();
            data.offline_txs.push(tx_data);
        }
        self.save();
    }
    /// Sauvegarde un fragment
    pub fn save_fragment(&self, fragment_data: serde_json::Value) {
        {
            let mut data = self.data.lock().unwrap();
            data.fragments.push(fragment_data);
        }
        self.save();
    }
    /// Sauvegarde une session de lecture
    pub fn save_player_session(&self, session_data: serde_json::Value) {
        {
            let mut data = self.data.lock().unwrap();
            data.player_sessions.push(session_data);
        }
        self.save();
    }
    /// Sauvegarde une connexion
    pub fn save_connection(&self, connection_data: serde_json::Value) {
        {
            let mut data = self.data.lock().unwrap();
            data.connections.push(connection_data);
        }
        self.save();
    }
    /// Sauvegarde une bulle
    pub fn save_bubble(&self, bubble_data: serde_json::Value) {
        {
            let mut data = self.data.lock().unwrap();
            data.bubbles.push(bubble_data);
        }
        self.save();
    }
    /// Sauvegarde des frais collectes
    pub fn save_fee(&self, fee_data: serde_json::Value) {
        {
            let mut data = self.data.lock().unwrap();
            data.fees.push(fee_data);
        }
        self.save();
    }
    /// Sauvegarde un vote
    pub fn save_vote(&self, vote_data: serde_json::Value) {
        {
            let mut data = self.data.lock().unwrap();
            data.votes.push(vote_data);
        }
        self.save();
    }
    /// Récupère toutes les offres (préférence SQLite)
    pub fn get_all_offers(&self) -> Vec<Offer> {
        if let Ok(conn) = rusqlite::Connection::open(&self.sqlite_path) {
            let mut stmt = match conn.prepare("SELECT id, seller, description, price_per_mb, total_size_mb, active, created_at FROM offers ORDER BY created_at DESC") {
                Ok(s) => s,
                Err(_) => return Vec::new(),
            };
            let rows = stmt.query_map([], |row| {
                Ok(Offer {
                    id: row.get(0)?,
                    seller: row.get(1)?,
                    description: row.get(2)?,
                    price_per_mb: row.get::<_, i64>(3)? as u64,
                    total_size_mb: row.get::<_, i64>(4)? as u64,
                    active: row.get::<_, i64>(5)? != 0,
                    created_at: row.get(6)?,
                })
            });
            if let Ok(iter) = rows {
                return iter.filter_map(Result::ok).collect();
            }
        }
        // fallback to JSON
        let data = self.data.lock().unwrap();
        data.offers.clone()
    }
    /// Récupère toutes les boutiques (préférence SQLite)
    pub fn get_all_shops(&self) -> Vec<serde_json::Value> {
        if let Ok(conn) = rusqlite::Connection::open(&self.sqlite_path) {
            let mut stmt = match conn.prepare("SELECT json FROM publications ORDER BY rowid DESC") {
                Ok(s) => s,
                Err(_) => return Vec::new(),
            };
            let rows = stmt.query_map([], |row| {
                let s: String = row.get(0)?;
                let v: serde_json::Value = serde_json::from_str(&s).unwrap_or(serde_json::json!({}));
                Ok(v)
            });
            if let Ok(iter) = rows {
                return iter.filter_map(Result::ok).collect();
            }
        }
        let data = self.data.lock().unwrap();
        data.publications.iter().map(|p| serde_json::json!(p)).collect()
    }

    /// Récupère le dashboard d'un utilisateur
    pub fn get_dashboard(&self, user_id: &str) -> serde_json::Value {
        let data = self.data.lock().unwrap();
        let tx_count = data.transactions.iter().filter(|t| t.sender == user_id || t.recipient == user_id).count();
        let msg_count = data.messages.iter().filter(|m| m.recipient == user_id && !m.delivered).count();
        let offers_count = data.offers.iter().filter(|o| o.seller == user_id && o.active).count();
        
        serde_json::json!({
            "user_id": user_id,
            "stats": {
                "balance_bubbles": 0,
                "balance_points": 0,
                "value_usd": 0.0,
                "transactions_count": tx_count,
                "messages_unread": msg_count,
                "offers_active": offers_count,
                "referral_count": 0,
                "referral_earnings": 0,
                "ubi_claimed_today": false,
                "ubi_total_claimed": 0
            }
        })
    }

    /// Sauvegarde une notification
    pub fn save_notification(&self, user_id: &str, content: &str) {
        let mut data = self.data.lock().unwrap();
        let notif = serde_json::json!({
            "id": format!("notif_{}", chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)),
            "user_id": user_id,
            "content": content,
            "read": false,
            "created_at": chrono::Utc::now().timestamp()
        });
        data.notifications.push(notif);
        drop(data);
        self.save();
    }

    /// Récupère les notifications d'un utilisateur
    pub fn get_notifications(&self, user_id: &str) -> Vec<serde_json::Value> {
        let data = self.data.lock().unwrap();
        data.notifications.iter()
            .filter(|n| n["user_id"] == user_id)
            .cloned()
            .collect()
    }
    // ------------------------------------------------------------------------
    // MÉTHODES POUR LE RECYCLAGE DES INACTIFS
    // ------------------------------------------------------------------------
    
    /// Récupère le timestamp de dernière activité
    pub fn get_last_activity(&self, account_id: &str) -> u64 {
        let data = self.data.lock().unwrap();
        data.accounts.iter()
            .find(|a| a.id == account_id)
            .map(|a| a.created_at as u64)
            .unwrap_or(0)
    }
    
    /// Met a jour le timestamp de derniere activite
    pub fn update_last_activity(&self, account_id: &str, timestamp: u64) {
        let mut data = self.data.lock().unwrap();
        if let Some(account) = data.accounts.iter_mut().find(|a| a.id == account_id) {
            // last_activity_date n'existe pas "” on utilise created_at
        }
        drop(data);
        self.save();
    }
    
    /// Recycle un compte inactif (5/10/50 ans)
    pub fn recycle_inactive(&self, account_id: &str, years_inactive: u64) -> Result<u128, String> {
        let mut data = self.data.lock().unwrap();
        let amount = data.accounts.iter()
            .find(|a| a.id == account_id)
            .map(|a| a.balance_points as u128)
            .unwrap_or(0);
        
        let recycle = match years_inactive {
            0..=5 => 0,
            5..=10 => amount * (years_inactive - 5) as u128 / 1000,
            10..=50 => amount / 2,
            _ => amount,
        };
        
        if recycle > 0 {
            // Transférer au Trésor
            if let Some(case) = data.six_cases.iter_mut().find(|c| c.name == "Treasury") {
                case.balance = case.balance.saturating_add(recycle as u64);
            }
            // Réduire le solde du compte
            if let Some(account) = data.accounts.iter_mut().find(|a| a.id == account_id) {
                account.balance_points = account.balance_points.saturating_sub(recycle as u64);
            }
        }
        
        drop(data);
        self.save();
        Ok(recycle)
    }
    
    /// Récupère les comptes inactifs depuis X années
    pub fn get_inactive_accounts(&self, years: u64) -> Vec<String> {
        let data = self.data.lock().unwrap();
        let now = chrono::Utc::now().timestamp();
        data.accounts.iter()
            .filter(|a| {
                let last = a.created_at;
                (now - last) > (years as i64 * 365 * 24 * 3600)
            })
            .map(|a| a.id.clone())
            .collect()
    }
    
    // ------------------------------------------------------------------------
    // MÉTHODES POUR L'ÉPARGNE
    // ------------------------------------------------------------------------
    
    /// Récupère les intérêts du crédit sur la dernière année
    pub fn get_credit_interests_last_year(&self) -> u128 {
        let data = self.data.lock().unwrap();
        let now = chrono::Utc::now().timestamp();
        let one_year_ago = now - 365 * 24 * 3600;
        
        data.transactions.iter()
            .filter(|tx| tx.timestamp > one_year_ago)
            .map(|tx| (tx.amount_points as u128) * 5 / 100)  // 5% d'intérêt estimé
            .sum()
    }
    
    /// Sauvegarde un contrat d'épargne
    pub fn save_contract(&self, contract_id: &str, account_id: &str, amount: u128, start: i64, end: i64) {
        let mut data = self.data.lock().unwrap();
        let contract = serde_json::json!({
            "id": contract_id,
            "account_id": account_id,
            "amount": amount,
            "start_time": start,
            "end_time": end,
            "active": true
        });
        data.savings_contracts.push(contract);
        drop(data);
        self.save();
    }
    
    /// Compte les contrats actifs
    pub fn get_contract_count(&self) -> u64 {
        let data = self.data.lock().unwrap();
        data.savings_contracts.iter()
            .filter(|c| c["active"] == true)
            .count() as u64
    }
    
    /// Distribue les rendements aux épargnants
    pub fn distribute_savings_yield(&self, total_yield: u128) {
        let mut data = self.data.lock().unwrap();
        let contract_count = data.savings_contracts.len() as u128;
        if contract_count > 0 {
            let per_contract = total_yield / contract_count;
            for contract in data.savings_contracts.iter_mut() {
                if let Some(amount) = contract["amount"].as_u64() {
                    contract["amount"] = serde_json::json!(amount as u128 + per_contract);
                }
            }
        }
        drop(data);
        self.save();
    }
    
    // ------------------------------------------------------------------------
    // MÉTHODES POUR LE CRÉDIT (BORB NÉGATIF)
    // ------------------------------------------------------------------------
    
    /// Nantit les épargnes comme garantie
    pub fn nantir_savings(&self, account_id: &str, amount: u128) {
        let mut data = self.data.lock().unwrap();
        let entry = serde_json::json!({
            "account_id": account_id,
            "amount": amount,
            "timestamp": chrono::Utc::now().timestamp()
        });
        data.nanties.push(entry);
        drop(data);
        self.save();
    }
    
    /// Enregistre une dette
    pub fn set_debt(&self, account_id: &str, amount: u128) {
        let mut data = self.data.lock().unwrap();
        let debt = serde_json::json!({
            "account_id": account_id,
            "amount": amount,
            "created_at": chrono::Utc::now().timestamp()
        });
        data.debts.push(debt);
        drop(data);
        self.save();
    }
    
    /// Récupère la dette d'un compte
    pub fn get_debt(&self, account_id: &str) -> u128 {
        let data = self.data.lock().unwrap();
        data.debts.iter()
            .filter(|d| d["account_id"] == account_id)
            .map(|d| d["amount"].as_u64().unwrap_or(0) as u128)
            .sum()
    }
    
    /// Récupère les garanties nanties
    pub fn get_nanties(&self, account_id: &str) -> u128 {
        let data = self.data.lock().unwrap();
        data.nanties.iter()
            .filter(|n| n["account_id"] == account_id)
            .map(|n| n["amount"].as_u64().unwrap_or(0) as u128)
            .sum()
    }
    
    /// Transfère les garanties saisies au Crédit
    pub fn transfer_to_credit(&self, account_id: &str, amount: u128) {
        let mut data = self.data.lock().unwrap();
        if let Some(case) = data.six_cases.iter_mut().find(|c| c.name == "Credit") {
            case.balance = case.balance.saturating_add(amount as u64);
        }
        drop(data);
        self.save();
    }
    
    /// Total des dettes actives
    pub fn get_total_debt(&self) -> u128 {
        let data = self.data.lock().unwrap();
        data.debts.iter()
            .map(|d| d["amount"].as_u64().unwrap_or(0) as u128)
            .sum()
    }
    
    /// Nombre de prêteurs actifs
    pub fn get_lender_count(&self) -> u64 {
        let data = self.data.lock().unwrap();
        data.debts.len() as u64
    }
    
    // ------------------------------------------------------------------------
    // MÉTHODES POUR LE ROBINET 90/10
    // ------------------------------------------------------------------------
    
    /// Le créateur fixe son prix par seconde
    pub fn set_creator_price(&self, creator_id: &str, price_per_second: u128) {
        let mut data = self.data.lock().unwrap();
        let entry = serde_json::json!({
            "creator_id": creator_id,
            "price_per_second": price_per_second
        });
        data.creator_prices.push(entry);
        drop(data);
        self.save();
    }
    
    /// Paiement atomique du contenu (7 transferts)
    pub fn atomic_content_payment(
        &self,
        creator_id: &str,
        creator_amount: u128,
        treasury: u128, fees: u128, rewards: u128,
        savings: u128, credit: u128, shield: u128
    ) -> Result<(), String> {
        let mut data = self.data.lock().unwrap();
        let cases = [
            ("creator", creator_amount),
            ("Treasury", treasury),
            ("Fees", fees),
            ("Rewards", rewards),
            ("Savings", savings),
            ("Credit", credit),
            ("Shield", shield),
        ];
        
        for (case_name, amount) in cases.iter() {
            if let Some(case) = data.six_cases.iter_mut().find(|c| c.name == *case_name) {
                case.balance = case.balance.saturating_add(*amount as u64);
            }
        }
        
        drop(data);
        self.save();
        Ok(())
    }
    
    /// Total streamé
    pub fn get_total_streamed(&self) -> u128 {
        let data = self.data.lock().unwrap();
        data.total_streamed
    }
    /// Récupère le journal individuel d'un compte
    pub fn get_journal(&self, account_id: &str) -> Vec<serde_json::Value> {
        let conn = match self.sqlite_pool.get() {
            Ok(c) => c,
            Err(_) => return vec![],
        };
        
        let mut stmt = match conn.prepare(
            "SELECT id, tx_type, amount_points, fee_points, counterparty, created_at 
             FROM journals WHERE account_id = ?1 ORDER BY created_at DESC LIMIT 1000"
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        
        let rows = match stmt.query_map(rusqlite::params![account_id], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, i64>(0)?,
                "tx_type": row.get::<_, String>(1)?,
                "amount_points": row.get::<_, i64>(2)?,
                "fee_points": row.get::<_, i64>(3)?,
                "counterparty": row.get::<_, Option<String>>(4)?,
                "created_at": row.get::<_, String>(5)?,
            }))
        }) {
            Ok(r) => r,
            Err(_) => return vec![],
        };
        
        let mut result = Vec::new();
        for row in rows {
            if let Ok(entry) = row {
                result.push(entry);
            }
        }
        result
    }
    /// Comptes vus dans les transactions locales (émetteur ou bénéficiaire)
    pub fn get_transaction_comptes(&self) -> Vec<String> {
        let data = self.data.lock().unwrap();
        let mut comptes: Vec<String> = data
            .transactions
            .iter()
            .flat_map(|t| [t.sender.clone(), t.recipient.clone()])
            .collect();
        comptes.sort();
        comptes.dedup();
        comptes
    }
    /// Transactions impliquant un compte, plus récentes d'abord
    pub fn get_transactions_compte(&self, uq: &str, limite: usize) -> Vec<TransactionRecord> {
        let data = self.data.lock().unwrap();
        let mut txs: Vec<TransactionRecord> = data
            .transactions
            .iter()
            .filter(|t| t.sender == uq || t.recipient == uq)
            .cloned()
            .collect();
        txs.sort_by_key(|t| std::cmp::Reverse(t.timestamp));
        txs.truncate(limite);
        txs
    }
    /// Dernières sanctions enregistrées, plus récentes d'abord
    pub fn get_sanctions(&self, limite: usize) -> Vec<serde_json::Value> {
        let data = self.data.lock().unwrap();
        let debut = data.sanctions.len().saturating_sub(limite);
        data.sanctions[debut..].iter().rev().cloned().collect()
    }

    /// Nombre de transactions reçues par compte, en une seule passe.
    pub fn compter_transactions_recues(&self) -> std::collections::HashMap<String, i64> {
        let data = self.data.lock().unwrap();
        let mut compteurs: std::collections::HashMap<String, i64> =
            std::collections::HashMap::new();
        for transaction in &data.transactions {
            *compteurs
                .entry(transaction.recipient.trim().to_uppercase())
                .or_insert(0) += 1;
        }
        compteurs
    }
    // ====================
    // 🛒 ZANDO — ÉLÉMENTS (6 fonctions)
    // ====================

    /// Crée un nouvel élément Zando
    pub fn create_shop_item(
        &self,
        shop_id: &str,
        item_type: &str,
        media_urls: &str,
        title: &str,
        subtitle: &str,
        description: &str,
        price_borb: u64,
        price_usd: f64,
        category: &str,
        commission_enabled: bool,
        commission_rate: u32,
    ) -> Result<String, String> {
        let conn = self.sqlite_pool.get().map_err(|error| error.to_string())?;
        
        let id = format!("item_{}", chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0));
        conn.execute(
            "INSERT INTO shop_items (id, shop_id, item_type, media_urls, title, subtitle, description, price_borb, price_usd, category, commission_enabled, commission_rate) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![id, shop_id, item_type, media_urls, title, subtitle, description, price_borb as i64, price_usd, category, if commission_enabled { 1 } else { 0 }, commission_rate as i64]
        ).map_err(|error| error.to_string())?;
        Ok(id)
    }

    /// Fait sortir du flux les ventes solo dont la fenetre de 48 h est passee.
    /// L'element reste dans le compte (`published = 0`) : le vendeur peut le
    /// republier et repartir pour 48 h. Une vitrine de boutique n'est jamais
    /// concernee : sa presence est geree par `synchroniser_vitrines_expirees`.
    pub fn synchroniser_elements_expires(&self) {
        let Ok(conn) = self.sqlite_pool.get() else { return };
        let _ = conn.execute(
            "UPDATE shop_items SET published = 0
              WHERE item_type != 'boutique'
                AND sold = 0
                AND published = 1
                AND expires_at IS NOT NULL
                AND expires_at <= datetime('now')",
            [],
        );
    }

    /// Supprime les BROUILLONS Zando (jamais publiés) de plus de 7 jours.
    /// Un element publie appartient au vendeur : il reste en ligne jusqu'a
    /// suppression explicite depuis le Studio. Une vitrine expiree passe
    /// `published = 0` mais reste dans le compte — jamais purgee.
    /// Les interactions et agents rattaches partent avec le brouillon.
    pub fn purger_elements_expires(&self) {
        let Ok(conn) = self.sqlite_pool.get() else { return };
        let cutoff = (chrono::Utc::now() - chrono::Duration::days(7))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        let mut ids: Vec<String> = Vec::new();
        if let Ok(mut stmt) = conn.prepare(
            "SELECT id FROM shop_items
             WHERE published = 0 AND published_at IS NULL AND created_at < ?1",
        ) {
            if let Ok(rows) = stmt.query_map(rusqlite::params![cutoff], |row| row.get::<_, String>(0)) {
                ids.extend(rows.flatten());
            }
        }
        for id in &ids {
            let _ = conn.execute(
                "DELETE FROM item_interactions WHERE item_id = ?1",
                rusqlite::params![id],
            );
            let _ = conn.execute(
                "DELETE FROM item_agents WHERE item_id = ?1",
                rusqlite::params![id],
            );
            let _ = conn.execute(
                "DELETE FROM shop_items WHERE id = ?1",
                rusqlite::params![id],
            );
        }
        if !ids.is_empty() {
            eprintln!("[ZANDO] {} brouillon(s) supprime(s) apres 7 jours", ids.len());
        }
    }

    /// Récupère tous les éléments d'une boutique
    /// Recupere TOUS les items Zando (sans filtre shop_id)
    pub fn get_all_shop_items(&self) -> Vec<serde_json::Value> {
        self.synchroniser_vitrines_expirees();
        self.synchroniser_elements_expires();
        self.purger_elements_expires();
        let conn = match self.sqlite_pool.get() {
            Ok(c) => c,
            Err(_) => return vec![],
        };
        
        let mut stmt = match conn.prepare(
            "SELECT id, shop_id, item_type, media_urls, title, subtitle, description, price_borb, price_usd, category, commission_enabled, commission_rate, rating, review_count, comment_count, share_count, published, sold, sold_at, edit_count, edited_at, expires_at, created_at, published_at FROM shop_items ORDER BY created_at DESC LIMIT 500"
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        
        let rows = match stmt.query_map([], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, String>(0)?,
                "shop_id": row.get::<_, String>(1)?,
                "type": row.get::<_, String>(2)?,
                "media_urls": row.get::<_, String>(3)?,
                "title": row.get::<_, String>(4)?,
                "subtitle": row.get::<_, String>(5).unwrap_or_default(),
                "description": row.get::<_, String>(6)?,
                "price_borb": row.get::<_, i64>(7)?,
                "price_usd": row.get::<_, f64>(8)?,
                "category": row.get::<_, String>(9)?,
                "commission_enabled": row.get::<_, i64>(10)? == 1,
                "commission_rate": row.get::<_, i64>(11)?,
                "rating": row.get::<_, f64>(12)?,
                "review_count": row.get::<_, i64>(13)?,
                "comment_count": row.get::<_, i64>(14)?,
                "share_count": row.get::<_, i64>(15)?,
                "published": row.get::<_, i64>(16)? == 1,
                "sold": row.get::<_, i64>(17).unwrap_or(0) == 1,
                "sold_at": row.get::<_, Option<String>>(18).unwrap_or_default(),
                "edit_count": row.get::<_, i64>(19).unwrap_or(0),
                "edited_at": row.get::<_, Option<String>>(20).unwrap_or_default(),
                "expires_at": row.get::<_, Option<String>>(21).unwrap_or_default(),
                "created_at": row.get::<_, String>(22).unwrap_or_default(),
                "published_at": row.get::<_, Option<String>>(23).unwrap_or_default(),
            }))
        }) {
            Ok(r) => r,
            Err(_) => return vec![],
        };
        
        let mut result = Vec::new();
        for row in rows {
            if let Ok(item) = row {
                result.push(item);
            }
        }
        result
    }
    
    pub fn get_shop_items(&self, shop_id: &str) -> Vec<serde_json::Value> {
        self.synchroniser_vitrines_expirees();
        self.synchroniser_elements_expires();
        self.purger_elements_expires();
        let conn = match self.sqlite_pool.get() {
            Ok(c) => c,
            Err(_) => return vec![],
        };
        
        let mut stmt = match conn.prepare(
            "SELECT id, shop_id, item_type, media_urls, title, subtitle, description, price_borb, price_usd, category, commission_enabled, commission_rate, rating, review_count, comment_count, share_count, published, sold, sold_at, edit_count, edited_at, expires_at,
                    (SELECT COUNT(*) FROM item_interactions i WHERE i.item_id = shop_items.id AND i.type = 'view'),
                    (SELECT COUNT(DISTINCT i.user_id) FROM item_interactions i WHERE i.item_id = shop_items.id AND i.type = 'like'),
                    (SELECT COUNT(*) FROM item_interactions i WHERE i.item_id = shop_items.id AND i.type = 'view' AND i.created_at >= datetime('now', '-30 days')),
                    created_at, published_at
             FROM shop_items WHERE shop_id = ?1 ORDER BY created_at DESC"
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        
        let rows = match stmt.query_map(rusqlite::params![shop_id], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, String>(0)?,
                "shop_id": row.get::<_, String>(1)?,
                "type": row.get::<_, String>(2)?,
                "media_urls": row.get::<_, String>(3)?,
                "title": row.get::<_, String>(4)?,
                "subtitle": row.get::<_, String>(5).unwrap_or_default(),
                "description": row.get::<_, String>(6)?,
                "price_borb": row.get::<_, i64>(7)?,
                "price_usd": row.get::<_, f64>(8)?,
                "category": row.get::<_, String>(9)?,
                "commission_enabled": row.get::<_, i64>(10)? == 1,
                "commission_rate": row.get::<_, i64>(11)?,
                "rating": row.get::<_, f64>(12)?,
                "review_count": row.get::<_, i64>(13)?,
                "comment_count": row.get::<_, i64>(14)?,
                "share_count": row.get::<_, i64>(15)?,
                "published": row.get::<_, i64>(16)? == 1,
                "sold": row.get::<_, i64>(17).unwrap_or(0) == 1,
                "sold_at": row.get::<_, Option<String>>(18).unwrap_or_default(),
                "edit_count": row.get::<_, i64>(19).unwrap_or(0),
                "edited_at": row.get::<_, Option<String>>(20).unwrap_or_default(),
                "expires_at": row.get::<_, Option<String>>(21).unwrap_or_default(),
                "view_count": row.get::<_, i64>(22)?,
                "like_count": row.get::<_, i64>(23)?,
                "views_last_30_days": row.get::<_, i64>(24)?,
                "created_at": row.get::<_, String>(25).unwrap_or_default(),
                "published_at": row.get::<_, Option<String>>(26).unwrap_or_default(),
            }))
        }) {
            Ok(r) => r,
            Err(_) => return vec![],
        };
        
        let mut result = Vec::new();
        for row in rows {
            if let Ok(v) = row {
                result.push(v);
            }
        }
        result
    }

    pub fn get_shop_daily_analytics(&self, shop_id: &str) -> Result<Vec<serde_json::Value>, String> {
        let conn = self.sqlite_pool.get().map_err(|error| error.to_string())?;
        let mut stmt = conn.prepare(
            "SELECT date(i.created_at) AS day,
                    SUM(CASE WHEN i.type = 'view' THEN 1 ELSE 0 END) AS views,
                    COUNT(DISTINCT CASE WHEN i.type = 'like' THEN i.user_id END) AS likes
             FROM shop_items item
             JOIN item_interactions i ON i.item_id = item.id
             WHERE item.shop_id = ?1
               AND date(i.created_at) >= date('now', '-29 days')
               AND date(i.created_at) <= date('now')
               AND i.type IN ('view', 'like')
             GROUP BY date(i.created_at)
             ORDER BY day ASC",
        ).map_err(|error| error.to_string())?;
        let rows = stmt.query_map(rusqlite::params![shop_id], |row| {
            Ok(serde_json::json!({
                "date": row.get::<_, String>(0)?,
                "views": row.get::<_, i64>(1)?,
                "likes": row.get::<_, i64>(2)?,
            }))
        }).map_err(|error| error.to_string())?;
        rows.map(|row| row.map_err(|error| error.to_string())).collect()
    }

    /// Récupère un élément par ID
    pub fn get_shop_item_by_id(&self, id: &str) -> Option<serde_json::Value> {
        let conn = self.sqlite_pool.get().ok()?;
        conn.query_row(
            "SELECT id, shop_id, item_type, media_urls, title, subtitle, description, price_borb, price_usd, category, published, sold, sold_at, edit_count, edited_at, expires_at, created_at, published_at
             FROM shop_items WHERE id = ?1",
            rusqlite::params![id],
            |row| {
                Ok(serde_json::json!({
                    "id": row.get::<_, String>(0)?,
                    "shop_id": row.get::<_, String>(1)?,
                    "type": row.get::<_, String>(2)?,
                    "media_urls": row.get::<_, String>(3)?,
                    "title": row.get::<_, String>(4)?,
                    "subtitle": row.get::<_, String>(5).unwrap_or_default(),
                    "description": row.get::<_, String>(6)?,
                    "price_borb": row.get::<_, i64>(7)?,
                    "price_usd": row.get::<_, f64>(8)?,
                    "category": row.get::<_, String>(9)?,
                    "published": row.get::<_, i64>(10)? == 1,
                    "sold": row.get::<_, i64>(11).unwrap_or(0) == 1,
                    "sold_at": row.get::<_, Option<String>>(12).unwrap_or_default(),
                    "edit_count": row.get::<_, i64>(13).unwrap_or(0),
                    "edited_at": row.get::<_, Option<String>>(14).unwrap_or_default(),
                    "expires_at": row.get::<_, Option<String>>(15).unwrap_or_default(),
                    "created_at": row.get::<_, String>(16).unwrap_or_default(),
                    "published_at": row.get::<_, Option<String>>(17).unwrap_or_default(),
                }))
            },
        ).ok()
    }

    /// Met à jour un élément Zando
    /// Met à jour un élément Zando (version complète avec tous les champs)
    pub fn update_shop_item(
        &self,
        id: &str,
        shop_id: &str,
        item_type: &str,
        media_urls: &str,
        title: &str,
        subtitle: &str,
        description: &str,
        price_borb: u64,
        price_usd: f64,
        category: &str,
        commission_enabled: bool,
        commission_rate: u32,
    ) -> Result<(), String> {
        let conn = self.sqlite_pool.get().map_err(|e| e.to_string())?;
        let changed = conn.execute(
            "UPDATE shop_items SET item_type = ?1, media_urls = ?2, title = ?3, subtitle = ?4, description = ?5, price_borb = ?6,
             price_usd = ?7, category = ?8, commission_enabled = ?9, commission_rate = ?10,
             updated_at = datetime('now') WHERE id = ?11 AND shop_id = ?12",
            rusqlite::params![
                item_type, media_urls, title, subtitle, description, price_borb as i64, price_usd, category,
                if commission_enabled { 1 } else { 0 }, commission_rate as i64, id, shop_id
            ],
        ).map_err(|e| e.to_string())?;
        if changed == 0 {
            return Err("Item introuvable ou accès refusé".to_string());
        }
        Ok(())
    }

    /// Enregistre une modification de vente solo : seuls le prix, la
    /// description et le media changent. Le compteur sert de garde-fou
    /// (2 modifications maximum) et `edited_at` horodate la derniere.
    pub fn modifier_element_solo(
        &self,
        id: &str,
        shop_id: &str,
        media_urls: &str,
        description: &str,
        price_borb: u64,
        price_usd: f64,
        commission_enabled: bool,
        commission_rate: u32,
    ) -> Result<(), String> {
        let conn = self.sqlite_pool.get().map_err(|e| e.to_string())?;
        let changed = conn.execute(
            "UPDATE shop_items
                SET media_urls = ?1, description = ?2, price_borb = ?3, price_usd = ?4,
                    commission_enabled = ?5, commission_rate = ?6,
                    edit_count = edit_count + 1, edited_at = datetime('now'),
                    updated_at = datetime('now')
              WHERE id = ?7 AND shop_id = ?8 AND sold = 0",
            rusqlite::params![
                media_urls, description, price_borb as i64, price_usd,
                if commission_enabled { 1 } else { 0 }, commission_rate as i64, id, shop_id
            ],
        ).map_err(|e| e.to_string())?;
        if changed == 0 {
            return Err("Item introuvable ou accès refusé".to_string());
        }
        Ok(())
    }

    /// Marque une vente solo comme vendue : elle quitte le flux pour toujours
    /// et ne laisse qu'une ligne d'historique dans les analytiques du vendeur.
    pub fn marquer_element_vendu(&self, id: &str, shop_id: &str) -> Result<(), String> {
        let conn = self.sqlite_pool.get().map_err(|e| e.to_string())?;
        let changed = conn.execute(
            "UPDATE shop_items
                SET sold = 1, sold_at = datetime('now'), published = 0,
                    updated_at = datetime('now')
              WHERE id = ?1 AND shop_id = ?2 AND sold = 0",
            rusqlite::params![id, shop_id],
        ).map_err(|e| e.to_string())?;
        if changed == 0 {
            return Err("Item introuvable, déjà vendu ou accès refusé".to_string());
        }
        Ok(())
    }

    /// Supprime un élément Zando
    pub fn delete_shop_item(&self, id: &str, shop_id: &str) -> Result<(), String> {
        let conn = self.sqlite_pool.get().map_err(|e| e.to_string())?;
        let changed = conn.execute(
            "DELETE FROM shop_items WHERE id = ?1 AND shop_id = ?2",
            rusqlite::params![id, shop_id],
        )
            .map_err(|e| e.to_string())?;
        if changed == 0 {
            return Err("Item introuvable ou accès refusé".to_string());
        }
        Ok(())
    }

    /// Publie un élément. Une vente solo repart pour 48 h à chaque mise en
    /// ligne ; une vitrine de boutique garde sa propre échéance de présence.
    pub fn publish_shop_item(&self, id: &str, shop_id: &str) -> Result<(), String> {
        let conn = self.sqlite_pool.get().map_err(|e| e.to_string())?;
        let changed = conn.execute(
            "UPDATE shop_items
                SET published = 1,
                    published_at = datetime('now'),
                    expires_at = CASE WHEN item_type = 'boutique'
                                      THEN NULL
                                      ELSE datetime('now', '+48 hours') END
              WHERE id = ?1 AND shop_id = ?2 AND sold = 0",
            rusqlite::params![id, shop_id],
        ).map_err(|e| e.to_string())?;
        if changed == 0 {
            return Err("Item introuvable, vendu ou accès refusé".to_string());
        }
        Ok(())
    }

    /// Retire une vitrine du flux sans la supprimer : l'élément reste dans le
    /// compte du vendeur (brouillon en ligne, visible depuis le Studio).
    pub fn depublier_shop_item(&self, id: &str, shop_id: &str) -> Result<(), String> {
        let conn = self.sqlite_pool.get().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE shop_items SET published = 0 WHERE id = ?1 AND shop_id = ?2",
            rusqlite::params![id, shop_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Une seule boutique, par identifiant, depuis la table `publications`.
    pub fn get_shop_by_id(&self, shop_id: &str) -> Option<serde_json::Value> {
        let conn = self.sqlite_pool.get().ok()?;
        let brut: String = conn
            .query_row(
                "SELECT json FROM publications WHERE id = ?1",
                rusqlite::params![shop_id],
                |row| row.get(0),
            )
            .ok()?;
        serde_json::from_str::<serde_json::Value>(&brut).ok()
    }

    // === ANNONCES DE CONNEXION ===
    //
    // Une table à part, et non `publications` : une annonce de connexion n'est
    // ni une boutique ni un post Zando. Elle doit pouvoir être lue par
    // distance sans traîner derrière elle la liste des vitrines.

    /// Ecrit (ou remplace) l'annonce d'un vendeur. Un compte tient une seule
    /// annonce : publier une seconde fois met à jour la même ligne.
    ///
    /// L'erreur remonte : une fiche que le serveur n'a pas pu écrire n'est pas
    /// une fiche publiée, et le vendeur doit l'entendre.
    pub fn save_annonce(&self, annonce: serde_json::Value) -> Result<(), String> {
        let id = annonce
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if id.is_empty() {
            return Err("Fiche sans identifiant".to_string());
        }
        let conn = self.sqlite_pool.get().map_err(|e| format!("Base inaccessible: {}", e))?;
        conn.execute(
            "INSERT OR REPLACE INTO connexion_offers (id, owner, json, published, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                &id,
                annonce.get("owner").and_then(|v| v.as_str()).unwrap_or(""),
                serde_json::to_string(&annonce).unwrap_or_default(),
                if annonce.get("publiee").and_then(|v| v.as_bool()).unwrap_or(false) { 1 } else { 0 },
                annonce.get("maj_a").and_then(|v| v.as_i64()).unwrap_or(0),
            ],
        )
        .map_err(|e| format!("Ecriture de la fiche impossible: {}", e))?;
        Ok(())
    }

    /// L'annonce d'un compte, qu'elle soit publiée ou retirée : le vendeur a
    /// besoin de relire sa fiche pour la modifier.
    pub fn get_annonce_par_compte(&self, owner: &str) -> Option<serde_json::Value> {
        let conn = self.sqlite_pool.get().ok()?;
        let brut: String = conn
            .query_row(
                "SELECT json FROM connexion_offers WHERE owner = ?1",
                rusqlite::params![owner],
                |row| row.get(0),
            )
            .ok()?;
        serde_json::from_str::<serde_json::Value>(&brut).ok()
    }

    /// Toutes les annonces publiées, dans l'ordre d'écriture. Le tri par
    /// distance et par puissance se fait dans `connexion::classer`, pas ici.
    pub fn get_annonces_publiees(&self) -> Vec<serde_json::Value> {
        let Ok(conn) = self.sqlite_pool.get() else { return Vec::new() };
        let bruts: Vec<String> = match conn.prepare("SELECT json FROM connexion_offers WHERE published = 1") {
            Ok(mut stmt) => match stmt.query_map([], |row| row.get::<_, String>(0)) {
                Ok(rows) => rows.flatten().collect(),
                Err(_) => Vec::new(),
            },
            Err(_) => Vec::new(),
        };
        bruts
            .into_iter()
            .filter_map(|brut| serde_json::from_str::<serde_json::Value>(&brut).ok())
            .collect()
    }

    /// Retire l'annonce du flux sans la supprimer : le vendeur garde sa fiche
    /// et la republie sans ressaisir son nom ni sa photo.
    pub fn depublier_annonce(&self, id: &str) -> Result<(), String> {
        let conn = self.sqlite_pool.get().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE connexion_offers SET published = 0 WHERE id = ?1",
            rusqlite::params![id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Poser (ou reprendre) l'avis d'un compte sur une fiche de connexion.
    ///
    /// La clef du couple fiche-auditeur fait qu'un même compte occupe une seule
    /// ligne : le compteur d'avis ne peut pas être monté à la main.
    pub fn save_annonce_avis(
        &self,
        annonce_id: &str,
        auditeur: &str,
        note: i64,
        texte: &str,
        maintenant: i64,
    ) -> Result<(), String> {
        let conn = self.sqlite_pool.get().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO connexion_avis (annonce_id, uq_noteur, note, texte, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(annonce_id, uq_noteur) DO UPDATE
             SET note = excluded.note, texte = excluded.texte, created_at = excluded.created_at",
            rusqlite::params![annonce_id, auditeur, note, texte, maintenant],
        )
        .map_err(|e| format!("Ecriture de l'avis impossible: {}", e))?;
        Ok(())
    }

    /// Les avis d'une fiche, du plus récent au plus ancien. L'identifiant de
    /// celui qui note ne sort pas : la fiche publique nomme un vendeur, pas un
    /// client.
    pub fn get_annonce_avis(&self, annonce_id: &str, limite: i64) -> Vec<serde_json::Value> {
        let Ok(conn) = self.sqlite_pool.get() else { return Vec::new() };
        let Ok(mut stmt) = conn.prepare(
            "SELECT note, texte, created_at FROM connexion_avis
             WHERE annonce_id = ?1 ORDER BY created_at DESC LIMIT ?2",
        ) else {
            return Vec::new();
        };
        let lignes = match stmt.query_map(rusqlite::params![annonce_id, limite], |row| {
            Ok(serde_json::json!({
                "note": row.get::<_, i64>(0)?,
                "texte": row.get::<_, String>(1)?,
                "ecrit_a": row.get::<_, i64>(2)?,
            }))
        }) {
            Ok(rows) => rows.flatten().collect::<Vec<_>>(),
            Err(_) => Vec::new(),
        };
        lignes
    }

    /// Moyenne et nombre d'avis de toutes les fiches, en une seule lecture :
    /// c'est ce que la liste classée vient coller sur chaque fiche, sans
    /// requête par ligne.
    pub fn get_avis_par_fiche(&self) -> std::collections::HashMap<String, (f64, i64)> {
        let Ok(conn) = self.sqlite_pool.get() else { return Default::default() };
        let Ok(mut stmt) =
            conn.prepare("SELECT annonce_id, COUNT(1), AVG(note) FROM connexion_avis GROUP BY annonce_id")
        else {
            return Default::default();
        };
        let lignes = match stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                (
                    row.get::<_, f64>(2).unwrap_or(0.0),
                    row.get::<_, i64>(1).unwrap_or(0),
                ),
            ))
        }) {
            Ok(rows) => rows.flatten().collect::<Vec<_>>(),
            Err(_) => Vec::new(),
        };
        lignes.into_iter().collect()
    }

    /// Valeur du catalogue d'un vendeur, dans l'unité des `price_borb` : la base
    /// des frais de présence (5 % par bloc de 24 h). Les brouillons et la
    /// vitrine elle-même ne comptent pas — on ne se facture pas son propre accrochage.
    pub fn valeur_catalogue(&self, owner: &str) -> i64 {
        let Ok(conn) = self.sqlite_pool.get() else { return 0 };
        conn.query_row(
            "SELECT COALESCE(SUM(price_borb), 0) FROM shop_items
             WHERE shop_id = ?1 AND published = 1 AND item_type <> 'boutique'",
            rusqlite::params![owner],
            |row| row.get(0),
        )
        .unwrap_or(0)
    }

    /// Le compteur de présence s'applique à la lecture du flux : une vitrine
    /// expirée passe hors ligne (`published = 0`), l'élément et la boutique
    /// restent dans le compte du vendeur. Rien n'est supprimé ici.
    pub fn synchroniser_vitrines_expirees(&self) {
        let Ok(conn) = self.sqlite_pool.get() else { return };
        let maintenant = chrono::Utc::now().timestamp();

        let mut a_depublier: Vec<(String, String)> = Vec::new();
        if let Ok(mut stmt) = conn.prepare("SELECT json FROM publications") {
            if let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(0)) {
                for brut in rows.flatten() {
                    let Ok(value) = serde_json::from_str::<serde_json::Value>(&brut) else {
                        continue;
                    };
                    if value.get("vitrine_item_id").and_then(|v| v.as_str()).unwrap_or("").is_empty()
                    {
                        continue;
                    }
                    let echeance = value.get("en_ligne_jusqu_a").and_then(|v| v.as_i64()).unwrap_or(0);
                    if echeance <= 0 || echeance > maintenant {
                        continue;
                    }
                    a_depublier.push((
                        value["vitrine_item_id"].as_str().unwrap_or("").to_string(),
                        value["owner"].as_str().unwrap_or("").to_string(),
                    ));
                }
            }
        }

        for (item_id, owner) in a_depublier {
            if item_id.is_empty() || owner.is_empty() {
                continue;
            }
            let change = conn
                .execute(
                    "UPDATE shop_items SET published = 0 WHERE id = ?1 AND shop_id = ?2 AND published = 1",
                    rusqlite::params![item_id, owner],
                )
                .unwrap_or(0);
            if change > 0 {
                eprintln!("[BOUTIQUE] vitrine expiree hors ligne : {}", item_id);
            }
        }
    }

    pub fn add_shop_item_interaction(
        &self,
        item_id: &str,
        user_id: &str,
        interaction_type: &str,
        data: &str,
    ) -> Result<(), String> {
        let conn = self.sqlite_pool.get().map_err(|e| e.to_string())?;
        let item_exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM shop_items WHERE id = ?1)",
                rusqlite::params![item_id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !item_exists {
            return Err("Item introuvable".to_string());
        }

        conn.execute(
            "INSERT INTO item_interactions (item_id, user_id, type, data) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![item_id, user_id, interaction_type, data],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn get_shop_item_interactions(
        &self,
        item_id: &str,
    ) -> Result<Vec<serde_json::Value>, String> {
        let conn = self.sqlite_pool.get().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT user_id, type, created_at, data
                 FROM item_interactions
                 WHERE item_id = ?1
                 ORDER BY created_at DESC, id DESC
                 LIMIT 500",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params![item_id], |row| {
                Ok(serde_json::json!({
                    "user_id": row.get::<_, String>(0)?,
                    "type": row.get::<_, String>(1)?,
                    "created_at": row.get::<_, String>(2)?,
                    "data": row.get::<_, String>(3).unwrap_or_else(|_| "{}".to_string()),
                }))
            })
            .map_err(|e| e.to_string())?;
        rows.map(|row| row.map_err(|e| e.to_string()))
            .collect()
    }

    // ====================
    // 📣 PUBLICATIONS — annonces, conferences (7 jours)
    // ====================

    /// Supprime les publications au-dela de leur duree de vie : 7 jours, ou
    /// 24 heures pour un statut. Appelee a chaque publication et chaque
    /// lecture, comme la purge des messages. Les likes et les vues deja
    /// enregistres restent dans leurs tables persistantes.
    pub fn purger_publications_expirees(&self) {
        let Ok(conn) = self.sqlite_pool.get() else { return };
        let maintenant = chrono::Utc::now().timestamp();
        let mut expires: Vec<String> = Vec::new();
        if let Ok(mut stmt) = conn.prepare("SELECT id, json FROM publications") {
            if let Ok(rows) = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            }) {
                for (id, raw) in rows.flatten() {
                    if let Ok(record) = serde_json::from_str::<PublicationRecord>(&raw) {
                        if record.created_at < maintenant - ttl_publication(&record.kind) {
                            expires.push(id);
                        }
                    }
                }
            }
        }
        for id in &expires {
            let _ = conn.execute("DELETE FROM publications WHERE id = ?1", rusqlite::params![id]);
        }
        if !expires.is_empty() {
            eprintln!("[PUBLICATIONS] {} publication(s) supprimee(s) apres expiration", expires.len());
        }
    }

    fn lire_publications(&self) -> Vec<PublicationRecord> {
        let Ok(conn) = self.sqlite_pool.get() else { return Vec::new() };
        let mut out = Vec::new();
        if let Ok(mut stmt) = conn.prepare("SELECT json FROM publications") {
            if let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(0)) {
                for raw in rows.flatten() {
                    if let Ok(record) = serde_json::from_str::<PublicationRecord>(&raw) {
                        out.push(record);
                    }
                }
            }
        }
        out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        out
    }

    fn ecrire_publication(&self, record: &PublicationRecord) -> Result<(), String> {
        let conn = self.sqlite_pool.get().map_err(|e| e.to_string())?;
        let raw = serde_json::to_string(record).map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO publications (id, json) VALUES (?1, ?2)
             ON CONFLICT(id) DO UPDATE SET json = excluded.json",
            rusqlite::params![record.id, raw],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Vue publique d'une publication : jamais de numero UQ, seulement le
    /// pseudo et l'avatar de l'auteur.
    fn publication_vers_json(record: &PublicationRecord, lecteur: &str) -> serde_json::Value {
        let (pseudo, name) = crate::uq_public_alias::get_public_identity(&record.author);
        let auteur = if pseudo.trim().is_empty() {
            "Membre UQ".to_string()
        } else {
            pseudo
        };
        let commentaires: Vec<serde_json::Value> = record
            .comments
            .iter()
            .map(|commentaire| {
                serde_json::json!({
                    "pseudo": commentaire.pseudo,
                    "content": commentaire.content,
                    "created_at": commentaire.created_at,
                    "mine": commentaire.author.eq_ignore_ascii_case(lecteur),
                })
            })
            .collect();
        serde_json::json!({
            "id": record.id,
            "kind": record.kind,
            "title": record.title,
            "body": record.body,
            "price": record.price,
            "media": if record.media.trim().is_empty() { serde_json::Value::Null } else { serde_json::json!(record.media) },
            "zando_item_id": if record.zando_item_id.trim().is_empty() { serde_json::Value::Null } else { serde_json::json!(record.zando_item_id) },
            "author_pseudo": auteur,
            "author_name": name,
            "author_avatar": crate::uq_public_alias::get_public_avatar_url(&record.author),
            "created_at": record.created_at,
            "expires_at": record.created_at + ttl_publication(&record.kind),
            "likes": record.likes.len(),
            "liked": record.likes.iter().any(|uq| uq.eq_ignore_ascii_case(lecteur)),
            "mine": record.author.eq_ignore_ascii_case(lecteur),
            "comments": commentaires,
            "comment_count": commentaires.len(),
        })
    }

    pub fn creer_publication(
        &self,
        author: &str,
        kind: &str,
        title: &str,
        body: &str,
        price: Option<f64>,
        media: &str,
        zando_item_id: &str,
    ) -> Result<serde_json::Value, String> {
        self.purger_publications_expirees();
        let record = PublicationRecord {
            id: format!("pub_{}", chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)),
            author: author.trim().to_uppercase(),
            kind: kind.to_string(),
            title: title.trim().to_string(),
            body: body.trim().to_string(),
            price,
            media: media.trim().to_string(),
            zando_item_id: zando_item_id.trim().to_string(),
            created_at: chrono::Utc::now().timestamp(),
            likes: Vec::new(),
            comments: Vec::new(),
        };
        self.ecrire_publication(&record)?;
        Ok(Self::publication_vers_json(&record, author))
    }

    pub fn lister_publications(&self, lecteur: &str) -> Vec<serde_json::Value> {
        self.purger_publications_expirees();
        let records = self.lire_publications();
        let conn = self.sqlite_pool.get().ok();
        let visibles: Vec<&PublicationRecord> = records.iter().take(100).collect();
        let badges: std::collections::HashMap<String, String> = match &conn {
            Some(c) => {
                let auteurs: Vec<String> = visibles.iter().map(|r| r.author.clone()).collect();
                crate::profile_routes::badges_collectifs(self, c, &auteurs)
            }
            None => std::collections::HashMap::new(),
        };
        let mut sortie: Vec<serde_json::Value> = visibles
            .into_iter()
            .map(|record| {
                let mut json = Self::publication_vers_json(record, lecteur);
                if let Some(c) = &conn {
                    json["likes"] = serde_json::json!(c
                        .query_row(
                            "SELECT COUNT(*) FROM publication_likes WHERE publication_id = ?1",
                            [&record.id],
                            |r| r.get::<_, i64>(0),
                        )
                        .unwrap_or(0));
                    json["liked"] = serde_json::json!(c
                        .query_row(
                            "SELECT 1 FROM publication_likes WHERE publication_id = ?1 AND uq_liker = ?2",
                            rusqlite::params![&record.id, lecteur.trim().to_uppercase()],
                            |_| Ok::<(), rusqlite::Error>(()),
                        )
                        .is_ok());
                    json["vues"] = serde_json::json!(c
                        .query_row(
                            "SELECT COUNT(*) FROM publication_views WHERE publication_id = ?1",
                            [&record.id],
                            |r| r.get::<_, i64>(0),
                        )
                        .unwrap_or(0));
                    let cle = record.author.trim().to_uppercase();
                    let badge = badges.get(&cle).cloned().unwrap_or_default();
                    json["author_badge"] = serde_json::json!(badge);
                    json["author_certifie"] = serde_json::json!(!badge.is_empty());
                }
                json
            })
            .collect();
        // Spécification PARTIE 4 : les comptes certifiés passent devant,
        // tri stable => l'ordre chronologique est conservé à l'intérieur.
        sortie.sort_by(|a, b| {
            certifie_du_json(b).cmp(&certifie_du_json(a))
        });
        sortie
    }

    /// Reporte les likes déjà présents dans les enregistrements de
    /// publications vers la table persistante (une seule fois, idempotent).
    pub fn migrer_tables_reactions(&self) {
        let Ok(conn) = self.sqlite_pool.get() else { return };
        let deja_migre: i64 = conn
            .query_row("SELECT COUNT(*) FROM publication_likes", [], |r| r.get(0))
            .unwrap_or(1);
        if deja_migre > 0 {
            return;
        }
        for record in self.lire_publications() {
            for liker in &record.likes {
                let _ = conn.execute(
                    "INSERT OR IGNORE INTO publication_likes (publication_id, uq_liker, uq_auteur, created_at) VALUES (?1,?2,?3,?4)",
                    rusqlite::params![record.id, liker.to_uppercase(), record.author.to_uppercase(), record.created_at],
                );
            }
        }
    }

    /// Colonnes ajoutees apres coup a `shop_items` (bases deja creees).
    /// SQLite refuse un doublon : on lit d'abord la liste reelle des colonnes.
    pub fn migrer_colonnes_zando(&self) {
        let Ok(conn) = self.sqlite_pool.get() else { return };
        let mut existantes: Vec<String> = Vec::new();
        if let Ok(mut stmt) = conn.prepare("PRAGMA table_info(shop_items)") {
            if let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(1)) {
                existantes.extend(rows.flatten());
            }
        }
        let colonnes: [(&str, &str); 6] = [
            ("subtitle", "TEXT DEFAULT ''"),
            ("sold", "INTEGER NOT NULL DEFAULT 0"),
            ("sold_at", "TEXT"),
            ("edit_count", "INTEGER NOT NULL DEFAULT 0"),
            ("edited_at", "TEXT"),
            ("expires_at", "TEXT"),
        ];
        for (nom, ddl) in colonnes {
            if existantes.iter().any(|colonne| colonne == nom) {
                continue;
            }
            if let Err(erreur) = conn.execute(
                &format!("ALTER TABLE shop_items ADD COLUMN {} {}", nom, ddl),
                [],
            ) {
                eprintln!("[ZANDO] migration colonne {} : {}", nom, erreur);
            }
        }
        // Les ventes solo deja en base n'ont pas d'echeance : on la deduit de
        // leur derniere mise en ligne (ou de leur creation) pour que la regle
        // des 48 h s'applique aussi a l'existant.
        let _ = conn.execute(
            "UPDATE shop_items
                SET expires_at = datetime(COALESCE(published_at, created_at), '+48 hours')
              WHERE item_type != 'boutique' AND expires_at IS NULL",
            [],
        );
    }

    /// Connexion SQLite principale partagée avec les modules de routes.
    pub fn connexion(&self) -> Result<PooledConnection<SqliteConnectionManager>, String> {
        self.sqlite_pool.get().map_err(|e| e.to_string())
    }

    /// Like persistant (table `publication_likes`, source de vérité).
    /// Idempotent : un même UQ ne compte qu'une fois.
    pub fn aimer_publication(
        &self,
        id: &str,
        user: &str,
    ) -> Result<serde_json::Value, String> {
        self.purger_publications_expirees();
        let record = self
            .lire_publications()
            .into_iter()
            .find(|record| record.id == id)
            .ok_or_else(|| "Publication introuvable".to_string())?;
        let liker = user.trim().to_uppercase();
        let conn = self.sqlite_pool.get().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT OR IGNORE INTO publication_likes (publication_id, uq_liker, uq_auteur, created_at) VALUES (?1,?2,?3,?4)",
            rusqlite::params![id, liker, record.author, chrono::Utc::now().timestamp()],
        )
        .map_err(|e| e.to_string())?;
        Ok(Self::publication_vers_json(&record, user))
    }

    pub fn commenter_publication(
        &self,
        id: &str,
        user: &str,
        content: &str,
    ) -> Result<serde_json::Value, String> {
        self.purger_publications_expirees();
        let mut record = self
            .lire_publications()
            .into_iter()
            .find(|record| record.id == id)
            .ok_or_else(|| "Publication introuvable".to_string())?;
        let (pseudo, name) = crate::uq_public_alias::get_public_identity(user);
        let auteur = if pseudo.trim().is_empty() {
            if name.trim().is_empty() { "Membre UQ".to_string() } else { name }
        } else {
            pseudo
        };
        record.comments.push(PublicationComment {
            author: user.trim().to_uppercase(),
            pseudo: auteur,
            content: content.trim().chars().take(500).collect(),
            created_at: chrono::Utc::now().timestamp(),
        });
        if record.comments.len() > 200 {
            let surplus = record.comments.len() - 200;
            record.comments.drain(0..surplus);
        }
        self.ecrire_publication(&record)?;
        Ok(Self::publication_vers_json(&record, user))
    }

    // ====================
    // 🛒 ZANDO — AGENTS (3 fonctions)
    // ====================

    /// Ajoute un agent à un élément
    pub fn add_item_agent(&self, item_id: &str, user_id: &str, commission_rate: u32) -> Result<(), String> {
        let conn = self.sqlite_pool.get().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT OR IGNORE INTO item_agents (item_id, user_id, commission_rate) VALUES (?1, ?2, ?3)",
            rusqlite::params![item_id, user_id, commission_rate as i64],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Récupère les agents d'un élément
    pub fn get_item_agents(&self, item_id: &str) -> Vec<serde_json::Value> {
        let conn = match self.sqlite_pool.get() {
            Ok(c) => c,
            Err(_) => return vec![],
        };
        
        let mut stmt = match conn.prepare(
            "SELECT user_id, commission_rate, sales_count, total_earned FROM item_agents WHERE item_id = ?1"
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        
        let rows = match stmt.query_map(rusqlite::params![item_id], |row| {
            Ok(serde_json::json!({
                "user_id": row.get::<_, String>(0)?,
                "commission_rate": row.get::<_, i64>(1)?,
                "sales_count": row.get::<_, i64>(2)?,
                "total_earned": row.get::<_, i64>(3)?,
            }))
        }) {
            Ok(r) => r,
            Err(_) => return vec![],
        };
        
        let mut result = Vec::new();
        for row in rows {
            if let Ok(v) = row {
                result.push(v);
            }
        }
        result
    }

    /// Enregistre une vente via un agent
    pub fn record_agent_sale(&self, item_id: &str, agent_id: &str, sale_amount_borb: u64) -> Result<u64, String> {
        let conn = self.sqlite_pool.get().map_err(|e| e.to_string())?;
        let rate: i64 = conn.query_row(
            "SELECT commission_rate FROM item_agents WHERE item_id = ?1 AND user_id = ?2",
            rusqlite::params![item_id, agent_id],
            |row| row.get(0),
        ).map_err(|e| e.to_string())?;
        let commission = (sale_amount_borb as i64 * rate) / 100;
        let _ = conn.execute(
            "UPDATE item_agents SET sales_count = sales_count + 1, total_earned = total_earned + ?1 WHERE item_id = ?2 AND user_id = ?3",
            rusqlite::params![commission, item_id, agent_id]
        );
        Ok(commission as u64)
    }

    // ====================
    // 📇 CONTACTS (5 fonctions)
    // ====================

    /// Crée une demande de contact
    pub fn create_contact_request(&self, requester_id: &str, target_id: &str, field: &str) -> Result<String, String> {
        let conn = self.sqlite_pool.get().map_err(|e| e.to_string())?;
        let id = format!("req_{}", chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0));
        conn.execute(
            "INSERT INTO contact_requests (id, requester_id, target_id, field, status) VALUES (?1, ?2, ?3, ?4, 'pending')",
            rusqlite::params![id, requester_id, target_id, field],
        ).map_err(|e| e.to_string())?;
        Ok(id)
    }

    /// Accepte une demande
    pub fn accept_contact_request(&self, id: &str) -> Result<(), String> {
        let conn = self.sqlite_pool.get().map_err(|e| e.to_string())?;
        let (requester, target, field): (String, String, String) = conn.query_row(
            "SELECT requester_id, target_id, field FROM contact_requests WHERE id = ?1",
            rusqlite::params![id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).map_err(|e| e.to_string())?;
        let _ = conn.execute(
            "UPDATE contact_requests SET status = 'accepted' WHERE id = ?1",
            rusqlite::params![id]
        );
        let _ = conn.execute(
            "INSERT OR IGNORE INTO shared_contacts (owner_id, shared_with_id, field) VALUES (?1, ?2, ?3)",
            rusqlite::params![target, requester, field]
        );
        Ok(())
    }

    /// Refuse une demande
    pub fn refuse_contact_request(&self, id: &str) -> Result<(), String> {
        let conn = self.sqlite_pool.get().map_err(|e| e.to_string())?;
        conn.execute("UPDATE contact_requests SET status = 'refused' WHERE id = ?1", rusqlite::params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Partage un contact
    pub fn share_contact(&self, owner_id: &str, shared_with_id: &str, field: &str) -> Result<(), String> {
        let conn = self.sqlite_pool.get().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT OR IGNORE INTO shared_contacts (owner_id, shared_with_id, field) VALUES (?1, ?2, ?3)",
            rusqlite::params![owner_id, shared_with_id, field],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Récupère les contacts partagés
    pub fn get_shared_contacts(&self, user_id: &str) -> Vec<serde_json::Value> {
        let conn = match self.sqlite_pool.get() {
            Ok(c) => c,
            Err(_) => return vec![],
        };
        
        let mut stmt = match conn.prepare("SELECT shared_with_id, field FROM shared_contacts WHERE owner_id = ?1") {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        
        let rows = match stmt.query_map(rusqlite::params![user_id], |row| {
            Ok(serde_json::json!({
                "shared_with_id": row.get::<_, String>(0)?,
                "field": row.get::<_, String>(1)?,
            }))
        }) {
            Ok(r) => r,
            Err(_) => return vec![],
        };
        
        let mut result = Vec::new();
        for row in rows {
            if let Ok(v) = row {
                result.push(v);
            }
        }
        result
    }

    // ====================
    // 📝 DRAFTS (2 fonctions)
    // ====================

    /// Sauvegarde un brouillon
    pub fn save_draft(&self, user_id: &str, draft_type: &str, data: &str) -> Result<String, String> {
        let conn = self.sqlite_pool.get().map_err(|e| e.to_string())?;
        let id = format!("draft_{}", chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0));
        conn.execute(
            "INSERT INTO drafts (id, user_id, draft_type, data) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![id, user_id, draft_type, data],
        ).map_err(|e| e.to_string())?;
        Ok(id)
    }

    /// Supprime un brouillon
    pub fn delete_draft(&self, id: &str) -> Result<(), String> {
        let conn = self.sqlite_pool.get().map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM drafts WHERE id = ?1", rusqlite::params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    /// Récupère un brouillon par son ID
    pub fn get_draft(&self, id: &str) -> Option<serde_json::Value> {
        let conn = self.sqlite_pool.get().ok()?;
        conn.query_row(
            "SELECT id, user_id, draft_type, data, created_at, updated_at 
             FROM drafts WHERE id = ?1",
            rusqlite::params![id],
            |row| {
                Ok(serde_json::json!({
                    "id": row.get::<_, String>(0)?,
                    "user_id": row.get::<_, String>(1)?,
                    "draft_type": row.get::<_, String>(2)?,
                    "data": row.get::<_, String>(3)?,
                    "created_at": row.get::<_, String>(4)?,
                    "updated_at": row.get::<_, String>(5)?,
                }))
            },
        ).ok()
    }

    
    
    
    

    // ====================
    // ⭐ CUSTOM TYPES (types de lieux personnalisés)
    // ====================

    /// Crée un nouveau type personnalisé
    pub fn create_custom_type(
        &self,
        owner_id: &str,
        name: &str,
        emoji: &str,
        color: &str,
        description: &str,
        fields: &str,
    ) -> String {
        let conn = match self.sqlite_pool.get() {
            Ok(c) => c,
            Err(_) => return String::new(),
        };
        
        let id = format!("ctype_{}", chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0));
        let now = chrono::Utc::now().timestamp();
        
        let result = conn.execute(
            "INSERT INTO custom_types (id, owner_id, name, emoji, color, description, fields, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![id, owner_id, name, emoji, color, description, fields, now]
        );
        
        if result.is_ok() { id } else { String::new() }
    }

    /// Récupère un type personnalisé par ID
    pub fn get_custom_type(&self, id: &str) -> Option<serde_json::Value> {
        let conn = self.sqlite_pool.get().ok()?;
        conn.query_row(
            "SELECT id, owner_id, name, emoji, color, description, fields, created_at FROM custom_types WHERE id = ?1",
            rusqlite::params![id],
            |row| {
                Ok(serde_json::json!({
                    "id": row.get::<_, String>(0)?,
                    "owner_id": row.get::<_, String>(1)?,
                    "name": row.get::<_, String>(2)?,
                    "emoji": row.get::<_, String>(3)?,
                    "color": row.get::<_, String>(4)?,
                    "description": row.get::<_, String>(5)?,
                    "fields": row.get::<_, String>(6)?,
                    "created_at": row.get::<_, i64>(7)?,
                }))
            },
        ).ok()
    }

    /// Liste les types personnalisés d'un utilisateur
    pub fn get_custom_types_by_owner(&self, owner_id: &str) -> Vec<serde_json::Value> {
        let conn = match self.sqlite_pool.get() {
            Ok(c) => c,
            Err(_) => return vec![],
        };
        
        let mut stmt = match conn.prepare(
            "SELECT id, owner_id, name, emoji, color, description, fields, created_at FROM custom_types WHERE owner_id = ?1 ORDER BY created_at DESC"
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        
        let rows = stmt.query_map(rusqlite::params![owner_id], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, String>(0)?,
                "owner_id": row.get::<_, String>(1)?,
                "name": row.get::<_, String>(2)?,
                "emoji": row.get::<_, String>(3)?,
                "color": row.get::<_, String>(4)?,
                "description": row.get::<_, String>(5)?,
                "fields": row.get::<_, String>(6)?,
                "created_at": row.get::<_, i64>(7)?,
            }))
        });
        
        let mut result = Vec::new();
        if let Ok(iter) = rows {
            for row in iter {
                if let Ok(entry) = row {
                    result.push(entry);
                }
            }
        }
        result
    }

    /// Liste TOUS les types personnalisés (pour le feed)
    pub fn get_all_custom_types(&self) -> Vec<serde_json::Value> {
        let conn = match self.sqlite_pool.get() {
            Ok(c) => c,
            Err(_) => return vec![],
        };
        
        let mut stmt = match conn.prepare(
            "SELECT id, owner_id, name, emoji, color, description, fields, created_at FROM custom_types ORDER BY created_at DESC LIMIT 100"
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        
        let rows = stmt.query_map([], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, String>(0)?,
                "owner_id": row.get::<_, String>(1)?,
                "name": row.get::<_, String>(2)?,
                "emoji": row.get::<_, String>(3)?,
                "color": row.get::<_, String>(4)?,
                "description": row.get::<_, String>(5)?,
                "fields": row.get::<_, String>(6)?,
                "created_at": row.get::<_, i64>(7)?,
            }))
        });
        
        let mut result = Vec::new();
        if let Ok(iter) = rows {
            for row in iter {
                if let Ok(entry) = row {
                    result.push(entry);
                }
            }
        }
        result
    }

    /// Supprime un type personnalisé
    pub fn delete_custom_type(&self, id: &str) -> Result<(), String> {
        let conn = self.sqlite_pool.get().map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM custom_types WHERE id = ?1", rusqlite::params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    // ====================
    // VERIFIER CREDENTIALS (email + password)
    // Retourne : Option<(is_owner, next_step)>
    // ====================

    /// Verifie si un compte existe avec ces identifiants
    /// Retourne Some((is_owner, next_step)) ou None
    pub fn verify_account_credentials(
        &self,
        number: &str,
        email: &str,
        password_hash: &str,
    ) -> Result<Option<(bool, String)>, String> {
        let conn = self.sqlite_pool.get().map_err(|e| e.to_string())?;

        let result: Result<(String, String, String), _> = conn.query_row(
            "SELECT uq_number, uq_email, status FROM accounts WHERE uq_number = ?1 AND uq_email = ?2 AND password_hash = ?3",
            rusqlite::params![number, email, password_hash],
            |row| Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            )),
        );

        match result {
            Ok((uq_number, _uq_email, status)) => {
                if status != "active" {
                    return Ok(None);
                }

                // Detecter si c'est le proprietaire
                let is_owner = uq_number == "UQ-996697110" || uq_number == "UQ-266335331";

                let next_step = if is_owner {
                    "unlock".to_string()
                } else {
                    "2fa".to_string()
                };

                Ok(Some((is_owner, next_step)))
            },
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }
}



















#[cfg(test)]
mod publications_tests {
    use super::*;

    fn base_test(nom: &str) -> (String, String) {
        let json = format!("target/{}_{}.json", nom, std::process::id());
        let sqlite = json.trim_end_matches(".json").to_string() + ".db";
        let _ = fs::remove_file(&json);
        let _ = fs::remove_file(&sqlite);
        (json, sqlite)
    }

    fn nettoyer(json: &str, sqlite: &str) {
        let _ = fs::remove_file(json);
        let _ = fs::remove_file(sqlite);
        let _ = fs::remove_file(format!("{}-wal", sqlite));
        let _ = fs::remove_file(format!("{}-shm", sqlite));
    }

    #[test]
    fn publications_expirent_apres_sept_jours() {
        let (json, sqlite) = base_test("pub_ttl");
        let db = Database::new(&json);
        let now = chrono::Utc::now().timestamp();
        let ancienne = PublicationRecord {
            id: "pub_old".to_string(),
            author: "UQ-996697110".to_string(),
            kind: "annonce".to_string(),
            title: "Ancienne".to_string(),
            body: String::new(),
            price: None,
            media: String::new(),
            zando_item_id: String::new(),
            created_at: now - PUBLICATION_TTL_SECS - 60,
            likes: Vec::new(),
            comments: Vec::new(),
        };
        let recente = PublicationRecord {
            id: "pub_new".to_string(),
            title: "Recente".to_string(),
            created_at: now - 60,
            ..ancienne.clone()
        };
        db.ecrire_publication(&ancienne).unwrap();
        db.ecrire_publication(&recente).unwrap();

        let liste = db.lister_publications("UQ-996697110");
        assert_eq!(liste.len(), 1, "seule la publication recente reste");
        assert_eq!(liste[0]["id"], "pub_new");

        let restantes: i64 = db
            .sqlite_pool
            .get()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM publications", [], |row| row.get(0))
            .unwrap();
        assert_eq!(restantes, 1, "la ligne de plus de 7 jours est supprimee");
        nettoyer(&json, &sqlite);
    }

    #[test]
    fn like_et_commentaire_sont_persistes() {
        let (json, sqlite) = base_test("pub_like");
        let db = Database::new(&json);
        let creee = db
            .creer_publication("UQ-996697110", "conference", "Titre", "Corps", None, "", "")
            .unwrap();
        let id = creee["id"].as_str().unwrap().to_string();

        db.aimer_publication(&id, "UQ-996697110").unwrap();
        let liste = db.lister_publications("UQ-996697110");
        assert_eq!(liste[0]["likes"], 1);
        assert_eq!(liste[0]["liked"], true);

        db.commenter_publication(&id, "UQ-996697110", "Bravo").unwrap();
        let liste = db.lister_publications("UQ-996697110");
        assert_eq!(liste[0]["comment_count"], 1);
        assert_eq!(liste[0]["comments"][0]["content"], "Bravo");
        nettoyer(&json, &sqlite);
    }

    #[test]
    fn les_quatre_tables_de_reputation_existent() {
        let (json, sqlite) = base_test("pub_tables");
        let db = Database::new(&json);
        let conn = db.sqlite_pool.get().unwrap();
        for table in [
            "publication_likes",
            "publication_views",
            "followers",
            "profile_ratings",
        ] {
            let presente: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name = ?1",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(presente, 1, "la table {table} doit être créée au démarrage");
        }
        nettoyer(&json, &sqlite);
    }

    #[test]
    fn les_reaction_survivent_a_l_expiration_de_la_publication() {
        let (json, sqlite) = base_test("pub_persist");
        let db = Database::new(&json);
        let now = chrono::Utc::now().timestamp();

        // Publication déjà vieille de 7 jours : la purge la retire du flux.
        let vieille = PublicationRecord {
            id: "pub_perimee".to_string(),
            author: "UQ-996697110".to_string(),
            kind: "annonce".to_string(),
            title: "Offre".to_string(),
            body: String::new(),
            price: None,
            media: String::new(),
            zando_item_id: String::new(),
            created_at: now - PUBLICATION_TTL_SECS - 60,
            likes: Vec::new(),
            comments: Vec::new(),
        };
        db.ecrire_publication(&vieille).unwrap();

        // Likes et vues enregistrés pendant sa période de visibilité.
        {
            let conn = db.sqlite_pool.get().unwrap();
            conn.execute(
                "INSERT INTO publication_likes (publication_id, uq_liker, uq_auteur, created_at) VALUES (?1,?2,?3,?4)",
                rusqlite::params!["pub_perimee", "UQ-00000002", "UQ-996697110", now],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO publication_views (publication_id, uq_viewer, uq_auteur, created_at) VALUES (?1,?2,?3,?4)",
                rusqlite::params!["pub_perimee", "UQ-00000003", "UQ-996697110", now],
            )
            .unwrap();
        }

        assert!(
            db.lister_publications("UQ-996697110").is_empty(),
            "la publication expirée a disparu du flux"
        );

        let conn = db.sqlite_pool.get().unwrap();
        let likes_auteur: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM publication_likes WHERE uq_auteur = ?1",
                ["UQ-996697110"],
                |row| row.get(0),
            )
            .unwrap();
        let vues_auteur: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM publication_views WHERE uq_auteur = ?1",
                ["UQ-996697110"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(likes_auteur, 1, "le compteur du profil conserve le like");
        assert_eq!(vues_auteur, 1, "le compteur du profil conserve la vue");
        nettoyer(&json, &sqlite);
    }

    #[test]
    fn un_statut_expire_apres_vingt_quatre_heures_et_ses_compteurs_restent() {
        let (json, sqlite) = base_test("statut_ttl");
        let db = Database::new(&json);
        let now = chrono::Utc::now().timestamp();

        let statut = PublicationRecord {
            id: "pub_statut".to_string(),
            author: "UQ-996697110".to_string(),
            kind: "status".to_string(),
            title: "Nouvelle offre".to_string(),
            body: String::new(),
            price: None,
            media: "https://exemple.test/photo.jpg".to_string(),
            zando_item_id: "item_123".to_string(),
            created_at: now - STATUT_TTL_SECS - 60,
            likes: Vec::new(),
            comments: Vec::new(),
        };
        // Publication classique vieille de 2 jours : le TTL du statut ne doit
        // pas la toucher.
        let annonce = PublicationRecord {
            id: "pub_annonce".to_string(),
            kind: "annonce".to_string(),
            media: String::new(),
            zando_item_id: String::new(),
            created_at: now - 2 * 24 * 3600,
            ..statut.clone()
        };
        db.ecrire_publication(&statut).unwrap();
        db.ecrire_publication(&annonce).unwrap();

        {
            let conn = db.sqlite_pool.get().unwrap();
            conn.execute(
                "INSERT INTO publication_likes (publication_id, uq_liker, uq_auteur, created_at) VALUES (?1,?2,?3,?4)",
                rusqlite::params!["pub_statut", "UQ-00000002", "UQ-996697110", now],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO publication_views (publication_id, uq_viewer, uq_auteur, created_at) VALUES (?1,?2,?3,?4)",
                rusqlite::params!["pub_statut", "UQ-00000003", "UQ-996697110", now],
            )
            .unwrap();
        }

        let liste = db.lister_publications("UQ-00000002");
        assert_eq!(liste.len(), 1, "le statut de plus de 24 h a disparu du flux");
        assert_eq!(liste[0]["id"], "pub_annonce");

        let conn = db.sqlite_pool.get().unwrap();
        let likes_auteur: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM publication_likes WHERE uq_auteur = ?1",
                ["UQ-996697110"],
                |row| row.get(0),
            )
            .unwrap();
        let vues_auteur: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM publication_views WHERE uq_auteur = ?1",
                ["UQ-996697110"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(likes_auteur, 1, "le like recu sur le statut expire reste au profil");
        assert_eq!(vues_auteur, 1, "la vue du statut expire reste au profil");
        nettoyer(&json, &sqlite);
    }

    #[test]
    fn un_statut_porte_son_media_et_son_lien_zando() {
        let (json, sqlite) = base_test("statut_media");
        let db = Database::new(&json);
        let creee = db
            .creer_publication(
                "UQ-996697110",
                "status",
                "Coque iPhone",
                "Disponible maintenant",
                None,
                "https://exemple.test/photo.jpg",
                "item_42",
            )
            .unwrap();
        assert_eq!(creee["kind"], "status");
        assert_eq!(creee["media"], "https://exemple.test/photo.jpg");
        assert_eq!(creee["zando_item_id"], "item_42");
        let created_at = creee["created_at"].as_i64().unwrap();
        let expires_at = creee["expires_at"].as_i64().unwrap();
        assert_eq!(expires_at - created_at, STATUT_TTL_SECS, "24 heures");
        nettoyer(&json, &sqlite);
    }

    #[test]
    fn elements_zando_expirent_apres_sept_jours() {
        let (json, sqlite) = base_test("zando_ttl");
        let db = Database::new(&json);
        {
            let conn = db.sqlite_pool.get().unwrap();
            conn.execute(
                "INSERT INTO shop_items (id, shop_id, item_type, title, published, created_at, published_at)
                 VALUES ('item_old', 'UQ-996697110', 'text', 'Vieux', 1, datetime('now','-8 days'), datetime('now','-8 days'))",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO shop_items (id, shop_id, item_type, title, published, created_at, published_at)
                 VALUES ('item_new', 'UQ-996697110', 'text', 'Recent', 1, datetime('now','-1 days'), datetime('now','-1 days'))",
                [],
            )
            .unwrap();
        }

        let items = db.get_all_shop_items();
        assert!(items.iter().any(|item| item["id"] == "item_new"));
        assert!(!items.iter().any(|item| item["id"] == "item_old"));

        let restants: i64 = db
            .sqlite_pool
            .get()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM shop_items", [], |row| row.get(0))
            .unwrap();
        assert_eq!(restants, 1, "l'element de plus de 7 jours est supprime");
        nettoyer(&json, &sqlite);
    }
}

#[cfg(test)]
mod chat_messages_tests {
    use super::*;

    fn base_test(nom: &str) -> (String, String) {
        let json = format!("target/{}_{}.json", nom, std::process::id());
        let sqlite = json.trim_end_matches(".json").to_string() + ".db";
        let _ = fs::remove_file(&json);
        let _ = fs::remove_file(&sqlite);
        (json, sqlite)
    }

    fn nettoyer(json: &str, sqlite: &str) {
        let _ = fs::remove_file(json);
        let _ = fs::remove_file(sqlite);
        let _ = fs::remove_file(format!("{}-wal", sqlite));
        let _ = fs::remove_file(format!("{}-shm", sqlite));
    }

    /// Recule la fin de vie du message `id` : la purge doit la traiter comme ecoulee.
    /// Les deux stores vieillissent ensemble, sinon le miroir ressusciterait a la
    /// relecture un message que la vue memoire considere deja comme fini.
    fn vieillir(db: &Database, id: &str, secondes: i64) {
        let maintenant = chrono::Utc::now().timestamp();
        {
            let mut data = db.data.lock().unwrap();
            for message in data.messages.iter_mut() {
                if message.id == id {
                    message.expires_at = Some(maintenant - secondes);
                }
            }
        }
        db.sqlite_pool
            .get()
            .unwrap()
            .execute(
                "UPDATE chat_messages SET expires_at = ?1 WHERE id = ?2",
                rusqlite::params![maintenant - secondes, id],
            )
            .unwrap();
    }

    #[test]
    fn un_message_normal_ne_porte_aucune_fin_de_vie() {
        let (json, sqlite) = base_test("chat_persistant");
        let db = Database::new(&json);
        let id = db.send_message("UQ-996697110", "UQ-00000002", "Bonjour");
        let porte = db
            .data
            .lock()
            .unwrap()
            .messages
            .iter()
            .find(|m| m.id == id)
            .and_then(|m| m.expires_at)
            .is_some();
        assert!(!porte, "sans mode ephemere, rien ne doit etre programme a la suppression");

        db.purger_messages_expires();
        assert_eq!(
            db.get_messages_between("UQ-996697110", "UQ-00000002").len(),
            1,
            "la purge laisse le message persistant en place"
        );
        nettoyer(&json, &sqlite);
    }

    #[test]
    fn un_message_ephemere_porte_sa_date_de_finpuis_disparait() {
        let (json, sqlite) = base_test("chat_ephemere");
        let db = Database::new(&json);
        let id = db.send_message_avec_image("UQ-996697110", "UQ-00000002", "Photo", None, true);
        let maintenant = chrono::Utc::now().timestamp();
        let fin = db
            .data
            .lock()
            .unwrap()
            .messages
            .iter()
            .find(|m| m.id == id)
            .and_then(|m| m.expires_at)
            .expect("le message ephemere doit porter sa fin de vie");
        assert_eq!(fin - maintenant, MESSAGE_TTL_SECS, "72 h apres l'envoi");

        vieillir(&db, &id, 60);
        db.purger_messages_expires();
        assert!(
            db.get_messages_between("UQ-996697110", "UQ-00000002")
                .iter()
                .all(|m| m.id != id),
            "un ephemere ecoule doit etre purge"
        );
        nettoyer(&json, &sqlite);
    }

    #[test]
    fn la_lecture_confirme_l_envoi_et_le_compte_rend_non_lus() {
        let (json, sqlite) = base_test("chat_lus");
        let db = Database::new(&json);
        db.send_message("UQ-996697110", "UQ-00000002", "Salut");
        db.send_message("UQ-996697110", "UQ-00000002", "Ca va ?");
        db.send_message("UQ-00000002", "UQ-996697110", "Oui");

        assert_eq!(db.compter_non_lus("UQ-00000002"), 2);
        assert_eq!(db.conversations_non_lues("UQ-00000002"), 1);
        assert_eq!(db.non_lus_par_pair("UQ-00000002").get("UQ-996697110").copied(), Some(2));

        let marques = db.marquer_lus("UQ-00000002", "UQ-996697110");
        assert_eq!(marques, 2, "les deux messages viennent d'etre relus");
        assert_eq!(db.compter_non_lus("UQ-00000002"), 0);
        assert!(
            db.get_messages_between("UQ-996697110", "UQ-00000002")
                .iter()
                .filter(|m| m.sender == "UQ-996697110")
                .all(|m| m.delivered),
            "l'expediteur doit voir ses messages marques lus"
        );

        assert_eq!(db.marquer_lus("UQ-00000002", "UQ-996697110"), 0, "plus rien a marquer");
        nettoyer(&json, &sqlite);
    }

    #[test]
    fn un_ephemere_ecoule_n_alimente_plus_les_compteurs() {
        let (json, sqlite) = base_test("chat_non_lus_ttl");
        let db = Database::new(&json);
        let id = db.send_message_avec_image("UQ-996697110", "UQ-00000002", "Vu une fois", None, true);
        assert_eq!(db.compter_non_lus("UQ-00000002"), 1);

        vieillir(&db, &id, 60);
        assert_eq!(
            db.compter_non_lus("UQ-00000002"),
            0,
            "un message fini ne doit etre ni compte ni montre"
        );
        nettoyer(&json, &sqlite);
    }

    #[test]
    fn un_redemarrage_ne_perd_pas_l_historique_du_chat() {
        let (json, sqlite) = base_test("chat_redemarrage");
        {
            let db = Database::new(&json);
            db.send_message("UQ-996697110", "UQ-00000002", "Bonjour");
            let relu = db.send_message("UQ-996697110", "UQ-00000002", "Tu la vois ?");
            db.send_message("UQ-00000002", "UQ-996697110", "Oui");
            db.marquer_lus("UQ-00000002", "UQ-996697110");
            let ephemere =
                db.send_message_avec_image("UQ-996697110", "UQ-00000002", "Photo", None, true);
            vieillir(&db, &ephemere, 60);
            db.purger_messages_expires();
            assert_eq!(db.get_messages_between("UQ-996697110", "UQ-00000002").len(), 3);
            assert!(relu.starts_with("msg_"));
        }
        // Second processus sur les memes fichiers : c'est ce que fait un
        // redemarrage du serveur, et l'historique doit en sortir intact.
        let db = Database::new(&json);
        let fils = db.get_messages_between("UQ-996697110", "UQ-00000002");
        assert_eq!(fils.len(), 3, "les trois messages survivent au redemarrage");
        assert_eq!(
            fils.iter().filter(|m| m.delivered).count(),
            2,
            "l'accuse de lecture survit aussi"
        );
        assert!(
            fils.iter().all(|m| m.expires_at.is_none()),
            "l'ephemere purge doit etre purge partout a la fois"
        );
        assert_eq!(db.compter_non_lus("UQ-00000002"), 0, "les accuses de lecture sont en base");
        assert_eq!(
            db.compter_non_lus("UQ-996697110"),
            1,
            "la reponse jamais relue reste comptee apres le redemarrage"
        );
        nettoyer(&json, &sqlite);
    }

    #[test]
    fn l_envoi_reel_laisse_la_trace_en_base() {
        let (json, sqlite) = base_test("chat_miroir");
        let db = Database::new(&json);
        let id = db.send_message("UQ-996697110", "UQ-00000002", "Trace");
        let conn = db.sqlite_pool.get().unwrap();
        let ligne = conn.query_row(
            "SELECT content, delivered, expires_at FROM chat_messages WHERE id = ?1",
            [id.clone()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, Option<i64>>(2)?)),
        );
        assert_eq!(ligne.map(|(c, d, e)| (c, d, e.is_none())), Ok(("Trace".to_string(), 0, true)));
        db.marquer_lus("UQ-00000002", "UQ-996697110");
        let marque: i64 = conn
            .query_row("SELECT delivered FROM chat_messages WHERE id = ?1", [id], |row| row.get(0))
            .unwrap();
        assert_eq!(marque, 1, "l UPDATE doit atteindre la table, pas seulement la memoire");
        nettoyer(&json, &sqlite);
    }
}
