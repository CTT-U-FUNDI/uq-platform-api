//! Reconciliation escrow : detecte les points geles sans commande ouverte
//! (crash entre geler() et l'insert p2p_orders) et les degèle automatiquement.
use rusqlite::Connection;

/// Statuts qui gardent l'escrow actif.
const STATUTS_ESCROW: &str = "'pending_payment','paid','shipped','in_progress'";

/// Retourne (vendeur -> points geles en attente de commande).
pub fn excédent_escrow(index: &Connection) -> Vec<(String, i64)> {
    let mut out = Vec::new();
    // 1) somme escrow par vendeur pour les commandes encore ouvertes
    let mut stmt = match index.prepare(
        &format!("SELECT seller, COALESCE(SUM(escrow_points),0) FROM p2p_orders WHERE status IN ({}) GROUP BY seller", STATUTS_ESCROW)
    ) { Ok(s) => s, Err(_) => return out };
    let attendu: std::collections::HashMap<String, i64> = stmt
        .query_map([], |r| Ok((r.get::<_,String>(0)?, r.get::<_,i64>(1)?)))
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default();

    // 2) pour chaque compte user, compare frozen_points vs attendu
    let users_dir = std::path::Path::new("data/users");
    let Ok(read_dir) = std::fs::read_dir(users_dir) else { return out };
    for entry in read_dir.flatten() {
        let nom = entry.file_name().to_string_lossy().to_string();
        if !nom.starts_with("uq_") || !nom.ends_with(".db") { continue; }
        let Ok(conn) = Connection::open(entry.path()) else { continue; };
        let frozen: i64 = conn.query_row(
            "SELECT COALESCE(frozen_points,0) FROM balance WHERE id=1", [], |r| r.get(0)).unwrap_or(0);
        if frozen <= 0 { continue; }
        // uq du fichier : uq_732688531.db -> UQ-732688531
        let uq = format!("UQ-{}", nom.trim_start_matches("uq_").trim_end_matches(".db"));
        let attendus = attendu.get(&uq).copied().unwrap_or(0);
        if frozen > attendus {
            out.push((uq, frozen - attendus));
        }
    }
    out
}

/// Degèle les excédents trouves. A appeler au demarrage + periodiquement.
pub fn reconcilier() {
    let index_path = crate::uq_index::uq_index_path();
    let Ok(index) = Connection::open(index_path) else { return };
    for (uq, points) in excédent_escrow(&index) {
        let now = chrono::Utc::now().timestamp();
        match crate::p2p_escrow::degeler(&uq, points, now) {
            Ok(()) => eprintln!("🧹 ESCROW réconcilié : {} pts rendus a {}", points, uq),
            Err(e) => eprintln!("⚠️  ESCROW réconciliation échouée pour {}: {}", uq, e.message()),
        }
    }
}

/// Defi chrono P2P : commande pending_payment de plus de 1h -> annulee,
/// fonds vendeur degeles, quantite offre restauree.
pub fn expirer_p2p() {
    let Ok(index) = Connection::open(crate::uq_index::uq_index_path()) else { return };
    let limite = chrono::Utc::now().timestamp() - 3600;
    let vieux: Vec<(i64, String, i64)> = index.prepare(
        "SELECT id, seller, offer_id FROM p2p_orders WHERE status='pending_payment' AND created_at<?1")
        .and_then(|mut s| s.query_map([limite], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map(|rows| rows.flatten().collect()))
        .unwrap_or_default();
    for (id, vendeur, offre_id) in vieux {
        // degeler les points du vendeur
        let pts: i64 = index.query_row("SELECT COALESCE(escrow_points,0) FROM p2p_orders WHERE id=?1", [id], |r| r.get(0)).unwrap_or(0);
        if pts > 0 {
            let _ = crate::p2p_escrow::degeler(&vendeur, pts, chrono::Utc::now().timestamp());
        }
        // restaurer la quantite sur l'offre
        let _ = index.execute(
            "UPDATE p2p_offers SET taken_amount = MAX(0, COALESCE(taken_amount,0) - (SELECT COALESCE(amount,0) FROM p2p_orders WHERE id=?1)) WHERE id=?2",
            rusqlite::params![id, offre_id]);
        let _ = index.execute("UPDATE p2p_orders SET status='expired', updated_at=?2 WHERE id=?1",
            rusqlite::params![id, chrono::Utc::now().timestamp()]);
        eprintln!("⏰ P2P expiré : commande {} vendeur {} — {} pts rendus", id, vendeur, pts);
    }
}
