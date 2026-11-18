// ====================
// DEMANDE CALCULATOR — Modèle C
// ====================
//
// Demande = Σ(Soldes) + (Volume 24h × Vitesse)
//
// Ce module calcule la demande totale en USD pour le régulateur.
//
// Le prix lu dans `prix.json` est la seule source de la valeur USD : s'il
// est absent ou illisible, le calcul s'arrête. Un repli à 1 USD inventerait
// une demande, donc un prix, qui se réécrirait dans `prix.json` au cycle
// suivant — le faux taux deviendrait le taux officiel par rétroaction.
//
// ====================

/// Représente l'état de la demande
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DemandeState {
    /// Somme de tous les soldes (en BORB)
    pub total_soldes_borb: u128,
    /// Volume des transactions 24h (en BORB)
    pub volume_24h_borb: u128,
    /// Vitesse de circulation = volume / soldes
    pub vitesse: f64,
    /// Demande totale (en USD)
    pub demande_totale_usd: u128,
    /// Prix calculé
    pub prix_calcule: String,
    /// Dernière mise à jour
    pub updated_at: i64,
}

impl DemandeState {
    /// Calcule la demande totale selon le Modèle C
    /// `prix_usd` vient de `prix.json` ; il doit être fini et strictement
    /// positif, sinon la demande n'a pas de valeur USD calculable.
    pub fn calculer(total_soldes_borb: u128, volume_24h_borb: u128, prix_usd: f64) -> Self {
        // 1. Vitesse de circulation
        let vitesse = if total_soldes_borb == 0 {
            0.0
        } else {
            volume_24h_borb as f64 / total_soldes_borb as f64
        };
        
        // 2. Demande = Soldes + (Volume × Vitesse)
        let volume_effectif = (volume_24h_borb as f64 * vitesse) as u128;
        let demande_borb = total_soldes_borb + volume_effectif;
        
        // 3. Convertir en USD
        let demande_usd = (demande_borb as f64 * prix_usd) as u128;
        
        // 4. Calculer le nouveau prix
        let nouveau_prix = crate::regulateur::Regulateur::calculer_prix(demande_usd);
        
        Self {
            total_soldes_borb,
            volume_24h_borb,
            vitesse,
            demande_totale_usd: demande_usd,
            prix_calcule: nouveau_prix.to_string(),
            updated_at: chrono::Utc::now().timestamp(),
        }
    }
}

/// Calcule la demande en parcourant tous les comptes.
/// Err() si le prix en stock est inutilisable : jamais de recalcul sur un prix inventé.
pub fn calculer_demande_globale() -> Result<DemandeState, String> {
    // 1. Lire le prix actuel
    let prix_state = crate::persistence::load_prix();
    let prix_usd: f64 = prix_state.prix_actuel.trim().parse().map_err(|e: std::num::ParseFloatError| {
        format!("prix_actuel illisible ({}) : aucun recalcul de la demande", e)
    })?;
    if !prix_usd.is_finite() || prix_usd <= 0.0 {
        return Err(format!(
            "prix_actuel non exploitable ({}) : la demande ne sera pas calculée sur un prix inventé",
            prix_state.prix_actuel
        ));
    }

    // 2. Parcourir tous les comptes dans uq_index.db
    let total_soldes = scanner_tous_les_soldes();

    // 3. Calculer le volume 24h (depuis WAL ou DBs users)
    let volume_24h = scanner_volume_24h();

    // 4. Calculer la demande
    Ok(DemandeState::calculer(
        total_soldes,
        volume_24h,
        prix_usd,
    ))
}

/// Scanne tous les soldes dans uq_index.db + data/users/
fn scanner_tous_les_soldes() -> u128 {
    let mut total: u128 = 0;
    
    // 1. Lire uq_index.db pour avoir la liste des comptes
    let idx = crate::uq_index::UqIndex::shared(&crate::uq_index::uq_index_path());
    
    // Pour chaque compte, ouvrir sa DB et lire balance
    let accounts_dir = "data/users";
    
    if let Ok(entries) = std::fs::read_dir(accounts_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("db") {
                if let Ok(conn) = rusqlite::Connection::open(&path) {
                    // Essayer plusieurs formats de solde
                    let balance: i64 = conn.query_row(
                        "SELECT COALESCE(points, 0) FROM balance WHERE id = 1",
                        [],
                        |row| row.get(0),
                    ).unwrap_or(0);
                    
                    // Convertir points → BORB (1 BORB = 100 points)
                    let borb = (balance / 100) as u128;
                    total += borb;
                }
            }
        }
    }
    
    eprintln!("💰 Soldes totaux : {} BORB", total);
    total
}

/// Scanne le volume des transactions des dernières 24h
fn scanner_volume_24h() -> u128 {
    let mut total: u128 = 0;
    let now = chrono::Utc::now().timestamp();
    let cutoff = now - 86400; // 24h
    
    let accounts_dir = "data/users";
    
    if let Ok(entries) = std::fs::read_dir(accounts_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("db") {
                if let Ok(conn) = rusqlite::Connection::open(&path) {
                    // Compter les transactions 24h
                    let count: i64 = conn.query_row(
                        "SELECT COALESCE(SUM(amount), 0) FROM transactions WHERE created_at > ?1",
                        rusqlite::params![cutoff],
                        |row| row.get(0),
                    ).unwrap_or(0);
                    
                    // Convertir points → BORB
                    let borb = (count / 100) as u128;
                    total += borb;
                }
            }
        }
    }
    
    eprintln!("📊 Volume 24h : {} BORB", total);
    total
}
