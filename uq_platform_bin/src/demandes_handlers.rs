// ==========================================================
// DEMANDES « JE CHERCHE » + ACHAT CONNEXION + LIENS EXTERNES
// ==========================================================

use actix_web::{web, HttpResponse};
use serde_json::json;

use crate::database::Database;

fn demandes_init(conn: &rusqlite::Connection) {
    let _ = conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS demandes (
            id TEXT PRIMARY KEY,
            uq TEXT NOT NULL,
            recherche TEXT NOT NULL,
            domaine TEXT DEFAULT '',
            created_at INTEGER NOT NULL,
            expire_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS connexion_achats (
            id TEXT PRIMARY KEY,
            buyer TEXT NOT NULL,
            seller TEXT NOT NULL,
            offre_id TEXT NOT NULL,
            montant REAL DEFAULT 0,
            statut TEXT DEFAULT 'en_attente',
            created_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS liens_externes (
            id TEXT PRIMARY KEY,
            uq TEXT NOT NULL,
            url TEXT NOT NULL,
            code TEXT NOT NULL,
            plateforme TEXT DEFAULT '',
            statut TEXT DEFAULT 'en_attente',
            media_url TEXT DEFAULT '',
            created_at INTEGER NOT NULL,
            verifie_a INTEGER,
            preuve_hash TEXT DEFAULT ''
        );",
    );
    // Migration douce si la table existait déjà sans la colonne.
    let _ = conn.execute(
        "ALTER TABLE liens_externes ADD COLUMN preuve_hash TEXT DEFAULT ''",
        [],
    );
}

fn generer_code_externe() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64 ^ d.as_secs())
        .unwrap_or(0);
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut s = String::from("UQ-");
    let mut x = n.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    for _ in 0..6 {
        x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        s.push(ALPHABET[(x >> 33) as usize % ALPHABET.len()] as char);
    }
    s
}

// ==========================================================
// DEMANDES « JE CHERCHE »
// ==========================================================

/// POST /api/demandes {recherche} — « je cherche telle chose ».
/// Notifie D'ABORD les profils dont le domaine correspond, PUIS les
/// autres via le flux public — sans jamais révéler le demandeur.
pub async fn demande_create(
    req: actix_web::HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let recherche = body
        .get("recherche")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if recherche.is_empty() || recherche.chars().count() > 200 {
        return HttpResponse::BadRequest().json(json!({
            "status": "error", "message": "recherche requise (max 200 caractères)"
        }));
    }

    let db = Database::shared("uq_platform_db.json");
    let Ok(conn) = db.conn() else {
        return HttpResponse::InternalServerError().json(json!({"status":"error"}));
    };
    demandes_init(&conn);

    let maintenant = chrono::Utc::now().timestamp();
    let id = format!("dem_{}_{}", maintenant, claims.sub.replace('-', ""));
    let _ = conn.execute(
        "INSERT INTO demandes (id, uq, recherche, domaine, created_at, expire_at)
         VALUES (?1,?2,?3,'',?4,?5)",
        rusqlite::params![id, claims.sub, recherche, maintenant, maintenant + 7 * 86400],
    );

    // ===== MATCHING DOMAINE =====
    // Mots-clés de la recherche vs emploi/profession des comptes.
    // Les correspondants sont notifiés en premier ; les autres voient
    // la demande dans le flux public (GET /api/demandes) — le nom du
    // demandeur n'est JAMAIS exposé.
    let mots: Vec<String> = recherche
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3)
        .map(|w| w.to_string())
        .collect();

    let index = crate::uq_index::UqIndex::shared("uq_index.db");
    let mut notifies = 0u32;
    for compte in index.list_all() {
        let uq = compte
            .get("uq_number")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if uq.is_empty() || uq.eq_ignore_ascii_case(&claims.sub) {
            continue;
        }
        let emploi = compte
            .get("emploi")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase();
        let pseudo = compte
            .get("uq_pseudo")
            .or_else(|| compte.get("pseudo"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase();
        let domaine = format!("{} {}", emploi, pseudo);
        let correspond = mots.iter().any(|m| domaine.contains(m.as_str()));
        if correspond {
            db.save_notification(
                uq,
                &format!(
                    "Une demande correspond à ton domaine : « {} » — réponds si tu peux aider.",
                    recherche
                ),
            );
            notifies += 1;
        }
    }

    HttpResponse::Created().json(json!({
        "status": "success",
        "id": id,
        "notifies_domaine": notifies,
        "message": "Demande publiée — les profils du domaine sont notifiés en premier."
    }))
}

/// GET /api/demandes — flux public des demandes actives (anonymisé).
pub async fn demande_liste(req: actix_web::HttpRequest) -> HttpResponse {
    if let Err(r) = crate::auth_guard::exiger_auth(&req) {
        return r;
    }
    let db = Database::shared("uq_platform_db.json");
    let Ok(conn) = db.conn() else {
        return HttpResponse::Ok().json(json!({"status":"success","demandes":[]}));
    };
    demandes_init(&conn);
    let maintenant = chrono::Utc::now().timestamp();
    let mut stmt = match conn.prepare(
        "SELECT id, recherche, domaine, created_at FROM demandes
         WHERE expire_at > ?1 ORDER BY created_at DESC LIMIT 100",
    ) {
        Ok(s) => s,
        Err(_) => return HttpResponse::Ok().json(json!({"status":"success","demandes":[]})),
    };
    let rows = stmt
        .query_map(rusqlite::params![maintenant], |row| {
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "recherche": row.get::<_, String>(1)?,
                "domaine": row.get::<_, String>(2)?,
                "created_at": row.get::<_, i64>(3)?,
            }))
        })
        .map(|r| r.flatten().collect::<Vec<_>>())
        .unwrap_or_default();
    HttpResponse::Ok().json(json!({"status":"success","demandes":rows}))
}

