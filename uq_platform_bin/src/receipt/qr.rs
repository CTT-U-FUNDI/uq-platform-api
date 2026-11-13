// ====================
// QR — Génération du QR Code
// ====================

use image::{ImageBuffer, Rgba, RgbaImage};
use qrcode::{QrCode, EcLevel};

/// Génère un QR code sous forme d'image RGBA
pub fn generate_qr(data: &str, size: u32) -> Result<RgbaImage, String> {
    // 1. Générer le QR code
    let code = QrCode::with_error_correction_level(data.as_bytes(), EcLevel::H)
        .map_err(|e| format!("QR encode: {}", e))?;
    
    // 2. Récupérer la matrice
    let width = code.width();
    let matrix = code.to_colors();
    
    // 3. Calculer la taille de chaque pixel
    let pixel_size = (size / width as u32).max(1);
    let actual_size = pixel_size * width as u32;
    
    // 4. Créer l'image
    let mut img: RgbaImage = ImageBuffer::from_pixel(
        actual_size,
        actual_size,
        Rgba([255, 255, 255, 255]),  // Fond blanc
    );
    
    // 5. Dessiner les pixels noirs
    for (i, color) in matrix.iter().enumerate() {
        let x = (i % width) as u32;
        let y = (i / width) as u32;
        
        if *color == qrcode::types::Color::Dark {
            for dy in 0..pixel_size {
                for dx in 0..pixel_size {
                    let px = x * pixel_size + dx;
                    let py = y * pixel_size + dy;
                    
                    if px < actual_size && py < actual_size {
                        img.put_pixel(px, py, Rgba([0, 0, 0, 255]));
                    }
                }
            }
        }
    }
    
    // 6. Ajouter une bordure or
    let border = 4u32;
    let mut final_img = ImageBuffer::from_pixel(
        actual_size + border * 2,
        actual_size + border * 2,
        Rgba([255, 215, 0, 255]),  // Or UQ
    );
    
    image::imageops::overlay(&mut final_img, &img, border as i64, border as i64);
    
    Ok(final_img)
}

/// Retourne le QR en bytes PNG
pub fn generate_qr_png_bytes(data: &str, size: u32) -> Result<Vec<u8>, String> {
    let img = generate_qr(data, size)?;
    let mut buffer = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut buffer);
    img.write_to(&mut cursor, image::ImageFormat::Png)
        .map_err(|e| format!("Encode QR PNG: {}", e))?;
    Ok(buffer)
}
