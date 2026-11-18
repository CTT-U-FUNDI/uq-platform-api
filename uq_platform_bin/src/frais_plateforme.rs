// ====================
// FRAIS DE PLATEFORME — source unique du barème et du partage 6 Cases
// ====================
//
// Barème (règle produit, aucune valeur inventée ici) :
//   - envoi de devise ............ 0,1 %
//   - retrait ................... 0,1 % fiat / 1 % crypto
//   - conversion BORB ↔ devise ... 0,1 % (PIN obligatoire, cf. handler)
//   - ordre marché .............. 0,1 % fiat / 1 % crypto
//   - ordre limite .............. 0 % tant qu'il n'est pas exécuté
//   - BORB ↔ BORB ............... 0 % (jamais de frais)
//
// Partage de chaque frais prélevé :
//   40 % Case 2 (Frais) → finance Earn
//   20 % Case 3 (Récompenses)
//   15 % Case 1 (Trésor)
//   10 % Case 4 (Épargne)
//   10 % Case 5 (Crédit)
//    5 % Case 6 (Bouclier)

use rusqlite::{params, Connection, Transaction};

/// Ordre des Cases dans le BlockOrb : Trésor, Frais, Récompenses,
/// Épargne, Crédit, Bouclier.
pub const REPARTITION_CASES: [f64; 6] = [0.15, 0.40, 0.20, 0.10, 0.10, 0.05];

pub const PART_CASE_FRAIS: f64 = REPARTITION_CASES[1];

pub const TAUX_FIAT: f64 = 0.001;
pub const TAUX_CRYPTO: f64 = 0.01;

pub const DEVISES_EARN: [&str; 3] = ["USD", "CDF", "EUR"];

/// Devises fiat reconnues par les portefeuilles virtuels.
pub const DEVISES_FIAT: [&str; 32] = [
    "CDF", "XAF", "XOF", "NGN", "KES", "ZAR", "MAD", "GHS", "UGX", "TZS", "EGP", "RWF",
    "BIF", "MZN", "AOA", "ETB", "USD", "EUR", "GBP", "CHF", "JPY", "CNY", "CAD", "AUD",
    "INR", "BRL", "MXN", "SEK", "NOK", "DKK", "PLN", "TRY",
];

pub fn est_crypto(code: &str) -> bool {
    matches!(
        code.trim().to_uppercase().as_str(),
        "BTC"
            | "ETH"
            | "USDT"
            | "USDC"
            | "BNB"
            | "XRP"
            | "SOL"
            | "ADA"
            | "DOGE"
            | "TRX"
            | "DOT"
            | "MATIC"
            | "LTC"
            | "SHIB"
            | "AVAX"
            | "LINK"
            | "ATOM"
            | "XLM"
            | "ALGO"
            | "XMR"
    )
}

pub fn est_borb(code: &str) -> bool {
    matches!(code.trim().to_uppercase().as_str(), "BORB" | "BORB")
}

pub fn est_fiat(code: &str) -> bool {
    DEVISES_FIAT.contains(&code.trim().to_uppercase().as_str())
}

/// Devise retirable : fiat ou crypto, jamais la réserve BORB.
pub fn est_retirable(code: &str) -> bool {
    est_fiat(code) || est_crypto(code)
}

/// Taux de frais configurables en DB (settings.frais_taux_fiat /
/// settings.frais_taux_crypto dans uq_index.db) — modifiables sans redeploy.
/// Fallback : les constantes historiques.
fn lire_taux_db(cle: &str, defaut: f64) -> f64 {
    rusqlite::Connection::open(crate::uq_index::uq_index_path())
        .ok()
        .and_then(|c| c.query_row(
            "SELECT value FROM settings WHERE key = ?1",
            rusqlite::params![cle], |r| r.get::<_, String>(0)).ok())
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v >= 0.0 && *v < 1.0)
        .unwrap_or(defaut)
}

/// Taux lus en DB a chaque appel -> modifiables sans redeploy.
pub fn taux_frais(code: &str) -> f64 {
    if est_crypto(code) {
        lire_taux_db("frais_taux_crypto", TAUX_CRYPTO)
    } else {
        lire_taux_db("frais_taux_fiat", TAUX_FIAT)
    }
}

