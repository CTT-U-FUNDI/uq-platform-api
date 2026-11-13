use serde::{Serialize, Deserialize};
use sha2::{Sha256, Digest};
use crate::error::ChainError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Identity {
    pub id_hash: [u8; 32],
    pub uq_number: String,
    pub uq_email: String,
    pub uq_pseudo: String,
    pub public_key_hash: [u8; 32],
    pub country_code: String,
    pub created_at: i64,
    pub kyc_verified: bool,
    pub kyc_level: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KycAttestation {
    pub identity_hash: [u8; 32],
    pub is_human: bool,
    pub is_adult: bool,
    pub country: String,
    pub verified_at: i64,
    pub verifier_signature: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Certificate {
    pub identity_hash: [u8; 32],
    pub uq_number: String,
    pub uq_email: String,
    pub uq_pseudo: String,
    pub public_key_hash: [u8; 32],
    pub issued_at: i64,
    pub expires_at: i64,
    pub signature: Vec<u8>,
}

const COUNTRY_FORMATS: &[(&str, &str, usize)] = &[
    ("FR", "+33 7", 8), ("CD", "+243 9", 8), ("US", "+1 5", 10),
    ("GB", "+44 7", 9), ("RU", "+7 9", 10), ("CN", "+86 1", 11),
    ("IN", "+91 9", 10), ("BR", "+55 9", 9), ("JP", "+81 9", 10), ("ZA", "+27 6", 9),
];

fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new(); h.update(data);
    let mut r = [0u8; 32]; r.copy_from_slice(&h.finalize()); r
}

pub fn create_identity(name: &str, country_code: &str, kyc_proof: &[u8]) -> Result<Identity, ChainError> {
    if name.trim().is_empty() { return Err(ChainError::InternalError("Nom requis".into())); }
    #[allow(unused_variables)]
    let country = COUNTRY_FORMATS.iter().find(|(c,_,_)| *c == country_code.to_uppercase()).ok_or_else(|| ChainError::InternalError("Pays non supporté".into()))?;
    let now = chrono::Utc::now().timestamp();
    let mut h = Sha256::new(); h.update(name.as_bytes()); h.update(country_code.as_bytes()); h.update(&now.to_le_bytes());
    let mut id_hash = [0u8; 32]; id_hash.copy_from_slice(&h.finalize());
    let uq_number = derive_uq_number(country_code, &id_hash)?;
    let uq_email = derive_uq_email(name, &id_hash)?;
    let uq_pseudo = derive_uq_pseudo(name, &id_hash)?;
    Ok(Identity { id_hash, uq_number, uq_email, uq_pseudo, public_key_hash:[0u8;32], country_code:country_code.to_uppercase(), created_at:now, kyc_verified:!kyc_proof.is_empty(), kyc_level:if kyc_proof.is_empty(){0}else{1} })
}

pub fn derive_uq_number(country_code: &str, id_hash: &[u8; 32]) -> Result<String, ChainError> {
    #[allow(unused_variables)]
    let country = COUNTRY_FORMATS.iter().find(|(c,_,_)| *c == country_code.to_uppercase()).ok_or_else(|| ChainError::InternalError("Pays non supporté".into()))?;
    let prefix = country.1; let digits_count = country.2;
    let mut digits = String::new();
    for i in 0..digits_count { digits.push_str(&((id_hash[i%32] % 10).to_string())); }
    Ok(match country_code.to_uppercase().as_str() {
        "FR" => format!("{}{} {} {} {} {}", prefix, &digits[0..1], &digits[1..3], &digits[3..5], &digits[5..7], &digits[7..8]),
        _ => format!("{}{}", prefix, digits),
    })
}

pub fn derive_uq_email(name: &str, _id_hash: &[u8; 32]) -> Result<String, ChainError> {
    let normalized = name.to_lowercase().chars().map(|c| match c { 'é'|'è'|'ê'|'ë'=>'e','à'|'â'|'ä'=>'a','ù'|'û'|'ü'=>'u','ô'|'ö'=>'o','î'|'ï'=>'i','ç'=>'c',' '=>'.',_ if c.is_alphanumeric()=>c,_=>'.' }).collect::<String>();
    Ok(format!("{}@ufundi.uq", normalized))
}

pub fn derive_uq_pseudo(name: &str, _id_hash: &[u8; 32]) -> Result<String, ChainError> {
    let first = name.split_whitespace().next().unwrap_or("user");
    let normalized = first.to_lowercase().chars().filter(|c| c.is_alphabetic()).map(|c| match c { 'é'|'è'|'ê'|'ë'=>'e','à'|'â'|'ä'=>'a','ù'|'û'|'ü'=>'u','ô'|'ö'=>'o','î'|'ï'=>'i','ç'=>'c',_=>c }).collect::<String>();
    Ok(format!("@{}", normalized))
}

pub fn get_identity() -> Result<Identity, ChainError> { Err(ChainError::InternalError("Not found".into())) }
pub fn get_identity_by_number(_number: &str) -> Result<Identity, ChainError> { Err(ChainError::InternalError("DHT lookup not implemented".into())) }
pub fn get_identity_by_email(_email: &str) -> Result<Identity, ChainError> { Err(ChainError::InternalError("DHT lookup not implemented".into())) }
pub fn verify_kyc(_proof: &[u8], _id_hash: &[u8; 32]) -> Result<Option<KycAttestation>, ChainError> { Ok(None) }
pub fn create_identity_certificate(identity: &Identity) -> Result<Certificate, ChainError> {
    let now = chrono::Utc::now().timestamp();
    Ok(Certificate { identity_hash:identity.id_hash, uq_number:identity.uq_number.clone(), uq_email:identity.uq_email.clone(), uq_pseudo:identity.uq_pseudo.clone(), public_key_hash:identity.public_key_hash, issued_at:now, expires_at:now+365*86400, signature:Vec::new() })
}
pub fn verify_identity_certificate(cert: &Certificate) -> Result<bool, ChainError> {
    let now = chrono::Utc::now().timestamp();
    Ok(now <= cert.expires_at && cert.issued_at <= now && !cert.signature.is_empty())
}
pub fn is_valid_uq_number(n: &str) -> bool { n.starts_with('+') && n.len()>=10 && n.len()<=18 }
pub fn is_valid_uq_email(e: &str) -> bool { e.ends_with("@ufundi.uq") && e.contains('@') }
pub fn is_valid_uq_pseudo(p: &str) -> bool { p.starts_with('@') && p.len()>=2 && p.len()<=30 }
pub fn get_supported_countries() -> Vec<&'static str> { COUNTRY_FORMATS.iter().map(|(c,_,_)| *c).collect() }

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn test_number() { assert!(derive_uq_number("FR",&[1u8;32]).unwrap_or_default().starts_with("+33 7")); }
    #[test] fn test_email() { assert!(derive_uq_email("Jean Dupont",&[3u8;32]).unwrap_or_default().ends_with("@ufundi.uq")); }
    #[test] fn test_pseudo() { assert!(derive_uq_pseudo("Jean Dupont",&[4u8;32]).unwrap_or_default().starts_with('@')); }
    #[test] fn test_create() { let id = create_identity("Jean Dupont","FR",&[]).unwrap_or_default(); assert!(id.uq_number.starts_with("+33")); }
    #[test] fn test_empty_name() { assert!(create_identity("","FR",&[]).is_err()); }
    #[test] fn test_bad_country() { assert!(create_identity("Test","XX",&[]).is_err()); }
    #[test] fn test_valid_number() { assert!(is_valid_uq_number("+33 7 12 34 56 78")); assert!(!is_valid_uq_number("06")); }
    #[test] fn test_valid_email() { assert!(is_valid_uq_email("jean@ufundi.uq")); assert!(!is_valid_uq_email("jean@gmail.com")); }
    #[test] fn test_valid_pseudo() { assert!(is_valid_uq_pseudo("@jean")); assert!(!is_valid_uq_pseudo("jean")); }
    #[test] fn test_countries() { let c = get_supported_countries(); assert!(c.contains(&"FR")); assert!(c.contains(&"CD")); }
}










