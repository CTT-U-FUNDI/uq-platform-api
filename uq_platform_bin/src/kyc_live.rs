//! Guidage visuel du KYC : client du moteur Python + sessions à nonce.
//!
//! Le process `scripts/kyc_python/vision_server.py` reste vivant pour que le
//! modele YuNet ne soit charge qu'une fois : une image de guidage coute ~250 ms
//! sur ce poste au lieu de ~900 ms par process neuf. Les images de guidage ne
//! sont jamais enregistrees ; seuls les fichiers valides partent dans le dossier KYC.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use actix_web::{web, HttpRequest, HttpResponse};
use base64::Engine;
use serde_json::{json, Value};

const SCRIPT: &str = "scripts/kyc_python/vision_server.py";
const DELAI_GUIDE: Duration = Duration::from_millis(4_000);
/// La sequence de vivacite cumule huit images : template matching + detection a
/// chaque image. Sur ce poste, 8 images prennent 1,3 a 4,2 s ; le delai du guide
/// temps reel serait trop court et tuerait le moteur en plein controle.
const DELAI_SEQUENCE: Duration = Duration::from_millis(25_000);
const INACTIVITE_RELANCE: Duration = Duration::from_secs(120);
const IMAGE_MAX: usize = 2_000_000;
const SEQUENCE_MAX: usize = 40;
const VIE_SESSION_SECS: i64 = 120;

fn delai_pour(commande: &str) -> Duration {
    if commande == "check" {
        DELAI_SEQUENCE
    } else {
        DELAI_GUIDE
    }
}

/// Alphabet sans ambiguite visuelle : I, O, 0 et 1 exclus, le nonce doit rester relisible.
const NONCE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

type Sortie = BufReader<std::process::ChildStdout>;

struct Cote {
    processus: Child,
    entree: std::process::ChildStdin,
    sortie: Option<Sortie>,
    prochain: u64,
}

impl Cote {
    fn demarrer() -> Result<Cote, String> {
        let mut processus = Command::new("python")
            .arg(SCRIPT)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Impossible de lancer le moteur de vision: {}", e))?;
        let entree = processus.stdin.take().ok_or("Entree du moteur indisponible")?;
        let sortie = processus.stdout.take().map(BufReader::new);
        Ok(Cote { processus, entree, sortie, prochain: 0 })
    }

    /// Le lecteur tourne dans son thread : on peut donc abandonner la lecture si
    /// le moteur ne repond pas, sans bloquer le worker HTTP pour toujours.
    fn lire(&mut self, delai: Duration) -> Result<Value, String> {
        let lecteur = self.sortie.take().ok_or("Le moteur est deja occupe")?;
        let attendu = self.prochain;
        let (expediteur, recepteur) = mpsc::channel();
        thread::spawn(move || {
            let mut lecteur = lecteur;
            let mut tampon = String::new();
            let resultat = lecteur.read_line(&mut tampon);
            let _ = expediteur.send((resultat, tampon, lecteur));
        });

        match recepteur.recv_timeout(delai) {
            Ok((Ok(n), tampon, lecteur)) => {
                self.sortie = Some(lecteur);
                if n == 0 {
                    return Err("Le moteur de vision s'est arrete".to_string());
                }
                let reponse: Value = serde_json::from_str(tampon.trim())
                    .map_err(|e| format!("Reponse illegible: {}", e))?;
                if reponse.get("id").and_then(|v| v.as_u64()) != Some(attendu) {
                    return Err("Reponse hors sequence du moteur".to_string());
                }
                if reponse.get("ok").and_then(|v| v.as_bool()) != Some(true) {
                    return Err(reponse
                        .get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("erreur du moteur")
                        .to_string());
                }
                Ok(reponse.get("data").cloned().unwrap_or(json!({})))
            }
            Ok((Err(e), _, _)) => Err(format!("Lecture impossible: {}", e)),
            Err(_) => Err(format!(
                "Le moteur de vision n'a pas repondu dans les {} ms",
                delai.as_millis()
            )),
        }
    }

    fn envoi(&mut self, commande: &str, args: &Value, delai: Duration) -> Result<Value, String> {
        self.prochain += 1;
        let mut requete = args.clone();
        let objet = requete.as_object_mut().ok_or("Arguments invalides")?;
        objet.insert("id".to_string(), json!(self.prochain));
        objet.insert("cmd".to_string(), json!(commande));

        writeln!(self.entree, "{}", requete).map_err(|e| format!("Envoi impossible: {}", e))?;
        self.entree.flush().map_err(|e| format!("Envoi impossible: {}", e))?;
        self.lire(delai)
    }

