// ====================
// 🔐 CADENAS — Systeme anti-piratage
// ====================

use serde::{Serialize, Deserialize};
use sha2::{Sha256, Digest};
use rusqlite::Connection;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InfosCompte {
    pub uq_number: String,
    pub nom: String,
    pub prenom: String,
    pub date_naissance: String,
    pub lieu_naissance: String,
    pub pays: String,
    pub adresse: Option<String>,
    pub telephone: Option<String>,
    pub created_at: i64,
}

pub fn calculer_hash_creation(infos: &InfosCompte) -> String {
    let secret = std::env::var("KYC_SECRET_SERVER")
        .unwrap_or_else(|_| "uq_default_secret_change_me".to_string());

    let mut hasher = Sha256::new();
    hasher.update(infos.uq_number.as_bytes());
    hasher.update(infos.nom.as_bytes());
    hasher.update(infos.prenom.as_bytes());
    hasher.update(infos.date_naissance.as_bytes());
    hasher.update(infos.lieu_naissance.as_bytes());
    hasher.update(infos.pays.as_bytes());
    if let Some(a) = &infos.adresse { hasher.update(a.as_bytes()); }
    if let Some(t) = &infos.telephone { hasher.update(t.as_bytes()); }
    hasher.update(infos.created_at.to_le_bytes());
    hasher.update(secret.as_bytes());

    format!("{:x}", hasher.finalize())
}

pub fn transformer_hash(hash: &str) -> String {
    let base = if hash.len() >= 16 { &hash[..16] } else { hash };
    let groupes: Vec<String> = base.as_bytes()
        .chunks(4)
        .map(|c| String::from_utf8_lossy(c).to_uppercase())
        .collect();
    groupes.join("-")
}

pub fn calculer_cadenas(infos: &InfosCompte) -> String {
    let hash = calculer_hash_creation(infos);
    transformer_hash(&hash)
}

/// Meme chemin que les handlers : `data/users/{uq}.db` depuis le repertoire
/// courant (les DB ne sont pas dans le dossier de l'executable).
fn chemin_db(uq_number: &str) -> String {
    format!("data/users/{}.db", uq_number.replace("-", "_").to_lowercase())
}

/// Pose le cadenas a la creation du compte
pub fn poser_cadenas_creation(uq_number: &str, infos: &InfosCompte) -> Result<String, String> {
    let hash_brut = calculer_hash_creation(infos);
    let hash_transforme = transformer_hash(&hash_brut);

    let db_path = chemin_db(uq_number);
    let conn = Connection::open(&db_path).map_err(|e| e.to_string())?;

    conn.execute(
        "INSERT OR REPLACE INTO cadenas_verification 
         (id, uq_number, hash_brut, hash_transforme, nom, prenom, date_naissance, 
          lieu_naissance, pays, adresse, telephone, created_at, est_verifie, verrouille)
         VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 0, 1)",
        rusqlite::params![
            uq_number,
            &hash_brut,
            &hash_transforme,
            &infos.nom,
            &infos.prenom,
            &infos.date_naissance,
            &infos.lieu_naissance,
            &infos.pays,
            &infos.adresse,
            &infos.telephone,
            infos.created_at,
        ],
    ).map_err(|e| e.to_string())?;

    let (mut orb, _garde_orb) = crate::persistence::orb_charge_verrouille();
    if let Some(p) = orb.comptes.get_mut(uq_number) {
        p.cadenas = Some(hash_transforme.clone());
        p.cadenas_at = Some(infos.created_at);
        p.alerte_piratage = false;
    }
    crate::persistence::save_global_orb(&orb)?;

    Ok(hash_transforme)
}

pub fn charger_infos_compte(uq_number: &str) -> Result<InfosCompte, String> {
    let db_path = chemin_db(uq_number);
    let conn = Connection::open(&db_path).map_err(|e| e.to_string())?;

    conn.query_row(
        "SELECT uq_number, nom, prenom, date_naissance, lieu_naissance, pays, 
                adresse, telephone, created_at
         FROM cadenas_verification WHERE id = 1",
        [],
        |row| Ok(InfosCompte {
            uq_number: row.get(0)?,
            nom: row.get(1)?,
            prenom: row.get(2)?,
            date_naissance: row.get(3)?,
            lieu_naissance: row.get(4)?,
            pays: row.get(5)?,
            adresse: row.get(6)?,
            telephone: row.get(7)?,
            created_at: row.get(8)?,
        }),
    ).map_err(|_| format!("Cadenas introuvable pour {}", uq_number))
}

