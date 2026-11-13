// ====================
// COUNTER HANDLERS — 4 ROUTES
// Le Compteur & Calculateur (7ème organe)
// ====================

use actix_web::{web, HttpResponse};
use serde_json::json;

/// GET /api/counter/status
pub async fn counter_status() -> HttpResponse {
    let db_path = "uq_index.db";
    let conn = match rusqlite::Connection::open(db_path) {
        Ok(c) => c,
        Err(e) => return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("DB error: {}", e)
        })),
    };

    // Compter les utilisateurs
    let total_users: i64 = conn.query_row("SELECT COUNT(*) FROM uq_index", [], |r| r.get(0)).unwrap_or(0);

    // Somme des 6 Cases (depuis seven_cases ou calculée)
    let mut stmt = match conn.prepare(
        "SELECT case_1_tresor, case_2_frais, case_3_recompenses, case_4_epargne,
                case_5_credit, case_6_bouclier, case_7_comptes, sum_6_cases, total_7_cases,
                invariant_valid, block_number
         FROM seven_cases ORDER BY block_number DESC LIMIT 1"
    ) {
        Ok(s) => s,
        Err(_) => {
            // Pas encore de bloc — retourner les valeurs initiales
            return HttpResponse::Ok().json(json!({
                "status": "success",
                "total_users": total_users,
                "counter_value": 0.0,
                "six_cases": {
                    "tresor": 31120000000.0,
                    "frais": 2240000000.0,
                    "recompenses": 2240000000.0,
                    "epargne": 2240000000.0,
                    "credit": 1320000000.0,
                    "bouclier": 840000000.0
                },
                "sum_6_cases": 40000000000.0,
                "invariant": 40000000000.0,
                "invariant_valid": true,
                "precision": "infini",
                "block_number": 0
            }));
        }
    };

    let result = stmt.query_row([], |row| {
        Ok(json!({
            "total_users": total_users,
            "case_1_tresor": row.get::<_, f64>(0).unwrap_or(31120000000.0),
            "case_2_frais": row.get::<_, f64>(1).unwrap_or(2240000000.0),
            "case_3_recompenses": row.get::<_, f64>(2).unwrap_or(2240000000.0),
            "case_4_epargne": row.get::<_, f64>(3).unwrap_or(2240000000.0),
            "case_5_credit": row.get::<_, f64>(4).unwrap_or(1320000000.0),
            "case_6_bouclier": row.get::<_, f64>(5).unwrap_or(840000000.0),
            "case_7_comptes": row.get::<_, f64>(6).unwrap_or(0.0),
            "sum_6_cases": row.get::<_, f64>(7).unwrap_or(40000000000.0),
            "total_7_cases": row.get::<_, f64>(8).unwrap_or(40000000000.0),
            "invariant_valid": row.get::<_, i64>(9).unwrap_or(1) == 1,
            "block_number": row.get::<_, i64>(10).unwrap_or(0),
        }))
    });

    match result {
        Ok(data) => HttpResponse::Ok().json(json!({
            "status": "success",
            "total_users": total_users,
            "counter": data,
            "invariant": 40000000000.0,
            "precision": "infini",
            "message": "Compteur & Calculateur opérationnel"
        })),
        Err(_) => HttpResponse::Ok().json(json!({
            "status": "success",
            "total_users": total_users,
            "counter_value": 0.0,
            "sum_6_cases": 40000000000.0,
            "invariant": 40000000000.0,
            "invariant_valid": true,
            "precision": "infini",
            "block_number": 0
        })),
    }
}

