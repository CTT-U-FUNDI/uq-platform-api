// ====================
// 🔒 ESCROW BORB — P2P mobile
// ====================
//
// Ce que le P2P mobile met en jeu : du BORB, contre un paiement hors plateforme
// (mobile money, virement bancaire). Le BORB du vendeur est donc immobilisé dès
// la prise d'ordre ; il ne change de main qu'à la libération, et revient tel
// quel à l'annulation.
//
// `balance.points` reste la source de vérité des soldes et n'est jamais touché
// par un gel : seule `balance.frozen_points` porte l'immobilisation. Partout
// ailleurs dans l'application la disponibilité se calcule déjà comme
// `points - frozen_points`, donc un gel et un envoi se neutralisent sans
// réécrire les autres chemins.

use rusqlite::{params, Connection};

/// Parité du grand livre : `amount_points = BORB * 100`, soit 1 point = 0,01 BORB.
pub const POINTS_PAR_BORB: i64 = 100;

/// Types d'écritures posées par cet escrow dans la table `transactions`.
pub const TX_VENTE_P2P: &str = "p2p_sell";
pub const TX_ACHAT_P2P: &str = "p2p_buy";

#[derive(Debug, PartialEq, Eq)]
pub enum ErreurEscrow {
    MontantNonValide,
    TauxNonValide,
    SoldeInsuffisant,
    GelInsuffisant,
    CompteIntrouvable,
    CadenasRequis,
    Ecriture(String),
}

impl ErreurEscrow {
    pub fn message(&self) -> String {
        match self {
            ErreurEscrow::MontantNonValide => "Montant de commande non valide".to_string(),
            ErreurEscrow::TauxNonValide => "Taux de l'offre non valide".to_string(),
            ErreurEscrow::SoldeInsuffisant => {
                "BORB disponible insuffisant pour immobiliser cette commande".to_string()
            }
            ErreurEscrow::GelInsuffisant => {
                "Aucune immobilisation correspondante sur ce compte".to_string()
            }
            ErreurEscrow::CompteIntrouvable => "Base du compte inaccessible".to_string(),
            ErreurEscrow::CadenasRequis => {
                "L'acheteur n'a pas de cadenas structurel réel : le BORB ne peut pas entrer".to_string()
            }
            ErreurEscrow::Ecriture(e) => format!("Écriture escrow impossible ({})", e),
        }
    }
}

/// Quantité de BORB (en points) que représente un montant dans la devise de
/// l'offre, au taux « 1 BORB = X devise ». Arrondi au point le plus proche :
/// la grandeur appelée n'existe pas en dessous du point.
pub fn points_pour_devise(montant_devise: f64, taux_devise_par_borb: f64) -> Result<i64, ErreurEscrow> {
    if !montant_devise.is_finite() || montant_devise <= 0.0 {
        return Err(ErreurEscrow::MontantNonValide);
    }
    if !taux_devise_par_borb.is_finite() || taux_devise_par_borb <= 0.0 {
        return Err(ErreurEscrow::TauxNonValide);
    }
    let borb = montant_devise / taux_devise_par_borb;
    if !borb.is_finite() || borb <= 0.0 {
        return Err(ErreurEscrow::TauxNonValide);
    }
    let points = (borb * POINTS_PAR_BORB as f64).round();
    // Garde d'overflow : le cast float→entier sature en Rust, donc on refuse
    // explicitement tout ce qui ne tient pas dans un solde en points.
    if points < 1.0 || points >= i64::MAX as f64 {
        return Err(ErreurEscrow::MontantNonValide);
    }
    Ok(points as i64)
}

/// Quantité de BORB demandée par l'acheteur, ramenée aux points du grand livre.
/// L'arrondi se fait au point près : la fraction de point n'existe pas.
pub fn points_pour_borb(borb: f64) -> Result<i64, ErreurEscrow> {
    if !borb.is_finite() || borb <= 0.0 {
        return Err(ErreurEscrow::MontantNonValide);
    }
    let points = (borb * POINTS_PAR_BORB as f64).round();
    if points < 1.0 || points >= i64::MAX as f64 {
        return Err(ErreurEscrow::MontantNonValide);
    }
    Ok(points as i64)
}

/// Un gel n'a le droit de dépasser ni le solde ni ce qui n'est pas déjà gelé.
pub fn gel_authorise(points: i64, deja_gele: i64, a_geler: i64) -> bool {
    if a_geler <= 0 {
        return false;
    }
    match deja_gele.checked_add(a_geler) {
        Some(total_immobilise) => points >= total_immobilise,
        None => false,
    }
}

