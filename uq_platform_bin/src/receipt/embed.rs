// ====================
// EMBED — Insertion de métadonnées signées dans le PNG
// ====================
//
// Format : chunks PNG personnalisés "tEXt"
// - uq_data      → JSON des données
// - uq_signature → Signature Ed25519 (hex)
// - uq_pubkey    → Clé publique Trésor (hex)
// - uq_hash      → SHA256 des données
// - uq_reference → Référence du reçu
//
// ====================

use super::ReceiptData;
use super::treasury_key;

/// Encode les données du reçu + signature dans un PNG
pub fn embed_receipt_metadata(
    png_bytes: &[u8],
    data: &ReceiptData,
) -> Result<Vec<u8>, String> {
    // 1. Sérialiser les données en JSON
    let json = serde_json::to_string(data)
        .map_err(|e| format!("JSON: {}", e))?;
    
    // 2. Calculer le hash
    let hash = treasury_key::hash_receipt_data(json.as_bytes());
    
    // 3. Signer le hash
    let (signature, public_key) = treasury_key::sign_with_treasury(hash.as_bytes())?;
    let signature_hex = hex::encode(signature);
    let public_key_hex = hex::encode(&public_key);
    
    // 4. Chercher la fin des chunks PNG avant IEND
    // Structure PNG : signature(8) + chunks + IEND
    // On insère nos chunks AVANT IEND
    
    // Trouver IEND (dernier chunk)
    let iend_pos = find_iend(png_bytes)?;
    
    // 5. Construire nos chunks
    let mut extra_chunks = Vec::new();
    extra_chunks.extend_from_slice(&make_text_chunk("uq_version", "1"));
    extra_chunks.extend_from_slice(&make_text_chunk("uq_data", &json));
    extra_chunks.extend_from_slice(&make_text_chunk("uq_hash", &hash));
    extra_chunks.extend_from_slice(&make_text_chunk("uq_signature", &signature_hex));
    extra_chunks.extend_from_slice(&make_text_chunk("uq_pubkey", &public_key_hex));
    extra_chunks.extend_from_slice(&make_text_chunk("uq_reference", &data.reference));
    
    // 6. Assembler : png_avant + extra_chunks + png_apres
    let mut result = Vec::with_capacity(png_bytes.len() + extra_chunks.len());
    result.extend_from_slice(&png_bytes[..iend_pos]);
    result.extend_from_slice(&extra_chunks);
    result.extend_from_slice(&png_bytes[iend_pos..]);
    
    eprintln!("🔐 Métadonnées embarquées : {} (hash: {}...)", 
        data.reference, &hash[..16]);
    eprintln!("   Signature: {}...", &signature_hex[..16]);
    
    Ok(result)
}

/// Trouve la position du chunk IEND
fn find_iend(png_bytes: &[u8]) -> Result<usize, String> {
    // IEND = 49 45 4E 44 (ASCII)
    let iend_signature = b"IEND";
    
    for i in 0..png_bytes.len() - 4 {
        if &png_bytes[i..i+4] == iend_signature {
            // Revenir de 4 bytes (length) + 4 bytes (type) = position du length
            if i >= 4 {
                return Ok(i - 4);
            }
        }
    }
    
    Err("IEND non trouvé".to_string())
}

/// Crée un chunk PNG tEXt
fn make_text_chunk(keyword: &str, text: &str) -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(keyword.as_bytes());
    data.push(0);  // Null separator
    data.extend_from_slice(text.as_bytes());
    
    let length = data.len() as u32;
    let chunk_type = b"tEXt";
    
    let mut chunk = Vec::new();
    chunk.extend_from_slice(&length.to_be_bytes());      // Length
    chunk.extend_from_slice(chunk_type);                  // Type
    chunk.extend_from_slice(&data);                       // Data
    
    // CRC32 du type + data
    let mut crc_data = Vec::new();
    crc_data.extend_from_slice(chunk_type);
    crc_data.extend_from_slice(&data);
    let crc = crc32fast::hash(&crc_data);
    chunk.extend_from_slice(&crc.to_be_bytes());
    
    chunk
}

/// Extrait un chunk tEXt du PNG
pub fn extract_text_chunk(png_bytes: &[u8], keyword: &str) -> Option<String> {
    let iend_pos = find_iend(png_bytes).ok()?;
    let header_size = 8;  // PNG signature
    
    let mut pos = header_size;
    
    while pos + 12 < png_bytes.len() {
        let length = u32::from_be_bytes([
            png_bytes[pos],
            png_bytes[pos + 1],
            png_bytes[pos + 2],
            png_bytes[pos + 3],
        ]) as usize;
        
        let chunk_type = &png_bytes[pos + 4..pos + 8];
        
        if chunk_type == b"tEXt" && length > 0 {
            let data = &png_bytes[pos + 8..pos + 8 + length];
            
            // Chercher le null separator
            if let Some(null_pos) = data.iter().position(|&b| b == 0) {
                let chunk_keyword = String::from_utf8_lossy(&data[..null_pos]);
                if chunk_keyword == keyword {
                    let text = String::from_utf8_lossy(&data[null_pos + 1..]);
                    return Some(text.to_string());
                }
            }
        }
        
        pos += 12 + length;
    }
    
    None
}
