// ====================
// INVOICE HANDLERS — 4 ROUTES
// Factures universelles (PDF + JSON)
// ====================

use actix_web::{web, HttpResponse};
use serde_json::json;
use rusqlite::params;
use printpdf::*;

/// GET /api/invoices
pub async fn invoices_list() -> HttpResponse {
    let db_path = crate::uq_index::uq_index_path();
    let conn = match rusqlite::Connection::open(db_path) {
        Ok(c) => c,
        Err(e) => return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("DB error: {}", e)
        })),
    };

    let mut stmt = match conn.prepare(
        "SELECT invoice_id, tx_id, created_at, amount_borb, amount_usd, rate_at_tx,
                fee_borb, type, from_user, to_user, status
         FROM invoices ORDER BY created_at DESC LIMIT 100"
    ) {
        Ok(s) => s,
        Err(_) => return HttpResponse::Ok().json(json!({
            "status": "success",
            "invoices": []
        })),
    };

    let rows = stmt.query_map([], |row| {
        Ok(json!({
            "invoice_id": row.get::<_, String>(0)?,
            "tx_id": row.get::<_, String>(1)?,
            "created_at": row.get::<_, i64>(2)?,
            "amount_borb": row.get::<_, f64>(3)?,
            "amount_usd": row.get::<_, f64>(4)?,
            "rate": row.get::<_, f64>(5)?,
            "fee_borb": row.get::<_, f64>(6).unwrap_or(0.0),
            "type": row.get::<_, String>(7).unwrap_or_default(),
            "from_user": row.get::<_, String>(8).unwrap_or_default(),
            "to_user": row.get::<_, String>(9).unwrap_or_default(),
            "status": row.get::<_, String>(10).unwrap_or("confirmed".to_string()),
        }))
    });

    let mut invoices = Vec::new();
    if let Ok(r) = rows {
        for row in r.flatten() { invoices.push(row); }
    }

    HttpResponse::Ok().json(json!({
        "status": "success",
        "total": invoices.len(),
        "invoices": invoices
    }))
}

/// GET /api/invoices/{id}
pub async fn invoice_get(req: actix_web::HttpRequest, path: web::Path<String>) -> HttpResponse {
    // 🔐 SÉCURITÉ (vague 1) — USER : JWT obligatoire
    if let Err(r) = crate::auth_guard::exiger_auth(&req) { return r; }

    let invoice_id = path.into_inner();
    let db_path = crate::uq_index::uq_index_path();
    let conn = match rusqlite::Connection::open(db_path) {
        Ok(c) => c,
        Err(e) => return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("DB error: {}", e)
        })),
    };

    let result = conn.query_row(
        "SELECT invoice_id, tx_id, created_at, confirmed_at, signature, tx_hash,
                block_number, fee_borb, fee_usd, fee_type, rate_at_tx, rate_source,
                amount_borb, amount_usd, total_debited_borb, total_received_borb,
                type, description, from_user, to_user, from_name, to_name,
                pdf_path, pdf_hash, status
         FROM invoices WHERE invoice_id = ?1",
        params![invoice_id],
        |row| {
            Ok(json!({
                "invoice_id": row.get::<_, String>(0)?,
                "tx_id": row.get::<_, String>(1)?,
                "created_at": row.get::<_, i64>(2)?,
                "confirmed_at": row.get::<_, Option<i64>>(3)?,
                "signature": row.get::<_, Option<String>>(4)?,
                "tx_hash": row.get::<_, Option<String>>(5)?,
                "block_number": row.get::<_, Option<i64>>(6)?,
                "fee_borb": row.get::<_, f64>(7).unwrap_or(0.0),
                "fee_usd": row.get::<_, f64>(8).unwrap_or(0.0),
                "fee_type": row.get::<_, Option<String>>(9)?,
                "rate_at_tx": row.get::<_, f64>(10)?,
                "rate_source": row.get::<_, String>(11).unwrap_or("blockchain".to_string()),
                "amount_borb": row.get::<_, f64>(12)?,
                "amount_usd": row.get::<_, f64>(13)?,
                "total_debited_borb": row.get::<_, Option<f64>>(14)?,
                "total_received_borb": row.get::<_, Option<f64>>(15)?,
                "type": row.get::<_, String>(16)?,
                "description": row.get::<_, Option<String>>(17)?,
                "from_user": row.get::<_, Option<String>>(18)?,
                "to_user": row.get::<_, Option<String>>(19)?,
                "from_name": row.get::<_, Option<String>>(20)?,
                "to_name": row.get::<_, Option<String>>(21)?,
                "pdf_path": row.get::<_, Option<String>>(22)?,
                "pdf_hash": row.get::<_, Option<String>>(23)?,
                "status": row.get::<_, String>(24).unwrap_or("confirmed".to_string()),
            }))
        },
    );

    match result {
        Ok(data) => HttpResponse::Ok().json(json!({
            "status": "success",
            "invoice": data
        })),
        Err(_) => HttpResponse::NotFound().json(json!({
            "status": "error",
            "message": "Facture introuvable"
        })),
    }
}

