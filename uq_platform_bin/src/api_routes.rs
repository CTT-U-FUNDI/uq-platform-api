use crate::api_handlers::*;
use crate::guardian_routes;
use crate::mini_routes;
use crate::player_routes;
use crate::publication_routes;
use crate::user_handlers;
use actix_web::{web, HttpResponse};
use serde_json::json;

pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg
        // ===== REÇUS PNG =====
        .route(
            "/api/transactions/{uq}/{id}/receipt.png",
            web::get().to(crate::receipt::routes::get_receipt_png),
        )
        // 2FA par compte UQ — wallets liés par clé publique Ed25519
        .route(
            "/api/2fa/enroll",
            web::post().to(crate::twofa_handlers::twofa_enroll),
        )
        .route(
            "/api/2fa/rotate",
            web::post().to(crate::twofa_handlers::twofa_rotate),
        )
        .route(
            "/api/2fa/status",
            web::get().to(crate::twofa_handlers::twofa_status),
        )
        .route(
            "/api/2fa/link-wallet",
            web::post().to(crate::twofa_handlers::twofa_link_wallet),
        )
        .route(
            "/api/2fa/code",
            web::post().to(crate::twofa_handlers::twofa_code),
        )
        .route(
            "/api/2fa/verify",
            web::post().to(crate::twofa_handlers::twofa_verify),
        )
        .route(
            "/api/2fa/secret-wallet",
            web::get().to(crate::twofa_handlers::twofa_secret_wallet),
        )
        .route(
            "/api/2fa/device/pair",
            web::post().to(crate::twofa_handlers::twofa_device_pair),
        )
        // Audit : statut public des chaînes + historique événements par compte
        .route(
            "/api/audit/status",
            web::get().to(crate::twofa_handlers::audit_status),
        )
        .route(
            "/api/audit/events/{uq}",
            web::get().to(crate::twofa_handlers::audit_events),
        )
        // ====================
        // WALLETS MULTI-COMPTES (max 10 : 6 online + 4 offline)
        // ====================
        .route(
            "/api/wallets/{uq}",
            web::get().to(crate::wallets_handlers::lister_wallets),
        )
        .route(
            "/api/wallets/{uq}/ajouter",
            web::post().to(crate::wallets_handlers::ajouter_wallet),
        )
        .route(
            "/api/wallets/{uq}/principal",
            web::post().to(crate::wallets_handlers::definir_principal),
        )
        .route(
            "/api/wallets/{uq}/statut",
            web::post().to(crate::wallets_handlers::maj_statut),
        )
        .route(
            "/api/wallets/{uq}/usb",
            web::post().to(crate::wallets_handlers::lier_usb),
        )
        .route(
            "/api/wallets/{uq}/code/{vk}",
            web::get().to(crate::wallets_handlers::code_2fa_wallet),
        )
        .route(
            "/api/wallets/{uq}/verifier-2fa",
            web::post().to(crate::wallets_handlers::verifier_2fa),
        )
        // ====================
        // PROTECTION BINAIRES PROPRIÉTAIRES
        // ====================
        .route(
            "/api/binaire/liste",
            web::get().to(crate::binaire_handlers::lister_binaires),
        )
        .route(
            "/api/binaire/definir",
            web::post().to(crate::binaire_handlers::definir_cle),
        )
        .route(
            "/api/binaire/decrypt",
            web::post().to(crate::binaire_handlers::decrypt_binaire),
        )
        .route(
            "/api/binaire/revoquer",
            web::post().to(crate::binaire_handlers::revoquer_cle),
        )
        .route(
            "/api/verify/receipt/{hash}",
            web::get().to(crate::receipt::routes::verify_receipt_json),
        )
        .route(
            "/verify/{hash}",
            web::get().to(crate::receipt::routes::verify_receipt_html),
        )
        // ====================
        // UQ MAIL — vraie messagerie (domaine réel via serveur mail VPS)
        // Lecture/envoi : signature Ed25519 (UQ_MAIL_V1). Entrée : clé inbound.
        // ====================
        .route(
            "/api/capsule/sceller",
            web::post().to(crate::capsule_routes::capsule_sceller),
        )
        .route(
            "/api/capsule/ouvrir",
            web::post().to(crate::capsule_routes::capsule_ouvrir),
        )
        .route(
            "/api/capsule/sante",
            web::get().to(crate::capsule_routes::capsule_sante),
        )
        .route(
            "/api/mail/adresse/{uq}",
            web::get().to(crate::mail_routes::mail_adresse),
        )
        .route(
            "/api/mail/boite/{uq}/{dossier}",
            web::get().to(crate::mail_routes::mail_lister),
        )
        .route(
            "/api/mail/message/{uq}/{id}",
            web::get().to(crate::mail_routes::mail_lire),
        )
        .route(
            "/api/mail/non-lus/{uq}",
            web::get().to(crate::mail_routes::mail_non_lus),
        )
        .route(
            "/api/mail/mark-read",
            web::post().to(crate::mail_routes::mail_mark_read),
        )
        .route(
            "/api/mail/send",
            web::post().to(crate::mail_routes::mail_send),
        )
        .route("/api/mail/in", web::post().to(crate::mail_routes::mail_in))
        // ====================
        // UQ SMS — relais OTP (vieux Android → /inbound ; signature Ed25519 user)
        // ====================
        .route(
            "/api/sms/inbound",
            web::post().to(crate::sms_routes::sms_inbound),
        )
        .route(
            "/api/sms/assign",
            web::post().to(crate::sms_routes::sms_assign),
        )
        .route(
            "/api/sms/release",
            web::post().to(crate::sms_routes::sms_release),
        )
        .route(
            "/api/sms/status/{uq}",
            web::get().to(crate::sms_routes::sms_status),
        )
        .route(
            "/api/sms/messages/{uq}",
            web::get().to(crate::sms_routes::sms_messages),
        )
        .route(
            "/api/sms/mark-read",
            web::post().to(crate::sms_routes::sms_mark_read),
        )
        .route(
            "/api/sms/pool/ajouter",
            web::post().to(crate::sms_routes::sms_pool_ajouter),
        )
        .route(
            "/api/sms/pool/libres",
            web::get().to(crate::sms_routes::sms_pool_libres),
        )
        .route(
            "/api/sms/pool/sante",
            web::get().to(crate::sms_routes::sms_pool_sante),
        )
        .route("/", web::get().to(index))
        .route("/api/logout", web::post().to(api_logout))
        // ===== UQ CHAT P2P — relais de signalement ephemere (RAM, zero stockage) =====
        .route("/api/p2p/health", web::get().to(crate::p2p_signal::health))
        .route(
            "/api/p2p/signal",
            web::post().to(crate::p2p_signal::signal_post),
        )
        .route(
            "/api/p2p/signal/{me}",
            web::get().to(crate::p2p_signal::signal_get),
        )
        .route("/api/player/status", web::get().to(api_player_status))
        .route("/api/studio/suggest", web::get().to(api_studio_suggest))
        .route("/api/studio/types", web::get().to(api_studio_types))
        .route("/api/me", web::get().to(api_me))
        // ====================
        // KYC AUTO (Tesseract)
        // ====================
        .route(
            "/api/kyc/status",
            web::get().to(crate::kyc_auto::kyc_status),
        )
        .route(
            "/api/kyc/doc/analyze",
            web::post().to(crate::kyc_auto::kyc_doc_analyze),
        )
        .route(
            "/api/kyc/face/compare",
            web::post().to(crate::kyc_auto::kyc_face_compare),
        )
        .route(
            "/api/kyc/liveness",
            web::post().to(crate::kyc_auto::kyc_liveness),
        )
        .route(
            "/api/kyc/auto/validate",
            web::post().to(crate::kyc_auto::kyc_auto_validate),
        )
        // ====================
        // KYC LIVE — guidage temps reel de la capture (moteur Python persistant)
        // ====================
        .route(
            "/api/kyc/live/session",
            web::post().to(crate::kyc_live::session_demarrer),
        )
        .service(
            web::resource("/api/kyc/live/face")
                .app_data(web::PayloadConfig::new(3_000_000))
                .route(web::post().to(crate::kyc_live::suivre_visage)),
        )
        .service(
            web::resource("/api/kyc/live/doc")
                .app_data(web::PayloadConfig::new(3_000_000))
                .route(web::post().to(crate::kyc_live::suivre_document)),
        )
        .service(
            web::resource("/api/kyc/live/check")
                .app_data(web::PayloadConfig::new(12_000_000))
                .route(web::post().to(crate::kyc_live::verifier_sequence)),
        )
        // .route("/api/ai/vision", web::post().to(crate::ai_vision::route_vision)) // ai_vision en pause (ort)
        .route(
            "/api/debug/log-error",
            web::post().to(crate::api_handlers::debug_log_error),
        )
        .route(
            "/api/admin/routes",
            web::get().to(crate::api_handlers::admin_routes),
        )
        .route(
            "/api/admin/promo-codes",
            web::get().to(crate::api_handlers::admin_promo_codes_lister),
        )
        .route(
            "/api/admin/promo-codes",
            web::post().to(crate::api_handlers::admin_promo_codes_creer),
        )
        .route("/api/ai/ask", web::post().to(crate::ai_routes::api_ai_ask))
        .route("/api/health", web::get().to(health))
        .route("/api/stats", web::get().to(stats))
        .route("/api/price/gold", web::get().to(price_gold))
        .route("/api/price/usd", web::get().to(price_usd))
        .route("/api/price/btc", web::get().to(price_btc))
        .route("/api/price/all", web::get().to(price_all))
        .route("/api/price/history/btc", web::get().to(price_history_btc))
        .route("/api/price/history/gold", web::get().to(price_history_gold))
        .route("/api/price/history/all", web::get().to(price_history_all))
        .route("/api/borb/history/{id}", web::get().to(borb_history))
        .route("/api/blockchain/info", web::get().to(blockchain_info))
        .route("/api/blockchain/supply", web::get().to(total_supply))
        .route("/api/account/create", web::post().to(account_create))
        .route("/api/account/infos/{uq}", web::get().to(account_infos))
        // ====================
        // 📞 VÉRIFICATIONS — page de création et porte anti-robot
        // Handlers déjà écrits dans verify_routes.rs ; la porte /api/human/*
        // precede l'ouverture du menu de l'ecran de connexion.
        // ====================
        .route(
            "/api/phone/check",
            web::post().to(crate::verify_routes::phone_check),
        )
        .route(
            "/api/verify/challenge",
            web::post().to(crate::verify_routes::verify_challenge),
        )
        .route(
            "/api/verify/confirm",
            web::post().to(crate::verify_routes::verify_confirm),
        )
        .route(
            "/api/human/config",
            web::get().to(crate::verify_routes::human_config),
        )
        .route(
            "/api/human/begin",
            web::post().to(crate::verify_routes::human_begin),
        )
        .route(
            "/api/human/verify",
            web::post().to(crate::verify_routes::human_verify),
        )
        .route(
            "/api/account/recovery/update",
            web::post().to(account_recovery_update),
        )
        // ====================
        // 🌐 QR DE RÉCEPTION + ORACLE DE CHANGE (Phase 1 — Coffre)
        // ⚠️ À garder AVANT /api/account/{id} (route dynamique, plus bas).
        // ====================
        .route(
            "/api/account/{uq}/qr.png",
            web::get().to(crate::public_routes::account_qr_png),
        )
        .route(
            "/api/account/{uq}/public",
            web::get().to(crate::public_routes::account_public_identity),
        )
        .route(
            "/api/account/public/{pseudo}",
            web::get().to(crate::public_routes::account_public_by_alias),
        )
        .route(
            "/api/oracle/fiat",
            web::get().to(crate::public_routes::oracle_fiat_tous),
        )
        .route(
            "/api/oracle/fiat/{currency}",
            web::get().to(crate::public_routes::oracle_fiat_devise),
        )
        // 🧠 Moteur de taux : lecture de l'état publié par le collecteur de fond.
        // Ces routes ne déclenchent aucun appel fournisseur et ne rendent jamais
        // un taux non observé (503 TAUX_INDISPONIBLE).
        .route(
            "/api/rates/live",
            web::get().to(crate::engines::rates::routes::live),
        )
        .route(
            "/api/rates/status",
            web::get().to(crate::engines::rates::routes::status),
        )
        .route(
            "/api/oracle/lock",
            web::post().to(crate::oracle_extensions::oracle_lock),
        )
        .route(
            "/api/oracle/locked/{devise}",
            web::get().to(crate::oracle_extensions::oracle_locked),
        )
        .route(
            "/api/oracle/detect",
            web::post().to(crate::oracle_extensions::oracle_detect),
        )
        .route(
            "/api/account/verification/submit",
            web::post().to(verification_submit),
        )
        .route(
            "/api/account/verification/{id}",
            web::get().to(verification_status),
        )
        .route("/api/account/verify", web::post().to(account_verify))
        .route("/api/account/recover-uq", web::post().to(account_recover_uq))
        .route(
            "/api/secure/step1",
            web::post().to(crate::secure_handlers::secure_step1),
        )
        .route(
            "/api/secure/step2",
            web::post().to(crate::secure_handlers::secure_step2),
        )
        .route(
            "/api/secure/step-login-pin",
            web::post().to(crate::secure_handlers::secure_step_login_pin),
        )
        .route(
            "/api/secure/finalize-owner",
            web::post().to(crate::secure_handlers::secure_finalize_owner),
        )
        .route(
            "/api/secure/finalize-user",
            web::post().to(crate::secure_handlers::secure_finalize_user),
        )
        .route(
            "/api/secure/forgot-password",
            web::post().to(crate::secure_handlers::secure_forgot_password),
        )
        .route(
            "/api/auth/verify-account",
            web::post().to(verify_account_exists),
        )
        // Le serveur decide du palier de connexion (24 h / 48 h / 30 jours).
        .route("/api/auth/resume", web::post().to(auth_resume))
        .route("/api/account/me", web::get().to(account_me))
        .route("/api/dev/regen-keys", web::post().to(dev_regen_keys))
        .route("/api/bubble/create", web::post().to(bubble_create))
        .route("/api/bubble/transfer", web::post().to(bubble_transfer))
        .route("/api/bubble/freeze", web::post().to(bubble_freeze))
        .route("/api/bubble/unfreeze", web::post().to(bubble_unfreeze))
        .route("/api/bubble/burn", web::post().to(bubble_burn))
        .route("/api/bubble/lend", web::post().to(bubble_lend))
        .route("/api/cases/stats", web::get().to(cases_stats))
        .route("/api/cases/live-stats", web::get().to(cases_live_stats))
        .route(
            "/api/account/wallets/transfers/{uq}",
            web::get().to(wallet_transfers),
        )
        .route(
            "/api/account/wallets/currencies/{uq}",
            web::get().to(get_wallet_currencies),
        )
        .route("/api/account/wallets/{uq}", web::get().to(get_user_wallets))
        .route(
            "/api/account/wallets/allocate",
            web::post().to(allocate_wallets),
        )
        .route(
            "/api/account/preferences/currency",
            web::post().to(set_currency_preferences),
        )
        .route(
            "/api/account/wallets/convert",
            web::post().to(convert_wallets),
        )
        .route(
            "/api/account/wallets/withdraw",
            web::post().to(withdraw_wallet),
        )
        // Dépôts mobile money : déclaration utilisateur + validation owner.
        // /api/wallet/depot/webhook est réservé aux futurs agrégateurs.
        .route(
            "/api/wallet/depot",
            web::post().to(crate::wallet_deposit::declarer),
        )
        .route(
            "/api/wallet/depot/webhook",
            web::post().to(crate::wallet_deposit::webhook_aggregateur),
        )
        .route(
            "/api/wallet/depot/{uq}",
            web::get().to(crate::wallet_deposit::mes_depots),
        )
        .route(
            "/api/wallet/depot/{id}/annuler",
            web::post().to(crate::wallet_deposit::annuler),
        )
        .route(
            "/api/owner/depots",
            web::get().to(crate::wallet_deposit::owner_liste),
        )
        .route(
            "/api/owner/depots/{id}/confirmer",
            web::post().to(crate::wallet_deposit::owner_confirmer),
        )
        .route(
            "/api/owner/depots/{id}/refuser",
            web::post().to(crate::wallet_deposit::owner_refuser),
        )
        .route(
            "/api/cases/treasury",
            web::get()
                .guard(crate::owner_guard::owner_guard())
                .to(case_treasury),
        )
        .route(
            "/api/cases/fees",
            web::get()
                .guard(crate::owner_guard::owner_guard())
                .to(case_fees),
        )
        .route(
            "/api/cases/rewards",
            web::get()
                .guard(crate::owner_guard::owner_guard())
                .to(case_rewards),
        )
        .route(
            "/api/cases/savings",
            web::get()
                .guard(crate::owner_guard::owner_guard())
                .to(case_savings),
        )
        .route(
            "/api/cases/credit",
            web::get()
                .guard(crate::owner_guard::owner_guard())
                .to(case_credit),
        )
        .route(
            "/api/cases/shield",
            web::get()
                .guard(crate::owner_guard::owner_guard())
                .to(case_shield),
        )
        .route("/api/ubi/info", web::get().to(ubi_info))
        .route("/api/ubi/claim", web::post().to(ubi_claim))
        .route("/api/exchange/order", web::post().to(exchange_order))
        .route("/api/exchange/confirm", web::post().to(exchange_confirm))
        .route("/api/exchange/complete", web::post().to(exchange_complete))
        .route("/api/exchange/cancel", web::post().to(exchange_cancel))
        .route("/api/exchange/sms", web::post().to(exchange_sms))
        // ====================
        // 💱 TRADING — marché, ordres, P2P mobile, notifications d'écran
        // ====================
        .route(
            "/api/trading/markets",
            web::get().to(crate::trading_routes::trading_markets),
        )
        .route(
            "/api/trading/context/{uq}",
            web::get().to(crate::trading_routes::trading_context),
        )
        .route(
            "/api/trading/overview/{uq}",
            web::get().to(crate::trading_routes::trading_overview),
        )
        .route(
            "/api/trading/funding/{uq}",
            web::get().to(crate::trading_routes::trading_funding),
        )
        .route(
            "/api/trading/earn/context/{uq}",
            web::get().to(crate::trading_routes::trading_earn_context),
        )
        .route(
            "/api/trading/earn/rewards/{uq}",
            web::get().to(crate::trading_routes::trading_earn_rewards),
        )
        .route(
            "/api/trading/earn/subscribe",
            web::post().to(crate::trading_routes::trading_earn_subscribe),
        )
        .route(
            "/api/trading/earn/withdraw",
            web::post().to(crate::trading_routes::trading_earn_withdraw),
        )
        .route(
            "/api/trading/orderbook/{pair}",
            web::get().to(crate::trading_routes::trading_orderbook),
        )
        .route(
            "/api/trading/trades/{pair}",
            web::get().to(crate::trading_routes::trading_trades_list),
        )
        .route(
            "/api/trading/orders",
            web::post().to(crate::trading_routes::trading_order_create),
        )
        .route(
            "/api/trading/orders/{uq}",
            web::get().to(crate::trading_routes::trading_orders_list),
        )
        .route(
            "/api/trading/performance/{uq}",
            web::get().to(crate::series_routes::trading_performance),
        )
        .route(
            "/api/trading/orders/{id}/cancel",
            web::post().to(crate::trading_routes::trading_order_cancel),
        )
        .route(
            "/api/p2p/market/offers",
            web::get().to(crate::trading_routes::p2p_offers_list),
        )
        .route(
            "/api/p2p/market/offers",
            web::post().to(crate::trading_routes::p2p_offer_create),
        )
        .route(
            "/api/p2p/market/offers/{id}/take",
            web::post().to(crate::trading_routes::p2p_offer_take),
        )
        .route(
            "/api/p2p/market/orders/{uq}",
            web::get().to(crate::trading_routes::p2p_orders_list),
        )
        .route(
            "/api/p2p/market/orders/{id}/confirm",
            web::post().to(crate::trading_routes::p2p_order_confirm),
        )
        .route(
            "/api/p2p/market/orders/{id}/complete",
            web::post().to(crate::trading_routes::p2p_order_complete),
        )
        .route(
            "/api/p2p/market/orders/{id}/cancel",
            web::post().to(crate::trading_routes::p2p_order_cancel),
        )
        .route(
            "/api/p2p/market/orders/{id}/proofs",
            web::post().to(crate::trading_routes::p2p_proof_attach),
        )
        .route(
            "/api/p2p/market/orders/{id}/proofs",
            web::get().to(crate::trading_routes::p2p_proof_list),
        )
        .route(
            "/api/p2p/market/proofs/{id}/image",
            web::get().to(crate::trading_routes::p2p_proof_image),
        )
        .route(
            "/api/p2p/receiving-methods/{uq}",
            web::get().to(crate::p2p_security::receiving_methods_list),
        )
        .route(
            "/api/p2p/receiving-methods",
            web::post().to(crate::p2p_security::receiving_method_create),
        )
        .route(
            "/api/p2p/receiving-methods/{id}/verify",
            web::post().to(crate::p2p_security::receiving_method_verify),
        )
        .route(
            "/api/p2p/receiving-methods/revoke",
            web::post().to(crate::p2p_security::receiving_method_revoke),
        )
        .route(
            "/api/p2p/market/orders/{id}/name-challenge",
            web::post().to(crate::p2p_security::blind_name_challenge),
        )
        .route(
            "/api/p2p/market/orders/{id}/codes/rotate",
            web::post().to(crate::p2p_security::rotate_cross_code),
        )
        .route(
            "/api/p2p/market/orders/{id}/codes/verify",
            web::post().to(crate::p2p_security::verify_cross_code),
        )
        .route(
            "/api/p2p/market/orders/{id}/dispute",
            web::post().to(crate::p2p_security::dispute_order),
        )
        .route(
            "/api/p2p/market/orders/{id}/arbitrate",
            web::post().to(crate::p2p_security::arbitrate_order),
        )
        .route(
            "/api/notifications/screen/{uq}",
            web::get().to(crate::trading_routes::screen_notifications),
        )
        .route(
            "/api/notifications/screen/read",
            web::post().to(crate::trading_routes::screen_notifications_read),
        )
        .route("/api/lending/offer", web::post().to(lending_offer))
        .route("/api/lending/borrow", web::post().to(lending_borrow))
        .route("/api/lending/repay", web::post().to(lending_repay))
        .route("/api/lending/liquidate", web::post().to(lending_liquidate))
        .route(
            "/api/insurance/contribute",
            web::post().to(insurance_contribute),
        )
        .route("/api/insurance/claim", web::post().to(insurance_claim))
        .route("/api/insurance/vote", web::post().to(insurance_vote))
        .route("/api/stablecoin/mint", web::post().to(stablecoin_mint))
        .route("/api/stablecoin/burn", web::post().to(stablecoin_burn))
        .route("/api/stablecoin/ratio", web::get().to(stablecoin_ratio))
        .route(
            "/api/oracle/price",
            web::post()
                .guard(crate::owner_guard::owner_guard())
                .to(oracle_report),
        )
        .route(
            "/api/oracle/current",
            web::get()
                .guard(crate::owner_guard::owner_guard())
                .to(oracle_current),
        )
        .route(
            "/api/oracle/median",
            web::get()
                .guard(crate::owner_guard::owner_guard())
                .to(oracle_median),
        )
        .route(
            "/api/oracle/validators",
            web::get()
                .guard(crate::owner_guard::owner_guard())
                .to(oracle_validators),
        )
        .route(
            "/api/governance/proposal",
            web::post().to(governance_create),
        )
        .route("/api/governance/vote", web::post().to(governance_vote))
        .route(
            "/api/governance/execute",
            web::post().to(governance_execute),
        )
        .route("/api/governance/stats", web::get().to(governance_stats))
        .route(
            "/api/fraud/report",
            web::post()
                .guard(crate::owner_guard::owner_guard())
                .to(fraud_report),
        )
        .route(
            "/api/fraud/blacklist",
            web::post()
                .guard(crate::owner_guard::owner_guard())
                .to(fraud_blacklist),
        )
        .route(
            "/api/fraud/unblacklist",
            web::post()
                .guard(crate::owner_guard::owner_guard())
                .to(fraud_unblacklist),
        )
        .route(
            "/api/reputation/endorse",
            web::post().to(reputation_endorse),
        )
        .route("/api/identity/create", web::post().to(identity_create))
        .route("/api/identity/kyc", web::post().to(identity_kyc))
        .route(
            "/api/identity/certificate",
            web::post().to(identity_certificate),
        )
        .route("/api/escrow/create", web::post().to(escrow_create))
        .route("/api/escrow/release", web::post().to(escrow_release))
        .route("/api/escrow/refund", web::post().to(escrow_refund))
        .route("/api/freeze/contract", web::post().to(freeze_create))
        .route("/api/freeze/complete", web::post().to(freeze_complete))
        .route("/api/freeze/early", web::post().to(freeze_early))
        .route("/api/shield/key-lost", web::post().to(shield_key_lost))
        .route("/api/shield/aid", web::post().to(shield_aid))
        .route("/api/shield/approve", web::post().to(shield_approve))
        .route(
            "/api/micro-credit/request",
            web::post().to(micro_credit_request),
        )
        .route(
            "/api/micro-credit/voucher",
            web::post().to(micro_credit_voucher),
        )
        .route("/api/micro-credit/fund", web::post().to(micro_credit_fund))
        .route(
            "/api/marketplace/publish",
            web::post().to(marketplace_publish),
        )
        .route("/api/marketplace/buy", web::post().to(marketplace_buy))
        .route(
            "/api/marketplace/complete",
            web::post().to(marketplace_complete),
        )
        .route("/api/offline/create", web::post().to(offline_create))
        .route("/api/offline/verify", web::post().to(offline_verify))
        .route("/api/offline/apply", web::post().to(offline_apply))
        .route("/api/mailbox/send", web::post().to(mailbox_send))
        .route("/api/mailbox/delete", web::post().to(mailbox_delete))
        .route("/api/mailbox/stats", web::get().to(mailbox_stats))
        .route(
            "/api/validator/list",
            web::get()
                .guard(crate::owner_guard::owner_guard())
                .to(validator_list),
        )
        .route(
            "/api/validator/validate",
            web::post()
                .guard(crate::owner_guard::owner_guard())
                .to(validator_validate),
        )
        .route(
            "/api/arbitration/dispute",
            web::post()
                .guard(crate::owner_guard::owner_guard())
                .to(arbitration_dispute),
        )
        .route(
            "/api/arbitration/vote",
            web::post()
                .guard(crate::owner_guard::owner_guard())
                .to(arbitration_vote),
        )
        .route(
            "/api/arbitration/status",
            web::get()
                .guard(crate::owner_guard::owner_guard())
                .to(arbitration_status),
        )
        .route("/api/network/peers", web::get().to(network_peers))
        .route("/api/network/status", web::get().to(network_status))
        .route("/api/network/threats", web::get().to(network_threats))
        .route(
            "/api/network/request",
            web::post().to(crate::network_routes::request),
        )
        .route(
            "/api/network/accept",
            web::post().to(crate::network_routes::accept),
        )
        .route(
            "/api/network/reject",
            web::post().to(crate::network_routes::reject),
        )
        .route(
            "/api/network/leave",
            web::post().to(crate::network_routes::leave),
        )
        .route(
            "/api/network/list",
            web::get().to(crate::network_routes::list),
        )
        .route("/api/d2d/connect", web::post().to(d2d_connect))
        .route("/api/d2d/disconnect", web::post().to(d2d_disconnect))
        .route("/api/d2d/scan", web::get().to(d2d_scan))
        .route("/api/d2d/peers", web::get().to(d2d_peers))
        .route("/api/mesh/relay", web::post().to(mesh_relay))
        .route("/api/mesh/routing", web::get().to(mesh_routing))
        .route("/api/mesh/route", web::get().to(mesh_route))
        .route("/api/mesh/sellers", web::get().to(mesh_sellers))
        // ===== BUS DE MOTEURS P2P =====
        .route(
            "/api/engines/etat",
            web::get().to(crate::engines::handlers::engines_etat),
        )
        .route(
            "/api/engines/pairs",
            web::get().to(crate::engines::handlers::engines_pairs),
        )
        .route(
            "/api/engines/gossip",
            web::post().to(crate::engines::handlers::engines_gossip),
        )
        .route(
            "/api/engines/appareil",
            web::post().to(crate::engines::handlers::engines_appareil),
        )
        .route(
            "/api/engines/evenement",
            web::post().to(crate::engines::handlers::engines_evenement),
        )
        .route(
            "/api/engines/oracle",
            web::get().to(crate::engines::handlers::engines_oracle),
        )
        .route(
            "/api/engines/reputation/{uq}",
            web::get().to(crate::engines::handlers::engines_reputation),
        )
        .route(
            "/api/engines/synchro/{uq}",
            web::get().to(crate::engines::handlers::engines_synchro),
        )
        .route("/api/frequency/status", web::get().to(frequency_status))
        .route(
            "/api/phantom/create",
            web::post().to(phantom_create_channel),
        )
        .route("/api/phantom/channels", web::get().to(phantom_channels))
        .route("/api/emergency/sos", web::post().to(emergency_sos))
        .route("/api/emergency/beacon", web::get().to(emergency_beacon))
        .route("/api/internet/sell", web::post().to(internet_sell))
        .route("/api/transport/status", web::get().to(transport_status))
        .route("/api/autopilot/status", web::get().to(autopilot_status))
        .route("/api/security/status", web::get().to(security_status))
        .route("/api/security/root", web::get().to(security_root_check))
        .route(
            "/api/security/emulator",
            web::get().to(security_emulator_check),
        )
        .route(
            "/api/security/biometric",
            web::get().to(security_biometric_sync),
        )
        .route("/api/security/memory", web::get().to(security_memory_guard))
        .route(
            "/api/security/storage",
            web::get().to(security_secure_storage),
        )
        .route(
            "/api/security/side-channel",
            web::get().to(security_side_channel),
        )
        .route("/api/security/ipc", web::get().to(security_trusted_ipc))
        .route("/api/security/vtee", web::post().to(security_vtee_generate))
        .route("/api/vtee/enclave", web::post().to(vtee_enclave))
        .route("/api/vtee/attest", web::post().to(vtee_attest))
        .route("/api/keys/derive", web::post().to(keys_derive))
        .route("/api/secure/store", web::post().to(secure_store))
        .route("/api/secure/retrieve", web::get().to(secure_retrieve))
        // Le bandeau d'etat du chat : compteurs mesures, pas un JSON fige.
        .route(
            "/api/chat/status",
            web::get().to(crate::chat_reseau::chat_reseau_status),
        )
        .route(
            "/api/chat/read",
            web::post().to(crate::chat_reseau::chat_marquer_lu),
        )
        // Signalement d'un pair : enregistre cote plateforme, traite par la
        // moderation, et branche sur le score de confiance.
        .route(
            "/api/chat/report",
            web::post().to(crate::chat_reports::chat_signaler),
        )
        .route(
            "/api/chat/reports",
            web::get().to(crate::chat_reports::chat_signalements_list),
        )
        .route(
            "/api/chat/reports/{id}/review",
            web::post().to(crate::chat_reports::chat_signalement_review),
        )
        .route("/api/chat/send", web::post().to(chat_send))
        .route("/api/chat/conversation", web::post().to(chat_conversation))
        .route("/api/chat/encrypt", web::post().to(chat_encrypt))
        // ===== PUBLICATIONS EPHEMERES (7 jours) =====
        .route(
            "/api/publications",
            web::get().to(publication_routes::publication_list),
        )
        .route(
            "/api/publications",
            web::post().to(publication_routes::publication_create),
        )
        .route(
            "/api/publications/{id}/like",
            web::post().to(crate::profile_routes::publication_like_add),
        )
        .route(
            "/api/publications/{id}/like",
            web::delete().to(crate::profile_routes::publication_like_remove),
        )
        .route(
            "/api/publications/{id}/view",
            web::post().to(crate::profile_routes::publication_view_record),
        )
        .route(
            "/api/publications/{id}/comment",
            web::post().to(publication_routes::publication_comment),
        )
        // ===== PROFIL PUBLIC : stats, score, abonnés, notation =====
        .route(
            "/api/profile/stats/{uq}",
            web::get().to(crate::profile_routes::profile_stats),
        )
        .route(
            "/api/profile/{uq}/note",
            web::post().to(crate::profile_routes::profile_note),
        )
        .route(
            "/api/follow/{uq}",
            web::post().to(crate::profile_routes::follow_add),
        )
        .route(
            "/api/follow/{uq}",
            web::delete().to(crate::profile_routes::follow_remove),
        )
        .route(
            "/api/follow/{uq}/followers",
            web::get().to(crate::profile_routes::follow_followers),
        )
        .route(
            "/api/follow/{uq}/following",
            web::get().to(crate::profile_routes::follow_following),
        )
        .route("/api/data/market", web::get().to(data_market))
        .route("/api/data/stats", web::get().to(data_stats))
        .route("/api/data/publish", web::post().to(data_publish))
        .route("/api/data/purchase", web::post().to(data_purchase))
        .route("/api/data/tunnel", web::post().to(data_tunnel))
        .route("/api/data/price", web::post().to(data_price))
        .route(
            "/api/shop/create",
            web::post().to(user_handlers::shop_create),
        )
        .route("/api/search", web::get().to(user_handlers::search))
        .route(
            "/api/location/update",
            web::post().to(user_handlers::location_update),
        )
        .route("/api/shops", web::get().to(shops_list))
        .route("/api/shops/list", web::get().to(user_handlers::shop_list))
        .route(
            "/api/shop/product/add",
            web::post().to(user_handlers::product_add),
        )
        .route(
            "/api/shop/product/buy",
            web::post().to(user_handlers::product_buy),
        )
        .route(
            "/api/search/nearby",
            web::get().to(user_handlers::search_nearby),
        )
        .route(
            "/api/location/distance",
            web::post().to(user_handlers::location_distance),
        )
        .route("/api/vault/status", web::get().to(vault_status))
        .route("/api/vault/guardians", web::get().to(vault_guardians))
        .route(
            "/api/vault/recovery/status",
            web::get().to(vault_recovery_status),
        )
        .route("/api/vault/authenticity", web::get().to(vault_authenticity))
        .route("/api/vault/compromised", web::get().to(vault_compromised))
        .route("/api/vault/p2p/usage", web::get().to(vault_p2p_usage))
        .route("/api/vault/split", web::post().to(vault_split))
        .route("/api/vault/combine", web::post().to(vault_combine))
        .route(
            "/api/vault/guardian/replace",
            web::post().to(vault_guardian_replace),
        )
        // Lecteur UQ (player)
        .route(
            "/api/player/start",
            web::post().to(player_routes::player_start),
        )
        .route(
            "/api/player/tick",
            web::post().to(player_routes::player_tick),
        )
        .route(
            "/api/player/stop",
            web::post().to(player_routes::player_stop),
        )
        .route(
            "/api/player/stats",
            web::get().to(player_routes::player_stats),
        )
        .route("/api/vault/proof", web::post().to(vault_proof))
        .route(
            "/api/vault/proof/verify",
            web::post().to(vault_proof_verify),
        )
        .route(
            "/api/vault/recovery/start",
            web::post().to(vault_recovery_start),
        )
        .route("/api/vault/session", web::post().to(vault_session))
        .route("/api/vault/p2p/store", web::post().to(vault_p2p_store))
        .route(
            "/api/validator/approvals",
            web::get()
                .guard(crate::owner_guard::owner_guard())
                .to(validator_approvals),
        )
        .route("/api/sellers", web::get().to(sellers_list))
        .route("/api/mini/peers", web::get().to(mini_peers))
        .route("/api/dao/stats", web::get().to(dao_stats))
        .route("/api/publish", web::post().to(publish_item))
        .route("/api/verify/youtube", web::post().to(verify_youtube))
        .route("/api/verify/spotify", web::post().to(verify_spotify))
        .route("/api/verify/instagram", web::post().to(verify_instagram))
        .route("/api/verify/tiktok", web::post().to(verify_tiktok))
        .route(
            "/api/biometric/template",
            web::post().to(biometric_template),
        )
        .route("/api/biometric/verify", web::post().to(biometric_verify))
        .route(
            "/api/biometric/liveness",
            web::post().to(biometric_liveness),
        )
        .route(
            "/api/biometric/spoofing",
            web::post().to(biometric_spoofing),
        )
        .route(
            "/api/owner/demande/current",
            web::get().to(owner_demande_current),
        )
        .route(
            "/api/owner/demande/recalculate",
            web::post().to(owner_demande_recalculate),
        )
        .route("/api/owner/force-price", web::post().to(owner_force_price))
        .route(
            "/api/owner/force-demande",
            web::post().to(owner_force_demande),
        )
        .route(
            "/api/borb/subunits/status",
            web::get().to(borb_subunits_status),
        )
        .route(
            "/api/borb/subunits/check-unlock",
            web::post().to(borb_subunits_check_unlock),
        )
        .route("/api/borb/stats", web::get().to(borb_tracer_stats))
        .route(
            "/api/borb/origin/{trace_key}",
            web::get().to(borb_get_origin),
        )
        .route(
            "/api/borb/origins/{recipient_uq}",
            web::get().to(borb_list_origins),
        )
        .route("/api/borb/units", web::get().to(borb_units_info))
        .route("/api/borb/supply", web::get().to(borb_supply))
        .route("/api/borb/convert", web::post().to(borb_convert))
        .route("/api/borb/send", web::post().to(borb_send))
        // ⚠️ /api/borb/receive/{id} est déclaré PLUS BAS avec owner_guard :
        //    ne pas le réenregistrer ici sans guard (la 1re route gagne).
        .route(
            "/api/borb/transactions/{id}",
            web::get().to(borb_transactions),
        ) // sécurisé dans le handler
        .route(
            "/api/borb/balance/series/{id}",
            web::get().to(crate::series_routes::borb_balance_series),
        )
        .route(
            "/api/analytics/wallet/{id}",
            web::get().to(crate::series_routes::wallet_analytics),
        )
        .route("/api/p2p/start", web::post().to(p2p_start))
        .route("/api/p2p/send", web::post().to(p2p_send))
        .route("/api/p2p/broadcast", web::post().to(p2p_broadcast))
        .route("/api/p2p/peers", web::get().to(p2p_peers))
        .route("/api/savings/contract", web::post().to(savings_contract))
        .route("/api/savings/yield", web::get().to(savings_yield))
        .route(
            "/api/guardian/login",
            web::post().to(guardian_routes::guardian_login),
        )
        .route(
            "/api/guardian/unlock",
            web::post().to(guardian_routes::guardian_unlock),
        )
        .route(
            "/api/guardian/commands",
            web::get().to(guardian_routes::guardian_commands),
        )
        .route(
            "/api/guardian/execute",
            web::post().to(guardian_routes::guardian_execute),
        )
        .route(
            "/api/guardian/status",
            web::get().to(guardian_routes::guardian_status),
        )
        .route(
            "/api/guardian/logout",
            web::post().to(guardian_routes::guardian_logout),
        )
        .route(
            "/api/distribution/stats",
            web::get()
                .guard(crate::owner_guard::owner_guard())
                .to(distribution_stats),
        )
        .route(
            "/api/distribution/treasury",
            web::get()
                .guard(crate::owner_guard::owner_guard())
                .to(distribution_treasury_status),
        )
        .route(
            "/api/distribution/delis",
            web::get()
                .guard(crate::owner_guard::owner_guard())
                .to(distribution_delis_status),
        )
        .route(
            "/api/distribution/sell-crypto",
            web::post()
                .guard(crate::owner_guard::owner_guard())
                .to(distribution_sell_crypto),
        )
        .route(
            "/api/distribution/sell-p2p",
            web::post()
                .guard(crate::owner_guard::owner_guard())
                .to(distribution_sell_p2p),
        )
        .route(
            "/api/distribution/recharge",
            web::post()
                .guard(crate::owner_guard::owner_guard())
                .to(distribution_auto_recharge),
        )
        .route(
            "/api/distribution/repartition",
            web::post().to(distribution_repartition),
        )
        .route(
            "/api/distribution/repartition/status",
            web::get().to(distribution_repartition_status),
        )
        .route(
            "/api/distribution/claim",
            web::post()
                .guard(crate::owner_guard::owner_guard())
                .to(distribution_claim),
        )
        .route(
            "/api/distribution/claim-with-pin",
            web::post().to(distribution_claim_with_pin),
        )
        .route("/api/kyc/can-reclaim", web::get().to(can_reclaim))
        .route(
            "/api/distribution/auto-validate-kyc",
            web::post().to(distribution_auto_validate_kyc),
        )
        .route(
            "/api/distribution/ubi",
            web::post()
                .guard(crate::owner_guard::owner_guard())
                .to(distribution_ubi_distribute),
        )
        // Journal individuel
        .route(
            "/api/journal/export/{account_id}",
            web::get().to(journal_export),
        )
        .route(
            "/api/journal/summary/{account_id}",
            web::get().to(journal_summary),
        )
        // ====================
        // 🛒 ZANDO — ROUTES
        // ====================
        // ===== ZANDO ÉLÉMENTS (6 routes) =====
        .route("/api/zando/item/create", web::post().to(zando_item_create))
        .route("/api/zando/items", web::get().to(zando_items_list))
        .route("/api/zando/items/{shop_id}", web::get().to(zando_items_get))
        .route(
            "/api/zando/analytics/{shop_id}",
            web::get().to(zando_seller_analytics),
        )
        .route("/api/zando/item/{id}", web::get().to(zando_item_get))
        .route(
            "/api/zando/item/{id}/interact",
            web::post().to(zando_item_interact),
        )
        .route(
            "/api/zando/item/{id}/interactions",
            web::get().to(zando_item_interactions),
        )
        .route(
            "/api/zando/item/{id}/update",
            web::post().to(zando_item_update),
        )
        .route(
            "/api/zando/item/{id}/delete",
            web::post().to(zando_item_delete),
        )
        .route(
            "/api/zando/item/{id}/publish",
            web::post().to(zando_item_publish),
        )
        .route("/api/zando/item/{id}/sold", web::post().to(zando_item_sold))
        // Reputation
        .route(
            "/api/reputation/{uq}",
            web::get().to(crate::reputation_routes::get_reputation),
        )
        .route(
            "/api/reputation/{uq}/action",
            web::post().to(crate::reputation_routes::add_action),
        )
        .route(
            "/api/reputation/{uq}/report",
            web::post().to(crate::reputation_routes::add_report),
        )
        .route(
            "/api/reputation/{uq}/verify",
            web::post().to(crate::reputation_routes::verify_report),
        )
        .route(
            "/api/reputation/{uq}/achat",
            web::post().to(crate::reputation_routes::test_achat),
        )
        // Dossiers publics
        .route(
            "/api/dossiers",
            web::get().to(crate::reputation_routes::list_dossiers),
        )
        .route(
            "/api/dossiers/{id}",
            web::get().to(crate::reputation_routes::get_dossier),
        )
        .route(
            "/api/dossiers/{id}/vote",
            web::post().to(crate::reputation_routes::vote_dossier),
        )
        .route(
            "/api/dossiers/{id}/comment",
            web::post().to(crate::reputation_routes::comment_dossier),
        )
        // ===== MARKETPLACE (escrow + machine a etats) =====
        .route(
            "/api/marketplace/transactions",
            web::post().to(crate::marketplace_routes::creer),
        )
        .route(
            "/api/marketplace/mes",
            web::get().to(crate::marketplace_routes::mes),
        )
        .route(
            "/api/marketplace/creneaux",
            web::post().to(crate::marketplace_routes::creer_creneau),
        )
        .route(
            "/api/marketplace/creneaux/{produit_id}",
            web::get().to(crate::marketplace_routes::creneaux),
        )
        .route(
            "/api/marketplace/assurance",
            web::post().to(crate::marketplace_routes::assurance),
        )
        .route(
            "/api/marketplace/assurance",
            web::get().to(crate::marketplace_routes::assurance_statut),
        )
        .route(
            "/api/marketplace/avertir",
            web::post().to(crate::marketplace_routes::avertir),
        )
        .route(
            "/api/marketplace/{id}",
            web::get().to(crate::marketplace_routes::detail),
        )
        .route(
            "/api/marketplace/{id}/payer",
            web::post().to(crate::marketplace_routes::payer),
        )
        .route(
            "/api/marketplace/{id}/accepter",
            web::post().to(crate::marketplace_routes::accepter),
        )
        .route(
            "/api/marketplace/{id}/refuser",
            web::post().to(crate::marketplace_routes::refuser),
        )
        .route(
            "/api/marketplace/{id}/livreur",
            web::post().to(crate::marketplace_routes::assigner_livreur),
        )
        .route(
            "/api/marketplace/{id}/livreur_accepte",
            web::post().to(crate::marketplace_routes::livreur_accepte),
        )
        .route(
            "/api/marketplace/{id}/livreur_refuse",
            web::post().to(crate::marketplace_routes::livreur_refuse),
        )
        .route(
            "/api/marketplace/{id}/gps",
            web::post().to(crate::marketplace_routes::gps),
        )
        .route(
            "/api/marketplace/{id}/positions",
            web::get().to(crate::marketplace_routes::positions),
        )
        .route(
            "/api/marketplace/{id}/action",
            web::post().to(crate::marketplace_routes::action),
        )
        .route(
            "/api/marketplace/{id}/mode-hors-uq/acheteur",
            web::post().to(crate::marketplace_routes::hors_uq_acheteur),
        )
        .route(
            "/api/marketplace/{id}/mode-hors-uq/vendeur",
            web::post().to(crate::marketplace_routes::hors_uq_vendeur),
        )
        .route(
            "/api/marketplace/{id}/mode-hors-uq/refuser",
            web::post().to(crate::marketplace_routes::hors_uq_refuser),
        )
        .route(
            "/api/marketplace/{id}/hors-uq/litige",
            web::post().to(crate::marketplace_routes::hors_uq_litige),
        )
        .route(
            "/api/marketplace/{id}/hors-uq/terminer",
            web::post().to(crate::marketplace_routes::hors_uq_terminer),
        )
        .route(
            "/api/marketplace/jobs",
            web::post().to(crate::marketplace_routes::jobs),
        )
        .route(
            "/api/livreur/register",
            web::post().to(crate::marketplace_routes::livreur_register),
        )
        .route(
            "/api/livreur/list",
            web::get().to(crate::marketplace_routes::livreur_list),
        )
        .route(
            "/api/livreur/remove",
            web::post().to(crate::marketplace_routes::livreur_remove),
        )
        .route(
            "/api/account/warnings",
            web::get().to(crate::marketplace_routes::warnings),
        )
        .route(
            "/api/account/restrictions",
            web::get().to(crate::marketplace_routes::restrictions),
        )
        // Gel gradué (7 niveaux) — application admin, consultation soi-même.
        .route(
            "/api/gel/appliquer",
            web::post()
                .guard(crate::owner_guard::owner_guard())
                .to(crate::gel_routes::appliquer),
        )
        .route(
            "/api/gel/degeler",
            web::post()
                .guard(crate::owner_guard::owner_guard())
                .to(crate::gel_routes::degeler),
        )
        .route(
            "/api/gel/escalader",
            web::post()
                .guard(crate::owner_guard::owner_guard())
                .to(crate::gel_routes::escalader),
        )
        .route("/api/gel/peut/{uq}", web::get().to(crate::gel_routes::peut))
        .route("/api/gel/{uq}", web::get().to(crate::gel_routes::voir))
        // Contestation publique des gels (dépôt + commentaires + votes + job)
        .route(
            "/api/contestation/ouvrir",
            web::post().to(crate::contestation_routes::ouvrir),
        )
        .route(
            "/api/contestation/mes",
            web::get().to(crate::contestation_routes::mes),
        )
        .route(
            "/api/contestation",
            web::get().to(crate::contestation_routes::liste),
        )
        .route(
            "/api/contestation/{id}",
            web::get().to(crate::contestation_routes::voir),
        )
        .route(
            "/api/contestation/{id}/commenter",
            web::post().to(crate::contestation_routes::commenter),
        )
        .route(
            "/api/contestation/{id}/voter",
            web::post().to(crate::contestation_routes::voter),
        )
        // ===== ZANDO AGENTS (3 routes) =====
        .route(
            "/api/zando/agent/become",
            web::post().to(zando_agent_become),
        )
        .route(
            "/api/zando/agents/{item_id}",
            web::get().to(zando_agents_get),
        )
        .route("/api/zando/agent/sale", web::post().to(zando_agent_sale))
        // ===== ZANDO CONTACTS (5 routes) =====
        .route(
            "/api/zando/contacts/request",
            web::post().to(zando_contact_request),
        )
        .route(
            "/api/zando/contacts/{id}/accept",
            web::post().to(zando_contact_accept),
        )
        .route(
            "/api/zando/contacts/{id}/refuse",
            web::post().to(zando_contact_refuse),
        )
        .route(
            "/api/zando/contacts/share",
            web::post().to(zando_contact_share),
        )
        .route(
            "/api/zando/contacts/shared/{user_id}",
            web::get().to(zando_contacts_shared),
        )
        // ===== ZANDO DRAFTS (3 routes) =====
        .route("/api/zando/drafts", web::post().to(zando_draft_save))
        .route("/api/zando/drafts/{id}", web::get().to(zando_draft_get))
        .route(
            "/api/zando/drafts/{id}",
            web::delete().to(zando_draft_delete),
        )
        // ===== CUSTOM TYPES (5 routes) =====
        .route(
            "/api/zando/custom-type/create",
            web::post().to(custom_type_create),
        )
        .route("/api/zando/custom-types", web::get().to(custom_types_list))
        .route(
            "/api/zando/custom-types/owner/{owner_id}",
            web::get().to(custom_types_by_owner),
        )
        .route(
            "/api/zando/custom-type/{id}",
            web::get().to(custom_type_get),
        )
        .route(
            "/api/zando/custom-type/{id}",
            web::delete().to(custom_type_delete),
        )
        .route("/api/uq/generate", web::post().to(uq_generate_address))
        // ===== CONTACTS (5 routes) =====
        .route(
            "/api/contact/request",
            web::post().to(contact_request_create),
        )
        .route(
            "/api/contact/request/{id}/accept",
            web::post().to(contact_request_accept),
        )
        .route(
            "/api/contact/request/{id}/refuse",
            web::post().to(contact_request_refuse),
        )
        .route("/api/contact/share", web::post().to(contact_share))
        .route(
            "/api/contact/shared/{user_id}",
            web::get().to(contact_shared_get),
        )
        // ===== DRAFTS (2 routes) =====
        .route("/api/draft/save", web::post().to(draft_save))
        .route("/api/draft/{id}/delete", web::post().to(draft_delete))
        // ====================
        // 🔗 ROUTES ORPHELINES — Handlers existants non connectés
        // ====================
        // ===== COMPTE =====
        .route(
            "/api/account/{id}",
            web::get()
                .guard(crate::owner_guard::owner_guard())
                .to(account_get),
        )
        .route(
            "/api/account/{id}/balance",
            web::get()
                .guard(crate::owner_guard::owner_guard())
                .to(account_balance),
        )
        .route(
            "/api/account/preferences/{id}",
            web::get().to(account_preferences_get),
        )
        .route("/api/uq/pseudos", web::get().to(uq_search_pseudos))
        .route(
            "/api/account/preferences",
            web::post().to(account_preferences_set),
        )
        .route(
            "/api/account/profile/update",
            web::post().to(account_profile_update),
        )
        .route(
            "/api/account/pin/verify",
            web::post().to(account_pin_verify_v2),
        )
        .route("/api/account/pin/set", web::post().to(account_pin_set_v2))
        .route(
            "/api/account/pin/set-v2",
            web::post().to(account_pin_set_v2),
        )
        .route(
            "/api/account/pin/status",
            web::post().to(account_pin_status_v2),
        )
        .route(
            "/api/account/pin/verify-password",
            web::post().to(account_pin_verify_password),
        )
        .route(
            "/api/account/password/change",
            web::post().to(account_password_change),
        )
        .route(
            "/api/account/pseudo/set",
            web::post().to(account_pseudo_set),
        )
        // ===== WEBAUTHN ROUTES =====
        .route(
            "/api/webauthn/register/begin",
            web::post().to(webauthn_register_begin),
        )
        .route(
            "/api/webauthn/register/complete",
            web::post().to(webauthn_register_complete),
        )
        .route(
            "/api/webauthn/login/begin",
            web::post().to(webauthn_login_begin),
        )
        .route(
            "/api/webauthn/login/complete",
            web::post().to(webauthn_login_complete),
        )
        .route(
            "/api/webauthn/transaction/begin",
            web::post().to(webauthn_transaction_begin),
        )
        .route(
            "/api/webauthn/transaction/complete",
            web::post().to(webauthn_transaction_complete),
        )
        .route("/api/webauthn/list", web::post().to(webauthn_list))
        .route("/api/webauthn/remove", web::post().to(webauthn_remove))
        .route(
            "/api/account/pin/remove",
            web::post().to(account_pin_remove_v2),
        )
        .route(
            "/api/account/pin/remove-v2",
            web::post().to(account_pin_remove_v2),
        )
        .route(
            "/api/account/devices/{id}",
            web::get().to(account_devices_list),
        )
        .route(
            "/api/account/devices/revoke",
            web::post().to(account_devices_revoke),
        )
        .route(
            "/api/account/avatar/set",
            web::post().to(account_avatar_set),
        )
        .route(
            "/api/account/avatar/get",
            web::post().to(account_avatar_get),
        )
        .route(
            "/api/account/font-size/set",
            web::post().to(account_font_size_set),
        )
        // ===== BULLE =====
        .route("/api/bubble/{id}", web::get().to(bubble_get))
        // ===== UBI =====
        .route("/api/ubi/check/{id}", web::get().to(ubi_check))
        // ===== IDENTITÉ =====
        .route("/api/identity/{id}", web::get().to(identity_get))
        .route("/api/identity/{id}/verify", web::post().to(identity_verify))
        // ===== GOUVERNANCE =====
        .route("/api/governance/proposals", web::get().to(governance_list))
        // ===== FRÉQUENCE =====
        .route("/api/frequency/hop", web::post().to(frequency_hop_now))
        // ===== CHAT =====
        .route("/api/chat/inbox/{user_id}", web::get().to(chat_inbox))
        .route("/api/chat/messages/{id}", web::get().to(chat_messages))
        .route(
            "/api/chat/contacts/{user_id}",
            web::get().to(crate::chat_contacts::chat_contacts_list),
        )
        .route(
            "/api/chat/contacts",
            web::post().to(crate::chat_contacts::chat_contact_update),
        )
        .route(
            "/api/chat/settings",
            web::post().to(crate::chat_contacts::chat_settings_update),
        )
        .service(
            web::resource("/api/chat/groups/{id}")
                .route(web::get().to(chat_groups_list))
                .route(web::delete().to(chat_group_delete)),
        )
        .route("/api/chat/groups", web::post().to(chat_group_create))
        .route(
            "/api/chat/groups/{id}/join",
            web::post().to(chat_group_join),
        )
        .route(
            "/api/chat/groups/{id}/leave",
            web::post().to(chat_group_leave),
        )
        .route(
            "/api/chat/groups/{id}/members",
            web::post().to(chat_group_invite),
        )
        .service(
            web::resource("/api/chat/groups/{id}/messages")
                .route(web::get().to(chat_group_messages))
                .route(web::post().to(chat_group_send)),
        )
        // ===== BLACKKEY =====
        .route("/api/blackkey/sign", web::post().to(blackkey_sign))
        .route("/api/blackkey/decrypt", web::post().to(blackkey_decrypt))
        .route("/api/blackkey/mutate", web::post().to(blackkey_mutate))
        .route("/api/blackkey/integrity", web::get().to(blackkey_integrity))
        // ===== BORB =====
        .route("/api/borb/balance/{id}", web::get().to(borb_balance))
        .route("/api/borb/receive/{id}", web::get().to(borb_receive))
        // ===== FRAUDE =====
        .route(
            "/api/fraud/history/{id}",
            web::get()
                .guard(crate::owner_guard::owner_guard())
                .to(fraud_history),
        )
        .route(
            "/api/fraud/score/{id}",
            web::get()
                .guard(crate::owner_guard::owner_guard())
                .to(fraud_score),
        )
        // ===== MAILBOX =====
        .route("/api/mailbox/{id}", web::get().to(mailbox_get))
        // ===== RÉPUTATION =====
        .route("/api/reputation/{id}", web::get().to(reputation_get))
        // ===== INDEX =====
        .route("/serve", web::get().to(serve_index))
        // ===== BOUTIQUE (user_handlers) =====
        .route(
            "/api/shop/{id}",
            web::get().to(crate::user_handlers::shop_get),
        )
        .route(
            "/api/shop/{id}/update",
            web::post().to(crate::user_handlers::shop_update),
        )
        .route(
            "/api/shop/{id}/renew",
            web::post().to(crate::api_handlers::shop_renew),
        )
        // ===== CONNEXION (user_handlers) =====
        .route(
            "/api/connect/sell",
            web::post().to(crate::user_handlers::connect_sell),
        )
        .route(
            "/api/connect/buy",
            web::post().to(crate::user_handlers::connect_buy),
        )
        .route(
            "/api/connect/sellers",
            web::get().to(crate::user_handlers::connect_sellers),
        )
        // ===== REVENTE DE CONNEXION (connexion) =====
        // Fiche réelle : un nom, une photo, un type de lien, une position et une
        // mesure. Le tarif par seconde ne passe jamais par ici.
        .route(
            "/api/connexion/publish",
            web::post().to(crate::connexion::connexion_publish),
        )
        .route(
            "/api/connexion/unpublish",
            web::post().to(crate::connexion::connexion_unpublish),
        )
        .route(
            "/api/connexion/mine",
            web::get().to(crate::connexion::connexion_mine),
        )
        .route(
            "/api/connexion/nearby",
            web::get().to(crate::connexion::connexion_nearby),
        )
        .route(
            "/api/connexion/acheter",
            web::post().to(crate::demandes_handlers::connexion_acheter),
        )
        .route(
            "/api/connexion/achats",
            web::get().to(crate::demandes_handlers::connexion_achats),
        )
        .route(
            "/api/demandes",
            web::post().to(crate::demandes_handlers::demande_create),
        )
        .route(
            "/api/demandes",
            web::get().to(crate::demandes_handlers::demande_liste),
        )
        .route(
            "/api/liens-externe/soumettre",
            web::post().to(crate::demandes_handlers::lien_externe_soumettre),
        )
        .route(
            "/api/liens-externe/verifier",
            web::post().to(crate::demandes_handlers::lien_externe_verifier),
        )
        .route(
            "/api/liens-externe/verifier-profil",
            web::post().to(crate::demandes_handlers::lien_externe_verifier_profil),
        )
        .route(
            "/api/liens-externe/mes-liens",
            web::get().to(crate::demandes_handlers::lien_externe_mes),
        )
        // Une voix par compte et par fiche : le compteur d'avis se lit, il ne
        // se fabrique pas.
        .route(
            "/api/connexion/avis",
            web::post().to(crate::connexion::connexion_avis_publier),
        )
        .route(
            "/api/connexion/avis",
            web::get().to(crate::connexion::connexion_avis_liste),
        )
        // ===== PARRAINAGE (user_handlers) =====
        .route(
            "/api/referral/create",
            web::post().to(crate::user_handlers::referral_create),
        )
        .route(
            "/api/referral/stats/{id}",
            web::get().to(crate::user_handlers::referral_stats),
        )
        .route(
            "/api/referral/claim",
            web::post().to(crate::user_handlers::referral_claim),
        )
        // ===== NOTIFICATIONS (user_handlers) =====
        .route(
            "/api/notifications/{id}",
            web::get().to(crate::user_handlers::notifications_list),
        )
        .route(
            "/api/notifications/read",
            web::post().to(crate::user_handlers::notifications_read),
        )
        // ===== DASHBOARD (user_handlers) =====
        .route(
            "/api/dashboard/{id}",
            web::get().to(crate::user_handlers::dashboard),
        )
        // ====================
        // NOUVELLES ROUTES — TREASURY + COUNTER + CASES + INVOICES
        // ====================
        // ===== TREASURY (10 routes) =====
        .route(
            "/api/treasury/status",
            web::get().to(crate::treasury_handlers::treasury_status),
        )
        .route(
            "/api/treasury/sales",
            web::get().to(crate::treasury_handlers::treasury_sales),
        )
        .route(
            "/api/treasury/revenue",
            web::get().to(crate::treasury_handlers::treasury_revenue),
        )
        .route(
            "/api/treasury/system/profiles",
            web::get().to(crate::treasury_handlers::treasury_system_profiles),
        )
        .route(
            "/api/treasury/system/messages",
            web::get().to(crate::treasury_handlers::treasury_system_messages),
        )
        .route(
            "/api/treasury/dev/ads",
            web::get().to(crate::treasury_handlers::treasury_dev_ads),
        )
        .route(
            "/api/treasury/dev/ads/create",
            web::post().to(crate::treasury_handlers::treasury_dev_ads_create),
        )
        .route(
            "/api/treasury/dev/ads/delete",
            web::post().to(crate::treasury_handlers::treasury_dev_ads_delete),
        )
        .route(
            "/api/treasury/wallets",
            web::get().to(crate::treasury_handlers::treasury_wallets),
        )
        .route(
            "/api/treasury/wallets/add",
            web::post().to(crate::treasury_handlers::treasury_wallets_add),
        )
        // ===== COUNTER (4 routes) =====
        .route(
            "/api/counter/status",
            web::get().to(crate::counter_handlers::counter_status),
        )
        .route(
            "/api/counter/calculate",
            web::post().to(crate::counter_handlers::counter_calculate),
        )
        .route(
            "/api/counter/transactions",
            web::get().to(crate::counter_handlers::counter_transactions),
        )
        .route(
            "/api/counter/rights",
            web::get().to(crate::counter_handlers::counter_rights),
        )
        .route(
            "/api/counter/units",
            web::get().to(crate::counter_handlers::counter_units),
        )
        // ===== 7 CASES (4 routes) =====
        .route(
            "/api/cases/seven",
            web::get().to(crate::cases_handlers::cases_seven),
        )
        .route(
            "/api/cases/bridges",
            web::get().to(crate::cases_handlers::cases_bridges),
        )
        .route(
            "/api/cases/anomalies",
            web::get().to(crate::cases_handlers::cases_anomalies),
        )
        .route(
            "/api/cases/optimizations",
            web::get().to(crate::cases_handlers::cases_optimizations),
        )
        // ===== INVOICES (4 routes) =====
        .route(
            "/api/invoices",
            web::get().to(crate::invoice_handlers::invoices_list),
        )
        .route(
            "/api/invoices/{id}",
            web::get().to(crate::invoice_handlers::invoice_get),
        )
        .route(
            "/api/invoices/{id}/pdf",
            web::get().to(crate::invoice_handlers::invoice_pdf),
        )
        .route(
            "/api/invoices/by-user/{uq}",
            web::get().to(crate::invoice_handlers::invoices_by_user),
        )
        // ===== COEUR BLOCKORB =====
        .route(
            "/api/core/send",
            web::post().to(crate::core_handlers::core_send),
        )
        .route(
            "/api/core/balance/{user}",
            web::get().to(crate::core_handlers::core_balance),
        )
        .route(
            "/api/core/block-orb",
            web::get().to(crate::core_handlers::core_block_orb),
        )
        .route(
            "/api/core/prix",
            web::get().to(crate::core_handlers::core_prix),
        )
        .route(
            "/api/core/cases",
            web::get().to(crate::core_handlers::core_cases),
        )
        .route(
            "/api/core/invariant",
            web::get().to(crate::core_handlers::core_invariant),
        )
        // Réconciliation BlockOrb ↔ soldes SQLite (owner = global, user = son compte)
        .route(
            "/api/core/reconcile",
            web::post().to(crate::core_handlers::core_reconcile),
        )
        .route(
            "/api/core/migrate-accounts",
            web::post().to(crate::core_handlers::core_migrate_accounts),
        )
        // ===== UBI =====
        .route(
            "/api/core/ubi/claim",
            web::post().to(crate::core_handlers::core_ubi_claim),
        )
        .route(
            "/api/core/ubi/info",
            web::get().to(crate::core_handlers::core_ubi_info),
        )
        .route(
            "/api/core/init",
            web::post().to(crate::core_handlers::core_init),
        )
        .route(
            "/api/core/cadeaux/{user}",
            web::get().to(crate::core_handlers::core_cadeaux),
        )
        .route(
            "/api/core/parrainage",
            web::post().to(crate::core_handlers::core_parrainage),
        )
        // Le dossier KYC transporte photo + selfie + clip : la limite par defaut
        // d'Actix (1 Mo) rejetait deja les videos authorizees par l'ecran.
        .service(
            web::resource("/api/core/verify-identity")
                .app_data(web::PayloadConfig::new(16_000_000))
                .route(web::post().to(crate::core_handlers::core_verify_identity)),
        )
        // ===== KYC =====
        .route(
            "/api/kyc/queue",
            web::get().to(crate::core_handlers::core_kyc_queue),
        )
        .route(
            "/api/kyc/validate",
            web::post().to(crate::core_handlers::core_kyc_validate),
        )
        .route(
            "/api/kyc/statut/{user}",
            web::get().to(crate::core_handlers::core_kyc_statut),
        )
        // ===== 🌊 OCEAN + TRÉSOR 1M (Phase 1-2) =====
        // ===== 🌐 APIs EXTERNES (crypto / fiat / bourse) =====
        .route(
            "/api/external/status",
            web::get().to(crate::external_routes::external_status),
        )
        .route(
            "/api/external/crypto/price/{symbol}",
            web::get().to(crate::external_routes::crypto_price),
        )
        .route(
            "/api/external/crypto/klines/{symbol}",
            web::get().to(crate::external_routes::crypto_klines),
        )
        .route(
            "/api/external/fiat/rates",
            web::get().to(crate::external_routes::fiat_rates),
        )
        .route(
            "/api/external/stock/quote/{symbol}",
            web::get().to(crate::external_routes::stock_quote),
        )
        .route(
            "/api/external/market/tick/{symbol}",
            web::get().to(crate::external_routes::market_tick),
        )
        .route(
            "/api/external/market/ohlc/{symbol}",
            web::get().to(crate::external_routes::market_ohlc),
        )
        .route(
            "/api/external/gold/spot",
            web::get().to(crate::external_routes::gold_spot),
        )
        // ===== 📊 BINANCE P2P — carnet d'ordres public (lecture seule) =====
        .route(
            "/api/binance-p2p/offers",
            web::get().to(crate::binance_p2p::offres_binance_p2p_handler),
        )
        .route(
            "/api/binance-p2p/resume",
            web::get().to(crate::binance_p2p::resume_binance_p2p_handler),
        )
        .route(
            "/api/binance-p2p/cache",
            web::get().to(crate::binance_p2p::etat_cache_handler),
        )
        .route(
            "/api/owner/kyc/auto-validate",
            web::post().to(crate::history_routes::owner_kyc_auto_validate),
        )
        .route(
            "/api/ocean/status",
            web::get().to(crate::ocean_routes::ocean_status),
        )
        .route(
            "/api/ocean/invariant",
            web::get().to(crate::ocean_routes::ocean_invariant),
        )
        .route(
            "/api/ocean/history",
            web::get().to(crate::ocean_routes::ocean_history),
        )
        .route(
            "/api/ocean/recharge/{uq}",
            web::post().to(crate::ocean_routes::ocean_recharge),
        )
        // ===== 🔄 LES 6 CASES — auto-régulation + marché =====
        .route(
            "/api/cases/status",
            web::get().to(crate::cases_routes::cases_status),
        )
        .route(
            "/api/cases/health",
            web::get().to(crate::cases_routes::cases_health),
        )
        .route(
            "/api/cases/flow",
            web::get().to(crate::cases_routes::cases_flow),
        )
        .route(
            "/api/cases/market",
            web::get().to(crate::cases_routes::cases_market),
        )
        .route(
            "/api/cases/invariant",
            web::get().to(crate::cases_routes::cases_invariant),
        )
        .route(
            "/api/cases/history",
            web::get().to(crate::cases_routes::cases_history),
        )
        .route(
            "/api/cases/auto-regulate",
            web::post().to(crate::cases_routes::cases_auto_regulate),
        )
        .route(
            "/api/treasury/1m/status",
            web::get().to(crate::ocean_routes::tresor_1m_status),
        )
        .route(
            "/api/treasury/1m/recharge",
            web::post().to(crate::ocean_routes::tresor_1m_recharge),
        )
        // ====================
        // 📜 HISTORIQUES LOCAUX + 🤝 RÉCUPÉRATION PAR VOISINS
        // (spécification « BlockOrb à taille fixe + historiques locaux »)
        // ====================
        // Supervision du BlockOrb à taille fixe
        .route(
            "/api/history/blockorb/size",
            web::get().to(crate::history_routes::blockorb_size),
        )
        // Étape 1 — publier une demande de récupération
        .route(
            "/api/history/request-recovery",
            web::post().to(crate::history_routes::history_request_recovery),
        )
        // Étape 2 — un voisin cherche et fournit ses fragments
        .route(
            "/api/history/pending-requests",
            web::get().to(crate::history_routes::history_pending_requests),
        )
        .route(
            "/api/history/provide-fragment",
            web::post().to(crate::history_routes::history_provide_fragment),
        )
        // Étape 3 — reconstitution chez le demandeur
        .route(
            "/api/history/reconstruct",
            web::post().to(crate::history_routes::history_reconstruct),
        )
        .route(
            "/api/history/recovery/{request_id}",
            web::get().to(crate::history_routes::history_recovery_status),
        )
        // Étape 4 — validation par les voisins (3+)
        .route(
            "/api/history/validate-recovery",
            web::post().to(crate::history_routes::history_validate_recovery),
        )
        // Auto-suppression > 1 an
        .route(
            "/api/history/cleanup-old",
            web::post().to(crate::history_routes::history_cleanup_old),
        )
        // ORDRE CRITIQUE : les routes LITTERALES ci-dessus DOIVENT rester
        //    avant les routes dynamiques ci-dessous, sinon
        //    « pending-requests » serait interprete comme un UQ number
        //    (Actix retient la premiere route qui matche).
        .route(
            "/api/history/{uq}",
            web::get().to(crate::history_routes::history_status),
        )
        .route(
            "/api/history/{uq}/verify",
            web::post().to(crate::history_routes::history_verify),
        )
        .route(
            "/api/history/{uq}/chain",
            web::get().to(crate::history_routes::history_chain),
        );
}
