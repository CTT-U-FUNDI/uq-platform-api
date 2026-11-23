// ====================
// WEBAUTHN VERIFY — Verification cryptographique reelle des assertions
// ====================
//
// Ce module remplace le theatre precedent : le backend ne se contente plus
// d'ignorer la signature (`let _ = signature;`) pour reinventer un token.
// Il verifie reellement l'assertion de l'authentificateur.
//
// chemin crypto (CTAP2 / FIDO2) :
//   attestationObject (CBOR) -> authData -> rpIdHash | flags | counter
//                                            | aaguid | credIdLen | credId | COSE key
//   message signe   = authData || sha256(clientDataJSON)
//   signature       = ECDSA P-256 / SHA-256, encodage DER (cle COSE alg -7)
//
// L'enclave (vTEE / platform authenticator) detient la cle privee ; le serveur
// ne stocke que la cle publique SEC1 non compressee 0x04 || X || Y.

use ring::signature::{UnparsedPublicKey, ECDSA_P256_SHA256_ASN1};
use serde_json::Value;
use sha2::{Digest, Sha256};

// ===== Flags authData =====
pub const FLAG_UP: u8 = 0x01; // utilisateur present
pub const FLAG_UV: u8 = 0x04; // verification utilisateur (biometrie acceptee par l'enclave)
pub const FLAG_AT: u8 = 0x40; // donnees de credential presentes (attestation)
pub const FLAG_ED: u8 = 0x80; // extensions presentes

// ===== Garde-fous du parseur CBOR =====
const PROFONDEUR_MAX: usize = 8;
const ENTREES_MAX: usize = 4096;
const TAILLE_MAX: usize = 1 << 20;

// ====================
// base64url (tolerant : padding absent, '-'/'_' ou '+'/'/', sauts de ligne)
// ====================

pub fn decodage_base64url(texte: &str) -> Result<Vec<u8>, String> {
    let mut bits: u32 = 0;
    let mut nbits: u32 = 0;
    let mut out: Vec<u8> = Vec::with_capacity(texte.len() / 4 * 3 + 3);

    for c in texte.chars() {
        if c.is_ascii_whitespace() {
            continue;
        }
        if c == '=' {
            // la fin du payload est implicite : les padding sont ignores
            continue;
        }
        let v: u32 = match c {
            'A'..='Z' => (c as u32) - ('A' as u32),
            'a'..='z' => (c as u32) - ('a' as u32) + 26,
            '0'..='9' => (c as u32) - ('0' as u32) + 52,
            '-' | '+' => 62,
            '_' | '/' => 63,
            other => return Err(format!("caractere base64url invalide: {:?}", other)),
        };
        bits = (bits << 6) | v;
        nbits += 6;
        if nbits >= 8 {
            nbits -= 8;
            out.push(((bits >> nbits) & 0xFF) as u8);
        }
    }

    if nbits >= 6 || (nbits > 0 && ((bits & ((1 << nbits) - 1)) != 0)) {
        return Err("troncon base64url incomplet".to_string());
    }
    Ok(out)
}

const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

pub fn encodage_base64url(donnees: &[u8]) -> String {
    let mut out = String::with_capacity((donnees.len() + 2) / 3 * 4);
    let mut i = 0;
    while i + 3 <= donnees.len() {
        let n =
            ((donnees[i] as u32) << 16) | ((donnees[i + 1] as u32) << 8) | donnees[i + 2] as u32;
        out.push(ALPHABET[((n >> 18) & 63) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 63) as usize] as char);
        out.push(ALPHABET[((n >> 6) & 63) as usize] as char);
        out.push(ALPHABET[(n & 63) as usize] as char);
        i += 3;
    }
    match donnees.len() - i {
        1 => {
            let n = (donnees[i] as u32) << 16;
            out.push(ALPHABET[((n >> 18) & 63) as usize] as char);
            out.push(ALPHABET[((n >> 12) & 63) as usize] as char);
        }
        2 => {
            let n = ((donnees[i] as u32) << 16) | ((donnees[i + 1] as u32) << 8);
            out.push(ALPHABET[((n >> 18) & 63) as usize] as char);
            out.push(ALPHABET[((n >> 12) & 63) as usize] as char);
            out.push(ALPHABET[((n >> 6) & 63) as usize] as char);
        }
        _ => {}
    }
    out
}

