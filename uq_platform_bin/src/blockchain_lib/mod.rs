// ====================
// ⛓️  BLOCKCHAIN LIB — Noyau UQ (primitives pures, sans I/O)
// ====================
//
// Ce module ne touche NI fichier NI base de données : uniquement du
// calcul. Il est utilisé par :
//   - block_orb.rs        (BlockOrb à taille fixe)
//   - persistence.rs      (charge / sauvegarde + garde de taille)
//   - api_handlers.rs     (écriture des historiques locaux)
//   - history_recovery.rs (récupération par voisins)
//   - auto_cleanup.rs     (purge > 1 an)
//
// RÈGLES DE LA SPÉCIFICATION :
//   - masse monétaire fixe : 40 000 000 000 BORB (invariant exact)
//   - le BlockOrb ne grandit JAMAIS (≤ 50 KB, pas d'historique dedans)
//   - l'historique vit dans la DB locale de chaque utilisateur
//   - la récupération exige la confirmation de 3 voisins minimum
//   - les historiques locaux de plus d'un an sont auto-supprimés
// ====================

use sha2::{Digest, Sha256};

// ====================
// CONSTANTES DU PROTOCOLE
// ====================

/// Taille maximale du BlockOrb global (spec : « ~50 KB max, JAMAIS plus »)
pub const MAX_BLOCKORB_BYTES: usize = 50 * 1024;

/// Durée de vie maximale d'un historique local (1 an)
pub const HISTORY_MAX_AGE_SECS: i64 = 365 * 24 * 60 * 60;

/// Nombre minimum de voisins pour valider une récupération
pub const RECOVERY_MIN_VALIDATORS: usize = 3;

/// Préfixes de domaine (évite les collisions entre types de hash)
const DOMAIN_TX: &str = "UQ_TX_V1";
const DOMAIN_BLOCK: &str = "UQ_BLOCK_V1";
const DOMAIN_CHAIN: &str = "UQ_CHAIN_V1";
const DOMAIN_FRAGMENT: &str = "UQ_FRAGMENT_V1";

// ====================
// HASH DE BASE
// ====================

pub fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

// ====================
// IDENTIFIANT DE TRANSACTION (borb_id)
// ====================

/// Calcule le `borb_id` d'une transaction : identifiant unique et
/// reproductible (mêmes entrées = même hash, donc vérifiable par un voisin).
pub fn hash_transaction(
    sender: &str,
    recipient: &str,
    amount_points: u64,
    fee_points: u64,
    timestamp: i64,
) -> String {
    sha256_hex(
        format!(
            "{}:{}:{}:{}:{}:{}",
            DOMAIN_TX, sender, recipient, amount_points, fee_points, timestamp
        )
        .as_bytes(),
    )
}

/// Message canonique signé par le client (Ed25519 côté téléphone)
pub fn message_signature(
    sender: &str,
    recipient: &str,
    amount_points: u64,
    fee_points: u64,
    timestamp: i64,
    borb_id: &str,
) -> String {
    format!(
        "UQ_TX_V1:{}:{}:{}:{}:{}:{}",
        sender, recipient, amount_points, fee_points, timestamp, borb_id
    )
}

// ====================
// CHAÎNAGE DE L'HISTORIQUE LOCAL
// ====================

/// Une ligne d'historique local (telle que stockée dans `transactions`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxRecord {
    pub borb_id: String,
    pub prev_hash: String,
    pub sender_uq: String,
    pub recipient_uq: String,
    pub amount: i64,
    pub fee: i64,
    pub timestamp: i64,
}

impl TxRecord {
    /// Empreinte d'une ligne (utilisée dans le chaînage)
    pub fn empreinte(&self) -> String {
        sha256_hex(
            format!(
                "{}|{}|{}|{}|{}|{}|{}",
                self.borb_id,
                self.prev_hash,
                self.sender_uq,
                self.recipient_uq,
                self.amount,
                self.fee,
                self.timestamp
            )
            .as_bytes(),
        )
    }
}

/// Empreinte de la chaîne complète (scellé de l'historique).
/// L'ordre compte : c'est l'ordre chronologique de la DB locale.
pub fn chain_hash(txs: &[TxRecord]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(DOMAIN_CHAIN.as_bytes());
    for tx in txs {
        hasher.update(tx.empreinte().as_bytes());
    }
    hex::encode(hasher.finalize())
}