    fn arrete(&mut self) -> bool {
        self.processus.try_wait().ok().flatten().is_some()
    }
}

static MOTEUR: OnceLock<Mutex<Moteur>> = OnceLock::new();

#[derive(Default)]
struct Moteur {
    cote: Option<Cote>,
    depuis: Option<Instant>,
}

fn verrou() -> Result<std::sync::MutexGuard<'static, Moteur>, String> {
    let garde = MOTEUR.get_or_init(|| Mutex::new(Moteur::default()));
    Ok(garde.lock().unwrap_or_else(|empoisonne| empoisonne.into_inner()))
}

/// Execute une commande : demarre le moteur au premier besoin, le relance s'il
/// est mort ou s'il est reste inactif trop longtemps.
pub fn moteur(commande: &str, args: &Value) -> Result<Value, String> {
    let delai = delai_pour(commande);
    let mut etat = verrou()?;

    if etat.cote.is_some() {
        let trop_vieux = etat.depuis.map(|t| t.elapsed() > INACTIVITE_RELANCE).unwrap_or(true);
        let vivant = etat.cote.as_mut().map(|cot| !cot.arrete()).unwrap_or(false);

        if !trop_vieux && vivant {
            let resultat = etat
                .cote
                .as_mut()
                .map(|cot| cot.envoi(commande, args, delai))
                .unwrap_or_else(|| Err("Le moteur de vision est indisponible".to_string()));
            etat.depuis = Some(Instant::now());
            if resultat.is_err() {
                detruire(&mut etat);
            }
            return resultat;
        }
        detruire(&mut etat);
    }

    let mut cot = Cote::demarrer()?;
    let data = cot.envoi(commande, args, delai);
    if data.is_ok() {
        etat.cote = Some(cot);
        etat.depuis = Some(Instant::now());
    } else {
        let _ = cot.processus.kill();
    }
    data
}

/// Un moteur en erreur ne doit pas rester en place : on coupe le process et on jette l'etat.
fn detruire(etat: &mut Moteur) {
    if let Some(mut cot) = etat.cote.take() {
        let _ = cot.processus.kill();
    }
    *etat = Moteur::default();
}

struct Session {
    uq: String,
    nonce: String,
    cree_le: i64,
    gabarit: String,
    ratio: f64,
    rect: Value,
    utilisee: bool,
}

/// Bilans rendus par le moteur, conserves apres l'expiration de la session :
/// c'est la seule trace faisant foi pour le dossier (le client ne peut pas les
/// inventer, il ne connait que l'identifiant de sa session).
static BILANS: OnceLock<Mutex<HashMap<String, Bilan>>> = OnceLock::new();

struct Bilan {
    uq: String,
    horodatage: i64,
    data: Value,
}

const VIE_BILAN_SECS: i64 = 900;

static SESSIONS: OnceLock<Mutex<HashMap<String, Session>>> = OnceLock::new();

fn table_sessions() -> std::sync::MutexGuard<'static, HashMap<String, Session>> {
    let garde = SESSIONS.get_or_init(|| Mutex::new(HashMap::new()));
    garde.lock().unwrap_or_else(|empoisonne| empoisonne.into_inner())
}

fn nonce_aleatoire(longueur: usize) -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    (0..longueur)
        .map(|_| NONCE_ALPHABET[rng.gen_range(0..NONCE_ALPHABET.len())] as char)
        .collect()
}

