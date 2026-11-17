// ====================
// ACCOUNT SERVICE — Création et migration des comptes
// ====================

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use crate::domain::validation::{self, AccountInput};
use crate::db;
use crate::uq_index::UqIndex;

// ====================
// INPUT / OUTPUT
// ====================

#[derive(Debug, Deserialize)]
pub struct CreateAccountInput {
    pub prenom: String,
    pub nom: String,
    pub password: String,
    pub date_naissance: String,
    pub lieu_naissance: String,
    pub pays: Option<String>,
    pub adresse: Option<String>,
    pub telephone: Option<String>,
    pub email: Option<String>,
    pub pseudo: Option<String>,
    /// Code promo du PARRAIN (parrainage) — facultatif.
    pub code_promo_parrain: Option<String>,
    // ===== PROFIL COMPLET (12 Oct 2026) =====
    pub sexe: Option<String>,
    pub ville_provenance: Option<String>,
    pub ville_actuelle: Option<String>,
    pub statut_marital: Option<String>,
    /// Profession declaree a la creation (mappe sur la colonne `emploi`).
    pub profession: Option<String>,
    // ===== RECUPERATION DE COMPTE =====
    /// Email externe (Gmail ou autre) : sert a recuperer le compte, jamais a se
    /// connecter. Le mail UQ reste genere par le backend.
    pub recovery_email: Option<String>,
    /// Telephone verifie par code a usage unique (e164) — sinon telephone saisi.
    pub recovery_phone: Option<String>,
    pub phone_verified: bool,
    pub email_verified: bool,
}

#[derive(Debug, Serialize)]
pub struct AccountCreated {
    pub uq_number: String,
    pub email: String,
    pub pseudo: String,
    pub full_name: String,
    pub prenom: String,
    pub nom: String,
    pub date_naissance: String,
    pub lieu_naissance: String,
    pub pays: String,
    pub code_promo: String,
    pub cadenas: String,
    pub db_path: String,
    pub position: u64,
    /// UQ du parrain si le code promo fourni a été reconnu
    pub parrain_uq: Option<String>,
    /// Contacts de recuperation enregistres (jamais le mail UQ).
    pub recovery_email: String,
    pub recovery_phone: String,
    pub phone_verified: bool,
    pub email_verified: bool,
    /// Le code promo du parrain etait-il connu.
    pub parrain_inconnu: bool,
}

// ====================
// CREATE ACCOUNT — Fonction principale
// ====================

