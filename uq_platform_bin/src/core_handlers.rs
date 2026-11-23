// ====================
// 🌊 CORE HANDLERS — Routes du cœur BlockOrb + UBI
// ====================
//
// « L'eau circule. Elle se calcule. »
//
// Ces handlers CALCULENT (pas de blocs) :
// - /api/core/send        → Transfert
// - /api/core/balance     → Solde
// - /api/core/block-orb   → État du BlockOrb
// - /api/core/prix        → Prix actuel
// - /api/core/cases       → État des 6 Cases
// - /api/core/invariant   → Vérification 40 Md
// - /api/core/ubi/claim   → Réclamation UBI
// - /api/core/ubi/info    → Info enveloppe UBI
// - /api/core/init        → Init compte de test
//
// ====================

use actix_web::{web, HttpResponse};
use rust_decimal::prelude::*;
use rust_decimal::Decimal;
use serde_json::json;
use std::str::FromStr;

use crate::block_orb::BlockOrb;
use crate::montant::{MontantBorb, CAPACITE_TOTALE};
use crate::persistence;
use crate::transaction_engine::{TransactionEngine, TxType};

// ====================
// CONSTANTES UBI
// ====================

/// Seuil minimum du Tresor (15% de 40 Md)
const SEUIL_TRESOR: u128 = 6_000_000_000_000_000_000_000_000_000;

/// Charge max de la blockchain (block_number)
const SEUIL_CHARGE: u64 = 10_000;

/// UBI initial en USD
const UBI_INITIAL_USD: f64 = 0.5;

// ====================
// PRIX DU RÉGULATEUR — porte commune
// ====================

/// Le prix BORB↔USD tel que le régulateur l'a réellement réglé.
///
/// Ces chemins écrivent en comptabilité (transfert, UBI, mint Trésor,
/// parrainage). Le `unwrap_or(Decimal::ONE)` qu'ils utilisaient affirmait
/// « 1 BORB = 1 USD » quand `prix.json` était illisible — et fabriquait donc de
/// la monnaie à partir d'un nombre absent. Sans prix réglé : 503, rien d'écrit.
fn prix_borb_regule() -> Result<Decimal, HttpResponse> {
    Decimal::from_str(&persistence::load_prix().prix_actuel)
        .ok()
        .filter(|p| *p > Decimal::ZERO)
        .ok_or_else(|| {
            HttpResponse::ServiceUnavailable().json(json!({
                "status": "error",
                "code": "PRIX_NON_REGLE",
                "message": "Le prix interne du BORB n'est pas réglé : aucun mouvement de valeur ne sera calculé sur 1 BORB = 1 USD inventé.",
                "reprise": "Régler le prix côté régulateur."
            }))
        })
}

// ====================
// POST /api/core/send
// ====================
pub async fn core_send(
    req: actix_web::HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let from = body.get("from").and_then(|v| v.as_str()).unwrap_or("");
    let to = body.get("to").and_then(|v| v.as_str()).unwrap_or("");
    let amount_usd = body
        .get("amount_usd")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    if from.is_empty() || to.is_empty() || amount_usd <= 0.0 {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Parametres manquants (from, to, amount_usd)"
        }));
    }

    // 🔐 Déplacement de valeur : l'appelant doit être le porteur (ou owner).
    if let Err(r) = crate::auth_guard::exiger_soi_ou_owner(&req, from) {
        return r;
    }

    // 🚪 PORTE BORB : le destinataire doit avoir un cadenas structurel réel.
    if let Err(e) = crate::kyc::cadenas::porte_cadenas_structurel(to) {
        return HttpResponse::Forbidden().json(json!({
            "status": "error",
            "code": "CADENAS_REQUIS",
            "message": format!("Destinataire refuse (cadenas structurel absent) : {}", e)
        }));
    }

    // 🚫 Section critique BlockOrb (load → mutate → save sous verrou global).
    let (mut orb, _garde) = persistence::orb_charge_verrouille();
    let prix_usd = match prix_borb_regule() {
        Ok(p) => p,
        Err(refus) => return refus,
    };

    let amount_dec = Decimal::from_f64_retain(amount_usd).unwrap_or(Decimal::ZERO);
    let montant = MontantBorb::from_usd_decimal(amount_dec, prix_usd);
    let result = TransactionEngine::calculer(amount_dec, prix_usd, TxType::Send);

    let invariant_avant = orb.verifier();

    match orb.traiter(from, to, montant) {
        Ok(_) => {
            let frais = result.frais_borb;
            let solde_from_apres = orb.get_solde(from);

            if solde_from_apres < frais {
                let solde_to_apres = orb.get_solde(to);
                let _ = orb.set_solde(from, solde_from_apres + montant.0);
                let _ = orb.set_solde(to, solde_to_apres - montant.0);

                return HttpResponse::BadRequest().json(json!({
                    "status": "error",
                    "message": "Solde insuffisant pour les frais"
                }));
            }

            if let Err(e) = orb.set_solde(from, solde_from_apres - frais) {
                return HttpResponse::InternalServerError().json(json!({
                    "status": "error",
                    "message": format!("Debit frais impossible: {}", e)
                }));
            }

            match orb.distribuer_frais(frais) {
                Ok(_) => {
                    let invariant_apres = orb.verifier();
                    let _ = persistence::save_global_orb(&orb);

                    HttpResponse::Ok().json(json!({
                        "status": "success",
                        "from": from,
                        "to": to,
                        "amount_usd": amount_usd,
                        "amount_borb": result.montant_borb.to_borb_decimal().to_string(),
                        "frais_usd": result.frais_usd.to_string(),
                        "frais_borb": frais.to_string(),
                        "invariant_avant": invariant_avant,
                        "invariant_apres": invariant_apres,
                        "block_number": orb.block_number
                    }))
                }
                Err(e) => HttpResponse::InternalServerError().json(json!({
                    "status": "error",
                    "message": format!("Erreur frais: {}", e)
                })),
            }
        }
        Err(e) => HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": e
        })),
    }
}

