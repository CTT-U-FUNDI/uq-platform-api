// ====================
// 🌍 SOURCES — Collecte indépendante des taux par source externe
// ====================
//
// Chaque adaptateur rend des taux RAMENÉS À USD :
//   devise X  → taux = combien d'unités de X pour 1 USD
//   actif Y   → taux = combien d'Y pour 1 USD (prix 77 000 USD/BTC → 1/77 000)
//
// Règles :
//   - une source ne fournit JAMAIS un taux inventé : en cas d'échec on
//     retourne une erreur, et c'est l'agrégation qui décide du repli
//   - les sources payantes sont désactivées tant qu'aucune clé n'est fournie
//     (aucune clé en dur dans le code)
//   - les timeouts sont calibrés sur les latences MESURÉES en conditions
//     réelles (p50 2,5-9,5 s ; p95 5,8-15,9 s). Un timeout trop serré fait
//     retomber en repli à chaque tick — c'est ce qui faisait servir
//     CDF 2800 au lieu de CDF 2309 en direct.
// ====================

use lazy_static::lazy_static;
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;

/// Catégorie d'une source : sert à choisir le badge de confiance côté affichage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Categorie {
    Forex,
    Or,
    Crypto,
    /// Cours officiel d'une banque centrale (BCC — République Démocratique du Congo).
    BanqueCentrale,
}

impl Categorie {
    pub fn cle(&self) -> &'static str {
        match self {
            Categorie::Forex => "forex",
            Categorie::Or => "or",
            Categorie::Crypto => "crypto",
            Categorie::BanqueCentrale => "banque_centrale",
        }
    }
}

#[derive(Debug, Clone)]
pub enum ErreurCollecte {
    Reseau(String),
    Http(u16),
    Format(String),
    /// Clé requise et absente : la source est exclue de l'agrégat.
    SansCle,
}

impl ErreurCollecte {
    pub fn cle(&self) -> &'static str {
        match self {
            ErreurCollecte::Reseau(_) => "reseau",
            ErreurCollecte::Http(_) => "http",
            ErreurCollecte::Format(_) => "format",
            ErreurCollecte::SansCle => "sans_cle",
        }
    }
}

/// Ce qu'une source a rendu pour un tick.
#[derive(Debug, Clone)]
pub struct ReponseSource {
    pub source: &'static str,
    pub categorie: Categorie,
    pub taux: HashMap<String, f64>,
    pub latence_ms: u64,
    pub recolle_a: i64,
}

/// Paire d'actifs pour les sources qui interrogent une devise à la fois
/// (Coinbase, Binance). La devise s'obtient en retirant le compteur d'USD.
pub struct Paire {
    pub actif: &'static str,
    pub paire: &'static str,
    pub url: &'static str,
}

pub struct SourceDef {
    pub nom: &'static str,
    pub categorie: Categorie,
    pub url: &'static str,
    /// Variable d'environnement tenant la clé. `None` = source gratuite.
    pub cle_env: Option<&'static str>,
    pub timeout: Duration,
    /// `paire` = identifiant demandé (`BTC-USD`, `BTCUSDT`), vide pour les
    /// sources qui rendent tout l'éventail en une requête.
    pub collecteur: fn(&Value, &str) -> Result<HashMap<String, f64>, ErreurCollecte>,
    /// Sources à un point par actif : une requête par paire.
    pub paires: Option<&'static [Paire]>,
    /// Période de rafraîchissement propre : solliciter CoinGecko ou l'or
    /// chaque minute ne sert à rien et épuise les quotas.
    pub rythme_secs: u64,
    /// `false` = adaptateur écrit mais non collecté (couvert par une autre
    /// source gratuite, ou coût de requêtes prohibitif).
    pub activee: bool,
}

fn secondes(s: u64) -> Duration {
    Duration::from_secs(s)
}

// ====================
// LECTEURS
// ====================

/// Objet `{ code: taux }` où le taux est déjà « unités par USD ».
fn lire_brut_pour_usd(
    v: &Value,
    chemin: &[&str],
) -> Result<HashMap<String, f64>, ErreurCollecte> {
    let mut nœud = v;
    for cle in chemin {
        nœud = nœud.get(*cle).ok_or_else(|| ErreurCollecte::Format("chemin absent".into()))?;
    }
    let obj = nœud
        .as_object()
        .ok_or_else(|| ErreurCollecte::Format("objet de taux absent".into()))?;
    let mut out = HashMap::new();
    for (code, val) in obj {
        if let Some(t) = val.as_f64() {
            if t > 0.0 && t.is_finite() {
                out.insert(code.to_uppercase(), t);
            }
        }
    }
    if out.is_empty() {
        return Err(ErreurCollecte::Format("aucun taux numérique".into()));
    }
    Ok(out)
}

