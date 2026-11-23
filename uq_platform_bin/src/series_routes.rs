// ====================
// Séries temporelles privées (graphiques Coffre et Trading)
// ====================
//
//   GET /api/borb/balance/series/{id}   USER (même compte, ou owner)
//   GET /api/trading/performance/{uq}     USER (même compte, ou owner)
//
// Ces lectures agrègent TOUT l'historique concerné : les routes de liste
// (`/api/borb/transactions`, `/api/trading/orders`) sont plafonnées à 50 et
// 100 lignes et ne peuvent pas servir de base à une courbe sur un an.
// Rien n'est interpolé : un jour sans mouvement n'apparaît pas dans la série.

use actix_web::{web, HttpRequest, HttpResponse};
use rusqlite::{params, Connection};
use serde_json::json;
use std::collections::{HashMap, VecDeque};
use std::path::Path;

const SERIES_PERIODES: [(&str, i64); 3] = [
    ("7j", 7 * 86_400),
    ("30j", 30 * 86_400),
    ("1an", 365 * 86_400),
];

/// Signe d'une ligne d'historique local sur le solde interne, en points.
///
/// `send` débite `amount + fee` (le coût total est prélevé à l'envoi),
/// `receive` et `ubi` créditent `amount`. Les écritures d'escrow P2P suivent la
/// même logique : `p2p_sell` est une sortie, `p2p_buy` une entrée. Les types
/// écrits par d'autres versions du logiciel ne sont pas devinés : ils sont
/// comptés à part et signalés, pour que l'état de réconciliation reste honnête.
enum EffetSolde {
    Entree,
    Sortie,
    Inconnu,
}

fn effet_solde(tx_type: &str) -> EffetSolde {
    match tx_type {
        "send" | "transfer_sent" | "p2p_sell" => EffetSolde::Sortie,
        "receive" | "transfer_received" | "ubi" | "p2p_buy" => EffetSolde::Entree,
        _ => EffetSolde::Inconnu,
    }
}

/// « 2026-10-05 » depuis un horodatage en secondes (UTC, comme `date(...,'unixepoch')`).
fn date_jour(secondes: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp(secondes, 0)
        .map(|instant| instant.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| format!("epoch-{}", secondes))
}

/// Devise dans laquelle une paire est cotée : la partie après le séparateur.
///
/// Le prix d'une paire est exprimé en unités de `quote` pour 1 `base`
/// (`trading_routes::prix_paire`), donc deux paires de `quote` différentes ne
/// décrivent pas la même grandeur. Additionner des PnL ou des volumes par-dessus
/// plusieurs devises produirait une valeur inventée : tout est agrégé par devise.
fn devise_de_la_paire(paire: &str) -> String {
    paire
        .split_once('-')
        .map(|(_, quote)| quote.to_uppercase())
        .unwrap_or_else(|| paire.to_uppercase())
}

/// Une exécution telle qu'elle est stockée dans `trading_trades`.
#[derive(Debug, Clone)]
pub struct Execution {
    pub paire: String,
    pub cote: String,
    pub prix: f64,
    pub quantite: f64,
    pub cree: i64,
}

/// Résultat de l'appariement FIFO, prêt à être agrégé et affiché.
pub struct Appariement {
    /// `devise -> jour -> (pnl réalisé, volume, nombre d'exécutions)`.
    pub par_devise: HashMap<String, HashMap<String, (f64, f64, i64)>>,
    /// Lots achetés jamais reliquides, par paire.
    pub lots_restants: HashMap<String, VecDeque<(f64, f64)>>,
    /// Ventes qui n'ont trouvé aucun lot à appairer.
    pub lots_non_apparies: i64,
}