// ====================
// GET /api/core/balance/{user}
// ====================

pub async fn core_balance(req: actix_web::HttpRequest, path: web::Path<String>) -> HttpResponse {
    // 🔐 SÉCURITÉ (vague 1) — USER : accès à SES données (ou owner)
    let __cible = path.to_string();
    if let Err(r) = crate::auth_guard::exiger_soi_ou_owner(&req, &__cible) {
        return r;
    }

    let user = path.into_inner();
    let orb = persistence::load_global_orb();
    let solde = orb.get_solde(&user);
    let solde_cadeaux = orb.comptes_cadeaux.get(&user).copied().unwrap_or(0);

    HttpResponse::Ok().json(json!({
        "status": "success",
        "user": user,
        "balance_femto": solde.to_string(),
        "balance_borb": MontantBorb(solde).to_borb_decimal().to_string(),
        "balance_cadeaux_femto": solde_cadeaux.to_string(),
        "balance_cadeaux_borb": MontantBorb(solde_cadeaux).to_borb_decimal().to_string()
    }))
}

// ====================
// GET /api/core/block-orb
// ====================

pub async fn core_block_orb() -> HttpResponse {
    let orb = persistence::load_global_orb();

    HttpResponse::Ok().json(json!({
        "status": "success",
        "block_number": orb.block_number,
        "cases_count": orb.cases.len(),
        "comptes_count": orb.comptes.len(),
        "comptes_cadeaux_count": orb.comptes_cadeaux.len(),
        "total_femto": orb.total().to_string(),
        "invariant_ok": orb.verifier()
    }))
}

// ====================
// GET /api/core/prix
// ====================

pub async fn core_prix() -> HttpResponse {
    let state = persistence::load_prix();

    HttpResponse::Ok().json(json!({
        "status": "success",
        "prix_usd": state.prix_actuel,
        "demande": state.demande,
        "dernier_ajustement": state.dernier_ajustement
    }))
}

// ====================
// GET /api/core/cases
// ====================

pub async fn core_cases() -> HttpResponse {
    let orb = persistence::load_global_orb();

    let cases_json: Vec<_> = orb
        .cases
        .iter()
        .enumerate()
        .map(|(i, &val)| {
            let nom = match i {
                0 => "Ocean (Tresor)",
                1 => "Nuages (Frais)",
                2 => "Rivieres (Recompenses)",
                3 => "Glaciers (Epargne)",
                4 => "Nappes (Credit)",
                5 => "Vapeur (Bouclier)",
                _ => "Inconnu",
            };
            json!({
                "index": i,
                "nom": nom,
                "femto": val.to_string(),
                "borb": MontantBorb(val).to_borb_decimal().to_string()
            })
        })
        .collect();

    HttpResponse::Ok().json(json!({
        "status": "success",
        "cases": cases_json,
        "somme_cases_femto": orb.somme_cases().to_string(),
        "somme_comptes_femto": orb.somme_comptes().to_string(),
        "total_femto": orb.total().to_string()
    }))
}

// ====================
// GET /api/core/invariant
// ====================

pub async fn core_invariant() -> HttpResponse {
    let orb = persistence::load_global_orb();
    let ok = orb.verifier();

    HttpResponse::Ok().json(json!({
        "status": if ok { "success" } else { "error" },
        "invariant_ok": ok,
        "total_femto": orb.total().to_string(),
        "capacite_femto": CAPACITE_TOTALE.to_string(),
        "message": if ok { "Invariant respecte (40 Md BORB)" } else { "INVARIANT CASSE !" }
    }))
}

// ====================
// POST /api/core/reconcile
// Aligne le BlockOrb sur les soldes SQLite (balance.points)
//   - owner/admin : tous les comptes
//   - utilisateur : son propre compte uniquement
// ====================