pub fn creer_session(uq: &str) -> Result<Value, String> {
    let nonce = nonce_aleatoire(6);
    let maintenant = chrono::Utc::now().timestamp();
    let texte = format!("UQ-{}-{}", nonce, maintenant);
    let gabarit = moteur("challenge", &json!({ "text": texte, "width": 640 }))?;
    let jpeg = gabarit.get("jpeg").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let ratio = gabarit.get("ratio").and_then(|v| v.as_f64()).unwrap_or(0.22);
    let rect = gabarit
        .get("rect")
        .cloned()
        .unwrap_or_else(|| json!({ "cx": 0.5, "cy": 0.82, "w": 0.88 }));
    if jpeg.is_empty() {
        return Err("Le moteur n'a pas produit de defi".to_string());
    }

    let id = format!("kyc_{}_{}", maintenant, nonce_aleatoire(4)).to_lowercase();
    let mut table = table_sessions();
    // Les sessions oubliees ne doivent pas s'accumuler indefiniment en memoire.
    table.retain(|_, s| maintenant - s.cree_le <= VIE_SESSION_SECS * 4);
    table.insert(
        id.clone(),
        Session {
            uq: uq.to_string(),
            nonce: nonce.clone(),
            cree_le: maintenant,
            gabarit: jpeg.clone(),
            ratio,
            rect: rect.clone(),
            utilisee: false,
        },
    );

    Ok(json!({
        "status": "success",
        "session": id,
        "uq": uq,
        "nonce": nonce,
        "texte": texte,
        "template": jpeg,
        "ratio": ratio,
        "rect": rect,
        "expires_in": VIE_SESSION_SECS,
    }))
}

#[derive(Clone)]
struct Empreinte {
    uq: String,
    gabarit: String,
    nonce: String,
    ratio: f64,
    rect: Value,
}

/// Gabarit + defi de la session, tant qu'elle existe et n'a pas encore servi.
fn session_en_cours(id: &str) -> Option<Empreinte> {
    let table = table_sessions();
    let session = table.get(id)?;
    let age = chrono::Utc::now().timestamp() - session.cree_le;
    if age < 0 || age > VIE_SESSION_SECS || session.utilisee {
        return None;
    }
    Some(Empreinte {
        uq: session.uq.clone(),
        gabarit: session.gabarit.clone(),
        nonce: session.nonce.clone(),
        ratio: session.ratio,
        rect: session.rect.clone(),
    })
}

/// Bilan enregistre par le moteur pour cette session, s'il est recent et au bon nom.
pub fn valider_session(session: &str, uq: &str) -> Option<Value> {
    let garde = BILANS.get_or_init(|| Mutex::new(HashMap::new()));
    let table = garde.lock().unwrap_or_else(|empoisonne| empoisonne.into_inner());
    let bilan = table.get(session)?;
    let age = chrono::Utc::now().timestamp() - bilan.horodatage;
    if bilan.uq != uq || age < 0 || age > VIE_BILAN_SECS {
        return None;
    }
    Some(bilan.data.clone())
}

fn enregistrer_bilan(session: &str, uq: &str, data: &Value) {
    let garde = BILANS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut table = garde.lock().unwrap_or_else(|empoisonne| empoisonne.into_inner());
    let maintenant = chrono::Utc::now().timestamp();
    table.retain(|_, b| maintenant - b.horodatage <= VIE_BILAN_SECS * 2);
    table.insert(
        session.to_string(),
        Bilan { uq: uq.to_string(), horodatage: maintenant, data: data.clone() },
    );
}

fn marquer_utilisee(id: &str) {
    let mut table = table_sessions();
    if let Some(session) = table.get_mut(id) {
        session.utilisee = true;
    }
}

fn reponse(data: Result<Value, String>) -> HttpResponse {
    match data {
        Ok(data) => HttpResponse::Ok().json(json!({ "status": "success", "data": data })),
        Err(message) => {
            HttpResponse::BadRequest().json(json!({ "status": "error", "message": message }))
        }
    }
}

/// `web::block` ajoute une couche d'erreur (le worker) sur celle du moteur : on aplatit.
fn aplatir<E: std::fmt::Display>(deux_layers: Result<Result<Value, String>, E>) -> Result<Value, String> {
    match deux_layers {
        Ok(interne) => interne,
        Err(erreur) => Err(format!("Worker de guidage indisponible: {}", erreur)),
    }
}

fn extraire_query(req: &HttpRequest) -> HashMap<String, String> {
    web::Query::<HashMap<String, String>>::from_query(req.query_string())
        .map(|query| query.into_inner())
        .unwrap_or_default()
}

/// POST /api/kyc/live/session — ouvre une session de guidage et son defi.
pub async fn session_demarrer(req: HttpRequest) -> HttpResponse {
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(claims) => claims,
        Err(refus) => return refus,
    };
    let uq = claims.sub.clone();
    match aplatir(web::block(move || creer_session(&uq)).await) {
        Ok(data) => HttpResponse::Ok().json(data),
        Err(message) => HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": message,
            "code": "MOTEUR_INDISPONIBLE"
        })),
    }
}

