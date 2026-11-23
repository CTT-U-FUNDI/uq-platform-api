// ====================
// GENERATOR — Génération du reçu PNG avec TEXTE
// ====================

use ab_glyph::{FontRef, PxScale};
use image::{ImageBuffer, Rgba, RgbaImage};
use imageproc::drawing::{draw_filled_rect_mut, draw_text_mut};
use imageproc::rect::Rect;

use super::qr;
use super::ReceiptData;

const WIDTH: u32 = 800;
const HEIGHT: u32 = 1100;
const PADDING: u32 = 40;

// Couleurs UQ
const BG_COLOR: Rgba<u8> = Rgba([10, 10, 15, 255]);
const CARD_COLOR: Rgba<u8> = Rgba([26, 26, 36, 255]);
const TEXT_COLOR: Rgba<u8> = Rgba([255, 255, 255, 255]);
const TEXT_SECONDARY: Rgba<u8> = Rgba([142, 142, 154, 255]);
const ACCENT_COLOR: Rgba<u8> = Rgba([255, 215, 0, 255]); // Or UQ
const BORDER_COLOR: Rgba<u8> = Rgba([42, 42, 56, 255]);
const SUCCESS_COLOR: Rgba<u8> = Rgba([74, 222, 128, 255]);

/// Charge la police embarquée
fn get_font() -> Result<FontRef<'static>, String> {
    // Essayer la police embarquée
    let font_data: &'static [u8] = include_bytes!("../../assets/fonts/UQFont.ttf");
    FontRef::try_from_slice(font_data).map_err(|e| format!("Font: {}", e))
}

/// Formate un timestamp en date lisible
fn format_timestamp(ts: i64) -> String {
    if let Some(dt) = chrono::DateTime::from_timestamp(ts, 0) {
        dt.format("%Y-%m-%d %H:%M:%S UTC").to_string()
    } else {
        ts.to_string()
    }
}

/// Tronque une string
fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}