pub async fn core_reconcile(req: actix_web::HttpRequest) -> HttpResponse {
    let claims =
        match crate::auth::AuthManager::require_role(&req, &["user", "admin", "guardian", "owner"])
        {
            Ok(c) => c,
            Err(resp) => return resp,
        };

    let est_admin = claims.role == "admin"
        || claims.role == "owner"
        || crate::owner_guard::check_is_owner(&claims.sub);

    let cible = if est_admin {
        None
    } else {
        Some(claims.sub.clone())
    };

    match persistence::reconcilier_blockorb(cible.as_deref()) {
        Ok(rapport) => {
            let appliques = rapport.iter().filter(|(_, _, ok)| *ok).count();

            let details: Vec<serde_json::Value> = rapport
                .iter()
                .map(|(uq, delta, ok)| {
                    json!({
                        "uq_number": uq,
                        "delta_femto": delta.to_string(),
                        "applique": ok
                    })
                })
                .collect();

            let orb = persistence::load_global_orb();

            HttpResponse::Ok().json(json!({
                "status": "success",
                "portee": if est_admin { "globale" } else { "compte appelant" },
                "comptes_modifies": appliques,
                "comptes_ignores": rapport.len() - appliques,
                "details": details,
                "invariant_ok": orb.verifier(),
                "total_femto": orb.total().to_string(),
                "message": "BlockOrb aligne sur les soldes SQLite"
            }))
        }
        Err(e) => HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": e
        })),
    }
}

// ====================
// POST /api/core/migrate-accounts  (OWNER)
// Applique les migrations aux comptes EXISTANTS :
//   colonnes de chaînage, tables v5, resynchronisation wallet /
//   my_borb_units, scellé d'historique.
// Aucun solde n'est modifié.
// ====================

pub async fn core_migrate_accounts(req: actix_web::HttpRequest) -> HttpResponse {
    let claims = match crate::auth::AuthManager::require_role(&req, &["admin", "guardian", "owner"])
    {
        Ok(c) => c,
        Err(resp) => return resp,
    };

    let est_owner = claims.sub == "UQ-00000001"
        || claims.role == "admin"
        || crate::owner_guard::check_is_owner(&claims.sub);

    if !est_owner {
        return HttpResponse::Forbidden().json(json!({
            "status": "error",
            "message": "Réservé au propriétaire"
        }));
    }

    let rapport = crate::migration_comptes::migrer_tous_les_comptes();
    let orb = persistence::load_global_orb();

    HttpResponse::Ok().json(json!({
        "status": "success",
        "comptes_traites": rapport.comptes_traites,
        "comptes_modifies": rapport.comptes_modifies,
        "total_erreurs": rapport.total_erreurs,
        "details": rapport.comptes,
        "invariant_ok": orb.verifier(),
        "message": "Migrations appliquees aux comptes existants (aucun solde modifie)"
    }))
}

// ====================
// POST /api/core/ubi/claim
// ====================

pub async fn core_ubi_claim(
    req: actix_web::HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let user = body.get("user").and_then(|v| v.as_str()).unwrap_or("");

    if user.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Parametre manquant : user"
        }));
    }

    // 🔐 Crédit depuis l'enveloppe UBI : réservé au porteur (ou owner).
    if let Err(r) = crate::auth_guard::exiger_soi_ou_owner(&req, user) {
        return r;
    }

    // 🚪 PORTE BORB : cadenas déverrouillé par la clé KYC ET cadenas
    //    structurel réel (identifiants personnels + hash conforme).
    if !crate::kyc::cadenas::est_deverrouille(user) {
        return HttpResponse::Forbidden().json(json!({
            "status": "error",
            "code": "UBI_CADENAS_REQUIRED",
            "message": "L'UBI est réservé aux comptes dont le cadenas (KYC) est vérifié."
        }));
    }
    if let Err(e) = crate::kyc::cadenas::porte_cadenas_structurel(user) {
        return HttpResponse::Forbidden().json(json!({
            "status": "error",
            "code": "UBI_CADENAS_STRUCTUREL_REQUIS",
            "message": format!("UBI refuse (cadenas structurel absent) : {}", e)
        }));
    }

    // 🚫 Section critique BlockOrb.
    let (mut orb, _garde) = persistence::orb_charge_verrouille();

    // 1. Verifier l'activite reelle
    if let Err(e) = verifier_activite_reelle(user) {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": e
        }));
    }

    // 2. Verifier l'enveloppe UBI
    if orb.ubi_enveloppe == 0 {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "UBI epuise definitivement."
        }));
    }

    // 3. Verifier le Tresor
    let tresor = orb.cases[0];
    if tresor < SEUIL_TRESOR {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Tresor trop bas (15%). UBI bloque."
        }));
    }

    // 4. Verifier la charge
    if orb.block_number > SEUIL_CHARGE {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Blockchain surchargee. UBI bloque."
        }));
    }

    // 5. Calculer le montant
    let prix_usd = match prix_borb_regule() {
        Ok(p) => p,
        Err(refus) => return refus,
    };

    let ubi_usd_dec = {
        let tresor_dec = Decimal::from_f64_retain(tresor as f64).unwrap_or(Decimal::ZERO);
        let capacite_dec = Decimal::from_f64_retain(CAPACITE_TOTALE as f64).unwrap_or(Decimal::ONE);
        let ratio = tresor_dec / capacite_dec;
        ratio * Decimal::from_f64_retain(UBI_INITIAL_USD).unwrap_or(Decimal::ZERO)
    };

    let montant = MontantBorb::from_usd_decimal(ubi_usd_dec, prix_usd);

    if montant.0 == 0 {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Montant UBI trop petit"
        }));
    }

    if montant.0 > orb.ubi_enveloppe {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Enveloppe UBI insuffisante"
        }));
    }

    // 6. Transfert atomique — via le porteur : le cadeau n'entre que dans un
    //    compte tracé ET cadenassé dans le BlockOrb.
    let solde_avant = orb.solde_cadeaux(user);
    orb.ubi_enveloppe -= montant.0;
    if let Err(e) = orb.crediter_cadeau(user, montant.0) {
        orb.ubi_enveloppe += montant.0;
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "code": "CADENAS_NON_LIE",
            "message": format!("UBI refuse par le BlockOrb : {}", e)
        }));
    }

    // 8. Sauvegarder
    if let Err(e) = persistence::save_global_orb(&orb) {
        return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": format!("Erreur sauvegarde: {}", e)
        }));
    }

    HttpResponse::Ok().json(json!({
        "status": "success",
        "user": user,
        "ubi_usd": ubi_usd_dec.to_string(),
        "ubi_borb": montant.to_borb_decimal().to_string(),
        "solde_cadeaux_avant": solde_avant.to_string(),
        "solde_cadeaux_apres": (solde_avant + montant.0).to_string(),
        "enveloppe_restante": orb.ubi_enveloppe.to_string(),
        "message": "UBI reclame avec succes"
    }))
}