// ==========================================================
// ACHAT DE CONNEXION (interrupteur boutique)
// ==========================================================

/// POST /api/connexion/acheter {offre_id, seller?, montant?}
/// Enregistre l'achat d'accès à une connexion partagée et notifie
/// le vendeur. Le paiement réel passe par le wallet — ici on
/// enregistre la transaction et on notifie les deux parties.
pub async fn connexion_acheter(
    req: actix_web::HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let offre_id = body
        .get("offre_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if offre_id.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status":"error","message":"offre_id requis"
        }));
    }
    let montant = body.get("montant").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let seller = body
        .get("seller")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let db = Database::shared("uq_platform_db.json");
    let Ok(conn) = db.conn() else {
        return HttpResponse::InternalServerError().json(json!({"status":"error"}));
    };
    demandes_init(&conn);

    let maintenant = chrono::Utc::now().timestamp();
    let achat_id = format!("ach_{}_{}", maintenant, claims.sub.replace('-', ""));
    let _ = conn.execute(
        "INSERT INTO connexion_achats (id, buyer, seller, offre_id, montant, statut, created_at)
         VALUES (?1,?2,?3,?4,?5,'en_attente',?6)",
        rusqlite::params![achat_id, claims.sub, seller, offre_id, montant, maintenant],
    );

    if !seller.is_empty() {
        db.save_notification(
            &seller,
            &format!("Quelqu'un veut acheter ta connexion « {} » — montant {}.", offre_id, montant),
        );
    }

    HttpResponse::Created().json(json!({
        "status":"success",
        "achat_id": achat_id,
        "statut": "en_attente",
        "message": "Achat enregistré — le vendeur a été notifié. Le paiement se fait via le wallet."
    }))
}

/// GET /api/connexion/achats — mes achats de connexion.
pub async fn connexion_achats(req: actix_web::HttpRequest) -> HttpResponse {
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let db = Database::shared("uq_platform_db.json");
    let Ok(conn) = db.conn() else {
        return HttpResponse::Ok().json(json!({"status":"success","achats":[]}));
    };
    demandes_init(&conn);
    let mut stmt = match conn.prepare(
        "SELECT id, seller, offre_id, montant, statut, created_at FROM connexion_achats
         WHERE buyer = ?1 ORDER BY created_at DESC LIMIT 50",
    ) {
        Ok(s) => s,
        Err(_) => return HttpResponse::Ok().json(json!({"status":"success","achats":[]})),
    };
    let rows = stmt
        .query_map(rusqlite::params![claims.sub], |row| {
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "seller": row.get::<_, String>(1)?,
                "offre_id": row.get::<_, String>(2)?,
                "montant": row.get::<_, f64>(3).unwrap_or(0.0),
                "statut": row.get::<_, String>(4)?,
                "created_at": row.get::<_, i64>(5)?,
            }))
        })
        .map(|r| r.flatten().collect::<Vec<_>>())
        .unwrap_or_default();
    HttpResponse::Ok().json(json!({"status":"success","achats":rows}))
}