/// Génère le reçu PNG complet
pub fn generate_receipt_png(data: &ReceiptData) -> Result<RgbaImage, String> {
    let mut img: RgbaImage = ImageBuffer::from_pixel(WIDTH, HEIGHT, BG_COLOR);
    let font = get_font()?;

    // ====================
    // FOND (carte)
    // ====================
    draw_filled_rect_mut(
        &mut img,
        Rect::at(PADDING as i32, PADDING as i32).of_size(WIDTH - PADDING * 2, HEIGHT - PADDING * 2),
        CARD_COLOR,
    );

    // Bordure or en haut
    draw_filled_rect_mut(
        &mut img,
        Rect::at(PADDING as i32, PADDING as i32).of_size(WIDTH - PADDING * 2, 4),
        ACCENT_COLOR,
    );

    // Bordure or en bas
    draw_filled_rect_mut(
        &mut img,
        Rect::at(PADDING as i32, (HEIGHT - PADDING - 4) as i32).of_size(WIDTH - PADDING * 2, 4),
        ACCENT_COLOR,
    );

    // ====================
    // TITRE
    // ====================
    let mut y = (PADDING + 30) as i32;
    let x = (PADDING + 30) as i32;

    draw_text_mut(
        &mut img,
        ACCENT_COLOR,
        x,
        y,
        PxScale::from(32.0),
        &font,
        "REÇU DE TRANSACTION UQ",
    );
    y += 50;

    // Ligne
    draw_filled_rect_mut(
        &mut img,
        Rect::at(x, y).of_size((WIDTH - PADDING * 2 - 60) as u32, 1),
        BORDER_COLOR,
    );
    y += 20;

    // ====================
    // APPAREIL
    // ====================
    draw_text_mut(
        &mut img,
        ACCENT_COLOR,
        x,
        y,
        PxScale::from(18.0),
        &font,
        "APPAREIL",
    );
    y += 28;
    draw_text_mut(
        &mut img,
        TEXT_COLOR,
        x,
        y,
        PxScale::from(16.0),
        &font,
        &format!(
            "{} / {} / {}",
            if data.device.is_empty() {
                "Inconnu"
            } else {
                &data.device
            },
            if data.os.is_empty() {
                "Inconnu"
            } else {
                &data.os
            },
            if data.browser.is_empty() {
                "Inconnu"
            } else {
                &data.browser
            }
        ),
    );
    y += 35;

    // ====================
    // SENDER
    // ====================
    draw_text_mut(
        &mut img,
        ACCENT_COLOR,
        x,
        y,
        PxScale::from(18.0),
        &font,
        "EXPÉDITEUR",
    );
    y += 28;
    draw_text_mut(
        &mut img,
        TEXT_COLOR,
        x,
        y,
        PxScale::from(16.0),
        &font,
        &format!("{} / {}", data.sender_uq, data.sender_display),
    );
    y += 25;
    draw_text_mut(
        &mut img,
        TEXT_SECONDARY,
        x,
        y,
        PxScale::from(14.0),
        &font,
        &format!("Cadenas: {}", truncate(&data.sender_cadenas, 40)),
    );
    y += 22;
    draw_text_mut(
        &mut img,
        TEXT_SECONDARY,
        x,
        y,
        PxScale::from(14.0),
        &font,
        &format!("IP: {}", data.sender_ip),
    );
    y += 35;

    // ====================
    // RECIPIENT
    // ====================
    draw_text_mut(
        &mut img,
        ACCENT_COLOR,
        x,
        y,
        PxScale::from(18.0),
        &font,
        "DESTINATAIRE",
    );
    y += 28;
    draw_text_mut(
        &mut img,
        TEXT_COLOR,
        x,
        y,
        PxScale::from(16.0),
        &font,
        &format!("{} / {}", data.recipient_uq, data.recipient_display),
    );
    y += 35;

    // ====================
    // MONTANT
    // ====================
    draw_text_mut(
        &mut img,
        ACCENT_COLOR,
        x,
        y,
        PxScale::from(18.0),
        &font,
        "MONTANT",
    );
    y += 28;
    draw_text_mut(
        &mut img,
        SUCCESS_COLOR,
        x,
        y,
        PxScale::from(24.0),
        &font,
        &match data.amount_usd {
            Some(usd) => format!("{:.2} BORB ({:.2} USD)", data.amount_borb, usd),
            None => format!("{:.2} BORB (USD non réglé)", data.amount_borb),
        },
    );
    y += 30;
    draw_text_mut(
        &mut img,
        TEXT_SECONDARY,
        x,
        y,
        PxScale::from(14.0),
        &font,
        &format!("Frais: {:.4} BORB | Taux: {}", data.fee_borb, data.taux_usd),
    );
    y += 35;

    // ====================
    // DATE
    // ====================
    draw_text_mut(
        &mut img,
        ACCENT_COLOR,
        x,
        y,
        PxScale::from(18.0),
        &font,
        "DATE",
    );
    y += 28;
    draw_text_mut(
        &mut img,
        TEXT_COLOR,
        x,
        y,
        PxScale::from(16.0),
        &font,
        &format_timestamp(data.created_at),
    );
    y += 35;

    // ====================
    // SÉCURITÉ
    // ====================
    draw_text_mut(
        &mut img,
        ACCENT_COLOR,
        x,
        y,
        PxScale::from(18.0),
        &font,
        "SÉCURITÉ",
    );
    y += 28;
    draw_text_mut(
        &mut img,
        TEXT_SECONDARY,
        x,
        y,
        PxScale::from(12.0),
        &font,
        &format!("Référence: {}", data.reference),
    );
    y += 20;
    draw_text_mut(
        &mut img,
        TEXT_SECONDARY,
        x,
        y,
        PxScale::from(11.0),
        &font,
        &format!("Signature: {}", truncate(&data.signature, 50)),
    );
    y += 20;
    draw_text_mut(
        &mut img,
        TEXT_SECONDARY,
        x,
        y,
        PxScale::from(11.0),
        &font,
        &format!("Hash: {}", truncate(&data.tx_hash, 50)),
    );

    // ====================
    // QR CODE (en bas à droite)
    // ====================
    let qr_size = 280u32;
    let qr_x = (WIDTH - PADDING - qr_size - 40) as i64;
    let qr_y = (HEIGHT - PADDING - qr_size - 80) as i64;

    let verify_url = format!(
        "http://localhost:5000/verify/{}",
        if data.tx_hash.len() >= 16 {
            &data.tx_hash[..16]
        } else {
            &data.tx_hash
        }
    );
    let qr_img = qr::generate_qr(&verify_url, qr_size)?;

    image::imageops::overlay(&mut img, &qr_img, qr_x, qr_y);

    // Label QR
    draw_text_mut(
        &mut img,
        TEXT_SECONDARY,
        (qr_x as i32),
        (qr_y + qr_size as i64 + 10) as i32,
        PxScale::from(12.0),
        &font,
        "Scannez pour vérifier",
    );

    // ====================
    // FOOTER
    // ====================
    let footer_y = (HEIGHT - PADDING - 30) as i32;
    draw_text_mut(
        &mut img,
        SUCCESS_COLOR,
        x,
        footer_y,
        PxScale::from(14.0),
        &font,
        "✅ Reçu officiel — Vérifiable en ligne",
    );

    eprintln!("🖼️  Reçu PNG généré : {}x{}", WIDTH, HEIGHT);
    Ok(img)
}

/// Sauvegarde le reçu dans un fichier
pub fn save_receipt_png(data: &ReceiptData, path: &str) -> Result<(), String> {
    let img = generate_receipt_png(data)?;
    img.save(path).map_err(|e| format!("Save PNG: {}", e))?;
    Ok(())
}

/// Retourne le reçu en bytes PNG
pub fn receipt_to_png_bytes(data: &ReceiptData) -> Result<Vec<u8>, String> {
    let img = generate_receipt_png(data)?;
    let mut buffer = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut buffer);
    img.write_to(&mut cursor, image::ImageFormat::Png)
        .map_err(|e| format!("Encode PNG: {}", e))?;
    Ok(buffer)
}