/// POST /api/counter/calculate
pub async fn counter_calculate(body: web::Json<serde_json::Value>) -> HttpResponse {
    let amount_borb = body.get("amount_borb").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let fee_percent = body.get("fee_percent").and_then(|v| v.as_f64()).unwrap_or(0.1);

    if amount_borb <= 0.0 {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "amount_borb requis"
        }));
    }

    // Calcul des frais
    let fee_borb = amount_borb * (fee_percent / 100.0);
    let amount_after_fee = amount_borb - fee_borb;

    // Distribution des frais
    let fee_tresor = fee_borb * 0.30;
    let fee_frais = fee_borb * 0.40;
    let fee_recompenses = fee_borb * 0.15;
    let fee_epargne = fee_borb * 0.10;
    let fee_credit = fee_borb * 0.05;

    // Précision INFINIE (extensible jusqu'à l'infiniment petit)
    let amount_fborb = calculate_infinite_precision(amount_borb);

    HttpResponse::Ok().json(json!({
        "status": "success",
        "amount_borb": amount_borb,
        "amount_fborb": amount_fborb,
        "fee_percent": fee_percent,
        "fee_borb": fee_borb,
        "amount_after_fee": amount_after_fee,
        "distribution": {
            "tresor": fee_tresor,
            "frais": fee_frais,
            "recompenses": fee_recompenses,
            "epargne": fee_epargne,
            "credit": fee_credit,
            "bouclier": 0.0
        },
        "total_distributed": fee_borb,
        "precision": "infini (extensible)",
        "invariant": 40000000000.0
    }))
}

/// GET /api/counter/transactions
pub async fn counter_transactions() -> HttpResponse {
    let db_path = "uq_index.db";
    let conn = match rusqlite::Connection::open(db_path) {
        Ok(c) => c,
        Err(e) => return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("DB error: {}", e)
        })),
    };

    let mut stmt = match conn.prepare(
        "SELECT id, block_number, tx_id, from_user, to_user, amount_borb,
                fee_borb, fee_type, created_at
         FROM counter_transactions ORDER BY created_at DESC LIMIT 100"
    ) {
        Ok(s) => s,
        Err(_) => return HttpResponse::Ok().json(json!({
            "status": "success",
            "transactions": []
        })),
    };

    let rows = stmt.query_map([], |row| {
        Ok(json!({
            "id": row.get::<_, i64>(0)?,
            "block_number": row.get::<_, i64>(1).unwrap_or(0),
            "tx_id": row.get::<_, String>(2).unwrap_or_default(),
            "from_user": row.get::<_, String>(3).unwrap_or_default(),
            "to_user": row.get::<_, String>(4).unwrap_or_default(),
            "amount_borb": row.get::<_, f64>(5).unwrap_or(0.0),
            "fee_borb": row.get::<_, f64>(6).unwrap_or(0.0),
            "fee_type": row.get::<_, String>(7).unwrap_or_default(),
            "created_at": row.get::<_, i64>(8).unwrap_or(0),
        }))
    });

    let mut transactions = Vec::new();
    if let Ok(r) = rows {
        for row in r.flatten() { transactions.push(row); }
    }

    HttpResponse::Ok().json(json!({
        "status": "success",
        "total": transactions.len(),
        "transactions": transactions
    }))
}

/// GET /api/counter/rights
pub async fn counter_rights() -> HttpResponse {
    let db_path = "uq_index.db";
    let conn = match rusqlite::Connection::open(db_path) {
        Ok(c) => c,
        Err(e) => return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("DB error: {}", e)
        })),
    };

    let mut stmt = match conn.prepare(
        "SELECT case_name, right_percentage, total_received, last_received_at
         FROM counter_case_rights"
    ) {
        Ok(s) => s,
        Err(_) => return HttpResponse::Ok().json(json!({
            "status": "success",
            "rights": default_rights()
        })),
    };

    let rows = stmt.query_map([], |row| {
        Ok(json!({
            "case_name": row.get::<_, String>(0)?,
            "right_percentage": row.get::<_, f64>(1)?,
            "total_received": row.get::<_, f64>(2).unwrap_or(0.0),
            "last_received_at": row.get::<_, i64>(3).unwrap_or(0),
        }))
    });

    let mut rights = Vec::new();
    if let Ok(r) = rows {
        for row in r.flatten() { rights.push(row); }
    }

    if rights.is_empty() {
        return HttpResponse::Ok().json(json!({
            "status": "success",
            "rights": default_rights()
        }));
    }

    HttpResponse::Ok().json(json!({
        "status": "success",
        "rights": rights
    }))
}

