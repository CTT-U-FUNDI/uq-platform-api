// ====================
// VERIF — Numéro de téléphone réel, codes de vérification, porte anti-robot
// ====================
//
// Trois services demandes par la page de creation et l'ecran de connexion :
//   1. /api/phone/check  — le numero saisi est-il un vrai numero (indicatif et
//      longueur d'abonne attribuables) ? 100 % local, sans cle, sans reseau.
//   2. /api/verify/*     — code a usage unique envoye sur le telephone de
//      recuperation. Le fournisseur SMS est ramable par SMS_PROVIDER_URL +
//      SMS_PROVIDER_API_KEY ; sans cle la verification est marquee « reportee »
//      et ne bloque jamais la creation du compte.
//   3. /api/human/*      — porte qui precede l'ouverture du menu de l'ecran de
//      connexion. Cloudflare Turnstile des que les cles sont posees, sinon
//      puzzle visuel a jeton unique et signaux d'interaction
//      facultatifs (HUMAN_REQUIRE_TURNSTILE=1 pour exiger Turnstile).
//      Le puzzle se rend en 3 echecs maximum : le jeton est brule et le client
//      est verrouille 5 minutes ; ses demandes de defi neuf sont plafonnees a
//      5 par fenetre de 15 minutes, sinon « 3 echecs » ne coutait rien.
//
// Securite appliquee ici :
//   - aucun code n'est renvoye a l'ecran ni journalise en clair ;
//   - comparaison uniquement via hash SHA-256 sale par le secret serveur ;
//   - jetons a usage unique, consommes a la creation du compte ;
//   - debit plafonne par cible, tentatives comptees, expiration courte.
// ====================

use actix_web::{web, HttpRequest, HttpResponse};
use rand::seq::SliceRandom;
use rand::Rng;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

// ====================
// CONSTANTES
// ====================

const CODE_TTL_SECS: i64 = 600;
const CODE_ENVOIS_MAX_PAR_CIBLE: usize = 3;
const CODE_TENTATIVES_MAX: i32 = 5;
pub const JETON_TTL_SECS: i64 = 1800;
const HUMAIN_TTL_SECS: i64 = 300;
const HUMAIN_DUREE_MIN_SECS: i64 = 1;
const HUMAIN_TENTATIVES_MAX: i32 = 3;
/// Fenêtre de verrouillage appliquée à un client qui a épuisé ses tentatives.
const HUMAIN_PENALITE_SECS: i64 = 300;
/// Nombre de défis neufs accordés à un même client sur HUMAIN_DEBUTS_FENETRE_SECS.
const HUMAIN_DEBUTS_MAX: usize = 5;
const HUMAIN_DEBUTS_FENETRE_SECS: i64 = 900;
const DEFIS_MAX: usize = 4096;
const SYMBOLES_HUMAINS: [&str; 6] = ["cercle", "triangle", "carre", "losange", "etoile", "coeur"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Portee {
    Telephone,
    Email,
}

impl Portee {
    pub fn as_str(self) -> &'static str {
        match self {
            Portee::Telephone => "telephone",
            Portee::Email => "email",
        }
    }
}

#[derive(Debug)]
struct Defis {
    map: HashMap<String, Defie>,
    envois: HashMap<String, Vec<i64>>,
    /// Horodatages des défis neufs demandés, par empreinte client : sans ce
    /// plafond, « tentatives épuisées » ne veut rien dire, le bot redemande un
    /// jeton et repart à zéro.
    debuts_humains: HashMap<String, Vec<i64>>,
    /// Empreinte client -> epoch de fin de verrouillage de la porte.
    penalites_humains: HashMap<String, i64>,
    /// Preuves de possession delivrees quand un code est confirme : c'est elles
    /// que la creation de compte et les contacts de recuperation consomment.
    /// Le defie d'origine est marque consomme a ce moment-la, il ne peut plus
    /// servir de preuve.
    preuves: HashMap<String, Preuve>,
}

#[derive(Debug, Clone)]
struct Preuve {
    cible: String,
    portee: Portee,
    expire_at: i64,
}

#[derive(Debug, Clone)]
struct Defie {
    hash_code: String,
    expire_at: i64,
    tentatives: i32,
    consomme: bool,
    cible: String,
    portee: Portee,
    human_puzzle: Option<HumanPuzzle>,
}

#[derive(Debug, Clone)]
struct HumanPuzzle {
    cible: String,
    symboles: Vec<String>,
    choix_attendu: usize,
    /// Scène animée : objets numérotés qui se mélangent ~2s.
    /// choix_attendu = le NUMÉRO attendu (10-99), pas un index.
    scene: Option<ScenePuzzle>,
}

#[derive(Debug, Clone)]
struct ScenePuzzle {
    /// Numéros uniques affichés sur chaque objet (index = position).
    numeros: Vec<u16>,
    /// Index de l'objet ciblé par la question.
    cible_idx: usize,
}

fn defis() -> &'static Mutex<Defis> {
    static DEFIS: OnceLock<Mutex<Defis>> = OnceLock::new();
    DEFIS.get_or_init(|| {
        Mutex::new(Defis {
            map: HashMap::new(),
            envois: HashMap::new(),
            debuts_humains: HashMap::new(),
            penalites_humains: HashMap::new(),
            preuves: HashMap::new(),
        })
    })
}

fn lock<T>(verrou: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match verrou.lock() {
        Ok(garde) => garde,
        Err(e) => e.into_inner(),
    }
}

fn maintenant() -> i64 {
    chrono::Utc::now().timestamp()
}

fn hex_aleatoire(n_octets: usize) -> String {
    let mut octets = vec![0u8; n_octets];
    rand::thread_rng().fill(&mut octets[..]);
    octets.iter().map(|o| format!("{:02x}", o)).collect()
}

fn hasher(valeur: &str) -> String {
    let mut h = Sha256::new();
    h.update(valeur.as_bytes());
    // Un sel rendu : le vol de la table ne livre pas les codes.
    if let Ok(sel) = std::env::var("JWT_SECRET") {
        h.update(sel.as_bytes());
    }
    format!("{:x}", h.finalize())
}