// ====================
// Parseur CBOR minimal (suffisant pour attestationObject / COSE key)
// ====================

#[derive(Debug, Clone, PartialEq)]
pub enum Cbor {
    Positif(u64),
    Negatif(i128),
    Octets(Vec<u8>),
    Texte(String),
    Tableau(Vec<Cbor>),
    Carte(Vec<(Cbor, Cbor)>),
    Etiquette(u64, Box<Cbor>),
}

impl Cbor {
    /// Valeur d'une entree de carte dont la cle est le texte `cle`.
    pub fn cle_texte(&self, cle: &str) -> Option<&Cbor> {
        self.cle(&Cbor::Texte(cle.to_string()))
    }

    /// Valeur d'une entree de carte dont la cle est l'entier COSE `cle` (signe).
    pub fn cle_entier(&self, cle: i128) -> Option<&Cbor> {
        let cherche = if cle >= 0 {
            Cbor::Positif(cle as u64)
        } else {
            Cbor::Negatif(cle)
        };
        self.cle(&cherche)
    }

    fn cle(&self, cherche: &Cbor) -> Option<&Cbor> {
        match self {
            Cbor::Carte(entrees) => entrees.iter().find(|(k, _)| k == cherche).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn octets(&self) -> Option<&[u8]> {
        match self {
            Cbor::Octets(b) => Some(b),
            Cbor::Etiquette(_, inner) => inner.octets(),
            _ => None,
        }
    }

    pub fn texte(&self) -> Option<&str> {
        match self {
            Cbor::Texte(t) => Some(t),
            _ => None,
        }
    }
}

fn reading(d: &[u8], pos: usize, n: usize) -> Result<&[u8], String> {
    if n > TAILLE_MAX {
        return Err("troncon CBOR trop long".to_string());
    }
    d.get(pos..pos + n)
        .ok_or_else(|| "CBOR tronque".to_string())
}

fn en_tete(d: &[u8], pos: &mut usize) -> Result<(u8, u64), String> {
    let b = *d.get(*pos).ok_or_else(|| "CBOR tronque".to_string())?;
    *pos += 1;
    let major = b >> 5;
    let aux = b & 0x1F;
    let valeur = match aux {
        0..=23 => aux as u64,
        24 => {
            let s = reading(d, *pos, 1)?;
            *pos += 1;
            s[0] as u64
        }
        25 => {
            let s = reading(d, *pos, 2)?;
            *pos += 2;
            u16::from_be_bytes([s[0], s[1]]) as u64
        }
        26 => {
            let s = reading(d, *pos, 4)?;
            *pos += 4;
            u32::from_be_bytes([s[0], s[1], s[2], s[3]]) as u64
        }
        27 => {
            let s = reading(d, *pos, 8)?;
            *pos += 8;
            u64::from_be_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]])
        }
        31 => return Err("longueurs CBOR indefinies non acceptees".to_string()),
        other => return Err(format!("entete CBOR invalide (additional {})", other)),
    };
    Ok((major, valeur))
}

fn lire_valeur(d: &[u8], pos: &mut usize, profondeur: usize) -> Result<Cbor, String> {
    if profondeur > PROFONDEUR_MAX {
        return Err("imbrication CBOR trop profonde".to_string());
    }
    let (major, val) = en_tete(d, pos)?;
    match major {
        0 => Ok(Cbor::Positif(val)),
        1 => Ok(Cbor::Negatif(-1 - val as i128)),
        2 => {
            let s = reading(d, *pos, val as usize)?;
            *pos += val as usize;
            Ok(Cbor::Octets(s.to_vec()))
        }
        3 => {
            let s = reading(d, *pos, val as usize)?;
            *pos += val as usize;
            String::from_utf8(s.to_vec())
                .map(Cbor::Texte)
                .map_err(|_| "texte CBOR non UTF-8".to_string())
        }
        4 => {
            if val as usize > ENTREES_MAX {
                return Err("tableau CBOR trop grand".to_string());
            }
            let mut items = Vec::with_capacity(val as usize);
            for _ in 0..val {
                items.push(lire_valeur(d, pos, profondeur + 1)?);
            }
            Ok(Cbor::Tableau(items))
        }
        5 => {
            if val as usize > ENTREES_MAX {
                return Err("carte CBOR trop grande".to_string());
            }
            let mut entrees = Vec::with_capacity(val as usize);
            for _ in 0..val {
                let k = lire_valeur(d, pos, profondeur + 1)?;
                let v = lire_valeur(d, pos, profondeur + 1)?;
                entrees.push((k, v));
            }
            Ok(Cbor::Carte(entrees))
        }
        6 => {
            let inner = lire_valeur(d, pos, profondeur + 1)?;
            Ok(Cbor::Etiquette(val, Box::new(inner)))
        }
        _ => Err("type CBOR non pris en charge".to_string()),
    }
}