/// Objet `{ code: prix_en_usd }` → unités par USD.
fn prix_en_usd_vers_unites(brut: HashMap<String, f64>) -> HashMap<String, f64> {
    brut
        .into_iter()
        .filter(|(_, p)| *p > 0.0)
        .map(|(c, p)| (c, 1.0 / p))
        .collect()
}

fn lire_objet_en_usd(v: &Value, chemin: &[&str]) -> Result<HashMap<String, f64>, ErreurCollecte> {
    lire_brut_pour_usd(v, chemin).map(prix_en_usd_vers_unites)
}

fn lire_er_api(v: &Value, _paire: &str) -> Result<HashMap<String, f64>, ErreurCollecte> {
    if v.get("result").and_then(Value::as_str) != Some("success") {
        return Err(ErreurCollecte::Format("result != success".into()));
    }
    lire_brut_pour_usd(v, &["rates"])
}

/// Frankfurter (BCE) : devise de base explicite, et le CDF n'est PAS publié
/// (liste BCE ≈ 29 majors) — la source ne peut donc pas couvrir CDF.
fn lire_frankfurter(v: &Value, _paire: &str) -> Result<HashMap<String, f64>, ErreurCollecte> {
    if v.get("base").and_then(Value::as_str) != Some("USD") {
        return Err(ErreurCollecte::Format("base non USD".into()));
    }
    lire_brut_pour_usd(v, &["rates"])
}

/// allratestoday / BCC : cours officiel quotidien. On ne retient que le
/// « middle » USD→CDF (buy/sell sont des fourchettes de comptoir).
fn lire_bcc(v: &Value, _paire: &str) -> Result<HashMap<String, f64>, ErreurCollecte> {
    let lignes = v
        .get("rates")
        .and_then(Value::as_array)
        .ok_or_else(|| ErreurCollecte::Format("rates absent".into()))?;
    let chaine = |l: &Value, cles: &[&str]| -> Option<String> {
        cles.iter()
            .find_map(|c| l.get(*c).and_then(Value::as_str))
            .map(|s| s.to_uppercase())
    };
    let mut out = HashMap::new();
    for l in lignes {
        let base = chaine(l, &["base", "base_ccy"]);
        let devise = chaine(l, &["quote", "quote_ccy"]);
        let type_taux = chaine(l, &["type", "rate_type"]);
        if base.as_deref() == Some("USD")
            && devise.as_deref() == Some("CDF")
            && type_taux.as_deref() == Some("MIDDLE")
        {
            if let Some(t) = l.get("value").and_then(Value::as_f64) {
                if t > 0.0 && t.is_finite() {
                    out.insert("CDF".to_string(), t);
                }
            }
        }
    }
    if out.is_empty() {
        return Err(ErreurCollecte::Format("aucune ligne middle USD/CDF".into()));
    }
    Ok(out)
}

/// biquote : citation d'une paire, symbole concaténé (`https://biquote.io/api/XAUUSD` ;
/// `XAU` seul renvoie 404 — c'était le bug de la version précédente).
///
/// La réponse est plate (`bid`, `ask`, `mid`, `stale`, `quoteAgeSeconds`) et le
/// niveau gratuit renvoie des citations déjà vieilles — mesuré : `stale: true`
/// avec 66 355 s d'âge, marché fermé. Le dateur de la réponse ne suffit donc
/// pas : c'est l'âge déclaré par le fournisseur qui décide. Une citation de la
/// veille n'est pas un taux en direct, et l'injecter dans la médiane
/// falsifierait l'agrégat.
fn lire_biquote(v: &Value, _paire: &str) -> Result<HashMap<String, f64>, ErreurCollecte> {
    let age_declare = v.get("quoteAgeSeconds").and_then(Value::as_f64).unwrap_or(0.0);
    if v.get("stale").and_then(Value::as_bool).unwrap_or(false) || age_declare > 900.0 {
        return Err(ErreurCollecte::Format(format!(
            "citation agee de {} s (stale)",
            age_declare.round()
        )));
    }

    // Le nom de la devise vient de la réponse, pas d'une table : XAUUSD -> XAU.
    let symbole = v.get("symbol").and_then(Value::as_str).unwrap_or("");
    let devise = symbole
        .strip_suffix("USD")
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "XAU".to_string());

    let prix_usd = ["mid", "bid", "last"]
        .iter()
        .filter_map(|cle| v.get(*cle).and_then(Value::as_f64))
        .find(|p| *p > 0.0)
        .ok_or_else(|| ErreurCollecte::Format("aucun prix positif".into()))?;

    let mut out = HashMap::new();
    out.insert(devise, 1.0 / prix_usd);
    Ok(out)
}