fn purger(defis: &mut Defis) {
    let maintenant = maintenant();
    defis
        .map
        .retain(|_, d| d.expire_at > maintenant && !d.consomme);
    if defis.map.len() > DEFIS_MAX {
        // On garde les plus recents : la table ne doit pas grossir sans fin.
        let mut ordonne: Vec<(String, i64)> = defis
            .map
            .iter()
            .map(|(k, d)| (k.clone(), d.expire_at))
            .collect();
        ordonne.sort_by_key(|(_, expire)| *expire);
        for (jeton, _) in ordonne.into_iter().take(defis.map.len() - DEFIS_MAX) {
            defis.map.remove(&jeton);
        }
    }
    defis.preuves.retain(|_, p| p.expire_at > maintenant);
    defis.debuts_humains.retain(|_, v| {
        v.iter()
            .any(|t| maintenant - *t < HUMAIN_DEBUTS_FENETRE_SECS)
    });
    defis.penalites_humains.retain(|_, fin| *fin > maintenant);
}

fn enregistrer_defie(jeton: &str, defie: Defie) {
    let mut garde = lock(defis());
    purger(&mut garde);
    garde.map.insert(jeton.to_string(), defie);
}

fn lire_defie(jeton: &str) -> Option<Defie> {
    lock(defis()).map.get(jeton).cloned()
}

fn marquer_consomme(jeton: &str) {
    if let Some(d) = lock(defis()).map.get_mut(jeton) {
        d.consomme = true;
    }
}

fn augmenter_tentatives(jeton: &str) {
    if let Some(defi) = lock(defis()).map.get_mut(jeton) {
        defi.tentatives += 1;
    }
}

// ====================
// PASS HUMAIN — preuve réutilisable à durée limitée
// ====================
// Une vérification réussie (scène ou Turnstile) émet un pass qui autorise
// les routes sensibles côté serveur : création de compte, routes 2FA.
// Le frontend ne peut pas se contenter d'afficher la scène : l'API exige le pass.

const PASS_TTL_SECS: i64 = 900; // 15 min — le formulaire de création est long

fn passes_humains() -> &'static Mutex<HashMap<String, i64>> {
    static P: OnceLock<Mutex<HashMap<String, i64>>> = OnceLock::new();
    P.get_or_init(|| Mutex::new(HashMap::new()))
}

fn emettre_pass() -> String {
    let pass = format!("hpass-{}", hex_aleatoire(16));
    if let Ok(mut p) = passes_humains().lock() {
        p.retain(|_, e| *e > maintenant());
        p.insert(pass.clone(), maintenant() + PASS_TTL_SECS);
    }
    pass
}

/// Vrai si le pass est présent et non expiré (sans le consommer —
/// pour les routes 2FA où une session peut enchaîner plusieurs appels).
pub fn preuve_humaine_valide(token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    passes_humains()
        .lock()
        .map(|p| p.get(token).map(|e| *e > maintenant()).unwrap_or(false))
        .unwrap_or(false)
}

/// Valide ET consomme le pass — usage unique (création de compte :
/// un humain = un compte, pas un passage illimité à 15 min).
pub fn consommer_preuve_humaine(token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    passes_humains()
        .lock()
        .map(|mut p| {
            let ok = p.get(token).map(|e| *e > maintenant()).unwrap_or(false);
            if ok {
                p.remove(token);
            }
            ok
        })
        .unwrap_or(false)
}

enum ResultatPuzzle {
    Correct,
    Incorrect,
    Limite,
    Expire,
    TropRapide,
}

fn verifier_et_consumer_puzzle(jeton: &str, choix: Option<usize>) -> ResultatPuzzle {
    let mut garde = lock(defis());
    let Some(defi) = garde.map.get_mut(jeton) else {
        return ResultatPuzzle::Expire;
    };
    if defi.consomme || defi.expire_at <= maintenant() {
        return ResultatPuzzle::Expire;
    }
    let Some(puzzle) = defi.human_puzzle.as_ref() else {
        return ResultatPuzzle::Expire;
    };
    if defi.tentatives >= HUMAIN_TENTATIVES_MAX {
        defi.consomme = true;
        return ResultatPuzzle::Limite;
    }
    if choix != Some(puzzle.choix_attendu) {
        defi.tentatives += 1;
        if defi.tentatives >= HUMAIN_TENTATIVES_MAX {
            // Le jeton est brule et le client est mis en penalite : sans ca, il
            // suffirait de redemander un defi pour repartir a zero.
            defi.consomme = true;
            return ResultatPuzzle::Limite;
        }
        return ResultatPuzzle::Incorrect;
    }
    if maintenant() - (defi.expire_at - HUMAIN_TTL_SECS) < HUMAIN_DUREE_MIN_SECS {
        return ResultatPuzzle::TropRapide;
    }
    defi.consomme = true;
    ResultatPuzzle::Correct
}

/// Vrai si la cible a encore droit a un envoi (plafond par periode de vie du code).
fn quota_respecte(empreinte: &str) -> bool {
    let mut garde = lock(defis());
    let maintenant = maintenant();
    let entrees = garde.envois.entry(empreinte.to_string()).or_default();
    entrees.retain(|t| maintenant - *t < CODE_TTL_SECS);
    if entrees.len() >= CODE_ENVOIS_MAX_PAR_CIBLE {
        return false;
    }
    entrees.push(maintenant);
    if garde.envois.len() > DEFIS_MAX {
        garde
            .envois
            .retain(|_, v| v.iter().any(|t| maintenant - *t < CODE_TTL_SECS));
    }
    true
}

/// Empreinte du client : IP de la connexion, sinon la premiere entree de
/// X-Forwarded-For quand l'app tourne derriere un proxy.
fn empreinte_client(req: &HttpRequest) -> String {
    req.headers()
        .get("X-Forwarded-For")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .or_else(|| req.peer_addr().map(|a| a.ip().to_string()))
        .unwrap_or_else(|| "inconnu".to_string())
}

/// Epoch de fin de verrouillage tant que le client reste sous penalite.
fn penalite_humaine(empreinte: &str) -> Option<i64> {
    let mut garde = lock(defis());
    let fin = garde.penalites_humains.get(empreinte).copied();
    match fin {
        Some(fin) if fin > maintenant() => Some(fin),
        Some(_) => {
            garde.penalites_humains.remove(empreinte);
            None
        }
        None => None,
    }
}

fn reponse_penalite(fin: i64) -> HttpResponse {
    HttpResponse::TooManyRequests().json(json!({
        "status": "error",
        "message": "Trop d'echecs. Nouvelle verification possible dans quelques minutes.",
        "raison": "penalite",
        "deblocage_a": fin,
        "secondes_restantes": (fin - maintenant()).max(0),
    }))
}