/// Une image JPEG arrive en corps brut : ni base64 ni JSON, 33 % de trafic en moins.
fn image_brute(corps: &[u8]) -> Result<String, String> {
    if corps.is_empty() {
        return Err("Aucune image".to_string());
    }
    if corps.len() > IMAGE_MAX {
        return Err("Image trop lourde (2 Mo maximum)".to_string());
    }
    Ok(base64::engine::general_purpose::STANDARD.encode(corps))
}

/// POST /api/kyc/live/face?session=… — metrics du visage pour le guide temps reel.
pub async fn suivre_visage(req: HttpRequest, corps: web::Bytes) -> HttpResponse {
    if crate::auth_guard::exiger_auth(&req).is_err() {
        return HttpResponse::Unauthorized().json(json!({
            "status": "error", "message": "Authentification requise"
        }));
    }
    let query = extraire_query(&req);
    let brut = match image_brute(&corps) {
        Ok(brut) => brut,
        Err(message) => return HttpResponse::BadRequest().json(json!({"status":"error","message":message})),
    };
    let session = query.get("session").cloned().unwrap_or_default();
    let resultat = web::block(move || moteur("face", &json!({ "img": brut, "session": session }))).await;
    reponse(aplatir(resultat))
}

/// POST /api/kyc/live/doc?session=…&warp=1 — quad du document + qualite de l'image.
pub async fn suivre_document(req: HttpRequest, corps: web::Bytes) -> HttpResponse {
    if crate::auth_guard::exiger_auth(&req).is_err() {
        return HttpResponse::Unauthorized().json(json!({
            "status": "error", "message": "Authentification requise"
        }));
    }
    let query = extraire_query(&req);
    let session = match query.get("session") {
        Some(valeur) if !valeur.is_empty() => valeur.clone(),
        _ => return HttpResponse::BadRequest().json(json!({
            "status": "error", "code": "SESSION_REQUISE", "message": "Session de guidage requise"
        })),
    };
    if session_en_cours(&session).is_none() {
        return HttpResponse::BadRequest().json(json!({
            "status": "error", "code": "SESSION_EXPIREE",
            "message": "Session expiree : relance la verification"
        }));
    }
    let warp = query.get("warp").map(|v| v == "1").unwrap_or(false);
    let brut = match image_brute(&corps) {
        Ok(brut) => brut,
        Err(message) => return HttpResponse::BadRequest().json(json!({"status":"error","message":message})),
    };
    let resultat = web::block(move || moteur("doc", &json!({ "img": brut, "warp": warp }))).await;
    reponse(aplatir(resultat))
}

#[derive(serde::Deserialize)]
pub struct Sequence {
    pub session: String,
    pub frames: Vec<String>,
}

