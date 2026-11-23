// ====================
// 🔄 CASES ROUTES — API des 6 cases (régulation + marché)
// ====================
//
//   GET  /api/cases/status          PUBLIC  (état des 6 cases)
//   GET  /api/cases/health          PUBLIC  (stabilité)
//   GET  /api/cases/flow            PUBLIC  (cycle interne)
//   GET  /api/cases/market          PUBLIC  (offre / demande)
//   GET  /api/cases/invariant       PUBLIC  (contrôle 40 Md)
//   GET  /api/cases/history         OWNER   (mouvements)
//   POST /api/cases/auto-regulate   OWNER   (déclenchement manuel)
// ====================

use actix_web::{HttpRequest, HttpResponse};
use serde_json::json;

/// GET /api/cases/status
pub async fn cases_status() -> HttpResponse {
    let e = crate::cases_loop::etat();
    HttpResponse::Ok().json(json!({
        "status": "success",
        "cases": e.cases,
        "invariant_ok": e.invariant_ok,
        "total_femto": e.total_femto,
        "capacite_femto": e.capacite_femto,
        "cases_sous_seuil": e.cases_sous_seuil,
        "sante": e.sante
    }))
}

/// GET /api/cases/health
pub async fn cases_health() -> HttpResponse {
    let e = crate::cases_loop::etat();
    let stables = e.cases.iter().filter(|c| !c.sous_seuil).count();

    HttpResponse::Ok().json(json!({
        "status": if e.invariant_ok { "success" } else { "error" },
        "sante": e.sante,
        "cases_stables": stables,
        "cases_total": e.cases.len(),
        "cases_sous_seuil": e.cases_sous_seuil,
        "invariant_ok": e.invariant_ok,
        "objectifs": {
            "aucune_case_vide": e.cases.iter().all(|c| c.borb > 0.0),
            "aucune_case_bloquee": stables == e.cases.len(),
            "circulation_permanente": true,
            "invariant_40_md": e.invariant_ok
        }
    }))
}

/// GET /api/cases/flow — le cycle perpétuel et ses règles
pub async fn cases_flow() -> HttpResponse {
    let e = crate::cases_loop::etat();
    HttpResponse::Ok().json(json!({
        "status": "success",
        "cycle": "Ocean → Nuages → Rivieres → Glaciers → Nappes → Vapeur → Ocean",
        "regles": [
            "Ocean < 10% → Nuages alimente Ocean",
            "Nuages < 5% → Rivieres alimente Nuages",
            "Rivieres < 10% → Ocean alimente Rivieres",
            "Glaciers < 5% → Nappes alimente Glaciers",
            "Nappes < 5% → Vapeur alimente Nappes",
            "Vapeur < 5% → Ocean alimente Vapeur"
        ],
        "etat_actuel": e.cases.iter().map(|c| json!({
            "nom": c.nom, "borb": c.borb, "pct": c.pct_capacite,
            "seuil": c.seuil_pct, "sous_seuil": c.sous_seuil
        })).collect::<Vec<_>>(),
        "sante": e.sante
    }))
}

/// GET /api/cases/market — analyse offre / demande
pub async fn cases_market() -> HttpResponse {
    let m = crate::cases_loop::analyser_marche();
    HttpResponse::Ok().json(json!({
        "status": "success",
        "marche": m,
        "note": "Le carnet d'ordres P2P du Profil 1 (Phase 3) affinera cette analyse"
    }))
}

/// GET /api/cases/invariant
pub async fn cases_invariant() -> HttpResponse {
    let orb = crate::persistence::load_global_orb();
    let total = orb.total();
    let capacite = crate::montant::CAPACITE_TOTALE;
    HttpResponse::Ok().json(json!({
        "status": if total == capacite { "success" } else { "error" },
        "invariant_ok": total == capacite,
        "formule": "6 cases + enveloppe UBI + comptes = 40 Md BORB",
        "cases_femto": orb.cases.iter().map(|c| c.to_string()).collect::<Vec<_>>(),
        "ubi_femto": orb.ubi_enveloppe.to_string(),
        "comptes_femto": orb.somme_comptes().to_string(),
        "total_femto": total.to_string(),
        "capacite_femto": capacite.to_string()
    }))
}

/// GET /api/cases/history — OWNER
pub async fn cases_history(req: HttpRequest) -> HttpResponse {
    if let Err(r) = crate::auth_guard::exiger_owner(&req) {
        return r;
    }
    let h = crate::cases_loop::historique();
    HttpResponse::Ok().json(json!({
        "status": "success",
        "total": h.len(),
        "mouvements": h
    }))
}

/// POST /api/cases/auto-regulate — OWNER
pub async fn cases_auto_regulate(req: HttpRequest) -> HttpResponse {
    if let Err(r) = crate::auth_guard::exiger_owner(&req) {
        return r;
    }
    match crate::cases_loop::auto_reguler() {
        Ok(r) => HttpResponse::Ok().json(json!({
            "status": "success",
            "applique": r.applique,
            "raison": r.raison,
            "mouvements": r.mouvements,
            "sante_avant": r.sante_avant,
            "sante_apres": r.sante_apres,
            "invariant_ok": r.invariant_ok
        })),
        Err(e) => HttpResponse::BadRequest().json(json!({"status": "error", "message": e})),
    }
}

// ===== / T che de fond (Phase D) : vérifie et régule toutes les 60 s. =====
pub async fn boucle_regulation() {
    tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
    loop {
        match crate::cases_loop::auto_reguler() {
            Ok(r) if r.applique => {
                eprintln!("🔄 Auto-regulation : {} mouvement(s)", r.mouvements.len());
            }
            Ok(_) => {}
            Err(e) => eprintln!("⚠️  Auto-regulation impossible : {}", e),
        }
        let m = crate::cases_loop::analyser_marche();
        if m.tendance != "equilibre" {
            eprintln!(
                "🌍 Marche : {} (offres={} demandes={})",
                m.tendance, m.offres, m.demandes
            );
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
    }
}