pub fn lire_cbor(d: &[u8]) -> Result<Cbor, String> {
    let mut pos = 0;
    let valeur = lire_valeur(d, &mut pos, 0)?;
    if pos != d.len() {
        return Err("octets CBOR inattendus en fin de payload".to_string());
    }
    Ok(valeur)
}

// ====================
// authData
// ====================

#[derive(Debug, Clone, PartialEq)]
pub struct DonneesAuthentificateur {
    pub rp_id_hash: [u8; 32],
    pub flags: u8,
    pub compteur: u32,
    pub credential_id: Option<Vec<u8>>,
    pub cle_cose: Option<Cbor>,
}

pub fn analyser_auth_data(d: &[u8]) -> Result<DonneesAuthentificateur, String> {
    if d.len() < 37 {
        return Err(format!("authData trop courte ({} octets)", d.len()));
    }
    let mut rp_id_hash = [0u8; 32];
    rp_id_hash.copy_from_slice(&d[0..32]);
    let flags = d[32];
    let compteur = u32::from_be_bytes([d[33], d[34], d[35], d[36]]);

    let mut credential_id = None;
    let mut cle_cose = None;

    if flags & FLAG_AT != 0 {
        // aaguid (16) + credIdLen (2) + credId + COSE key
        if d.len() < 37 + 18 {
            return Err("authData tronquee avant la credential".to_string());
        }
        let long = u16::from_be_bytes([d[53], d[54]]) as usize;
        let debut = 55usize;
        let fin = debut
            .checked_add(long)
            .ok_or("longueur de credential invalide")?;
        if fin > d.len() {
            return Err("longueur de credential hors authData".to_string());
        }
        credential_id = Some(d[debut..fin].to_vec());
        let mut pos = fin;
        let cose = lire_valeur(d, &mut pos, 0)?;
        if pos != d.len() {
            return Err("octets inattendus apres la cle COSE".to_string());
        }
        cle_cose = Some(cose);
    }

    Ok(DonneesAuthentificateur {
        rp_id_hash,
        flags,
        compteur,
        credential_id,
        cle_cose,
    })
}

/// Cle COSE ES256 -> cle publique SEC1 non compressee (0x04 || X || Y).
pub fn cle_sec1_de_cose(cose: &Cbor) -> Result<Vec<u8>, String> {
    match cose.cle_entier(1) {
        Some(Cbor::Positif(2)) => {}
        _ => return Err("cle COSE non EC2 (kty != 2)".to_string()),
    }
    match cose.cle_entier(3) {
        Some(Cbor::Negatif(-7)) | Some(Cbor::Positif(7)) => {}
        _ => return Err("algorithme COSE non ES256 (attendu -7)".to_string()),
    }
    match cose.cle_entier(-1) {
        Some(Cbor::Negatif(-1)) | Some(Cbor::Positif(1)) => {}
        _ => return Err("courbe COSE non P-256".to_string()),
    }

    let x = cose
        .cle_entier(-2)
        .and_then(|c| c.octets())
        .ok_or("cle COSE sans coordonnee X")?;
    let y = cose
        .cle_entier(-3)
        .and_then(|c| c.octets())
        .ok_or("cle COSE sans coordonnee Y")?;
    if x.len() != 32 || y.len() != 32 {
        return Err("coordonnees ECDSA de taille invalide".to_string());
    }

    let mut sec1 = Vec::with_capacity(65);
    sec1.push(0x04);
    sec1.extend_from_slice(x);
    sec1.extend_from_slice(y);
    Ok(sec1)
}