/// Enregistre une demande de defi neuf et renvoie les secondes a attendre si le
/// client a deja consomme son quota sur la fenetre glissante.
fn consommer_quota_humain(empreinte: &str) -> Option<i64> {
    let mut garde = lock(defis());
    let maintenant = maintenant();
    let entrees = garde
        .debuts_humains
        .entry(empreinte.to_string())
        .or_default();
    entrees.retain(|t| maintenant - *t < HUMAIN_DEBUTS_FENETRE_SECS);
    if entrees.len() >= HUMAIN_DEBUTS_MAX {
        let plus_ancienne = entrees.iter().copied().min().unwrap_or(maintenant);
        return Some((plus_ancienne + HUMAIN_DEBUTS_FENETRE_SECS - maintenant).max(1));
    }
    entrees.push(maintenant);
    if garde.debuts_humains.len() > DEFIS_MAX {
        garde.debuts_humains.retain(|_, v| {
            v.iter()
                .any(|t| maintenant - *t < HUMAIN_DEBUTS_FENETRE_SECS)
        });
    }
    None
}

fn infliger_penalite_humaine(empreinte: &str) -> i64 {
    let fin = maintenant() + HUMAIN_PENALITE_SECS;
    lock(defis())
        .penalites_humains
        .insert(empreinte.to_string(), fin);
    fin
}

// ====================
// 1. PLAN DE NUMEROTATION
// ====================
struct Regle {
    indicatif: &'static str,
    iso: &'static str,
    pays: &'static str,
    /// Longueurs d'abonne mobiles admises (apres indicatif et zero national).
    longueurs: &'static [usize],
    /// Prefixes d'abonne -> operateur, du plus court au plus long.
    operateurs: &'static [(&'static str, &'static str)],
}

const REGLES: &[Regle] = &[
    Regle {
        indicatif: "243",
        iso: "CD",
        pays: "RD Congo",
        longueurs: &[9, 10],
        operateurs: &[
            ("89", "Airtel"),
            ("91", "Airtel"),
            ("97", "Airtel"),
            ("85", "Vodacom"),
            ("90", "Vodacom"),
            ("84", "Orange"),
            ("890", "Orange"),
            ("891", "Orange"),
            ("892", "Orange"),
            ("899", "Orange"),
            ("970", "Africell"),
            ("971", "Africell"),
            ("972", "Africell"),
        ],
    },
    Regle {
        indicatif: "242",
        iso: "CG",
        pays: "Congo-Brazzaville",
        longueurs: &[8, 9],
        operateurs: &[("05", "Airtel"), ("06", "Airtel"), ("22", "MTN")],
    },
    Regle {
        indicatif: "225",
        iso: "CI",
        pays: "Côte d’Ivoire",
        longueurs: &[8, 10],
        operateurs: &[("01", "Orange"), ("05", "MTN"), ("07", "Moov")],
    },
    Regle {
        indicatif: "221",
        iso: "SN",
        pays: "Sénégal",
        longueurs: &[9],
        operateurs: &[
            ("70", "Free"),
            ("76", "Expresso"),
            ("77", "Orange"),
            ("78", "Free"),
        ],
    },
    Regle {
        indicatif: "226",
        iso: "BF",
        pays: "Burkina Faso",
        longueurs: &[8],
        operateurs: &[
            ("70", "Orange"),
            ("74", "FasoNet"),
            ("75", "Vodafone"),
            ("76", "Moov"),
        ],
    },
    Regle {
        indicatif: "223",
        iso: "ML",
        pays: "Mali",
        longueurs: &[8],
        operateurs: &[("76", "Orange"), ("94", "Malitel"), ("65", "Orange")],
    },
    Regle {
        indicatif: "228",
        iso: "TG",
        pays: "Togo",
        longueurs: &[7, 8],
        operateurs: &[
            ("90", "Togocom"),
            ("91", "Togocom"),
            ("92", "Moov"),
            ("93", "Moov"),
        ],
    },
    Regle {
        indicatif: "250",
        iso: "RW",
        pays: "Rwanda",
        longueurs: &[9],
        operateurs: &[("72", "Airtel"), ("73", "MTN"), ("78", "Telikom")],
    },
    Regle {
        indicatif: "257",
        iso: "BI",
        pays: "Burundi",
        longueurs: &[8],
        operateurs: &[
            ("29", "Onatel"),
            ("61", "Smart"),
            ("68", "Lumitel"),
            ("79", "Econet"),
        ],
    },
    Regle {
        indicatif: "241",
        iso: "GA",
        pays: "Gabon",
        longueurs: &[8, 9],
        operateurs: &[("06", "Airtel"), ("07", "Moov"), ("77", "Airtel")],
    },
    Regle {
        indicatif: "237",
        iso: "CM",
        pays: "Cameroun",
        longueurs: &[9],
        operateurs: &[("65", "Orange"), ("67", "MTN")],
    },
    Regle {
        indicatif: "229",
        iso: "BJ",
        pays: "Bénin",
        longueurs: &[9],
        operateurs: &[
            ("90", "MTN"),
            ("95", "Moov"),
            ("96", "Etisalat"),
            ("97", "BBCOM"),
        ],
    },
    Regle {
        indicatif: "258",
        iso: "MZ",
        pays: "Mozambique",
        longueurs: &[9],
        operateurs: &[
            ("82", "Movitel"),
            ("84", "Vodacom"),
            ("86", "Tmcel"),
            ("87", "Movitel"),
        ],
    },
    Regle {
        indicatif: "261",
        iso: "MG",
        pays: "Madagascar",
        longueurs: &[9],
        operateurs: &[
            ("32", "Orange"),
            ("33", "Telima"),
            ("34", "MTN"),
            ("37", "Airtel"),
            ("38", "Bluel"),
        ],
    },
    Regle {
        indicatif: "255",
        iso: "TZ",
        pays: "Tanzanie",
        longueurs: &[9],
        operateurs: &[
            ("65", "Tigo"),
            ("66", "Vodacom"),
            ("67", "Airtel"),
            ("68", "Saliti"),
            ("69", "Helius"),
        ],
    },
    Regle {
        indicatif: "254",
        iso: "KE",
        pays: "Kenya",
        longueurs: &[9],
        operateurs: &[
            ("70", "Safaricom"),
            ("71", "Safaricom"),
            ("72", "Safaricom"),
            ("73", "Safaricom"),
            ("74", "Airtel"),
            ("75", "Equitel"),
            ("76", "Airtel"),
        ],
    },
    Regle {
        indicatif: "256",
        iso: "UG",
        pays: "Ouganda",
        longueurs: &[9],
        operateurs: &[
            ("70", "Airtel"),
            ("74", "MTN"),
            ("75", "Vodafone"),
            ("77", "MTN"),
            ("78", "Airtel"),
        ],
    },
    Regle {
        indicatif: "27",
        iso: "ZA",
        pays: "Afrique du Sud",
        longueurs: &[9],
        operateurs: &[
            ("60", "MTN"),
            ("61", "Vodacom"),
            ("63", "Vodacom"),
            ("64", "Cell C"),
            ("65", "Geonet"),
            ("66", "Vodacom"),
            ("70", "Cell C"),
            ("71", "BMW"),
            ("72", "Vodacom"),
            ("73", "MTN"),
            ("74", "Rain"),
            ("76", "Rain"),
            ("78", "Glo"),
            ("82", "MTN"),
            ("83", "Orange"),
        ],
    },
    Regle {
        indicatif: "234",
        iso: "NG",
        pays: "Nigéria",
        longueurs: &[10],
        operateurs: &[
            ("70", "Airtel"),
            ("80", "MTN"),
            ("81", "MTN"),
            ("90", "Glo"),
        ],
    },
    Regle {
        indicatif: "238",
        iso: "CV",
        pays: "Cap-Vert",
        longueurs: &[9],
        operateurs: &[("9", "Vodacom")],
    },
    Regle {
        indicatif: "33",
        iso: "FR",
        pays: "France",
        longueurs: &[9],
        operateurs: &[("6", "Mobile"), ("7", "Mobile")],
    },
    Regle {
        indicatif: "32",
        iso: "BE",
        pays: "Belgique",
        longueurs: &[9],
        operateurs: &[("4", "Mobile")],
    },
    Regle {
        indicatif: "41",
        iso: "CH",
        pays: "Suisse",
        longueurs: &[9],
        operateurs: &[("7", "Mobile")],
    },
    Regle {
        indicatif: "1",
        iso: "CA",
        pays: "Canada",
        longueurs: &[10],
        operateurs: &[],
    },
];