/// Cadenas structurel RÉEL : la DB du compte porte des identifiants
/// personnels complets ET le hash recalculé depuis ces identifiants est
/// identique au hash stocké. Retourne le hash transformé (forme affichée).
pub fn cadenas_structurel_reel(uq_number: &str) -> Result<String, String> {
    let infos = charger_infos_compte(uq_number)?;
    for champ in [
        infos.uq_number.as_str(),
        infos.nom.as_str(),
        infos.prenom.as_str(),
        infos.date_naissance.as_str(),
        infos.lieu_naissance.as_str(),
        infos.pays.as_str(),
    ] {
        if champ.trim().is_empty() {
            return Err(format!(
                "CADENAS_STRUCT: identifiants incomplets pour {}", uq_number
            ));
        }
    }
    // recompute == stocké, sinon l'identité a été modifiée après création
    verifier_cadenas(uq_number)?;
    Ok(calculer_cadenas(&infos))
}

/// 🚪 Porteur d'entrée BORB : un wallet ne peut recevoir que s'il a un
/// cadenas structurel réel (le sien, ou celui de son principal lié).
pub fn porte_cadenas_structurel(uq_number: &str) -> Result<(), String> {
    match cadenas_structurel_reel(uq_number) {
        Ok(_) => Ok(()),
        Err(e) => {
            if let Some(principal) = uq_principal_si_lie(uq_number) {
                if cadenas_structurel_reel(&principal).is_ok() {
                    return Ok(());
                }
            }
            Err(e)
        }
    }
}


// ===== / =====
/// VERIFIE SI UN UQ EST LIE A UN AUTRE (owner avec 2 profils)
/// Uniquement pour UQ-732337351 (Profil 2 du owner)
// ===== / =====
pub fn uq_principal_si_lie(uq_number: &str) -> Option<String> {
    // Verifier dans circle_members (uq_index.db)
    let trouve = Connection::open("uq_index.db").ok().and_then(|conn| {
        conn.query_row(
            "SELECT owner_uq FROM circle_members WHERE member_uq = ?1 AND is_active = 1 LIMIT 1",
            rusqlite::params![uq_number],
            |row| row.get::<_, String>(0),
        )
        .ok()
    });
    if let Some(principal) = trouve {
        eprintln!("[CADENAS-LIE] {} est lie a {} via circle_members", uq_number, principal);
        return Some(principal);
    }

    // Le profil public du owner n'a pas de DB d'identite propre : il porte le
    // cadenas de son principal, sinon aucune porte BORB ne l'accepterait et ses
    // receptions gelaient le compte en « piratage ».
    if uq_number == crate::security_login::OWNER_PUBLIC_UQ {
        eprintln!(
            "[CADENAS-LIE] {} est le profil public du owner, cadenas de {}",
            uq_number, OWNER_UQ_NUMBER
        );
        return Some(OWNER_UQ_NUMBER.to_string());
    }
    None
}

/// Verifier le cadenas d'un UQ lie (utilise le cadenas du principal)
pub fn verifier_cadenas_lie(uq_lie: &str) -> Result<(), String> {
    let principal = uq_principal_si_lie(uq_lie)
        .ok_or_else(|| format!("UQ {} n'est pas lie a un principal", uq_lie))?;
    
    // Verifier le cadenas du principal (owner)
    verifier_cadenas(&principal)?;
    
    // Le cadenas du principal est valide -> le UQ lie est valide
    eprintln!("[CADENAS-LIE] {} valide via principal {}", uq_lie, principal);
    Ok(())
}

pub fn verifier_cadenas(uq_number: &str) -> Result<(), String> {
    let infos = charger_infos_compte(uq_number)?;
    let cadenas_actuel = calculer_cadenas(&infos);

    // ====================
    // LIRE LE CADENAS DEPUIS LA DB DU COMPTE (pas le JSON)
    // ====================
    let db_path = chemin_db(uq_number);
    let conn = Connection::open(&db_path)
        .map_err(|e| format!("DB introuvable: {}", e))?;

    let cadenas_stocke: String = conn.query_row(
        "SELECT hash_transforme FROM cadenas_verification WHERE id = 1",
        [],
        |row| row.get(0),
    ).map_err(|_| format!("Cadenas introuvable pour {}", uq_number))?;

    if cadenas_actuel != cadenas_stocke {
        return Err(format!(
            "PIRATAGE — cadenas actuel ({}) != cadenas stocke ({})",
            cadenas_actuel, cadenas_stocke
        ));
    }

    Ok(())
}