/// GET /api/invoices/{id}/pdf
/// Génère un PDF pour la facture
pub async fn invoice_pdf(req: actix_web::HttpRequest, path: web::Path<String>) -> HttpResponse {
    // 🔐 SÉCURITÉ (vague 1) — USER : JWT obligatoire
    if let Err(r) = crate::auth_guard::exiger_auth(&req) { return r; }

    let invoice_id = path.into_inner();
    let db_path = crate::uq_index::uq_index_path();
    let conn = match rusqlite::Connection::open(db_path) {
        Ok(c) => c,
        Err(e) => return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("DB error: {}", e)
        })),
    };

    let result = conn.query_row(
        "SELECT invoice_id, tx_id, created_at, amount_borb, amount_usd, rate_at_tx,
                fee_borb, type, from_user, to_user, signature
         FROM invoices WHERE invoice_id = ?1",
        params![invoice_id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, f64>(3)?,
                row.get::<_, f64>(4)?,
                row.get::<_, f64>(5)?,
                row.get::<_, f64>(6).unwrap_or(0.0),
                row.get::<_, String>(7).unwrap_or_default(),
                row.get::<_, String>(8).unwrap_or_default(),
                row.get::<_, String>(9).unwrap_or_default(),
                row.get::<_, Option<String>>(10)?,
            ))
        },
    );

    let (inv_id, tx_id, created_at, amount_borb, amount_usd, rate, fee_borb, inv_type, from_user, to_user, signature) = match result {
        Ok(data) => data,
        Err(_) => return HttpResponse::NotFound().json(json!({
            "status": "error",
            "message": "Facture introuvable"
        })),
    };

    // Générer le PDF
    let (doc, page1, layer1) = PdfDocument::new("Facture UQ", Mm(210.0), Mm(297.0), "Layer 1");
    let layer = doc.get_page(page1).get_layer(layer1);

    let font = doc.add_builtin_font(BuiltinFont::Helvetica).unwrap();
    let font_bold = doc.add_builtin_font(BuiltinFont::HelveticaBold).unwrap();

    // Titre
    layer.use_text("UQ PLATFORM", 24.0, Mm(20.0), Mm(270.0), &font_bold);
    layer.use_text("FACTURE OFFICIELLE", 16.0, Mm(20.0), Mm(260.0), &font);

    // ID Facture
    layer.use_text(format!("Facture N° : {}", inv_id), 12.0, Mm(20.0), Mm(245.0), &font_bold);
    layer.use_text(format!("Transaction : {}", tx_id), 10.0, Mm(20.0), Mm(238.0), &font);

    // Date
    let date_str = chrono::DateTime::from_timestamp(created_at, 0)
        .map(|d| d.format("%d/%m/%Y %H:%M").to_string())
        .unwrap_or_else(|| "Date inconnue".to_string());
    layer.use_text(format!("Date : {}", date_str), 10.0, Mm(20.0), Mm(231.0), &font);

    // Type
    layer.use_text(format!("Objet : {}", inv_type), 10.0, Mm(20.0), Mm(224.0), &font);

    // Acteurs
    layer.use_text(format!("De : {}", from_user), 10.0, Mm(20.0), Mm(210.0), &font);
    layer.use_text(format!("A : {}", to_user), 10.0, Mm(20.0), Mm(203.0), &font);

    // Montants
    layer.use_text("DETAILS FINANCIERS", 12.0, Mm(20.0), Mm(185.0), &font_bold);
    layer.use_text(format!("Montant : {} BORB", amount_borb), 10.0, Mm(20.0), Mm(175.0), &font);
    layer.use_text(format!("Equivalent USD : {:.2} USD", amount_usd), 10.0, Mm(20.0), Mm(168.0), &font);
    layer.use_text(format!("Taux applique : 1 BORB = {:.4} USD", rate), 10.0, Mm(20.0), Mm(161.0), &font);
    layer.use_text(format!("Frais : {} BORB", fee_borb), 10.0, Mm(20.0), Mm(154.0), &font);

    // Signature
    if let Some(sig) = signature {
        layer.use_text("SIGNATURE", 10.0, Mm(20.0), Mm(135.0), &font_bold);
        layer.use_text(format!("Hash : {}", &sig[..sig.len().min(40)]), 8.0, Mm(20.0), Mm(128.0), &font);
    }

    // Footer
    layer.use_text("Document genere par UQ Platform", 8.0, Mm(20.0), Mm(20.0), &font);
    layer.use_text("Verifiable sur la blockchain UQ", 8.0, Mm(20.0), Mm(15.0), &font);

    // Sauvegarder en bytes (printpdf 0.7 attend un BufWriter)
    let mut pdf_bytes: Vec<u8> = Vec::new();
    {
        let mut writer = std::io::BufWriter::new(&mut pdf_bytes);
        doc.save(&mut writer).unwrap();
    }

    HttpResponse::Ok()
        .content_type("application/pdf")
        .insert_header(("Content-Disposition", format!("attachment; filename=\"{}.pdf\"", inv_id)))
        .body(pdf_bytes)
}

