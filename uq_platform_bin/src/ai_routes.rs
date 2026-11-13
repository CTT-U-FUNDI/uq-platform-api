//! Passerelle UQ AI.
//!
//! La cle du fournisseur reste du cote serveur : le frontend ne la voit jamais,
//! sinon chaque visiteur pourrait la lire dans le bundle et consommer le quota.
//! Sans cle (ou sans reseau), la route repond `disponible: false` et le chat
//! retombe sur sa base de connaissances locale.

use actix_web::{web, HttpResponse};
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const CLE_ENV: &str = "GEMINI_API_KEY";
const MODELE_ENV: &str = "GEMINI_MODEL";
/// Modele verifie sur ce projet : `gemini-2.5-flash` est refuse aux nouveaux
/// comptes (404) et les alias « latest » restent files (503 / reponse > 90 s).
const MODELE_DEFAUT: &str = "gemini-3.5-flash";
const URL_API: &str = "https://generativelanguage.googleapis.com/v1beta/models";

/// Borne envoyee brute : au-dela on refuse plutot que de payer une facture.
const QUESTION_MAX_CAR: usize = 1200;
const ECHANGES_MAX: usize = 12;
const REPONSE_MAX_TOKENS: u32 = 700;
const MINUTE: Duration = Duration::from_secs(60);
const REQUETES_PAR_MINUTE: u32 = 12;

/// Les regles du projet, pour ne pas laisser le modele inventer la plateforme.
const CONSIGNES: &str = concat!(
    "Tu es UQ AI, l'assistant de la plateforme UQ (application de portefeuille, ",
    "d'echange et de messagerie).\n",
    "Regles absolues:\n",
    "- Le BORB est la monnaie reelle de la plateforme, dotee d'une masse monetaire ",
    "fixe de 40 milliards. Un point de fidelite vaut 0,01 BORB. BORB et Borb sont ",
    "des noms internes de conversion : ne les presente jamais comme des monnaies ",
    "differentes du BORB.\n",
    "- Tu n'inventes jamais un solde, un tarif, un identifiant ou une adresse de ",
    "page. Si la reponse depend des donnees du compte, dis a l'utilisateur d'ouvrir ",
    "l'ecran concerne (Portefeuille, Echange, Chat, Profil) et explique ou chercher.\n",
    "- « UQ » n'est jamais un pseudo d'utilisateur.\n",
    "- Repond dans la langue de la question. Phrases courtes, utiles, sans jargon.\n",
    "- Ecris en texte brut, sans markdown ni étoiles : la reponse se lit dans une ",
    "bulle de messagerie, elle ne rend que du texte.\n",
    "- Ne pret rien savoir du solde reel de l'utilisateur : tu ne recois que ses ",
    "questions, jamais ses donnees de compte.\n",
    "- Si la demande sort du perimetre de la plateforme, repond quand meme brievement, ",
    "puis signale ce qui doit etre verifie.",
);

fn compteurs() -> &'static Mutex<HashMap<String, (u32, Instant)>> {
    static COMPTEURS: OnceLock<Mutex<HashMap<String, (u32, Instant)>>> = OnceLock::new();
    COMPTEURS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Une fenetre glissante de 60 s par compte : la cle ne doit pas devenir un
/// service public gratuit pour quiconque sait appeler la route.
fn autorise(compte: &str) -> bool {
    let maintenant = Instant::now();
    let Ok(mut etat) = compteurs().lock() else {
        return true;
    };
    let entree = etat.entry(compte.to_string()).or_insert((0, maintenant));
    if maintenant.duration_since(entree.1) >= MINUTE {
        entree.0 = 0;
        entree.1 = maintenant;
    }
    if entree.0 >= REQUETES_PAR_MINUTE {
        return false;
    }
    entree.0 += 1;
    if etat.len() > 512 {
        etat.retain(|_, v| maintenant.duration_since(v.1) < MINUTE);
    }
    true
}

fn texte_candidates(corps: &serde_json::Value) -> Option<String> {
    let parts = corps.get("candidates")?.get(0)?.get("content")?.get("parts")?;
    let phrases: Vec<&str> = parts
        .as_array()?
        .iter()
        .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
        .collect();
    let texte = phrases.join("").trim().to_string();
    if texte.is_empty() {
        None
    } else {
        Some(texte)
    }
}