#[derive(Debug, Clone, PartialEq)]
pub struct CredentialEnclave {
    pub credential_id: String,
    pub public_key_sec1: Vec<u8>,
    pub compteur: u32,
}

/// Decode l'attestationObject envoye par `navigator.credentials.create()`.
pub fn decoder_attestation(attestation_b64url: &str) -> Result<CredentialEnclave, String> {
    let brut = decodage_base64url(attestation_b64url)?;
    if brut.is_empty() {
        return Err("attestationObject vide".to_string());
    }
    let cbor = lire_cbor(&brut)?;

    // fmt = "none" est ce que nous demandons ; toute autre forme n'est pas
    // interpretée comme une preuve de provenance constructeur.
    let auth_bytes = cbor
        .cle_texte("authData")
        .and_then(|c| c.octets())
        .ok_or("attestationObject sans authData")?
        .to_vec();

    let analyse = analyser_auth_data(&auth_bytes)?;
    let credential_id = analyse
        .credential_id
        .ok_or("authData sans donnees de credential (flag AT absent)")?;
    let cose = analyse.cle_cose.ok_or("authData sans cle COSE")?;
    let public_key_sec1 = cle_sec1_de_cose(&cose)?;

    Ok(CredentialEnclave {
        credential_id: encodage_base64url(&credential_id),
        public_key_sec1,
        compteur: analyse.compteur,
    })
}

// ====================
// Configuration relying party (source unique de verite)
// ====================

#[derive(Debug, Clone, PartialEq)]
pub struct ConfigRp {
    pub rp_id: String,
    pub origines: Vec<String>,
    pub exiger_uv: bool,
}

impl ConfigRp {
    /// `UQ_WEBAUTHN_RP_ID` et `UQ_WEBAUTHN_ORIGINES` (liste separee par
    /// virgules). Les defauts sont identiques a ce que renvoie
    /// /api/webauthn/register/begin : meme autorite pour l'enrollment et la
    /// verification.
    pub fn par_defaut() -> Self {
        let rp_id = std::env::var("UQ_WEBAUTHN_RP_ID").unwrap_or_else(|_| "localhost".to_string());
        let origines = std::env::var("UQ_WEBAUTHN_ORIGINES")
            .map(|brut| {
                brut.split(',')
                    .map(|o| o.trim().to_string())
                    .filter(|o| !o.is_empty())
                    .collect::<Vec<String>>()
            })
            .unwrap_or_else(|_| {
                vec![
                    "http://localhost:8082".to_string(),
                    "http://127.0.0.1:8082".to_string(),
                ]
            });
        ConfigRp {
            rp_id,
            origines,
            exiger_uv: true,
        }
    }

    pub fn empreinte_rp(&self) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(self.rp_id.as_bytes());
        let out = h.finalize();
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&out);
        hash
    }
}

// ====================
// Verification d'une assertion
// ====================

#[derive(Debug, Clone)]
pub struct Assertion {
    pub credential_id: String,
    pub auth_data: String,
    pub client_data_json: String,
    pub signature: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Attentes<'a> {
    pub challenge: &'a str,
    pub types_acceptes: &'a [&'a str],
    pub rp: &'a ConfigRp,
    /// Compteur deja enregistre pour cette credential (monotonie anti-clonage).
    pub compteur_precedent: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VerificationReussie {
    pub compteur: u32,
    pub origine: String,
}

/// Comparaison en temps constant (le challenge n'est pas un secret mais une
/// fuite d'information laterale reste inutile).
pub fn comparaison_constante(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut ecart = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        ecart |= x ^ y;
    }
    ecart == 0
}

fn est_autorisee(origine: &str, liste: &[String]) -> bool {
    liste.iter().any(|o| o == origine)
}