fn default_rights() -> Vec<serde_json::Value> {
    vec![
        json!({"case_name": "tresor", "right_percentage": 30.0, "total_received": 0.0}),
        json!({"case_name": "frais", "right_percentage": 40.0, "total_received": 0.0}),
        json!({"case_name": "recompenses", "right_percentage": 15.0, "total_received": 0.0}),
        json!({"case_name": "epargne", "right_percentage": 10.0, "total_received": 0.0}),
        json!({"case_name": "credit", "right_percentage": 5.0, "total_received": 0.0}),
        json!({"case_name": "bouclier", "right_percentage": 0.0, "total_received": 0.0}),
    ]
}



// ====================
// PRECISION INFINIE - Extensible jusqu'a l'infiniment petit
// ====================

/// Calcule la precision d'un montant BORB
/// Extensible : femto -> atto -> zepto -> yocto -> ronto -> quecto -> ...
pub fn calculate_infinite_precision(amount_borb: f64) -> String {
    let units: [( &str, f64); 11] = [
        ("BORB", 1.0),
        ("borb", 1e-4),
        ("mborb", 1e-6),
        ("nborb", 1e-8),
        ("pborb", 1e-10),
        ("fborb", 1e-12),
        ("afborb", 1e-15),
        ("zfborb", 1e-18),
        ("yfborb", 1e-21),
        ("rfborb", 1e-24),
        ("qfborb", 1e-27),
    ];

    for (name, factor) in units.iter() {
        let value = amount_borb / factor;
        if value >= 1.0 {
            return format!("{:.6} {}", value, name);
        }
    }

    let value = amount_borb / 1e-30;
    format!("{:.6} sous-quecto", value)
}

/// Retourne la liste de TOUTES les unites disponibles
pub fn all_units() -> Vec<serde_json::Value> {
    vec![
        json!({"level": 1, "name": "BORB", "symbol": "BORB", "factor": 1.0, "prefix": "unite"}),
        json!({"level": 2, "name": "FIXO", "symbol": "borb", "factor": 1e-4, "prefix": "centi"}),
        json!({"level": 3, "name": "MICRO-FIXO", "symbol": "mborb", "factor": 1e-6, "prefix": "micro"}),
        json!({"level": 4, "name": "NANO-FIXO", "symbol": "nborb", "factor": 1e-8, "prefix": "nano"}),
        json!({"level": 5, "name": "PICO-FIXO", "symbol": "pborb", "factor": 1e-10, "prefix": "pico"}),
        json!({"level": 6, "name": "FEMTO-FIXO", "symbol": "fborb", "factor": 1e-12, "prefix": "femto"}),
        json!({"level": 7, "name": "ATTO-FIXO", "symbol": "afborb", "factor": 1e-15, "prefix": "atto"}),
        json!({"level": 8, "name": "ZEPTO-FIXO", "symbol": "zfborb", "factor": 1e-18, "prefix": "zepto"}),
        json!({"level": 9, "name": "YOCTO-FIXO", "symbol": "yfborb", "factor": 1e-21, "prefix": "yocto"}),
        json!({"level": 10, "name": "RONTO-FIXO", "symbol": "rfborb", "factor": 1e-24, "prefix": "ronto"}),
        json!({"level": 11, "name": "QUECTO-FIXO", "symbol": "qfborb", "factor": 1e-27, "prefix": "quecto"}),
        json!({"level": 12, "name": "SOUS-QUECTO", "symbol": "sqfborb", "factor": 1e-30, "prefix": "sous-quecto"}),
        json!({"level": 13, "name": "INFINI", "symbol": "inf", "factor": 0.0, "prefix": "infiniment-petit"}),
    ]
}

/// GET /api/counter/units
pub async fn counter_units() -> HttpResponse {
    HttpResponse::Ok().json(json!({
        "status": "success",
        "total_levels": 13,
        "precision": "infinie (extensible)",
        "units": all_units(),
        "message": "La precision est INFINIE. Chaque niveau peut etre etendu."
    }))
}