pub struct NumeroValide {
    pub e164: String,
    pub iso: String,
    pub pays: String,
    pub operateur: Option<String>,
}

/// Reconnait un numero reel : indicatif attribue + longueur d'abonne possible.
/// Rend `None` sinon — c'est le seul controle faisable sans fournisseur payant.
pub fn analyser_telephone(saisie: &str) -> Option<NumeroValide> {
    let chiffre = |c: char| c.is_ascii_digit();
    let brut: String = saisie.chars().filter(|c| chiffre(*c)).collect();
    if brut.len() < 8 || brut.len() > 15 {
        return None;
    }

    // Formats acceptes : +243…, 00243…, 243…, et la saisie locale (0…).
    let base = if let Some(sans_00) = brut.strip_prefix("00") {
        sans_00.to_string()
    } else {
        brut.clone()
    };

    let mut candidats: Vec<String> = Vec::new();
    candidats.push(base.clone());
    for regle in REGLES.iter() {
        if let Some(abonne) = base.strip_prefix(regle.indicatif) {
            // Le zero national devant l'abonne est courant (089… = +24389…).
            if let Some(sans_zero) = abonne.strip_prefix('0') {
                candidats.push(format!("{}{}", regle.indicatif, sans_zero));
            }
            break;
        }
    }
    // Saisie purement locale : « 0890123456 » doit tomber sur +243890123456.
    if let Some(abonne) = base.strip_prefix('0') {
        candidats.push(format!("243{}", abonne));
    }

    let mut meilleur: Option<(usize, NumeroValide)> = None;
    for candidat in candidats.iter().map(|c| c.as_str()) {
        for regle in REGLES.iter() {
            let abonne = match candidat.strip_prefix(regle.indicatif) {
                Some(a) => a,
                None => continue,
            };
            if !regle.longueurs.contains(&abonne.len()) {
                continue;
            }
            let valide = NumeroValide {
                e164: format!("+{}{}", regle.indicatif, abonne),
                iso: regle.iso.to_string(),
                pays: regle.pays.to_string(),
                operateur: operateur_de(regle, abonne).map(|o| o.to_string()),
            };
            // Indicatif le plus long d'abord : « 243 » passe avant « 24 ».
            let poids = regle.indicatif.len();
            if meilleur.as_ref().map(|(p, _)| poids >= *p).unwrap_or(true) {
                meilleur = Some((poids, valide));
            }
        }
    }
    meilleur.map(|(_, n)| n)
}

fn operateur_de(regle: &Regle, abonne: &str) -> Option<&'static str> {
    // Du prefixe le plus long au plus court : « 890 » (Orange) doit gagner
    // devant « 89 » (Airtel).
    let mut prefixes: Vec<(&str, &str)> = regle.operateurs.iter().copied().collect();
    prefixes.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
    prefixes
        .into_iter()
        .find(|(p, _)| abonne.starts_with(p))
        .map(|(_, nom)| nom)
}

// ====================
// 2. /api/phone/check
// ====================

pub async fn phone_check(body: web::Json<serde_json::Value>) -> HttpResponse {
    let saisie = body.get("telephone").and_then(|v| v.as_str()).unwrap_or("");
    match analyser_telephone(saisie) {
        Some(n) => HttpResponse::Ok().json(json!({
            "status": "success",
            "valide": true,
            "e164": n.e164,
            "iso": n.iso,
            "pays": n.pays,
            "operateur": n.operateur,
        })),
        None => HttpResponse::Ok().json(json!({
            "status": "success",
            "valide": false,
            "raison": "indicatif_ou_longueur_inconnu",
            "e164": null,
            "operateur": null,
        })),
    }
}

// ====================
// 3. /api/verify/*
// ====================

fn email_valide(valeur: &str) -> bool {
    let v = valeur.trim();
    if v.is_empty() || v.len() > 254 || !v.chars().all(|c| !c.is_whitespace()) {
        return false;
    }
    match v.split_once('@') {
        Some((local, domaine)) => {
            local.len() >= 2 && domaine.contains('.') && !domaine.starts_with('.')
        }
        None => false,
    }
}

/// Variante publique utilisee par la creation de compte.
pub fn email_valide_pub(valeur: &str) -> bool {
    email_valide(valeur)
}