pub fn create_account(input: CreateAccountInput, is_owner: bool) -> Result<AccountCreated, String> {
    // ===== 1. VALIDATION =====
    let validation_input = AccountInput {
        prenom: input.prenom.clone(),
        nom: input.nom.clone(),
        password: input.password.clone(),
        date_naissance: input.date_naissance.clone(),
        lieu_naissance: input.lieu_naissance.clone(),
        pays: input.pays.clone().unwrap_or_else(|| "RDC".to_string()),
        telephone: input.telephone.clone().unwrap_or_default(),
    };

    let v = validation::validate_account_input(&validation_input);
    if !v.is_ok() {
        return Err(v.message());
    }

    let pays = input.pays.clone().unwrap_or_else(|| "RDC".to_string()).to_uppercase();
    let adresse = input.adresse.clone().unwrap_or_default();
    let telephone = input.telephone.clone().unwrap_or_default();

    // ===== 2. GÉNÉRATION UQ NUMBER =====
    let index = UqIndex::shared("uq_index.db");
    let position = index.next_position();

    use uq_blockchain::uq_address::{generate_uq_number, generate_uq_email, generate_uq_pseudo};

    // Le numéro UQ = code pays GSM + suffixe aléatoire crypto.
    // Chaque appel génère une valeur différente : la boucle de collision
    // réessaie jusqu'à trouver un numéro libre.
    let mut generated_number = generate_uq_number(&pays);
    let mut uq_attempts = 0u64;
    while index.pattern_exists("uq_number", &generated_number) {
        uq_attempts += 1;
        if uq_attempts > 1000 {
            return Err("Impossible de générer un UQ number unique".to_string());
        }
        generated_number = generate_uq_number(&pays);
    }

    // ===== 3. GENERATION MAIL / PSEUDO (collision geree) =====
    // Le mail UQ et le pseudo sont TOUJOURS generes par le backend : un email
    // externe saisi a l'inscription est un contact de recuperation, il ne
    // remplace pas l'identite UQ (voir recovery_email plus bas).
    let mut collision = 0u64;
    let mut email = String::new();
    let mut pseudo = String::new();

    loop {
        let test_email = generate_uq_email(&input.prenom, collision);
        let test_pseudo = generate_uq_pseudo(&input.prenom, &input.nom, collision);

        let email_exists = index.pattern_exists("email", &test_email)
            || index.get_account_by_email(&test_email).is_some();
        let pseudo_exists = index.pattern_exists("pseudo", &test_pseudo)
            || index.get_account_by_pseudo(&test_pseudo).is_some();

        if !email_exists && !pseudo_exists {
            email = test_email;
            pseudo = test_pseudo;
            break;
        }

        collision += 1;
        if collision > 100 {
            email = generate_uq_email(&input.prenom, position + 1);
            pseudo = generate_uq_pseudo(&input.prenom, &input.nom, position + 1);
            break;
        }
    }

    let mut code_promo = format!("UQ-{}-{}", input.prenom.to_uppercase(), position);
    let mut promo_attempts = 0u64;
    while index.pattern_exists("code_promo", &code_promo) {
        promo_attempts += 1;
        if promo_attempts > 1000 {
            return Err("Impossible de générer un code promo unique".to_string());
        }
        code_promo = format!("UQ-{}-{}", input.prenom.to_uppercase(), position + promo_attempts);
    }

    // ===== 4. CRÉATION DB + MIGRATIONS =====
    let db_filename = generated_number.replace("-", "_").to_lowercase();
    let db_path = format!("data/users/{}.db", db_filename);

    std::fs::create_dir_all("data/users").map_err(|e| e.to_string())?;

    db::init_with_migrations(&db_path)
        .map_err(|e| format!("Erreur init DB: {}", e))?;

    // ===== 5. GÉNÉRATION CLÉS WALLET =====
    let placeholder_public_key: [u8; 32] = [0u8; 32];

    // ===== 6. INSERTION uq_index =====
    let salt = format!("uq_{}_salt", generated_number.to_lowercase().replace("-", "_"));
    let hash_password = UqIndex::hash_password(&input.password, &salt);
    let hash_identity = UqIndex::hash_identity(&input.nom, &input.prenom, &pays, position);

    let _ = index.record_pattern("uq_number", &generated_number, position, &generated_number);
    let _ = index.record_pattern("email", &email, position, &generated_number);
    let _ = index.record_pattern("pseudo", &pseudo, position, &generated_number);
    let _ = index.record_pattern("code_promo", &code_promo, position, &generated_number);

    eprintln!("📋 Patterns enregistrés pour {} :", generated_number);
    eprintln!("   - uq_number  : {}", generated_number);
    eprintln!("   - email      : {}", email);
    eprintln!("   - pseudo     : {}", pseudo);
    eprintln!("   - code_promo : {}", code_promo);

    index.insert_account(
        &generated_number,
        &email,
        &pseudo,
        &hash_identity,
        &hash_password,
        &salt,
        &placeholder_public_key,
        is_owner,
        &db_path,
    ).map_err(|e| format!("Erreur index: {}", e))?;

    // ===== 6.7 ENRICHISSEMENT PROFIL =====
    if let Err(e) = index.update_profile_fields(
        &generated_number,
        input.sexe.as_deref().unwrap_or(""),
        input.ville_provenance.as_deref().unwrap_or(""),
        input.ville_actuelle.as_deref().unwrap_or(""),
        input.statut_marital.as_deref().unwrap_or(""),
    ) {
        eprintln!("[ATTENTION] Enrichissement profil echoue : {}", e);
    } else {
        eprintln!("[OK] Profil enrichi pour {}", generated_number);
    }

    // ===== 6.75 PROFESSION + CONTACTS DE RECUPERATION =====
    let recovery_email = input
        .recovery_email
        .as_deref()
        .map(|v| v.trim().to_lowercase())
        .filter(|v| crate::verify_routes::email_valide_pub(v))
        .unwrap_or_default();
    let recovery_phone = input
        .recovery_phone
        .as_deref()
        .and_then(|v| crate::verify_routes::analyser_telephone(v).map(|n| n.e164))
        .unwrap_or_default();

    if let Err(e) = index.update_creation_profil(
        &generated_number,
        input.profession.as_deref().unwrap_or(""),
        &recovery_email,
        &recovery_phone,
        input.phone_verified && !recovery_phone.is_empty(),
        input.email_verified && !recovery_email.is_empty(),
    ) {
        eprintln!("[ATTENTION] Contacts de recuperation non enregistres : {}", e);
    }

    // ===== 6.6 INSÉRER LE CODE PROMO DANS code_promos =====
    if let Err(e) = index.insert_code_promo(&code_promo, &generated_number, &input.prenom, position) {
        eprintln!("⚠️  Erreur insertion code_promo: {}", e);
    } else {
        eprintln!("✅ Code promo inséré: {}", code_promo);
    }

    // ===== 7. CONFIGURATION COMPLÈTE DU COMPTE =====
    let conn = Connection::open(&db_path).map_err(|e| e.to_string())?;
    let now = chrono::Utc::now().timestamp();

    // 7.1 user_login_config
    let (unlock, triple, pin_step, max_fails) = if is_owner { (1, 1, 3, 3) } else { (0, 0, 1, 5) };
    conn.execute(
        "INSERT OR REPLACE INTO user_login_config 
         (uq_number, is_owner, requires_unlock_code, requires_triple_attempt, pin_required_step, max_fails_before_iphone, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![&generated_number, is_owner as i64, unlock, triple, pin_step, max_fails, now],
    ).map_err(|e| e.to_string())?;

    // 7.2 identities
    let identity_type = if is_owner { "owner" } else { "public" };
    conn.execute(
        "INSERT OR REPLACE INTO identities 
         (identity_type, uq_number, public_uq, nom, prenom, email, pseudo, date_naissance, lieu_naissance, pays, adresse, telephone, is_public, is_verified, created_at, updated_at)
         VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 0, ?13, ?13)",
        params![
            identity_type, &generated_number,
            &input.nom, &input.prenom, &email, &pseudo,
            &input.date_naissance, &input.lieu_naissance, &pays,
            &adresse, &telephone,
            if is_owner { 0 } else { 1 },
            now
        ],
    ).map_err(|e| e.to_string())?;

    // 7.3 privileges
    let privileges: Vec<(&str, &str)> = if is_owner {
        vec![
            ("owner_full_access", "Accès complet propriétaire"),
            ("coffre_access", "Accès coffres"),
            ("wallet_access", "Accès portefeuille"),
            ("borb_operations", "Opérations BORB"),
            ("ubi_claim", "Réclamation UBI"),
            ("zando_publish", "Publication Zando"),
            ("kyc_bypass", "KYC auto-validé"),
            ("admin_panel", "Panneau admin"),
            ("guardian_access", "Terminal gardien"),
            ("freeze_override", "Gel/dégel comptes"),
            ("arbitration_judge", "Juge arbitrage"),
            ("validator_node", "Noeud validateur PoS"),
            ("public_transfer", "Transfert UQ public"),
        ]
    } else {
        vec![
            ("coffre_access", "Accès coffres"),
            ("wallet_access", "Accès portefeuille"),
            ("borb_operations", "Opérations BORB"),
            ("ubi_claim", "Réclamation UBI"),
            ("zando_publish", "Publication Zando"),
        ]
    };

    for (name, notes) in privileges {
        conn.execute(
            "INSERT OR REPLACE INTO privileges (privilege_name, enabled, granted_at, granted_by, notes)
             VALUES (?1, 1, ?2, 'system', ?3)",
            params![name, now, notes],
        ).ok();
    }

    // 7.4 wallet
    conn.execute(
        "INSERT OR REPLACE INTO wallet (id, balance_borb, balance_points, balance_frozen, last_updated)
         VALUES (1, 0.0, 0, 0, ?1)",
        params![now],
    ).ok();

    // 7.5 balance
    conn.execute(
        "INSERT OR REPLACE INTO balance (id, bubbles, points, frozen_points, updated_at)
         VALUES (1, 0, 0, 0, ?1)",
        params![now],
    ).ok();

    // 7.6 my_borb_units
    for unit in ["BORB", "borb", "mborb", "nborb", "pborb", "fborb"].iter() {
        conn.execute(
            "INSERT OR IGNORE INTO my_borb_units (unit, balance, created_at)
             VALUES (?1, 0, ?2)",
            params![unit, now],
        ).ok();
    }

    // 7.65 multi_wallets : creer les 6 devises par defaut
    {
        let wallets_defaut = vec![
            ("BORB", 0.0),
            ("CDF", 0.0),
            ("USD", 0.0),
            ("EUR", 0.0),
            ("BTC", 0.0),
            ("XAU", 0.0),
        ];

        for (currency, balance) in wallets_defaut.iter() {
            conn.execute(
                "INSERT OR IGNORE INTO multi_wallets (uq_number, currency, balance, updated_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![&generated_number, currency, balance, now],
            ).ok();
        }
        eprintln!("💰 6 wallets crees pour {}", generated_number);
    }

    // 7.7 my_profile
    conn.execute(
        "INSERT OR REPLACE INTO my_profile (id, full_name, phone, address, city, country, kyc_status, created_at)
         VALUES (1, ?1, ?2, ?3, '', ?4, 'pending', ?5)",
        params![
            format!("{} {}", &input.prenom, &input.nom).trim(),
            &telephone, &adresse, &pays, now
        ],
    ).ok();

    // 7.75 vaults (4 coffres)
    let vaults = vec![
        ("Coffre Principal", "owner"),
        ("Coffre UBI", "ubi"),
        ("Coffre Savings", "savings"),
        ("Coffre Shield", "shield"),
    ];
    for (name, vtype) in vaults {
        conn.execute(
            "INSERT OR IGNORE INTO vaults (vault_name, vault_type, balance_borb, balance_points, locked, created_at)
             VALUES (?1, ?2, 0.0, 0, 0, ?3)",
            params![name, vtype, now],
        ).ok();
    }

    // 7.8 pin_status
    conn.execute(
        "INSERT OR REPLACE INTO pin_status (id, pin_enabled, created_at)
         VALUES (1, 0, ?1)",
        params![now],
    ).ok();

    // 7.9 security_login_cooldown_uq
    conn.execute(
        "INSERT OR REPLACE INTO security_login_cooldown_uq 
         (uq_number, is_owner, failure_count, cooldown_level, cooldown_until, last_attempt, first_failure_at, attempts_ok, is_locked, locked_until)
         VALUES (?1, ?2, 0, 0, 0, ?3, 0, 0, 0, 0)",
        params![&generated_number, is_owner as i64, now],
    ).ok();

    // ===== 7.85 PARRAINAGE =====
    let parrain_uq: Option<String> = input
        .code_promo_parrain
        .as_deref()
        .map(|c| c.trim())
        .filter(|c| !c.is_empty())
        .and_then(|code| index.get_uq_by_code_promo(code));

    if let Some(code) = input
        .code_promo_parrain
        .as_deref()
        .map(|c| c.trim().to_uppercase())
        .filter(|c| !c.is_empty())
    {
        let _ = conn.execute(
            "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('code_promo_parrain', ?1, ?2)",
            params![&code, now],
        );

        match &parrain_uq {
            Some(p) => {
                let _ = conn.execute(
                    "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('parrain_uq', ?1, ?2)",
                    params![p, now],
                );
                eprintln!("🤝 Parrainage enregistre : {} parraine {} (code {})", p, generated_number, code);
            }
            None => {
                eprintln!("⚠️  Code promo parrain inconnu « {} » pour {}", code, generated_number);
            }
        }
    }

    // 7.10 my_private_key : GÉNÉRÉ AU KYC (pas à la création)

    // ===== 8. POSE CADENAS KYC (structurel, à la création) =====
    // Sans cadenas créé avec des identifiants exacts, pas de compte :
    // c'est ce lien qui interdit les comptes fantômes et le BORB non tracé.
    for (champ, nom) in [
        (&input.nom, "nom"), (&input.prenom, "prenom"),
        (&input.date_naissance, "date_naissance"), (&input.lieu_naissance, "lieu_naissance"),
        (&pays, "pays"),
    ] {
        if champ.trim().is_empty() {
            return Err(format!(
                "CADENAS_REQUIS: identifiant manquant ({}) — creation refusee", nom
            ));
        }
    }

    let infos_cadenas = crate::kyc::cadenas::InfosCompte {
        uq_number: generated_number.clone(),
        nom: input.nom.clone(),
        prenom: input.prenom.clone(),
        date_naissance: input.date_naissance.clone(),
        lieu_naissance: input.lieu_naissance.clone(),
        pays: pays.clone(),
        adresse: if adresse.is_empty() { None } else { Some(adresse.clone()) },
        telephone: if telephone.is_empty() { None } else { Some(telephone.clone()) },
        created_at: now,
    };

    let cadenas_hash = crate::kyc::cadenas::poser_cadenas_creation(&generated_number, &infos_cadenas)
        .map_err(|e| format!("Creation refusee — cadenas structurel impossible: {}", e))?;

    // ===== 9. PORTEFEUILLE GLOBAL =====
    // ✅ CORRECTION : tous les champs de Portefeuille sont initialisés
    {
        let (mut orb, _garde_orb) = crate::persistence::orb_charge_verrouille();
        let mut portefeuille = crate::block_orb::Portefeuille::new();
        portefeuille.solde = 0;
        portefeuille.cadenas = Some(cadenas_hash.clone());
        portefeuille.cadenas_at = Some(now);
        portefeuille.alerte_piratage = false;
        portefeuille.alerte_details = None;
        portefeuille.internal_clock = now;
        portefeuille.last_signature = None;
        portefeuille.last_tx_hash = None;
        portefeuille.universal_key_hash = None;
        portefeuille.history = Vec::new();

        orb.comptes.insert(generated_number.clone(), portefeuille);
        let _ = crate::persistence::save_global_orb(&orb);
    }

    eprintln!("✅ Compte créé: {} (position {})", generated_number, position);

    // ===== 9.5 PROVISIONNEMENT WALLET BORB =====
    // Chaque compte UQ a son wallet BORB : on provisionne le compte shell
    // sur le nœud (solde 0, sans clé — l'utilisateur rattache sa clé
    // Ed25519 via le wallet BORB plus tard). Ne fait jamais échouer
    // la création UQ : le nœud peut être momentanément injoignable.
    provisionner_wallet_borb(&generated_number);

    // ===== 9.6 JOURNAL D'AUDIT (hash-chain) =====
    // Preuve infalsifiable de l'émission : chaque entrée chaîne avec la
    // précédente. Ne contient ni mot de passe ni clé — identifiants publics
    // + hash d'identité uniquement. Un échec n'annule pas la création.
    crate::audit_log::record_account_issued(
        &generated_number,
        &email,
        &pseudo,
        &code_promo,
        &hash_identity,
    );

    let parrain_inconnu = input
        .code_promo_parrain
        .as_deref()
        .map(|c| c.trim())
        .filter(|c| !c.is_empty())
        .is_some()
        && parrain_uq.is_none();

    Ok(AccountCreated {
        uq_number: generated_number,
        email,
        pseudo,
        full_name: format!("{} {}", input.prenom, input.nom).trim().to_string(),
        prenom: input.prenom,
        nom: input.nom,
        date_naissance: input.date_naissance,
        lieu_naissance: input.lieu_naissance,
        pays,
        code_promo,
        cadenas: cadenas_hash,
        db_path,
        position,
        parrain_uq,
        recovery_email,
        recovery_phone,
        phone_verified: input.phone_verified,
        email_verified: input.email_verified,
        parrain_inconnu,
    })
}

