mod api_handlers;
mod application;
mod auth;
mod borb_distribution;
mod chat_contacts;
mod chat_reports;
mod chat_reseau;
mod command_registry;
mod db;
mod domain;
mod frais_plateforme;
mod geo;
mod guardian_routes;
mod owner_guard;
mod owner_routes;
mod p2p_signal;
mod permissions;
mod receipt;
mod secure_handlers;
mod security_helpers;
mod security_login;
mod wal;
// mod ai_vision; // en pause : dependance ort (ONNX) a installer — route /api/ai/vision desactivee
mod ai_routes;
mod api_routes;
mod audit_log;
mod auth_guard;
mod auto_cleanup;
mod backup;
mod binaire_handlers;
mod binaire_protector;
mod binance_p2p;
mod block_orb;
mod blockchain_lib;
mod borb_tracer;
mod capsule_routes;
mod cases_handlers;
mod cases_loop;
mod cases_routes;
mod chat_risk;
mod connexion;
mod contestation;
mod contestation_routes;
mod core_handlers;
mod counter_handlers;
mod database;
mod demande_calculator;
mod engines;
mod external_api;
mod external_routes;
mod gel;
mod gel_routes;
mod history_recovery;
mod history_routes;
mod invoice_handlers;
mod kyc_auto;
mod kyc_live;
mod local_history;
mod mail_routes;
mod mail_store;
mod marketplace;
mod marketplace_routes;
mod migration_comptes;
mod mini_routes;
mod montant;
mod network_routes;
mod ocean;
mod ocean_routes;
mod oracle_extensions;
mod p2p_escrow;
mod p2p_history;
mod persistence;
mod pin_manager;
mod player_routes;
mod profile_routes;
mod public_routes;
mod publication_routes;
mod real_gov;
mod real_p2p;
mod real_services;
mod real_state;
mod real_vault;
mod regulateur;
mod reputation_routes;
mod reserved_usernames;
mod robinet;
mod routes_list;
mod series_routes;
mod sms_routes;
mod sms_store;
mod subunit_unlock;
mod totp;
mod trading_routes;
mod transaction_engine;
mod treasury_handlers;
mod twofa_handlers;
mod twofa_store;
mod udp_network;
mod uq_index;
mod uq_public_alias;
mod user_handlers;
mod user_models;
mod username_utils;
mod vitrines;
mod wallet_deposit;
mod wallets_handlers;
mod wallets_store;
mod webauthn_manager;

// KYC — Verification d'identite
mod kyc;
mod security;
mod verify_routes;

// CONFIANCE — lecture des metadonnees et detection des images generees par IA.
#[path = "ai_detector/src/lib.rs"]
mod ai_detector;
mod image_meta;

// PREUVE DE PAIEMENT P2P — analyse des captures et apprentissage des poids.
mod p2p_preuve;
mod p2p_security;

// CONNEXION — liaison compte<->appareil (paliers 24 h / 48 h / 30 jours) et
// verification cryptographique reelle des assertions WebAuthn.
mod device_auth;
mod webauthn_verify;

use actix_cors::Cors;
use actix_files;
use actix_web::{web, App, HttpServer};
use database::Database;
use mini_routes::MiniServerState;
use player_routes::PlayerState;
use real_p2p::P2pState;
use real_state::GlobalState;
use std::sync::Arc;

