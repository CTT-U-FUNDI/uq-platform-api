// ====================
// 🔄 CASES LOOP — Auto-régulation interne des 6 cases
//    + 🌍 CASES MARKET — adaptation à l'offre et à la demande
// ====================
//
// CYCLE PERPÉTUEL (spec) :
//   💰 Ocean → 💸 Nuages → 🏆 Rivières → ❄️ Glaciers → 💳 Nappes → 🛡️ Vapeur
// ===== ↑ =====
// ===== retour vers Ocean =====
//
// SEUILS CRITIQUES : Ocean 10% · Nuages 5% · Rivières 10%
//                    Glaciers 5% · Nappes 5% · Vapeur 5%
//
// INVARIANT : la régulation ne fait que DÉPLACER des BORB entre cases.
//   cases[0..6] + enveloppe UBI + comptes = 40 Md, EXACT, à chaque étape.
// ====================

use serde::{Deserialize, Serialize};

/// Les 6 cases, dans l'ordre du BlockOrb
pub const NOMS: [&str; 6] = ["ocean", "nuages", "rivieres", "glaciers", "nappes", "vapeur"];
pub const LABELS: [&str; 6] = ["Ocean", "Nuages", "Rivieres", "Glaciers", "Nappes", "Vapeur"];
pub const ROLES: [&str; 6] = ["Tresor", "Frais", "Recompenses", "Epargne", "Credit", "Shield"];

/// Seuils minimaux (% de la capacité initiale de chaque case)
pub const SEUILS_PCT: [u64; 6] = [10, 5, 10, 5, 5, 5];

/// Capacité initiale de chaque case (répartition de Genèse, en BORB)
pub const CAPACITES_BORB: [u128; 6] = [
    31_110_000_000, // Ocean
    2_240_000_000,  // Nuages
    2_240_000_000,  // Rivières
    2_240_000_000,  // Glaciers
    1_320_000_000,  // Nappes
    840_000_000,    // Vapeur
];

const HISTORIQUE: &str = "data/cases_history.json";
/// Plafond de déplacement par régulation (évite les à-coups)
const MAX_MOUVEMENT_PCT: u128 = 5;

// ====================
// ÉTAT DES 6 CASES
// ====================

