// ============================================================================
// chat_risk.rs — Detection de spam, phishing, faux clients
// ============================================================================

use rusqlite::{params, Connection};
use serde_json::json;

/// Signaux de risque detectes.
pub struct AnalyseRisque {
    pub score: i64,
    pub signaux: Vec<String>,
    pub action: String,
}

/// Detecte un lien dans un message.
pub fn contient_lien(message: &str) -> bool {
    let msg = message.to_lowercase();
    let liens = [
        "http://", "https://", "www.", "t.me/", "bit.ly/",
        "tinyurl.com", "goo.gl/", "is.gd/", "ow.ly/", "cutt.ly/",
        "rb.gy/", "shorturl.at", "rebrand.ly",
    ];
    liens.iter().any(|l| msg.contains(l))
}

/// Masque les liens si pas ami.
pub fn masquer_liens(message: &str, est_ami: bool) -> String {
    if est_ami {
        return message.to_string();
    }
    let mut resultat = String::new();
    for mot in message.split_whitespace() {
        let mot_lower = mot.to_lowercase();
        if mot_lower.starts_with("http://")
            || mot_lower.starts_with("https://")
            || mot_lower.starts_with("www.")
            || mot_lower.starts_with("t.me/")
            || mot_lower.starts_with("bit.ly/")
            || mot_lower.starts_with("tinyurl.com")
            || mot_lower.starts_with("goo.gl/")
            || mot_lower.starts_with("is.gd/")
            || mot_lower.starts_with("ow.ly/")
            || mot_lower.starts_with("cutt.ly/")
            || mot_lower.starts_with("rb.gy/")
            || mot_lower.starts_with("shorturl.at")
            || mot_lower.starts_with("rebrand.ly")
        {
            resultat.push_str("[lien masque] ");
        } else {
            resultat.push_str(mot);
            resultat.push(' ');
        }
    }
    resultat.trim().to_string()
}

/// Valide un fichier (PNG/JPG, < 500 Ko).
pub fn valider_fichier(nom: &str, taille: usize) -> Result<(), String> {
    let nom_lower = nom.to_lowercase();
    let ext = nom_lower.rsplit('.').next().unwrap_or("");
    if ext != "png" && ext != "jpg" && ext != "jpeg" {
        return Err("Seuls les PNG et JPG sont autorises".to_string());
    }
    if taille > 500 * 1024 {
        return Err("Fichier trop lourd (max 500 Ko)".to_string());
    }
    Ok(())
}

/// Analyse le risque d'un message.
pub fn analyser_risque(
    message: &str,
    sender: &str,
    _recipient: &str,
    _est_ami: bool,
    conn: &Connection,
) -> Result<AnalyseRisque, String> {
    let mut score = 0i64;
    let mut signaux = Vec::new();
    let msg_lower = message.to_lowercase();
    let now = chrono::Utc::now().timestamp();

    // 1. Demande de PIN/mot de passe
    if msg_lower.contains("pin") || msg_lower.contains("mot de passe")
        || msg_lower.contains("password") || msg_lower.contains("code secret") {
        score += 80;
        signaux.push("demande_pin".to_string());
    }

    // 2. Demande hors plateforme
    if msg_lower.contains("whatsapp") || msg_lower.contains("telegram")
        || msg_lower.contains("hors uq") {
        score += 60;
        signaux.push("hors_plateforme".to_string());
    }

    // 3. Volume 24h
    let messages_24h: i64 = conn.query_row(
        "SELECT COUNT(*) FROM chat_messages WHERE sender = ?1 AND timestamp >= ?2",
        params![sender, now - 86400],
        |row| row.get(0),
    ).unwrap_or(0);
    if messages_24h > 50 {
        score += 30;
        signaux.push("volume_50".to_string());
    }

    // 4. Messages repetitifs
    let messages_identiques: i64 = conn.query_row(
        "SELECT COUNT(DISTINCT recipient) FROM chat_messages
         WHERE sender = ?1 AND content = ?2 AND timestamp >= ?3",
        params![sender, message, now - 86400],
        |row| row.get(0),
    ).unwrap_or(0);
    if messages_identiques >= 3 {
        score += 40;
        signaux.push("message_repetitif".to_string());
    }

    // 5. Compte recent
    let cree_le: i64 = conn.query_row(
        "SELECT created_at FROM accounts WHERE uq_number = ?1",
        params![sender],
        |row| row.get(0),
    ).unwrap_or(0);
    if cree_le > 0 && now - cree_le < 7 * 86400 {
        score += 20;
        signaux.push("compte_recent".to_string());
    }

    // 6. MAJUSCULES
    let lettres = message.chars().filter(|c| c.is_alphabetic()).count();
    let majuscules = message.chars().filter(|c| c.is_uppercase()).count();
    if lettres > 10 && (majuscules as f64 / lettres as f64) > 0.7 {
        score += 10;
        signaux.push("majuscules".to_string());
    }

    let action = if score >= 90 {
        "block"
    } else if score >= 70 {
        "alert"
    } else if score >= 30 {
        "warn"
    } else {
        "pass"
    };

    Ok(AnalyseRisque {
        score,
        signaux,
        action: action.to_string(),
    })
}

/// Enregistre un risque dans chat_risk_scores.
pub fn logguer_risque(
    conn: &Connection,
    sender: &str,
    recipient: &str,
    analyse: &AnalyseRisque,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO chat_risk_scores (sender, recipient, score, signaux, action, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            sender,
            recipient,
            analyse.score,
            json!(analyse.signaux).to_string(),
            analyse.action,
            chrono::Utc::now().timestamp()
        ],
    ).map_err(|e| e.to_string())?;
    Ok(())
}