// ====================
// GET /api/core/ubi/info
// ====================

pub async fn core_ubi_info() -> HttpResponse {
    let orb = persistence::load_global_orb();
    let tresor = orb.cases[0];

    let enveloppe_initiale = 10_000_000u128 * crate::montant::FEMTO_PAR_BORB;
    let consomme = enveloppe_initiale.saturating_sub(orb.ubi_enveloppe);

    HttpResponse::Ok().json(json!({
        "status": "success",
        "enveloppe_initiale": enveloppe_initiale.to_string(),
        "enveloppe_restante": orb.ubi_enveloppe.to_string(),
        "consomme": consomme.to_string(),
        "epuise": orb.ubi_enveloppe == 0,
        "tresor_actuel": tresor.to_string(),
        "seuil_tresor": SEUIL_TRESOR.to_string(),
        "tresor_ok": tresor >= SEUIL_TRESOR,
        "charge_actuelle": orb.block_number,
        "seuil_charge": SEUIL_CHARGE,
        "charge_ok": orb.block_number <= SEUIL_CHARGE,
        "ubi_initial_usd": UBI_INITIAL_USD
    }))
}

// ====================
// POST /api/core/init
// ====================

pub async fn core_init(
    req: actix_web::HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let user = body.get("user").and_then(|v| v.as_str()).unwrap_or("");
    let amount_usd = body
        .get("amount_usd")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    if user.is_empty() || amount_usd <= 0.0 {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Parametres manquants"
        }));
    }

    // 🔐 Mintage depuis le Trésor : réservé à l'owner.
    if let Err(r) = crate::auth_guard::exiger_owner(&req) {
        return r;
    }

    // 🚪 PORTE BORB : un mint du Trésor n'entre que dans un wallet cadenassé
    //    pour de vrai (identifiants personnels + hash conforme).
    if let Err(e) = crate::kyc::cadenas::porte_cadenas_structurel(user) {
        return HttpResponse::Forbidden().json(json!({
            "status": "error",
            "code": "CADENAS_REQUIS",
            "message": format!("Compte {} refuse au mint : {}", user, e)
        }));
    }

    // 🚫 Section critique BlockOrb.
    let (mut orb, _garde) = persistence::orb_charge_verrouille();

    if !orb.porte_cadenas(user) {
        return HttpResponse::NotFound().json(json!({
            "status": "error",
            "code": "CADENAS_NON_LIE",
            "message": format!("Compte {} absent du BlockOrb ou cadenas non lie — mint refuse", user)
        }));
    }
    let prix_usd = match prix_borb_regule() {
        Ok(p) => p,
        Err(refus) => return refus,
    };

    let amount_dec = Decimal::from_f64_retain(amount_usd).unwrap_or(Decimal::ZERO);
    let montant = MontantBorb::from_usd_decimal(amount_dec, prix_usd);

    if orb.cases[0] < montant.0 {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Tresor insuffisant"
        }));
    }

    orb.cases[0] -= montant.0;
    let solde_avant = orb.get_solde(user);
    let _ = orb.set_solde(user, solde_avant + montant.0);

    if !orb.verifier() {
        orb.cases[0] += montant.0;
        let _ = orb.set_solde(user, solde_avant);
        return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": "Invariant casse"
        }));
    }

    let _ = persistence::save_global_orb(&orb);

    HttpResponse::Ok().json(json!({
        "status": "success",
        "user": user,
        "amount_usd": amount_usd,
        "solde_apres": (solde_avant + montant.0).to_string(),
        "tresor_apres": orb.cases[0].to_string()
    }))
}

