// ====================
// SCHEMA CENTRALISÉ — UQ PLATFORM
// Un SEUL endroit pour TOUTES les tables, index et données par défaut
// ====================

use rusqlite::{Connection, Result};

// ====================
// TABLE CATEGORIES
// ====================

pub const TABLE_BASE: &[&str] = &[
    "balance",
    "transactions",
    "settings",
    "my_profile",
    "my_private_key",
    "wallet",
];

pub const TABLE_ZANDO: &[&str] = &[
    "my_items",
    "my_purchases",
    "my_sales",
    "my_reviews",
    "my_contacts",
    "my_favorites",
    "my_bookmarks",
    "my_item_views",
];

pub const TABLE_FINANCE: &[&str] = &[
    "freeze_contracts",
    "loans",
    "insurance",
    "my_savings",
    "my_trades",
    "my_trade_history",
    "my_p2p_orders",
    "my_borb_conversions",
    "my_borb_units",
    "borb_operations",
];

pub const TABLE_SOCIAL: &[&str] = &[
    "my_contacts_online",
    "my_contact_requests_sent",
    "my_contact_requests_received",
    "my_shared_contacts",
    "my_blocked_contacts",
    "my_groups",
    "my_group_members",
    "my_group_messages",
    "my_presence",
    "messages",
    "my_likes",
    "my_comments",
    "my_received_comments",
    "my_received_likes",
    "my_profile_views",
];

pub const TABLE_NOTIFICATIONS: &[&str] = &[
    "notifications",
    "my_notification_queue",
    "my_notification_prefs",
    "my_push_tokens",
];

pub const TABLE_SECURITY: &[&str] = &[
    "security_log",
    "reputation_history",
    "referrals",
    "cadenas_verification",
    "user_login_config",
    "pin_status",
    "kyc_documents",
    "security_login_cooldown_uq",
    "identities",
    "uq_aliases",
    "privileges",
    "sessions",
    "vaults",
    "public_transfers",
];

pub const TABLE_TREASURY: &[&str] = &[
    "owner_account",
    "treasury_system",
    "dev_profile",
    "system_profiles",
    "treasury_system_sales",
    "dev_ads",
    "treasury_revenue",
    "treasury_wallets",
    "system_messages",
    "user_system_messages",
];

pub const TABLE_COUNTER: &[&str] = &[
    "counter_calculator",
    "counter_transactions",
    "counter_case_rights",
    "seven_cases",
    "bridge_actions",
    "anomalies",
    "optimizations",
    "invoices",
];

// ====================
// SCHEMA SQL COMPLET
// ====================

pub const SCHEMA_SQL: &str = r#"
-- ═══════════════════════════════════════════════════════════════
-- 1. BASE
-- ═══════════════════════════════════════════════════════════════

CREATE TABLE IF NOT EXISTS balance (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    bubbles INTEGER DEFAULT 0,
    points INTEGER DEFAULT 0,
    frozen_points INTEGER DEFAULT 0,
    updated_at INTEGER NOT NULL
);
INSERT OR IGNORE INTO balance (id, bubbles, points, frozen_points, updated_at)
VALUES (1, 0, 0, 0, strftime('%s', 'now'));

CREATE TABLE IF NOT EXISTS transactions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    tx_type TEXT NOT NULL,
    amount INTEGER NOT NULL,
    fee INTEGER DEFAULT 0,
    counterparty TEXT,
    memo TEXT,
    signature BLOB,
    created_at INTEGER NOT NULL,
    -- ═══ Traçabilité BORB (utilisée par bubble_transfer) ═══
    device TEXT,
    os TEXT,
    browser TEXT,
    ip TEXT,
    sender_uq TEXT,
    recipient_uq TEXT,
    amount_borb REAL DEFAULT 0,
    amount_usd REAL DEFAULT 0,
    taux_usd TEXT,
    public_key TEXT,
    status TEXT DEFAULT 'success',
    reference TEXT,
    device_hash TEXT,
    tampered INTEGER DEFAULT 0,
    cadenas_hash TEXT,
    internal_clock INTEGER,
    universal_key_hash TEXT,
    -- ═══ Historique local : chaînage (spécification) ═══
    borb_id TEXT,
    prev_hash TEXT,
    block_number INTEGER DEFAULT 0
);

CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT,
    updated_at INTEGER NOT NULL
);
INSERT OR IGNORE INTO settings (key, value, updated_at) VALUES
    ('language', 'fr', strftime('%s', 'now')),
    ('theme', 'dark', strftime('%s', 'now')),
    ('font_size', 'small', strftime('%s', 'now')),
    ('display_unit', 'BORB', strftime('%s', 'now')),
    ('pin_enabled', 'false', strftime('%s', 'now'));

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

CREATE TABLE IF NOT EXISTS my_private_key (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    encrypted_key BLOB NOT NULL,
    iv BLOB NOT NULL,
    salt BLOB NOT NULL,
    algorithm TEXT DEFAULT 'AES-256-GCM',
    created_at INTEGER NOT NULL,
    rotated_at INTEGER
);

CREATE TABLE IF NOT EXISTS wallet (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    balance_borb REAL DEFAULT 0.0,
    balance_points INTEGER DEFAULT 0,
    balance_frozen INTEGER DEFAULT 0,
    total_received INTEGER DEFAULT 0,
    total_sent INTEGER DEFAULT 0,
    last_updated INTEGER DEFAULT 0
);
INSERT OR IGNORE INTO wallet (id, last_updated) VALUES (1, strftime('%s', 'now'));

-- ═══════════════════════════════════════════════════════════════
-- 2. ZANDO
-- ═══════════════════════════════════════════════════════════════

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

