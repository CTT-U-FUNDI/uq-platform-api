// ====================
// TREASURY HANDLERS — 10 ROUTES
// ====================

use actix_web::{web, HttpResponse};
use serde_json::json;
use rusqlite::params;

/// GET /api/treasury/status
pub async fn treasury_status(req: actix_web::HttpRequest) -> HttpResponse {
    // 🔐 SÉCURITÉ (vague 1) — OWNER uniquement
    if let Err(r) = crate::auth_guard::exiger_owner(&req) { return r; }

    let db_path = crate::uq_index::uq_index_path();
    let conn = match rusqlite::Connection::open(db_path) {
        Ok(c) => c,
        Err(e) => return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("DB error: {}", e)
        })),
    };

    let mut stmt = match conn.prepare(
        "SELECT profile_name, balance_borb, balance_usd, percentage FROM treasury_system
         UNION ALL
         SELECT 'dev_profile', balance_borb, balance_usd, percentage FROM dev_profile"
    ) {
        Ok(s) => s,
        Err(_) => return HttpResponse::Ok().json(json!({
            "status": "success",
            "treasury": {},
            "dev": {},
            "total_borb": 0.0
        })),
    };

    let mut treasury = json!({});
    let mut dev = json!({});
    let mut total_borb = 0.0;

    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, f64>(1)?,
            row.get::<_, f64>(2)?,
            row.get::<_, f64>(3)?,
        ))
    });

    if let Ok(r) = rows {
        for row in r.flatten() {
            let (name, borb, usd, pct) = row;
            total_borb += borb;
            if name == "dev_profile" {
                dev = json!({ "balance_borb": borb, "balance_usd": usd, "percentage": pct });
            } else {
                treasury = json!({ "profile_name": name, "balance_borb": borb, "balance_usd": usd, "percentage": pct });
            }
        }
    }

    HttpResponse::Ok().json(json!({
        "status": "success",
        "treasury": treasury,
        "dev": dev,
        "total_borb": total_borb,
        "invariant": 40_000_000_000.0
    }))
}

/// GET /api/treasury/sales
pub async fn treasury_sales() -> HttpResponse {
    let db_path = crate::uq_index::uq_index_path();
    let conn = match rusqlite::Connection::open(db_path) {
        Ok(c) => c,
        Err(e) => return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("DB error: {}", e)
        })),
    };

    let mut stmt = match conn.prepare(
        "SELECT id, amount_borb, crypto_type, crypto_amount, rate_at_sale, status, created_at
         FROM treasury_system_sales ORDER BY created_at DESC LIMIT 50"
    ) {
        Ok(s) => s,
        Err(_) => return HttpResponse::Ok().json(json!({ "status": "success", "sales": [] })),
    };

    let rows = stmt.query_map([], |row| {
        Ok(json!({
            "id": row.get::<_, i64>(0)?,
            "amount_borb": row.get::<_, f64>(1)?,
            "crypto_type": row.get::<_, String>(2)?,
            "crypto_amount": row.get::<_, f64>(3)?,
            "rate": row.get::<_, f64>(4)?,
            "status": row.get::<_, String>(5)?,
            "created_at": row.get::<_, i64>(6)?,
        }))
    });

    let mut sales = Vec::new();
    if let Ok(r) = rows {
        for row in r.flatten() { sales.push(row); }
    }

    HttpResponse::Ok().json(json!({ "status": "success", "sales": sales }))
}

