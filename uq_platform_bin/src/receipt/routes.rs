// ====================
// ROUTES — Routes HTTP pour les reçus
// ====================

use actix_web::{web, HttpResponse, Responder};
use serde_json::json;

use super::{embed, generator, treasury_key, verifier, ReceiptData};

/// GET /api/transactions/{id}/receipt.png
/// Génère et retourne le reçu PNG d'une transaction
pub async fn get_receipt_png(path: web::Path<(String, i64)>) -> impl Responder {
    let (uq_number, tx_id) = path.into_inner();

    // 1. Ouvrir la DB du compte
    let db_path = format!(
        "data/users/{}.db",
        uq_number.replace("-", "_").to_lowercase()
    );

    if !std::path::Path::new(&db_path).exists() {
        return HttpResponse::NotFound().json(json!({
            "status": "error",
            "message": "Compte introuvable"
        }));
    }

    let conn = match rusqlite::Connection::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("DB error: {}", e)
            }))
        }
    };

    // 2. Récupérer la transaction
    let result: Result<
        (
            String,
            i64,
            i64,
            String,
            String,
            i64,
            String,
            String,
            String,
            String,
        ),
        _,
    > = conn.query_row(
        "SELECT tx_type, amount, fee, counterparty, memo, created_at,
                COALESCE(sender_uq, ''), COALESCE(recipient_uq, ''),
                COALESCE(reference, ''), COALESCE(device, '')
         FROM transactions WHERE id = ?1",
        rusqlite::params![tx_id],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
                row.get(8)?,
                row.get(9)?,
            ))
        },
    );

    let (
        tx_type,
        amount,
        fee,
        counterparty,
        memo,
        created_at,
        sender_uq,
        recipient_uq,
        reference,
        device,
    ) = match result {
        Ok(r) => r,
        Err(e) => {
            return HttpResponse::NotFound().json(json!({
                "status": "error",
                "message": format!("Transaction {} introuvable: {}", tx_id, e)
            }))
        }
    };

    // 3. Charger le cadenas du sender
    let sender_cadenas = conn
        .query_row(
            "SELECT hash_transforme FROM cadenas_verification WHERE id = 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap_or_else(|_| "NON_POSE".to_string());

    // 4. Construire les données du reçu
    // Contre-valeur USD : le prix réellement réglé par le régulateur. Sans
    // prix réglé, le reçu n'imprime aucune équivalence et aucun « taux 1.0 » —
    // ces deux lignes affirmaient « 1 BORB = 1 USD » sans aucune observation.
    let prix_borb = crate::public_routes::prix_borb_usd();
    let amount_borb = amount as f64 / 100.0;
    let receipt_data = ReceiptData {
        reference: if reference.is_empty() {
            format!("UQ-TX-{}", tx_id)
        } else {
            reference
        },
        tx_hash: memo.clone(),
        sender_uq: sender_uq.clone(),
        sender_display: sender_uq.clone(),
        sender_cadenas,
        sender_ip: "unknown".to_string(),
        recipient_uq: recipient_uq.clone(),
        recipient_display: recipient_uq.clone(),
        amount_borb,
        amount_usd: prix_borb.map(|p| amount_borb * p),
        fee_borb: fee as f64 / 100.0,
        taux_usd: match prix_borb {
            Some(p) => format!("{:.6}", p),
            None => "non réglé".to_string(),
        },
        device,
        os: "".to_string(),
        browser: "".to_string(),
        created_at,
        signature: memo.clone(),
        public_key: "".to_string(),
        status: "success".to_string(),
    };

    // 5. Générer le PNG
    let png_bytes = match generator::receipt_to_png_bytes(&receipt_data) {
        Ok(b) => b,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Génération PNG: {}", e)
            }))
        }
    };

    // 6. Embarquer les métadonnées signées
    let signed_png = match embed::embed_receipt_metadata(&png_bytes, &receipt_data) {
        Ok(b) => b,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "message": format!("Embed: {}", e)
            }))
        }
    };

    // 7. Retourner le PNG
    HttpResponse::Ok()
        .content_type("image/png")
        .insert_header((
            "Content-Disposition",
            format!("attachment; filename=\"receipt-{}.png\"", tx_id),
        ))
        .body(signed_png)
}

/// GET /api/verify/receipt/{hash}
/// Vérifie un reçu (JSON)
pub async fn verify_receipt_json(path: web::Path<String>) -> impl Responder {
    let hash = path.into_inner();

    // ⚠️ TODO : Implémenter la vérification par hash
    // Pour l'instant, on retourne une réponse basique

    HttpResponse::Ok().json(json!({
        "status": "success",
        "hash": hash,
        "valid": false,
        "message": "Vérification par hash à implémenter"
    }))
}

/// GET /verify/{hash}
/// Page HTML de vérification
pub async fn verify_receipt_html(path: web::Path<String>) -> impl Responder {
    let hash = path.into_inner();

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="fr">
<head>
    <meta charset="UTF-8">
    <title>Vérification de reçu UQ</title>
    <style>
        body {{
            background: #0A0A0F;
            color: #FFFFFF;
            font-family: -apple-system, sans-serif;
            display: flex;
            justify-content: center;
            align-items: center;
            min-height: 100vh;
            margin: 0;
        }}
        .card {{
            background: #1A1A24;
            padding: 40px;
            border-radius: 20px;
            border: 1px solid #2A2A38;
            max-width: 500px;
            text-align: center;
        }}
        h1 {{ color: #FFD700; }}
        .hash {{
            font-family: monospace;
            background: #252533;
            padding: 10px;
            border-radius: 8px;
            margin: 20px 0;
            word-break: break-all;
        }}
    </style>
</head>
<body>
    <div class="card">
        <h1>🔍 Vérification de reçu</h1>
        <div class="hash">{}</div>
        <p>⚠️ Vérification à implémenter</p>
        <p style="color: #8E8E9A; font-size: 14px;">UQ Platform — Reçus signés Ed25519</p>
    </div>
</body>
</html>"#,
        hash
    );

    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(html)
}