pub fn verifier_assertion(
    cle_publique_sec1: &[u8],
    assertion: &Assertion,
    attentes: &Attentes,
) -> Result<VerificationReussie, String> {
    let auth_brut = decodage_base64url(&assertion.auth_data)?;
    let client_brut = decodage_base64url(&assertion.client_data_json)?;
    let signature = decodage_base64url(&assertion.signature)?;
    if signature.is_empty() {
        return Err("signature absente".to_string());
    }

    // 1. authData : empreinte du relying party + presence utilisateur
    let analyse = analyser_auth_data(&auth_brut)?;
    if !comparaison_constante(&analyse.rp_id_hash, &attentes.rp.empreinte_rp()) {
        return Err(format!(
            "rpIdHash different de sha256({:?})",
            attentes.rp.rp_id
        ));
    }
    if analyse.flags & FLAG_UP == 0 {
        return Err("authenticateur sans preuve de presence (flag UP absent)".to_string());
    }
    if attentes.rp.exiger_uv && analyse.flags & FLAG_UV == 0 {
        return Err(
            "verification utilisateur non confirmee par l'enclave (flag UV absent)".to_string(),
        );
    }
    if analyse.flags & FLAG_AT != 0 {
        return Err("authData d'attestation refusee pour une assertion".to_string());
    }

    // 2. clientDataJSON : type, challenge, origine
    let client: Value = serde_json::from_slice(&client_brut)
        .map_err(|e| format!("clientDataJSON invalide: {}", e))?;
    let type_reponse = client
        .get("type")
        .and_then(|t| t.as_str())
        .ok_or("clientDataJSON sans champ type")?;
    if !attentes.types_acceptes.contains(&type_reponse) {
        return Err(format!(
            "type de reponse inattendu: {:?} (attendu {:?})",
            type_reponse, attentes.types_acceptes
        ));
    }
    let challenge_recu = client
        .get("challenge")
        .and_then(|c| c.as_str())
        .ok_or("clientDataJSON sans champ challenge")?;
    if !comparaison_constante(challenge_recu.as_bytes(), attentes.challenge.as_bytes()) {
        return Err("challenge different de celui emis par le serveur".to_string());
    }
    let origine = client
        .get("origin")
        .and_then(|o| o.as_str())
        .ok_or("clientDataJSON sans champ origin")?;
    if !est_autorisee(origine, &attentes.rp.origines) {
        return Err(format!("origine non autorisee: {:?}", origine));
    }

    // 3. Signature : message = authData || sha256(clientDataJSON)
    let hash_client = Sha256::digest(&client_brut);
    let mut message = Vec::with_capacity(auth_brut.len() + 32);
    message.extend_from_slice(&auth_brut);
    message.extend_from_slice(&hash_client);

    let cle = UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, cle_publique_sec1);
    cle.verify(&message, &signature)
        .map_err(|_| "signature ECDSA invalide pour cette credential".to_string())?;

    // 4. Monotonie du compteur (0 = authentificateur sans compteur, tolere)
    if let Some(precedent) = attentes.compteur_precedent {
        if analyse.compteur != 0 && analyse.compteur <= precedent {
            return Err(format!(
                "compteur non progressif ({} <= {}) : credential possibly clonnee",
                analyse.compteur, precedent
            ));
        }
    }

    Ok(VerificationReussie {
        compteur: analyse.compteur,
        origine: origine.to_string(),
    })
}

// ====================
// Tests
// ====================
//
// Les vecteurs ci-dessous sont de vrais payloads WebAuthn produits par une
// vraie cle P-256 (script de generation des vecteurs), pas des donnees
// inventees : ils valident le chemin de succes complet, pas seulement les
// rejets.

