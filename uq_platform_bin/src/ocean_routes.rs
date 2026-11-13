// ====================
// 🌊 OCEAN ROUTES — API de la case Trésor
// ====================
//
//   GET  /api/ocean/status              PUBLIC (lecture du Trésor)
//   GET  /api/ocean/invariant           PUBLIC (contrôle 40 Md)
//   GET  /api/ocean/history             OWNER  (mouvements)
//   POST /api/ocean/recharge/{uq}       OWNER  (recharge depuis Ocean)
//   POST /api/treasury/1m/recharge      OWNER  (alias : recharge Profil 1)
//   GET  /api/treasury/1m/status        USER   (Trésor 1M en lecture)
// ====================

use actix_web::{web, HttpRequest, HttpResponse};
use serde_json::json;

/// GET /api/ocean/status — état de la case Ocean + Trésor du Profil 1
pub async fn ocean_status() -> HttpResponse {
    let etat = crate::ocean::etat();
    HttpResponse::Ok().json(json!({
        "status": "success",
        "ocean": {
            "femto": etat.ocean_femto,
            "borb": etat.ocean_borb,
            "part_du_total_pct": etat.ocean_part_pct,
            "role": "Reserve principale — source et retour du Tresor 1M"
        },
        "profil1": {
            "uq": crate::ocean::PROFIL_1,
            "borb": etat.profil1_borb,
            "points": etat.profil1_points,
            "tresor_cible_borb": etat.tresor_cible_borb,
            "seuil_recharge_borb": etat.seuil_recharge_borb,
            "recharge_necessaire": etat.recharge_necessaire,
            "manque_pour_cible_borb": etat.manque_pour_cible_borb
        },
        "users_femto": etat.users_femto,
        "invariant": {
            "ok": etat.invariant_ok,
            "total_femto": etat.total_femto,
            "capacite_femto": etat.capacite_femto,
            "formule": "Ocean + 5 cases + enveloppe UBI + Profil 1 + Users = 40 Md BORB"
        }
    }))
}

/// GET /api/ocean/invariant — contrôle strict de la masse
pub async fn ocean_invariant() -> HttpResponse {
    let orb = crate::persistence::load_global_orb();
    let total = orb.total();
    let capacite = crate::montant::CAPACITE_TOTALE;

    HttpResponse::Ok().json(json!({
        "status": if total == capacite { "success" } else { "error" },
        "invariant_ok": total == capacite,
        "ocean_femto": orb.cases[0].to_string(),
        "profil1_femto": orb.comptes.get(crate::ocean::PROFIL_1).map(|p| p.solde).unwrap_or(0).to_string(),
        "users_femto": orb.somme_comptes().to_string(),
        "autres_cases_femto": (orb.somme_cases() - orb.cases[0]).to_string(),
        "ubi_enveloppe_femto": orb.ubi_enveloppe.to_string(),
        "total_femto": total.to_string(),
        "capacite_femto": capacite.to_string(),
        "ecart_femto": if total > capacite { (total - capacite).to_string() } else { (capacite - total).to_string() }
    }))
}

/// GET /api/ocean/history — mouvements Ocean
pub async fn ocean_history(req: HttpRequest) -> HttpResponse {
    if let Err(r) = crate::auth_guard::exiger_owner(&req) {
        return r;
    }
    let h = crate::ocean::historique();
    HttpResponse::Ok().json(json!({
        "status": "success",
        "total": h.len(),
        "mouvements": h
    }))
}

/// POST /api/ocean/recharge/{uq} — recharge le Trésor depuis Ocean
/// `uq` doit être le Profil 1 (les autres comptes ne puisent pas dans Ocean).
pub async fn ocean_recharge(req: HttpRequest, path: web::Path<String>) -> HttpResponse {
    if let Err(r) = crate::auth_guard::exiger_owner(&req) {
        return r;
    }

    let uq = path.into_inner();
    if !uq.eq_ignore_ascii_case(crate::ocean::PROFIL_1) {
        return HttpResponse::Forbidden().json(json!({
            "status": "error",
            "code": "RESERVE_PROFIL1",
            "message": format!("Le Tresor 1M est reserve au Profil 1 ({})", crate::ocean::PROFIL_1)
        }));
    }

    // `force=true` pour recharger même au-dessus du seuil
    let forcer = req
        .query_string()
        .contains("force=true");

    match crate::ocean::recharger_profil1(forcer) {
        Ok(r) => HttpResponse::Ok().json(json!({
            "status": "success",
            "applique": r.applique,
            "raison": r.raison,
            "ocean_avant_borb": r.ocean_avant_borb,
            "ocean_apres_borb": r.ocean_apres_borb,
            "profil1_avant_borb": r.profil1_avant_borb,
            "profil1_apres_borb": r.profil1_apres_borb,
            "montant_recu_borb": r.montant_recu_borb,
            "retour_ocean_borb": r.retour_ocean_borb,
            "invariant_ok": r.invariant_ok
        })),
        Err(e) => HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": e
        })),
    }
}

/// GET /api/treasury/1m/status — lecture du Trésor 1M (tout utilisateur connecté)
pub async fn tresor_1m_status(req: HttpRequest) -> HttpResponse {
    if let Err(r) = crate::auth_guard::exiger_auth(&req) {
        return r;
    }
    let etat = crate::ocean::etat();
    HttpResponse::Ok().json(json!({
        "status": "success",
        "tresor_1m": {
            "profil": crate::ocean::PROFIL_1,
            "solde_borb": etat.profil1_borb,
            "solde_points": etat.profil1_points,
            "cible_borb": etat.tresor_cible_borb,
            "seuil_recharge_borb": etat.seuil_recharge_borb,
            "recharge_necessaire": etat.recharge_necessaire,
            "source": "CASE OCEAN",
            "transferable": false,
            "note": "Le solde du Profil 1 n'est pas transferable : il sort uniquement via le carnet d'ordres P2P"
        },
        "ocean_borb": etat.ocean_borb,
        "invariant_ok": etat.invariant_ok
    }))
}

/// POST /api/treasury/1m/recharge — alias owner de la recharge Ocean
pub async fn tresor_1m_recharge(req: HttpRequest) -> HttpResponse {
    if let Err(r) = crate::auth_guard::exiger_owner(&req) {
        return r;
    }
    let forcer = req.query_string().contains("force=true");
    match crate::ocean::recharger_profil1(forcer) {
        Ok(r) => HttpResponse::Ok().json(json!({
            "status": "success",
            "applique": r.applique,
            "raison": r.raison,
            "montant_recu_borb": r.montant_recu_borb,
            "retour_ocean_borb": r.retour_ocean_borb,
            "profil1_apres_borb": r.profil1_apres_borb,
            "ocean_apres_borb": r.ocean_apres_borb,
            "invariant_ok": r.invariant_ok
        })),
        Err(e) => HttpResponse::BadRequest().json(json!({"status": "error", "message": e})),
    }
}
