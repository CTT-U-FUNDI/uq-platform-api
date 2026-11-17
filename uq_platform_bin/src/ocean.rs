// ====================
// 🌊 OCEAN — La case Trésor (1ʳᵉ des 6 cases)
// ====================
//
// RÈGLE 0 (fondamentale) :
//   Le Trésor 1M du Profil 1 VIENT de la CASE OCEAN et y RETOURNE.
//   Ocean est la réserve principale des 40 Md BORB.
//
// INVARIANT PERMANENT :
//   Ocean (cases[0]) + Profil 1 (comptes["UQ-996697110"]) + Users
//   + les 5 autres cases + enveloppe UBI = 40 000 000 000 BORB, EXACT.
//
// Toute recharge est un TRANSFERT INTERNE : rien n'est créé, rien n'est
// détruit. Si Ocean ne peut pas couvrir, on refuse.
// ====================

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

/// Profil 1 = propriétaire de la plateforme
pub const PROFIL_1: &str = "UQ-996697110";

/// Trésor cible du Profil 1 : 1 000 000 BORB
pub const TRESOR_CIBLE_BORB: i64 = 1_000_000;

/// Seuil de déclenchement : 50 % du dernier solde
pub const SEUIL_RECHARGE_PCT: i64 = 50;

/// 1 BORB = 100 points
pub const POINTS_PAR_BORB: i64 = 100;

/// Fichier d'historique des mouvements Ocean
const OCEAN_HISTORY: &str = "data/ocean_history.json";

/// Conversion points → femto-BORB
pub fn points_vers_femto(points: i64) -> u128 {
    (points.max(0) as u128) * (crate::montant::FEMTO_PAR_BORB / POINTS_PAR_BORB as u128)
}

/// Conversion femto-BORB → points
pub fn femto_vers_points(femto: u128) -> i64 {
    (femto / (crate::montant::FEMTO_PAR_BORB / POINTS_PAR_BORB as u128)) as i64
}

// ====================
// ÉTAT D'OCEAN
// ====================

#[derive(Debug, Clone, Serialize)]
pub struct EtatOcean {
    pub ocean_femto: String,
    pub ocean_borb: f64,
    pub ocean_part_pct: f64,
    pub profil1_femto: String,
    pub profil1_borb: f64,
    pub profil1_points: i64,
    pub tresor_cible_borb: i64,
    pub seuil_recharge_borb: f64,
    pub recharge_necessaire: bool,
    pub manque_pour_cible_borb: f64,
    pub users_femto: String,
    pub total_femto: String,
    pub capacite_femto: String,
    pub invariant_ok: bool,
}

/// Lit l'état courant d'Ocean + Profil 1 (sans rien modifier).
pub fn etat() -> EtatOcean {
    let orb = crate::persistence::load_global_orb();

    let ocean = orb.cases[0];
    let profil1 = orb
        .comptes
        .get(PROFIL_1)
        .map(|p| p.solde)
        .unwrap_or(0);

    // Solde dépensable réel du Profil 1 (source de vérité)
    let profil1_points = lire_points(PROFIL_1).unwrap_or_else(|| femto_vers_points(profil1));

    let cible_points = TRESOR_CIBLE_BORB * POINTS_PAR_BORB;
    let seuil_points = cible_points * SEUIL_RECHARGE_PCT / 100;

    let femto_par_borb = crate::montant::FEMTO_PAR_BORB as f64;

    EtatOcean {
        ocean_femto: ocean.to_string(),
        ocean_borb: ocean as f64 / femto_par_borb,
        ocean_part_pct: if orb.total() > 0 {
            (ocean as f64 / orb.total() as f64) * 100.0
        } else {
            0.0
        },
        profil1_femto: profil1.to_string(),
        profil1_borb: profil1_points as f64 / POINTS_PAR_BORB as f64,
        profil1_points,
        tresor_cible_borb: TRESOR_CIBLE_BORB,
        seuil_recharge_borb: (seuil_points / POINTS_PAR_BORB) as f64,
        recharge_necessaire: profil1_points < seuil_points,
        manque_pour_cible_borb: ((cible_points - profil1_points).max(0) as f64)
            / POINTS_PAR_BORB as f64,
        users_femto: orb.somme_comptes().to_string(),
        total_femto: orb.total().to_string(),
        capacite_femto: crate::montant::CAPACITE_TOTALE.to_string(),
        invariant_ok: orb.verifier(),
    }
}

fn lire_points(uq: &str) -> Option<i64> {
    let chemin = format!("data/users/{}.db", uq.replace("-", "_").to_lowercase());
    let conn = Connection::open(chemin).ok()?;
    conn.query_row("SELECT COALESCE(points,0) FROM balance WHERE id = 1", [], |r| {
        r.get::<_, i64>(0)
    })
    .ok()
}

// ====================
// RECHARGE DEPUIS OCEAN (RÈGLE 2 / RÈGLE 6)
// ====================

