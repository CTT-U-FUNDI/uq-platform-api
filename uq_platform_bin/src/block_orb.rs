// ====================
// 🌊 BLOCKORB — Disque dur en u128
// ====================
//
// « L'eau ne se crée pas. L'eau ne se détruit pas. L'eau circule. »
//
// BlockOrb est un DISQUE DUR :
// - Capacité fixe : 40 Md BORB
// - Il CALCULE
// - Il REJETTE ce qui dépasse
// - Il NE GÉNÈRE RIEN
//
// ====================

use crate::montant::{MontantBorb, CAPACITE_TOTALE, FEMTO_PAR_BORB};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Placeholders historiques qui ne sont PAS un cadenas structurel.
pub const CADENAS_PLACEHOLDER: &str = "NON_POSE";

pub fn cadenas_credible(h: &str) -> bool {
    !h.is_empty() && h != CADENAS_PLACEHOLDER && h != "DEVERROUILLE_AUTO_OWNER"
}

// ====================
// MODULES DE SÉRIALISATION U128 EN STRING
// ====================

pub mod u128_string {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(value: &u128, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&value.to_string())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<u128, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse::<u128>().map_err(serde::de::Error::custom)
    }
}

pub mod u128_array_string {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(value: &[u128; 6], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeSeq;
        let mut seq = serializer.serialize_seq(Some(6))?;
        for v in value.iter() {
            seq.serialize_element(&v.to_string())?;
        }
        seq.end()
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<[u128; 6], D::Error>
    where
        D: Deserializer<'de>,
    {
        let v: Vec<String> = Vec::deserialize(deserializer)?;
        if v.len() != 6 {
            return Err(serde::de::Error::custom("expected array of 6 u128 strings"));
        }
        let mut result = [0u128; 6];
        for (i, s) in v.iter().enumerate() {
            result[i] = s.parse::<u128>().map_err(serde::de::Error::custom)?;
        }
        Ok(result)
    }
}

pub mod u128_map_string {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::collections::HashMap;

    pub fn serialize<S>(value: &HashMap<String, u128>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(value.len()))?;
        for (k, v) in value.iter() {
            map.serialize_entry(k, &v.to_string())?;
        }
        map.end()
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<HashMap<String, u128>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let map: HashMap<String, String> = HashMap::deserialize(deserializer)?;
        let mut result = HashMap::new();
        for (k, s) in map.iter() {
            result.insert(
                k.clone(),
                s.parse::<u128>().map_err(serde::de::Error::custom)?,
            );
        }
        Ok(result)
    }
}

// ===== LES 6 CASES (formes de l'eau) =====
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FormeEau {
    Ocean,    // 💰 Trésor
    Nuages,   // 💸 Frais
    Rivieres, // 🏆 Récompenses
    Glaciers, // ❄️ Épargne
    Nappes,   // 💳 Crédit
    Vapeur,   // 🛡️ Bouclier
}

impl FormeEau {
    pub fn all() -> [FormeEau; 6] {
        [
            FormeEau::Ocean,
            FormeEau::Nuages,
            FormeEau::Rivieres,
            FormeEau::Glaciers,
            FormeEau::Nappes,
            FormeEau::Vapeur,
        ]
    }

    pub fn index(&self) -> usize {
        match self {
            FormeEau::Ocean => 0,
            FormeEau::Nuages => 1,
            FormeEau::Rivieres => 2,
            FormeEau::Glaciers => 3,
            FormeEau::Nappes => 4,
            FormeEau::Vapeur => 5,
        }
    }
}

// === TRACABILITE BORB ===
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TxBorb {
    pub borb_id: String,
    pub from_uq: String,
    pub to_uq: String,
    #[serde(with = "u128_string")]
    pub amount: u128,
    #[serde(with = "u128_string")]
    pub fee: u128,
    pub timestamp: i64,
    pub signature: String,
    pub cadenas_hash: String,
    pub universal_key: String,
    pub prev_hash: String,
    pub status: String,
    #[serde(default)]
    pub last_seen_at: i64,
}