/// Provisionne le compte BORB d'un nouveau compte UQ sur le nœud.
/// POST {BORB_NODE_URL}/compte/provisionner avec le token admin.
/// Silencieux en cas d'échec : la création UQ ne dépend pas du nœud.
fn provisionner_wallet_borb(uq_number: &str) {
    let base = std::env::var("BORB_NODE_URL").unwrap_or_else(|_| "http://127.0.0.1:8787".to_string());
    let token = std::env::var("BORB_ADMIN_TOKEN").unwrap_or_default();
    if token.is_empty() {
        eprintln!("⚠️  BORB_ADMIN_TOKEN absent — wallet BORB non provisionné pour {}", uq_number);
        return;
    }

    let body = format!("{{\"uq\":\"{}\"}}", uq_number);
    // base = http://hote:port — on parse sans dépendance externe
    let autorite = base
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .trim_end_matches('/');
    let (hote, port) = match autorite.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse::<u16>().unwrap_or(8787)),
        None => (autorite.to_string(), 8787),
    };
    if hote.contains('/') {
        return; // URL avec chemin non supportée
    }
    let requete = format!(
        "POST /compte/provisionner HTTP/1.1\r\nHost: {}:{}\r\nContent-Type: application/json\r\nX-Admin-Token: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        hote, port, token, body.len(), body
    );

    match std::net::TcpStream::connect((hote.as_str(), port)) {
        Ok(mut s) => {
            use std::io::{Read, Write};
            let _ = s.set_read_timeout(Some(std::time::Duration::from_secs(3)));
            let _ = s.set_write_timeout(Some(std::time::Duration::from_secs(3)));
            if s.write_all(requete.as_bytes()).is_ok() {
                let mut buf = [0u8; 512];
                if let Ok(n) = s.read(&mut buf) {
                    let reponse = String::from_utf8_lossy(&buf[..n]);
                    if reponse.contains("compte_provisionne") {
                        eprintln!("💰 Wallet BORB provisionné pour {}", uq_number);
                    } else {
                        eprintln!("⚠️  Provisionnement BORB refusé pour {} : {}", uq_number, reponse.lines().last().unwrap_or(""));
                    }
                }
            }
        }
        Err(e) => eprintln!("⚠️  Nœud BORB injoignable ({}:{}) — {} non provisionné : {}", hote, port, uq_number, e),
    }
}