/// Vérifie que chaque ligne pointe bien sur la précédente
/// (`prev_hash` == `borb_id` de la ligne d'avant).
pub fn verify_chain(txs: &[TxRecord]) -> bool {
    if txs.len() < 2 {
        return true;
    }
    for i in 1..txs.len() {
        if txs[i].prev_hash != txs[i - 1].borb_id {
            return false;
        }
    }
    true
}

// ====================
// EN-TÊTE DE BLOC (BlockOrb à taille fixe)
// ====================

/// Tout ce qui entre dans le hash de bloc. Le BlockOrb ne stocke que
/// ces valeurs : aucune liste, donc taille bornée.
#[derive(Debug, Clone)]
pub struct BlockHeader {
    pub block_number: u64,
    pub last_block_at: i64,
    pub somme_cases: u128,
    pub ubi_enveloppe: u128,
    pub somme_comptes: u128,
    pub prev_hash: String,
}

impl BlockHeader {
    pub fn hash(&self) -> String {
        sha256_hex(
            format!(
                "{}:{}:{}:{}:{}:{}:{}",
                DOMAIN_BLOCK,
                self.block_number,
                self.last_block_at,
                self.somme_cases,
                self.ubi_enveloppe,
                self.somme_comptes,
                self.prev_hash
            )
            .as_bytes(),
        )
    }
}

// ====================
// FRAGMENTS D'HISTORIQUE (récupération par voisins)
// ====================

/// Empreinte d'un fragment fourni par un voisin : lie le fournisseur,
/// le bénéficiaire et la transaction, pour qu'un fragment ne puisse pas
/// être rejoué pour un autre compte.
pub fn hash_fragment(provider_uq: &str, lost_uq: &str, borb_id: &str, prev_hash: &str) -> String {
    sha256_hex(
        format!(
            "{}:{}:{}:{}:{}",
            DOMAIN_FRAGMENT, provider_uq, lost_uq, borb_id, prev_hash
        )
        .as_bytes(),
    )
}

// ====================
// TESTS
// ====================

#[cfg(test)]
mod tests {
    use super::*;

    fn tx(id: &str, prev: &str, ts: i64) -> TxRecord {
        TxRecord {
            borb_id: id.to_string(),
            prev_hash: prev.to_string(),
            sender_uq: "UQ-1".to_string(),
            recipient_uq: "UQ-2".to_string(),
            amount: 100,
            fee: 1,
            timestamp: ts,
        }
    }

    #[test]
    fn borb_id_est_reproductible() {
        let a = hash_transaction("UQ-1", "UQ-2", 100, 1, 1000);
        let b = hash_transaction("UQ-1", "UQ-2", 100, 1, 1000);
        let c = hash_transaction("UQ-1", "UQ-2", 101, 1, 1000);
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn chainage_detecte_une_rupture() {
        let t1 = tx("aaa", "", 1);
        let t2 = tx("bbb", "aaa", 2);
        let t3 = tx("ccc", "bbb", 3);
        assert!(verify_chain(&[t1.clone(), t2.clone(), t3.clone()]));

        let mut casse = t3.clone();
        casse.prev_hash = "zzz".to_string();
        assert!(!verify_chain(&[t1, t2, casse]));
    }

    #[test]
    fn chain_hash_change_si_une_ligne_change() {
        let t1 = tx("aaa", "", 1);
        let t2 = tx("bbb", "aaa", 2);
        let h1 = chain_hash(&[t1.clone(), t2.clone()]);

        let mut modifie = t2.clone();
        modifie.amount = 999;
        let h2 = chain_hash(&[t1, modifie]);

        assert_ne!(h1, h2);
    }

    #[test]
    fn header_hash_depend_du_contenu() {
        let h = BlockHeader {
            block_number: 1,
            last_block_at: 1000,
            somme_cases: 10,
            ubi_enveloppe: 5,
            somme_comptes: 3,
            prev_hash: "genesis".to_string(),
        };
        let a = h.hash();
        let mut h2 = h.clone();
        h2.block_number = 2;
        assert_ne!(a, h2.hash());
        assert_eq!(a.len(), 64);
    }
}