/// Apparie achats et ventes en FIFO, jour par jour et devise par devise.
///
/// Les executions doivent être triées dans l'ordre chronologique : un lot créé
/// après la vente ne peut pas l'appairer.
pub fn apparier_fifo(executions: &[Execution]) -> Appariement {
    let mut lots: HashMap<String, VecDeque<(f64, f64)>> = HashMap::new();
    let mut par_devise: HashMap<String, HashMap<String, (f64, f64, i64)>> = HashMap::new();
    let mut lots_non_apparies = 0i64;

    for execution in executions {
        let Execution {
            paire,
            cote,
            prix,
            quantite,
            cree,
        } = execution;
        let devise = devise_de_la_paire(paire);
        let jour = date_jour(*cree);
        let case = par_devise
            .entry(devise)
            .or_default()
            .entry(jour)
            .or_insert((0.0, 0.0, 0));
        case.1 += prix * quantite;
        case.2 += 1;

        match cote.as_str() {
            "buy" => {
                lots.entry(paire.clone())
                    .or_insert_with(VecDeque::new)
                    .push_back((*quantite, *prix));
            }
            "sell" => {
                let file = lots.entry(paire.clone()).or_insert_with(VecDeque::new);
                let mut reste = *quantite;
                while reste > 1e-12 && !file.is_empty() {
                    let (mutlot, prix_lot) = file.front().copied().unwrap_or((0.0, 0.0));
                    let appari = mutlot.min(reste);
                    case.0 += (prix - prix_lot) * appari;
                    reste -= appari;
                    file.pop_front();
                    if mutlot - appari > 1e-12 {
                        file.push_front((mutlot - appari, prix_lot));
                    }
                }
                if reste > 1e-12 {
                    lots_non_apparies += 1;
                }
            }
            _ => {}
        }
    }

    Appariement {
        par_devise,
        lots_restants: lots,
        lots_non_apparies,
    }
}

/// GET /api/trading/performance/{uq}?periode=7j|30j|1an
///
/// PnL réalisé par appariement FIFO des exécutions du compte (`trading_trades`
/// porte son `uq`). Les achats forment une file de lots ; une vente consomme
/// cette file et le gain/perte réalisé est `(prix de vente - prix du lot) x
/// quantité appariée`. Une vente sans lot correspondant n'est pas inventée en
/// position courte : elle est comptée dans `lots_non_apparies`.
pub async fn trading_performance(
    req: HttpRequest,
    path: web::Path<String>,
    query: web::Query<HashMap<String, String>>,
) -> HttpResponse {
    let uq = path.into_inner();
    if !crate::auth_guard::ressemble_a_un_uq(&uq) {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "code": "UQ_INVALIDE",
            "message": "Identifiant de compte attendu (format UQ-00000000)"
        }));
    }
    if let Err(response) = crate::auth_guard::exiger_soi_ou_owner(&req, &uq) {
        return response;
    }

    let periode_brut = query
        .get("periode")
        .map(|v| v.trim().to_string())
        .unwrap_or_else(|| "30j".to_string());
    let Some((periode, secondes)) = SERIES_PERIODES
        .iter()
        .find(|(nom, _)| *nom == periode_brut)
        .copied()
    else {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "code": "PERIODE_INVALIDE",
            "message": "Période acceptée : 7j, 30j ou 1an",
            "allowed": SERIES_PERIODES.iter().map(|(nom, _)| *nom).collect::<Vec<_>>()
        }));
    };

    let conn = match ouvrir_index() {
        Ok(conn) => conn,
        Err(e) => return echec(e),
    };

    // Tout l'historique d'exécutions, sinon un ordre du début de fenêtre
    // serait apparié avec des lots qui n'existent pas encore dans la fenêtre.
    let mut executions: Vec<Execution> = Vec::new();
    {
        let mut stmt = match conn.prepare(
            "SELECT pair, side, price, amount, created_at FROM trading_trades
             WHERE UPPER(uq) = UPPER(?1) ORDER BY created_at ASC, id ASC",
        ) {
            Ok(stmt) => stmt,
            Err(e) => {
                return HttpResponse::InternalServerError().json(json!({
                    "status": "error",
                    "code": "LECTURE_IMPOSSIBLE",
                    "message": format!("Exécutions illisibles : {}", e)
                }))
            }
        };
        let lignes = stmt.query_map(params![uq], |row| {
            Ok(Execution {
                paire: row.get(0)?,
                cote: row.get(1)?,
                prix: row.get(2)?,
                quantite: row.get(3)?,
                cree: row.get(4)?,
            })
        });
        match lignes {
            Ok(rows) => executions.extend(rows.flatten()),
            Err(e) => {
                return HttpResponse::InternalServerError().json(json!({
                    "status": "error",
                    "code": "LECTURE_IMPOSSIBLE",
                    "message": format!("Lignes d'exécution illisibles : {}", e)
                }))
            }
        }
    }

    let Appariement {
        par_devise,
        lots_restants: lots,
        lots_non_apparies,
    } = apparier_fifo(&executions);

    let positions_ouvertes: Vec<serde_json::Value> = {
        let mut entrees: Vec<(&String, &VecDeque<(f64, f64)>)> =
            lots.iter().filter(|(_, file)| !file.is_empty()).collect();
        entrees.sort_by(|a, b| a.0.cmp(b.0));
        entrees
            .iter()
            .map(|(paire, file)| {
                let quantite: f64 = file.iter().map(|(q, _)| q).sum();
                let valeur_achete: f64 = file.iter().map(|(q, p)| q * p).sum();
                json!({
                    "pair": paire,
                    "devise": devise_de_la_paire(paire),
                    "quantite": quantite,
                    "prix_moyen": if quantite > 0.0 { valeur_achete / quantite } else { 0.0 }
                })
            })
            .collect()
    };

    let horodatage = chrono::Utc::now().timestamp();
    let depuis = horodatage - secondes;

    // Une série par devise : le cumul ne prend du sens que dans une seule unité.
    let mut devises: Vec<String> = par_devise.keys().cloned().collect();
    devises.sort();

    let mut series_par_devise = serde_json::Map::new();
    let mut totaux: Vec<serde_json::Value> = Vec::new();

    for devise in &devises {
        let Some(jours_devise) = par_devise.get(devise) else {
            continue;
        };
        let mut jours: Vec<&String> = jours_devise.keys().collect();
        jours.sort();

        let mut cumul = 0f64;
        let mut pnl_total = 0f64;
        let mut volume_total = 0f64;
        let mut trades_total = 0i64;
        let mut serie: Vec<serde_json::Value> = Vec::new();

        for jour in jours {
            let (realise, volume, nb) = jours_devise[jour];
            cumul += realise;
            pnl_total += realise;
            volume_total += volume;
            trades_total += nb;
            let date_secondes =
                chrono::DateTime::parse_from_rfc3339(&format!("{}T00:00:00Z", jour))
                    .map(|instant| instant.timestamp())
                    .unwrap_or(0);
            if date_secondes < depuis {
                continue;
            }
            serie.push(json!({
                "date": jour,
                "pnl_realise": realise,
                "pnl_cumule": cumul,
                "volume": volume,
                "trades": nb
            }));
        }

        series_par_devise.insert(devise.clone(), json!(serie));
        totaux.push(json!({
            "devise": devise,
            "pnl_realise": pnl_total,
            "volume": volume_total,
            "trades": trades_total
        }));
    }

    HttpResponse::Ok().json(json!({
        "status": "success",
        "uq": uq,
        "periode": periode,
        "secondes": secondes,
        "executions_comptees": executions.len(),
        "lots_non_apparies": lots_non_apparies,
        "positions_ouvertes": positions_ouvertes,
        "devises": totaux,
        "series": series_par_devise
    }))
}

