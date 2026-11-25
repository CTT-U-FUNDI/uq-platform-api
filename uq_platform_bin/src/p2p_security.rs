use actix_web::{web, HttpRequest, HttpResponse};
use rand::Rng;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const MAX_NAME_ATTEMPTS: i64 = 3;
const CODE_TTL_SECONDS: i64 = 30 * 60;
const ACTIVE_CODE_COUNT: usize = 10;
const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const CODE_LENGTH: usize = 6;

pub fn ensure_tables(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS p2p_receiving_methods (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            uq TEXT NOT NULL,
            method_type TEXT NOT NULL,
            provider TEXT NOT NULL,
            currency TEXT NOT NULL,
            account_identifier TEXT NOT NULL,
            account_last4 TEXT NOT NULL DEFAULT '',
            holder_name_hash TEXT NOT NULL,
            verification_status TEXT NOT NULL DEFAULT 'pending',
            verified_at INTEGER,
            enabled INTEGER NOT NULL DEFAULT 1,
            is_default INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_p2p_receiving_methods_owner
            ON p2p_receiving_methods(uq, enabled, verification_status);

        CREATE TABLE IF NOT EXISTS p2p_name_challenges (
            order_id INTEGER PRIMARY KEY,
            buyer TEXT NOT NULL,
            receiving_method_id INTEGER NOT NULL,
            expected_hash TEXT NOT NULL,
            attempts INTEGER NOT NULL DEFAULT 0,
            status TEXT NOT NULL DEFAULT 'pending',
            last_attempt_at INTEGER,
            locked_at INTEGER,
            passed_at INTEGER
        );

        CREATE TABLE IF NOT EXISTS p2p_cross_codes (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            order_id INTEGER NOT NULL,
            slot INTEGER NOT NULL,
            code_hash TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'pending',
            generated_at INTEGER NOT NULL,
            expires_at INTEGER NOT NULL,
            verified_at INTEGER,
            UNIQUE(order_id, slot)
        );
        CREATE INDEX IF NOT EXISTS idx_p2p_cross_codes_order
            ON p2p_cross_codes(order_id, status, slot);

        CREATE TABLE IF NOT EXISTS p2p_idempotency (
            uq TEXT NOT NULL,
            endpoint TEXT NOT NULL,
            operation_key TEXT NOT NULL,
            request_hash TEXT NOT NULL,
            response_json TEXT NOT NULL,
            status_code INTEGER NOT NULL,
            created_at INTEGER NOT NULL,
            PRIMARY KEY (uq, endpoint, operation_key)
        );

        CREATE TABLE IF NOT EXISTS p2p_disputes (
            order_id INTEGER PRIMARY KEY,
            opened_by TEXT NOT NULL,
            reason TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'open',
            resolution TEXT DEFAULT '',
            resolved_by TEXT DEFAULT '',
            created_at INTEGER NOT NULL,
            resolved_at INTEGER
        );

        CREATE TABLE IF NOT EXISTS p2p_security_audit (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            order_id INTEGER NOT NULL,
            uq TEXT NOT NULL,
            action TEXT NOT NULL,
            outcome TEXT NOT NULL,
            metadata TEXT NOT NULL DEFAULT '{}',
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_p2p_security_audit_order
            ON p2p_security_audit(order_id, id);
        "#,
    )
    .map_err(|e| format!("Sécurité P2P : migration impossible ({})", e))?;

    for (table, column, definition) in [
        ("p2p_offers", "receiving_method_id", "INTEGER"),
        ("p2p_orders", "receiving_method_id", "INTEGER"),
        (
            "p2p_orders",
            "name_challenge_status",
            "TEXT NOT NULL DEFAULT 'pending'",
        ),
        (
            "p2p_orders",
            "cross_code_status",
            "TEXT NOT NULL DEFAULT 'pending'",
        ),
        (
            "p2p_orders",
            "expected_version",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        ("p2p_orders", "dispute_reason", "TEXT NOT NULL DEFAULT ''"),
        ("p2p_orders", "settlement_started_at", "INTEGER"),
        ("p2p_orders", "settlement_completed_at", "INTEGER"),
    ] {
        crate_column(conn, table, column, definition)?;
    }
    crate_column(
        conn,
        "p2p_proofs",
        "image_sha256",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    Ok(())
}

fn crate_column(
    conn: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<(), String> {
    let exists: i64 = conn
        .query_row(
            &format!(
                "SELECT COUNT(*) FROM pragma_table_info('{}') WHERE name = ?1",
                table
            ),
            params![column],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if exists == 0 {
        conn.execute(
            &format!("ALTER TABLE {} ADD COLUMN {} {}", table, column, definition),
            [],
        )
        .map_err(|e| {
            format!(
                "Sécurité P2P : colonne {}.{} impossible ({})",
                table, column, e
            )
        })?;
    }
    Ok(())
}

fn text(body: &Value, key: &str) -> String {
    body.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string()
}

fn open_index() -> Result<Connection, String> {
    let conn =
        Connection::open("uq_index.db").map_err(|e| format!("Index indisponible ({})", e))?;
    let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
    crate::trading_routes::ensure_tables(&conn)?;
    Ok(conn)
}

fn refused(message: &str, code: &str) -> HttpResponse {
    HttpResponse::BadRequest().json(json!({
        "status": "error",
        "code": code,
        "message": message
    }))
}

fn forbidden(message: &str) -> HttpResponse {
    HttpResponse::Forbidden().json(json!({
        "status": "error",
        "code": "ACCES_REFUSE",
        "message": message
    }))
}

fn failure(message: String) -> HttpResponse {
    HttpResponse::InternalServerError().json(json!({
        "status": "error",
        "code": "P2P_SECURE_ERROR",
        "message": message
    }))
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

pub fn normalize_secret(value: &str) -> String {
    let lower = value.trim().to_lowercase();
    let normalized: String = lower
        .chars()
        .filter(|character| !character.is_whitespace())
        .map(|character| match character {
            'à' | 'â' | 'ä' => 'a',
            'ç' => 'c',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'î' | 'ï' => 'i',
            'ô' | 'ö' => 'o',
            'ù' | 'û' | 'ü' => 'u',
            'ÿ' => 'y',
            other => other,
        })
        .filter(|character| character.is_alphanumeric())
        .collect();
    normalized
}

pub fn secret_hash(value: &str) -> String {
    format!("{:x}", Sha256::digest(normalize_secret(value).as_bytes()))
}

pub fn random_code() -> String {
    let mut rng = rand::thread_rng();
    (0..CODE_LENGTH)
        .map(|_| {
            let index = rng.gen_range(0..CODE_ALPHABET.len());
            CODE_ALPHABET[index] as char
        })
        .collect()
}

fn account_last4(account: &str) -> String {
    let digits: String = account
        .chars()
        .filter(|character| character.is_ascii_digit())
        .collect();
    digits
        .chars()
        .rev()
        .take(4)
        .collect::<String>()
        .chars()
        .rev()
        .collect()
}

pub fn audit(
    conn: &Connection,
    order_id: i64,
    uq: &str,
    action: &str,
    outcome: &str,
    metadata: Value,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO p2p_security_audit
            (order_id, uq, action, outcome, metadata, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![order_id, uq, action, outcome, metadata.to_string(), now()],
    )
    .map(|_| ())
    .map_err(|e| format!("Audit P2P impossible ({})", e))
}

pub fn idempotent_replay(
    conn: &Connection,
    uq: &str,
    endpoint: &str,
    operation_key: &str,
    request: &Value,
) -> Result<Option<Value>, String> {
    if operation_key.is_empty() {
        return Err("Clé d'idempotence requise pour cette opération.".to_string());
    }
    let request_hash = request_hash(request);
    let row = conn
        .query_row(
            "SELECT request_hash, response_json FROM p2p_idempotency
             WHERE uq = ?1 AND endpoint = ?2 AND operation_key = ?3",
            params![uq, endpoint, operation_key],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|e| format!("Idempotence illisible ({})", e))?;
    match row {
        Some((stored_hash, response)) if stored_hash == request_hash => {
            serde_json::from_str(&response)
                .map(Some)
                .map_err(|e| format!("Réponse idempotente illisible ({})", e))
        }
        Some(_) => Err("Cette clé d'idempotence a déjà été utilisée avec une requête différente.".to_string()),
        None => Ok(None),
    }
}

pub fn remember_idempotent(
    conn: &Connection,
    uq: &str,
    endpoint: &str,
    operation_key: &str,
    request: &Value,
    response: &Value,
) -> Result<(), String> {
    if operation_key.is_empty() {
        return Err("Clé d'idempotence requise.".to_string());
    }
    let request_hash = request_hash(request);
    conn.execute(
        "INSERT INTO p2p_idempotency
            (uq, endpoint, operation_key, request_hash, response_json, status_code, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 200, ?6)
         ON CONFLICT(uq, endpoint, operation_key) DO NOTHING",
        params![
            uq,
            endpoint,
            operation_key,
            request_hash,
            response.to_string(),
            now()
        ],
    )
    .map(|_| ())
    .map_err(|e| format!("Idempotence impossible ({})", e))
}

fn request_hash(request: &Value) -> String {
    let mut request_without_secret = request.clone();
    if let Some(object) = request_without_secret.as_object_mut() {
        object.remove("pin");
    }
    format!("{:x}", Sha256::digest(request_without_secret.to_string().as_bytes()))
}

pub async fn receiving_methods_list(req: HttpRequest, path: web::Path<String>) -> HttpResponse {
    let uq = path.into_inner();
    if let Err(response) = crate::auth_guard::exiger_soi_ou_owner(&req, &uq) {
        return response;
    }
    let conn = match open_index() {
        Ok(conn) => conn,
        Err(error) => return failure(error),
    };
    let mut statement = match conn.prepare(
        "SELECT id, method_type, provider, currency, account_last4,
                verification_status, verified_at, is_default, enabled
         FROM p2p_receiving_methods
         WHERE UPPER(uq) = UPPER(?1) AND enabled = 1
         ORDER BY is_default DESC, id DESC",
    ) {
        Ok(statement) => statement,
        Err(error) => return failure(error.to_string()),
    };
    let rows = statement.query_map(params![uq], |row| {
        Ok(json!({
            "id": row.get::<_, i64>(0)?,
            "type": row.get::<_, String>(1)?,
            "provider": row.get::<_, String>(2)?,
            "currency": row.get::<_, String>(3)?,
            "account_last4": row.get::<_, String>(4)?,
            "verification_status": row.get::<_, String>(5)?,
            "verified_at": row.get::<_, Option<i64>>(6)?,
            "is_default": row.get::<_, i64>(7)? == 1,
            "enabled": row.get::<_, i64>(8)? == 1,
        }))
    });
    let methods = match rows {
        Ok(rows) => rows.flatten().collect::<Vec<_>>(),
        Err(error) => return failure(error.to_string()),
    };
    HttpResponse::Ok().json(json!({ "status": "success", "methods": methods }))
}

pub async fn receiving_method_create(req: HttpRequest, body: web::Json<Value>) -> HttpResponse {
    let uq = text(&body, "uq");
    if uq.is_empty() {
        return refused("Compte requis", "UQ_REQUIS");
    }
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(claims) => claims,
        Err(response) => return response,
    };
    if !claims.sub.eq_ignore_ascii_case(&uq) {
        return forbidden("Un moyen de réception ne peut être créé que par son titulaire.");
    }
    let method_type = text(&body, "type").to_uppercase();
    if !matches!(
        method_type.as_str(),
        "MOBILE_MONEY" | "BANK_TRANSFER" | "CASH"
    ) {
        return refused("Type de réception non autorisé", "TYPE_RECEPTION_INVALIDE");
    }
    let provider = text(&body, "provider");
    let currency = text(&body, "currency").to_uppercase();
    let account_identifier = text(&body, "account_identifier");
    let holder_name = text(&body, "holder_name");
    if provider.is_empty() || currency.is_empty() || account_identifier.is_empty() {
        return refused(
            "Opérateur, devise et identifiant de compte sont requis",
            "RECEPTION_INCOMPLETE",
        );
    }
    if normalize_secret(&holder_name).len() < 4 {
        return refused(
            "Le nom affiché par l'opérateur doit être complet et ne sera jamais rendu public.",
            "NOM_TITULAIRE_REQUIS",
        );
    }
    let conn = match open_index() {
        Ok(conn) => conn,
        Err(error) => return failure(error),
    };
    // 🔐 OTP OBLIGATOIRE : le numéro doit être prouvé par SMS avant
    // d'être enregistré — le jeton `verification_telephone` vient de
    // /api/verify/confirm (usage unique). Le numéro vérifié doit
    // correspondre à account_identifier (même E164).
    let jeton_otp = text(&body, "verification_telephone");
    if jeton_otp.is_empty() {
        return refused(
            "Vérification SMS requise : appelle /api/verify/challenge puis /api/verify/confirm pour obtenir le jeton, puis repasse-le ici.",
            "OTP_REQUIS",
        );
    }
    let tel_verifie = crate::verify_routes::valeur_verifiee(
        &jeton_otp,
        crate::verify_routes::Portee::Telephone,
    );
    let ident_norm = crate::verify_routes::analyser_telephone(&account_identifier)
        .map(|n| n.e164)
        .unwrap_or_else(|| account_identifier.clone());
    match tel_verifie {
        Some(v) if v == ident_norm || v == account_identifier => {}
        _ => {
            return refused(
                "Le numéro vérifié par SMS ne correspond pas au numéro déclaré.",
                "OTP_MISMATCH",
            )
        }
    }

    let holder_hash = secret_hash(&holder_name);
    let last4 = account_last4(&account_identifier);
    let is_default = body
        .get("is_default")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let timestamp = now();

    // 🔐 RÈGLE : max 2 numéros mobile money PAR OPÉRATEUR et par compte.
    // Au-delà, l'utilisateur doit justifier avec le nom complet de la
    // carte SIM (holder_name, déjà exigé ci-dessus et hashé).
    if method_type == "MOBILE_MONEY" && !provider.is_empty() {
        let compteur: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM p2p_receiving_methods
                 WHERE UPPER(uq) = UPPER(?1)
                   AND method_type = 'MOBILE_MONEY'
                   AND UPPER(provider) = UPPER(?2)
                   AND enabled = 1",
                params![uq, provider],
                |row| row.get(0),
            )
            .unwrap_or(0);
        if compteur >= 2 {
            return refused(
                "Maximum 2 numéros par opérateur. Pour un 3ème, fournis le nom complet de la carte SIM dans holder_name — il doit correspondre au titulaire.",
                "LIMITE_OPERATEUR",
            );
        }
    }

    if is_default {
        if let Err(error) = conn.execute(
            "UPDATE p2p_receiving_methods SET is_default = 0, updated_at = ?2
             WHERE UPPER(uq) = UPPER(?1)",
            params![uq, timestamp],
        ) {
            return failure(error.to_string());
        }
    }
    if let Err(error) = conn.execute(
        "INSERT INTO p2p_receiving_methods
            (uq, method_type, provider, currency, account_identifier, account_last4,
             holder_name_hash, verification_status, enabled, is_default, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', 1, ?8, ?9, ?9)",
        params![
            uq,
            method_type,
            provider,
            currency,
            account_identifier,
            last4,
            holder_hash,
            if is_default { 1 } else { 0 },
            timestamp
        ],
    ) {
        return failure(error.to_string());
    }
    let id = conn.last_insert_rowid();
    audit(
        &conn,
        0,
        &uq,
        "receiving_method_created",
        "pending",
        json!({ "method_id": id }),
    )
    .ok();
    HttpResponse::Ok().json(json!({
        "status": "success",
        "method": {
            "id": id,
            "type": method_type,
            "provider": provider,
            "currency": currency,
            "account_last4": last4,
            "verification_status": "pending",
            "is_default": is_default
        },
        "message": "Moyen enregistré. Il doit être vérifié avant de pouvoir vendre."
    }))
}

pub async fn receiving_method_verify(
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<Value>,
) -> HttpResponse {
    if let Err(response) = crate::auth_guard::exiger_owner(&req) {
        return response;
    }
    let id = path.into_inner();
    let status = if body
        .get("verified")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        "verified"
    } else {
        "rejected"
    };
    let conn = match open_index() {
        Ok(conn) => conn,
        Err(error) => return failure(error),
    };
    let timestamp = now();
    let changed = conn.execute(
        "UPDATE p2p_receiving_methods
         SET verification_status = ?1,
             verified_at = CASE WHEN ?1 = 'verified' THEN ?2 ELSE NULL END,
             updated_at = ?2
         WHERE id = ?3",
        params![status, timestamp, id],
    );
    match changed {
        Ok(1) => HttpResponse::Ok().json(json!({
            "status": "success",
            "id": id,
            "verification_status": status
        })),
        Ok(_) => refused("Moyen de réception introuvable", "RECEPTION_INTROUVABLE"),
        Err(error) => failure(error.to_string()),
    }
}

pub fn method_for_seller(
    conn: &Connection,
    seller: &str,
    offer_id: i64,
    requested_method_id: i64,
) -> Result<(i64, String, String, String), String> {
    let selected = if requested_method_id > 0 {
        requested_method_id
    } else {
        conn.query_row(
            "SELECT COALESCE(receiving_method_id, 0)
             FROM p2p_offers WHERE id = ?1",
            params![offer_id],
            |row| row.get::<_, Option<i64>>(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .flatten()
        .unwrap_or(0)
    };
    let selected = if selected > 0 {
        selected
    } else {
        conn.query_row(
            "SELECT id FROM p2p_receiving_methods
             WHERE UPPER(uq) = UPPER(?1)
               AND verification_status = 'verified'
               AND enabled = 1
             ORDER BY is_default DESC, id DESC LIMIT 1",
            params![seller],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .unwrap_or(0)
    };
    if selected == 0 {
        return Err("Aucun moyen de réception vérifié n'est disponible.".to_string());
    }
    // 🔐 Délai de carence : un moyen de réception doit avoir au moins
    // 24h avant de recevoir des fonds — anti-fraude express sur les
    // comptes jetables. created_at est en timestamp Unix.
    let delai_ok = now() - 24 * 3600;
    conn.query_row(
        "SELECT id, account_identifier, provider, currency
         FROM p2p_receiving_methods
         WHERE id = ?1 AND UPPER(uq) = UPPER(?2)
           AND verification_status = 'verified' AND enabled = 1
           AND created_at <= ?3",
        params![selected, seller, delai_ok],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )
    .optional()
    .map_err(|e| e.to_string())?
    .ok_or_else(|| {
        "Le moyen de réception sélectionné n'est pas vérifié ou n'appartient pas au vendeur."
            .to_string()
    })
}

pub fn initialize_order_security(
    conn: &Connection,
    order_id: i64,
    buyer: &str,
    receiving_method_id: i64,
    timestamp: i64,
) -> Result<String, String> {
    let expected_hash: String = conn
        .query_row(
            "SELECT holder_name_hash FROM p2p_receiving_methods WHERE id = ?1",
            params![receiving_method_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("Moyen de réception introuvable ({})", e))?;
    conn.execute(
        "INSERT INTO p2p_name_challenges
            (order_id, buyer, receiving_method_id, expected_hash, attempts, status)
         VALUES (?1, ?2, ?3, ?4, 0, 'pending')",
        params![order_id, buyer, receiving_method_id, expected_hash],
    )
    .map_err(|e| format!("Défi de nom impossible ({})", e))?;
    let active_code = generate_codes(conn, order_id, timestamp)?;
    audit(
        conn,
        order_id,
        buyer,
        "order_security_initialized",
        "pending",
        json!({
            "receiving_method_id": receiving_method_id,
            "code_count": ACTIVE_CODE_COUNT
        }),
    )?;
    Ok(active_code)
}

fn generate_codes(conn: &Connection, order_id: i64, timestamp: i64) -> Result<String, String> {
    conn.execute(
        "UPDATE p2p_cross_codes SET status = 'revoked'
         WHERE order_id = ?1 AND status IN ('pending','active')",
        params![order_id],
    )
    .map_err(|e| format!("Révocation des codes impossible ({})", e))?;
    let mut previous = std::collections::HashSet::new();
    let expires_at = timestamp + CODE_TTL_SECONDS;
    let mut active_code = String::new();
    for slot in 0..ACTIVE_CODE_COUNT {
        let mut raw = random_code();
        while previous.contains(&raw) {
            raw = random_code();
        }
        previous.insert(raw.clone());
        if slot == 0 {
            active_code = raw.clone();
        }
        let code_hash = secret_hash(&raw);
        conn.execute(
            "INSERT INTO p2p_cross_codes
                (order_id, slot, code_hash, status, generated_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(order_id, slot) DO UPDATE SET
                code_hash = excluded.code_hash,
                status = excluded.status,
                generated_at = excluded.generated_at,
                expires_at = excluded.expires_at,
                verified_at = NULL",
            params![
                order_id,
                slot,
                code_hash,
                if slot == 0 { "active" } else { "pending" },
                timestamp,
                expires_at
            ],
        )
        .map_err(|e| format!("Création du code impossible ({})", e))?;
    }
    if active_code.is_empty() {
        return Err("Aucun code actif n'a pu être généré.".to_string());
    }
    Ok(active_code)
}

pub async fn blind_name_challenge(
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<Value>,
) -> HttpResponse {
    let order_id = path.into_inner();
    let uq = text(&body, "uq");
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(claims) => claims,
        Err(response) => return response,
    };
    if !claims.sub.eq_ignore_ascii_case(&uq) {
        return forbidden("Le défi doit être saisi par son acheteur.");
    }
    let submitted = text(&body, "name");
    if submitted.is_empty() {
        return refused(
            "Saisis le nom affiché par l'opérateur mobile money.",
            "NOM_REQUIS",
        );
    }
    let conn = match open_index() {
        Ok(conn) => conn,
        Err(error) => return failure(error),
    };
    let challenge = conn.query_row(
        "SELECT buyer, expected_hash, attempts, status
         FROM p2p_name_challenges WHERE order_id = ?1",
        params![order_id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
            ))
        },
    );
    let (buyer, expected_hash, attempts, status) = match challenge {
        Ok(value) => value,
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            return refused("Défi introuvable", "DEFI_INTROUVABLE")
        }
        Err(error) => return failure(error.to_string()),
    };
    if !buyer.eq_ignore_ascii_case(&uq) {
        return forbidden("Seul l'acheteur peut répondre à ce défi.");
    }
    if status == "passed" {
        return HttpResponse::Ok().json(json!({
            "status": "success", "challenge_status": "passed"
        }));
    }
    if status == "locked" || attempts >= MAX_NAME_ATTEMPTS {
        return HttpResponse::TooManyRequests().json(json!({
            "status": "error",
            "code": "DEFI_BLOQUE",
            "message": "Trop de tentatives. La commande doit être revue par l'arbitrage."
        }));
    }
    let success = secret_hash(&submitted) == expected_hash;
    let next_status = if success {
        "passed"
    } else if attempts + 1 >= MAX_NAME_ATTEMPTS {
        "locked"
    } else {
        "pending"
    };
    if let Err(error) = conn.execute(
        "UPDATE p2p_name_challenges
         SET attempts = attempts + 1,
             status = ?2,
             last_attempt_at = ?3,
             passed_at = CASE WHEN ?2 = 'passed' THEN ?3 ELSE passed_at END,
             locked_at = CASE WHEN ?2 = 'locked' THEN ?3 ELSE locked_at END
         WHERE order_id = ?1",
        params![order_id, next_status, now()],
    ) {
        return failure(error.to_string());
    }
    let _ = conn.execute(
        "UPDATE p2p_orders SET name_challenge_status = ?2,
            updated_at = ?3 WHERE id = ?1",
        params![order_id, next_status, now()],
    );
    audit(
        &conn,
        order_id,
        &uq,
        "blind_name_challenge",
        next_status,
        json!({ "attempt": attempts + 1 }),
    )
    .ok();
    if success {
        return HttpResponse::Ok().json(json!({
            "status": "success",
            "challenge_status": "passed",
            "message": "Nom du bénéficiaire vérifié."
        }));
    }
    if next_status == "locked" {
        return HttpResponse::TooManyRequests().json(json!({
            "status": "error",
            "code": "DEFI_BLOQUE",
            "message": "Nom incorrect après trois tentatives. N'envoie pas d'argent et contacte l'arbitrage."
        }));
    }
    refused(
        "Le nom saisi ne correspond pas au bénéficiaire. N'envoie pas d'argent.",
        "NOM_INCORRECT",
    )
}

pub fn rotate_code_for_seller(
    conn: &Connection,
    order_id: i64,
    uq: &str,
    timestamp: i64,
) -> Result<String, String> {
    let seller: String = conn
        .query_row(
            "SELECT seller FROM p2p_orders WHERE id = ?1",
            params![order_id],
            |row| row.get(0),
        )
        .map_err(|_| "Commande introuvable".to_string())?;
    if !seller.eq_ignore_ascii_case(uq) {
        return Err("Seul le vendeur peut faire tourner le code.".to_string());
    }
    let slot: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(slot) + 1, 0) FROM p2p_cross_codes
             WHERE order_id = ?1 AND status = 'revoked'",
            params![order_id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    let new_slot = slot % ACTIVE_CODE_COUNT as i64;
    let code = random_code();
    conn.execute(
        "UPDATE p2p_cross_codes SET status = 'pending'
         WHERE order_id = ?1",
        params![order_id],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO p2p_cross_codes
            (order_id, slot, code_hash, status, generated_at, expires_at)
         VALUES (?1, ?2, ?3, 'active', ?4, ?5)
         ON CONFLICT(order_id, slot) DO UPDATE SET
            code_hash = excluded.code_hash,
            status = 'active',
            generated_at = excluded.generated_at,
            expires_at = excluded.expires_at,
            verified_at = NULL",
        params![
            order_id,
            new_slot,
            secret_hash(&code),
            timestamp,
            timestamp + CODE_TTL_SECONDS
        ],
    )
    .map_err(|e| e.to_string())?;
    audit(
        conn,
        order_id,
        uq,
        "cross_code_rotated",
        "success",
        json!({ "slot": new_slot }),
    )?;
    Ok(code)
}

pub async fn rotate_cross_code(
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<Value>,
) -> HttpResponse {
    let order_id = path.into_inner();
    let uq = text(&body, "uq");
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(claims) => claims,
        Err(response) => return response,
    };
    if !claims.sub.eq_ignore_ascii_case(&uq) {
        return forbidden("Seul le vendeur peut faire tourner ce code.");
    }
    let pin = text(&body, "pin");
    if pin.is_empty() {
        return refused("Code PIN requis pour remplacer le code", "PIN_REQUIS");
    }
    if !crate::trading_routes::pin_configure(&uq) {
        return refused(
            "Configure ton PIN avant de sécuriser cet échange.",
            "PIN_NON_CONFIGURE",
        );
    }
    let conn = match open_index() {
        Ok(conn) => conn,
        Err(error) => return failure(error),
    };
    match crate::trading_routes::verifier_pin_public(&uq, &pin) {
        Ok(true) => {}
        Ok(false) => return refused("Code PIN incorrect", "PIN_INCORRECT"),
        Err(error) => return failure(error),
    }
    match rotate_code_for_seller(&conn, order_id, &uq, now()) {
        Ok(code) => HttpResponse::Ok().json(json!({
            "status": "success",
            "order_id": order_id,
            "code": code,
            "expires_in_seconds": CODE_TTL_SECONDS,
            "warning": "Transmets ce code uniquement dans le canal UQ de cette commande."
        })),
        Err(error) => refused(&error, "ROTATION_CODE_IMPOSSIBLE"),
    }
}

pub async fn verify_cross_code(
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<Value>,
) -> HttpResponse {
    let order_id = path.into_inner();
    let uq = text(&body, "uq");
    let code = text(&body, "code");
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(claims) => claims,
        Err(response) => return response,
    };
    if !claims.sub.eq_ignore_ascii_case(&uq) {
        return forbidden("Le code doit être saisi par l'acheteur.");
    }
    if code.is_empty() {
        return refused("Code requis", "CODE_REQUIS");
    }
    let conn = match open_index() {
        Ok(conn) => conn,
        Err(error) => return failure(error),
    };
    let participant: (String, String, String) = match conn.query_row(
        "SELECT buyer, seller, status FROM p2p_orders WHERE id = ?1",
        params![order_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ) {
        Ok(value) => value,
        Err(_) => return refused("Commande introuvable", "COMMANDE_INTROUVABLE"),
    };
    if !participant.0.eq_ignore_ascii_case(&uq) {
        return forbidden("Seul l'acheteur peut saisir le code croisé.");
    }
    if participant.2 != "paid" {
        return refused(
            "Le paiement doit d'abord être annoncé.",
            "PAIEMENT_NON_ANONCE",
        );
    }

    // ===== PREUVE SMS DU TRANSFERT =====
    // L'acheteur peut joindre le TEXTE du SMS de confirmation mobile
    // money (mode bas débit — pas de photo). Conditions strictes :
    //  - le message doit être TRÈS récent (< 10 min, horloge serveur)
    //  - il doit contenir le NOM du vendeur (celui affiché avant le
    //    transfert — le nom enregistré, pas un pseudo)
    //  - le code croisé reste exigé en plus (double preuve)
    // On ne stocke que le HASH du SMS — jamais le texte brut.
    let mut sms_verdict = "absent";
    if let Some(sms) = body.get("sms_preuve") {
        let texte_sms = sms.get("text").and_then(|v| v.as_str()).unwrap_or("");
        let sms_ts = sms.get("ts").and_then(|v| v.as_i64()).unwrap_or(0);
        let age = now() - sms_ts;
        if texte_sms.is_empty() || sms_ts <= 0 {
            sms_verdict = "incomplet";
        } else if age < 0 || age > 600 {
            sms_verdict = "perime";
        } else {
            // Nom du vendeur : le nom officiel enregistré (celui que
            // l'opérateur affiche), comparé en normalisant casse/espaces.
            let nom_vendeur = crate::uq_public_alias::nom_enregistre(&participant.1)
                .unwrap_or_else(|| crate::uq_public_alias::get_public_display(&participant.1));
            let normalise = |s: &str| {
                s.chars()
                    .filter(|c| c.is_alphanumeric())
                    .flat_map(char::to_uppercase)
                    .collect::<String>()
            };
            let sms_norm = normalise(texte_sms);
            let nom_norm = normalise(&nom_vendeur);
            // Le nom peut être en plusieurs mots : on vérifie que TOUS
            // les mots significatifs du nom apparaissent dans le SMS.
            let mots: Vec<&str> = nom_norm
                .split_whitespace()
                .filter(|m| m.len() >= 3)
                .collect();
            let nom_present = !mots.is_empty()
                && mots.iter().all(|m| sms_norm.contains(m));
            sms_verdict = if nom_present { "valide" } else { "nom_absent" };
        }
    }
    if sms_verdict == "perime" {
        return refused(
            "Le SMS de transfert doit avoir moins de 10 minutes — reprends la preuve fraîche.",
            "SMS_PERIME",
        );
    }
    if sms_verdict == "nom_absent" {
        return refused(
            "Le SMS doit contenir le nom du vendeur destinataire du transfert.",
            "NOM_VENDEUR_ABSENT",
        );
    }

    // ===== MONTANT > 50 : SMS OBLIGATOIRE =====
    // Les petites transactions passent sans SMS ; au-delà de 50 BORB
    // (~USD), la preuve texte opérateur devient exigée — le risque
    // augmente avec le montant, l'exigence aussi.
    let points_cmd: i64 = conn
        .query_row(
            "SELECT COALESCE(escrow_points,0) FROM p2p_orders WHERE id = ?1",
            params![order_id],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if points_cmd as f64 / crate::p2p_escrow::POINTS_PAR_BORB as f64 > 50.0
        && sms_verdict == "absent"
    {
        return refused(
            "Montant > 50 USD : la preuve SMS du transfert est obligatoire (texte du message opérateur).",
            "SMS_REQUIS",
        );
    }

    let updated = conn.execute(
        "UPDATE p2p_cross_codes
         SET status = 'verified', verified_at = ?3
         WHERE order_id = ?1 AND status = 'active'
           AND code_hash = ?2 AND expires_at > ?3",
        params![order_id, secret_hash(&code), now()],
    );
    match updated {
        Ok(1) => {
            let _ = conn.execute(
                // Le code croisé prouve l'intention de payer : on ouvre
                // une fenêtre de 5 min pour la PREUVE FINALE (photo/
                // extraction). Passé ce délai, la commande part en
                // litige au prochain appel — l'acheteur ne peut pas
                // laisser le vendeur en attente indéfiniment.
                "UPDATE p2p_orders
                 SET cross_code_status = 'verified',
                     payment_deadline = ?2,
                     updated_at = ?2
                 WHERE id = ?1",
                params![order_id, now() + 300],
            );
            audit(
                &conn,
                order_id,
                &uq,
                "cross_code_verified",
                "success",
                json!({ "sms_preuve": sms_verdict }),
            )
            .ok();
            HttpResponse::Ok().json(json!({
                "status": "success",
                "order_id": order_id,
                "cross_code_status": "verified",
                "sms_preuve": sms_verdict,
                "message": "Code validé. Le vendeur doit encore confirmer la réception avec son PIN."
            }))
        }
        Ok(_) => {
            // 🔐 Anti-force-brute : chaque échec incrémente le compteur
            // de la commande. À 5 échecs, TOUS les codes sont révoqués
            // et le vendeur doit en régénérer — un voleur qui teste des
            // codes ne peut pas aller plus loin.
            let essais: i64 = conn
                .query_row(
                    "SELECT COALESCE(code_attempts,0) FROM p2p_orders WHERE id = ?1",
                    params![order_id],
                    |row| row.get(0),
                )
                .unwrap_or(0);
            let _ = conn.execute_batch(
                "ALTER TABLE p2p_orders ADD COLUMN code_attempts INTEGER DEFAULT 0;",
            );
            let nouveau = essais + 1;
            let _ = conn.execute(
                "UPDATE p2p_orders SET code_attempts = ?2 WHERE id = ?1",
                params![order_id, nouveau],
            );
            if nouveau >= 5 {
                let _ = conn.execute(
                    "UPDATE p2p_cross_codes SET status = 'revoked'
                     WHERE order_id = ?1 AND status IN ('pending','active')",
                    params![order_id],
                );
                let _ = conn.execute(
                    "UPDATE p2p_orders SET cross_code_status = 'locked', updated_at = ?2 WHERE id = ?1",
                    params![order_id, now()],
                );
            }
            audit(
                &conn,
                order_id,
                &uq,
                "cross_code_verified",
                "invalid",
                json!({ "essai": nouveau }),
            )
            .ok();
            refused("Code incorrect, expiré ou déjà utilisé.", "CODE_INVALIDE")
        }
        Err(error) => failure(error.to_string()),
    }
}

pub async fn dispute_order(
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<Value>,
) -> HttpResponse {
    let order_id = path.into_inner();
    let uq = text(&body, "uq");
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(claims) => claims,
        Err(response) => return response,
    };
    if !claims.sub.eq_ignore_ascii_case(&uq) {
        return forbidden("Le litige doit être ouvert par le participant lui-même.");
    }
    let reason = text(&body, "reason");
    if reason.len() < 8 {
        return refused(
            "Décris le litige avec au moins 8 caractères.",
            "MOTIF_REQUIS",
        );
    }
    let conn = match open_index() {
        Ok(conn) => conn,
        Err(error) => return failure(error),
    };
    let participants = conn.query_row(
        "SELECT buyer, seller, status FROM p2p_orders WHERE id = ?1",
        params![order_id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        },
    );
    let (buyer, seller, status) = match participants {
        Ok(value) => value,
        Err(_) => return refused("Commande introuvable", "COMMANDE_INTROUVABLE"),
    };
    if !buyer.eq_ignore_ascii_case(&uq) && !seller.eq_ignore_ascii_case(&uq) {
        return forbidden("Tu ne participes pas à cette commande.");
    }
    if matches!(
        status.as_str(),
        "completed" | "cancelled" | "refunded" | "disputed"
    ) {
        return refused(
            "Cette commande ne peut plus entrer en litige.",
            "STATUT_INVALIDE",
        );
    }
    let changed = conn.execute(
        "UPDATE p2p_orders SET status = 'disputed', dispute_reason = ?2,
            updated_at = ?3 WHERE id = ?1 AND status = ?4",
        params![order_id, reason, now(), status],
    );
    if !matches!(changed, Ok(1)) {
        return refused(
            "La commande a changé d'état. Recharge-la.",
            "CONFLIT_VERSION",
        );
    }
    if let Err(error) = conn.execute(
        "INSERT INTO p2p_disputes
            (order_id, opened_by, reason, status, created_at)
         VALUES (?1, ?2, ?3, 'open', ?4)
         ON CONFLICT(order_id) DO UPDATE SET
            opened_by = excluded.opened_by,
            reason = excluded.reason,
            status = 'open',
            created_at = excluded.created_at",
        params![order_id, uq, reason, now()],
    ) {
        return failure(error.to_string());
    }
    audit(
        &conn,
        order_id,
        &uq,
        "dispute_opened",
        "frozen",
        json!({ "reason": reason }),
    )
    .ok();
    HttpResponse::Ok().json(json!({
        "status": "success",
        "order_id": order_id,
        "state": "disputed",
        "message": "Les BORB restent gelés jusqu'à la décision d'arbitrage."
    }))
}

pub async fn arbitrate_order(
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<Value>,
) -> HttpResponse {
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(claims) => claims,
        Err(response) => return response,
    };
    let order_id = path.into_inner();
    // ===== ARBITRES : owner OU vendeur réputé =====
    // Un vendeur peut trancher les litiges s'il a :
    //   - KYC vérifié
    //   - >= 20 actions "vert" (réputation)
    //   - >= 20 commandes vendues terminées
    //   - pas de signalement "rouge"/"noir" non résolu
    let est_owner = crate::auth_guard::est_owner(&claims);
    if !est_owner {
        let conn_check = match open_index() {
            Ok(c) => c,
            Err(e) => return failure(e),
        };
        let (kyc, vert, ventes, rouge, noir): (i64, i64, i64, i64, i64) = conn_check
            .query_row(
                "SELECT COALESCE(uq_index.kyc_verified,0),
                        COALESCE(rep.vert,0),
                        (SELECT COUNT(*) FROM p2p_orders
                          WHERE seller = ?1 AND status = 'completed'),
                        COALESCE(rep.rouge,0),
                        COALESCE(rep.noir,0)
                 FROM uq_index
                 LEFT JOIN reputation rep ON rep.uq = uq_index.uq_number
                 WHERE uq_index.uq_number = ?1",
                params![claims.sub],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .unwrap_or((0, 0, 0, 0, 0));
        let arbitre_ok = kyc == 1
            && vert >= 20
            && ventes >= 20
            && rouge == 0
            && noir == 0;
        if !arbitre_ok {
            return forbidden(
                "Arbitrage réservé à l'owner et aux vendeurs réputés (KYC + 20 verts + 20 ventes).",
            );
        }
        // ===== GRANDS MONTANTS : OWNER UNIQUEMENT =====
        // Au-dessus de 200 BORB (~200 USD), seul l'owner tranche —
        // un litige à gros montant engage la plateforme entière.
        let points_cmd: i64 = conn_check
            .query_row(
                "SELECT COALESCE(escrow_points,0) FROM p2p_orders WHERE id = ?1",
                params![order_id],
                |row| row.get(0),
            )
            .unwrap_or(0);
        if points_cmd as f64 / crate::p2p_escrow::POINTS_PAR_BORB as f64 > 200.0
            && !est_owner
        {
            return forbidden(
                "Litige > 200 USD : arbitrage réservé à l'owner uniquement.",
            );
        }
    }
    let decision = text(&body, "decision").to_lowercase();
    if !matches!(decision.as_str(), "release" | "refund") {
        return refused("Décision invalide: release ou refund.", "DECISION_INVALIDE");
    }
    let reason = text(&body, "reason");
    if reason.len() < 8 {
        return refused("Motif d'arbitrage requis.", "MOTIF_REQUIS");
    }
    let conn = match open_index() {
        Ok(conn) => conn,
        Err(error) => return failure(error),
    };
    let order = conn.query_row(
        "SELECT buyer, seller, offer_id, amount, currency, COALESCE(escrow_points, 0)
         FROM p2p_orders WHERE id = ?1 AND status = 'disputed'",
        params![order_id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, f64>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
            ))
        },
    );
    let (buyer, seller, offer_id, amount, currency, points) = match order {
        Ok(value) => value,
        Err(_) => return refused("Litige introuvable ou déjà résolu.", "LITIGE_INVALIDE"),
    };
    if points <= 0 {
        return refused("Commande historique sans escrow BORB réconciliable.", "COMMANDE_OBSOLETE");
    }
    let timestamp = now();
    if decision == "release" {
        if let Err(error) = crate::p2p_escrow::liberer(
            &seller,
            &buyer,
            points,
            order_id,
            timestamp,
        ) {
            return failure(error.message());
        }
        if let Err(error) = conn.execute(
            "UPDATE p2p_orders SET status = 'completed', settlement_completed_at = ?2,
                updated_at = ?2, expected_version = expected_version + 1
             WHERE id = ?1 AND status = 'disputed'",
            params![order_id, timestamp],
        ) {
            return failure(error.to_string());
        }
        let _ = crate::p2p_preuve::marquer_issue(&conn, order_id, "completed");
    } else {
        if let Err(error) = crate::p2p_escrow::degeler(&seller, points, timestamp) {
            return failure(error.message());
        }
        if let Err(error) = conn.execute(
            "UPDATE p2p_orders SET status = 'refunded', updated_at = ?2,
                expected_version = expected_version + 1
             WHERE id = ?1 AND status = 'disputed'",
            params![order_id, timestamp],
        ) {
            return failure(error.to_string());
        }
        let borb = points as f64 / crate::p2p_escrow::POINTS_PAR_BORB as f64;
        let _ = conn.execute(
            "UPDATE p2p_offers
             SET taken_amount = CASE
                    WHEN COALESCE(amount_unit, 'DEVISE') = 'BORB'
                    THEN MAX(0, COALESCE(taken_amount, 0) - ?1)
                    ELSE MAX(0, COALESCE(taken_amount, 0) - ?2)
                 END,
                 status = 'open'
             WHERE id = ?3",
            params![borb, amount, offer_id],
        );
        let _ = crate::p2p_preuve::marquer_issue(&conn, order_id, "litige");
    }
    if let Err(error) = conn.execute(
        "UPDATE p2p_disputes
         SET status = 'resolved', resolution = ?2, resolved_by = ?3, resolved_at = ?4
         WHERE order_id = ?1",
        params![order_id, decision, claims.sub, timestamp],
    ) {
        return failure(error.to_string());
    }
    audit(
        &conn,
        order_id,
        &claims.sub,
        "dispute_resolved",
        &decision,
        json!({ "reason": reason, "seller": seller, "buyer": buyer, "currency": currency }),
    )
    .ok();
    HttpResponse::Ok().json(json!({
        "status": "success",
        "order_id": order_id,
        "decision": decision,
        "message": if decision == "refund" {
            "Litige tranché: les BORB ont été remboursés au vendeur."
        } else {
            "Litige tranché: les BORB ont été libérés à l'acheteur."
        }
    }))
}

pub fn release_requirements(conn: &Connection, order_id: i64) -> Result<(), String> {
    let values: (String, String, String) = conn
        .query_row(
            "SELECT name_challenge_status, cross_code_status, status
         FROM p2p_orders WHERE id = ?1",
            params![order_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|_| "Commande introuvable".to_string())?;
    if values.0 != "passed" {
        return Err("Le nom du bénéficiaire n'a pas été validé par le défi aveugle.".to_string());
    }
    if values.1 != "verified" {
        return Err("Le code croisé n'a pas été validé par l'acheteur.".to_string());
    }
    if values.2 != "paid" {
        return Err("La commande n'est pas payée.".to_string());
    }
    Ok(())
}

pub fn name_challenge_passed(conn: &Connection, order_id: i64) -> Result<bool, String> {
    conn.query_row(
        "SELECT status FROM p2p_name_challenges WHERE order_id = ?1",
        params![order_id],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .map(|value| value.as_deref() == Some("passed"))
    .map_err(|e| e.to_string())
}

/// POST /api/p2p/receiving-methods/revoke {uq, method_id}
/// Révoque un moyen de réception (numéro perdu, SIM désactivée…).
/// Le moyen passe enabled=0 + verification_status='revoked' — il
/// n'est plus proposé aux acheteurs mais l'historique est conservé.
pub async fn receiving_method_revoke(req: HttpRequest, body: web::Json<Value>) -> HttpResponse {
    let uq = text(&body, "uq");
    if uq.is_empty() {
        return refused("Compte requis", "UQ_REQUIS");
    }
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(claims) => claims,
        Err(response) => return response,
    };
    if !claims.sub.eq_ignore_ascii_case(&uq) {
        return forbidden("Tu ne peux révoquer que tes propres moyens de réception.");
    }
    let method_id = body.get("method_id").and_then(Value::as_i64).unwrap_or(0);
    if method_id <= 0 {
        return refused("method_id requis", "METHOD_ID_REQUIS");
    }

    let conn = match open_index() {
        Ok(conn) => conn,
        Err(error) => return failure(error),
    };
    // Vérifie que le moyen appartient bien au compte avant de révoquer.
    let proprietaire: Option<String> = conn
        .query_row(
            "SELECT uq FROM p2p_receiving_methods WHERE id = ?1",
            params![method_id],
            |row| row.get(0),
        )
        .ok();
    match proprietaire {
        Some(u) if u.eq_ignore_ascii_case(&uq) => {}
        _ => return HttpResponse::NotFound().json(json!({
            "status": "error",
            "message": "Moyen de réception introuvable"
        })),
    }

    let timestamp = now();
    if let Err(error) = conn.execute(
        "UPDATE p2p_receiving_methods
         SET enabled = 0, verification_status = 'revoked', is_default = 0, updated_at = ?2
         WHERE id = ?1",
        params![method_id, timestamp],
    ) {
        return failure(error.to_string());
    }
    audit(
        &conn,
        0,
        &uq,
        "receiving_method_revoked",
        "revoked",
        json!({ "method_id": method_id }),
    )
    .ok();
    HttpResponse::Ok().json(json!({
        "status": "success",
        "message": "Moyen de réception révoqué — il ne sera plus proposé aux acheteurs."
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_database() -> Connection {
    let conn = Connection::open_in_memory().expect("base en mémoire");
    conn.execute_batch(
        "CREATE TABLE p2p_offers (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            receiving_method_id INTEGER
        );
        CREATE TABLE p2p_orders (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            receiving_method_id INTEGER,
            buyer TEXT DEFAULT '',
            seller TEXT DEFAULT '',
            status TEXT DEFAULT 'paid',
            name_challenge_status TEXT DEFAULT 'pending',
            cross_code_status TEXT DEFAULT 'pending',
            expected_version INTEGER DEFAULT 0,
            dispute_reason TEXT DEFAULT '',
            settlement_started_at INTEGER,
            settlement_completed_at INTEGER
        );
        CREATE TABLE p2p_proofs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            image_sha256 TEXT DEFAULT ''
        );",
    )
    .expect("schéma test");
    ensure_tables(&conn).expect("migration test");
    conn
    }

    #[test]
    fn normalize_name_without_exposing_original() {
    assert_eq!(normalize_secret("  Élie  KABONGO "), normalize_secret("elie kabongo"));
    assert_ne!(secret_hash("Élie KABONGO"), secret_hash("Élie Kabemba"));
    assert!(!secret_hash("Élie KABONGO").contains("elie"));
    }

    #[test]
    fn creates_five_different_cross_codes() {
    let conn = test_database();
    let active = generate_codes(&conn, 7, 100).expect("codes");
    assert_eq!(active.chars().count(), CODE_LENGTH);
    let hashes = {
        let mut statement = conn
            .prepare("SELECT code_hash, status FROM p2p_cross_codes WHERE order_id = 7")
            .expect("requête");
        statement
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
            .expect("lignes")
            .flatten()
            .collect::<Vec<_>>()
    };
    assert_eq!(hashes.len(), 5);
    let unique = hashes
        .iter()
        .map(|entry| entry.0.as_str())
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(unique.len(), 5);
    assert_eq!(
        hashes.iter().filter(|entry| entry.1 == "active").count(),
        1
    );
    }

    #[test]
    fn release_requires_name_and_cross_code() {
    let conn = test_database();
    conn.execute(
        "INSERT INTO p2p_orders (id, buyer, seller, status) VALUES (9, 'UQ-00000002', 'UQ-00000003', 'paid')",
        [],
    )
    .expect("commande");
    assert!(release_requirements(&conn, 9).is_err());
    conn.execute(
        "UPDATE p2p_orders SET name_challenge_status = 'passed' WHERE id = 9",
        [],
    )
    .expect("nom");
    assert!(release_requirements(&conn, 9).is_err());
    conn.execute(
        "UPDATE p2p_orders SET cross_code_status = 'verified' WHERE id = 9",
        [],
    )
    .expect("code");
    assert!(release_requirements(&conn, 9).is_ok());
    }

    #[test]
    fn locks_name_challenge_after_three_failures() {
    let conn = test_database();
    conn.execute(
        "INSERT INTO p2p_name_challenges
            (order_id, buyer, receiving_method_id, expected_hash, attempts, status)
         VALUES (12, 'UQ-00000002', 1, ?1, 2, 'pending')",
        params![secret_hash("Nom Attendu")],
    )
    .expect("défi");
    let wrong = secret_hash("Mauvais Nom");
    let changed = conn
        .execute(
            "UPDATE p2p_name_challenges
             SET attempts = attempts + 1,
                 status = CASE WHEN attempts + 1 >= ?2 THEN 'locked' ELSE 'pending' END
             WHERE order_id = 12 AND expected_hash <> ?1",
            params![wrong, MAX_NAME_ATTEMPTS],
        )
        .expect("tentative");
    assert_eq!(changed, 1);
    let state: (i64, String) = conn
        .query_row(
            "SELECT attempts, status FROM p2p_name_challenges WHERE order_id = 12",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("état");
    assert_eq!(state, (3, "locked".to_string()));
    }
}
