use serde::{Serialize, Deserialize};
use crate::error::ChainError;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MobileProvider { MPesa, AirtelMoney, OrangeMoney, MTNMoMo, TigoCash, VodafoneCash, FreeMoney, Ecocash, MoovMoney, Wave, Other(String) }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Country { CD, KE, TZ, GH, NG, CI, SN, BJ, ZW, Other(String) }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmsParseResult {
    pub success: bool, pub amount: Option<u64>, pub currency: Option<String>,
    pub sender: Option<String>, pub recipient: Option<String>,
    pub transaction_id: Option<String>, pub provider: Option<MobileProvider>,
    pub raw_text: String, pub confidence: f64,
}

impl SmsParseResult {
    fn new(raw: &str) -> Self {
        SmsParseResult { success: false, amount: None, currency: None, sender: None, recipient: None, transaction_id: None, provider: None, raw_text: raw.to_string(), confidence: 0.0 }
    }
}

pub fn parse_sms(sms_text: &str, country: &Country) -> Result<SmsParseResult, ChainError> {
    if sms_text.is_empty() { return Err(ChainError::InvalidProof("SMS vide".into())); }
    let mut r = SmsParseResult::new(sms_text);
    let s = sms_text.to_lowercase();
    r.provider = detect_provider(&s);
    r.amount = extract_amount(&s);
    r.currency = extract_currency(&s, country);
    r.sender = extract_phone(&s);
    r.recipient = extract_phone(&s);
    r.transaction_id = extract_txn_id(&s);
    r.confidence = confidence(&r);
    r.success = r.confidence >= 0.4 && r.amount.is_some();
    Ok(r)
}

fn detect_provider(s: &str) -> Option<MobileProvider> {
    let p = [("m-pesa", MobileProvider::MPesa), ("mpesa", MobileProvider::MPesa), ("airtel money", MobileProvider::AirtelMoney), ("airtel", MobileProvider::AirtelMoney), ("orange money", MobileProvider::OrangeMoney), ("orange", MobileProvider::OrangeMoney), ("mtn momo", MobileProvider::MTNMoMo), ("mtn mobile money", MobileProvider::MTNMoMo), ("tigo cash", MobileProvider::TigoCash), ("tigo", MobileProvider::TigoCash), ("vodafone cash", MobileProvider::VodafoneCash), ("free money", MobileProvider::FreeMoney), ("ecocash", MobileProvider::Ecocash), ("moov money", MobileProvider::MoovMoney), ("wave", MobileProvider::Wave)];
    for (k, v) in p.iter() { if s.contains(k) { return Some(v.clone()); } }
    None
}

fn extract_amount(s: &str) -> Option<u64> {
    let words: Vec<&str> = s.split_whitespace().collect();
    let kw = ["received","sent","amount","montant","payment","paiement","transfer"];
    for (i, w) in words.iter().enumerate() {
        let c: String = w.chars().filter(|c| c.is_digit(10)).collect();
        if c.len() >= 3 { if let Ok(a) = c.parse::<u64>() {
            let ctx = (i>0 && kw.iter().any(|k| words[i-1].to_lowercase().contains(k))) || (i+1<words.len() && is_currency(words[i+1]));
            if ctx || a >= 100 { return Some(a); }
        }}
    }
    let mut max: u64 = 0;
    for w in words { let c: String = w.chars().filter(|c| c.is_digit(10)).collect(); if let Ok(a) = c.parse::<u64>() { if a>max&&a>=100 { max=a; } } }
    if max>0 { Some(max) } else { None }
}

fn is_currency(w: &str) -> bool { ["cdf","usd","eur","kes","tzs","ghs","ngn","xof","xaf","zar"].contains(&w) }