fn lire_xaus(v: &Value, _paire: &str) -> Result<HashMap<String, f64>, ErreurCollecte> {
    let mut out = HashMap::new();
    for (cle, code) in [
        ("spot_usd_oz", "XAU"),
        ("silver_usd_oz", "XAG"),
        ("btc_usd", "BTC"),
    ] {
        if let Some(p) = v.get(cle).and_then(Value::as_f64) {
            if p > 0.0 {
                out.insert(code.to_string(), 1.0 / p);
            }
        }
    }
    if out.is_empty() {
        return Err(ErreurCollecte::Format("aucun spot".into()));
    }
    Ok(out)
}

const EQUIVALENCES_COINGECKO: &[(&str, &str)] = &[
    ("bitcoin", "BTC"),
    ("ethereum", "ETH"),
    ("tether", "USDT"),
    ("usd-coin", "USDC"),
    ("binancecoin", "BNB"),
    ("ripple", "XRP"),
    ("solana", "SOL"),
    ("cardano", "ADA"),
    ("dogecoin", "DOGE"),
    ("tron", "TRX"),
    ("polkadot", "DOT"),
    ("litecoin", "LTC"),
    ("avalanche-2", "AVAX"),
    ("chainlink", "LINK"),
    ("cosmos", "ATOM"),
    ("stellar", "XLM"),
    ("monero", "XMR"),
];

fn lire_coin_gecko(v: &Value, _paire: &str) -> Result<HashMap<String, f64>, ErreurCollecte> {
    let obj = v
        .as_object()
        .ok_or_else(|| ErreurCollecte::Format("racine non objet".into()))?;
    let mut out = HashMap::new();
    for (id, code) in EQUIVALENCES_COINGECKO {
        if let Some(p) = obj.get(*id).and_then(|e| e.get("usd")).and_then(Value::as_f64) {
            if p > 0.0 {
                out.insert((*code).to_string(), 1.0 / p);
            }
        }
    }
    if out.is_empty() {
        return Err(ErreurCollecte::Format("aucun prix usd".into()));
    }
    Ok(out)
}

/// { "data": { "amount": "77338.675" } } — Coinbase, une paire par requête.
fn lire_coinbase(v: &Value, paire: &str) -> Result<HashMap<String, f64>, ErreurCollecte> {
    let amount = v
        .get("data")
        .and_then(|d| d.get("amount"))
        .and_then(Value::as_str)
        .ok_or_else(|| ErreurCollecte::Format("data.amount absent".into()))?;
    let p: f64 = amount
        .parse()
        .map_err(|_| ErreurCollecte::Format("montant invalide".into()))?;
    if p <= 0.0 {
        return Err(ErreurCollecte::Format("montant nul".into()));
    }
    // « BTC-USD » → « BTC » : le code d'actif vient de la paire demandée.
    let actif = paire.split('-').next().unwrap_or(paire).to_uppercase();
    if actif.is_empty() {
        return Err(ErreurCollecte::Format("paire sans actif".into()));
    }
    Ok(HashMap::from([(actif, 1.0 / p)]))
}

/// { "symbol": "BTCUSDT", "price": "77000.00" } — Binance, une paire par requête.
fn lire_binance(v: &Value, _paire: &str) -> Result<HashMap<String, f64>, ErreurCollecte> {
    let symbole = v
        .get("symbol")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_uppercase();
    let actif = symbole
        .strip_suffix("USDT")
        .or_else(|| symbole.strip_suffix("USDC"))
        .or_else(|| symbole.strip_suffix("USD"))
        .unwrap_or(&symbole)
        .to_string();
    let p: f64 = v
        .get("price")
        .and_then(Value::as_str)
        .ok_or_else(|| ErreurCollecte::Format("price absent".into()))?
        .parse()
        .map_err(|_| ErreurCollecte::Format("price invalide".into()))?;
    if p <= 0.0 || actif.is_empty() {
        return Err(ErreurCollecte::Format("symbole ou prix invalide".into()));
    }
    Ok(HashMap::from([(actif, 1.0 / p)]))
}