/// GET /api/treasury/revenue
pub async fn treasury_revenue() -> HttpResponse {
    let db_path = crate::uq_index::uq_index_path();
    let conn = match rusqlite::Connection::open(db_path) {
        Ok(c) => c,
        Err(e) => return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("DB error: {}", e)
        })),
    };

    let mut stmt = match conn.prepare(
        "SELECT profile_name, SUM(amount_borb), SUM(amount_usd) FROM treasury_revenue GROUP BY profile_name"
    ) {
        Ok(s) => s,
        Err(_) => return HttpResponse::Ok().json(json!({
            "status": "success",
            "treasury": 0.0,
            "dev": 0.0,
            "total": 0.0
        })),
    };

    let mut treasury_total = 0.0;
    let mut dev_total = 0.0;

    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, f64>(1).unwrap_or(0.0),
            row.get::<_, f64>(2).unwrap_or(0.0),
        ))
    });

    if let Ok(r) = rows {
        for row in r.flatten() {
            let (name, borb, _usd) = row;
            if name == "dev_profile" { dev_total += borb; } else { treasury_total += borb; }
        }
    }

    HttpResponse::Ok().json(json!({
        "status": "success",
        "treasury": treasury_total,
        "dev": dev_total,
        "total": treasury_total + dev_total
    }))
}

/// GET /api/treasury/system/profiles
pub async fn treasury_system_profiles() -> HttpResponse {
    let db_path = crate::uq_index::uq_index_path();
    let conn = match rusqlite::Connection::open(db_path) {
        Ok(c) => c,
        Err(e) => return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("DB error: {}", e)
        })),
    };

    // Si la table est vide, on retourne les 6 profils par defaut
    let mut stmt = match conn.prepare(
        "SELECT code, display_name, icon, description, is_active FROM system_profiles"
    ) {
        Ok(s) => s,
        Err(_) => return HttpResponse::Ok().json(json!({
            "status": "success",
            "profiles": default_profiles()
        })),
    };

    let rows = stmt.query_map([], |row| {
        Ok(json!({
            "code": row.get::<_, String>(0)?,
            "display_name": row.get::<_, String>(1)?,
            "icon": row.get::<_, String>(2).unwrap_or_default(),
            "description": row.get::<_, String>(3).unwrap_or_default(),
            "is_active": row.get::<_, i64>(4).unwrap_or(1) == 1,
        }))
    });

    let mut profiles = Vec::new();
    if let Ok(r) = rows {
        for row in r.flatten() { profiles.push(row); }
    }

    if profiles.is_empty() {
        return HttpResponse::Ok().json(json!({
            "status": "success",
            "profiles": default_profiles()
        }));
    }

    HttpResponse::Ok().json(json!({ "status": "success", "profiles": profiles }))
}

fn default_profiles() -> Vec<serde_json::Value> {
    vec![
        json!({"code": "uq_trading", "display_name": "UQ Trading", "icon": "💱", "description": "Ventes automatiques BORB", "is_active": true}),
        json!({"code": "uq_ia", "display_name": "UQ IA", "icon": "🧠", "description": "Assistance intelligente", "is_active": true}),
        json!({"code": "uq_marche", "display_name": "UQ Marché", "icon": "📢", "description": "Publicités officielles", "is_active": true}),
        json!({"code": "uq_guide", "display_name": "UQ Guide", "icon": "📖", "description": "Guides et tutoriels", "is_active": true}),
        json!({"code": "uq_info", "display_name": "UQ Info", "icon": "ℹ️", "description": "Informations officielles", "is_active": true}),
        json!({"code": "uq_alerte", "display_name": "UQ Alerte", "icon": "⚠️", "description": "Mises en garde", "is_active": true}),
    ]
}

/// GET /api/treasury/system/messages
pub async fn treasury_system_messages() -> HttpResponse {
    let db_path = crate::uq_index::uq_index_path();
    let conn = match rusqlite::Connection::open(db_path) {
        Ok(c) => c,
        Err(e) => return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("DB error: {}", e)
        })),
    };

    let mut stmt = match conn.prepare(
        "SELECT id, profile_code, title, content, type, priority, created_at
         FROM system_messages WHERE is_active = 1 ORDER BY priority DESC, created_at DESC LIMIT 50"
    ) {
        Ok(s) => s,
        Err(_) => return HttpResponse::Ok().json(json!({ "status": "success", "messages": [] })),
    };

    let rows = stmt.query_map([], |row| {
        Ok(json!({
            "id": row.get::<_, i64>(0)?,
            "profile_code": row.get::<_, String>(1)?,
            "title": row.get::<_, String>(2)?,
            "content": row.get::<_, String>(3)?,
            "type": row.get::<_, String>(4).unwrap_or("info".to_string()),
            "priority": row.get::<_, i64>(5).unwrap_or(0),
            "created_at": row.get::<_, i64>(6)?,
        }))
    });

    let mut messages = Vec::new();
    if let Ok(r) = rows {
        for row in r.flatten() { messages.push(row); }
    }

    HttpResponse::Ok().json(json!({ "status": "success", "messages": messages }))
}