// ===== LE DISQUE DUR =====
/// 🔐 Un portefeuille avec son cadenas
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Portefeuille {
    /// Solde en femto-BORB
    #[serde(with = "u128_string")]
    pub solde: u128,

    /// 🔐 CADENAS (hash transformé, stocké sous autre forme)
    pub cadenas: Option<String>,

    /// Date de création du cadenas
    pub cadenas_at: Option<i64>,

    /// 🚨 Alerte piratage
    #[serde(default)]
    pub alerte_piratage: bool,

    /// Détails de l'alerte
    pub alerte_details: Option<String>,

    // ====================
    // 🆕 TRAÇABILITÉ BORB
    // ====================
    /// ⏰ Horloge interne du BORB
    #[serde(default)]
    pub internal_clock: i64,

    /// 🔑 Dernière signature Ed25519
    #[serde(default)]
    pub last_signature: Option<String>,

    /// 🧬 Hash de la dernière transaction
    #[serde(default)]
    pub last_tx_hash: Option<String>,

    /// 🌐 Clé universelle (rotation 30s)
    #[serde(default)]
    pub universal_key_hash: Option<String>,

    /// 📜 Historique complet des transactions
    #[serde(default)]
    pub history: Vec<TxBorb>,
}

impl Portefeuille {
    pub fn new() -> Self {
        Self {
            solde: 0,
            cadenas: None,
            cadenas_at: None,
            alerte_piratage: false,
            alerte_details: None,
            internal_clock: chrono::Utc::now().timestamp(),
            last_signature: None,
            last_tx_hash: None,
            universal_key_hash: None,
            history: Vec::new(),
        }
    }

    /// Ajoute une trace de transaction dans l'historique
    pub fn add_trace(&mut self, trace: TxBorb) {
        self.last_tx_hash = Some(trace.borb_id.clone());
        self.internal_clock = trace.last_seen_at;
        self.history.push(trace);
        if self.history.len() > 1000 {
            self.history.remove(0);
        }
    }

    /// Met à jour l'horloge interne
    pub fn tick_clock(&mut self) {
        self.internal_clock = chrono::Utc::now().timestamp();
    }

    /// Vérifie le chaînage de l'historique
    pub fn verify_history(&self) -> bool {
        if self.history.len() < 2 {
            return true;
        }
        for i in 1..self.history.len() {
            if self.history[i].from_uq != self.history[i - 1].to_uq {
                return false;
            }
        }
        true
    }

    pub fn solde(&self) -> u128 {
        self.solde
    }

    pub fn set_solde(&mut self, nouveau: u128) {
        self.solde = nouveau;
    }

    pub fn ajouter(&mut self, montant: u128) -> Result<(), String> {
        self.solde = self.solde.checked_add(montant).ok_or("Depassement")?;
        Ok(())
    }

    pub fn retirer(&mut self, montant: u128) -> Result<(), String> {
        self.solde = self.solde.checked_sub(montant).ok_or("Solde insuffisant")?;
        Ok(())
    }

    pub fn poser_cadenas(&mut self, cadenas: String, at: i64) {
        self.cadenas = Some(cadenas);
        self.cadenas_at = Some(at);
    }

    /// Cadenas structurel présent et crédible (jamais un placeholder).
    pub fn porte_cadenas(&self) -> bool {
        self.cadenas
            .as_deref()
            .map(cadenas_credible)
            .unwrap_or(false)
    }

    pub fn geler(&mut self, details: String) {
        self.alerte_piratage = true;
        self.alerte_details = Some(details);
    }