// ====================
// MIGRATE EXISTING ACCOUNT — Réparer un compte existant
// ====================

pub fn migrate_existing_account(uq_number: &str) -> Result<(), String> {
    let db_path = format!("data/users/{}.db", uq_number.replace("-", "_").to_lowercase());

    if !std::path::Path::new(&db_path).exists() {
        return Err(format!("DB introuvable: {}", db_path));
    }

    db::init_with_migrations(&db_path)?;

    let conn = Connection::open(&db_path).map_err(|e| e.to_string())?;
    let now = chrono::Utc::now().timestamp();

    let is_owner = uq_number == "UQ-996697110";

    let existing = conn.query_row(
        "SELECT COUNT(*) FROM user_login_config WHERE uq_number = ?1",
        params![uq_number],
        |row| row.get::<_, i64>(0),
    ).unwrap_or(0);

    if existing == 0 {
        let (unlock, triple, pin_step, max_fails) = if is_owner { (1, 1, 3, 3) } else { (0, 0, 1, 5) };
        conn.execute(
            "INSERT INTO user_login_config 
             (uq_number, is_owner, requires_unlock_code, requires_triple_attempt, pin_required_step, max_fails_before_iphone, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![uq_number, is_owner as i64, unlock, triple, pin_step, max_fails, now],
        ).ok();
        eprintln!("   ✅ user_login_config créé pour {}", uq_number);
    }

    let priv_count = conn.query_row("SELECT COUNT(*) FROM privileges", [], |r| r.get::<_, i64>(0)).unwrap_or(0);
    if priv_count == 0 {
        let privileges: Vec<(&str, &str)> = if is_owner {
            vec![
                ("owner_full_access", "Accès complet propriétaire"),
                ("coffre_access", "Accès coffres"),
                ("wallet_access", "Accès portefeuille"),
                ("borb_operations", "Opérations BORB"),
                ("ubi_claim", "Réclamation UBI"),
                ("zando_publish", "Publication Zando"),
                ("kyc_bypass", "KYC auto-validé"),
                ("admin_panel", "Panneau admin"),
                ("guardian_access", "Terminal gardien"),
                ("freeze_override", "Gel/dégel comptes"),
                ("arbitration_judge", "Juge arbitrage"),
                ("validator_node", "Noeud validateur PoS"),
                ("public_transfer", "Transfert UQ public"),
            ]
        } else {
            vec![
                ("coffre_access", "Accès coffres"),
                ("wallet_access", "Accès portefeuille"),
                ("borb_operations", "Opérations BORB"),
                ("ubi_claim", "Réclamation UBI"),
                ("zando_publish", "Publication Zando"),
            ]
        };
        for (name, notes) in privileges {
            conn.execute(
                "INSERT OR REPLACE INTO privileges (privilege_name, enabled, granted_at, granted_by, notes)
                 VALUES (?1, 1, ?2, 'system', ?3)",
                params![name, now, notes],
            ).ok();
        }
        eprintln!("   ✅ privileges créés pour {}", uq_number);
    }

    let wallet_count = conn.query_row("SELECT COUNT(*) FROM wallet", [], |r| r.get::<_, i64>(0)).unwrap_or(0);
    if wallet_count == 0 {
        let points: i64 = conn.query_row("SELECT COALESCE(points, 0) FROM balance WHERE id = 1", [], |r| r.get(0)).unwrap_or(0);
        conn.execute(
            "INSERT INTO wallet (id, balance_borb, balance_points, balance_frozen, last_updated)
             VALUES (1, ?1, ?2, 0, ?3)",
            params![points as f64 / 100.0, points, now],
        ).ok();
        eprintln!("   ✅ wallet créé pour {} ({} points)", uq_number, points);
    }

    let profile_count = conn.query_row("SELECT COUNT(*) FROM my_profile", [], |r| r.get::<_, i64>(0)).unwrap_or(0);
    if profile_count == 0 {
        conn.execute(
            "INSERT OR REPLACE INTO my_profile (id, kyc_status, created_at)
             VALUES (1, 'pending', ?1)",
            params![now],
        ).ok();
    }

    eprintln!("✅ Compte migré: {}", uq_number);
    Ok(())
}

// ====================
// MIGRATE ALL EXISTING ACCOUNTS
// ====================

pub fn migrate_all_existing_accounts() -> Result<Vec<String>, String> {
    let index = UqIndex::shared("uq_index.db");
    let accounts = index.list_all();

    let mut migrated = Vec::new();

    for account in accounts {
        let uq = account.get("uq_number").and_then(|v| v.as_str()).unwrap_or("");
        if uq.is_empty() { continue; }

        match migrate_existing_account(uq) {
            Ok(_) => migrated.push(uq.to_string()),
            Err(e) => eprintln!("⚠️  {} : {}", uq, e),
        }
    }

    Ok(migrated)
}