/// GET /api/treasury/dev/ads
pub async fn treasury_dev_ads() -> HttpResponse {
    let db_path = crate::uq_index::uq_index_path();
    let conn = match rusqlite::Connection::open(db_path) {
        Ok(c) => c,
        Err(e) => return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("DB error: {}", e)
        })),
    };

    let mut stmt = match conn.prepare(
        "SELECT id, title, description, amount_borb, price_usd, status, created_at
         FROM dev_ads WHERE status = 'active' ORDER BY created_at DESC LIMIT 50"
    ) {
        Ok(s) => s,
        Err(_) => return HttpResponse::Ok().json(json!({ "status": "success", "ads": [] })),
    };

    let rows = stmt.query_map([], |row| {
        Ok(json!({
            "id": row.get::<_, i64>(0)?,
            "title": row.get::<_, String>(1)?,
            "description": row.get::<_, String>(2).unwrap_or_default(),
            "amount_borb": row.get::<_, f64>(3)?,
            "price_usd": row.get::<_, f64>(4)?,
            "status": row.get::<_, String>(5)?,
            "created_at": row.get::<_, i64>(6)?,
        }))
    });

    let mut ads = Vec::new();
    if let Ok(r) = rows {
        for row in r.flatten() { ads.push(row); }
    }

    HttpResponse::Ok().json(json!({ "status": "success", "ads": ads }))
}

/// POST /api/treasury/dev/ads/create
pub async fn treasury_dev_ads_create(body: web::Json<serde_json::Value>) -> HttpResponse {
    let title = body.get("title").and_then(|v| v.as_str()).unwrap_or("");
    let description = body.get("description").and_then(|v| v.as_str()).unwrap_or("");
    let amount_borb = body.get("amount_borb").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let price_usd = body.get("price_usd").and_then(|v| v.as_f64()).unwrap_or(0.0);

    if title.is_empty() || amount_borb <= 0.0 {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Titre et montant requis"
        }));
    }

    let db_path = crate::uq_index::uq_index_path();
    let conn = match rusqlite::Connection::open(db_path) {
        Ok(c) => c,
        Err(e) => return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("DB error: {}", e)
        })),
    };

    let now = chrono::Utc::now().timestamp();
    match conn.execute(
        "INSERT INTO dev_ads (title, description, amount_borb, price_usd, status, created_at)
         VALUES (?1, ?2, ?3, ?4, 'active', ?5)",
        params![title, description, amount_borb, price_usd, now]
    ) {
        Ok(_) => HttpResponse::Ok().json(json!({
            "status": "success",
            "message": "Annonce créée",
            "ad_id": conn.last_insert_rowid()
        })),
        Err(e) => HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("Insert error: {}", e)
        })),
    }
}

/// POST /api/treasury/dev/ads/delete
pub async fn treasury_dev_ads_delete(body: web::Json<serde_json::Value>) -> HttpResponse {
    let ad_id = body.get("ad_id").and_then(|v| v.as_i64()).unwrap_or(0);

    if ad_id <= 0 {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "ad_id requis"
        }));
    }

    let db_path = crate::uq_index::uq_index_path();
    let conn = match rusqlite::Connection::open(db_path) {
        Ok(c) => c,
        Err(e) => return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("DB error: {}", e)
        })),
    };

    match conn.execute("UPDATE dev_ads SET status = 'deleted' WHERE id = ?1", params![ad_id]) {
        Ok(_) => HttpResponse::Ok().json(json!({
            "status": "success",
            "message": "Annonce supprimée"
        })),
        Err(e) => HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("Delete error: {}", e)
        })),
    }
}