/// Lit les deux taux (endpoint owner).
pub fn taux_actuels() -> serde_json::Value {
    serde_json::json!({
        "frais_taux_fiat": lire_taux_db("frais_taux_fiat", TAUX_FIAT),
        "frais_taux_crypto": lire_taux_db("frais_taux_crypto", TAUX_CRYPTO),
    })
}

/// Ecrit un taux en DB (endpoint owner). Retourne Err si invalide.
pub fn definir_taux(cle: &str, valeur: f64) -> Result<(), String> {
    if !valeur.is_finite() || valeur < 0.0 || valeur >= 1.0 {
        return Err("Taux invalide (0 <= t < 1)".to_string());
    }
    let cle = match cle {
        "fiat" => "frais_taux_fiat",
        "crypto" => "frais_taux_crypto",
        _ => return Err("Cle inconnue (fiat|crypto)".to_string()),
    };
    let conn = rusqlite::Connection::open(crate::uq_index::uq_index_path()).map_err(|e| e.to_string())?;
    conn.execute("CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT)", []).map_err(|e| e.to_string())?;
    conn.execute("INSERT INTO settings (key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        rusqlite::params![cle, valeur.to_string()]).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn devise_earn(brut: &str) -> Option<String> {
    let code = brut.trim().to_uppercase();
    if DEVISES_EARN.contains(&code.as_str()) {
        Some(code)
    } else {
        None
    }
}

pub fn arrondi_centimes(valeur: f64) -> f64 {
    if !valeur.is_finite() {
        return 0.0;
    }
    (valeur * 100.0).round() / 100.0
}

/// Frais en centimes d'une opération, arrondis au centime.
pub fn frais_centimes(montant_centimes: i64, code: &str) -> i64 {
    if montant_centimes <= 0 {
        return 0;
    }
    (montant_centimes as f64 * taux_frais(code)).round() as i64
}

/// Frais d'une conversion BORB ↔ devise. BORB ↔ BORB est gratuit par règle.
pub fn frais_conversion(from: &str, to: &str, montant: f64) -> f64 {
    if est_borb(from) && est_borb(to) {
        return 0.0;
    }
    let devise = if est_borb(from) { to } else { from };
    arrondi_centimes(montant * taux_frais(devise))
}

/// Répartit un frais entre les 6 Cases, au centime près.
/// Méthode du plus grand reste : la somme des parts est exactement le frais.
pub fn repartir(frais: f64) -> [f64; 6] {
    let frais = arrondi_centimes(frais.max(0.0));
    let total_cents = (frais * 100.0).round() as i64;
    if total_cents <= 0 {
        return [0.0; 6];
    }

    let mut cents = [0i64; 6];
    let mut restes: [(f64, usize); 6] = [(0.0, 0); 6];
    let mut attribue = 0i64;
    for i in 0..6 {
        let exact = total_cents as f64 * REPARTITION_CASES[i];
        let base = exact.floor() as i64;
        cents[i] = base;
        attribue += base;
        restes[i] = (exact - base as f64, i);
    }

    let manque = (total_cents - attribue).max(0) as usize;
    restes.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    for &(_, index) in restes.iter().take(manque) {
        cents[index] += 1;
    }

    [
        cents[0] as f64 / 100.0,
        cents[1] as f64 / 100.0,
        cents[2] as f64 / 100.0,
        cents[3] as f64 / 100.0,
        cents[4] as f64 / 100.0,
        cents[5] as f64 / 100.0,
    ]
}

pub fn assurer_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS platform_fees (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            source TEXT NOT NULL,
            reference TEXT NOT NULL DEFAULT '',
            uq TEXT NOT NULL,
            currency TEXT NOT NULL,
            amount REAL NOT NULL,
            case_tresor REAL NOT NULL,
            case_frais REAL NOT NULL,
            case_recompenses REAL NOT NULL,
            case_epargne REAL NOT NULL,
            case_credit REAL NOT NULL,
            case_bouclier REAL NOT NULL,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_platform_fees_created
            ON platform_fees(created_at);
        CREATE INDEX IF NOT EXISTS idx_platform_fees_source
            ON platform_fees(source, currency);

        CREATE TABLE IF NOT EXISTS earn_pool (
            currency TEXT PRIMARY KEY,
            collected REAL NOT NULL DEFAULT 0,
            distributed REAL NOT NULL DEFAULT 0,
            updated_at INTEGER NOT NULL
        );",
    )
}