// ====================
// FONCTION : verifier_activite_reelle
// ====================

fn verifier_activite_reelle(_user: &str) -> Result<(), String> {
    // TODO: Connecter a uq_index.db
    Ok(())
}

// ====================
// FONCTION : charger_infos_utilisateur
// ====================

/// Charge les infos personnelles depuis la DB utilisateur
pub fn charger_infos_utilisateur(db_path: &str) -> (String, String, String, String) {
    use rusqlite::Connection;

    if !std::path::Path::new(db_path).exists() {
        return (
            String::new(),
            String::new(),
            String::new(),
            "RDC".to_string(),
        );
    }

    let conn = match Connection::open(db_path) {
        Ok(c) => c,
        Err(_) => {
            return (
                String::new(),
                String::new(),
                String::new(),
                "RDC".to_string(),
            )
        }
    };

    let mut nom = String::new();
    let mut prenom = String::new();
    let mut date_naissance = String::new();
    let mut pays = "RDC".to_string();

    if let Ok(mut stmt) = conn.prepare("SELECT key, value FROM settings WHERE key IN ('nom', 'prenom', 'date_naissance','pays', 'full_name')") {
        if let Ok(rows) = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        }) {
            for row in rows.flatten() {
                match row.0.as_str() {
                    "nom" => nom = row.1,
                    "prenom" => prenom = row.1,
                    "date_naissance" => date_naissance = row.1,
                    "pays" => pays = row.1,
                    "full_name" => {
                        let parts: Vec<&str> = row.1.split_whitespace().collect();
                        if parts.len() >= 2 {
                            prenom = parts[0].to_string();
                            nom = parts[1..].join(" ");
                        } else if parts.len() == 1 {
                            prenom = parts[0].to_string();
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    (nom, prenom, date_naissance, pays)
}

// ====================
// GET /api/core/cadeaux/{user}
// ====================
//
// Retourne le solde CADEAUX d'un utilisateur
//
// ====================

pub async fn core_cadeaux(req: actix_web::HttpRequest, path: web::Path<String>) -> HttpResponse {
    // 🔐 SÉCURITÉ (vague 1) — USER : accès à SES données (ou owner)
    let __cible = path.to_string();
    if let Err(r) = crate::auth_guard::exiger_soi_ou_owner(&req, &__cible) {
        return r;
    }

    let user = path.into_inner();
    let orb = persistence::load_global_orb();

    let solde = orb.comptes_cadeaux.get(&user).copied().unwrap_or(0);

    HttpResponse::Ok().json(json!({
        "status": "success",
        "user": user,
        "balance_cadeaux_femto": solde.to_string(),
        "balance_cadeaux_borb": MontantBorb(solde).to_borb_decimal().to_string()
    }))
}

// ====================
// POST /api/core/parrainage
// ====================
//
// Enregistre un parrainage :
// - Le parrain recoit 1 USD en BORB (solde CADEAUX)
// - Le filleul est enregistre
//
// Body :
// {
//   "parrain": "UQ-00000104",
//   "filleul": "UQ-00000105",
//   "code_promo": "UQ-DESTIN-1234"
// }
//
// ====================

pub async fn core_parrainage(
    req: actix_web::HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let parrain = body.get("parrain").and_then(|v| v.as_str()).unwrap_or("");
    let filleul = body.get("filleul").and_then(|v| v.as_str()).unwrap_or("");
    let code_promo = body
        .get("code_promo")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if parrain.is_empty() || filleul.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Parametres manquants (parrain, filleul)"
        }));
    }

    if parrain == filleul {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Le parrain et le filleul ne peuvent pas etre identiques"
        }));
    }

    // 🔐 Crédit Trésor/enveloppe : réservé à l'owner (le flow d'inscription passe par le backend).
    if let Err(r) = crate::auth_guard::exiger_owner(&req) {
        return r;
    }

    // 🚫 Section critique BlockOrb.
    let (mut orb, _garde) = persistence::orb_charge_verrouille();

    // 1. Verifier que le parrain existe
    if !orb.comptes.contains_key(parrain) && !orb.comptes_cadeaux.contains_key(parrain) {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Parrain introuvable"
        }));
    }

    // 2. Verifier que le filleul existe
    if !orb.comptes.contains_key(filleul) && !orb.comptes_cadeaux.contains_key(filleul) {
        // Accepter quand meme (le filleul vient d'etre cree)
    }

    // 3. Montant : 1 USD en BORB
    let prix_usd = match prix_borb_regule() {
        Ok(p) => p,
        Err(refus) => return refus,
    };
    let montant_usd = Decimal::from_f64_retain(1.0).unwrap_or(Decimal::ZERO);
    let montant = MontantBorb::from_usd_decimal(montant_usd, prix_usd);

    // 4. Verifier l'enveloppe (ou le Tresor)
    let deposite_dans_enveloppe = orb.ubi_enveloppe >= montant.0;
    if !deposite_dans_enveloppe {
        // Prendre du Tresor
        if orb.cases[0] < montant.0 {
            return HttpResponse::BadRequest().json(json!({
                "status": "error",
                "message": "Tresor insuffisant pour le parrainage"
            }));
        }
        orb.cases[0] -= montant.0;
    } else {
        orb.ubi_enveloppe -= montant.0;
    }

    // 5. Crediter le parrain (solde CADEAUX) — via le porteur : cadeau
    //    refuse si le parrain n'est pas trace avec un cadenas structurel.
    let solde_avant = orb.comptes_cadeaux.get(parrain).copied().unwrap_or(0);
    if let Err(e) = orb.crediter_cadeau(parrain, montant.0) {
        // Rembourse la source (enveloppe ou Tresor) precedemment debitée
        if deposite_dans_enveloppe {
            orb.ubi_enveloppe += montant.0;
        } else {
            orb.cases[0] += montant.0;
        }
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "code": "CADENAS_REQUIS",
            "message": format!("Parrainage refuse : {}", e)
        }));
    }

    // 6. Verifier invariant
    if !orb.verifier() {
        orb.comptes_cadeaux.insert(parrain.to_string(), solde_avant);
        return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "message": "INVARIANT CASSE — parrainage annule"
        }));
    }

    // 7. Sauvegarder
    let _ = persistence::save_global_orb(&orb);

    HttpResponse::Ok().json(json!({
        "status": "success",
        "parrain": parrain,
        "filleul": filleul,
        "code_promo": code_promo,
        "montant_usd": "1.0",
        "montant_borb": montant.to_borb_decimal().to_string(),
        "solde_cadeaux_apres": (solde_avant + montant.0).to_string(),
        "message": "Parrainage enregistre — le parrain a recu 1 USD"
    }))
}