/// GET /api/treasury/wallets
pub async fn treasury_wallets() -> HttpResponse {
    let db_path = crate::uq_index::uq_index_path();
    let conn = match rusqlite::Connection::open(db_path) {
        Ok(c) => c,
        Err(e) => return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("DB error: {}", e)
        })),
    };

    let mut stmt = match conn.prepare(
        "SELECT id, crypto_type, wallet_address, label, is_default FROM treasury_wallets"
    ) {
        Ok(s) => s,
        Err(_) => return HttpResponse::Ok().json(json!({ "status": "success", "wallets": [] })),
    };

    let rows = stmt.query_map([], |row| {
        Ok(json!({
            "id": row.get::<_, i64>(0)?,
            "crypto_type": row.get::<_, String>(1)?,
            "wallet_address": row.get::<_, String>(2)?,
            "label": row.get::<_, String>(3).unwrap_or_default(),
            "is_default": row.get::<_, i64>(4).unwrap_or(0) == 1,
        }))
    });

    let mut wallets = Vec::new();
    if let Ok(r) = rows {
        for row in r.flatten() { wallets.push(row); }
    }

    HttpResponse::Ok().json(json!({ "status": "success", "wallets": wallets }))
}

/// POST /api/treasury/wallets/add
pub async fn treasury_wallets_add(req: actix_web::HttpRequest, body: web::Json<serde_json::Value>) -> HttpResponse {
    // 🔐 Le guard de scope laisse passer tout JWT sur cette route (pas d'UQ dans le chemin) :
    // vérification owner explicite ici.
    if let Err(r) = crate::auth_guard::exiger_owner(&req) { return r; }

    let crypto_type = body.get("crypto_type").and_then(|v| v.as_str()).unwrap_or("");
    let wallet_address = body.get("wallet_address").and_then(|v| v.as_str()).unwrap_or("");
    let label = body.get("label").and_then(|v| v.as_str()).unwrap_or("");

    if crypto_type.is_empty() || wallet_address.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "crypto_type et wallet_address requis"
        }));
    }

    let db_path = crate::uq_index::uq_index_path();
    let conn = match rusqlite::Connection::open(db_path) {
        Ok(c) => c,
        Err(e) => return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("DB error: {}", e)
        })),
    };

    let now = chrono::Utc::now().timestamp();
    match conn.execute(
        "INSERT INTO treasury_wallets (crypto_type, wallet_address, label, is_default, created_at)
         VALUES (?1, ?2, ?3, 0, ?4)",
        params![crypto_type, wallet_address, label, now]
    ) {
        Ok(_) => HttpResponse::Ok().json(json!({
            "status": "success",
            "message": "Wallet ajouté",
            "wallet_id": conn.last_insert_rowid()
        })),
        Err(e) => HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("Insert error: {}", e)
        })),
    }
}

/// GET /api/owner/frais — taux fiat/crypto actuels
pub async fn owner_frais_get(_req: actix_web::HttpRequest) -> actix_web::HttpResponse {
    actix_web::HttpResponse::Ok().json(crate::frais_plateforme::taux_actuels())
}

/// POST /api/owner/frais {"type":"fiat"|"crypto","taux":0.0015}
pub async fn owner_frais_set(
    req: actix_web::HttpRequest,
    body: web::Json<serde_json::Value>,
) -> actix_web::HttpResponse {
    let ty = body.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let taux = body.get("taux").and_then(|v| v.as_f64()).unwrap_or(-1.0);
    match crate::frais_plateforme::definir_taux(ty, taux) {
        Ok(()) => actix_web::HttpResponse::Ok().json(serde_json::json!({
            "status":"success","frais":crate::frais_plateforme::taux_actuels()
        })),
        Err(e) => actix_web::HttpResponse::BadRequest().json(serde_json::json!({
            "status":"error","message":e
        })),
    }
}