    pub fn est_gele(&self) -> bool {
        self.alerte_piratage
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockOrb {
    /// Les 6 Cases (en femto-BORB, u128)
    #[serde(with = "u128_array_string")]
    pub cases: [u128; 6],

    /// 🎁 Enveloppe UBI (10 M BORB, preleve du Tresor)
    #[serde(default)]
    #[serde(with = "u128_string")]
    pub ubi_enveloppe: u128,

    /// Les comptes utilisateurs (solde NORMAL)
    pub comptes: HashMap<String, Portefeuille>,

    /// 🎁 Les comptes CADEAUX (parrainage + UBI)
    #[serde(default)]
    #[serde(with = "u128_map_string")]
    pub comptes_cadeaux: HashMap<String, u128>,

    /// Numéro de bloc (pour historique)
    pub block_number: u64,

    /// Timestamp du dernier bloc
    pub last_block_at: i64,

    /// 🧬 Hash du dernier bloc (chaînage global)
    #[serde(default)]
    pub last_block_hash: String,
}

impl BlockOrb {
    /// Crée un nouveau BlockOrb avec la capacité initiale
    pub fn new() -> Self {
        let mut cases = [0u128; 6];

        // Enveloppe UBI : 10 M BORB (preleve du Tresor)
        let ubi_enveloppe = 10_000_000u128 * FEMTO_PAR_BORB;

        // Repartition initiale (Tresor reduit de 10M)
        cases[0] = 31_110_000_000 * FEMTO_PAR_BORB; // Tresor
        cases[1] = 2_240_000_000 * FEMTO_PAR_BORB; // Frais
        cases[2] = 2_240_000_000 * FEMTO_PAR_BORB; // Recompenses
        cases[3] = 2_240_000_000 * FEMTO_PAR_BORB; // Epargne
        cases[4] = 1_320_000_000 * FEMTO_PAR_BORB; // Credit
        cases[5] = 840_000_000 * FEMTO_PAR_BORB; // Bouclier

        Self {
            cases,
            ubi_enveloppe,
            comptes: HashMap::new(),
            comptes_cadeaux: HashMap::new(),
            block_number: 0,
            last_block_at: chrono::Utc::now().timestamp(),
            last_block_hash: String::new(),
        }
    }

    /// Somme des 6 Cases
    pub fn somme_cases(&self) -> u128 {
        self.cases.iter().sum()
    }

    /// Somme des comptes (normal + cadeaux)
    pub fn somme_comptes(&self) -> u128 {
        self.comptes.values().map(|p| p.solde).sum::<u128>()
            + self.comptes_cadeaux.values().sum::<u128>()
    }

    /// Total (doit être 40 Md)
    pub fn total(&self) -> u128 {
        self.somme_cases() + self.ubi_enveloppe + self.somme_comptes()
    }

    /// Vérifie l'invariant (tolérance = 1 micro-BORB, absorbe les arrondis de
    /// conversion USD→femto ; jamais assez large pour cacher un BORB entier).
    pub fn verifier(&self) -> bool {
        let total = self.total();
        let capacite = CAPACITE_TOTALE;

        let diff = if total > capacite {
            total - capacite
        } else {
            capacite - total
        };

        // 1 micro-BORB = 10^12 femto. Avant : 1 BORB entier toléré par compte
        // (une fuite d'1 BORB passait pour « sain » et était remintée par
        // réconciliation depuis le Trésor).
        let tolerance = 1_000_000_000_000u128;

        diff <= tolerance
    }

    /// Taille du BlockOrb en octets (sérialisé)
    pub fn taille_octets(&self) -> usize {
        serde_json::to_string(self).map(|s| s.len()).unwrap_or(0)
    }

    /// Scelle le bloc (numéro + horodatage + hash de chaînage)
    pub fn sceller_bloc(&mut self) {
        self.block_number += 1;
        self.last_block_at = chrono::Utc::now().timestamp();

        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(self.block_number.to_le_bytes());
        hasher.update(self.last_block_at.to_le_bytes());
        hasher.update(self.last_block_hash.as_bytes());
        let result = hasher.finalize();
        self.last_block_hash = hex::encode(result);
    }

    /// Le portefeuille existe-t-il et porte-t-il un cadenas structurel
    /// crédible (posé à la création, jamais un placeholder) ?
    /// C'est LE porteur : sans lui, aucun BORB n'entre.
    pub fn porte_cadenas(&self, uq_number: &str) -> bool {
        self.comptes
            .get(uq_number)
            .map(|p| p.porte_cadenas())
            .unwrap_or(false)
    }

    /// Traite une transaction (u128 = ultra rapide).
    /// Règle BlockOrb : un transfert ne crée JAMAIS de portefeuille et
    /// n'entre dans aucun compte sans cadenas structurel.
    pub fn traiter(&mut self, from: &str, to: &str, montant: MontantBorb) -> Result<(), String> {
        let p_from = self
            .comptes
            .get(from)
            .ok_or_else(|| format!("CADENAS_REQUIS: compte {from} absent du BlockOrb"))?;
        if p_from.alerte_piratage {
            return Err(format!("Compte {from} gele (alerte piratage)"));
        }
        if !p_from.porte_cadenas() {
            return Err(format!(
                "CADENAS_REQUIS: {from} n'a pas de cadenas structurel — sortie refusee"
            ));
        }
        if p_from.solde < montant.0 {
            return Err(format!(
                "Solde insuffisant : {} < {}",
                p_from.solde, montant.0
            ));
        }

        let p_to = self
            .comptes
            .get(to)
            .ok_or_else(|| format!("CADENAS_REQUIS: compte {to} absent du BlockOrb"))?;
        if !p_to.porte_cadenas() {
            return Err(format!(
                "CADENAS_REQUIS: {to} n'a pas de cadenas structurel — BORB refuse"
            ));
        }
        if p_to.alerte_piratage {
            return Err(format!("Compte {to} gele (alerte piratage)"));
        }

        let solde_from = p_from.solde;
        let solde_to = p_to.solde;

        self.comptes.get_mut(from).unwrap().solde = solde_from - montant.0;
        self.comptes.get_mut(to).unwrap().solde = solde_to + montant.0;

        if !self.verifier() {
            self.comptes.get_mut(from).unwrap().solde = solde_from;
            self.comptes.get_mut(to).unwrap().solde = solde_to;
            return Err("Invariant cassé — ROLLBACK".to_string());
        }

        self.block_number += 1;
        self.last_block_at = chrono::Utc::now().timestamp();

        Ok(())
    }

    /// Distribue les frais aux 6 Cases.
    /// Règle : 40 % Frais, 20 % Récompenses, 15 % Trésor, 10 % Épargne,
    /// 10 % Crédit, 5 % Bouclier (reliquat d'arrondi sur le Bouclier).
    pub fn distribuer_frais(&mut self, frais: u128) -> Result<(), String> {
        let parts = [
            frais * 15 / 100,
            frais * 40 / 100,
            frais * 20 / 100,
            frais * 10 / 100,
            frais * 10 / 100,
            0,
        ];
        let attribue: u128 = parts.iter().sum();
        let parts = [
            parts[0],
            parts[1],
            parts[2],
            parts[3],
            parts[4],
            frais.saturating_sub(attribue),
        ];

        for (i, part) in parts.iter().enumerate() {
            self.cases[i] = self.cases[i]
                .checked_add(*part)
                .ok_or("Dépassement u128 dans les Cases")?;
        }

        if !self.verifier() {
            return Err("Invariant cassé après distribution".to_string());
        }

        Ok(())
    }

    /// Crée un nouveau compte **en prélevant du Trésor** (jamais en créant
    /// de la valeur). Le cadenas structurel est exigé dès la création :
    /// sans identifiants personnels hashés, pas de compte, pas de BORB.
    pub fn creer_compte(
        &mut self,
        uq_number: &str,
        solde_initial: u128,
        cadenas: &str,
    ) -> Result<(), String> {
        if self.comptes.contains_key(uq_number) {
            return Ok(());
        }
        if !cadenas_credible(cadenas) {
            return Err(format!(
                "CADENAS_REQUIS: creation de {} refusee sans cadenas structurel",
                uq_number
            ));
        }
        if self.cases[0] < solde_initial {
            return Err("Trésor insuffisant pour creer le compte".to_string());
        }
        let avant = self.cases[0];
        self.cases[0] -= solde_initial;
        let mut p = Portefeuille::new();
        p.solde = solde_initial;
        p.cadenas = Some(cadenas.to_string());
        p.cadenas_at = Some(chrono::Utc::now().timestamp());
        self.comptes.insert(uq_number.to_string(), p);
        if !self.verifier() {
            self.cases[0] = avant;
            self.comptes.remove(uq_number);
            return Err("Invariant casse a la creation du compte".to_string());
        }
        Ok(())
    }

    /// 🎁 Crédite le solde CADEAUX (parrainage, UBI).
    /// Le cadeau n'entre que dans un compte tracé ET cadenassé.
    pub fn crediter_cadeau(&mut self, uq_number: &str, montant: u128) -> Result<(), String> {
        if !self.porte_cadenas(uq_number) {
            return Err(format!(
                "CADENAS_REQUIS: cadeau refuse pour {} (compte absent ou sans cadenas)",
                uq_number
            ));
        }
        let solde = self.comptes_cadeaux.get(uq_number).copied().unwrap_or(0);
        self.comptes_cadeaux
            .insert(uq_number.to_string(), solde + montant);

        if !self.verifier() {
            self.comptes_cadeaux.insert(uq_number.to_string(), solde);
            return Err("Invariant casse apres credit cadeau".to_string());
        }

        Ok(())
    }

    /// 🎁 Solde CADEAUX d'un utilisateur
    pub fn solde_cadeaux(&self, uq_number: &str) -> u128 {
        self.comptes_cadeaux.get(uq_number).copied().unwrap_or(0)
    }

    /// 🎁 Transferer depuis le solde CADEAUX
    pub fn traiter_cadeau(&mut self, from: &str, to: &str, montant: u128) -> Result<(), String> {
        if !self.porte_cadenas(to) {
            return Err(format!(
                "CADENAS_REQUIS: cadeau vers {} refuse (compte absent ou sans cadenas)",
                to
            ));
        }
        let solde_from = self.comptes_cadeaux.get(from).copied().unwrap_or(0);
        if solde_from < montant {
            return Err(format!(
                "Solde cadeaux insuffisant: {} < {}",
                solde_from, montant
            ));
        }

        let solde_to = self.comptes_cadeaux.get(to).copied().unwrap_or(0);

        self.comptes_cadeaux
            .insert(from.to_string(), solde_from - montant);
        self.comptes_cadeaux
            .insert(to.to_string(), solde_to + montant);

        if !self.verifier() {
            self.comptes_cadeaux.insert(from.to_string(), solde_from);
            self.comptes_cadeaux.insert(to.to_string(), solde_to);
            return Err("Invariant casse — ROLLBACK".to_string());
        }

        Ok(())
    }
}

impl BlockOrb {
    /// Get solde d'un compte (compatibilite)
    pub fn get_solde(&self, uq_number: &str) -> u128 {
        self.comptes.get(uq_number).map(|p| p.solde).unwrap_or(0)
    }

    /// Set solde d'un compte — n'invente jamais de portefeuille.
    pub fn set_solde(&mut self, uq_number: &str, solde: u128) -> Result<(), String> {
        let p = self
            .comptes
            .get_mut(uq_number)
            .ok_or_else(|| format!("Compte {} absent du BlockOrb", uq_number))?;
        p.solde = solde;
        Ok(())
    }

    /// Ajouter a un compte — entrée de BORB : cadenas structurel exigé.
    pub fn ajouter_solde(&mut self, uq_number: &str, montant: u128) -> Result<(), String> {
        let p = self
            .comptes
            .get_mut(uq_number)
            .ok_or_else(|| format!("CADENAS_REQUIS: compte {} absent du BlockOrb", uq_number))?;
        if !p.porte_cadenas() {
            return Err(format!(
                "CADENAS_REQUIS: {} sans cadenas structurel",
                uq_number
            ));
        }
        p.ajouter(montant)
    }

    /// Retirer d'un compte
    pub fn retirer_solde(&mut self, uq_number: &str, montant: u128) -> Result<(), String> {
        let p = self
            .comptes
            .get_mut(uq_number)
            .ok_or("Compte introuvable")?;
        p.retirer(montant)
    }

    /// Sommer les soldes
    pub fn somme_soldes(&self) -> u128 {
        self.comptes.values().map(|p| p.solde).sum()
    }
}

impl Default for BlockOrb {
    fn default() -> Self {
        Self::new()
    }
}

// ===== TESTS =====
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_block_orb_initial() {
        let orb = BlockOrb::new();
        assert!(orb.verifier());
        assert_eq!(orb.total(), CAPACITE_TOTALE);
    }

    fn orb_avec_comptes() -> BlockOrb {
        let mut orb = BlockOrb::new();
        orb.creer_compte("UQ-A", 1000 * FEMTO_PAR_BORB, "AAAA-BBBB-CCCC-DDDD")
            .unwrap();
        orb.creer_compte("UQ-B", 500 * FEMTO_PAR_BORB, "EEEE-FFFF-0000-1111")
            .unwrap();
        orb
    }

    #[test]
    fn test_transaction_simple() {
        let mut orb = orb_avec_comptes();

        let montant = MontantBorb::from_borb(100);
        orb.traiter("UQ-A", "UQ-B", montant).unwrap();
        assert!(orb.verifier());
        assert_eq!(orb.total(), CAPACITE_TOTALE);
    }

    #[test]
    fn traiter_ne_cree_jamais_de_portefeuille_fantome() {
        let mut orb = orb_avec_comptes();
        let total_avant = orb.total();
        let comptes_avant = orb.comptes.len();

        // Destinataire inconnu = refus, pas de création implicite.
        let montant = MontantBorb::from_borb(10);
        assert!(orb.traiter("UQ-A", "UQ-FANTOME", montant).is_err());
        assert!(orb.traiter("UQ-INCONNU", "UQ-A", montant).is_err());
        assert_eq!(orb.comptes.len(), comptes_avant, "aucun compte invente");
        assert_eq!(orb.total(), total_avant);
    }

    #[test]
    fn credit_refuse_sans_cadenas_structurel() {
        let mut orb = orb_avec_comptes();
        // Compte tracé avec solde légitime (prélevé du Trésor) mais dont le
        // cadenas disparaît : plus rien ne doit pouvoir y entrer.
        orb.creer_compte("UQ-C", 0, "").unwrap_err();
        orb.creer_compte("UQ-D", 10 * FEMTO_PAR_BORB, "9999-8888-7777-6666")
            .unwrap();
        orb.comptes.get_mut("UQ-D").unwrap().cadenas = None;

        assert!(
            orb.traiter("UQ-D", "UQ-A", MontantBorb::from_borb(1))
                .is_err(),
            "sortie sans cadenas refusee"
        );
        assert!(
            orb.traiter("UQ-A", "UQ-D", MontantBorb::from_borb(1))
                .is_err(),
            "entree sans cadenas refusee"
        );
        assert!(orb
            .traiter("UQ-A", "UQ-B", MontantBorb::from_borb(1))
            .is_ok());

        assert!(orb.ajouter_solde("UQ-D", FEMTO_PAR_BORB).is_err());
        assert!(orb.crediter_cadeau("UQ-D", FEMTO_PAR_BORB).is_err());
        // cadeau légitime : débit d'abord l'enveloppe, puis le porteur
        orb.ubi_enveloppe -= FEMTO_PAR_BORB;
        assert!(orb.crediter_cadeau("UQ-A", FEMTO_PAR_BORB).is_ok());
        assert!(orb.verifier());
    }

    #[test]
    fn creer_compte_exige_un_cadenas() {
        let mut orb = BlockOrb::new();
        assert!(orb.creer_compte("UQ-X", 100, "").is_err());
        assert!(orb.creer_compte("UQ-X", 100, "NON_POSE").is_err());
        assert!(orb
            .creer_compte("UQ-X", 100, "DEVERROUILLE_AUTO_OWNER")
            .is_err());
        assert!(orb.creer_compte("UQ-X", 100, "1234-ABCD-5678-EF01").is_ok());
    }

    #[test]
    fn creer_compte_ne_cree_jamais_de_valeur() {
        let mut orb = BlockOrb::new();
        let total_avant = orb.total();
        orb.creer_compte("UQ-A", 1000 * FEMTO_PAR_BORB, "AAAA-BBBB-CCCC-DDDD")
            .unwrap();
        assert_eq!(
            orb.total(),
            total_avant,
            "la creation de compte preleve du Tresor, ne mint pas"
        );
        assert_eq!(
            orb.cases[0],
            31_110_000_000 * FEMTO_PAR_BORB - 1000 * FEMTO_PAR_BORB
        );
    }

    #[test]
    fn verifier_refuse_un_trou_d_un_borb() {
        let mut orb = BlockOrb::new();
        orb.cases[0] -= 1_000_000_000_000_000_000; // 1 BORB d'evaporation
        assert!(
            !orb.verifier(),
            "la tolerance doit etre bien plus petite qu'un BORB"
        );
    }

    #[test]
    fn verifier_accepte_l_arrondi_micro() {
        let mut orb = BlockOrb::new();
        orb.cases[0] -= 1_000_000_000_000; // 1 micro-BORB
        assert!(orb.verifier());
    }
}