CREATE TABLE IF NOT EXISTS my_purchases (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    item_id TEXT NOT NULL,
    seller_id TEXT NOT NULL,
    price_paid INTEGER NOT NULL,
    status TEXT DEFAULT 'completed',
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_purchases_item ON my_purchases(item_id);

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

CREATE TABLE IF NOT EXISTS my_reviews (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    item_id TEXT NOT NULL,
    target_user TEXT NOT NULL,
    rating INTEGER CHECK (rating >= 1 AND rating <= 5),
    comment TEXT,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_reviews_item ON my_reviews(item_id);

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

CREATE TABLE IF NOT EXISTS my_favorites (
    item_id TEXT PRIMARY KEY,
    added_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS my_bookmarks (
    publication_id TEXT PRIMARY KEY,
    added_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS my_item_views (
    item_id TEXT PRIMARY KEY,
    view_count INTEGER DEFAULT 0,
    last_view INTEGER
);

-- ═══════════════════════════════════════════════════════════════
-- 3. FINANCE
-- ═══════════════════════════════════════════════════════════════

CREATE TABLE IF NOT EXISTS freeze_contracts (
    id TEXT PRIMARY KEY,
    amount INTEGER NOT NULL,
    rate_percent REAL DEFAULT 2.0,
    locked_until INTEGER NOT NULL,
    status TEXT DEFAULT 'active',
    reward_earned INTEGER DEFAULT 0,
    created_at INTEGER NOT NULL
);

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

CREATE TABLE IF NOT EXISTS insurance (
    id TEXT PRIMARY KEY,
    contribution INTEGER NOT NULL,
    claim_amount INTEGER DEFAULT 0,
    claim_reason TEXT,
    status TEXT DEFAULT 'contributed',
    created_at INTEGER NOT NULL
);

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

CREATE TABLE IF NOT EXISTS my_borb_conversions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    from_unit TEXT NOT NULL,
    to_unit TEXT NOT NULL,
    amount_from INTEGER NOT NULL,
    amount_to INTEGER NOT NULL,
    rate REAL NOT NULL,
    created_at INTEGER NOT NULL
);

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

CREATE TABLE IF NOT EXISTS borb_operations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    operation_type TEXT NOT NULL,
    amount_borb REAL DEFAULT 0.0,
    amount_points INTEGER DEFAULT 0,
    from_unit TEXT DEFAULT 'BORB',
    to_unit TEXT DEFAULT 'BORB',
    status TEXT DEFAULT 'pending',
    tx_hash TEXT DEFAULT '',
    created_at INTEGER DEFAULT 0
);

-- ═══════════════════════════════════════════════════════════════
-- 4. SOCIAL
-- ═══════════════════════════════════════════════════════════════

CREATE TABLE IF NOT EXISTS my_contacts_online (
    contact_id TEXT PRIMARY KEY,
    is_online INTEGER DEFAULT 0,
    last_seen INTEGER,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS my_contact_requests_sent (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    to_user TEXT NOT NULL,
    item_id TEXT,
    status TEXT DEFAULT 'pending',
    created_at INTEGER NOT NULL,
    responded_at INTEGER
);
CREATE INDEX IF NOT EXISTS idx_req_sent_status ON my_contact_requests_sent(status);

CREATE TABLE IF NOT EXISTS my_contact_requests_received (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    from_user TEXT NOT NULL,
    item_id TEXT,
    status TEXT DEFAULT 'pending',
    created_at INTEGER NOT NULL,
    responded_at INTEGER
);
CREATE INDEX IF NOT EXISTS idx_req_recv_status ON my_contact_requests_received(status);

CREATE TABLE IF NOT EXISTS my_shared_contacts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    contact_user TEXT NOT NULL,
    shared_at INTEGER NOT NULL,
    fields TEXT
);

CREATE TABLE IF NOT EXISTS my_blocked_contacts (
    user_id TEXT PRIMARY KEY,
    reason TEXT,
    blocked_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS my_groups (
    group_id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    type TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS my_group_members (
    group_id TEXT NOT NULL,
    member_id TEXT NOT NULL,
    role TEXT DEFAULT 'member',
    joined_at INTEGER NOT NULL,
    PRIMARY KEY (group_id, member_id)
);

CREATE TABLE IF NOT EXISTS my_group_messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    group_id TEXT NOT NULL,
    sender_id TEXT NOT NULL,
    content TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_group_msg_group ON my_group_messages(group_id);

CREATE TABLE IF NOT EXISTS my_presence (
    user_id TEXT PRIMARY KEY,
    is_online INTEGER DEFAULT 0,
    last_seen INTEGER,
    updated_at INTEGER NOT NULL
);

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

CREATE TABLE IF NOT EXISTS my_likes (
    item_id TEXT PRIMARY KEY,
    liked_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS my_comments (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    item_id TEXT NOT NULL,
    content TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_comments_item ON my_comments(item_id);

CREATE TABLE IF NOT EXISTS my_received_comments (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    item_id TEXT NOT NULL,
    from_user TEXT NOT NULL,
    content TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS my_received_likes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    item_id TEXT NOT NULL,
    from_user TEXT NOT NULL,
    liked_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS my_profile_views (
    viewer_id TEXT PRIMARY KEY,
    viewed_at INTEGER NOT NULL
);

-- ═══════════════════════════════════════════════════════════════
-- 5. NOTIFICATIONS
-- ═══════════════════════════════════════════════════════════════

CREATE TABLE IF NOT EXISTS notifications (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL,
    content TEXT,
    notif_type TEXT,
    read INTEGER DEFAULT 0,
    created_at INTEGER NOT NULL
);

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

CREATE TABLE IF NOT EXISTS my_notification_prefs (
    type TEXT PRIMARY KEY,
    enabled INTEGER DEFAULT 1,
    sound INTEGER DEFAULT 1,
    vibration INTEGER DEFAULT 1
);

CREATE TABLE IF NOT EXISTS my_push_tokens (
    token TEXT PRIMARY KEY,
    platform TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    last_used INTEGER
);

-- ═══════════════════════════════════════════════════════════════
-- 6. SECURITY
-- ═══════════════════════════════════════════════════════════════

CREATE TABLE IF NOT EXISTS security_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    event_type TEXT NOT NULL,
    ip TEXT,
    device TEXT,
    success INTEGER DEFAULT 1,
    details TEXT,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS reputation_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    old_score INTEGER NOT NULL,
    new_score INTEGER NOT NULL,
    reason TEXT,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS referrals (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    referrer_id TEXT NOT NULL,
    referred_id TEXT NOT NULL,
    commission_earned INTEGER DEFAULT 0,
    created_at INTEGER NOT NULL
);

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

CREATE TABLE IF NOT EXISTS user_login_config (
    uq_number TEXT PRIMARY KEY,
    is_owner INTEGER DEFAULT 0,
    requires_unlock_code INTEGER DEFAULT 0,
    requires_triple_attempt INTEGER DEFAULT 0,
    pin_required_step INTEGER DEFAULT 1,
    max_fails_before_iphone INTEGER DEFAULT 5,
    created_at INTEGER DEFAULT 0
);

CREATE TABLE IF NOT EXISTS pin_status (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    pin_hash TEXT DEFAULT '',
    pin_enabled INTEGER DEFAULT 0,
    pin_length INTEGER DEFAULT 4,
    modified_at INTEGER DEFAULT 0,
    attempts INTEGER DEFAULT 0,
    locked_until INTEGER DEFAULT 0,
    created_at INTEGER DEFAULT 0
);
INSERT OR IGNORE INTO pin_status (id, created_at)
VALUES (1, strftime('%s', 'now'));

CREATE TABLE IF NOT EXISTS kyc_documents (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    doc_type TEXT NOT NULL,
    doc_hash TEXT DEFAULT '',
    doc_path TEXT DEFAULT '',
    verified INTEGER DEFAULT 0,
    uploaded_at INTEGER DEFAULT 0,
    verified_at INTEGER DEFAULT 0
);

CREATE TABLE IF NOT EXISTS security_login_cooldown_uq (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    uq_number TEXT UNIQUE NOT NULL,
    is_owner INTEGER DEFAULT 0,
    failure_count INTEGER DEFAULT 0,
    cooldown_level INTEGER DEFAULT 0,
    cooldown_until INTEGER DEFAULT 0,
    last_attempt INTEGER DEFAULT 0,
    first_failure_at INTEGER DEFAULT 0,
    attempts_ok INTEGER DEFAULT 0,
    is_locked INTEGER DEFAULT 0,
    locked_until INTEGER DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_cooldown_uq ON security_login_cooldown_uq(uq_number);

CREATE TABLE IF NOT EXISTS identities (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    identity_type TEXT NOT NULL CHECK (identity_type IN ('owner', 'public')),
    uq_number TEXT UNIQUE NOT NULL,
    public_uq TEXT DEFAULT '',
    nom TEXT DEFAULT '',
    prenom TEXT DEFAULT '',
    email TEXT DEFAULT '',
    pseudo TEXT DEFAULT '',
    date_naissance TEXT DEFAULT '',
    lieu_naissance TEXT DEFAULT '',
    pays TEXT DEFAULT 'RDC',
    adresse TEXT DEFAULT '',
    telephone TEXT DEFAULT '',
    is_public INTEGER DEFAULT 0,
    is_verified INTEGER DEFAULT 0,
    created_at INTEGER DEFAULT 0,
    updated_at INTEGER DEFAULT 0
);

CREATE TABLE IF NOT EXISTS uq_aliases (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    real_uq TEXT NOT NULL,
    public_uq TEXT NOT NULL,
    display_name TEXT DEFAULT '',
    active INTEGER DEFAULT 1,
    created_at INTEGER DEFAULT 0,
    UNIQUE(real_uq, public_uq)
);

CREATE TABLE IF NOT EXISTS privileges (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    privilege_name TEXT UNIQUE NOT NULL,
    enabled INTEGER DEFAULT 0,
    granted_at INTEGER DEFAULT 0,
    granted_by TEXT DEFAULT 'system',
    notes TEXT DEFAULT ''
);

CREATE TABLE IF NOT EXISTS sessions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    token TEXT UNIQUE NOT NULL,
    device TEXT DEFAULT '',
    ip TEXT DEFAULT '',
    user_agent TEXT DEFAULT '',
    created_at INTEGER DEFAULT 0,
    expires_at INTEGER DEFAULT 0,
    active INTEGER DEFAULT 1
);

CREATE TABLE IF NOT EXISTS vaults (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    vault_name TEXT NOT NULL,
    vault_type TEXT DEFAULT 'standard',
    balance_borb REAL DEFAULT 0.0,
    balance_points INTEGER DEFAULT 0,
    locked INTEGER DEFAULT 0,
    unlock_date INTEGER DEFAULT 0,
    created_at INTEGER DEFAULT 0
);

CREATE TABLE IF NOT EXISTS public_transfers (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    sender_real_uq TEXT DEFAULT '',
    sender_public_uq TEXT DEFAULT '',
    sender_display_name TEXT DEFAULT '',
    recipient_uq TEXT DEFAULT '',
    amount_points INTEGER DEFAULT 0,
    amount_borb REAL DEFAULT 0.0,
    tx_hash TEXT DEFAULT '',
    created_at INTEGER DEFAULT 0
);

-- ═══════════════════════════════════════════════════════════════
-- 7. TREASURY
-- ═══════════════════════════════════════════════════════════════

CREATE TABLE IF NOT EXISTS owner_account (
    id INTEGER PRIMARY KEY,
    uq_number TEXT UNIQUE NOT NULL,
    email TEXT UNIQUE NOT NULL,
    created_at INTEGER DEFAULT (strftime('%s', 'now'))
);

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
CREATE TABLE IF NOT EXISTS distribution_state (
    id INTEGER PRIMARY KEY,
    owner_id TEXT UNIQUE NOT NULL,
    distributed_at INTEGER NOT NULL,
    initial_total_borb REAL NOT NULL,
    profil1_borb REAL NOT NULL,
    profil2_borb REAL NOT NULL
);

CREATE TABLE IF NOT EXISTS system_profiles (
    id INTEGER PRIMARY KEY,
    code TEXT UNIQUE NOT NULL,
    display_name TEXT NOT NULL,
    icon TEXT,
    description TEXT,
    is_active INTEGER DEFAULT 1,
    created_at INTEGER DEFAULT (strftime('%s', 'now'))
);

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

CREATE TABLE IF NOT EXISTS treasury_wallets (
    id INTEGER PRIMARY KEY,
    crypto_type TEXT NOT NULL,
    wallet_address TEXT NOT NULL,
    label TEXT,
    is_default INTEGER DEFAULT 0,
    created_at INTEGER DEFAULT (strftime('%s', 'now'))
);

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

CREATE TABLE IF NOT EXISTS user_system_messages (
    id INTEGER PRIMARY KEY,
    user_id TEXT NOT NULL,
    message_id INTEGER NOT NULL,
    is_read INTEGER DEFAULT 0,
    read_at INTEGER,
    created_at INTEGER DEFAULT (strftime('%s', 'now')),
    FOREIGN KEY (message_id) REFERENCES system_messages(id)
);

-- ═══════════════════════════════════════════════════════════════
-- 8. COUNTER & CASES
-- ═══════════════════════════════════════════════════════════════

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
CREATE INDEX IF NOT EXISTS idx_counter_tx_block ON counter_transactions(block_number);

CREATE TABLE IF NOT EXISTS counter_case_rights (
    id INTEGER PRIMARY KEY,
    case_name TEXT UNIQUE,
    right_percentage REAL,
    total_received REAL DEFAULT 0,
    last_received_at INTEGER,
    updated_at INTEGER DEFAULT (strftime('%s', 'now'))
);

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
CREATE INDEX IF NOT EXISTS idx_bridges_block ON bridge_actions(block_number);

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
CREATE INDEX IF NOT EXISTS idx_anomalies_block ON anomalies(block_number);

CREATE TABLE IF NOT EXISTS optimizations (
    id INTEGER PRIMARY KEY,
    block_number INTEGER,
    action_type TEXT,
    details TEXT,
    applied INTEGER DEFAULT 1,
    created_at INTEGER DEFAULT (strftime('%s', 'now'))
);
CREATE INDEX IF NOT EXISTS idx_optimizations_block ON optimizations(block_number);

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
CREATE INDEX IF NOT EXISTS idx_invoices_tx_id ON invoices(tx_id);
CREATE INDEX IF NOT EXISTS idx_invoices_from_user ON invoices(from_user);
CREATE INDEX IF NOT EXISTS idx_invoices_to_user ON invoices(to_user);
CREATE INDEX IF NOT EXISTS idx_invoices_type ON invoices(type);
CREATE INDEX IF NOT EXISTS idx_invoices_created_at ON invoices(created_at);
"#;

// ====================
// FONCTIONS
// ====================

pub fn apply_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA_SQL)?;
    Ok(())
}

pub fn get_all_table_names() -> Vec<&'static str> {
    let mut all = Vec::new();
    all.extend_from_slice(TABLE_BASE);
    all.extend_from_slice(TABLE_ZANDO);
    all.extend_from_slice(TABLE_FINANCE);
    all.extend_from_slice(TABLE_SOCIAL);
    all.extend_from_slice(TABLE_NOTIFICATIONS);
    all.extend_from_slice(TABLE_SECURITY);
    all.extend_from_slice(TABLE_TREASURY);
    all.extend_from_slice(TABLE_COUNTER);
    all
}