fn valeur_valide(portee: Portee, valeur: &str) -> bool {
    match portee {
        Portee::Telephone => analyser_telephone(valeur).is_some(),
        Portee::Email => email_valide(valeur),
    }
}

/// Envoie le code chez le fournisseur SMS configure.
/// Rend Ok(false) quand aucune cle n'est poseee : verification reportee.
async fn envoyer_sms(cible: &str, code: &str) -> Result<bool, String> {
    let url = std::env::var("SMS_PROVIDER_URL").unwrap_or_default();
    let cle = std::env::var("SMS_PROVIDER_API_KEY").unwrap_or_default();
    if url.trim().is_empty() || cle.trim().is_empty() {
        return Ok(false);
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|_| "client_http".to_string())?;

    let payload = json!({
        "to": cible,
        "message": format!(
            "UQ Platform : ton code de verification est {}. Il expire dans 10 minutes.",
            code
        ),
        "sender": std::env::var("SMS_SENDER").unwrap_or_else(|_| "UQ".to_string()),
        "api_key": cle,
    });

    let reponse = client
        .post(url.trim())
        .json(&payload)
        .send()
        .await
        .map_err(|_| "reseau".to_string())?;

    let statut = reponse.status();
    let corps: serde_json::Value = reponse.json().await.unwrap_or_default();
    if !statut.is_success() {
        for cle_reponse in ["detail", "message", "error", "reason"] {
            if let Some(descripteur) = corps.get(cle_reponse).and_then(|v| v.as_str()) {
                return Err(descripteur.to_string());
            }
        }
        return Err("fournisseur".to_string());
    }
    Ok(true)
}

pub async fn verify_challenge(body: web::Json<serde_json::Value>) -> HttpResponse {
    let portee = match body
        .get("portee")
        .and_then(|v| v.as_str())
        .unwrap_or("telephone")
    {
        "email" => Portee::Email,
        "telephone" => Portee::Telephone,
        autre => {
            return HttpResponse::BadRequest().json(err("Portee inconnue", Some(autre)));
        }
    };
    let saisie = body
        .get("valeur")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();

    if !valeur_valide(portee, saisie) {
        return HttpResponse::BadRequest().json(err(
            match portee {
                Portee::Telephone => "Ce numéro n'existe pas ou est incomplet.",
                Portee::Email => "Adresse email invalide.",
            },
            Some(match portee {
                Portee::Telephone => "numero_inconnu",
                Portee::Email => "email_invalide",
            }),
        ));
    }

    let cible = match portee {
        Portee::Telephone => analyser_telephone(saisie)
            .map(|n| n.e164)
            .unwrap_or_default(),
        Portee::Email => saisie.to_lowercase(),
    };

    let empreinte = format!("{}:{}", portee.as_str(), hasher(&cible));
    if !quota_respecte(&empreinte) {
        return HttpResponse::TooManyRequests().json(err(
            "Trop de demandes. Réessaie dans quelques minutes.",
            Some("quota"),
        ));
    }

    let code: String = (0..6)
        .map(|_| rand::thread_rng().gen_range(0..10).to_string())
        .collect();
    let jeton = hex_aleatoire(16);

    enregistrer_defie(
        &jeton,
        Defie {
            hash_code: hasher(&code),
            expire_at: maintenant() + CODE_TTL_SECS,
            tentatives: 0,
            consomme: false,
            cible: cible.clone(),
            portee,
            human_puzzle: None,
        },
    );

    let envoi = match portee {
        // L'envoi d'email n'a pas encore de fournisseur configure : la porte
        // est preparee, le champ reste « reportee ».
        Portee::Email => Ok(false),
        Portee::Telephone => envoyer_sms(&cible, &code).await,
    };

    match envoi {
        Ok(reel) => HttpResponse::Ok().json(json!({
            "status": "success",
            "jeton": jeton,
            "portee": portee.as_str(),
            "cible": masque(&cible, portee),
            "envoye": reel,
            "mode": if reel { "reel" } else { "reportee" },
            "expire_at": maintenant() + CODE_TTL_SECS,
            "message": if reel {
                "Code envoyé au numéro indiqué."
            } else {
                "Vérification reportée : le numéro est enregistré, à confirmer depuis ton profil."
            },
        })),
        Err(raison) => HttpResponse::Ok().json(json!({
            "status": "success",
            "jeton": jeton,
            "portee": portee.as_str(),
            "cible": masque(&cible, portee),
            "envoye": false,
            "mode": "reportee",
            "raison": raison,
            "expire_at": maintenant() + CODE_TTL_SECS,
            "message": "Vérification reportée : le réseau SMS n'a pas répondu.",
        })),
    }
}

fn masque(cible: &str, portee: Portee) -> String {
    match portee {
        Portee::Telephone => {
            let caracteres: Vec<char> = cible.chars().collect();
            if caracteres.len() < 7 {
                return "••••".to_string();
            }
            let debut: String = caracteres[..4].iter().collect();
            let fin: String = caracteres[caracteres.len() - 2..].iter().collect();
            format!("{}••••{}", debut, fin)
        }
        Portee::Email => match cible.split_once('@') {
            Some((local, domaine)) => {
                let garde: String = local.chars().take(2).collect();
                format!("{}•••@{}", garde, domaine)
            }
            None => "•••".to_string(),
        },
    }
}

pub async fn verify_confirm(body: web::Json<serde_json::Value>) -> HttpResponse {
    let jeton = body.get("jeton").and_then(|v| v.as_str()).unwrap_or("");
    let saisi: String = body
        .get("code")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .chars()
        .filter(|c| c.is_ascii_digit())
        .collect();

    let defie = match lire_defie(jeton) {
        Some(d) if !d.consomme && d.expire_at > maintenant() => d,
        _ => {
            return HttpResponse::BadRequest().json(err("Code expiré ou inconnu.", Some("expire")))
        }
    };

    if defie.tentatives >= CODE_TENTATIVES_MAX {
        marquer_consomme(jeton);
        return HttpResponse::TooManyRequests().json(err(
            "Trop de tentatives. Demande un nouveau code.",
            Some("tentatives"),
        ));
    }

    if saisi.len() != 6 || hasher(&saisi) != defie.hash_code {
        augmenter_tentatives(jeton);
        return HttpResponse::Unauthorized().json(err("Code incorrect.", Some("code")));
    }

    marquer_consomme(jeton);
    // Le defie est brule : ce qui reste a presenter au backend, c'est cette
    // preuve a usage unique. Elle porte la cible verifiee et son portee.
    let preuve = delivrer_preuve(&defie.cible, defie.portee);
    HttpResponse::Ok().json(json!({
        "status": "success",
        "verifie": true,
        "portee": defie.portee.as_str(),
        "valeur": defie.cible,
        "jeton_verifie": preuve,
        "expire_at": maintenant() + JETON_TTL_SECS,
    }))
}