/// Une image du dossier doit etre decodee PUIS ecrite. Avaler l'echec laisserait
/// un dossier « soumis » sans piece — inutilisable par l'examinateur.
fn enregistrer_image(
    b64: Option<&str>,
    ecrire: impl FnOnce(&[u8]) -> Result<String, String>,
) -> Result<Option<String>, (&'static str, String)> {
    let Some(b64) = b64.filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    use base64::Engine;
    let octets = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| ("base64_illisible", e.to_string()))?;
    ecrire(&octets).map(Some).map_err(|e| ("disque", e))
}

fn rejet_image(champ: &str, cause: &str, message: &str) -> HttpResponse {
    let corps = json!({
        "status": "error",
        "code": cause,
        "message": format!("{} non enregistree : {}", champ, message)
    });
    if cause == "disque" {
        HttpResponse::InternalServerError().json(corps)
    } else {
        HttpResponse::BadRequest().json(corps)
    }
}

// ====================
// POST /api/core/verify-identity
// ====================
//
// Soumet une verification d'identite :
// - Enregistre dans la file d'attente KYC
// - Statut : EN_ATTENTE
// - Le proprietaire valide plus tard
//
// Body :
// {
//   "uq_number": "UQ-00000104",
//   "nom": "MUKENDI",
//   "prenom": "Destin",
//   "date_naissance": "1990-03-15",
//   "pays": "CD",
//   "adresse": "...",
//   "telephone": "+243812345678",
//   "document_number": "123456789",
//   "photo_base64": "...",
//   "video_base64": "..." (optionnel)
// }
//
// ====================