#[derive(Debug, Clone, Serialize)]
pub struct ResultatRecharge {
    pub applique: bool,
    pub raison: String,
    pub ocean_avant_borb: f64,
    pub ocean_apres_borb: f64,
    pub profil1_avant_borb: f64,
    pub profil1_apres_borb: f64,
    pub montant_recu_borb: f64,
    pub retour_ocean_borb: f64,
    pub invariant_ok: bool,
}

/// Recharge le Trésor du Profil 1 **depuis Ocean**, pour atteindre 1 000 000 BORB.
///
/// RÈGLE 6 : si le Profil 1 détient déjà un solde, ce solde est d'abord
/// RENDU à Ocean, puis Ocean reverse exactement 1 000 000 BORB.
/// (un seul mouvement net, traçable, invariant préservé)
pub fn recharger_profil1(forcer: bool) -> Result<ResultatRecharge, String> {
    let (mut orb, _garde_orb) = crate::persistence::orb_charge_verrouille();

    let ocean_avant = orb.cases[0];
    let profil1_avant = orb.comptes.get(PROFIL_1).map(|p| p.solde).unwrap_or(0);
    let points_avant = lire_points(PROFIL_1).unwrap_or_else(|| femto_vers_points(profil1_avant));

    let cible_points = TRESOR_CIBLE_BORB * POINTS_PAR_BORB;
    let seuil_points = cible_points * SEUIL_RECHARGE_PCT / 100;

    // RÈGLE 2 : on ne recharge que si le solde est sous le seuil (sauf forçage owner)
    if !forcer && points_avant >= seuil_points {
        return Ok(ResultatRecharge {
            applique: false,
            raison: format!(
                "Solde du Profil 1 au-dessus du seuil ({} / {} BORB) — pas de recharge",
                points_avant / POINTS_PAR_BORB,
                seuil_points / POINTS_PAR_BORB
            ),
            ocean_avant_borb: ocean_avant as f64 / crate::montant::FEMTO_PAR_BORB as f64,
            ocean_apres_borb: ocean_avant as f64 / crate::montant::FEMTO_PAR_BORB as f64,
            profil1_avant_borb: points_avant as f64 / POINTS_PAR_BORB as f64,
            profil1_apres_borb: points_avant as f64 / POINTS_PAR_BORB as f64,
            montant_recu_borb: 0.0,
            retour_ocean_borb: 0.0,
            invariant_ok: orb.verifier(),
        });
    }

    // RÈGLE 6 : l'ancien solde retourne d'abord à Ocean
    let retour = profil1_avant;
    orb.cases[0] = orb.cases[0].saturating_add(retour);

    // Puis Ocean verse exactement la cible
    let cible_femto = points_vers_femto(cible_points);
    if orb.cases[0] < cible_femto {
        return Err(format!(
            "Ocean insuffisant : {} BORB disponibles, {} BORB requis",
            orb.cases[0] as f64 / crate::montant::FEMTO_PAR_BORB as f64,
            cible_femto as f64 / crate::montant::FEMTO_PAR_BORB as f64
        ));
    }
    orb.cases[0] -= cible_femto;

    // Le compte doit exister AVEC cadenas structurel : Ocean ne recharge
    // jamais un portefeuille fantôme (rien n'est sauvegardé en cas d'erreur).
    match orb.comptes.get_mut(PROFIL_1) {
        Some(p) if p.porte_cadenas() => p.solde = cible_femto,
        Some(_) => {
            return Err(format!(
                "OCEAN refuse : {} n'a pas de cadenas structurel lie", PROFIL_1
            ));
        }
        None => {
            return Err(format!(
                "OCEAN refuse : {} absent du BlockOrb (compte fantome)", PROFIL_1
            ));
        }
    }

    // INVARIANT : on refuse et on annule si l'égalité 40 Md n'est plus exacte
    if !orb.verifier() {
        return Err("Invariant casse — recharge Ocean annulee (aucune ecriture)".to_string());
    }

    // Écriture SQLite (solde dépensable du Profil 1)
    ecrire_points(PROFIL_1, cible_points, "recharge_ocean")?;

    orb.sceller_bloc();
    crate::persistence::save_global_orb(&orb)?;

    // Historique Ocean
    let _ = journaliser(
        "recharge_profil1",
        PROFIL_1,
        cible_points - points_avant,
        retour,
    );

    let femto_par_borb = crate::montant::FEMTO_PAR_BORB as f64;
    eprintln!(
        "🌊 Recharge Ocean → Profil 1 : +{} BORB (retour {} BORB) | Ocean {} → {} BORB",
        (cible_points - points_avant) as f64 / POINTS_PAR_BORB as f64,
        retour as f64 / femto_par_borb,
        ocean_avant as f64 / femto_par_borb,
        orb.cases[0] as f64 / femto_par_borb
    );

    Ok(ResultatRecharge {
        applique: true,
        raison: "Trésor rechargé depuis Ocean".to_string(),
        ocean_avant_borb: ocean_avant as f64 / femto_par_borb,
        ocean_apres_borb: orb.cases[0] as f64 / femto_par_borb,
        profil1_avant_borb: points_avant as f64 / POINTS_PAR_BORB as f64,
        profil1_apres_borb: cible_points as f64 / POINTS_PAR_BORB as f64,
        montant_recu_borb: (cible_points - points_avant) as f64 / POINTS_PAR_BORB as f64,
        retour_ocean_borb: retour as f64 / femto_par_borb,
        invariant_ok: orb.verifier(),
    })
}