#[cfg(test)]
mod vecteurs {
    pub const X: &str = "1b9d134fa21e0ae159979099aa1f4ae1a4ee9649a98ff6aa5b58b91b7921ad34";
    pub const Y: &str = "fae8f56b58944f21d39ea10833edd30aed6291c457759f189d2783acf375337a";
    pub const CRED_ID_HEX: &str =
        "a1b2c3d4e5f60718293a4b5c6d7e8f900fedcba9876543210123456789abcdef";
    pub const ATTESTATION: &str = "o2NmbXRkbm9uZWhhdXRoRGF0YVikSZYN5YgOjGh0NBcPZHZgW4_krrmihjLHmVzzuoMdl2NFAAAACwAAAAAAAAAAAAAAAAAAAAAAIKGyw9Tl9gcYKTpLXG1-j5AP7cuph2VDIQEjRWeJq83vpQECAyYgICFYIBudE0-iHgrhWZeQmaofSuGk7pZJqY_2qltYuRt5Ia00Ilgg-uj1a1iUTyHTnqEIM-3TCu1ikcRXdZ8YnSeDrPN1M3pnYXR0U3RtdKA";
    pub const LOGIN_AUTH_DATA: &str = "SZYN5YgOjGh0NBcPZHZgW4_krrmihjLHmVzzuoMdl2MFAAAADA";
    pub const LOGIN_CLIENT_DATA: &str = "eyJ0eXBlIjoid2ViYXV0aG4uZ2V0IiwiY2hhbGxlbmdlIjoiZEdWemRDMWphR0ZzYkdWdVoyVXRNVEl6TkRVMk56ZzVNREV5TXpRMU5nIiwib3JpZ2luIjoiaHR0cDovL2xvY2FsaG9zdDo4MDgyIiwiY3Jvc3NPcmlnaW4iOmZhbHNlfQ";
    pub const LOGIN_SIGNATURE: &str = "MEQCIGSVBmh7hcGHRvT19DS3mf5boVTY5a8BV25icsMP-g25AiBcfToClhIgbJakw7ka6dXqK5zCNVrs8uewznaj-DriEw";
    pub const CHALLENGE: &str = "dGVzdC1jaGFsbGVuZ2UtMTIzNDU2Nzg5MDEyMzQ1Ng";
    pub const BACK_AUTH_DATA: &str = "SZYN5YgOjGh0NBcPZHZgW4_krrmihjLHmVzzuoMdl2MFAAAABQ";
    pub const BACK_CLIENT_DATA: &str = "eyJ0eXBlIjoid2ViYXV0aG4uZ2V0IiwiY2hhbGxlbmdlIjoiWVc1MGNtVXRjbVZ3ZFdGNUxXTm9ZV3hzWlc1blpRIiwib3JpZ2luIjoiaHR0cDovL2xvY2FsaG9zdDo4MDgyIiwiY3Jvc3NPcmlnaW4iOmZhbHNlfQ";
    pub const BACK_CHALLENGE: &str = "YW50cmUtcmVwdWF5LWNoYWxsZW5nZQ";
    pub const BACK_SIGNATURE: &str = "MEUCIQDkzDoYnKVgOal-EAmKiT41vdq8rMXxg8iPLiMDUCbvEwIgSn5OVpkZ6GqN_OlFw-a2zsImsU-kD_L5THvOPrQuZhg";
    pub const FAUX_SIGNATURE: &str = "MEUCIHEWF_o-P4pj6fHcf_srdmZ1G6luG5DEVrz3248X_NXNAiEA7rMTpe3q_2JGIvvfUKSWlo7E6ZMHGgB0M4Av1-AXODI";
    pub const NOUV_AUTH_DATA: &str = "SZYN5YgOjGh0NBcPZHZgW4_krrmihjLHmVzzuoMdl2MBAAAADQ";
    pub const NOUV_CLIENT_DATA: &str = "eyJ0eXBlIjoid2ViYXV0aG4uZ2V0IiwiY2hhbGxlbmdlIjoiZEdWemRDMWphR0ZzYkdWdVoyVXRNVEl6TkRVMk56ZzVNREV5TXpRMU5nIiwib3JpZ2luIjoiaHR0cDovL2xvY2FsaG9zdDo4MDgyIiwiY3Jvc3NPcmlnaW4iOmZhbHNlfQ";
    pub const NOUV_SIGNATURE: &str = "MEQCIGV6qpHxS5wMn5PEDbw4JdiY9Yv_P96WeyToBw4ZwifZAiAPeghihnTwTAP7y5WTo_5AL-wVQfdWRGYOGOw6TlfeCQ";
}

#[cfg(test)]
mod tests {
    use super::*;
    use vecteurs::*;

    fn cle() -> Vec<u8> {
        let mut sec1 = vec![0x04];
        sec1.extend(hex::decode(X).unwrap());
        sec1.extend(hex::decode(Y).unwrap());
        sec1
    }

    fn rp() -> ConfigRp {
        ConfigRp {
            rp_id: "localhost".to_string(),
            origines: vec!["http://localhost:8082".to_string()],
            exiger_uv: true,
        }
    }