pub async fn core_verify_identity(
    req: actix_web::HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    use crate::kyc::dossier::{KycDossier, KycStatus};
    use crate::kyc::queue::KycQueue;
    use crate::kyc::storage::KycStore;

    let uq_number = body.get("uq_number").and_then(|v| v.as_str()).unwrap_or("");

    if uq_number.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Parametre manquant : uq_number"
        }));
    }

    if let Err(refus) = crate::auth_guard::exiger_soi_ou_owner(&req, uq_number) {
        return refus;
    }

    let pays = body
        .get("pays")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let numero_document = body
        .get("document_number")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let photo = body
        .get("photo_base64")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let selfie = body
        .get("selfie_base64")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let session = body
        .get("session_vivacite")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if pays.is_empty() || numero_document.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error", "code": "IDENTITE_INCOMPLETE",
            "message": "Le pays et le numéro du document sont obligatoires"
        }));
    }
    if photo.is_empty() || selfie.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error", "code": "CAPTURES_REQUISES",
            "message": "La photo du document et le selfie sont obligatoires"
        }));
    }
    let vivacite = match crate::kyc_live::valider_session(session, uq_number) {
        Some(bilan) => bilan,
        None => {
            return HttpResponse::BadRequest().json(json!({
                "status": "error", "code": "VIVACITE_REQUISE",
                "message": "Une vivacité confirmée pour ce compte est obligatoire"
            }))
        }
    };

    let mut queue = KycStore::load_queue();

    // 2. Creer le dossier
    let mut dossier = KycDossier::new(uq_number.to_string());
    dossier.nom = body
        .get("nom")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    dossier.prenom = body
        .get("prenom")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    dossier.date_naissance = body
        .get("date_naissance")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    dossier.pays = pays.to_string();
    dossier.adresse = body
        .get("adresse")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    dossier.telephone = body
        .get("telephone")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    dossier.document_number = numero_document.to_string();

    if queue.document_deja_utilise(pays, numero_document, uq_number) {
        return HttpResponse::Conflict().json(json!({
            "status": "error", "code": "DOCUMENT_DEJA_UTILISE",
            "message": "Ce document est déjà associé à un autre dossier KYC"
        }));
    }

    // 3. Piece d'identite
    match enregistrer_image(
        body.get("photo_base64").and_then(|v| v.as_str()),
        |octets| KycStore::save_photo(uq_number, octets),
    ) {
        Ok(chemin) => dossier.photo_identite_path = chemin,
        Err((cause, message)) => return rejet_image("Photo", cause, &message),
    }

    // 4. Video de vivacite (parcours historique)
    match enregistrer_image(
        body.get("video_base64").and_then(|v| v.as_str()),
        |octets| KycStore::save_video(uq_number, octets),
    ) {
        Ok(chemin) => dossier.video_selfie_path = chemin,
        Err((cause, message)) => return rejet_image("Video", cause, &message),
    }

    // 4bis. Selfie guide de l'assistant + bilan de vivacite deja verifie par le
    // moteur (l'en-tete est separe : le nonce ne vient jamais de l'appelant).
    match enregistrer_image(
        body.get("selfie_base64").and_then(|v| v.as_str()),
        |octets| KycStore::save_selfie(uq_number, octets),
    ) {
        Ok(chemin) => dossier.selfie_path = chemin,
        Err((cause, message)) => return rejet_image("Selfie", cause, &message),
    }
    dossier.vivacite = Some(vivacite);

    // 5. Metriques IA avant la mise en file : le dossier enregistre doit etre
    // complet, la version en file etait jusque-la vidée de ses mesures.
    use crate::kyc::ia::KycIa;
    use crate::kyc::metrics_calc::calculer_metriques;

    let metrics = calculer_metriques(&[], None);
    let ia = KycIa::new();
    let suggestion = ia.proposer(&metrics);

    dossier.ai_metrics = metrics;
    dossier.ai_suggestion = Some(format!("{:?}", suggestion.decision));
    dossier.ai_confidence = Some(suggestion.confiance);
    queue.ajouter(dossier.clone());

    // 6. Sauvegarder : un dossier qui n'est pas écrit n'est pas soumis.
    if let Err(e) = KycStore::save_queue(&queue) {
        return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "code": "disque",
            "message": format!("Dossier non enregistre : {}", e)
        }));
    }

    HttpResponse::Ok().json(json!({
        "status": "success",
        "uq_number": uq_number,
        "statut": "EN_ATTENTE",
        "message": "Dossier soumis — en attente de validation",
        "position_file": queue.count_en_attente(),
        "ia": {
            "decision": format!("{:?}", suggestion.decision),
            "confiance": suggestion.confiance,
            "corrections": suggestion.corrections
        }
    }))
}

// ====================
// GET /api/kyc/queue
// ====================
//
// Liste les dossiers KYC en attente
//
// ====================

pub async fn core_kyc_queue(req: actix_web::HttpRequest) -> HttpResponse {
    // 🔐 SÉCURITÉ (vague 1) — OWNER uniquement
    if let Err(r) = crate::auth_guard::exiger_owner(&req) {
        return r;
    }

    use crate::kyc::dossier::KycStatus;
    use crate::kyc::storage::KycStore;

    let queue = KycStore::load_queue();

    let en_attente: Vec<_> = queue
        .dossiers
        .iter()
        .filter(|d| d.statut == KycStatus::EnAttente)
        .map(|d| {
            serde_json::json!({
                "uq_number": d.uq_number,
                "nom": d.nom,
                "prenom": d.prenom,
                "date_naissance": d.date_naissance,
                "pays": d.pays,
                "adresse": d.adresse,
                "telephone": d.telephone,
                "document_number": d.document_number,
                "photo_path": d.photo_identite_path,
                "video_path": d.video_selfie_path,
                "selfie_path": d.selfie_path,
                "vivacite": d.vivacite,
                "statut": "EN_ATTENTE",
                "soumis_at": d.soumis_at
            })
        })
        .collect();

    HttpResponse::Ok().json(json!({
        "status": "success",
        "total": en_attente.len(),
        "dossiers": en_attente
    }))
}

// ====================
// POST /api/kyc/validate
// ====================
//
// Valide ou refuse un dossier KYC
//
// Body :
// {
//   "uq_number": "UQ-00000104",
//   "decision": "VALIDER" ou "REFUSER",
//   "raison": "photo_illisible" (si refuse),
//   "cases_cochees": ["nom_correct", "photo_lisible", ...]
// }
//
// ====================

