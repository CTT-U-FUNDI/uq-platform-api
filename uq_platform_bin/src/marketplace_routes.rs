// ============================================================================
// marketplace_routes.rs — Routes REST du marketplace UQ
// ============================================================================

use actix_web::{web, HttpRequest, HttpResponse};
use serde_json::json;

use crate::auth_guard::exiger_auth;
use crate::marketplace;

fn ok(value: serde_json::Value) -> HttpResponse {
    HttpResponse::Ok().json(json!({ "status": "success", "data": value }))
}

fn err(message: &str) -> HttpResponse {
    HttpResponse::BadRequest().json(json!({ "status": "error", "message": message }))
}

fn auth(req: &HttpRequest) -> Result<String, HttpResponse> {
    exiger_auth(req)
        .map(|c| c.sub.trim().to_uppercase())
        .map_err(|r| r)
}

/// Vérifie le PIN de connexion du compte (Q4/Q18) via la fonction officielle
/// du projet. Retourne la réponse d'erreur HTTP (403/400/429) le cas échéant.
fn verifier_pin(uq: &str, pin: &str, action: &str) -> Result<(), HttpResponse> {
    let account_db_path = format!("data/users/{}.db", uq.replace('-', "_").to_lowercase());
    crate::api_handlers::verifier_pin_compte(&account_db_path, uq, pin, action)
}