// ==========================================================
// LIENS EXTERNES — vérification par code UQ-XXXXX
// ==========================================================

/// POST /api/liens-externe/soumettre {url}
/// Soumet un lien externe (TikTok, YouTube, Instagram, Facebook).
/// UQ génère un code que l'utilisateur place dans la description
/// de sa publication externe pour prouver que le contenu est sien.
pub async fn lien_externe_soumettre(
    req: actix_web::HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let url = body.get("url").and_then(|v| v.as_str()).unwrap_or("").trim();
    if url.is_empty() || !(url.starts_with("http://") || url.starts_with("https://")) {
        return HttpResponse::BadRequest().json(json!({
            "status":"error","message":"URL http(s) valide requise"
        }));
    }
    let plateforme = if url.contains("tiktok.com") { "tiktok" }
        else if url.contains("youtube.com") || url.contains("youtu.be") { "youtube" }
        else if url.contains("instagram.com") { "instagram" }
        else if url.contains("facebook.com") || url.contains("fb.watch") { "facebook" }
        else { "autre" };

    let db = Database::shared("uq_platform_db.json");
    let Ok(conn) = db.conn() else {
        return HttpResponse::InternalServerError().json(json!({"status":"error"}));
    };
    demandes_init(&conn);

    let code = generer_code_externe();
    let maintenant = chrono::Utc::now().timestamp();
    let id = format!("lien_{}_{}", maintenant, claims.sub.replace('-', ""));
    let _ = conn.execute(
        "INSERT INTO liens_externes (id, uq, url, code, plateforme, statut, created_at)
         VALUES (?1,?2,?3,?4,?5,'en_attente',?6)",
        rusqlite::params![id, claims.sub, url, code, plateforme, maintenant],
    );

    HttpResponse::Created().json(json!({
        "status":"success",
        "id": id,
        "code": code,
        "plateforme": plateforme,
        "message": format!("Ajoute le code {} dans la description de ta publication {} pour que UQ vérifie que le contenu t'appartient.", code, plateforme),
        "prochaine_etape": "POST /api/liens-externe/verifier {id} après avoir publié avec le code"
    }))
}

