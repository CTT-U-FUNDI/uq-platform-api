use sha2::{Sha256, Digest};

/// VRAI Shamir Secret Sharing (polynôme de Lagrange)
/// 
/// Principe : Le secret est le coefficient constant d'un polynôme de degré k-1.
/// Chaque fragment est un point (x, P(x)) sur ce polynôme.
/// Pour reconstruire le secret, il faut k fragments (interpolation de Lagrange).

/// Génère k fragments à partir d'un secret (seuil k sur n)
pub fn split_key(secret: &[u8], threshold: usize, total_shards: usize) -> Result<Vec<(u8, Vec<u8>)>, String> {
    if threshold < 2 {
        return Err("Le seuil doit être au moins 2".to_string());
    }
    if total_shards < threshold {
        return Err("Le nombre total de fragments doit être >= au seuil".to_string());
    }
    if secret.is_empty() {
        return Err("Le secret ne peut pas être vide".to_string());
    }
    
    let mut shards = Vec::new();
    
    // Pour chaque octet du secret, créer un polynôme
    for (byte_idx, &secret_byte) in secret.iter().enumerate() {
        // Générer les coefficients du polynôme : P(x) = secret + a1*x + a2*x^2 + ... + a(k-1)*x^(k-1)
        let mut coefficients = Vec::with_capacity(threshold);
        coefficients.push(secret_byte as u32); // a0 = le secret
        
        // Générer les coefficients aléatoires restants
        for _ in 1..threshold {
            let random_coeff = rand::random::<u8>() as u32;
            coefficients.push(random_coeff);
        }
        
        // Créer les fragments (points sur le polynôme)
        for x in 1..=total_shards as u8 {
            let mut y = 0u32;
            for (power, coeff) in coefficients.iter().enumerate() {
                let x_pow = (x as u32).pow(power as u32);
                y = y.wrapping_add(coeff.wrapping_mul(x_pow));
            }
            
            if shards.len() < total_shards {
                shards.push((x, Vec::new()));
            }
            shards[(x - 1) as usize].1.push((y & 0xFF) as u8);
        }
    }
    
    Ok(shards)
}

/// Reconstruit le secret à partir de k fragments (interpolation de Lagrange)
pub fn combine_key(shards: &[(u8, Vec<u8>)], threshold: usize) -> Result<Vec<u8>, String> {
    if shards.len() < threshold {
        return Err(format!("Il faut au moins {} fragments, reçu {}", threshold, shards.len()));
    }
    if shards.is_empty() {
        return Err("Aucun fragment fourni".to_string());
    }
    
    let secret_len = shards[0].1.len();
    let mut secret = Vec::with_capacity(secret_len);
    
    // Pour chaque octet du secret
    for byte_idx in 0..secret_len {
        let mut result = 0f64;
        
        // Interpolation de Lagrange
        for i in 0..threshold {
            let (xi, yi) = (&shards[i].0, &shards[i].1[byte_idx]);
            let mut lagrange = 1f64;
            
            for j in 0..threshold {
                if i != j {
                    let xj = shards[j].0 as f64;
                    let xi_f = *xi as f64;
                    lagrange *= (0.0 - xj) / (xi_f - xj);
                }
            }
            
            result += (*yi as f64) * lagrange;
        }
        
        secret.push(result.round() as u8);
    }
    
    Ok(secret)
}