/// La base d'index du module trading, ouverte en lecture/écriture comme ailleurs.
fn ouvrir_index() -> Result<Connection, String> {
    let conn =
        Connection::open("uq_index.db").map_err(|e| format!("Index indisponible ({})", e))?;
    let _ = conn.busy_timeout(std::time::Duration::from_secs(5));
    crate::trading_routes::ensure_tables(&conn).map_err(|e| e.to_string())?;
    Ok(conn)
}

fn echec(erreur: String) -> HttpResponse {
    HttpResponse::InternalServerError().json(json!({
        "status": "error",
        "code": "INDEX_IMPOSSIBLE",
        "message": erreur
    }))
}

/// Une ligne du grand livre local d'un compte.
#[derive(Debug, Clone)]
pub struct LigneGrandLivre {
    pub cree: i64,
    pub montant: i64,
    pub frais: i64,
    pub tx_type: String,
    pub statut: String,
}

/// Effets classés du grand livre, jour par jour.
#[derive(Debug)]
pub struct Flux {
    /// `(instant, delta en points)` des seules lignes retenues.
    pub deltas: Vec<(i64, i64)>,
    pub types_non_classes: HashMap<String, i64>,
    pub lignes_rejetees: i64,
}

/// Convertit le grand livre en flux signés.
///
/// Une ligne non réussie n'a pas de valeur, un type inconnu n'est pas deviné :
/// les deux sont comptés séparément pour que la réconciliation reste honnête.
pub fn classer_grand_livre(lignes: &[LigneGrandLivre]) -> Flux {
    let mut deltas = Vec::new();
    let mut types_non_classes: HashMap<String, i64> = HashMap::new();
    let mut lignes_rejetees = 0i64;

    for ligne in lignes {
        if !ligne.statut.is_empty() && ligne.statut != "success" {
            lignes_rejetees += 1;
            continue;
        }
        match effet_solde(&ligne.tx_type) {
            EffetSolde::Entree => deltas.push((ligne.cree, ligne.montant)),
            EffetSolde::Sortie => deltas.push((ligne.cree, -(ligne.montant + ligne.frais))),
            EffetSolde::Inconnu => {
                *types_non_classes.entry(ligne.tx_type.clone()).or_insert(0) += 1;
            }
        }
    }

    Flux {
        deltas,
        types_non_classes,
        lignes_rejetees,
    }
}