// Load .env for local development
use dotenv::dotenv;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // ====================
    // CHARGEMENT EXPLICITE DU .env (essaie PLUSIEURS chemins)
    // ====================
    let env_loaded = {
        let mut loaded = false;
        if let Some(path) = std::env::current_exe().ok().and_then(|exe| {
            exe.parent().and_then(|dir| {
                dir.ancestors()
                    .map(|ancestor| ancestor.join(".env"))
                    .find(|path| path.is_file())
            })
        }) {
            if dotenv::from_path(&path).is_ok() {
                eprintln!("✅ .env chargé depuis: {}", path.display());
                loaded = true;
            }
        }
        let paths = vec![
            ".env",
            "../.env",
            "../../.env",
            "uq_platform_bin/.env",
            "./uq_platform_bin/.env",
        ];
        for path in &paths {
            if !loaded && std::path::Path::new(path).exists() {
                if dotenv::from_path(path).is_ok() {
                    eprintln!("✅ .env chargé depuis: {}", path);
                    loaded = true;
                    break;
                }
            }
        }
        if !loaded {
            eprintln!("⚠️  Aucun .env trouvé, essai dotenv() par défaut");
            dotenv().ok();
        }
        loaded
    };

    // Vérifier IMMÉDIATEMENT que la variable est là
    if std::env::var("OWNER_UNLOCK_CODE").is_err() {
        eprintln!("❌ OWNER_UNLOCK_CODE non trouvé après chargement");
        eprintln!("   Variables d'environnement actuelles:");
        for (key, _) in std::env::vars() {
            if key.starts_with("OWNER") || key.starts_with("JWT") {
                eprintln!("   - {}", key);
            }
        }
        std::process::exit(1);
    } else {
        eprintln!("✅ OWNER_UNLOCK_CODE trouvé");
    }

    let required_envs = [
        "OWNER_UNLOCK_CODE",
        "OWNER_PASSWORD_INITIAL",
        "TERMINAL_AUTH_KEY",
        "JWT_SECRET",
    ];
    for key in &required_envs {
        if std::env::var(key).is_err() {
            eprintln!("ERROR: required environment variable '{}' is not set. Set it in the environment or in a .env (development). Exiting.", key);
            std::process::exit(1);
        }
    }

    println!("🚀 UQ PLATFORM — Démarrage...");

    let db = Database::shared("uq_platform_db.json");
    // Reporte les likes historiques des publications dans la table persistante
    db.migrer_tables_reactions();
    // Colonnes de la vente solo (sous-titre, vendu, modifications, échéance 48 h)
    db.migrer_colonnes_zando();

    // ====================
    // MIGRATION UQ_INDEX (ajout colonnes normalized_email/pseudo)
    // ====================
    {
        let idx = uq_index::UqIndex::shared("uq_index.db");
        match idx.migrate_uq_index() {
            Ok(_) => eprintln!("✅ Migration uq_index OK"),
            Err(e) => eprintln!("⚠️  Migration uq_index : {}", e),
        }
    }
    let state = Arc::new(GlobalState::new());
    let player_state = Arc::new(PlayerState::new());
    let mini_state = Arc::new(MiniServerState::new());

    // 🆕 AJOUT — P2pState pour le réseau P2P
    let p2p_state = Arc::new(P2pState::new([0u8; 32]));

    // ====================
    // 🧠 BUS DE MOTEURS — oracle, fraude, réputation, synchro (P2P)
    // ====================
    // Les moteurs publient des mises à jour signées dans un store à
    // quota (UQ_ENGINE_QUOTA_MB, défaut 100 Mo par appareil) et les
    // diffusent aux pairs UDP ; /api/engines/gossip est le fallback HTTP
    // des appareils connectés. La synchro métier ne sort qu'aux appareils
    // appairés du même compte.
    let moteur_bus = engines::bus::BusMoteurs::nouvelles(Arc::clone(&p2p_state.udp_network));
    engines::bus::BusMoteurs::brancher_reception_udp(Arc::clone(&moteur_bus));
    {
        let bus_boucle = Arc::clone(&moteur_bus);

        // Job d'expiration : toutes les heures, expirer les demandes de 12h
        {
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
                loop {
                    interval.tick().await;
                    match crate::network_routes::expirer_demandes() {
                        Ok(expirees) => {
                            if !expirees.is_empty() {
                                log::info!("Demandes expirees : {:?}", expirees);
                            }
                        }
                        Err(e) => {
                            log::error!("Erreur expiration : {}", e);
                        }
                    }
                }
            });
        }

        // Job de purge : toutes les heures, supprimer les donnees > 7 jours (par donnee)
        {
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
                loop {
                    interval.tick().await;
                    match crate::reputation_routes::purger_donnees_anciennes() {
                        Ok((tx, chat, offers)) => {
                            if tx + chat + offers > 0 {
                                log::info!("Purge : {} tx, {} chat, {} offers", tx, chat, offers);
                            }
                        }
                        Err(e) => {
                            log::error!("Erreur purge : {}", e);
                        }
                    }
                    match crate::marketplace::jobs_periodiques() {
                        Ok(_) => {}
                        Err(e) => log::error!("Erreur jobs marketplace : {}", e),
                    }
                }
            });
        }

        // Job de jugement : toutes les heures, juger les dossiers de 3 jours
        {
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
                loop {
                    interval.tick().await;
                    let db = crate::database::Database::shared("uq_platform_db.json");
                    match crate::reputation_routes::juger_dossiers_expires(&db) {
                        Ok(juges) => {
                            if !juges.is_empty() {
                                log::info!("Dossiers juges : {:?}", juges);
                            }
                        }
                        Err(e) => {
                            log::error!("Erreur jugement : {}", e);
                        }
                    }
                    // Contestations de gel : réduction progressive des sanctions
                    // (ratio >= 80% -> -1 niveau/semaine ; 6-7 : >= 50% en 20h -> niv. 5).
                    match crate::contestation::reduire_sanctions() {
                        Ok(actions) => {
                            if !actions.is_empty() {
                                log::info!("Contestations : {:?}", actions);
                            }
                        }
                        Err(e) => {
                            log::error!("Erreur contestations : {}", e);
                        }
                    }
                }
            });
        }

        // Job marketplace : toutes les heures, sur-réservations + resynchro créneaux
        // + détection des escrows stale (PARTIE 4 — jobs_periodiques).
        {
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
                loop {
                    interval.tick().await;
                    match crate::marketplace::jobs_periodiques() {
                        Ok(v) => {
                            let actions = v
                                .get("actions")
                                .and_then(|a| a.as_array())
                                .map(|a| a.len())
                                .unwrap_or(0);
                            if actions > 0 {
                                log::info!("Jobs marketplace : {} action(s) — {}", actions, v);
                            }
                        }
                        Err(e) => {
                            log::error!("Erreur jobs marketplace : {}", e);
                        }
                    }
                }
            });
        }

        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(20)).await;
            engines::bus::boucle_moteurs(bus_boucle).await;
        });
    }
    println!(
        "🧠 Bus de moteurs initialisé (quota {} Mo)",
        std::env::var("UQ_ENGINE_QUOTA_MB").unwrap_or_else(|_| "100".to_string())
    );

    // ====================
    // 💱 MOTEUR DE TAUX — collecte multi-sources en arrière-plan
    // ====================
    // Interroge les sources, agrège à la médiane, écarte les valeurs
    // aberrantes et publie l'état lu par /api/rates/* et /api/oracle/fiat*.
    // Démarre sans attendre pour que la première requête d'un utilisateur
    // trouve une valeur réelle plutôt que de payer un timeout.
    tokio::spawn(async move {
        engines::rates::boucle().await;
    });
    println!(
        "💱 Moteur de taux démarré ({} sources déclarées, tick {} s)",
        engines::rates::sources::sources().len(),
        engines::rates::tick_secs()
    );

    // ====================
    // 🪙 FIX 9 — DÉBLOCAGE AUTOMATIQUE DES SOUS-UNITÉS (thread bg)
    // ====================
    // Vérifie toutes les 5 minutes si de nouvelles sous-unités
    // doivent être débloquées selon le prix BORB actuel.
    // ====================
    tokio::spawn(async move {
        use std::str::FromStr;
        use tokio::time::{interval, Duration};

        let mut ticker = interval(Duration::from_secs(300)); // 5 minutes

        // Attendre 30 secondes au démarrage pour que tout soit prêt
        tokio::time::sleep(Duration::from_secs(30)).await;

        loop {
            ticker.tick().await;

            // 1. Charger le prix actuel
            let prix = crate::persistence::load_prix();
            let prix_bd = match bigdecimal::BigDecimal::from_str(&prix.prix_actuel) {
                Ok(p) => p,
                Err(_) => continue,
            };

            // 2. Charger le nombre d'unités débloquées
            let current = load_unlocked_count_bg();

            // 3. Vérifier si déblocage nécessaire
            let to_unlock = crate::subunit_unlock::check_and_unlock(&prix_bd, current);

            // 4. Si déblocage, sauvegarder
            if to_unlock > 0 {
                let new_count = current + to_unlock;
                save_unlocked_count_bg(new_count);

                eprintln!("");
                eprintln!("╔══════════════════════════════════════════════════════════╗");
                eprintln!("║   🪙 DÉBLOCAGE AUTOMATIQUE                               ║");
                eprintln!("╠══════════════════════════════════════════════════════════╣");
                eprintln!(
                    "║   Nouvelles unités  : {}                                  ",
                    to_unlock
                );
                eprintln!(
                    "║   Total débloquées  : {}                                  ",
                    new_count
                );
                eprintln!(
                    "║   Prix BORB actuel   : {}                                  ",
                    prix.prix_actuel
                );
                eprintln!("╚══════════════════════════════════════════════════════════╝");
                eprintln!("");
            }
        }
    });

    // ====================
    // FONCTIONS HELPERS POUR LE THREAD BACKGROUND
    // ====================
    fn load_unlocked_count_bg() -> usize {
        let path = "data/subunits.json";
        if let Ok(content) = std::fs::read_to_string(path) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                return json
                    .get("unlocked_count")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(6) as usize;
            }
        }
        6
    }

    fn save_unlocked_count_bg(count: usize) {
        let path = "data/subunits.json";
        let json = serde_json::json!({
            "unlocked_count": count,
            "updated_at": chrono::Utc::now().timestamp()
        });
        let _ = std::fs::write(path, json.to_string());
    }

    println!("📦 Base JSON initialisée");
    println!("🎵 PlayerState initialisé");
    println!("🔐 MiniServerState initialisé");
    let udp_port = std::env::var("UDP_PORT").unwrap_or_else(|_| "8080".to_string());
    println!("🌐 P2pState initialisé (UDP sur 0.0.0.0:{})", udp_port);

    // ====================
    // 🧹 AUTO-CLEANUP — AUCUNE purge globale ici (règle 2)
    // Chaque compte gère SON cycle au démarrage de SON app : la purge est
    // déclenchée au login du compte (voir secure_handlers::finalize_*).
    // Une purge globale au boot supprimerait l'historique de tous les
    // comptes d'un coup : c'est explicitement interdit.
    // ====================

    // 🔄 PHASE D — boucle de régulation des 6 cases (toutes les 60 s)
    tokio::spawn(crate::cases_routes::boucle_regulation());

    // ====================
    // 🪟 ROBINET DE PRÉSENCE — les vitrines payent 5 % du catalogue par 24 h,
    // seconde par seconde (UQ_ROBINET_VITRINE_SECS, défaut 60). Le prélèvement
    // part du coffre du vendeur et tombe dans les 6 Cases par le même chemin
    // qu'un frais de transfert ; un solde qui ne suit pas passe la vitrine hors
    // ligne, sans jamais la purger.
    // ====================
    tokio::spawn(crate::vitrines::boucle_robinet());
    println!(
        "🪟 Robinet de présence des vitrines démarré (tick {} s)",
        crate::vitrines::tick_secs()
    );

    // 👑 KYC OWNER AUTO-VALIDE (Mission 7)
    if let Err(e) = crate::kyc::cadenas::valider_kyc_owner_auto() {
        eprintln!("KYC owner auto : {}", e);
    }

    let host = std::env::var("RUST_HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port = std::env::var("RUST_PORT").unwrap_or_else(|_| "5000".to_string());
    let bind_addr = format!("{}:{}", host, port);

    let allowed_origins = std::env::var("ALLOWED_ORIGINS").unwrap_or_else(|_| {
        "http://localhost:5173,http://127.0.0.1:5173,http://localhost:3000,http://127.0.0.1:3000,https://uqplatform.com,https://www.uqplatform.com,https://borb.uqplatform.com,https://api.uqplatform.com,capacitor://localhost,https://localhost,http://localhost".to_string()
    });
    let allowed_roots: Vec<String> = allowed_origins
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    // Sauvegarde SQLite auto : VACUUM INTO toutes les 6h, 7 dernieres gardees.
    let sqlite_path = if "uq_platform_db.json".ends_with(".json") {
        "uq_platform_db.db".to_string()
    } else {
        "uq_platform_db.db".to_string()
    };
    crate::backup::demarrer(sqlite_path);

    println!("🌐 Serveur sur http://{}", bind_addr);
    println!("🔒 Allowed origins: {:?}", allowed_roots);

    HttpServer::new(move || {
        let mut cors = Cors::default().allow_any_method().allow_any_header();

        for origin in &allowed_roots {
            cors = cors.allowed_origin(origin);
        }

        App::new()
            // ====================
            // 1. D'ABORD les routes API (avant les fichiers statiques)
            // ====================
            .configure(api_routes::configure_routes)
            // ====================
            // 2. ENSUITE les fichiers statiques
            // ====================
            .service(actix_files::Files::new("/", "static").index_file("index.html"))
            // ====================
            // 3. Données partagées
            // ====================
            .app_data(web::Data::new(db.clone()))
            .app_data(web::Data::new(state.clone()))
            .app_data(web::Data::new(player_state.clone()))
            .app_data(web::Data::new(mini_state.clone()))
            .app_data(web::Data::new(p2p_state.clone()))
            .app_data(web::Data::new(moteur_bus.clone()))
            // ====================
            // 4. CORS
            // ====================
            .wrap(cors)
    })
    .bind(&bind_addr)?
    .run()
    .await
}