/// { "data": { "BTC": { "code": "BTC", "price": "77000" } } } — CoinCap v3.
fn lire_coin_cap(v: &Value, _paire: &str) -> Result<HashMap<String, f64>, ErreurCollecte> {
    let obj = v
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(|| ErreurCollecte::Format("data absent".into()))?;
    let mut out = HashMap::new();
    for (code, entree) in obj {
        if let Some(p) = entree.get("price").and_then(Value::as_str).and_then(|s| s.parse::<f64>().ok()) {
            if p > 0.0 {
                out.insert(code.to_uppercase(), 1.0 / p);
            }
        }
    }
    if out.is_empty() {
        return Err(ErreurCollecte::Format("aucun prix".into()));
    }
    Ok(out)
}

fn cle_environnement(cle: &str) -> Option<String> {
    std::env::var(cle)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

// ====================
// PAIRES COTÉES ACTIF PAR ACTIF
// ====================

const ACTIFS_CRYPTOS: &[(&str, &str)] = &[
    ("BTC", "BTC"),
    ("ETH", "ETH"),
    ("BNB", "BNB"),
    ("SOL", "SOL"),
    ("XRP", "XRP"),
    ("ADA", "ADA"),
    ("DOGE", "DOGE"),
    ("TRX", "TRX"),
    ("DOT", "DOT"),
    ("LINK", "LINK"),
    ("LTC", "LTC"),
    ("AVAX", "AVAX"),
    ("ATOM", "ATOM"),
    ("XLM", "XLM"),
    ("XMR", "XMR"),
];

lazy_static! {
    static ref PAIRES_COINBASE: Vec<Paire> = ACTIFS_CRYPTOS
        .iter()
        .map(|(actif, _)| Paire {
            actif,
            paire: Box::leak(format!("{}-USD", actif).into_boxed_str()),
            url: Box::leak(
                format!("https://api.coinbase.com/v2/prices/{}-USD/spot", actif).into_boxed_str(),
            ),
        })
        .collect();

    static ref PAIRES_BINANCE: Vec<Paire> = ACTIFS_CRYPTOS
        .iter()
        .map(|(actif, _)| Paire {
            actif,
            paire: Box::leak(format!("{}USDT", actif).into_boxed_str()),
            url: Box::leak(
                format!("https://api.binance.com/api/v3/ticker/price?symbol={}USDT", actif)
                    .into_boxed_str(),
            ),
        })
        .collect();

    static ref SOURCES: Vec<SourceDef> = vec![
        // ---------- Forex (6) ----------
        SourceDef {
            nom: "er-api",
            categorie: Categorie::Forex,
            url: "https://open.er-api.com/v6/latest/USD",
            cle_env: None,
            timeout: secondes(10),
            collecteur: lire_er_api,
            paires: None,
            rythme_secs: 60,
            activee: true,
        },
        SourceDef {
            nom: "frankfurter",
            categorie: Categorie::Forex,
            url: "https://api.frankfurter.dev/v1/latest?base=USD",
            cle_env: None,
            timeout: secondes(10),
            collecteur: lire_frankfurter,
            paires: None,
            rythme_secs: 60,
            activee: true,
        },
        SourceDef {
            nom: "currencyapi",
            categorie: Categorie::Forex,
            url: "https://api.currencyapi.com/v3/latest",
            cle_env: Some("UQ_RATE_KEY_CURRENCYAPI"),
            timeout: secondes(10),
            collecteur: |v: &Value, _: &str| lire_brut_pour_usd(v, &["data"]),
            paires: None,
            rythme_secs: 600,
            activee: true,
        },
        SourceDef {
            nom: "fixer",
            categorie: Categorie::Forex,
            url: "https://data.fixer.io/api/latest",
            cle_env: Some("UQ_RATE_KEY_FIXER"),
            timeout: secondes(10),
            collecteur: |v: &Value, _: &str| lire_brut_pour_usd(v, &["rates"]),
            paires: None,
            rythme_secs: 600,
            activee: true,
        },
        SourceDef {
            nom: "exchangerate-host",
            categorie: Categorie::Forex,
            url: "https://api.exchangerate.host/latest?base=USD",
            cle_env: Some("UQ_RATE_KEY_EXCHANGEHOST"),
            timeout: secondes(10),
            collecteur: |v: &Value, _: &str| lire_brut_pour_usd(v, &["rates"]),
            paires: None,
            rythme_secs: 600,
            activee: true,
        },
        SourceDef {
            nom: "open-exchange-rates",
            categorie: Categorie::Forex,
            url: "https://api.openexchangerates.org/latest.json",
            cle_env: Some("UQ_RATE_KEY_OXR"),
            timeout: secondes(10),
            collecteur: |v: &Value, _: &str| lire_brut_pour_usd(v, &["rates"]),
            paires: None,
            rythme_secs: 600,
            activee: true,
        },
        // ---------- Or (3) ----------
        SourceDef {
            nom: "xaus",
            categorie: Categorie::Or,
            url: "https://xaus.com/api/v1/spot",
            cle_env: None,
            timeout: secondes(10),
            collecteur: lire_xaus,
            paires: None,
            rythme_secs: 300,
            activee: true,
        },
        SourceDef {
            nom: "biquote",
            categorie: Categorie::Or,
            url: "https://biquote.io/api/XAUUSD",
            cle_env: None,
            timeout: secondes(12),
            collecteur: lire_biquote,
            paires: None,
            rythme_secs: 300,
            activee: true,
        },
        SourceDef {
            nom: "goldapi",
            categorie: Categorie::Or,
            url: "https://www.goldapi.io/api/XAU/USD",
            cle_env: Some("UQ_RATE_KEY_GOLDAPI"),
            timeout: secondes(10),
            // goldapi rend { "price_gram_24k": ... } ou { "open_price_aud": .. }
            // selon le plan ; on lit le champ prix en USD par once.
            collecteur: |v: &Value, _: &str| {
                let p = v
                    .get("price_ounce_usd")
                    .or_else(|| v.get("price"))
                    .and_then(Value::as_f64)
                    .ok_or_else(|| ErreurCollecte::Format("prix or absent".into()))?;
                if p <= 0.0 {
                    return Err(ErreurCollecte::Format("prix or nul".into()));
                }
                Ok(HashMap::from([("XAU".to_string(), 1.0 / p)]))
            },
            paires: None,
            rythme_secs: 300,
            activee: true,
        },
        // ---------- Crypto (2 + 2 en cascade) ----------
        SourceDef {
            nom: "coingecko",
            categorie: Categorie::Crypto,
            url: "https://api.coingecko.com/api/v3/simple/price?ids=bitcoin,ethereum,binancecoin,solana,ripple,cardano,dogecoin,tron,polkadot,chainlink,litecoin,avalanche-2,cosmos,stellar,monero,tether,usd-coin&vs_currencies=usd",
            cle_env: None,
            timeout: secondes(15),
            collecteur: lire_coin_gecko,
            paires: None,
            rythme_secs: 120,
            activee: true,
        },
        SourceDef {
            nom: "coincap",
            categorie: Categorie::Crypto,
            url: "https://rest.coincap.io/v3/assets?search=bitcoin",
            cle_env: Some("UQ_RATE_KEY_COINCAP"),
            timeout: secondes(15),
            collecteur: lire_coin_cap,
            paires: None,
            rythme_secs: 300,
            activee: true,
        },
        SourceDef {
            nom: "coinbase",
            categorie: Categorie::Crypto,
            url: "",
            cle_env: None,
            timeout: secondes(10),
            collecteur: lire_coinbase,
            paires: Some(&PAIRES_COINBASE),
            rythme_secs: 120,
            activee: false,
        },
        SourceDef {
            nom: "binance",
            categorie: Categorie::Crypto,
            url: "",
            cle_env: None,
            timeout: secondes(10),
            collecteur: lire_binance,
            paires: Some(&PAIRES_BINANCE),
            rythme_secs: 60,
            activee: false,
        },
        // ---------- Banque centrale (1) ----------
        SourceDef {
            nom: "bcc",
            categorie: Categorie::BanqueCentrale,
            url: "https://allratestoday.com/api/open/central-bank/bcc",
            cle_env: None,
            timeout: secondes(10),
            collecteur: lire_bcc,
            paires: None,
            rythme_secs: 3600,
            activee: true,
        },
    ];
}

pub fn sources() -> &'static [SourceDef] {
    &SOURCES
}