/// Extrait le PIN du body JSON ; erreur 403 PIN_REQUIS si absent.
fn pin_du_body(body: &Option<web::Json<serde_json::Value>>) -> Result<String, HttpResponse> {
    let pin = body
        .as_ref()
        .and_then(|b| b.get("pin"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if pin.is_empty() {
        return Err(HttpResponse::Forbidden().json(json!({
            "status": "error",
            "code": "PIN_REQUIS",
            "message": "Code PIN requis"
        })));
    }
    Ok(pin.to_string())
}

/// Garde-fou gel gradué (gel.rs) : 403 si l'action est bloquée pour le compte.
fn exiger_gel(uq: &str, action: &str) -> Result<(), HttpResponse> {
    crate::gel::exiger(uq, action).map_err(|e| {
        HttpResponse::Forbidden().json(json!({ "status": "error", "message": e }))
    })
}

/// POST /api/marketplace/transactions — créer une réservation.
/// Body : { produit_id, type_produit, vendeur, points_total, points_livraison?, devise?, acompte? }
pub async fn creer(req: HttpRequest, body: web::Json<serde_json::Value>) -> HttpResponse {
    let acheteur = match auth(&req) { Ok(u) => u, Err(r) => return r };
    let vendeur = body.get("vendeur").and_then(|v| v.as_str()).unwrap_or("");
    let produit_id = body.get("produit_id").and_then(|v| v.as_str()).unwrap_or("");
    let type_produit = body.get("type_produit").and_then(|v| v.as_str()).unwrap_or("physique");
    let points_total = body.get("points_total").and_then(|v| v.as_i64()).unwrap_or(0);
    let points_livraison = body.get("points_livraison").and_then(|v| v.as_i64()).unwrap_or(0);
    let devise = body.get("devise").and_then(|v| v.as_str()).unwrap_or("USD");
    let acompte = body.get("acompte").and_then(|v| v.as_bool()).unwrap_or(false);
    let creneau_id = body.get("creneau_id").and_then(|v| v.as_i64());

    if vendeur.is_empty() || produit_id.is_empty() {
        return err("produit_id et vendeur obligatoires");
    }
    // Gel gradué : acheter pour l'acheteur, vendre pour le vendeur.
    if let Err(r) = exiger_gel(&acheteur, "acheter") { return r; }
    if let Err(r) = exiger_gel(vendeur, "vendre") { return r; }
    match marketplace::creer_reservation(
        &acheteur, produit_id, type_produit, vendeur,
        points_total, points_livraison, devise, acompte, creneau_id,
    ) {
        Ok(id) => ok(json!({ "id": id })),
        Err(e) => err(&e),
    }
}

/// GET /api/marketplace/{id} — le prix est masqué pour le livreur (Q8).
pub async fn detail(req: HttpRequest, path: web::Path<i64>) -> HttpResponse {
    // Auth optionnelle : un visiteur non connecté voit la transaction sans prix
    // seulement s'il n'est pas le livreur ; les champs prix restent visibles
    // pour acheteur/vendeur authentifiés.
    let uq = exiger_auth(&req).ok().map(|c| c.sub.trim().to_uppercase());
    match marketplace::detail_pour(path.into_inner(), uq.as_deref()) {
        Ok(v) => ok(v),
        Err(e) => err(&e),
    }
}

/// GET /api/marketplace/mes?role=acheteur|vendeur|livreur
pub async fn mes(req: HttpRequest, query: web::Query<std::collections::HashMap<String, String>>) -> HttpResponse {
    let uq = match auth(&req) { Ok(u) => u, Err(r) => return r };
    let role = query.get("role").map(String::as_str).unwrap_or("acheteur");
    match marketplace::mes(&uq, role, 50) {
        Ok(list) => ok(json!(list)),
        Err(e) => err(&e),
    }
}

/// POST /api/marketplace/{id}/payer — body { pin } : Q4, le PIN de connexion
/// est exigé avant tout gel du séquestre.
pub async fn payer(
    req: HttpRequest,
    path: web::Path<i64>,
    body: Option<web::Json<serde_json::Value>>,
) -> HttpResponse {
    let uq = match auth(&req) { Ok(u) => u, Err(r) => return r };
    let pin = match pin_du_body(&body) { Ok(p) => p, Err(r) => return r };
    if let Err(r) = exiger_gel(&uq, "acheter") { return r; }
    if let Err(reponse) = verifier_pin(&uq, &pin, "payer une transaction marketplace") {
        return reponse;
    }
    match marketplace::payer(path.into_inner(), &uq) {
        Ok(v) => ok(v),
        Err(e) => err(&e),
    }
}

/// POST /api/marketplace/{id}/accepter
pub async fn accepter(req: HttpRequest, path: web::Path<i64>) -> HttpResponse {
    let uq = match auth(&req) { Ok(u) => u, Err(r) => return r };
    if let Err(r) = exiger_gel(&uq, "vendre") { return r; }
    match marketplace::accepter(path.into_inner(), &uq) {
        Ok(v) => ok(v),
        Err(e) => err(&e),
    }
}

/// POST /api/marketplace/{id}/refuser
pub async fn refuser(req: HttpRequest, path: web::Path<i64>) -> HttpResponse {
    let uq = match auth(&req) { Ok(u) => u, Err(r) => return r };
    if let Err(r) = exiger_gel(&uq, "vendre") { return r; }
    match marketplace::refuser(path.into_inner(), &uq) {
        Ok(v) => ok(v),
        Err(e) => err(&e),
    }
}

/// POST /api/marketplace/{id}/livreur — body { livreur }
pub async fn assigner_livreur(req: HttpRequest, path: web::Path<i64>, body: web::Json<serde_json::Value>) -> HttpResponse {
    let uq = match auth(&req) { Ok(u) => u, Err(r) => return r };
    let livreur = body.get("livreur").and_then(|v| v.as_str()).unwrap_or("");
    if let Err(r) = exiger_gel(&uq, "vendre") { return r; }
    if let Err(r) = exiger_gel(livreur, "livraison") { return r; }
    match marketplace::assigner_livreur(path.into_inner(), &uq, livreur) {
        Ok(v) => ok(v),
        Err(e) => err(&e),
    }
}

/// POST /api/marketplace/{id}/livreur_accepte
pub async fn livreur_accepte(req: HttpRequest, path: web::Path<i64>) -> HttpResponse {
    let uq = match auth(&req) { Ok(u) => u, Err(r) => return r };
    if let Err(r) = exiger_gel(&uq, "livraison") { return r; }
    match marketplace::livreur_accepte(path.into_inner(), &uq) {
        Ok(v) => ok(v),
        Err(e) => err(&e),
    }
}

/// POST /api/marketplace/{id}/livreur_refuse
pub async fn livreur_refuse(req: HttpRequest, path: web::Path<i64>) -> HttpResponse {
    let uq = match auth(&req) { Ok(u) => u, Err(r) => return r };
    if let Err(r) = exiger_gel(&uq, "livraison") { return r; }
    match marketplace::livreur_refuse(path.into_inner(), &uq) {
        Ok(v) => ok(v),
        Err(e) => err(&e),
    }
}

/// POST /api/marketplace/{id}/action — body { action, code?, motif? }
/// actions : livre | recoit | recupere | code | litige | annuler
pub async fn action(req: HttpRequest, path: web::Path<i64>, body: web::Json<serde_json::Value>) -> HttpResponse {
    let uq = match auth(&req) { Ok(u) => u, Err(r) => return r };
    let action = body.get("action").and_then(|v| v.as_str()).unwrap_or("");
    let code = body.get("code").and_then(|v| v.as_str());
    let motif = body.get("motif").and_then(|v| v.as_str());
    // Gel gradué : l'action dépend du rôle de l'acteur dans la transaction.
    // 'litige' et 'annuler' restent toujours possibles (contestation/droit de recours).
    if !matches!(action, "litige" | "annuler" | "code") {
        if let Ok(detail) = marketplace::detail(*path) {
            let role_action = if detail.get("vendeur").and_then(|v| v.as_str()) == Some(uq.as_str()) {
                "vendre"
            } else if detail.get("livreur_uq").and_then(|v| v.as_str()) == Some(uq.as_str()) {
                "livraison"
            } else {
                "acheter"
            };
            if let Err(r) = exiger_gel(&uq, role_action) { return r; }
        }
    }
    match marketplace::action(path.into_inner(), &uq, action, code, motif) {
        Ok(v) => ok(v),
        Err(e) => err(&e),
    }
}

/// POST /api/marketplace/{id}/gps — body { lat, lng } : point GPS de la transaction (Q2).
pub async fn gps(
    req: HttpRequest,
    path: web::Path<i64>,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let uq = match auth(&req) { Ok(u) => u, Err(r) => return r };
    let lat = body.get("lat").and_then(|v| v.as_f64()).unwrap_or(f64::NAN);
    let lng = body.get("lng").and_then(|v| v.as_f64()).unwrap_or(f64::NAN);
    match marketplace::enregistrer_gps(path.into_inner(), &uq, lat, lng) {
        Ok(()) => ok(json!({ "enregistre": true })),
        Err(e) => err(&e),
    }
}

/// POST /api/livreur/register — body { livreur, charte_acceptee } : le vendeur
/// connecté enregistre un livreur de confiance (Q9 + conditions PARTIE 2).
pub async fn livreur_register(req: HttpRequest, body: web::Json<serde_json::Value>) -> HttpResponse {
    let vendeur = match auth(&req) { Ok(u) => u, Err(r) => return r };
    let livreur = body.get("livreur").and_then(|v| v.as_str()).unwrap_or("");
    let charte = body.get("charte_acceptee").and_then(|v| v.as_bool()).unwrap_or(false);
    if let Err(r) = exiger_gel(&vendeur, "vendre") { return r; }
    if let Err(r) = exiger_gel(livreur, "livraison") { return r; }
    match marketplace::enregistrer_livreur(&vendeur, livreur, charte) {
        Ok(()) => ok(json!({ "enregistre": true })),
        Err(e) => err(&e),
    }
}

/// GET /api/livreur/list — livreurs de confiance du vendeur connecté (Q9).
pub async fn livreur_list(req: HttpRequest) -> HttpResponse {
    let vendeur = match auth(&req) { Ok(u) => u, Err(r) => return r };
    match marketplace::lister_livreurs(&vendeur) {
        Ok(list) => ok(json!(list)),
        Err(e) => err(&e),
    }
}

/// POST /api/livreur/remove — body { livreur } : retire un livreur (Q9).
pub async fn livreur_remove(req: HttpRequest, body: web::Json<serde_json::Value>) -> HttpResponse {
    let vendeur = match auth(&req) { Ok(u) => u, Err(r) => return r };
    let livreur = body.get("livreur").and_then(|v| v.as_str()).unwrap_or("");
    match marketplace::retirer_livreur(&vendeur, livreur) {
        Ok(()) => ok(json!({ "retire": true })),
        Err(e) => err(&e),
    }
}

/// POST /api/marketplace/creneaux — body { produit_id, vendeur, date_debut,
/// date_fin, capacite } : crée un créneau (Q14). Vendeur = utilisateur connecté
/// ou champ vendeur explicite (le vendeur doit correspondre au compte).
pub async fn creer_creneau(
    req: HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let uq = match auth(&req) { Ok(u) => u, Err(r) => return r };
    let produit_id = body.get("produit_id").and_then(|v| v.as_str()).unwrap_or("");
    let vendeur = body.get("vendeur").and_then(|v| v.as_str()).unwrap_or(uq.as_str());
    let date_debut = body.get("date_debut").and_then(|v| v.as_i64()).unwrap_or(0);
    let date_fin = body.get("date_fin").and_then(|v| v.as_i64()).unwrap_or(0);
    let capacite = body.get("capacite").and_then(|v| v.as_i64()).unwrap_or(0);
    if vendeur != uq {
        return err("Tu ne peux créer des créneaux que pour ton propre compte");
    }
    if let Err(r) = exiger_gel(&uq, "vendre") { return r; }
    match marketplace::creer_creneau(produit_id, vendeur, date_debut, date_fin, capacite) {
        Ok(id) => ok(json!({ "id": id })),
        Err(e) => err(&e),
    }
}

/// GET /api/marketplace/creneaux/{produit_id} — créneaux d'un produit (Q14).
pub async fn creneaux(_req: HttpRequest, path: web::Path<String>) -> HttpResponse {
    match marketplace::lister_creneaux(&path.into_inner()) {
        Ok(list) => ok(json!(list)),
        Err(e) => err(&e),
    }
}

/// POST /api/marketplace/assurance — body { type_produit, fichier_path } :
/// dépose une attestation d'assurance (Q16).
pub async fn assurance(req: HttpRequest, body: web::Json<serde_json::Value>) -> HttpResponse {
    let uq = match auth(&req) { Ok(u) => u, Err(r) => return r };
    let type_produit = body.get("type_produit").and_then(|v| v.as_str()).unwrap_or("");
    let fichier = body.get("fichier_path").and_then(|v| v.as_str()).unwrap_or("");
    if let Err(r) = exiger_gel(&uq, "vendre") { return r; }
    match marketplace::uploader_assurance(&uq, type_produit, fichier) {
        Ok(id) => ok(json!({ "id": id })),
        Err(e) => err(&e),
    }
}

/// GET /api/marketplace/assurance?type=... — le vendeur connecté est-il couvert ?
pub async fn assurance_statut(req: HttpRequest, query: web::Query<std::collections::HashMap<String, String>>) -> HttpResponse {
    let uq = match auth(&req) { Ok(u) => u, Err(r) => return r };
    let type_produit = query.get("type").map(String::as_str).unwrap_or("");
    match marketplace::assurance_couverte(&uq, type_produit) {
        Ok(valide) => ok(json!({ "valide": valide })),
        Err(e) => err(&e),
    }
}

/// POST /api/marketplace/avertir — body { vendeur, motif } : enregistre un
/// avertissement ; au 5e, le vendeur est suspendu 7 jours (Q12).
pub async fn avertir(req: HttpRequest, body: web::Json<serde_json::Value>) -> HttpResponse {
    let _uq = match auth(&req) { Ok(u) => u, Err(r) => return r };
    let vendeur = body.get("vendeur").and_then(|v| v.as_str()).unwrap_or("");
    let motif = body.get("motif").and_then(|v| v.as_str()).unwrap_or("");
    if vendeur.is_empty() {
        return err("vendeur obligatoire");
    }
    match marketplace::avertir(vendeur, motif) {
        Ok(v) => ok(v),
        Err(e) => err(&e),
    }
}

// ---------------------------------------------------------------------------
// Q18 — Paiement hors UQ
// ---------------------------------------------------------------------------

/// POST /api/marketplace/{id}/mode-hors-uq/acheteur — body { pin } :
/// l'acheteur déclare le paiement hors UQ (1 % de frais, sans garantie UQ).
pub async fn hors_uq_acheteur(
    req: HttpRequest,
    path: web::Path<i64>,
    body: Option<web::Json<serde_json::Value>>,
) -> HttpResponse {
    let uq = match auth(&req) { Ok(u) => u, Err(r) => return r };
    let pin = match pin_du_body(&body) { Ok(p) => p, Err(r) => return r };
    if let Err(r) = exiger_gel(&uq, "acheter") { return r; }
    if let Err(reponse) = verifier_pin(&uq, &pin, "déclarer un paiement hors UQ") {
        return reponse;
    }
    match marketplace::declarer_hors_uq(path.into_inner(), &uq) {
        Ok(v) => ok(v),
        Err(e) => err(&e),
    }
}

/// POST /api/marketplace/{id}/mode-hors-uq/vendeur — body { pin } :
/// le vendeur confirme le mode hors UQ (1 % de frais de son côté).
pub async fn hors_uq_vendeur(
    req: HttpRequest,
    path: web::Path<i64>,
    body: Option<web::Json<serde_json::Value>>,
) -> HttpResponse {
    let uq = match auth(&req) { Ok(u) => u, Err(r) => return r };
    let pin = match pin_du_body(&body) { Ok(p) => p, Err(r) => return r };
    if let Err(r) = exiger_gel(&uq, "vendre") { return r; }
    if let Err(reponse) = verifier_pin(&uq, &pin, "confirmer un paiement hors UQ") {
        return reponse;
    }
    match marketplace::confirmer_hors_uq(path.into_inner(), &uq) {
        Ok(v) => ok(v),
        Err(e) => err(&e),
    }
}

/// POST /api/marketplace/{id}/mode-hors-uq/refuser — le vendeur refuse le
/// hors UQ : la transaction revient en mode sécurisé.
pub async fn hors_uq_refuser(req: HttpRequest, path: web::Path<i64>) -> HttpResponse {
    let uq = match auth(&req) { Ok(u) => u, Err(r) => return r };
    match marketplace::refuser_hors_uq(path.into_inner(), &uq) {
        Ok(v) => ok(v),
        Err(e) => err(&e),
    }
}

/// POST /api/marketplace/{id}/hors-uq/litige — body { motif }.
pub async fn hors_uq_litige(req: HttpRequest, path: web::Path<i64>, body: web::Json<serde_json::Value>) -> HttpResponse {
    let uq = match auth(&req) { Ok(u) => u, Err(r) => return r };
    let motif = body.get("motif").and_then(|v| v.as_str()).unwrap_or("");
    match marketplace::litige_hors_uq(path.into_inner(), &uq, motif) {
        Ok(v) => ok(v),
        Err(e) => err(&e),
    }
}

/// POST /api/marketplace/{id}/hors-uq/terminer — l'acheteur marque « reçu ».
pub async fn hors_uq_terminer(req: HttpRequest, path: web::Path<i64>) -> HttpResponse {
    let uq = match auth(&req) { Ok(u) => u, Err(r) => return r };
    match marketplace::terminer_hors_uq(path.into_inner(), &uq) {
        Ok(v) => ok(v),
        Err(e) => err(&e),
    }
}

// ---------------------------------------------------------------------------
// Compte : avertissements et restrictions
// ---------------------------------------------------------------------------

/// GET /api/account/warnings — avertissements + blocages du compte connecté.
pub async fn warnings(req: HttpRequest) -> HttpResponse {
    let uq = match auth(&req) { Ok(u) => u, Err(r) => return r };
    match marketplace::avertissements(&uq) {
        Ok(v) => ok(v),
        Err(e) => err(&e),
    }
}

/// GET /api/account/restrictions — alias de /api/account/warnings.
pub async fn restrictions(req: HttpRequest) -> HttpResponse {
    warnings(req).await
}

/// POST /api/marketplace/jobs — déclenche les jobs périodiques marketplace
/// (sur-réservation, resynchro créneaux, escrows stale). À planifier côté
/// scheduler ; exposé ici pour déclenchement manuel/admin.
pub async fn jobs(req: HttpRequest) -> HttpResponse {
    let _uq = match auth(&req) { Ok(u) => u, Err(r) => return r };
    match marketplace::jobs_periodiques() {
        Ok(v) => ok(v),
        Err(e) => err(&e),
    }
}