/// GET /api/borb/balance/series/{id}?periode=7j|30j|1an
///
/// Courbe privée du coffre, agrégée sur tout l'historique du compte.
pub async fn borb_balance_series(
    req: HttpRequest,
    path: web::Path<String>,
    query: web::Query<HashMap<String, String>>,
) -> HttpResponse {
    let compte = path.into_inner();
    if !crate::auth_guard::ressemble_a_un_uq(&compte) {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "code": "UQ_INVALIDE",
            "message": "Identifiant de compte attendu (format UQ-00000000)"
        }));
    }
    if let Err(response) = crate::auth_guard::exiger_soi_ou_owner(&req, &compte) {
        return response;
    }

    let periode_brut = query
        .get("periode")
        .map(|v| v.trim().to_string())
        .unwrap_or_else(|| "30j".to_string());
    let Some((periode, secondes)) = SERIES_PERIODES
        .iter()
        .find(|(nom, _)| *nom == periode_brut)
        .copied()
    else {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "code": "PERIODE_INVALIDE",
            "message": "Période acceptée : 7j, 30j ou 1an",
            "allowed": SERIES_PERIODES.iter().map(|(nom, _)| *nom).collect::<Vec<_>>()
        }));
    };

    let db_path = format!("data/users/{}.db", compte.replace("-", "_").to_lowercase());
    if !Path::new(&db_path).exists() {
        return HttpResponse::NotFound().json(json!({
            "status": "error",
            "code": "COMPTE_INTROUVABLE",
            "message": format!("Compte introuvable: {}", compte)
        }));
    }
    let conn = match Connection::open(&db_path) {
        Ok(conn) => conn,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "code": "DB_IMPOSSIBLE",
                "message": format!("Base de compte illisible : {}", e)
            }))
        }
    };
    if let Err(e) = crate::db::ensure_transactions_columns(&conn) {
        eprintln!(
            "⚠️  Alignement schéma transactions (série {}): {}",
            compte, e
        );
    }

    // Solde interne actuel : la référence à laquelle la courbe doit se raccorder.
    let points_actuels: i64 = conn
        .query_row(
            "SELECT COALESCE(points, 0) FROM balance WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    let mut lignes: Vec<LigneGrandLivre> = Vec::new();
    {
        let mut stmt = match conn.prepare(
            "SELECT COALESCE(created_at, 0), COALESCE(amount, 0), COALESCE(fee, 0),
                    COALESCE(tx_type, ''), COALESCE(status, '')
             FROM transactions ORDER BY created_at ASC",
        ) {
            Ok(stmt) => stmt,
            Err(e) => {
                return HttpResponse::InternalServerError().json(json!({
                    "status": "error",
                    "code": "LECTURE_IMPOSSIBLE",
                    "message": format!("Historique illisible : {}", e)
                }))
            }
        };
        let lecture = stmt.query_map([], |row| {
            Ok(LigneGrandLivre {
                cree: row.get(0)?,
                montant: row.get(1)?,
                frais: row.get(2)?,
                tx_type: row.get(3)?,
                statut: row.get(4)?,
            })
        });
        match lecture {
            Ok(rows) => lignes.extend(rows.flatten()),
            Err(e) => {
                return HttpResponse::InternalServerError().json(json!({
                    "status": "error",
                    "code": "LECTURE_IMPOSSIBLE",
                    "message": format!("Lignes d'historique illisibles : {}", e)
                }))
            }
        }
    }

    let Flux {
        deltas,
        types_non_classes,
        lignes_rejetees,
    } = classer_grand_livre(&lignes);

    // Réconciliation : le grand livre part de zéro. S'il n'explique pas le
    // solde affiché, la courbe ne peut pas prétendre être un solde — elle
    // reste un cumul de flux et l'écart est exposé tel quel.
    let flux_total: i64 = deltas.iter().map(|(_, delta)| delta).sum();
    let ecart = points_actuels - flux_total;
    let reconcilie = ecart == 0;

    let mut par_jour: HashMap<String, (i64, i64, i64, i64)> = HashMap::new();
    for (cree, delta) in &deltas {
        let jour = date_jour(*cree);
        let case = par_jour.entry(jour).or_insert((0, 0, 0, 0));
        if *delta >= 0 {
            case.1 += *delta;
        } else {
            case.2 += -*delta;
        }
        case.0 += *delta;
        case.3 += 1;
    }

    let mut jours: Vec<String> = par_jour.keys().cloned().collect();
    jours.sort();

    // Le solde d'un jour = somme de TOUTES les lignes jusqu'à la fin de ce jour.
    // La fenêtre demandée ne sert qu'à filtrer les points rendus.
    let horodatage = chrono::Utc::now().timestamp();
    let depuis = horodatage - secondes;
    let mut prefix: i64 = 0;
    let mut serie: Vec<serde_json::Value> = Vec::new();
    for jour in &jours {
        let (net, entrees, sorties, nb) = par_jour[jour];
        prefix += net;
        let date_secondes = chrono::DateTime::parse_from_rfc3339(&format!("{}T00:00:00Z", jour))
            .map(|instant| instant.timestamp())
            .unwrap_or(0);
        if date_secondes < depuis {
            continue;
        }
        serie.push(json!({
            "date": jour,
            "flux_net": net,
            "entrees": entrees,
            "sorties": sorties,
            "transactions": nb,
            "cumul_flux": prefix,
            "solde_points": if reconcilie { serde_json::Value::from(prefix) } else { serde_json::Value::Null }
        }));
    }

    HttpResponse::Ok().json(json!({
        "status": "success",
        "account": compte,
        "periode": periode,
        "unite": "points",
        "solde_actuel_points": points_actuels,
        "reconcilie": reconcilie,
        "ecart_reconciliation_points": ecart,
        "lignes_historique": deltas.len(),
        "lignes_rejetees": lignes_rejetees,
        "types_non_classes": types_non_classes,
        "series": serie
    }))
}