/// Enregistre la reussite d'un code et rend le jeton qui la represente.
fn delivrer_preuve(cible: &str, portee: Portee) -> String {
    let preuve = hex_aleatoire(16);
    let mut garde = lock(defis());
    purger(&mut garde);
    garde.preuves.insert(
        preuve.clone(),
        Preuve {
            cible: cible.to_string(),
            portee,
            expire_at: maintenant() + JETON_TTL_SECS,
        },
    );
    preuve
}

/// Retombe la cible d'une verification reussie, et consomme la preuve.
pub fn valeur_verifiee(jeton: &str, portee: Portee) -> Option<String> {
    let mut garde = lock(defis());
    let valide = garde
        .preuves
        .get(jeton)
        .filter(|p| p.expire_at > maintenant() && p.portee == portee)
        .map(|p| p.cible.clone());
    if valide.is_some() {
        // Usage unique : la preuve ne survit pas a la creation du compte.
        garde.preuves.remove(jeton);
    }
    valide
}

// ====================
// 4. /api/human/* — porte anti-robot
// ====================

fn turnstile_cles() -> Option<(String, String)> {
    let site = std::env::var("TURNSTILE_SITE_KEY")
        .unwrap_or_default()
        .trim()
        .to_string();
    let secret = std::env::var("TURNSTILE_SECRET_KEY")
        .unwrap_or_default()
        .trim()
        .to_string();
    if site.is_empty() || secret.is_empty() {
        return None;
    }
    Some((site, secret))
}

