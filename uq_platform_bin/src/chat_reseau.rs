//! Etat reseau du chat : presence mesuree et compteurs du bandeau.
//!
//! Pas de connexion persistante ni de battement d'horloge dedie : un compte est
//! tenu pour actif quand il a appele une route chat authentifiee dans les
//! `PRESENCE_SECS` dernieres. C'est mesure, pas invente — et cela disparait au
//! redemarrage du serveur, comme toute presence en memoire.

use actix_web::{web, HttpRequest, HttpResponse};
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// Fenetre de presence : au-dela, le contact n'est plus affiche comme actif.
pub const PRESENCE_SECS: i64 = 5 * 60;

fn horloges() -> &'static Mutex<HashMap<String, i64>> {
    static HORLOGES: OnceLock<Mutex<HashMap<String, i64>>> = OnceLock::new();
    HORLOGES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn normaliser(uq: &str) -> String {
    uq.trim().to_uppercase()
}

/// Enregistre l'activite d'un compte. Appele par les routes chat authentifiees.
pub fn signaler(uq: &str) {
    if uq.trim().is_empty() {
        return;
    }
    let Ok(mut etat) = horloges().lock() else {
        return;
    };
    let maintenant = chrono::Utc::now().timestamp();
    etat.insert(normaliser(uq), maintenant);
    // La table ne doit pas grossir sans fin : on oublie les comptes silencieux
    // depuis une heure.
    if etat.len() > 512 {
        etat.retain(|_, vu| maintenant - *vu < 3600);
    }
}

/// Vrai si le compte a ete vu dans la fenetre de presence.
pub fn actif(uq: &str) -> bool {
    let Ok(etat) = horloges().lock() else {
        return false;
    };
    match etat.get(&normaliser(uq)) {
        Some(vu) => chrono::Utc::now().timestamp() - *vu <= PRESENCE_SECS,
        None => false,
    }
}

/// GET /api/chat/status — compteurs reels pour le bandeau du chat.
pub async fn chat_reseau_status(req: HttpRequest) -> HttpResponse {
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(claims) => claims,
        Err(response) => return response,
    };
    let moi = normaliser(&claims.sub);
    signaler(&moi);

    let db = crate::database::Database::shared("uq_platform_db.json");
    let conversations = db.get_chat_inbox(&moi);
    let contacts = crate::chat_contacts::liste_contacts(&claims.sub).unwrap_or_default();

    let en_ligne = contacts
        .iter()
        .filter_map(|contact| contact.get("peer_id").and_then(|v| v.as_str()))
        .filter(|peer| actif(peer))
        .count();
    let pairs_actifs = conversations
        .iter()
        .filter_map(|c| c.get("peer_id").and_then(|v| v.as_str()))
        .filter(|peer| actif(peer))
        .count();

    HttpResponse::Ok().json(json!({
        "status": "success",
        "chiffrement": "transport https, stockage serveur UQ",
        "ephemere_par_defaut": false,
        "ttl_ephemere_secondes": crate::database::MESSAGE_TTL_SECS,
        "presence_secondes": PRESENCE_SECS,
        "contacts_en_ligne": en_ligne,
        "conversations_en_ligne": pairs_actifs,
        "total_conversations": conversations.len(),
        "messages_non_lus": db.compter_non_lus(&moi),
        "conversations_non_lues": db.conversations_non_lues(&moi),
    }))
}

/// POST /api/chat/read — { peer } : le lecteur confirme avoir relu le fil.
/// La marque est la meme que l'ouverture du fil (message `delivered`), mais le
/// client peut la poser sans charger toute la conversation.
pub async fn chat_marquer_lu(req: HttpRequest, body: web::Json<serde_json::Value>) -> HttpResponse {
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(claims) => claims,
        Err(response) => return response,
    };
    let peer = match body.get("peer").and_then(|v| v.as_str()) {
        Some(value) if crate::chat_contacts::uq_valide(value) => value.to_string(),
        _ => {
            return HttpResponse::BadRequest().json(json!({
                "status": "error",
                "message": "peer invalide",
            }))
        }
    };
    signaler(&claims.sub);
    let db = crate::database::Database::shared("uq_platform_db.json");
    let marques = db.marquer_lus(&claims.sub, &peer);
    HttpResponse::Ok().json(json!({
        "status": "success",
        "marques": marques,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn un_compte_vient_d_etre_vu_est_actif() {
        let uq = "UQ-900000001";
        signaler(uq);
        assert!(actif(uq), "la presence doit sortir de la mesure, pas d un defaut");
    }

    #[test]
    fn un_compte_jamais_vu_n_est_pas_actif() {
        assert!(!actif("UQ-900000002"), "rien ne doit etre declare en ligne sans appel");
    }

    #[test]
    fn la_normalisation_ne_depend_pas_de_la_saisie() {
        signaler("uq-900000003");
        assert!(actif("UQ-900000003"));
    }
}