/// Cadenas que ce wallet porte : le sien, ou celui de son principal quand le
/// compte est un profil lié (Profil 2 du owner, sans DB d'identité propre).
pub fn cadenas_porte_par(uq_number: &str) -> Option<String> {
    match cadenas_stocke(uq_number) {
        Some(h) if crate::block_orb::cadenas_credible(&h) => Some(h),
        _ => uq_principal_si_lie(uq_number).and_then(|p| cadenas_stocke(&p)),
    }
}

/// Cadenas transformé stocké pour un compte (None si inexistant)
pub fn cadenas_stocke(uq_number: &str) -> Option<String> {
    let db_path = chemin_db(uq_number);
    let conn = Connection::open(&db_path).ok()?;
    conn.query_row(
        "SELECT hash_transforme FROM cadenas_verification WHERE id = 1",
        [],
        |row| row.get::<_, String>(0),
    )
    .ok()
}

/// Le portefeuille est-il déverrouillé (KYC validé) ?
pub fn est_deverrouille(uq_number: &str) -> bool {
    // 1. Verification normale du cadenas de l'UQ
    let db_path = chemin_db(uq_number);
    if let Ok(conn) = Connection::open(&db_path) {
        if let Ok(verrouille) = conn.query_row::<i64, _, _>(
            "SELECT verrouille FROM cadenas_verification WHERE id = 1",
            [],
            |row| row.get(0),
        ) {
            if verrouille == 0 {
                return true;
            }
        }
    }
    
    // 2. Si c'est un UQ lie (Profil 2), verifier le cadenas du principal
    if let Some(principal) = uq_principal_si_lie(uq_number) {
        let owner_db = chemin_db(&principal);
        if let Ok(conn) = Connection::open(&owner_db) {
            if let Ok(verrouille) = conn.query_row::<i64, _, _>(
                "SELECT verrouille FROM cadenas_verification WHERE id = 1",
                [],
                |row| row.get(0),
            ) {
                if verrouille == 0 {
                    eprintln!("[CADENAS-LIE] {} deverrouille via {}", uq_number, principal);
                    return true;
                }
            }
        }
    }
    
    false
}

pub fn deverrouiller(uq_number: &str) -> Result<(), String> {
    let db_path = chemin_db(uq_number);
    let conn = Connection::open(&db_path).map_err(|e| e.to_string())?;

    conn.execute(
        "UPDATE cadenas_verification SET est_verifie = 1, verifie_at = ?1, verrouille = 0 WHERE id = 1",
        rusqlite::params![chrono::Utc::now().timestamp()],
    ).map_err(|e| e.to_string())?;

    Ok(())
}

pub fn geler_compte_piratage(uq_number: &str, details: &str) -> Result<(), String> {
    let (mut orb, _garde_orb) = crate::persistence::orb_charge_verrouille();
    if let Some(p) = orb.comptes.get_mut(uq_number) {
        p.alerte_piratage = true;
        p.alerte_details = Some(details.to_string());
    }
    crate::persistence::save_global_orb(&orb)?;
    eprintln!("🚨 ALERTE PIRATAGE : {} — {}", uq_number, details);
    Ok(())
}

pub fn est_gele(uq_number: &str) -> bool {
    let orb = crate::persistence::load_global_orb();
    orb.comptes.get(uq_number)
        .map(|p| p.alerte_piratage)
        .unwrap_or(false)
}

// ====================
// 👑 KYC OWNER AUTO-VALIDÉ (Mission 7)
// ====================
// Le propriétaire (UQ-996697110) n'a pas à fournir photo/vidéo :
// son KYC est validé d'office. Sans cela, son cadenas restait
// « verrouille = 1 » et il ne pouvait rien faire (même pas tester
// la Règle 4).
//
// Colonnes réelles de `cadenas_verification` :
//   id, uq_number, hash_brut, hash_transforme, nom, prenom,
//   date_naissance, lieu_naissance, pays, adresse, telephone,
//   created_at, verifie_at, est_verifie, verrouille, est_actif

pub const OWNER_UQ_NUMBER: &str = "UQ-996697110";