/// En production, exiger Turnstile et rien d'autre.
fn exige_turnstile() -> bool {
    std::env::var("HUMAN_REQUIRE_TURNSTILE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

pub async fn human_config() -> HttpResponse {
    let cles = turnstile_cles();
    let mode = match (&cles, exige_turnstile()) {
        (Some(_), _) => "turnstile",
        (None, true) => "indisponible",
        (None, false) => "visuel_local",
    };
    HttpResponse::Ok().json(json!({
        "status": "success",
        "mode": mode,
        "site_key": cles.map(|(site, _)| site),
    }))
}

pub async fn human_begin(req: HttpRequest) -> HttpResponse {
    if exige_turnstile() && turnstile_cles().is_none() {
        return HttpResponse::ServiceUnavailable().json(err(
            "Vérification indisponible pour le moment.",
            Some("fournisseur_absent"),
        ));
    }
    let empreinte = empreinte_client(&req);
    if let Some(fin) = penalite_humaine(&empreinte) {
        return reponse_penalite(fin);
    }
    if let Some(attente) = consommer_quota_humain(&empreinte) {
        return HttpResponse::TooManyRequests().json(json!({
            "status": "error",
            "message": "Trop de vérifications demandées. Réessaie dans quelques minutes.",
            "raison": "debit",
            "secondes_restantes": attente,
        }));
    }
    let defi = hex_aleatoire(24);
    // ?type=scene force la scène animée (clients qui n'implémentent que
    // ce rendu — ex. création de compte, appairage device).
    let forcer_scene = req.query_string().split('&').any(|p| p == "type=scene");
    let puzzle = if forcer_scene {
        generer_puzzle_scene(&mut rand::thread_rng())
    } else {
        generer_puzzle_humain()
    };
    enregistrer_defie(
        &defi,
        Defie {
            hash_code: String::new(),
            expire_at: maintenant() + HUMAIN_TTL_SECS,
            tentatives: 0,
            consomme: false,
            cible: "humain".to_string(),
            portee: Portee::Email,
            human_puzzle: Some(puzzle.clone()),
        },
    );
    let mut reponse = json!({
        "status": "success",
        "jeton": defi,
        "cible": puzzle.cible,
        "symboles": puzzle.symboles,
        "expire_at": maintenant() + HUMAIN_TTL_SECS,
    });
    if let Some(scene) = &puzzle.scene {
        reponse["mode"] = json!("scene");
        reponse["objets"] = json!(scene.numeros);
        reponse["cible_idx"] = json!(scene.cible_idx);
    } else {
        reponse["mode"] = json!("symboles");
    }
    HttpResponse::Ok().json(reponse)
}

fn generer_puzzle_humain() -> HumanPuzzle {
    let mut rng = rand::thread_rng();
    // ~65% scène animée (objets numérotés en mouvement), ~35% symboles.
    if rng.gen_bool(0.65) {
        return generer_puzzle_scene(&mut rng);
    }
    generer_puzzle_symboles(&mut rng)
}

fn generer_puzzle_symboles(rng: &mut impl rand::Rng) -> HumanPuzzle {
    let cible = SYMBOLES_HUMAINS[rng.gen_range(0..SYMBOLES_HUMAINS.len())].to_string();
    let mut symboles: Vec<String> = SYMBOLES_HUMAINS
        .iter()
        .map(|symbole| (*symbole).to_string())
        .collect();
    symboles.shuffle(rng);
    let choix_attendu = symboles
        .iter()
        .position(|symbole| symbole == &cible)
        .expect("le symbole cible est toujours dans les choix");
    HumanPuzzle {
        cible,
        symboles,
        choix_attendu,
        scene: None,
    }
}

/// Scène : 5 objets avec numéros uniques (10-99). La question demande le
/// numéro de l'objet à `cible_idx` — le client renvoie le numéro lu.
fn generer_puzzle_scene(rng: &mut impl rand::Rng) -> HumanPuzzle {
    use rand::seq::SliceRandom;
    // 5 numéros distincts entre 10 et 99
    let mut numeros: Vec<u16> = (10..=99).collect();
    numeros.shuffle(rng);
    numeros.truncate(5);
    let cible_idx = rng.gen_range(0..numeros.len());
    let cible_numero = numeros[cible_idx];
    HumanPuzzle {
        cible: format!("objet:{}", cible_idx),
        symboles: Vec::new(),
        choix_attendu: cible_numero as usize,
        scene: Some(ScenePuzzle { numeros, cible_idx }),
    }
}

fn heuristique_interaction(interaction: Option<&serde_json::Value>) -> &'static str {
    let Some(interaction) = interaction else {
        return "indisponible";
    };
    let nombre = |cle: &str| interaction.get(cle).and_then(|v| v.as_u64()).unwrap_or(0);
    let mesure = |cle: &str| {
        interaction
            .get(cle)
            .and_then(|v| v.as_f64())
            .filter(|v| v.is_finite())
            .unwrap_or(0.0)
    };

    let pointeurs = nombre("pointer_samples");
    let chemin = mesure("path_length");
    let capteurs = nombre("motion_samples");
    let variance = mesure("motion_variance");
    if pointeurs > 512 || chemin > 100.0 || capteurs > 256 || variance > 10_000.0 {
        return "incoherent";
    }
    if pointeurs >= 3 && chemin >= 0.01 {
        return "mouvement_pointeur";
    }
    if capteurs >= 3 && (0.0001..=10_000.0).contains(&variance) {
        return "mouvement_capteur";
    }
    if interaction.get("input_type").and_then(|v| v.as_str()) == Some("keyboard") {
        return "clavier";
    }
    "insuffisant"
}

pub async fn human_verify(req: HttpRequest, body: web::Json<serde_json::Value>) -> HttpResponse {
    let empreinte = empreinte_client(&req);
    if let Some(fin) = penalite_humaine(&empreinte) {
        return reponse_penalite(fin);
    }
    // 1. Turnstile : des que les cles sont posees, c'est la seule porte.
    if let Some((_, secret)) = turnstile_cles() {
        let jeton = body.get("jeton").and_then(|v| v.as_str()).unwrap_or("");
        return verifier_turnstile(&secret, jeton).await;
    }
    if exige_turnstile() {
        return HttpResponse::ServiceUnavailable().json(err(
            "Vérification indisponible pour le moment.",
            Some("fournisseur_absent"),
        ));
    }

    // 2. Défi visuel local, validé côté serveur et à usage unique.
    let defi = body.get("jeton").and_then(|v| v.as_str()).unwrap_or("");
    let choix = body
        .get("choix")
        .and_then(|v| v.as_u64())
        .and_then(|v| usize::try_from(v).ok());
    let resultat = verifier_et_consumer_puzzle(defi, choix);
    match resultat {
        ResultatPuzzle::Incorrect => {
            return HttpResponse::Unauthorized().json(err(
                "Ce n'est pas le bon emplacement. Réessaie.",
                Some("choix"),
            ));
        }
        ResultatPuzzle::Limite => {
            let fin = infliger_penalite_humaine(&empreinte);
            return reponse_penalite(fin);
        }
        ResultatPuzzle::Expire => {
            return HttpResponse::BadRequest().json(err("Vérification expirée.", Some("expire")));
        }
        ResultatPuzzle::TropRapide => {
            return HttpResponse::BadRequest().json(err(
                "Prends un instant pour vérifier le symbole.",
                Some("temporisation"),
            ));
        }
        ResultatPuzzle::Correct => {}
    }

    HttpResponse::Ok().json(json!({
        "status": "success",
        "verification_reussie": true,
        "mode": "visuel_local",
        "pass": emettre_pass(),
        "indice_interaction": heuristique_interaction(body.get("interaction")),
        "expire_at": maintenant() + HUMAIN_TTL_SECS,
    }))
}

async fn verifier_turnstile(secret: &str, jeton: &str) -> HttpResponse {
    if jeton.trim().is_empty() {
        return HttpResponse::BadRequest()
            .json(err("Jeton de vérification manquant.", Some("jeton")));
    }

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(client) => client,
        Err(_) => {
            return HttpResponse::InternalServerError()
                .json(err("Erreur interne.", Some("client_http")))
        }
    };

    let requete = [
        ("secret", secret.to_string()),
        ("response", jeton.to_string()),
    ];

    let reponse = client
        .post("https://challenges.cloudflare.com/turnstile/v0/siteverify")
        .form(&requete)
        .send()
        .await;

    let reponse = match reponse {
        Ok(reponse) => reponse,
        Err(_) => {
            return HttpResponse::BadGateway().json(err(
                "Vérification indisponible pour le moment.",
                Some("reseau"),
            ))
        }
    };

    let corps: serde_json::Value = match reponse.json().await {
        Ok(corps) => corps,
        Err(_) => {
            return HttpResponse::BadGateway().json(err(
                "Vérification indisponible pour le moment.",
                Some("reponse"),
            ))
        }
    };

    if corps
        .get("success")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return HttpResponse::Ok().json(json!({
            "status": "success",
            "verification_reussie": true,
            "mode": "turnstile",
            "pass": emettre_pass(),
            "expire_at": maintenant() + HUMAIN_TTL_SECS,
        }));
    }

    HttpResponse::Unauthorized().json(err("Vérification échouée.", Some("turnstile_refuse")))
}

fn err(message: &str, raison: Option<&str>) -> serde_json::Value {
    let mut objet = serde_json::Map::new();
    objet.insert("status".into(), json!("error"));
    objet.insert("message".into(), json!(message));
    if let Some(raison) = raison {
        objet.insert("raison".into(), json!(raison));
    }
    serde_json::Value::Object(objet)
}