pub async fn api_ai_ask(
    req: actix_web::HttpRequest,
    body: web::Json<serde_json::Value>,
) -> HttpResponse {
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(claims) => claims,
        Err(reponse) => return reponse,
    };

    let cle = match std::env::var(CLE_ENV) {
        Ok(cle) if !cle.trim().is_empty() => cle.trim().to_string(),
        _ => {
            return HttpResponse::ServiceUnavailable().json(json!({
                "disponible": false,
                "raison": "cle_absente",
            }))
        }
    };

    /* La requete d'abord, le compteur ensuite : une question vide ou trop
       longue doit etre refusee sans entamer le quota de la minute. */
    let question = body.get("question").and_then(|v| v.as_str()).unwrap_or("").trim();
    if question.is_empty() {
        return HttpResponse::BadRequest().json(json!({
            "disponible": false,
            "raison": "question_vide",
        }));
    }
    if question.chars().count() > QUESTION_MAX_CAR {
        return HttpResponse::PayloadTooLarge().json(json!({
            "disponible": false,
            "raison": "question_trop_longue",
        }));
    }

    if !autorise(&claims.sub) {
        return HttpResponse::TooManyRequests().json(json!({
            "disponible": false,
            "raison": "quota",
        }));
    }

    // L'historique sert seulement a la coherence du fil : le serveur ne le garde pas.
    let mut echanges: Vec<serde_json::Value> = Vec::new();
    if let Some(histoire) = body.get("histoire").and_then(|v| v.as_array()) {
        for tour in histoire.iter().rev().take(ECHANGES_MAX).rev() {
            let texte = tour.get("texte").and_then(|v| v.as_str()).unwrap_or("").trim();
            let role = match tour.get("role").and_then(|v| v.as_str()) {
                Some("assistant") => "model",
                _ => "user",
            };
            if !texte.is_empty() {
                echanges.push(json!({ "role": role, "parts": [{ "text": texte }] }));
            }
        }
    }
    echanges.push(json!({ "role": "user", "parts": [{ "text": question }] }));

    let modele = std::env::var(MODELE_ENV)
        .unwrap_or_else(|_| MODELE_DEFAUT.to_string())
        .trim()
        .to_string();
    let payload = json!({
        "systemInstruction": { "parts": [{ "text": CONSIGNES }] },
        "contents": echanges,
        "generationConfig": { "temperature": 0.4, "maxOutputTokens": REPONSE_MAX_TOKENS },
    });

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(25))
        .build()
    {
        Ok(client) => client,
        Err(_) => {
            return HttpResponse::InternalServerError().json(json!({
                "disponible": false,
                "raison": "client_http",
            }))
        }
    };

    let appel = client
        .post(format!("{}/{}:generateContent", URL_API, modele))
        .header("x-goog-api-key", &cle)
        .json(&payload)
        .send()
        .await;

    let reponse = match appel {
        Ok(reponse) => reponse,
        Err(erreur) => {
            // Le texte de l'erreur peut porter l'URL ; on ne journalise que le
            // fait d'avoir echoue, jamais la cle.
            eprintln!("UQ AI: appel fournisseur echoue ({})", erreur.is_timeout());
            return HttpResponse::BadGateway().json(json!({
                "disponible": false,
                "raison": "reseau",
            }));
        }
    };

    let statut = reponse.status();
    if !statut.is_success() {
        // 401/403 = cle refusee, 429 = quota fournisseur : dans tous les cas le
        // chat doit retomber sur l'assistant local sans afficher de secret.
        eprintln!("UQ AI: reponse fournisseur {} pour le modele {}", statut.as_u16(), modele);
        return HttpResponse::BadGateway().json(json!({
            "disponible": false,
            "raison": "fournisseur",
        }));
    }

    let corps = match reponse.json::<serde_json::Value>().await {
        Ok(corps) => corps,
        Err(_) => {
            return HttpResponse::BadGateway().json(json!({
                "disponible": false,
                "raison": "reponse_ilisible",
            }))
        }
    };

    match texte_candidates(&corps) {
        Some(texte) => HttpResponse::Ok().json(json!({
            "disponible": true,
            "reponse": texte,
            "source": "gemini",
            "modele": modele,
        })),
        None => HttpResponse::Ok().json(json!({
            "disponible": false,
            "raison": "reponse_vide",
        })),
    }
}