/// Applique la décision KYC sur le compte : déverrouillage si valide,
/// puis PURGE des données biométriques (photo/vidéo) — Render ne
/// conserve aucune trace du dossier après décision.
fn appliquer_decision_kyc(uq_number: &str, valide: bool) {
    let nom_db = uq_number.replace('-', "_").to_lowercase();
    let db_path = format!("data/users/{}.db", nom_db);
    if let Ok(conn) = rusqlite::Connection::open(&db_path) {
        let now = chrono::Utc::now().timestamp();
        if valide {
            let _ = conn.execute(
                "UPDATE identities SET is_verified = 1, updated_at = ?1 WHERE uq_number = ?2",
                rusqlite::params![now, uq_number],
            );
            let _ = conn.execute(
                "UPDATE my_profile SET kyc_status = 'verified' WHERE id = 1",
                [],
            );
            let _ = conn.execute(
                "UPDATE cadenas_verification SET est_verifie = 1, verrouille = 0, verifie_at = ?1 WHERE id = 1",
                rusqlite::params![now],
            );
        } else {
            let _ = conn.execute(
                "UPDATE my_profile SET kyc_status = 'rejected' WHERE id = 1",
                [],
            );
        }
    }
    // Purge biométrique : photo + vidéo + dossier détruits dans les
    // deux cas — le KYC ne garde rien sur Render après décision.
    let _ = crate::kyc::storage::KycStore::supprimer_dossier(uq_number);
}

pub async fn core_kyc_validate(
    req: actix_web::HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    // 🔐 SÉCURITÉ (vague 1) — OWNER uniquement
    if let Err(r) = crate::auth_guard::exiger_owner(&req) {
        return r;
    }

    use crate::kyc::queue::KycQueue;
    use crate::kyc::storage::KycStore;

    let uq_number = body.get("uq_number").and_then(|v| v.as_str()).unwrap_or("");
    let decision = body.get("decision").and_then(|v| v.as_str()).unwrap_or("");
    let raison = body.get("raison").and_then(|v| v.as_str()).unwrap_or("");

    if uq_number.is_empty() || decision.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Parametres manquants (uq_number, decision)"
        }));
    }

    let mut queue = KycStore::load_queue();

    match decision {
        "VALIDER" => match queue.valider(uq_number, "proprietaire") {
            Ok(_) => match KycStore::save_queue(&queue) {
                Ok(_) => {
                    // Déverrouillage + purge KYC : Render ne garde AUCUNE
                    // donnée biométrique — la décision valide libère le
                    // compte puis les fichiers photo/vidéo sont détruits.
                    appliquer_decision_kyc(uq_number, true);
                    HttpResponse::Ok().json(json!({
                        "status": "success",
                        "uq_number": uq_number,
                        "decision": "VALIDE",
                        "message": "Identite verifiee et compte deverrouille"
                    }))
                }
                Err(e) => HttpResponse::InternalServerError().json(json!({
                    "status": "error",
                    "code": "disque",
                    "message": format!("Validation non enregistree : {}", e)
                })),
            },
            Err(e) => HttpResponse::BadRequest().json(json!({
                "status": "error",
                "message": e
            })),
        },
        "REFUSER" => match queue.refuser(uq_number, raison, "proprietaire") {
            Ok(_) => match KycStore::save_queue(&queue) {
                Ok(_) => {
                    // Refus : purge aussi les données biométriques.
                    appliquer_decision_kyc(uq_number, false);
                    HttpResponse::Ok().json(json!({
                        "status": "success",
                        "uq_number": uq_number,
                        "decision": "REFUSE",
                        "raison": raison,
                        "message": "Identite refusee"
                    }))
                }
                Err(e) => HttpResponse::InternalServerError().json(json!({
                    "status": "error",
                    "code": "disque",
                    "message": format!("Refus non enregistre : {}", e)
                })),
            },
            Err(e) => HttpResponse::BadRequest().json(json!({
                "status": "error",
                "message": e
            })),
        },
        _ => HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": "Decision invalide (VALIDER ou REFUSER)"
        })),
    }
}

// ====================
// GET /api/kyc/statut/{user}
// ====================
//
// Retourne le statut KYC d'un utilisateur
//
// ====================

pub async fn core_kyc_statut(req: actix_web::HttpRequest, path: web::Path<String>) -> HttpResponse {
    // 🔐 SÉCURITÉ (vague 1) — USER : accès à SES données (ou owner)
    let __cible = path.to_string();
    if let Err(r) = crate::auth_guard::exiger_soi_ou_owner(&req, &__cible) {
        return r;
    }

    use crate::kyc::storage::KycStore;

    let user = path.into_inner();
    let queue = KycStore::load_queue();

    let dossier = queue.dossiers.iter().find(|d| d.uq_number == user);

    match dossier {
        Some(d) => HttpResponse::Ok().json(json!({
            "status": "success",
            "uq_number": user,
            "statut": format!("{:?}", d.statut),
            "soumis_at": d.soumis_at,
            "decision_at": d.decision_at
        })),
        None => HttpResponse::Ok().json(json!({
            "status": "success",
            "uq_number": user,
            "statut": "AUCUN",
            "message": "Aucun dossier KYC"
        })),
    }
}