pub fn valider_kyc_owner_auto() -> Result<(), String> {
    let db_path = chemin_db(OWNER_UQ_NUMBER);
    if !std::path::Path::new(&db_path).exists() {
        return Err(format!("DB owner introuvable : {}", db_path));
    }

    let conn = Connection::open(db_path).map_err(|e| e.to_string())?;
    let now = chrono::Utc::now().timestamp();

    // Crée la ligne si elle n'existe pas encore
    conn.execute(
        "INSERT OR IGNORE INTO cadenas_verification (id, uq_number, created_at)
         VALUES (1, ?1, ?2)",
        rusqlite::params![OWNER_UQ_NUMBER, now],
    )
    .map_err(|e| e.to_string())?;

    // Valide d'office + renseigne l'identité du propriétaire
    conn.execute(
        "UPDATE cadenas_verification SET
            uq_number       = ?1,
            nom             = 'ELIEZER',
            prenom          = 'DESTIN',
            date_naissance  = '1996-06-01',
            lieu_naissance  = 'Sud-Kivu, Kabare',
            pays            = 'RDC',
            adresse         = 'Kalemi',
            telephone       = COALESCE(NULLIF(telephone,''), '+243000000000'),
            est_verifie     = 1,
            verrouille      = 0,
            est_actif       = 1,
            verifie_at      = ?2
         WHERE id = 1",
        rusqlite::params![OWNER_UQ_NUMBER, now],
    )
    .map_err(|e| e.to_string())?;

    // Cadenas structurel canonique : hash recalculé depuis l'identité
    // réelle, écrit dans la DB ET lié dans le BlockOrb (pas de placeholder).
    let infos = charger_infos_compte(OWNER_UQ_NUMBER)
        .map_err(|e| format!("Identite owner: {}", e))?;
    let hash_brut = calculer_hash_creation(&infos);
    let hash_transforme = transformer_hash(&hash_brut);
    let conn2 = Connection::open(chemin_db(OWNER_UQ_NUMBER)).map_err(|e| e.to_string())?;
    conn2.execute(
        "UPDATE cadenas_verification SET hash_brut = ?1, hash_transforme = ?2 WHERE id = 1",
        rusqlite::params![hash_brut, hash_transforme],
    ).map_err(|e| e.to_string())?;
    drop(conn2);

    // Le BlockOrb ne doit plus porter d'alerte sur le propriétaire
    let (mut orb, _garde_orb) = crate::persistence::orb_charge_verrouille();
    if let Some(p) = orb.comptes.get_mut(OWNER_UQ_NUMBER) {
        p.alerte_piratage = false;
        p.alerte_details = None;
        p.cadenas = Some(hash_transforme);
        if p.cadenas_at.is_none() {
            p.cadenas_at = Some(now);
        }
    }
    let _ = crate::persistence::save_global_orb(&orb);

    eprintln!("👑 KYC owner auto-valide (UQ-996697110) — cadenas ouvert");
    Ok(())
}

// ============================================================
// RESYNCHRONISER CADENAS - Recalcule hash_brut + hash_transforme
// avec LA MEME formule qu'a la creation (calculer_hash_creation),
// puis relie le cadenas dans le BlockOrb.
// (Avant : formule « pipe » différente -> faux PIRATAGE.)
// ============================================================
pub fn resynchroniser_cadenas(uq_number: &str) -> Result<(), String> {
    let infos = charger_infos_compte(uq_number)?;

    if infos.nom.trim().is_empty() && infos.prenom.trim().is_empty() {
        return Err("Identite vide - rien a resynchroniser".to_string());
    }

    let hash_brut = calculer_hash_creation(&infos);
    let hash_transforme = transformer_hash(&hash_brut);

    let db_path = chemin_db(uq_number);
    let conn = Connection::open(&db_path).map_err(|e| format!("DB: {}", e))?;
    conn.execute(
        "UPDATE cadenas_verification SET hash_brut = ?1, hash_transforme = ?2 WHERE id = 1",
        rusqlite::params![hash_brut, hash_transforme],
    ).map_err(|e| format!("UPDATE: {}", e))?;
    drop(conn);

    // Lien BlockOrb : le registre central porte le meme cadenas que la DB.
    let (mut orb, _garde_orb) = crate::persistence::orb_charge_verrouille();
    if let Some(p) = orb.comptes.get_mut(uq_number) {
        p.cadenas = Some(hash_transforme);
        if p.cadenas_at.is_none() {
            p.cadenas_at = Some(infos.created_at);
        }
    }
    crate::persistence::save_global_orb(&orb)?;

    eprintln!("✅ Cadenas resynchronise (formule canonique) pour {}", uq_number);
    Ok(())
}

