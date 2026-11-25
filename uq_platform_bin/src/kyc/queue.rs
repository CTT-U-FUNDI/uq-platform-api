// ====================
// 📋 FILE D'ATTENTE KYC
// ====================
//
// Les dossiers sont traites 1 par 1 par le proprietaire.
//
// ====================

use crate::kyc::dossier::{KycDossier, KycStatus};
use serde::{Deserialize, Serialize};
use sha2::Digest;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KycQueue {
    pub dossiers: Vec<KycDossier>,
}

impl KycQueue {
    pub fn new() -> Self {
        Self {
            dossiers: Vec::new(),
        }
    }

    /// Ajoute un dossier a la file
    pub fn ajouter(&mut self, dossier: KycDossier) {
        // Verifier si deja present
        if let Some(existing) = self
            .dossiers
            .iter_mut()
            .find(|d| d.uq_number == dossier.uq_number)
        {
            *existing = dossier;
        } else {
            self.dossiers.push(dossier);
        }
    }

    pub fn document_deja_utilise(&self, pays: &str, numero: &str, sauf_uq: &str) -> bool {
        let pays = normaliser_identifiant(pays);
        let numero = normaliser_identifiant(numero);
        self.dossiers.iter().any(|dossier| {
            dossier.uq_number != sauf_uq
                && normaliser_identifiant(&dossier.pays) == pays
                && normaliser_identifiant(&dossier.document_number) == numero
                && !numero.is_empty()
        })
    }

    /// Retourne le prochain dossier en attente
    pub fn prochain(&self) -> Option<&KycDossier> {
        self.dossiers
            .iter()
            .find(|d| d.statut == KycStatus::EnAttente)
    }

    /// Liste tous les dossiers en attente
    pub fn en_attente(&self) -> Vec<&KycDossier> {
        self.dossiers
            .iter()
            .filter(|d| d.statut == KycStatus::EnAttente)
            .collect()
    }

    /// Valide un dossier : enregistre le HASH de vérification dans
    /// l'index (preuve + niveau débloqué) puis ANONYMISE le dossier —
    /// les données KYC brutes (nom, document, photos) ne persistent pas.
    pub fn valider(&mut self, uq_number: &str, par: &str) -> Result<(), String> {
        let dossier = self
            .dossiers
            .iter_mut()
            .find(|d| d.uq_number == uq_number)
            .ok_or("Dossier introuvable")?;

        // Hash SHA-256 des champs d'identité = preuve de vérification.
        // C'est TOUT ce qui est conservé — jamais les données brutes.
        let brut = format!(
            "{}|{}|{}|{}|{}",
            dossier.uq_number,
            dossier.document_number,
            dossier.date_naissance,
            dossier.nom,
            dossier.prenom
        );
        let hash = format!(
            "{:x}",
            sha2::Sha256::digest(brut.as_bytes())
        );

        dossier.statut = KycStatus::Verifie;
        dossier.decision_at = Some(chrono::Utc::now().timestamp());
        dossier.decision_par = Some(par.to_string());
        // Bonus promo réservé à l'inscription : le $1 n'est crédité
        // qu'ici, une fois le compte réellement vérifié.
        crate::api_handlers::promo_sur_kyc_verifie(uq_number);

        // Persiste le hash + niveau dans l'index (débloque restrictions).
        let index = crate::uq_index::UqIndex::shared("uq_index.db");
        let _ = index.set_kyc_verification(uq_number, &hash, 1);

        // Anonymise : purge les données personnelles du dossier JSON.
        dossier.anonymiser();
        Ok(())
    }

    /// Refuse un dossier : marque le refus dans l'index et anonymise.
    pub fn refuser(&mut self, uq_number: &str, raison: &str, par: &str) -> Result<(), String> {
        let dossier = self
            .dossiers
            .iter_mut()
            .find(|d| d.uq_number == uq_number)
            .ok_or("Dossier introuvable")?;

        dossier.statut = KycStatus::Refuse;
        dossier.decision_at = Some(chrono::Utc::now().timestamp());
        dossier.decision_par = Some(par.to_string());
        dossier.raison_refus = Some(raison.to_string());

        // Refus : pas de hash de vérification, juste la trace du refus.
        let index = crate::uq_index::UqIndex::shared("uq_index.db");
        let _ = index.set_kyc_refuse(uq_number);

        // Anonymise aussi les refusés — les données ne persistent pas.
        dossier.anonymiser();
        Ok(())
    }

    /// Compte les dossiers en attente
    pub fn count_en_attente(&self) -> usize {
        self.dossiers
            .iter()
            .filter(|d| d.statut == KycStatus::EnAttente)
            .count()
    }
}

fn normaliser_identifiant(valeur: &str) -> String {
    valeur
        .chars()
        .filter(|caractere| caractere.is_alphanumeric())
        .flat_map(char::to_uppercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detecte_un_document_deja_soumis_apres_normalisation() {
        let mut queue = KycQueue::new();
        let mut dossier = KycDossier::new("UQ-1".to_string());
        dossier.pays = "Congo (RD)".to_string();
        dossier.document_number = "ab-123 45".to_string();
        queue.ajouter(dossier);

        assert!(queue.document_deja_utilise("congo rd", "AB12345", "UQ-2"));
        assert!(!queue.document_deja_utilise("congo rd", "AB12345", "UQ-1"));
        assert!(!queue.document_deja_utilise("CD", "AB12345", "UQ-2"));
        assert!(!queue.document_deja_utilise("congo rd", "", "UQ-2"));
    }
}
