// ====================
// 🗄️ STORE — État des moteurs, budget d'octets par appareil
// ====================
//
// data/uq_engines.db (SQLite, WAL). Le quota (UQ_ENGINE_QUOTA_MB, défaut
// 100) porte sur la taille réelle des fichiers du store ; à chaque
// écriture, si le quota est dépassé on purge le plus ancien
// (historique puis états périmés) jusqu'à repasser sous le budget.
//
// L'état est stocké PAR ORIGINE (moteur, cle, origine) : la fusion est
// un last-write-win par appareil émetteur, l'agrégation collective
// (médiane, sommes) se calcule à la lecture sur toutes les origines.

use rusqlite::Connection;
use serde_json::json;

use super::gossip::Diffusion;

pub struct StoreMoteurs {
    conn: Connection,
    quota_octets: u64,
}

fn taille_fichier(chemin: &str) -> u64 {
    std::fs::metadata(chemin).map(|m| m.len()).unwrap_or(0)
}

impl StoreMoteurs {
    pub fn ouvrir() -> Self {
        let chemin = std::env::var("UQ_ENGINES_DB").unwrap_or_else(|_| "data/uq_engines.db".to_string());
        Self::ouvrir_chemin(&chemin)
    }

    pub fn ouvrir_chemin(chemin: &str) -> Self {
        let conn = match Connection::open(chemin) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("⚠️  Store moteurs '{}' indisponible ({}) — repli en mémoire", chemin, e);
                Connection::open_in_memory().expect("store moteurs mémoire")
            }
        };
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS moteur_etat (
               moteur TEXT NOT NULL,
               cle TEXT NOT NULL,
               origine TEXT NOT NULL,
               version INTEGER NOT NULL,
               valeur TEXT NOT NULL,
               ts INTEGER NOT NULL,
               hash TEXT NOT NULL,
               PRIMARY KEY (moteur, cle, origine)
             );
             CREATE TABLE IF NOT EXISTS moteur_histo (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               moteur TEXT NOT NULL,
               cle TEXT NOT NULL,
               origine TEXT NOT NULL,
               version INTEGER NOT NULL,
               valeur TEXT NOT NULL,
               ts INTEGER NOT NULL,
               hash TEXT NOT NULL UNIQUE
             );
             CREATE TABLE IF NOT EXISTS moteur_vus (
               hash TEXT PRIMARY KEY,
               recu INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS moteur_appareils (
               uq TEXT NOT NULL,
               appareil TEXT NOT NULL,
               addr TEXT NOT NULL,
               appaire INTEGER NOT NULL,
               PRIMARY KEY (uq, appareil)
             );
             CREATE INDEX IF NOT EXISTS idx_etat_cle ON moteur_etat (moteur, cle);
             CREATE INDEX IF NOT EXISTS idx_histo_ts ON moteur_histo (ts);",
        )
        .expect("schéma store moteurs");

        let quota_mb: u64 = std::env::var("UQ_ENGINE_QUOTA_MB")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(100);
        Self { conn, quota_octets: quota_mb * 1024 * 1024 }
    }

    pub fn octets_utilises(&self) -> u64 {
        let page_size: u64 = self
            .conn
            .query_row("PRAGMA page_size", [], |r| r.get::<_, i64>(0))
            .unwrap_or(0) as u64;
        let page_count: u64 = self
            .conn
            .query_row("PRAGMA page_count", [], |r| r.get::<_, i64>(0))
            .unwrap_or(0) as u64;
        let chemin = self
            .conn
            .path()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "data/uq_engines.db".to_string());
        page_size * page_count + taille_fichier(&format!("{}-wal", chemin))
    }

    pub fn quota_depasse(&self) -> bool {
        self.octets_utilises() > self.quota_octets
    }

    /// Purge : l'historique d'abord (le plus vieux), puis les états les
    /// plus anciens par moteur. Borné pour ne jamais bloquer le bus.
    fn purger(&mut self) {
        for _ in 0..50 {
            if !self.quota_depasse() {
                return;
            }
            let suppr = match self.conn.execute(
                "DELETE FROM moteur_histo WHERE id IN
                 (SELECT id FROM moteur_histo ORDER BY ts ASC LIMIT 500)",
                [],
            ) {
                Ok(n) if n > 0 => n,
                _ => self
                    .conn
                    .execute(
                        "DELETE FROM moteur_etat WHERE rowid IN
                         (SELECT rowid FROM moteur_etat ORDER BY ts ASC LIMIT 200)",
                        [],
                    )
                    .unwrap_or(0),
            };
            if suppr == 0 {
                let _ = self.conn.execute("DELETE FROM moteur_vus WHERE recu < (SELECT MAX(recu) FROM moteur_vus) - 86400", []);
                let _ = self.conn.execute("VACUUM", []);
                return;
            }
        }
    }

    pub fn deja_vu(&self, hash: &str) -> bool {
        self.conn
            .query_row("SELECT 1 FROM moteur_vus WHERE hash = ?1", [hash], |_| Ok(()))
            .is_ok()
    }

    fn marquer_vu(&mut self, hash: &str, recu: i64) {
        let _ = self.conn.execute(
            "INSERT OR IGNORE INTO moteur_vus (hash, recu) VALUES (?1, ?2)",
            rusqlite::params![hash, recu],
        );
    }

    pub fn version_origine(&self, moteur: &str, cle: &str, origine: &str) -> u64 {
        self.conn
            .query_row(
                "SELECT version FROM moteur_etat WHERE moteur=?1 AND cle=?2 AND origine=?3",
                rusqlite::params![moteur, cle, origine],
                |r| r.get::<_, i64>(0),
            )
            .map(|v| v.max(0) as u64)
            .unwrap_or(0)
    }

    /// Ligne d'état existante pour (moteur, cle, origine) : version, ts, hash.
    fn ligne_etat(&self, moteur: &str, cle: &str, origine: &str) -> Option<(u64, i64, String)> {
        self.conn
            .query_row(
                "SELECT version, ts, hash FROM moteur_etat WHERE moteur=?1 AND cle=?2 AND origine=?3",
                rusqlite::params![moteur, cle, origine],
                |r| Ok((r.get::<_, i64>(0)?.max(0) as u64, r.get::<_, i64>(1)?, r.get::<_, String>(2)?)),
            )
            .ok()
    }

    /// Fusion last-write-win par (moteur, cle, origine) : seule une version
    /// STRICTEMENT supérieure remplace l'état. Les versions égales sont des
    /// doublons ou des conflits → rejetées. Retourne true si retenue.
    pub fn appliquer(&mut self, d: &Diffusion) -> Result<bool, String> {
        let hash = d.empreinte();
        if let Some((version_locale, _, _)) = self.ligne_etat(&d.moteur, &d.cle, &d.origine) {
            if d.version <= version_locale {
                return Ok(false);
            }
        }
        self.conn
            .execute(
                "INSERT INTO moteur_etat (moteur, cle, origine, version, valeur, ts, hash)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)
                 ON CONFLICT(moteur, cle, origine) DO UPDATE SET
                   version=excluded.version, valeur=excluded.valeur,
                   ts=excluded.ts, hash=excluded.hash",
                rusqlite::params![d.moteur, d.cle, d.origine, d.version as i64, serde_json::to_string(&d.valeur).unwrap_or_default(), d.ts, hash],
            )
            .map_err(|e| e.to_string())?;
        self.marquer_vu(&hash, d.ts);
        let _ = self.conn.execute(
            "INSERT OR IGNORE INTO moteur_histo (moteur, cle, origine, version, valeur, ts, hash)
             VALUES (?1,?2,?3,?4,?5,?6,?7)",
            rusqlite::params![d.moteur, d.cle, d.origine, d.version as i64, serde_json::to_string(&d.valeur).unwrap_or_default(), d.ts, hash],
        );
        if self.quota_depasse() {
            self.purger();
        }
        Ok(true)
    }

    /// Toutes les lignes d'état d'un moteur (une par origine).
    pub fn lignes(&self, moteur: &str) -> Vec<(String, String, serde_json::Value, i64)> {
        let mut stmt = match self.conn.prepare(
            "SELECT cle, origine, valeur, ts FROM moteur_etat WHERE moteur = ?1",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        let rows = match stmt.query_map([moteur], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                serde_json::from_str::<serde_json::Value>(&r.get::<_, String>(2)?).unwrap_or(serde_json::Value::Null),
                r.get::<_, i64>(3)?,
            ))
        }) {
            Ok(v) => v,
            Err(_) => return vec![],
        };
        rows.filter_map(|x| x.ok()).collect()
    }

    /// Médiane des « valeur.prix » de toutes origines pour une clé.
    pub fn mediane_prix(&self, moteur: &str, cle: &str) -> Option<(f64, usize)> {
        let mut prix: Vec<f64> = self
            .lignes(moteur)
            .into_iter()
            .filter(|(c, _, _, _)| c == cle)
            .filter_map(|(_, _, v, _)| v.get("prix").and_then(|p| p.as_f64()))
            .collect();
        if prix.is_empty() {
            return None;
        }
        prix.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let milieu = prix.len() / 2;
        let mediane = if prix.len() % 2 == 0 {
            (prix[milieu - 1] + prix[milieu]) / 2.0
        } else {
            prix[milieu]
        };
        Some((mediane, prix.len()))
    }

    pub fn compte_lignes(&self, table: &str) -> i64 {
        let sql = match table {
            "etat" => "SELECT COUNT(*) FROM moteur_etat",
            "histo" => "SELECT COUNT(*) FROM moteur_histo",
            "vus" => "SELECT COUNT(*) FROM moteur_vus",
            "appareils" => "SELECT COUNT(*) FROM moteur_appareils",
            _ => return 0,
        };
        self.conn
            .query_row(sql, [], |r| r.get::<_, i64>(0))
            .unwrap_or(0)
    }

    /// Adresses des appareils appairés d'un compte (pour la synchro ciblée).
    pub fn appareils_de(&self, uq: &str) -> Vec<String> {
        let mut stmt = match self.conn.prepare("SELECT addr FROM moteur_appareils WHERE uq = ?1") {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map([uq], |r| r.get::<_, String>(0))
            .map(|rows| rows.filter_map(|x| x.ok()).collect())
            .unwrap_or_default()
    }

    pub fn appairer(&mut self, uq: &str, appareil: &str, addr: &str, maintenant: i64) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO moteur_appareils (uq, appareil, addr, appaire) VALUES (?1,?2,?3,?4)
                 ON CONFLICT(uq, appareil) DO UPDATE SET addr=excluded.addr, appaire=excluded.appaire",
                rusqlite::params![uq, appareil, addr, maintenant],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn liste_appareils(&self) -> Vec<(String, String, String, i64)> {
        let mut stmt = match self
            .conn
            .prepare("SELECT uq, appareil, addr, appaire FROM moteur_appareils ORDER BY appaire DESC LIMIT 100")
        {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
            ))
        })
        .map(|rows| rows.filter_map(|x| x.ok()).collect())
        .unwrap_or_default()
    }

    pub fn stats(&self) -> serde_json::Value {
        json!({
            "octets_utilises": self.octets_utilises(),
            "quota_octets": self.quota_octets,
            "quota_mb": self.quota_octets / (1024 * 1024),
            "lignes_etat": self.compte_lignes("etat"),
            "lignes_histo": self.compte_lignes("histo"),
            "empreintes_vues": self.compte_lignes("vus"),
            "appareils_appaires": self.compte_lignes("appareils"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::gossip;
    use super::*;

    fn diffusion(moteur: &str, cle: &str, origine: &str, version: u64, prix: f64) -> Diffusion {
        let mut d = Diffusion {
            moteur: moteur.to_string(),
            cle: cle.to_string(),
            version,
            valeur: json!({"prix": prix}),
            ts: gossip::maintenant(),
            origine: origine.to_string(),
            destinataire: None,
            saut: 0,
            signature: String::new(),
        };
        gossip::signer(&mut d);
        d
    }

    fn store_test(nom: &str) -> StoreMoteurs {
        let dir = std::env::temp_dir().join(format!("uq_engines_test_{}_{}", nom, std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        StoreMoteurs::ouvrir_chemin(dir.join("test.db").to_str().unwrap())
    }

    #[test]
    fn fusion_last_write_win_par_origine() {
        let mut s = store_test("lww");
        assert!(s.appliquer(&diffusion("oracle", "fx:USD", "A", 1, 2200.0)).unwrap());
        assert!(!s.appliquer(&diffusion("oracle", "fx:USD", "A", 1, 2300.0)).unwrap(), "version égale = rejet");
        assert!(s.appliquer(&diffusion("oracle", "fx:USD", "A", 2, 100.0)).unwrap(), "version 2 passe");
        assert!(s.appliquer(&diffusion("oracle", "fx:USD", "A", 3, 2450.0)).unwrap());
        assert!(!s.appliquer(&diffusion("oracle", "fx:USD", "A", 2, 999.0)).unwrap(), "version ancienne = rejet");
        let (mediane, origines) = s.mediane_prix("oracle", "fx:USD").unwrap();
        assert_eq!(origines, 1, "une seule origine après fusion");
        assert!((mediane - 2450.0).abs() < 1e-9, "l'état retenu est la version 3");
    }

    #[test]
    fn mediane_collective_multi_origines() {
        let mut s = store_test("mediane");
        for (o, p) in [("A", 100.0), ("B", 110.0), ("C", 90.0), ("D", 500.0)] {
            s.appliquer(&diffusion("oracle", "crypto:X", o, 1, p)).unwrap();
        }
        let (mediane, n) = s.mediane_prix("oracle", "crypto:X").unwrap();
        assert_eq!(n, 4);
        assert!((mediane - 105.0).abs() < 1e-9, "médiane de 90/100/110/500 = 105, l'origine folle est diluée");
    }

    #[test]
    fn dedup_par_empreinte() {
        let mut s = store_test("dedup");
        let d = diffusion("fraude", "sanction:UQ-1:abc", "A", 1, 1.0);
        let hash = d.empreinte();
        s.appliquer(&d).unwrap();
        assert!(s.deja_vu(&hash));
    }
}