fn chemin_compte(uq: &str) -> String {
    format!("data/users/{}.db", uq.replace("-", "_").to_lowercase())
}

fn ouvrir_compte(uq: &str) -> Result<Connection, ErreurEscrow> {
    let chemin = chemin_compte(uq);
    if !std::path::Path::new(&chemin).exists() {
        return Err(ErreurEscrow::CompteIntrouvable);
    }
    let conn = Connection::open(&chemin).map_err(|e| ErreurEscrow::Ecriture(e.to_string()))?;
    let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
    if let Err(e) = crate::db::ensure_transactions_columns(&conn) {
        eprintln!("⚠️  Alignement schéma transactions (escrow {}): {}", uq, e);
    }
    Ok(conn)
}

/// Solde et immobilisation en cours sur un compte.
pub fn etat_compte(conn: &Connection) -> Result<(i64, i64), ErreurEscrow> {
    conn.query_row(
        "SELECT COALESCE(points, 0), COALESCE(frozen_points, 0) FROM balance WHERE id = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .map_err(|e| ErreurEscrow::Ecriture(e.to_string()))
}

/// Immobilise `a_geler` points chez `uq`. Le solde affiché ne bouge pas : seule
/// la partie disponible recule.
pub fn geler(uq: &str, a_geler: i64, horodatage: i64) -> Result<(), ErreurEscrow> {
    if a_geler <= 0 {
        return Err(ErreurEscrow::MontantNonValide);
    }
    let conn = ouvrir_compte(uq)?;
    let touche = conn
        .execute(
            "UPDATE balance
             SET frozen_points = COALESCE(frozen_points, 0) + ?1, updated_at = ?2
             WHERE id = 1 AND points - COALESCE(frozen_points, 0) >= ?1",
            params![a_geler, horodatage],
        )
        .map_err(|e| ErreurEscrow::Ecriture(e.to_string()))?;
    if touche != 1 {
        return Err(ErreurEscrow::SoldeInsuffisant);
    }
    Ok(())
}

/// Rend une immobilisation à son propriétaire sans rien créditer.
pub fn degeler(uq: &str, a_degeler: i64, horodatage: i64) -> Result<(), ErreurEscrow> {
    if a_degeler <= 0 {
        return Err(ErreurEscrow::MontantNonValide);
    }
    let conn = ouvrir_compte(uq)?;
    let touche = conn
        .execute(
            "UPDATE balance
             SET frozen_points = COALESCE(frozen_points, 0) - ?1, updated_at = ?2
             WHERE id = 1 AND COALESCE(frozen_points, 0) >= ?1",
            params![a_degeler, horodatage],
        )
        .map_err(|e| ErreurEscrow::Ecriture(e.to_string()))?;
    if touche != 1 {
        return Err(ErreurEscrow::GelInsuffisant);
    }
    Ok(())
}

/// Libère la commande : les points quittent le vendeur et son immobilisation en
/// même temps, puis entrent chez l'acheteur, avec une écriture de grand livre
/// de chaque côté.
pub fn liberer(
    vendeur: &str,
    acheteur: &str,
    points: i64,
    commande_id: i64,
    horodatage: i64,
) -> Result<(), ErreurEscrow> {
    if points <= 0 {
        return Err(ErreurEscrow::MontantNonValide);
    }
    // 🚪 PORTE BORB : le BORB livré n'entre chez l'acheteur que si son
    //    cadenas structurel est réel (identifiants) ET lié dans le BlockOrb.
    if let Err(e) = crate::kyc::cadenas::porte_cadenas_structurel(acheteur) {
        eprintln!("🚫 [ESCROW] livraison refusée pour {} : {}", acheteur, e);
        return Err(ErreurEscrow::CadenasRequis);
    }
    let lie_dans_orb = {
        let (orb, _garde) = crate::persistence::orb_charge_verrouille();
        orb.porte_cadenas(acheteur)
    };
    if !lie_dans_orb {
        eprintln!("🚫 [ESCROW] {} absent du BlockOrb ou cadenas non lié", acheteur);
        return Err(ErreurEscrow::CadenasRequis);
    }
    // Contrepartie écrite dans l'historique : pseudo et nom, jamais l'identifiant.
    let nom_vendeur = crate::uq_public_alias::get_public_display(vendeur);
    let nom_acheteur = crate::uq_public_alias::get_public_display(acheteur);
    let mut db_vendeur = ouvrir_compte(vendeur)?;
    let mut db_acheteur = ouvrir_compte(acheteur)?;

    // Les deux comptes forment une seule séquence : si l'un refuse, l'autre ne
    // part pas sans contrepartie.
    db_vendeur
        .execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| ErreurEscrow::Ecriture(e.to_string()))?;
    if let Err(e) = db_acheteur.execute_batch("BEGIN IMMEDIATE") {
        let _ = db_vendeur.execute_batch("ROLLBACK");
        return Err(ErreurEscrow::Ecriture(e.to_string()));
    }

    let suite = (|| -> Result<(), ErreurEscrow> {
        let cede = db_vendeur
            .execute(
                "UPDATE balance
                 SET points = points - ?1,
                     frozen_points = COALESCE(frozen_points, 0) - ?1,
                     updated_at = ?2
                 WHERE id = 1 AND points >= ?1 AND COALESCE(frozen_points, 0) >= ?1",
                params![points, horodatage],
            )
            .map_err(|e| ErreurEscrow::Ecriture(e.to_string()))?;
        if cede != 1 {
            return Err(ErreurEscrow::GelInsuffisant);
        }

        ecrire_grand_livre(
            &db_vendeur,
            vendeur,
            acheteur,
            TX_VENTE_P2P,
            points,
            commande_id,
            &nom_acheteur,
            horodatage,
        )?;
        resynchroniser_sous_unites(&db_vendeur, horodatage)?;

        db_acheteur
            .execute(
                "UPDATE balance SET points = points + ?1, updated_at = ?2 WHERE id = 1",
                params![points, horodatage],
            )
            .map_err(|e| ErreurEscrow::Ecriture(e.to_string()))?;
        ecrire_grand_livre(
            &db_acheteur,
            vendeur,
            acheteur,
            TX_ACHAT_P2P,
            points,
            commande_id,
            &nom_vendeur,
            horodatage,
        )?;
        resynchroniser_sous_unites(&db_acheteur, horodatage)?;
        Ok(())
    })();

    match suite {
        Ok(()) => {
            db_vendeur
                .execute_batch("COMMIT")
                .map_err(|e| ErreurEscrow::Ecriture(e.to_string()))?;
            if let Err(e) = db_acheteur.execute_batch("COMMIT") {
                // Le débit du vendeur est validé, le crédit de l'acheteur non :
                // c'est un incident à rejouer, pas une donnée à deviner.
                eprintln!("🚨 ESCROW commande {} : débit validé, crédit non commité ({})", commande_id, e);
                return Err(ErreurEscrow::Ecriture(e.to_string()));
            }
            Ok(())
        }
        Err(e) => {
            let _ = db_vendeur.execute_batch("ROLLBACK");
            let _ = db_acheteur.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

/// Contrepartie affichée : pseudo et nom, jamais l'identifiant technique.
fn ecrire_grand_livre(
    conn: &Connection,
    vendeur: &str,
    acheteur: &str,
    tx_type: &str,
    points: i64,
    commande_id: i64,
    contrepartie: &str,
    horodatage: i64,
) -> Result<(), ErreurEscrow> {
    let empreinte = hash_de_transaction(&format!(
        "UQ_P2P_ESCROW:{}:{}:{}:{}:{}",
        commande_id, vendeur, acheteur, points, horodatage
    ));
    let cadenas = hash_de_transaction(&format!("CADENAS:{}:{}", vendeur, horodatage));
    let cle_universelle = hash_de_transaction(&format!(
        "UQ_BORB_UNIVERSAL:{}",
        horodatage / 30
    ));
    let note = format!("P2P mobile · commande {}", commande_id);

    crate::local_history::enregistrer_transaction(
        conn,
        &crate::local_history::NouvelleTransaction {
            uq_number: acheteur,
            tx_type,
            amount: points,
            fee: 0,
            counterparty: contrepartie,
            memo: &note,
            borb_id: &empreinte,
            device: "p2p_escrow",
            os: "",
            browser: "",
            ip: "",
            cadenas_hash: &cadenas,
            internal_clock: horodatage,
            universal_key_hash: &cle_universelle,
            signature: "unsigned",
            created_at: horodatage,
            sender_uq: vendeur,
            recipient_uq: acheteur,
        },
    )
    .map_err(|e| ErreurEscrow::Ecriture(e.to_string()))?;
    Ok(())
}

/// Les sous-unités BORB sont le miroir de `balance.points` : elles suivent chaque
/// mouvement, comme sur le chemin d'envoi classique. Publique pour que la
/// conversion coffre ↔ portefeuille utilise la même correction.
pub(crate) fn resynchroniser_sous_unites(conn: &Connection, horodatage: i64) -> Result<(), ErreurEscrow> {
    let solde_apres: i64 = conn
        .query_row("SELECT points FROM balance WHERE id = 1", [], |row| row.get(0))
        .map_err(|e| ErreurEscrow::Ecriture(e.to_string()))?;
    conn.execute(
        "UPDATE my_borb_units SET balance = ?1, updated_at = ?2 WHERE unit = 'BORB'",
        params![solde_apres / POINTS_PAR_BORB, horodatage],
    )
    .map_err(|e| ErreurEscrow::Ecriture(e.to_string()))?;
    Ok(())
}

pub(crate) fn hash_de_transaction(source: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(source.as_bytes());
    hex::encode(hasher.finalize())
}

// ====================
// TESTS — arithmétique et gardes, sans base de données
// ====================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn taux_par_borb_ramene_la_quantite_de_borb() {
        // 2 400 CDF à 2 400 CDF le BORB = 1 BORB = 100 points.
        assert_eq!(points_pour_devise(2400.0, 2400.0), Ok(100));
        // 1 200 CDF = un demi-BORB = 50 points.
        assert_eq!(points_pour_devise(1200.0, 2400.0), Ok(50));
    }

    #[test]
    fn arrondi_au_point_le_plus_proche() {
        // 100 CDF à 2 261,56 CDF/BORB ≈ 0,0442 BORB → 4 points.
        assert_eq!(points_pour_devise(100.0, 2261.56), Ok(4));
        assert_eq!(points_pour_devise(1.0, 3.0), Ok(33));
    }

    #[test]
    fn taux_et_montant_deformes_sont_refuses() {
        assert_eq!(points_pour_devise(0.0, 2400.0), Err(ErreurEscrow::MontantNonValide));
        assert_eq!(points_pour_devise(-10.0, 2400.0), Err(ErreurEscrow::MontantNonValide));
        assert_eq!(points_pour_devise(10.0, 0.0), Err(ErreurEscrow::TauxNonValide));
        assert_eq!(points_pour_devise(10.0, -2400.0), Err(ErreurEscrow::TauxNonValide));
        assert_eq!(
            points_pour_devise(f64::NAN, 2400.0),
            Err(ErreurEscrow::MontantNonValide)
        );
    }

    #[test]
    fn quantite_de_borb_directe_en_points() {
        assert_eq!(points_pour_borb(1.0), Ok(100));
        assert_eq!(points_pour_borb(0.5), Ok(50));
        // 12,345 BORB -> 1235 points (0,005 près, arrondi au point le plus proche).
        assert_eq!(points_pour_borb(12.345), Ok(1235));
        assert_eq!(points_pour_borb(0.0), Err(ErreurEscrow::MontantNonValide));
        assert_eq!(points_pour_borb(-2.0), Err(ErreurEscrow::MontantNonValide));
        assert_eq!(points_pour_borb(f64::NAN), Err(ErreurEscrow::MontantNonValide));
        // Moins d'un demi-point : rien à immobiliser, la commande est refusée.
        assert_eq!(points_pour_borb(0.004), Err(ErreurEscrow::MontantNonValide));
    }

    #[test]
    fn gel_plafonne_a_ce_qui_n_est_pas_deja_gele() {
        assert!(gel_authorise(1000, 0, 1000));
        assert!(!gel_authorise(1000, 0, 1001));
        // 400 déjà immobilisés : il reste 600 de disponible.
        assert!(gel_authorise(1000, 400, 600));
        assert!(!gel_authorise(1000, 400, 601));
        assert!(!gel_authorise(1000, 400, 0));
        // Débordement i64 refusé plutôt que laissé passer.
        assert!(!gel_authorise(10, i64::MAX, 10));
    }

    #[test]
    fn gel_et_degel_refusent_une_quantite_nulle() {
        // Horodatage et compte ignorés : la gardeprecede toute ouverture de base.
        assert_eq!(geler("UQ-00000000", 0, 1), Err(ErreurEscrow::MontantNonValide));
        assert_eq!(degeler("UQ-00000000", -5, 1), Err(ErreurEscrow::MontantNonValide));
        assert_eq!(
            liberer("UQ-00000000", "UQ-996697110", 0, 1, 1),
            Err(ErreurEscrow::MontantNonValide)
        );
    }

    #[test]
    fn compte_inaccessible_est_signale_sans_panic() {
        // Chemin relatif au dossier de test : le compte n'existe pas ici.
        assert_eq!(
            geler("UQ-99999999", 100, 1),
            Err(ErreurEscrow::CompteIntrouvable)
        );
    }
}