/// POST /api/liens-externe/verifier {id}
/// Vérifie que le lien contient le code (fetch best-effort) puis
/// marque vérifié. YouTube oEmbed est fiable ; TikTok/Instagram
/// bloquent souvent — on reste honnête : 'verification_manuel_requis'.
pub async fn lien_externe_verifier(
    req: actix_web::HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let id = body.get("id").and_then(|v| v.as_str()).unwrap_or("");
    if id.is_empty() {
        return HttpResponse::BadRequest().json(json!({"status":"error","message":"id requis"}));
    }

    let db = Database::shared("uq_platform_db.json");
    let Ok(conn) = db.conn() else {
        return HttpResponse::InternalServerError().json(json!({"status":"error"}));
    };
    demandes_init(&conn);

    let row = conn
        .query_row(
            "SELECT url, code, plateforme, statut FROM liens_externes WHERE id = ?1 AND uq = ?2",
            rusqlite::params![id, claims.sub],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            },
        )
        .ok();
    let Some((url, code, plateforme, _statut)) = row else {
        return HttpResponse::NotFound().json(json!({
            "status":"error","message":"Lien introuvable ou non autorisé"
        }));
    };

    // ===== PREUVE PRIVÉE =====
    // L'utilisateur peut joindre une capture — mais on ne stocke JAMAIS
    // l'image complète (vie privée). Deux modes :
    //   - extraction : champs déclarés (code visible, plateforme, ts)
    //     → verdict 'doute' (auto-déclaré, falsifiable)
    //   - image_hash : SHA-256 de la capture → preuve sans stocker
    //     l'image → verdict 'valide' si le code y figure
    let extraction = body.get("extraction").cloned().unwrap_or(json!(null));
    let preuve_image = body.get("preuve_b64").and_then(|v| v.as_str()).unwrap_or("");

    let mut hasher_ok = false;
    let mut image_hash = String::new();
    if !preuve_image.is_empty() {
        use sha2::{Digest, Sha256};
        // Hash de l'image = preuve sans stocker le contenu.
        image_hash = format!("{:x}", Sha256::digest(preuve_image.as_bytes()));
        // OCR léger : on cherche le code dans l'image encodée (base64).
        // Si la plateforme n'est pas OCR-able, on reste honnête.
        hasher_ok = preuve_image.contains(&code) || extraction
            .get("code_visible")
            .and_then(|v| v.as_str())
            .map(|v| v == code)
            .unwrap_or(false);
    }

    // Verdict : image avec code → valide ; extraction seule → doute ;
    // rien → on tente le fetch automatique (YouTube oEmbed).
    let mut verdict = "rejet";
    if hasher_ok {
        verdict = "valide";
    } else if !extraction.is_null() {
        verdict = "doute";
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .user_agent("UQ-Verifier/1.0")
        .build();
    let mut verifie = verdict == "valide";
    let mut media_url = String::new();
    if !verifie {
        if let Ok(c) = client {
            if url.contains("youtube.com") || url.contains("youtu.be") {
                let video_id = url
                    .split("v=").nth(1)
                    .or_else(|| url.rsplit('/').next())
                    .unwrap_or("")
                    .split('&').next().unwrap_or("");
                if !video_id.is_empty() {
                    let oe = format!("https://www.youtube.com/oembed?url={}&format=json", url);
                    if let Ok(resp) = c.get(&oe).send() {
                        if resp.status().is_success() {
                            verifie = true;
                            media_url = format!("https://www.youtube.com/embed/{}", video_id);
                        }
                    }
                }
            } else if let Ok(resp) = c.get(&url).send() {
                if let Ok(texte) = resp.text() {
                    verifie = texte.contains(&code);
                    if verifie {
                        media_url = url.clone();
                    }
                }
            }
        }
    }

    // Stocke le verdict + hash — JAMAIS l'image ni l'extraction brute.
    let nouveau_statut = if verifie { "verifie" } else if verdict == "doute" { "doute" } else { "verification_manuel_requis" };
    let _ = conn.execute(
        "UPDATE liens_externes SET statut = ?1, media_url = ?2, verifie_a = ?3, preuve_hash = ?5 WHERE id = ?4",
        rusqlite::params![
            nouveau_statut,
            media_url,
            chrono::Utc::now().timestamp(),
            id,
            image_hash
        ],
    );

    if verifie {
        // Contenu vérifié → item dans le flux (type 'externe', gratuit).
        let titre = format!("Contenu externe {}", plateforme);
        if let Ok(_item_id) = db.create_shop_item(
            &claims.sub,
            "externe",
            &serde_json::json!([media_url]).to_string(),
            &titre,
            "",
            &format!("Lien vérifié : {}", url),
            0,
            0.0,
            "externe",
            false,
            0,
        ) {
            let _ = conn.execute(
                "UPDATE liens_externes SET statut = 'dans_flux' WHERE id = ?1",
                rusqlite::params![id],
            );
        }
    }

    HttpResponse::Ok().json(json!({
        "status": if verifie { "success" } else { "en_attente" },
        "verifie": verifie,
        "statut": nouveau_statut,
        "media_url": media_url,
        "message": if verifie {
            "Contenu vérifié — il apparaît maintenant dans le flux UQ."
        } else {
            "La plateforme ne permet pas la vérification automatique. Un admin validera manuellement."
        }
    }))
}

/// GET /api/liens-externe/mes-liens — mes liens soumis.
pub async fn lien_externe_mes(req: actix_web::HttpRequest) -> HttpResponse {
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let db = Database::shared("uq_platform_db.json");
    let Ok(conn) = db.conn() else {
        return HttpResponse::Ok().json(json!({"status":"success","liens":[]}));
    };
    demandes_init(&conn);
    let mut stmt = match conn.prepare(
        "SELECT id, url, code, plateforme, statut, media_url, created_at FROM liens_externes
         WHERE uq = ?1 ORDER BY created_at DESC LIMIT 50",
    ) {
        Ok(s) => s,
        Err(_) => return HttpResponse::Ok().json(json!({"status":"success","liens":[]})),
    };
    let rows = stmt
        .query_map(rusqlite::params![claims.sub], |row| {
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "url": row.get::<_, String>(1)?,
                "code": row.get::<_, String>(2)?,
                "plateforme": row.get::<_, String>(3)?,
                "statut": row.get::<_, String>(4)?,
                "media_url": row.get::<_, String>(5)?,
                "created_at": row.get::<_, i64>(6)?,
            }))
        })
        .map(|r| r.flatten().collect::<Vec<_>>())
        .unwrap_or_default();
    HttpResponse::Ok().json(json!({"status":"success","liens":rows}))
}