/// Écrit le solde du Profil 1 dans sa DB locale (+ tables dérivées + chaînage).
fn ecrire_points(uq: &str, points: i64, motif: &str) -> Result<(), String> {
    let chemin = format!("data/users/{}.db", uq.replace("-", "_").to_lowercase());
    let conn = Connection::open(&chemin).map_err(|e| e.to_string())?;
    crate::local_history::init_tables(&conn)?;

    conn.execute(
        "UPDATE balance SET points = ?1, updated_at = ?2 WHERE id = 1",
        rusqlite::params![points, chrono::Utc::now().timestamp()],
    )
    .map_err(|e| e.to_string())?;

    conn.execute(
        "UPDATE wallet SET balance_points = ?1, balance_borb = ?2 WHERE id = 1",
        rusqlite::params![points, points as f64 / POINTS_PAR_BORB as f64],
    )
    .ok();

    conn.execute(
        "UPDATE my_borb_units SET balance = ?1 WHERE unit = 'BORB'",
        rusqlite::params![points / POINTS_PAR_BORB],
    )
    .ok();

    // Trace dans l'historique local (chaînée)
    let now = chrono::Utc::now().timestamp();
    let borb_id = crate::blockchain_lib::hash_transaction("OCEAN", uq, points as u64, 0, now);
    let prev = crate::local_history::dernier_borb_id(&conn);
    let bloc = crate::persistence::load_global_orb().block_number as i64;

    conn.execute(
        "INSERT INTO transactions (tx_type, amount, fee, counterparty, memo, created_at,
                                   sender_uq, recipient_uq, status, reference,
                                   borb_id, prev_hash, block_number)
         VALUES ('receive', ?1, 0, 'CASE OCEAN', ?2, ?3, 'CASE OCEAN', ?4, 'success', ?5, ?6, ?7, ?8)",
        rusqlite::params![
            points,
            motif,
            now,
            uq,
            format!("UQ-OCEAN-{}", now),
            borb_id,
            prev,
            bloc
        ],
    )
    .map_err(|e| e.to_string())?;

    crate::local_history::sceller(&conn, uq)?;
    Ok(())
}

// ====================
// HISTORIQUE OCEAN
// ====================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MouvementOcean {
    pub type_mouvement: String,
    pub uq: String,
    pub montant_points: i64,
    pub retour_points: i64,
    pub at: i64,
}

pub fn historique() -> Vec<MouvementOcean> {
    std::fs::read_to_string(OCEAN_HISTORY)
        .ok()
        .and_then(|c| serde_json::from_str::<Vec<MouvementOcean>>(&c).ok())
        .unwrap_or_default()
}

fn journaliser(type_mouvement: &str, uq: &str, montant_points: i64, retour_femto: u128) -> Result<(), String> {
    let mut h = historique();
    h.push(MouvementOcean {
        type_mouvement: type_mouvement.to_string(),
        uq: uq.to_string(),
        montant_points,
        retour_points: femto_vers_points(retour_femto),
        at: chrono::Utc::now().timestamp(),
    });
    if h.len() > 1000 {
        let n = h.len() - 1000;
        h.drain(0..n);
    }
    let json = serde_json::to_string_pretty(&h).map_err(|e| e.to_string())?;
    std::fs::write(OCEAN_HISTORY, json).map_err(|e| e.to_string())?;
    Ok(())
}

// ====================
// TESTS
// ====================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions_points_femto() {
        let f = points_vers_femto(100); // 1 BORB
        assert_eq!(f, crate::montant::FEMTO_PAR_BORB);
        assert_eq!(femto_vers_points(f), 100);
    }

    #[test]
    fn cible_1m_convertie_correctement() {
        let cible = points_vers_femto(TRESOR_CIBLE_BORB * POINTS_PAR_BORB);
        // 1 000 000 BORB = 10^6 * 10^18 femto
        assert_eq!(cible, 1_000_000u128 * crate::montant::FEMTO_PAR_BORB);
    }

    #[test]
    fn seuil_est_la_moitie() {
        let cible_points = TRESOR_CIBLE_BORB * POINTS_PAR_BORB;
        let seuil = cible_points * SEUIL_RECHARGE_PCT / 100;
        assert_eq!(seuil, cible_points / 2);
    }
}