// ====================
// TESTS
// ====================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numero_rdc_format_international() {
        let n = analyser_telephone("+243 912 345 678").expect("numero valide");
        assert_eq!(n.e164, "+243912345678");
        assert_eq!(n.iso, "CD");
        assert_eq!(n.operateur.as_deref(), Some("Airtel"));
    }

    #[test]
    fn prefixe_long_gagne_sur_prefixe_court() {
        let n = analyser_telephone("00243890123456").expect("numero valide");
        assert_eq!(n.operateur.as_deref(), Some("Orange"));
    }

    #[test]
    fn saisie_locale_sans_indicatif() {
        let n = analyser_telephone("0890123456").expect("numero valide");
        assert_eq!(n.e164, "+243890123456");
    }

    #[test]
    fn abonne_trop_court_refuse() {
        assert!(analyser_telephone("+243810").is_none());
    }

    #[test]
    fn indicatif_inconnu_refuse() {
        assert!(analyser_telephone("+99912345678").is_none());
    }

    #[test]
    fn numero_kinois_a_10_chiffres() {
        let n = analyser_telephone("+243810123456").expect("numero valide");
        assert_eq!(n.iso, "CD");
    }

    #[test]
    fn puzzle_humain_expose_un_seul_choix_correct() {
        let puzzle = generer_puzzle_symboles(&mut rand::thread_rng());
        assert_eq!(puzzle.symboles.len(), SYMBOLES_HUMAINS.len());
        assert_eq!(puzzle.symboles[puzzle.choix_attendu], puzzle.cible);
        assert_eq!(
            puzzle
                .symboles
                .iter()
                .filter(|symbole| **symbole == puzzle.cible)
                .count(),
            1
        );
    }

    #[test]
    fn heuristique_interaction_reste_informative_sans_bloquer() {
        assert_eq!(heuristique_interaction(None), "indisponible");
        assert_eq!(
            heuristique_interaction(Some(&json!({"input_type":"keyboard"}))),
            "clavier"
        );
        assert_eq!(
            heuristique_interaction(Some(&json!({
                "pointer_samples": 12,
                "path_length": 0.4,
                "motion_samples": 0,
                "motion_variance": 0.0
            }))),
            "mouvement_pointeur"
        );
        assert_eq!(
            heuristique_interaction(Some(&json!({
                "pointer_samples": 900,
                "path_length": 0.4,
                "motion_samples": 0,
                "motion_variance": 0.0
            }))),
            "incoherent"
        );
    }

    #[test]
    fn jeton_visuel_est_a_usage_unique() {
        let jeton = format!("human-{}", hex_aleatoire(12));
        let puzzle = generer_puzzle_symboles(&mut rand::thread_rng());
        let choix_attendu = puzzle.choix_attendu;
        enregistrer_defie(
            &jeton,
            Defie {
                hash_code: String::new(),
                expire_at: maintenant() + HUMAIN_TTL_SECS,
                tentatives: 0,
                consomme: false,
                cible: "humain".to_string(),
                portee: Portee::Email,
                human_puzzle: Some(puzzle),
            },
        );
        std::thread::sleep(std::time::Duration::from_secs(HUMAIN_DUREE_MIN_SECS as u64));

        assert!(matches!(
            verifier_et_consumer_puzzle(&jeton, Some(choix_attendu)),
            ResultatPuzzle::Correct
        ));
        assert!(matches!(
            verifier_et_consumer_puzzle(&jeton, Some(choix_attendu)),
            ResultatPuzzle::Expire
        ));
    }

    #[test]
    fn trois_choix_incorrects_epuisent_le_jeton() {
        let jeton = format!("human-{}", hex_aleatoire(12));
        let puzzle = generer_puzzle_symboles(&mut rand::thread_rng());
        let choix_attendu = puzzle.choix_attendu;
        let choix_incorrect = (puzzle.choix_attendu + 1) % puzzle.symboles.len();
        enregistrer_defie(
            &jeton,
            Defie {
                hash_code: String::new(),
                expire_at: maintenant() + HUMAIN_TTL_SECS,
                tentatives: 0,
                consomme: false,
                cible: "humain".to_string(),
                portee: Portee::Email,
                human_puzzle: Some(puzzle),
            },
        );

        for _ in 0..(HUMAIN_TENTATIVES_MAX - 1) {
            assert!(matches!(
                verifier_et_consumer_puzzle(&jeton, Some(choix_incorrect)),
                ResultatPuzzle::Incorrect
            ));
        }
        // Le dernier echec est annonce comme tel : le client est verrouille.
        assert!(matches!(
            verifier_et_consumer_puzzle(&jeton, Some(choix_incorrect)),
            ResultatPuzzle::Limite
        ));
        // Le bon choix lui-meme ne rouvre plus rien : le jeton est brule.
        assert!(matches!(
            verifier_et_consumer_puzzle(&jeton, Some(choix_attendu)),
            ResultatPuzzle::Expire
        ));
    }

    #[test]
    fn penalite_humaine_verrouille_puis_se_releve() {
        let empreinte = format!("test-{}", hex_aleatoire(6));
        assert_eq!(penalite_humaine(&empreinte), None);
        let fin = infliger_penalite_humaine(&empreinte);
        assert_eq!(penalite_humaine(&empreinte), Some(fin));
        lock(defis()).penalites_humains.remove(&empreinte);
        assert_eq!(penalite_humaine(&empreinte), None);
    }

    #[test]
    fn quota_humain_plafonne_les_defis_neufs() {
        let empreinte = format!("test-{}", hex_aleatoire(6));
        for _ in 0..HUMAIN_DEBUTS_MAX {
            assert_eq!(consommer_quota_humain(&empreinte), None);
        }
        let attente = consommer_quota_humain(&empreinte);
        assert!(attente.is_some());
        assert!((1..=HUMAIN_DEBUTS_FENETRE_SECS).contains(&attente.unwrap()));
    }

    #[test]
    fn preuve_de_verification_est_a_usage_unique() {
        let jeton = delivrer_preuve("+243890123456", Portee::Telephone);
        assert_eq!(
            valeur_verifiee(&jeton, Portee::Telephone).as_deref(),
            Some("+243890123456")
        );
        assert_eq!(valeur_verifiee(&jeton, Portee::Telephone), None);

        let autre = delivrer_preuve("jean@gmail.com", Portee::Email);
        // Mauvaise portee : la preuve est conservee, pas brulee.
        assert_eq!(valeur_verifiee(&autre, Portee::Telephone), None);
        assert_eq!(
            valeur_verifiee(&autre, Portee::Email).as_deref(),
            Some("jean@gmail.com")
        );
    }

    #[test]
    fn masque_ne_livre_pas_la_cible() {
        assert_eq!(masque("+243890123456", Portee::Telephone), "+243••••56");
        assert_eq!(
            masque("jean.mukendi@gmail.com", Portee::Email),
            "je•••@gmail.com"
        );
    }

    #[test]
    fn quota_de_reenvoi_tient() {
        let empreinte = format!("telephone:test-{}", hex_aleatoire(4));
        assert!(quota_respecte(&empreinte));
        assert!(quota_respecte(&empreinte));
        assert!(quota_respecte(&empreinte));
        assert!(!quota_respecte(&empreinte));
    }

    #[test]
    fn adresses_email_acceptees_et_refusees() {
        assert!(email_valide("jean.mukendi@gmail.com"));
        assert!(!email_valide("jean@gmail"));
        assert!(!email_valide("j@gmail.com"));
        assert!(!email_valide("jean@ gmail.com"));
    }
}