/// POST /api/kyc/live/check — la sequence a-t-elle vraiment contenu le defi ?
pub async fn verifier_sequence(req: HttpRequest, corps: web::Json<Sequence>) -> HttpResponse {
    if crate::auth_guard::exiger_auth(&req).is_err() {
        return HttpResponse::Unauthorized().json(json!({
            "status": "error", "message": "Authentification requise"
        }));
    }
    let corps = corps.into_inner();
    if corps.frames.is_empty() || corps.frames.len() > SEQUENCE_MAX {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "message": format!("Sequence invalide : entre 1 et {} images", SEQUENCE_MAX)
        }));
    }
    let empreinte = match session_en_cours(&corps.session) {
        Some(empreinte) => empreinte,
        None => {
            return HttpResponse::BadRequest().json(json!({
                "status": "error", "code": "SESSION_EXPIREE",
                "message": "Session expirée : recommence la vivacité"
            }))
        }
    };
    // Une session ne sert qu'une fois : sans cela, le même défi filmé une fois
    // pourrait etre représenté image par image autant qu'il le souhaite.
    marquer_utilisee(&corps.session);

    let args = json!({
        "frames": corps.frames.iter().map(|b64| json!({ "img": b64 })).collect::<Vec<Value>>(),
        "template": empreinte.gabarit,
        "ratio": empreinte.ratio,
        "rect": empreinte.rect,
    });
    let attendant = (corps.session.clone(), empreinte.uq.clone(), empreinte.nonce.clone());
    let resultat = web::block(move || moteur("check", &args).map(|data| (data, attendant))).await;
    match resultat {
        Ok(Ok((mut data, (session, uq, nonce)))) => {
            data["session"] = json!(session);
            data["nonce"] = json!(nonce);
            // Le bilan est garde ici, pas dans l'application : le dossier ne pourra
            // citer que ce que le moteur a vraiment mesure.
            enregistrer_bilan(&session, &uq, &data);
            HttpResponse::Ok().json(json!({ "status": "success", "data": data }))
        }
        Ok(Err(message)) => {
            HttpResponse::BadRequest().json(json!({ "status": "error", "message": message }))
        }
        Err(erreur) => HttpResponse::BadRequest().json(json!({
            "status": "error", "message": erreur.to_string()
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonce_sans_ambiguite() {
        for _ in 0..50 {
            let nonce = nonce_aleatoire(6);
            assert_eq!(nonce.len(), 6);
            assert!(!nonce.chars().any(|c| matches!(c, 'I' | 'O' | '0' | '1')));
            assert!(nonce.chars().all(|c| NONCE_ALPHABET.contains(&(c as u8))));
        }
    }

    #[test]
    fn image_acceptee_selon_sa_taille() {
        assert!(image_brute(&[]).is_err());
        assert!(image_brute(&vec![7u8; IMAGE_MAX + 1]).is_err());
        assert!(image_brute(&vec![7u8; 128]).is_ok());
    }

    #[test]
    fn delai_sequence_plus_long_que_le_guide() {
        // Huit images a controlevent : le budget du guide temps réel ne suffit pas.
        assert!(delai_pour("check") > delai_pour("face"));
        assert_eq!(delai_pour("doc"), DELAI_GUIDE);
    }

    #[test]
    fn session_inconnue_refuse_le_controle() {
        assert!(session_en_cours("kyc_inexistente").is_none());
    }

    #[test]
    fn session_utilisee_ne_repasse_plus() {
        let maintenant = chrono::Utc::now().timestamp();
        table_sessions().insert(
            "kyc_test_une_fois".to_string(),
            Session {
                uq: "UQ-00000001".to_string(),
                nonce: "ABC234".to_string(),
                cree_le: maintenant,
                gabarit: "Zm9v".to_string(),
                ratio: 0.22,
                rect: json!({ "cx": 0.5, "cy": 0.82, "w": 0.88 }),
                utilisee: true,
            },
        );
        assert!(session_en_cours("kyc_test_une_fois").is_none());
        marquer_utilisee("kyc_test_une_fois");
    }

    #[test]
    fn session_trop_ancienne_est_refusee() {
        let tres_ancienne = chrono::Utc::now().timestamp() - VIE_SESSION_SECS - 10;
        table_sessions().insert(
            "kyc_test_expiree".to_string(),
            Session {
                uq: "UQ-00000002".to_string(),
                nonce: "XYZ789".to_string(),
                cree_le: tres_ancienne,
                gabarit: "Zm9v".to_string(),
                ratio: 0.22,
                rect: json!(null),
                utilisee: false,
            },
        );
        assert!(session_en_cours("kyc_test_expiree").is_none());
    }

    #[test]
    fn bilan_attache_le_bon_compte() {
        enregistrer_bilan(
            "kyc_test_bilan",
            "UQ-00000003",
            &json!({ "ok": true, "nonce_ratio": 1.0 }),
        );
        // Le dossier d'un autre compte ne peut pas reclamer ce bilan.
        assert!(valider_session("kyc_test_bilan", "UQ-00000004").is_none());
        let bilan = valider_session("kyc_test_bilan", "UQ-00000003").expect("bilan attendu");
        assert_eq!(bilan["ok"], json!(true));
    }

    #[test]
    fn bilan_presque_obsolete_refuse() {
        let garde = BILANS.get_or_init(|| Mutex::new(HashMap::new()));
        let age = chrono::Utc::now().timestamp() - VIE_BILAN_SECS - 5;
        garde.lock().unwrap().insert(
            "kyc_test_vieux".to_string(),
            Bilan { uq: "UQ-00000005".to_string(), horodatage: age, data: json!({ "ok": true }) },
        );
        assert!(valider_session("kyc_test_vieux", "UQ-00000005").is_none());
    }
}