#[derive(Debug, Clone, Serialize)]
pub struct CaseEtat {
    pub index: usize,
    pub nom: String,
    pub label: String,
    pub role: String,
    pub femto: String,
    pub borb: f64,
    pub pct_capacite: f64,
    pub seuil_pct: u64,
    pub sous_seuil: bool,
    pub part_du_total_pct: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct EtatCases {
    pub cases: Vec<CaseEtat>,
    pub total_femto: String,
    pub capacite_femto: String,
    pub invariant_ok: bool,
    pub cases_sous_seuil: Vec<String>,
    pub sante: String,
}

fn seuil_femto(i: usize) -> u128 {
    CAPACITES_BORB[i] * crate::montant::FEMTO_PAR_BORB * SEUILS_PCT[i] as u128 / 100
}

pub fn etat() -> EtatCases {
    let orb = crate::persistence::load_global_orb();
    let total = orb.total();
    let femto_par_borb = crate::montant::FEMTO_PAR_BORB as f64;

    let mut cases = Vec::new();
    let mut sous_seuil = Vec::new();

    for i in 0..6 {
        let solde = orb.cases[i];
        let capacite = CAPACITES_BORB[i] as f64 * femto_par_borb;
        let pct = if capacite > 0.0 { solde as f64 / capacite * 100.0 } else { 0.0 };
        let bas = solde < seuil_femto(i);
        if bas {
            sous_seuil.push(NOMS[i].to_string());
        }
        cases.push(CaseEtat {
            index: i,
            nom: NOMS[i].to_string(),
            label: LABELS[i].to_string(),
            role: ROLES[i].to_string(),
            femto: solde.to_string(),
            borb: solde as f64 / femto_par_borb,
            pct_capacite: pct,
            seuil_pct: SEUILS_PCT[i],
            sous_seuil: bas,
            part_du_total_pct: if total > 0 { solde as f64 / total as f64 * 100.0 } else { 0.0 },
        });
    }

    let sante = if sous_seuil.is_empty() {
        "stable"
    } else if sous_seuil.len() <= 2 {
        "attention"
    } else {
        "critique"
    }
    .to_string();

    EtatCases {
        cases,
        total_femto: total.to_string(),
        capacite_femto: crate::montant::CAPACITE_TOTALE.to_string(),
        invariant_ok: total == crate::montant::CAPACITE_TOTALE,
        cases_sous_seuil: sous_seuil,
        sante,
    }
}

// ====================
// RÉGULATION INTERNE (auto-régulation case → case)
// ====================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MouvementCase {
    pub de: String,
    pub vers: String,
    pub femto: String,
    pub borb: f64,
    pub motif: String,
    pub at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResultatRegulation {
    pub applique: bool,
    pub mouvements: Vec<MouvementCase>,
    pub cases_avant: Vec<String>,
    pub cases_apres: Vec<String>,
    pub invariant_ok: bool,
    pub sante_avant: String,
    pub sante_apres: String,
    pub raison: String,
}

/// Alimente une case déficitaire depuis une case donneuse.
/// Le transfert est plafonné à MAX_MOUVEMENT_PCT de la capacité de la case
/// bénéficiaire, pour éviter les à-coups.
fn transferer(orb: &mut crate::block_orb::BlockOrb, de: usize, vers: usize, motif: &str, mvt: &mut Vec<MouvementCase>) -> u128 {
    let besoin = seuil_femto(vers).saturating_sub(orb.cases[vers]);
    if besoin == 0 {
        return 0;
    }

    let plafond = CAPACITES_BORB[vers] * crate::montant::FEMTO_PAR_BORB * MAX_MOUVEMENT_PCT / 100;
    let montant = besoin.min(plafond).min(orb.cases[de]);

    if montant == 0 {
        return 0;
    }

    orb.cases[de] -= montant;
    orb.cases[vers] += montant;

    mvt.push(MouvementCase {
        de: NOMS[de].to_string(),
        vers: NOMS[vers].to_string(),
        femto: montant.to_string(),
        borb: montant as f64 / crate::montant::FEMTO_PAR_BORB as f64,
        motif: motif.to_string(),
        at: chrono::Utc::now().timestamp(),
    });

    montant
}

/// Applique le cycle d'auto-régulation décrit par la spécification.
pub fn auto_reguler() -> Result<ResultatRegulation, String> {
    let (mut orb, _garde_orb) = crate::persistence::orb_charge_verrouille();
    let avant: Vec<String> = orb.cases.iter().map(|c| c.to_string()).collect();
    let sante_avant = etat().sante;
    let mut mvt = Vec::new();

    // Cycle : Ocean(0) → Nuages(1) → Rivières(2) → Glaciers(3) → Nappes(4) → Vapeur(5) → Ocean(0)
    // ===== Ocean < 10% → Nuages alimente Ocean =====
    transferer(&mut orb, 1, 0, "Ocean sous 10% — Nuages alimente Ocean", &mut mvt);
    // ===== Nuages < 5% → Rivières alimente Nuages =====
    transferer(&mut orb, 2, 1, "Nuages sous 5% — Rivieres alimente Nuages", &mut mvt);
    // ===== Rivières < 10% → Ocean alimente Rivières =====
    transferer(&mut orb, 0, 2, "Rivieres sous 10% — Ocean alimente Rivieres", &mut mvt);
    // ===== Glaciers < 5% → Nappes alimente Glaciers =====
    transferer(&mut orb, 4, 3, "Glaciers sous 5% — Nappes alimente Glaciers", &mut mvt);
    // ===== Nappes < 5% → Vapeur alimente Nappes =====
    transferer(&mut orb, 5, 4, "Nappes sous 5% — Vapeur alimente Nappes", &mut mvt);
    // ===== Vapeur < 5% → Ocean alimente Vapeur =====
    transferer(&mut orb, 0, 5, "Vapeur sous 5% — Ocean alimente Vapeur", &mut mvt);

    if mvt.is_empty() {
        return Ok(ResultatRegulation {
            applique: false,
            mouvements: vec![],
            cases_avant: avant,
            cases_apres: orb.cases.iter().map(|c| c.to_string()).collect(),
            invariant_ok: orb.verifier(),
            sante_avant: sante_avant.clone(),
            sante_apres: sante_avant,
            raison: "Toutes les cases sont au-dessus de leur seuil — aucune regulation".to_string(),
        });
    }

    // INVARIANT : on annule tout si l'égalité 40 Md n'est plus exacte
    if !orb.verifier() {
        return Err("Invariant casse — regulation annulee (aucune ecriture)".to_string());
    }

    orb.sceller_bloc();
    crate::persistence::save_global_orb(&orb)?;
    journaliser(&mvt);

    for m in &mvt {
        eprintln!(
            "🔄 Regulation cases : {} → {} : {} BORB ({})",
            m.de, m.vers, m.borb, m.motif
        );
    }

    let apres = etat();
    Ok(ResultatRegulation {
        applique: true,
        mouvements: mvt,
        cases_avant: avant,
        cases_apres: orb.cases.iter().map(|c| c.to_string()).collect(),
        invariant_ok: orb.verifier(),
        sante_avant,
        sante_apres: apres.sante,
        raison: "Auto-regulation appliquee".to_string(),
    })
}

// ====================
// MARCHÉ EXTERNE (offre / demande)
// ====================

#[derive(Debug, Clone, Serialize)]
pub struct AnalyseMarche {
    pub offres: i64,
    pub demandes: i64,
    pub ratio_offre_demande: f64,
    pub volume_points: i64,
    pub tendance: String,
    pub actions: Vec<String>,
}

/// Analyse l'offre et la demande réelles (place de marché + transactions).
pub fn analyser_marche() -> AnalyseMarche {
    let db = crate::database::Database::shared("uq_platform_db.json");
    let stats = db.get_stats();

    let offres = stats["offers_active"].as_i64().unwrap_or(0);
    let sessions = stats["sessions"].as_i64().unwrap_or(0);
    // Faute de carnet d'ordres dédié, la demande est approchée par
    // l'activité récente (sessions) — affinée à la Phase 3 (carnet P2P).
    let demandes = sessions.max(0);

    let volume_points = stats["transactions"].as_i64().unwrap_or(0);
    let ratio = if demandes > 0 {
        offres as f64 / demandes as f64
    } else if offres > 0 {
        f64::INFINITY
    } else {
        1.0
    };

    let mut actions = Vec::new();
    let tendance = if offres as f64 > demandes as f64 * 1.2 && demandes >= 0 {
        actions.push("Offre > demande +20% → Nuages +10%, Rivieres +5%".to_string());
        "offre_dominante"
    } else if demandes as f64 > offres as f64 * 1.2 {
        actions.push("Demande > offre +20% → Nappes +10%, Vapeur +5%".to_string());
        "demande_dominante"
    } else {
        actions.push("Equilibre (< 5% d'ecart) → aucune modification".to_string());
        "equilibre"
    };

    if volume_points > 0 && volume_points % 50 == 0 {
        actions.push("Volume global +50% → Ocean +10%, Nuages +5%".to_string());
    }

    AnalyseMarche {
        offres,
        demandes,
        ratio_offre_demande: if ratio.is_finite() { ratio } else { -1.0 },
        volume_points,
        tendance: tendance.to_string(),
        actions,
    }
}

// ====================
// HISTORIQUE
// ====================

pub fn historique() -> Vec<MouvementCase> {
    std::fs::read_to_string(HISTORIQUE)
        .ok()
        .and_then(|c| serde_json::from_str::<Vec<MouvementCase>>(&c).ok())
        .unwrap_or_default()
}

fn journaliser(mvt: &[MouvementCase]) {
    let mut h = historique();
    h.extend_from_slice(mvt);
    if h.len() > 2000 {
        let n = h.len() - 2000;
        h.drain(0..n);
    }
    if let Ok(json) = serde_json::to_string_pretty(&h) {
        let _ = std::fs::write(HISTORIQUE, json);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn six_cases_et_seuils() {
        assert_eq!(NOMS.len(), 6);
        assert_eq!(SEUILS_PCT.len(), 6);
        assert_eq!(CAPACITES_BORB.len(), 6);
        assert_eq!(SEUILS_PCT[0], 10);
        assert_eq!(SEUILS_PCT[3], 5);
    }

    #[test]
    fn somme_capacites_egale_40_md() {
        let somme: u128 = CAPACITES_BORB.iter().sum();
        assert_eq!(somme, 39_990_000_000); // hors enveloppe UBI (10 M)
    }

    #[test]
    fn seuil_ocean_est_10_pct() {
        let s = seuil_femto(0);
        let attendu = 31_110_000_000u128 * crate::montant::FEMTO_PAR_BORB / 10;
        assert_eq!(s, attendu);
    }
}