fn extract_currency(s: &str, country: &Country) -> Option<String> {
    let cc = [("cdf","CDF"),("congolese","CDF"),("kes","KES"),("shilling","KES"),("tzs","TZS"),("ghs","GHS"),("cedi","GHS"),("ngn","NGN"),("naira","NGN"),("usd","USD"),("dollar","USD"),("eur","EUR"),("euro","EUR")];
    for (k,v) in cc.iter() { if s.contains(k) { return Some(v.to_string()); } }
    match country { Country::CD=>Some("CDF".into()), Country::KE=>Some("KES".into()), Country::TZ=>Some("TZS".into()), Country::GH=>Some("GHS".into()), Country::NG=>Some("NGN".into()), _=>None }
}

fn extract_phone(s: &str) -> Option<String> {
    for w in s.split_whitespace() { let c: String = w.chars().filter(|c| c.is_digit(10)||*c=='+').collect(); if c.len()>=10 && c.len()<=15 && (c.starts_with('+')||c.starts_with('0')) { return Some(c); } }
    None
}

fn extract_txn_id(s: &str) -> Option<String> {
    for p in ["ref:","reference:","trans id:","transaction id:","id:","confirmation code:"] {
        if let Some(pos) = s.find(p) {
            let a = &s[pos+p.len()..]; let id: String = a.chars().take_while(|c| c.is_alphanumeric()).collect();
            if !id.is_empty() { return Some(id.to_uppercase()); }
        }
    }
    for w in s.split_whitespace() { let c: String = w.chars().filter(|c| c.is_alphanumeric()).collect(); if c.len()>=8 && c.len()<=12 && c.chars().any(|c| c.is_digit(10)) { return Some(c.to_uppercase()); } }
    None
}

fn confidence(r: &SmsParseResult) -> f64 {
    let mut s: f64 = 0.0;
    if r.provider.is_some() { s += 0.3; }
    if r.amount.is_some() { s += 0.3; }
    if r.currency.is_some() { s += 0.1; }
    if r.transaction_id.is_some() { s += 0.2; }
    if r.sender.is_some() || r.recipient.is_some() { s += 0.1; }
    if s > 1.0 { 1.0 } else { s }
}

pub fn get_providers_for_country(country: &Country) -> Vec<MobileProvider> {
    match country {
        Country::CD => vec![MobileProvider::MPesa, MobileProvider::AirtelMoney, MobileProvider::OrangeMoney, MobileProvider::FreeMoney],
        Country::KE => vec![MobileProvider::MPesa, MobileProvider::AirtelMoney],
        Country::GH => vec![MobileProvider::MTNMoMo, MobileProvider::VodafoneCash, MobileProvider::AirtelMoney, MobileProvider::TigoCash],
        Country::NG => vec![MobileProvider::MTNMoMo, MobileProvider::AirtelMoney],
        Country::CI => vec![MobileProvider::OrangeMoney, MobileProvider::MTNMoMo, MobileProvider::MoovMoney, MobileProvider::Wave],
        Country::SN => vec![MobileProvider::OrangeMoney, MobileProvider::Wave, MobileProvider::FreeMoney],
        Country::TZ => vec![MobileProvider::MPesa, MobileProvider::AirtelMoney, MobileProvider::TigoCash],
        _ => vec![MobileProvider::Other("Inconnu".into())],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mpesa() {
        let r = parse_sms("M-PESA: You have received 50000 KES from JOHN. ID: MP123", &Country::KE).unwrap_or_default();
        assert!(r.success); assert_eq!(r.amount, Some(50000)); assert_eq!(r.provider, Some(MobileProvider::MPesa));
    }

    #[test]
    fn test_orange() {
        let r = parse_sms("Orange Money: Vous avez recu 25000 CDF. Ref: OM987", &Country::CD).unwrap_or_default();
        assert!(r.success); assert_eq!(r.amount, Some(25000));
    }

    #[test]
    fn test_empty() { assert!(parse_sms("", &Country::KE).is_err()); }

    #[test]
    fn test_providers_cd() {
        let p = get_providers_for_country(&Country::CD);
        assert!(p.len() >= 3);
    }
}