/// Une source est active si elle est gratuite, ou si sa clé est fournie.
pub fn source_active(def: &SourceDef) -> bool {
    match def.cle_env {
        None => true,
        Some(cle) => cle_environnement(cle).is_some(),
    }
}

fn entete_auth(def: &SourceDef) -> Option<(&'static str, String)> {
    let valeur = def.cle_env.and_then(cle_environnement)?;
    match def.nom {
        "goldapi" => Some(("x-access-token", valeur)),
        "currencyapi" | "coincap" => Some(("apikey", valeur)),
        // fixer / open-exchange-rates / exchangerate-host passent par l'URL
        _ => None,
    }
}

/// Certaines clés se transmettent en paramètre d'URL plutôt qu'en en-tête.
fn url_pour(def: &SourceDef, base: &str) -> String {
    let cle = def.cle_env.and_then(cle_environnement);
    match def.nom {
        "fixer" | "open-exchange-rates" | "exchangerate-host" => match cle {
            Some(c) if base.contains('?') => format!("{}&access_key={}", base, c),
            Some(c) => format!("{}?access_key={}", base, c),
            None => base.to_string(),
        },
        _ => base.to_string(),
    }
}

// ====================
// COLLECTE
// ====================

/// Une tentative : requête + lecture JSON + adaptateur.
async fn tenter(
    client: &reqwest::Client,
    def: &SourceDef,
    url: &str,
    paire: &str,
) -> Result<HashMap<String, f64>, ErreurCollecte> {
    let mut requete = client.get(url).timeout(def.timeout);
    if let Some((nom, valeur)) = entete_auth(def) {
        requete = requete.header(nom, valeur);
    }
    let resp = requete
        .send()
        .await
        .map_err(|e| ErreurCollecte::Reseau(e.to_string()))?;
    let statut = resp.status().as_u16();
    if statut != 200 {
        return Err(ErreurCollecte::Http(statut));
    }
    let v = resp
        .json::<Value>()
        .await
        .map_err(|e| ErreurCollecte::Format(e.to_string()))?;
    (def.collecteur)(&v, paire)
}