/// GET /api/invoices/by-user/{uq}
pub async fn invoices_by_user(req: actix_web::HttpRequest, path: web::Path<String>) -> HttpResponse {
    // 🔐 SÉCURITÉ (vague 1) — USER : accès à SES données (ou owner)
    let __cible = path.to_string();
    if let Err(r) = crate::auth_guard::exiger_soi_ou_owner(&req, &__cible) { return r; }

    let uq_number = path.into_inner();
    let db_path = crate::uq_index::uq_index_path();
    let conn = match rusqlite::Connection::open(db_path) {
        Ok(c) => c,
        Err(e) => return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("DB error: {}", e)
        })),
    };

    let mut stmt = match conn.prepare(
        "SELECT invoice_id, tx_id, created_at, amount_borb, amount_usd, rate_at_tx,
                fee_borb, type, from_user, to_user, status
         FROM invoices
         WHERE from_user = ?1 OR to_user = ?1
         ORDER BY created_at DESC LIMIT 100"
    ) {
        Ok(s) => s,
        Err(_) => return HttpResponse::Ok().json(json!({
            "status": "success",
            "uq_number": uq_number,
            "invoices": []
        })),
    };

    let rows = stmt.query_map(params![uq_number], |row| {
        Ok(json!({
            "invoice_id": row.get::<_, String>(0)?,
            "tx_id": row.get::<_, String>(1)?,
            "created_at": row.get::<_, i64>(2)?,
            "amount_borb": row.get::<_, f64>(3)?,
            "amount_usd": row.get::<_, f64>(4)?,
            "rate": row.get::<_, f64>(5)?,
            "fee_borb": row.get::<_, f64>(6).unwrap_or(0.0),
            "type": row.get::<_, String>(7).unwrap_or_default(),
            "from_user": row.get::<_, String>(8).unwrap_or_default(),
            "to_user": row.get::<_, String>(9).unwrap_or_default(),
            "status": row.get::<_, String>(10).unwrap_or("confirmed".to_string()),
        }))
    });

    let mut invoices = Vec::new();
    if let Ok(r) = rows {
        for row in r.flatten() { invoices.push(row); }
    }

    HttpResponse::Ok().json(json!({
        "status": "success",
        "uq_number": uq_number,
        "total": invoices.len(),
        "invoices": invoices
    }))
}