/// Enregistre un frais, son partage 6 Cases, et alimente Case 2 → Earn
/// lorsque la devise est éligible (USD, CDF, EUR).
pub fn enregistrer_tx(
    tx: &Transaction<'_>,
    source: &str,
    reference: &str,
    uq: &str,
    currency: &str,
    frais: f64,
    created_at: i64,
) -> rusqlite::Result<[f64; 6]> {
    let parts = repartir(frais);
    tx.execute(
        "INSERT INTO platform_fees
            (source, reference, uq, currency, amount,
             case_tresor, case_frais, case_recompenses,
             case_epargne, case_credit, case_bouclier, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            source,
            reference,
            uq,
            currency,
            parts.iter().sum::<f64>(),
            parts[0],
            parts[1],
            parts[2],
            parts[3],
            parts[4],
            parts[5],
            created_at
        ],
    )?;

    if devise_earn(currency).is_some() && parts[1] > 0.0 {
        tx.execute(
            "INSERT INTO earn_pool (currency, collected, distributed, updated_at)
             VALUES (?1, ?2, 0, ?3)
             ON CONFLICT(currency) DO UPDATE SET
                 collected = earn_pool.collected + excluded.collected,
                 updated_at = excluded.updated_at",
            params![currency.trim().to_uppercase(), parts[1], created_at],
        )?;
    }

    Ok(parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repartition_exacte_des_frais() {
        let parts = repartir(100.0);
        assert_eq!(parts, [15.0, 40.0, 20.0, 10.0, 10.0, 5.0]);
        assert!((parts.iter().sum::<f64>() - 100.0).abs() < 1e-9);
    }

    #[test]
    fn un_centime_de_frais_va_a_case2() {
        let parts = repartir(0.01);
        assert_eq!(parts, [0.0, 0.01, 0.0, 0.0, 0.0, 0.0]);
        assert!((parts.iter().sum::<f64>() - 0.01).abs() < 1e-9);
    }

    #[test]
    fn envoi_de_100_usd_preleve_0_1_pour_cent() {
        let frais = frais_centimes(10_000, "USD");
        assert_eq!(frais, 10);
        assert_eq!(10_000 - frais, 9_990);
        assert_eq!(frais_centimes(10_000, "BTC"), 100);
    }

    #[test]
    fn borb_borb_toujours_gratuit() {
        assert_eq!(frais_conversion("BORB", "BORB", 500.0), 0.0);
        assert_eq!(frais_conversion("BORB", "BORB", 500.0), 0.0);
        assert!((frais_conversion("BORB", "USD", 500.0) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn case2_finance_earn_pour_les_devises_eligibles() {
        let conn = Connection::open_in_memory().unwrap();
        assurer_tables(&conn).unwrap();
        let tx = conn.unchecked_transaction().unwrap();

        let parts = enregistrer_tx(&tx, "send", "", "UQ-1", "USD", 10.0, 1).unwrap();
        assert_eq!(parts[1], 4.0);
        enregistrer_tx(&tx, "market", "order:1", "UQ-1", "BTC", 10.0, 1).unwrap();
        tx.commit().unwrap();

        let usd: f64 = conn
            .query_row(
                "SELECT collected FROM earn_pool WHERE currency = 'USD'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!((usd - 4.0).abs() < 1e-9);

        let btc: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM earn_pool WHERE currency = 'BTC'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(btc, 0);

        let total: f64 = conn
            .query_row("SELECT SUM(amount) FROM platform_fees", [], |row| row.get(0))
            .unwrap();
        assert!((total - 20.0).abs() < 1e-9);
    }
}