    fn assertion(auth: &str, client: &str, sig: &str) -> Assertion {
        Assertion {
            credential_id: CRED_ID_HEX.to_string(),
            auth_data: auth.to_string(),
            client_data_json: client.to_string(),
            signature: sig.to_string(),
        }
    }

    #[test]
    fn attestation_reelle_donne_la_cle_publique_et_l_id() {
        let credential = decoder_attestation(ATTESTATION).expect("attestation lisible");
        assert_eq!(credential.public_key_sec1, cle());
        assert_eq!(credential.compteur, 11);
        assert_eq!(
            decodage_base64url(&credential.credential_id).unwrap(),
            hex::decode(CRED_ID_HEX).unwrap()
        );
        assert_eq!(
            credential.credential_id,
            encodage_base64url(&hex::decode(CRED_ID_HEX).unwrap())
        );
    }

    #[test]
    fn assertion_valide_acceptee() {
        let resultat = verifier_assertion(
            &cle(),
            &assertion(LOGIN_AUTH_DATA, LOGIN_CLIENT_DATA, LOGIN_SIGNATURE),
            &Attentes {
                challenge: CHALLENGE,
                types_acceptes: &["webauthn.get"],
                rp: &rp(),
                compteur_precedent: None,
            },
        )
        .expect("le vecteur reel doit passer");
        assert_eq!(resultat.compteur, 12);
        assert_eq!(resultat.origine, "http://localhost:8082");
    }

    #[test]
    fn signature_d_une_autre_cle_rejetee() {
        let err = verifier_assertion(
            &cle(),
            &assertion(LOGIN_AUTH_DATA, LOGIN_CLIENT_DATA, FAUX_SIGNATURE),
            &Attentes {
                challenge: CHALLENGE,
                types_acceptes: &["webauthn.get"],
                rp: &rp(),
                compteur_precedent: None,
            },
        )
        .unwrap_err();
        assert!(err.contains("signature"), "message: {}", err);
    }

    #[test]
    fn cle_publique_inconsistante_rejetee() {
        let mut autre = cle();
        autre[33] ^= 0x01;
        let err = verifier_assertion(
            &autre,
            &assertion(LOGIN_AUTH_DATA, LOGIN_CLIENT_DATA, LOGIN_SIGNATURE),
            &Attentes {
                challenge: CHALLENGE,
                types_acceptes: &["webauthn.get"],
                rp: &rp(),
                compteur_precedent: None,
            },
        )
        .unwrap_err();
        assert!(err.contains("signature"), "message: {}", err);
    }

    #[test]
    fn compteur_non_progressif_rejete() {
        // vecteur au compteur 5 apres un compteur deja a 12
        let err = verifier_assertion(
            &cle(),
            &assertion(BACK_AUTH_DATA, BACK_CLIENT_DATA, BACK_SIGNATURE),
            &Attentes {
                challenge: BACK_CHALLENGE,
                types_acceptes: &["webauthn.get"],
                rp: &rp(),
                compteur_precedent: Some(12),
            },
        )
        .unwrap_err();
        assert!(err.contains("compteur"), "message: {}", err);
    }

    #[test]
    fn compteur_progressif_accepte() {
        // vecteur au compteur 13, compteur precedent enregistre = 12
        let un_rp = ConfigRp {
            rp_id: "localhost".to_string(),
            origines: vec!["http://localhost:8082".to_string()],
            exiger_uv: false,
        };
        let resultat = verifier_assertion(
            &cle(),
            &assertion(NOUV_AUTH_DATA, NOUV_CLIENT_DATA, NOUV_SIGNATURE),
            &Attentes {
                challenge: CHALLENGE,
                types_acceptes: &["webauthn.get"],
                rp: &un_rp,
                compteur_precedent: Some(12),
            },
        )
        .expect("13 > 12 : la progression est acceptee");
        assert_eq!(resultat.compteur, 13);
    }

    #[test]
    fn challenge_different_rejete() {
        let err = verifier_assertion(
            &cle(),
            &assertion(LOGIN_AUTH_DATA, LOGIN_CLIENT_DATA, LOGIN_SIGNATURE),
            &Attentes {
                challenge: "autre-challenge",
                types_acceptes: &["webauthn.get"],
                rp: &rp(),
                compteur_precedent: None,
            },
        )
        .unwrap_err();
        assert!(err.contains("challenge"), "message: {}", err);
    }

