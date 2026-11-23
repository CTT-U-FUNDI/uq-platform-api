// ====================
// 📡 GOSSIP — Enveloppe de diffusion entre moteurs
// ====================
//
// Reprend la convention de signature de p2p_history (SHA-256
// secret|message). Secret partagé = JWT_SECRET, comme
// history_recovery::signature_par_defaut. L'empreinte sert de clé de
// déduplication ; le compteur de sauts borne le flood du mesh.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const PREFIXE_GOSSIP: &str = "UQ_GOSSIP:";

pub fn maintenant() -> i64 {
    chrono::Utc::now().timestamp()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diffusion {
    pub moteur: String,
    pub cle: String,
    pub version: u64,
    pub valeur: serde_json::Value,
    pub ts: i64,
    pub origine: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destinataire: Option<String>,
    pub saut: u8,
    pub signature: String,
}

impl Diffusion {
    pub fn message_a_signer(&self) -> String {
        format!(
            "UQ_GOSSIP_V1:{}:{}:{}:{}:{}:{}",
            self.origine,
            self.moteur,
            self.cle,
            self.version,
            self.ts,
            serde_json::to_string(&self.valeur).unwrap_or_else(|_| "{}".to_string())
        )
    }

    pub fn empreinte(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.message_a_signer().as_bytes());
        hex::encode(hasher.finalize())
    }
}

/// Secret partagé du déploiement (même convention que l'historique P2P)
pub fn secret_partage() -> String {
    std::env::var("JWT_SECRET").unwrap_or_else(|_| "uq_history_default_secret".to_string())
}

/// Identité stable de CET appareil, persistée dans data/engine_device.key
pub fn identifiant_appareil() -> String {
    let chemin = "data/engine_device.key";
    if let Ok(existant) = std::fs::read_to_string(chemin) {
        let propre = existant.trim().to_string();
        if !propre.is_empty() {
            return propre;
        }
    }
    let mut bytes = [0u8; 32];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut bytes);
    let nouveau = hex::encode(bytes);
    if let Some(dossier) = std::path::Path::new(chemin).parent() {
        let _ = std::fs::create_dir_all(dossier);
    }
    let _ = std::fs::write(chemin, &nouveau);
    nouveau
}

pub fn signer(diffusion: &mut Diffusion) {
    diffusion.signature =
        crate::p2p_history::signature_locale(&secret_partage(), &diffusion.message_a_signer());
}

/// Deux modes de vérification :
/// - « stricte » (défaut) : l'émetteur doit connaître le secret partagé
/// - « structurelle » : accepte toute diffusion non vide (mesh ouvert de
///   développement), la médiane collective et le quota bornent les dégâts
pub fn signature_valide(diffusion: &Diffusion) -> bool {
    if diffusion.moteur.is_empty() || diffusion.cle.is_empty() || diffusion.origine.is_empty() {
        return false;
    }
    match std::env::var("UQ_GOSSIP_VERIFICATION").ok().as_deref() {
        Some("structurelle") => !diffusion.signature.is_empty(),
        _ => {
            crate::p2p_history::signature_locale(&secret_partage(), &diffusion.message_a_signer())
                == diffusion.signature
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exemple() -> Diffusion {
        Diffusion {
            moteur: "oracle".to_string(),
            cle: "fx:USD".to_string(),
            version: 1,
            valeur: serde_json::json!({"prix": 1.0}),
            ts: maintenant(),
            origine: "test-appareil".to_string(),
            destinataire: None,
            saut: 0,
            signature: String::new(),
        }
    }

    #[test]
    fn signature_signe_et_verifie() {
        let mut d = exemple();
        signer(&mut d);
        assert!(signature_valide(&d));
        d.valeur = serde_json::json!({"prix": 9999.0});
        assert!(
            !signature_valide(&d),
            "un prix modifié doit casser la signature"
        );
    }

    #[test]
    fn empreinte_stable() {
        let mut d = exemple();
        signer(&mut d);
        assert_eq!(d.empreinte(), d.empreinte());
    }
}