/// Le retry ne couvre que les pannes réseau et les 5xx : un 401 ou un JSON
/// malformé est une erreur de lecture, pas une incident passager.
fn merite_retry(erreur: &ErreurCollecte) -> bool {
    match erreur {
        ErreurCollecte::Reseau(_) => true,
        ErreurCollecte::Http(s) => *s >= 500,
        _ => false,
    }
}

async fn avec_retry(
    client: &reqwest::Client,
    def: &SourceDef,
    url: &str,
    paire: &str,
) -> Result<HashMap<String, f64>, ErreurCollecte> {
    let mut derniere = ErreurCollecte::Reseau("aucune tentative".into());
    for tentative in 0..3u32 {
        if tentative > 0 {
            let base = 400u64 * (1u64 << (tentative - 1));
            let jitter: u64 = {
                use rand::Rng;
                rand::thread_rng().gen_range(0..250)
            };
            tokio::time::sleep(Duration::from_millis(base + jitter)).await;
        }
        let issue = tenter(client, def, url, paire).await;
        match issue {
            Ok(taux) => return Ok(taux),
            Err(e) => {
                if !merite_retry(&e) {
                    return Err(e);
                }
                derniere = e;
            }
        }
    }
    Err(derniere)
}

/// Collecte une source complète. Pour les sources à paires, une seule paire
/// qui répond suffit : la réponse agrège ce qui a pu être lu.
pub async fn collecter(
    client: &reqwest::Client,
    def: &'static SourceDef,
) -> Result<ReponseSource, ErreurCollecte> {
    if !source_active(def) {
        return Err(ErreurCollecte::SansCle);
    }
    let debut = std::time::Instant::now();
    let mut taux: HashMap<String, f64> = HashMap::new();

    match def.paires {
        None => {
            taux = avec_retry(client, def, &url_pour(def, def.url), "").await?;
        }
        Some(paires) => {
            let mut reussies = 0usize;
            for p in paires {
                if let Ok(u) = avec_retry(client, def, &url_pour(def, p.url), p.paire).await {
                    taux.extend(u);
                    reussies += 1;
                }
            }
            if reussies == 0 {
                return Err(ErreurCollecte::Reseau(format!(
                    "0/{} paires répondent",
                    paires.len()
                )));
            }
        }
    }

    if taux.is_empty() {
        return Err(ErreurCollecte::Format("aucun taux exploitable".into()));
    }

    Ok(ReponseSource {
        source: def.nom,
        categorie: def.categorie,
        taux,
        latence_ms: debut.elapsed().as_millis() as u64,
        recolle_a: chrono::Utc::now().timestamp(),
    })
}