#[derive(Clone, Copy)]
struct SourceEcriture {
    code: &'static str,
    libelle: &'static str,
    sens: &'static str,
}

fn source_ecriture(tx_type: &str) -> Option<SourceEcriture> {
    match tx_type {
        "ubi" => Some(SourceEcriture {
            code: "ubi",
            libelle: "Récompenses UBI",
            sens: "entree",
        }),
        "receive" | "transfer_received" => Some(SourceEcriture {
            code: "transferts_recus",
            libelle: "Transferts reçus",
            sens: "entree",
        }),
        "p2p_buy" => Some(SourceEcriture {
            code: "p2p_recus",
            libelle: "Réceptions P2P",
            sens: "entree",
        }),
        "send" | "transfer_sent" => Some(SourceEcriture {
            code: "transferts_envoyes",
            libelle: "Transferts envoyés",
            sens: "sortie",
        }),
        "p2p_sell" => Some(SourceEcriture {
            code: "p2p_ventes",
            libelle: "Ventes P2P",
            sens: "sortie",
        }),
        _ => None,
    }
}

/// GET /api/analytics/wallet/{id}?periode=7j|30j|1an
///
/// Analyse privée des écritures réellement enregistrées dans le coffre.
/// Les entrées de transferts et P2P ne sont pas assimilées à des revenus ;
/// seules les écritures UBI sont identifiées ici comme une récompense générée.
pub async fn wallet_analytics(
    req: HttpRequest,
    path: web::Path<String>,
    query: web::Query<HashMap<String, String>>,
) -> HttpResponse {
    let compte = path.into_inner();
    if !crate::auth_guard::ressemble_a_un_uq(&compte) {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "code": "UQ_INVALIDE",
            "message": "Identifiant de compte attendu (format UQ-00000000)"
        }));
    }
    if let Err(response) = crate::auth_guard::exiger_soi_ou_owner(&req, &compte) {
        return response;
    }

    let periode_brut = query
        .get("periode")
        .map(|v| v.trim().to_string())
        .unwrap_or_else(|| "30j".to_string());
    let Some((periode, secondes)) = SERIES_PERIODES
        .iter()
        .find(|(nom, _)| *nom == periode_brut)
        .copied()
    else {
        return HttpResponse::BadRequest().json(json!({
            "status": "error",
            "code": "PERIODE_INVALIDE",
            "message": "Période acceptée : 7j, 30j ou 1an",
            "allowed": SERIES_PERIODES.iter().map(|(nom, _)| *nom).collect::<Vec<_>>()
        }));
    };

    let db_path = format!("data/users/{}.db", compte.replace("-", "_").to_lowercase());
    if !Path::new(&db_path).exists() {
        return HttpResponse::NotFound().json(json!({
            "status": "error",
            "code": "COMPTE_INTROUVABLE",
            "message": format!("Compte introuvable: {}", compte)
        }));
    }
    let conn = match Connection::open(&db_path) {
        Ok(conn) => conn,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "code": "DB_IMPOSSIBLE",
                "message": format!("Base de compte illisible : {}", e)
            }))
        }
    };
    if let Err(e) = conn.busy_timeout(std::time::Duration::from_secs(5)) {
        return HttpResponse::InternalServerError().json(json!({
            "status": "error",
            "code": "DB_IMPOSSIBLE",
            "message": format!("Verrouillage de la base impossible : {}", e)
        }));
    }
    let solde_actuel_points = match conn.query_row(
        "SELECT COALESCE(points, 0) FROM balance WHERE id = 1",
        [],
        |row| row.get::<_, i64>(0),
    ) {
        Ok(solde) => solde,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "code": "LECTURE_IMPOSSIBLE",
                "message": format!("Solde du coffre illisible : {}", e)
            }))
        }
    };
    let mut stmt = match conn.prepare(
        "SELECT COALESCE(created_at, 0), COALESCE(amount, 0), COALESCE(fee, 0),
                COALESCE(tx_type, ''), COALESCE(status, '')
         FROM transactions ORDER BY created_at ASC",
    ) {
        Ok(stmt) => stmt,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "code": "LECTURE_IMPOSSIBLE",
                "message": format!("Historique illisible : {}", e)
            }))
        }
    };
    let mut rows = match stmt.query([]) {
        Ok(rows) => rows,
        Err(e) => {
            return HttpResponse::InternalServerError().json(json!({
                "status": "error",
                "code": "LECTURE_IMPOSSIBLE",
                "message": format!("Lecture de l'historique impossible : {}", e)
            }))
        }
    };

    let depuis = chrono::Utc::now().timestamp() - secondes;
    let mut entrees = 0i64;
    let mut sorties = 0i64;
    let mut transactions = 0i64;
    let mut flux_historique = 0i64;
    let mut lignes_non_classifiees_historique = 0i64;
    let mut lignes_rejetees = 0i64;
    let mut types_non_classes: HashMap<String, i64> = HashMap::new();
    let mut totaux_sources: HashMap<String, (String, String, i64, i64)> = HashMap::new();
    let mut par_jour: HashMap<String, (i64, i64, i64, HashMap<String, i64>)> = HashMap::new();

    loop {
        let row = match rows.next() {
            Ok(Some(row)) => row,
            Ok(None) => break,
            Err(e) => {
                return HttpResponse::InternalServerError().json(json!({
                    "status": "error",
                    "code": "LECTURE_IMPOSSIBLE",
                    "message": format!("Ligne d'historique illisible : {}", e)
                }))
            }
        };
        let valeurs: rusqlite::Result<(i64, i64, i64, String, String)> = (|| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })();
        let (cree, montant, frais, tx_type, statut) = match valeurs {
            Ok(valeurs) => valeurs,
            Err(e) => {
                return HttpResponse::InternalServerError().json(json!({
                    "status": "error",
                    "code": "LECTURE_IMPOSSIBLE",
                    "message": format!("Écriture du coffre illisible : {}", e)
                }))
            }
        };
        let ligne_validee = statut.is_empty() || statut == "success";
        if ligne_validee {
            if let Some(source_historique) = source_ecriture(&tx_type) {
                let montant_historique = if source_historique.sens == "sortie" {
                    montant + frais
                } else {
                    montant
                };
                if source_historique.sens == "entree" {
                    flux_historique += montant_historique;
                } else {
                    flux_historique -= montant_historique;
                }
            } else {
                lignes_non_classifiees_historique += 1;
            }
        }
        if cree < depuis {
            continue;
        }
        if !ligne_validee {
            lignes_rejetees += 1;
            continue;
        }
        let Some(source) = source_ecriture(&tx_type) else {
            *types_non_classes.entry(tx_type).or_insert(0) += 1;
            continue;
        };
        let montant_source = if source.sens == "sortie" {
            montant + frais
        } else {
            montant
        };
        if source.sens == "entree" {
            entrees += montant_source;
        } else {
            sorties += montant_source;
        }
        transactions += 1;

        let total_source = totaux_sources.entry(source.code.to_string()).or_insert((
            source.libelle.to_string(),
            source.sens.to_string(),
            0,
            0,
        ));
        total_source.2 += montant_source;
        total_source.3 += 1;

        let valeur_jour = par_jour
            .entry(date_jour(cree))
            .or_insert((0, 0, 0, HashMap::new()));
        if source.sens == "entree" {
            valeur_jour.0 += montant_source;
        } else {
            valeur_jour.1 += montant_source;
        }
        valeur_jour.2 += 1;
        *valeur_jour.3.entry(source.code.to_string()).or_insert(0) += montant_source;
    }

    let mut sources: Vec<serde_json::Value> = totaux_sources
        .into_iter()
        .map(|(code, (libelle, sens, montant, nombre))| {
            json!({
                "code": code,
                "label": libelle,
                "direction": sens,
                "amount": montant,
                "transactions": nombre
            })
        })
        .collect();
    sources.sort_by(|a, b| {
        a["code"]
            .as_str()
            .unwrap_or_default()
            .cmp(b["code"].as_str().unwrap_or_default())
    });

    let mut jours: Vec<String> = par_jour.keys().cloned().collect();
    jours.sort();
    let serie: Vec<serde_json::Value> = jours
        .into_iter()
        .map(|date| {
            let (jour_entrees, jour_sorties, nombre, sources_jour) =
                par_jour.remove(&date).unwrap_or_default();
            json!({
                "date": date,
                "entrees": jour_entrees,
                "sorties": jour_sorties,
                "net": jour_entrees - jour_sorties,
                "transactions": nombre,
                "by_source": sources_jour
            })
        })
        .collect();

    let ecart_reconciliation_points = solde_actuel_points - flux_historique;
    let reconcilie = ecart_reconciliation_points == 0 && lignes_non_classifiees_historique == 0;
    HttpResponse::Ok().json(json!({
        "status": "success",
        "account": compte,
        "periode": periode,
        "unit": "points",
        "solde_actuel_points": solde_actuel_points,
        "reconcilie": reconcilie,
        "ecart_reconciliation_points": ecart_reconciliation_points,
        "lignes_non_classifiees_historique": lignes_non_classifiees_historique,
        "totals": {
            "entrees": entrees,
            "sorties": sorties,
            "net": entrees - sorties,
            "transactions": transactions
        },
        "sources": sources,
        "series": serie,
        "lignes_rejetees": lignes_rejetees,
        "types_non_classes": types_non_classes
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn execution(paire: &str, cote: &str, prix: f64, quantite: f64, cree: i64) -> Execution {
        Execution {
            paire: paire.to_string(),
            cote: cote.to_string(),
            prix,
            quantite,
            cree,
        }
    }

    fn ligne(tx_type: &str, montant: i64, frais: i64, statut: &str, cree: i64) -> LigneGrandLivre {
        LigneGrandLivre {
            cree,
            montant,
            frais,
            tx_type: tx_type.to_string(),
            statut: statut.to_string(),
        }
    }

    /// Un instant de jour fixe pour que les tests ne dépendent pas de l'heure.
    const JOUR: i64 = 1_767_225_600; // 2026-01-01T00:00:00Z

    #[test]
    fn fifo_apparie_un_vente_sur_plusieurs_lots() {
        let appariement = apparier_fifo(&[
            execution("BTC-USD", "buy", 100.0, 2.0, JOUR),
            execution("BTC-USD", "buy", 120.0, 1.0, JOUR),
            execution("BTC-USD", "sell", 150.0, 2.5, JOUR),
        ]);

        let jours = &appariement.par_devise["USD"];
        let case = &jours[&date_jour(JOUR)];
        assert_eq!(case.2, 3, "les trois exécutions sont comptées");
        assert!(
            (case.1 - (200.0 + 120.0 + 375.0)).abs() < 1e-9,
            "volume = somme des prix x quantités, vente comprise"
        );
        // (150-100)*2 + (150-120)*0.5 = 115
        assert!(
            (case.0 - 115.0).abs() < 1e-9,
            "PnL FIFO : {} attendu, obtenu {}",
            115.0,
            case.0
        );

        let restant = &appariement.lots_restants["BTC-USD"];
        assert_eq!(restant.len(), 1, "un seul lot subsiste");
        assert!((restant[0].0 - 0.5).abs() < 1e-9 && (restant[0].1 - 120.0).abs() < 1e-9);
        assert_eq!(appariement.lots_non_apparies, 0);
    }

    #[test]
    fn vente_sans_lot_ne_cree_jamais_de_position_courte() {
        let appariement = apparier_fifo(&[
            execution("ETH-USD", "buy", 100.0, 1.0, JOUR),
            execution("ETH-USD", "sell", 90.0, 3.0, JOUR),
        ]);

        let case = &appariement.par_devise["USD"][&date_jour(JOUR)];
        // Seule la partie appairée (1) donne un PnL : (90-100)*1 = -10.
        assert!(
            (case.0 + 10.0).abs() < 1e-9,
            "le PnL ne couvre que le lot réellement appairé"
        );
        assert_eq!(appariement.lots_non_apparies, 1);
        assert!(appariement.lots_restants["ETH-USD"].is_empty());
    }

    #[test]
    fn devises_ne_sont_jamais_sommees_ensemble() {
        let appariement = apparier_fifo(&[
            execution("BTC-USD", "buy", 100.0, 1.0, JOUR),
            execution("BTC-USD", "sell", 150.0, 1.0, JOUR),
            execution("BTC-CDF", "buy", 100.0, 1.0, JOUR),
            execution("BTC-CDF", "sell", 150.0, 1.0, JOUR),
        ]);

        assert_eq!(appariement.par_devise.len(), 2, "USD et CDF séparément");
        for devise in ["USD", "CDF"] {
            let case = &appariement.par_devise[devise][&date_jour(JOUR)];
            assert!(
                (case.0 - 50.0).abs() < 1e-9,
                "chaque devise porte son propre PnL, obtenu {}",
                case.0
            );
            assert_eq!(case.2, 2);
        }
    }

    #[test]
    fn un_lot_achete_apres_la_vente_ne_apparie_rien() {
        let appariement = apparier_fifo(&[
            execution("BTC-USD", "sell", 150.0, 1.0, JOUR),
            execution("BTC-USD", "buy", 100.0, 1.0, JOUR + 3600),
        ]);
        assert_eq!(appariement.lots_non_apparies, 1);
        assert_eq!(appariement.par_devise["USD"][&date_jour(JOUR)].0, 0.0);
    }

    #[test]
    fn grand_livre_classe_signe_et_rejette() {
        let flux = classer_grand_livre(&[
            ligne("ubi", 500, 0, "success", JOUR),
            ligne("receive", 300, 0, "success", JOUR),
            ligne("send", 100, 7, "success", JOUR + 60),
            ligne("send", 999, 1, "pending", JOUR + 120),
            ligne("test", 42, 0, "success", JOUR + 180),
        ]);

        assert_eq!(
            flux.deltas.len(),
            3,
            "les lignes non réussies sont écartées"
        );
        assert_eq!(flux.lignes_rejetees, 1);
        assert_eq!(flux.types_non_classes.get("test"), Some(&1));

        // 500 + 300 - (100 + 7)
        let total: i64 = flux.deltas.iter().map(|(_, delta)| delta).sum();
        assert_eq!(total, 693);
        assert_eq!(flux.deltas[2].1, -107, "une sortie débite montant ET frais");
    }

    #[test]
    fn reconciliation_accepte_un_ecart_non_nul() {
        let flux = classer_grand_livre(&[ligne("ubi", 500, 0, "success", JOUR)]);
        let total: i64 = flux.deltas.iter().map(|(_, delta)| delta).sum();
        assert_eq!(600 - total, 100, "l'écart est exposé, pas comblé");
        assert_eq!(total, 500);
    }

    #[test]
    fn les_sources_separent_recompenses_transferts_et_p2p() {
        assert_eq!(source_ecriture("ubi").unwrap().code, "ubi");
        assert_eq!(source_ecriture("receive").unwrap().sens, "entree");
        assert_eq!(source_ecriture("p2p_buy").unwrap().code, "p2p_recus");
        assert_eq!(source_ecriture("send").unwrap().sens, "sortie");
        assert_eq!(source_ecriture("p2p_sell").unwrap().code, "p2p_ventes");
        assert!(source_ecriture("type_inconnu").is_none());
    }
}