    #[test]
    fn type_de_reponse_inattendu_rejete() {
        let err = verifier_assertion(
            &cle(),
            &assertion(LOGIN_AUTH_DATA, LOGIN_CLIENT_DATA, LOGIN_SIGNATURE),
            &Attentes {
                challenge: CHALLENGE,
                types_acceptes: &["webauthn.create"],
                rp: &rp(),
                compteur_precedent: None,
            },
        )
        .unwrap_err();
        assert!(err.contains("type"), "message: {}", err);
    }

    #[test]
    fn origine_non_autorisee_rejete() {
        let un_rp = ConfigRp {
            rp_id: "localhost".to_string(),
            origines: vec!["https://app.uq.co".to_string()],
            exiger_uv: true,
        };
        let err = verifier_assertion(
            &cle(),
            &assertion(LOGIN_AUTH_DATA, LOGIN_CLIENT_DATA, LOGIN_SIGNATURE),
            &Attentes {
                challenge: CHALLENGE,
                types_acceptes: &["webauthn.get"],
                rp: &un_rp,
                compteur_precedent: None,
            },
        )
        .unwrap_err();
        assert!(err.contains("origine"), "message: {}", err);
    }

    #[test]
    fn autre_rp_id_rejete() {
        let un_rp = ConfigRp {
            rp_id: "uq.co".to_string(),
            origines: vec!["http://localhost:8082".to_string()],
            exiger_uv: true,
        };
        let err = verifier_assertion(
            &cle(),
            &assertion(LOGIN_AUTH_DATA, LOGIN_CLIENT_DATA, LOGIN_SIGNATURE),
            &Attentes {
                challenge: CHALLENGE,
                types_acceptes: &["webauthn.get"],
                rp: &un_rp,
                compteur_precedent: None,
            },
        )
        .unwrap_err();
        assert!(err.contains("rpIdHash"), "message: {}", err);
    }

    #[test]
    fn absence_de_verification_utilisateur_rejetee() {
        let err = verifier_assertion(
            &cle(),
            &assertion(NOUV_AUTH_DATA, NOUV_CLIENT_DATA, NOUV_SIGNATURE),
            &Attentes {
                challenge: CHALLENGE,
                types_acceptes: &["webauthn.get"],
                rp: &rp(),
                compteur_precedent: None,
            },
        )
        .unwrap_err();
        assert!(err.contains("UV"), "message: {}", err);
    }

    #[test]
    fn auth_data_trop_courte_rejetee() {
        let courte = encodage_base64url(&[0u8; 20]);
        let err = analyser_auth_data(&decodage_base64url(&courte).unwrap()).unwrap_err();
        assert!(err.contains("trop courte"), "message: {}", err);
    }

    #[test]
    fn cbor_rejete_les_longueurs_indefinies() {
        assert!(lire_cbor(&[0x9f, 0x01]).is_err());
        assert!(lire_cbor(&[0xbf]).is_err());
    }

    #[test]
    fn base64url_tolere_padding_et_alias() {
        let brut = vec![0xffu8, 0xff, 0xff];
        assert_eq!(encodage_base64url(&brut), "____");
        assert_eq!(decodage_base64url("____").unwrap(), brut);
        assert_eq!(decodage_base64url("____=").unwrap(), brut);
        assert_eq!(decodage_base64url("____==").unwrap(), brut);
        // l'alphabet standard est accepte aux cotes de l'alphabet url
        assert_eq!(
            decodage_base64url("/+8").unwrap(),
            decodage_base64url("_-8").unwrap()
        );
        assert_eq!(decodage_base64url("").unwrap(), Vec::<u8>::new());
        assert!(decodage_base64url("a!*").is_err());
    }

    #[test]
    fn roundtrip_base64url_sur_200_octets() {
        let brut: Vec<u8> = (0..200u32).map(|i| (i * 7 % 256) as u8).collect();
        assert_eq!(
            decodage_base64url(&encodage_base64url(&brut)).unwrap(),
            brut
        );
    }

    #[test]
    fn comparaison_constante_rejete_longueurs_differentes() {
        assert!(!comparaison_constante(b"abc", b"abcd"));
        assert!(comparaison_constante(b"abcd", b"abcd"));
    }
}
