// ====================
// 📦 PERSISTENCE — Charger/Sauvegarder BlockOrb
// ====================
//
// « L'eau circule. Elle se persiste. »
//
// Ce module gère la persistance :
// - Charger le BlockOrb global (40 Md)
// - Sauvegarder le BlockOrb global
// - Charger/Sauvegarder le BlockOrb d'un utilisateur
// - Charger/Sauvegarder le prix (Regulateur)
// - Reconstruire le BlockOrb depuis les bases SQLite (P2P)
//
// ====================

use crate::block_orb::BlockOrb;
use crate::regulateur::RegulateurState;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

const DATA_DIR: &str = "data";
const USERS_DIR: &str = "data/users";
const PRIX_FILE: &str = "data/prix.json";
const BORB_REGISTRY: &str = "data/borb_registry.json";

/// 🚫 Verrou global du BlockOrb : serialize les sections critiques
/// lecture-écriture-sauvegarde entre les workers actix.
/// Prendre ce verrou AVANT d'appeler load_global_orb côté mutation ;
/// mirroir_transfert et reconcilier_blockorb le prennent eux-mêmes.
static ORB_LOCK: Mutex<()> = Mutex::new(());

/// Rattache base<->BlockOrb : une seule fois par processus (scan des DB).
static RATTACHAGE: std::sync::Once = std::sync::Once::new();

/// Verrouille le BlockOrb (récupère un mutexempoisonné sans paniquer).
pub fn verrou_orb() -> MutexGuard<'static, ()> {
    ORB_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// (BlockOrb, garde) : la section critique commence au load et finit au drop
/// du garde — appeler save_global_orb(&orb) pendant que le garde est tenu.
pub fn orb_charge_verrouille() -> (BlockOrb, MutexGuard<'static, ()>) {
    let guard = verrou_orb();
    (load_global_orb(), guard)
}

/// Racine `data` prioritaire : le cwd (lancement historique depuis
/// uq_platform_bin), avec repli sur le dossier de l'executable si le
/// fichier n'y est pas — sinon deux copies divergeaient (split-brain).
fn data_root_candidates() -> Vec<PathBuf> {
    let mut v = vec![PathBuf::from(".")];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let p = dir.to_path_buf();
            if !v.contains(&p) {
                v.push(p);
            }
        }
    }
    v
}

/// Chemin absolu basé sur le répertoire de l'exécutable.
/// Garantit que le BlockOrb est toujours trouvé : le cwd d'abord (données
/// historiques de prod), repli sur le dossier de l'executable.
fn global_orb_path() -> std::path::PathBuf {
    let rel = Path::new("data/block_orb_global.json");
    for cand in data_root_candidates() {
        let p = cand.join(rel);
        if p.exists() {
            return p;
        }
    }
    PathBuf::from(".").join(rel)
}

/// Dossier users au même régime que global_orb_path (mêmes données que le
/// BlockOrb, sinon miroir et SQLite parlent de deux univers différents).
fn users_dir_path() -> std::path::PathBuf {
    let rel = Path::new("data/users");
    for cand in data_root_candidates() {
        let p = cand.join(rel);
        if p.exists() {
            return p;
        }
    }
    PathBuf::from(".").join(rel)
}

// ====================
// INITIALISATION DES DOSSIERS
// ====================

pub fn init_dirs() -> Result<(), String> {
    fs::create_dir_all(DATA_DIR).map_err(|e| e.to_string())?;
    fs::create_dir_all(USERS_DIR).map_err(|e| e.to_string())?;

    // Créer aussi le dossier data à côté de l'exécutable
    let exe_data = global_orb_path().parent().map(|p| p.to_path_buf());
    if let Some(p) = exe_data {
        fs::create_dir_all(&p).map_err(|e| e.to_string())?;
        fs::create_dir_all(p.join("users")).map_err(|e| e.to_string())?;
    }

    Ok(())
}

// ====================
// BLOCKORB GLOBAL
// ====================

/// Rattache au BlockOrb les bases `data/users/*.db` qui n'y ont pas de wallet.
///
/// Un compte n'est rattaché que si sa base porte un cadenas structurel crédible :
/// sans lien avec des identifiants personnels, il reste fantôme et la porte BORB
/// lui refuse toute entrée. Le solde est prélevé du Trésor, jamais créé.
fn rattache_comptes_manquants(orb: &mut BlockOrb) -> usize {
    let users_dir = users_dir_path();
    let entries = match fs::read_dir(&users_dir) {
        Ok(e) => e,
        Err(_) => return 0,
    };
    let femto_par_point = crate::montant::FEMTO_PAR_BORB / 100;
    let mut rattaches = 0;

    for entry in entries.flatten() {
        let nom = entry.file_name().to_string_lossy().to_string();
        if !nom.starts_with("uq_") || !nom.ends_with(".db") {
            continue;
        }
        let brut = &nom[3..nom.len() - 3];
        if brut.is_empty() || !brut.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let uq = format!("UQ-{}", brut);
        if orb.comptes.contains_key(&uq) {
            continue;
        }

        let cadenas = crate::kyc::cadenas::cadenas_porte_par(&uq);
        if !cadenas
            .as_deref()
            .map(crate::block_orb::cadenas_credible)
            .unwrap_or(false)
        {
            eprintln!(
                "  👻 {} : aucun cadenas structurel en base — wallet non cree",
                uq
            );
            continue;
        }

        let points = rusqlite::Connection::open(users_dir.join(&nom))
            .and_then(|conn| {
                conn.query_row("SELECT points FROM balance WHERE id = 1", [], |r| {
                    r.get::<_, i64>(0)
                })
            })
            .unwrap_or(0)
            .max(0);

        match orb.creer_compte(
            &uq,
            (points as u128) * femto_par_point,
            cadenas.as_deref().unwrap(),
        ) {
            Ok(()) => {
                rattaches += 1;
                eprintln!(
                    "  🔗 {} rattaché au BlockOrb ({} pts, cadenas posé)",
                    uq, points
                );
            }
            Err(e) => eprintln!("  ⚠️  {} : rattache refuse — {}", uq, e),
        }
    }

    rattaches
}

/// Charge le BlockOrb global.
///
/// Si le fichier n'existe pas, ou s'il est vide (0 comptes), on reconstruit
/// le BlockOrb depuis les bases SQLite présentes dans `data/users/*.db`.
///
/// C'est la méthode P2P : chaque nœud peut reconstruire l'état depuis ses
/// bases locales, sans dépendre d'un serveur central.
pub fn load_global_orb() -> BlockOrb {
    // ===== 1. Essayer de charger le fichier existant =====
    if global_orb_path().exists() {
        match fs::read_to_string(global_orb_path()) {
            Ok(content) => {
                if content.trim().is_empty() {
                    eprintln!("⚠️  BlockOrb vide (fichier vide), reconstruction...");
                } else {
                    match serde_json::from_str::<BlockOrb>(&content) {
                        Ok(mut orb) => {
                            // Si le BlockOrb a des comptes, on le retourne
                            if !orb.comptes.is_empty() {
                                eprintln!(
                                    "✅ BlockOrb global chargé : {} comptes, {} cases",
                                    orb.comptes.len(),
                                    orb.cases.len()
                                );
                                RATTACHAGE.call_once(|| {
                                    let n = rattache_comptes_manquants(&mut orb);
                                    if n > 0 {
                                        if let Err(e) = save_global_orb(&orb) {
                                            eprintln!("⚠️  Rattache enregistree : {}", e);
                                        }
                                    }
                                });
                                return orb;
                            }
                            // Sinon, on va reconstruire
                            eprintln!(
                                "⚠️  BlockOrb chargé mais sans comptes ({} cases), reconstruction...",
                                orb.cases.len()
                            );
                        }
                        Err(e) => {
                            eprintln!("⚠️  Erreur parsing BlockOrb : {}", e);
                            eprintln!("   → Reconstruction depuis les bases SQLite...");
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("⚠️  Erreur lecture BlockOrb : {}", e);
            }
        }
    }

    // ===== 2. Créer un nouveau BlockOrb =====
    eprintln!("📦 Création BlockOrb global (40 Md BORB)");
    let mut orb = BlockOrb::new();

    // ===== 3. Scanner les bases SQLite et charger les soldes =====
    let users_dir = users_dir_path();
    let femto_par_point = crate::montant::FEMTO_PAR_BORB / 100;

    let mut comptes_charges = 0;
    let mut total_femto: u128 = 0;

    if users_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&users_dir) {
            for entry in entries.flatten() {
                let nom = entry.file_name().to_string_lossy().to_string();

                // Filtrer : uq_<chiffres>.db uniquement
                if !nom.starts_with("uq_") || !nom.ends_with(".db") {
                    continue;
                }
                let brut = &nom[3..nom.len() - 3];
                if brut.is_empty() || !brut.chars().all(|c| c.is_ascii_digit()) {
                    continue;
                }

                let uq = format!("UQ-{}", brut);
                let db_path = users_dir.join(&nom);

                // Lire balance.points
                match rusqlite::Connection::open(&db_path) {
                    Ok(conn) => {
                        match conn.query_row("SELECT points FROM balance WHERE id = 1", [], |r| {
                            r.get::<_, i64>(0)
                        }) {
                            Ok(points) => {
                                let solde = (points.max(0) as u128) * femto_par_point;

                                // Créer le portefeuille avec tous les champs.
                                // Le cadenas structurel est repris de la DB du
                                // compte : une reconstruction ne réinvente jamais
                                // un wallet sans cadenas.
                                let cadenas = crate::kyc::cadenas::cadenas_porte_par(&uq);
                                if !cadenas
                                    .as_deref()
                                    .map(crate::block_orb::cadenas_credible)
                                    .unwrap_or(false)
                                {
                                    eprintln!(
                                        "  👻 {} : aucun cadenas structurel en base — non reconstruit",
                                        uq
                                    );
                                    continue;
                                }

                                let mut p = crate::block_orb::Portefeuille::new();
                                p.solde = solde;
                                p.cadenas = cadenas;

                                orb.comptes.insert(uq.clone(), p);
                                comptes_charges += 1;
                                total_femto += solde;

                                if points > 0 {
                                    eprintln!("  📥 {} : {} points → {} femto", uq, points, solde);
                                }
                            }
                            Err(e) => {
                                eprintln!("  ⚠️  {} : erreur lecture balance : {}", uq, e);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("  ⚠️  {} : erreur ouverture DB : {}", uq, e);
                    }
                }
            }
        }
    }

    eprintln!(
        "✅ BlockOrb reconstruit : {} comptes, {} femto total",
        comptes_charges, total_femto
    );

    // ===== 4. Ajuster le Trésor pour respecter l'invariant =====
    // Les comptes chargés depuis les bases représentent de la valeur
    // qui doit être retirée du Trésor (cases[0]) pour que le total reste 40 Md.
    let capacite = crate::montant::CAPACITE_TOTALE;

    if total_femto > 0 {
        if orb.cases[0] >= total_femto {
            orb.cases[0] -= total_femto;
            eprintln!(
                "🔧 Trésor ajusté : -{} femto (comptes chargés)",
                total_femto
            );
        } else {
            eprintln!(
                "⚠️  Trésor insuffisant pour ajuster : {} < {}",
                orb.cases[0], total_femto
            );
        }
    }

    // ===== 5. Vérifier l'invariant =====
    if !orb.verifier() {
        let total = orb.total();
        let diff = if total > capacite {
            total - capacite
        } else {
            capacite - total
        };
        eprintln!("⚠️  Invariant cassé après ajustement");
        eprintln!("   Total : {} / {}", total, capacite);
        eprintln!("   Différence : {}", diff);
    } else {
        eprintln!("✅ Invariant respecté (40 Md exact)");
    }

    // ===== 5. Sauvegarder le BlockOrb reconstruit =====
    if let Err(e) = save_global_orb(&orb) {
        eprintln!("⚠️  Erreur sauvegarde BlockOrb : {}", e);
    } else {
        eprintln!("💾 BlockOrb sauvegardé");
    }

    orb
}

/// Sauvegarde le BlockOrb global.
///
/// 📏 SPÉCIFICATION « TAILLE FIXE » : on refuse d'écrire un BlockOrb qui
/// dépasse MAX_BLOCKORB_BYTES (50 KB). Comme l'historique n'y est plus
/// stocké, la taille ne dépend que du nombre de comptes.
pub fn save_global_orb(orb: &BlockOrb) -> Result<(), String> {
    let json = serde_json::to_string_pretty(orb).map_err(|e| e.to_string())?;

    if json.len() > crate::blockchain_lib::MAX_BLOCKORB_BYTES {
        return Err(format!(
            "BlockOrb trop gros : {} octets > {} octets (taille fixe violée)",
            json.len(),
            crate::blockchain_lib::MAX_BLOCKORB_BYTES
        ));
    }

    // Créer le dossier parent si nécessaire
    if let Some(parent) = global_orb_path().parent() {
        let _ = fs::create_dir_all(parent);
    }

    fs::write(global_orb_path(), json).map_err(|e| e.to_string())?;
    Ok(())
}

// ====================
// PRIX (REGULATEUR)
// ====================

/// Charge l'état du régulateur (ou le crée)
pub fn load_prix() -> RegulateurState {
    if Path::new(PRIX_FILE).exists() {
        if let Ok(content) = fs::read_to_string(PRIX_FILE) {
            if let Ok(state) = serde_json::from_str::<RegulateurState>(&content) {
                return state;
            }
        }
    }

    // Créer nouveau
    let state = RegulateurState {
        prix_actuel: "1.000000000000000000000000000000000000000000000000000000000000".to_string(),
        demande: 0,
        rarete: "0".to_string(),
        vitesse: "0".to_string(),
        dernier_ajustement: chrono::Utc::now().timestamp(),
    };
    let _ = save_prix(&state);
    state
}

/// Sauvegarde l'état du régulateur
pub fn save_prix(state: &RegulateurState) -> Result<(), String> {
    let json = serde_json::to_string_pretty(state).map_err(|e| e.to_string())?;
    fs::write(PRIX_FILE, json).map_err(|e| e.to_string())?;
    Ok(())
}

// ====================
// BLOCKORB UTILISATEUR (personnel)
// ====================

fn user_orb_path(uq_number: &str) -> String {
    let clean = uq_number.replace("-", "_").to_lowercase();
    format!("{}/{}.json", USERS_DIR, clean)
}

/// Charge le BlockOrb personnel d'un utilisateur
pub fn load_user_orb(uq_number: &str) -> Option<BlockOrb> {
    let path = user_orb_path(uq_number);
    if Path::new(&path).exists() {
        if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(orb) = serde_json::from_str::<BlockOrb>(&content) {
                return Some(orb);
            }
        }
    }
    None
}

/// Sauvegarde le BlockOrb personnel d'un utilisateur
pub fn save_user_orb(uq_number: &str, orb: &BlockOrb) -> Result<(), String> {
    let path = user_orb_path(uq_number);
    let json = serde_json::to_string_pretty(orb).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())?;
    Ok(())
}

// ====================
// 🚀 CACHE BLOCKORB
// ====================

use std::sync::{Arc, OnceLock};

static ORB_CACHE: OnceLock<Arc<Mutex<Option<BlockOrb>>>> = OnceLock::new();

fn get_orb_cache() -> &'static Arc<Mutex<Option<BlockOrb>>> {
    ORB_CACHE.get_or_init(|| Arc::new(Mutex::new(None)))
}

/// Taille actuelle du BlockOrb sur disque (octets) — pour supervision
pub fn taille_blockorb() -> usize {
    fs::metadata(global_orb_path())
        .map(|m| m.len() as usize)
        .unwrap_or(0)
}

// ====================
// REGISTRE BORB
// ====================

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BorbEntry {
    pub borb_id: String,
    pub owner_uq: String,
    pub from_uq: Option<String>,
    pub amount: u128,
    pub created_at: i64,
    pub last_seen_at: i64,
    pub status: String,
}

pub fn load_registry() -> HashMap<String, BorbEntry> {
    if Path::new(BORB_REGISTRY).exists() {
        if let Ok(content) = fs::read_to_string(BORB_REGISTRY) {
            if let Ok(registry) = serde_json::from_str::<HashMap<String, BorbEntry>>(&content) {
                return registry;
            }
        }
    }
    HashMap::new()
}

pub fn save_registry(registry: &HashMap<String, BorbEntry>) -> Result<(), String> {
    let json = serde_json::to_string_pretty(registry).map_err(|e| e.to_string())?;
    fs::write(BORB_REGISTRY, json).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn register_borb(borb_id: &str, owner_uq: &str, from_uq: &str, amount: u128, now: i64) {
    let mut registry = load_registry();
    let entry = BorbEntry {
        borb_id: borb_id.to_string(),
        owner_uq: owner_uq.to_string(),
        from_uq: if from_uq.is_empty() {
            None
        } else {
            Some(from_uq.to_string())
        },
        amount,
        created_at: now,
        last_seen_at: now,
        status: "active".to_string(),
    };
    registry.insert(borb_id.to_string(), entry);
    if let Err(e) = save_registry(&registry) {
        eprintln!("Erreur registre: {}", e);
    } else {
        let court: String = borb_id.chars().take(16).collect();
        eprintln!("BORB enregistré: {}", court);
    }
}

pub fn verify_borb(borb_id: &str) -> bool {
    load_registry().contains_key(borb_id)
}

pub fn count_borb_by_owner(owner_uq: &str) -> usize {
    load_registry()
        .values()
        .filter(|e| e.owner_uq == owner_uq)
        .count()
}

pub fn sum_borb_by_owner(owner_uq: &str) -> u128 {
    load_registry()
        .values()
        .filter(|e| e.owner_uq == owner_uq)
        .map(|e| e.amount)
        .sum()
}

// ====================
// 🪞 MIROIR BLOCKORB — le disque dur suit la réalité SQLite
// ====================
//
// Les soldes dépensables vivent dans `balance.points` (SQLite).
// Le BlockOrb est le disque dur global : il doit refléter les mêmes
// montants, sinon les deux registres divergent.
//
// Règles :
//   - débit expéditeur = montant + frais
//   - crédit destinataire = montant
//   - frais répartis dans les 6 Cases (distribuer_frais)
//   - si l'expéditeur n'a pas assez dans le BlockOrb, la différence
//     est prélevée sur le Trésor (cases[0]) → l'invariant 40 Md reste exact
//   - BEST-EFFORT : ne bloque jamais le transfert déjà validé en SQLite
pub fn mirroir_transfert(
    sender: &str,
    recipient: &str,
    amount_points: u128,
    fee_points: u128,
) -> Result<(), String> {
    let _garde = verrou_orb();
    let mut orb = load_global_orb();
    let avant = orb.clone();

    // femto-BORB par point (1 BORB = 100 points)
    let femto_par_point = crate::montant::FEMTO_PAR_BORB / 100;
    let total = (amount_points + fee_points) * femto_par_point;
    let recu = amount_points * femto_par_point;
    let frais = fee_points * femto_par_point;

    if !orb.comptes.contains_key(sender) {
        return Err(format!("portefeuille {} absent du BlockOrb", sender));
    }
    if !orb.comptes.contains_key(recipient) {
        return Err(format!("portefeuille {} absent du BlockOrb", recipient));
    }
    // Défense en profondeur : le miroir ne credite jamais un wallet sans
    // cadenas structurel lie (la porte a deja refuse en amont du SQLite).
    if !orb.porte_cadenas(recipient) {
        return Err(format!("CADENAS_NON_LIE: miroir refuse vers {}", recipient));
    }

    // 1. Réconciliation éventuelle depuis le Trésor
    let solde_sender = orb.comptes.get(sender).map(|p| p.solde).unwrap_or(0);
    if solde_sender < total {
        let manque = total - solde_sender;
        if orb.cases[0] < manque {
            return Err(format!(
                "Trésor insuffisant pour réconcilier {} (manque {} femto)",
                sender, manque
            ));
        }
        orb.cases[0] -= manque;
        if let Some(p) = orb.comptes.get_mut(sender) {
            p.solde = total;
        }
        eprintln!(
            "🪞 Réconcilié {} : +{} femto prélevés sur le Trésor",
            sender, manque
        );
    }

    // 2. Débit / crédit
    if let Some(p) = orb.comptes.get_mut(sender) {
        p.solde = p.solde.saturating_sub(total);
    }
    if let Some(p) = orb.comptes.get_mut(recipient) {
        p.solde = p.solde.saturating_add(recu);
    }

    // 3. Frais → 6 Cases (30/40/15/10/5/0)
    if frais > 0 {
        if let Err(e) = orb.distribuer_frais(frais) {
            orb = avant;
            return Err(e);
        }
    }

    // 4. Invariant
    if !orb.verifier() {
        orb = avant;
        return Err("Invariant cassé par le miroir — BlockOrb inchangé".to_string());
    }

    // 5. Sceller le bloc (numéro + horodatage + hash de chaînage)
    orb.sceller_bloc();

    save_global_orb(&orb)
}

/// 🪞 FRAIS SEUL — le vendeur paie la plateforme (présence d'une vitrine).
///
/// Rien n'est crédité à un destinataire : la totalité du débit part dans les
/// 6 Cases, comme le frais d'un transfert. Meilleure-effort, à l'image de
/// `mirroir_transfert` : un échec laisse le BlockOrb inchangé et le SQLite
/// seul fait foi ; `reconcilier_blockorb` réaligne plus tard.
pub fn mirroir_frais(sender: &str, fee_points: u128) -> Result<(), String> {
    if fee_points == 0 {
        return Ok(());
    }
    let _garde = verrou_orb();
    let mut orb = load_global_orb();
    let avant = orb.clone();

    let femto_par_point = crate::montant::FEMTO_PAR_BORB / 100;
    let frais = fee_points * femto_par_point;

    if !orb.comptes.contains_key(sender) {
        return Err(format!("portefeuille {} absent du BlockOrb", sender));
    }

    let solde = orb.comptes.get(sender).map(|p| p.solde).unwrap_or(0);
    if solde < frais {
        let manque = frais - solde;
        if orb.cases[0] < manque {
            return Err(format!(
                "Trésor insuffisant pour réconcilier {} (manque {} femto)",
                sender, manque
            ));
        }
        orb.cases[0] -= manque;
        if let Some(p) = orb.comptes.get_mut(sender) {
            p.solde = frais;
        }
        eprintln!(
            "🪞 Réconcilié {} : +{} femto prélevés sur le Trésor",
            sender, manque
        );
    }

    if let Some(p) = orb.comptes.get_mut(sender) {
        p.solde = p.solde.saturating_sub(frais);
    }

    if let Err(e) = orb.distribuer_frais(frais) {
        orb = avant;
        return Err(e);
    }

    if !orb.verifier() {
        orb = avant;
        return Err("Invariant cassé par le miroir — BlockOrb inchangé".to_string());
    }

    orb.sceller_bloc();
    save_global_orb(&orb)
}

/// 🪞 RÉCONCILIATION — aligne le BlockOrb sur les soldes SQLite.
///
/// Pour chaque portefeuille visé possédant une DB :
///   cible = balance.points * FEMTO_PAR_BORB / 100
/// La différence est prélevée sur le Trésor (cases[0]) ou lui est rendue,
/// ce qui préserve exactement l'invariant des 40 Md BORB.
/// Les comptes présents dans data/users mais absents du BlockOrb sont créés.
///
/// `seulement` = Some(uq) pour ne traiter qu'un compte (cas non-admin).
/// Retourne (uq, delta_femto, applique) par compte modifié.
pub fn reconcilier_blockorb(seulement: Option<&str>) -> Result<Vec<(String, i128, bool)>, String> {
    let _garde = verrou_orb();
    let mut orb = load_global_orb();
    let femto_par_point = crate::montant::FEMTO_PAR_BORB / 100;

    // ===== Déterminer les comptes à traiter =====
    let mut uqs: Vec<String> = match seulement {
        Some(uq) => vec![uq.to_string()],
        None => {
            let mut v: Vec<String> = orb.comptes.keys().cloned().collect();

            // Comptes présents sur disque mais absents du BlockOrb
            let users_dir = users_dir_path();
            if let Ok(entries) = std::fs::read_dir(&users_dir) {
                for entry in entries.flatten() {
                    let nom = entry.file_name().to_string_lossy().to_string();
                    if !nom.starts_with("uq_") || !nom.ends_with(".db") {
                        continue;
                    }
                    let brut = &nom[3..nom.len() - 3];
                    if brut.is_empty() || !brut.chars().all(|c| c.is_ascii_digit()) {
                        continue;
                    }
                    let uq = format!("UQ-{}", brut);
                    if !v.contains(&uq) {
                        v.push(uq);
                    }
                }
            }
            v
        }
    };

    uqs.sort();
    uqs.dedup();

    let mut rapport: Vec<(String, i128, bool)> = Vec::new();

    for uq in uqs {
        let db_path = format!("data/users/{}.db", uq.replace("-", "_").to_lowercase());
        if !Path::new(&db_path).exists() {
            continue;
        }

        let points: i64 = match rusqlite::Connection::open(&db_path) {
            Ok(conn) => conn
                .query_row("SELECT points FROM balance WHERE id = 1", [], |r| r.get(0))
                .unwrap_or(0),
            Err(_) => continue,
        };

        let cible = (points.max(0) as u128) * femto_par_point;
        let actuel = orb.comptes.get(&uq).map(|p| p.solde).unwrap_or(0);

        // PORTE BORB : réconcilier revient à prélever du Trésor. Seul un
        // wallet porteur d'un cadenas structurel y a droit, sinon la
        // réconciliation ressusciterait du BORB non tracé en base SQLite.
        let cadenas_db = crate::kyc::cadenas::cadenas_porte_par(&uq);
        let cadenas_credible = cadenas_db
            .as_deref()
            .map(crate::block_orb::cadenas_credible)
            .unwrap_or(false);

        if orb.comptes.contains_key(&uq) {
            if !orb.porte_cadenas(&uq) {
                if cadenas_credible {
                    // Lien perdu : on le reconstruit depuis la DB, sans valeur.
                    if let Some(p) = orb.comptes.get_mut(&uq) {
                        p.cadenas = cadenas_db.clone();
                    }
                } else {
                    rapport.push((uq.clone(), 0, false));
                    continue;
                }
            }
        } else {
            if !cadenas_credible {
                rapport.push((uq.clone(), 0, false));
                continue;
            }
            if orb.cases[0] < cible {
                rapport.push((uq.clone(), cible as i128, false));
                continue;
            }
            match orb.creer_compte(&uq, cible, cadenas_db.as_deref().unwrap_or_default()) {
                Ok(()) => {
                    rapport.push((uq.clone(), cible as i128, true));
                    continue;
                }
                Err(e) => {
                    eprintln!("⚠️  Réconciliation {} refusée : {}", uq, e);
                    rapport.push((uq.clone(), cible as i128, false));
                    continue;
                }
            }
        }

        // Rien à faire si le solde est déjà bon ET que le portefeuille existe
        if actuel == cible {
            continue;
        }

        if cible > actuel {
            let manque = cible - actuel;
            if orb.cases[0] < manque {
                rapport.push((uq.clone(), manque as i128, false));
                continue;
            }
            orb.cases[0] -= manque;
            rapport.push((uq.clone(), manque as i128, true));
        } else {
            let surplus = actuel - cible;
            orb.cases[0] += surplus;
            rapport.push((uq.clone(), -(surplus as i128), true));
        }

        // Le portefeuille existe (garanti par la porte ci-dessus).
        if let Some(p) = orb.comptes.get_mut(&uq) {
            p.solde = cible;
        }
    }

    if !orb.verifier() {
        return Err("Invariant cassé — réconciliation annulée (aucune écriture)".to_string());
    }

    save_global_orb(&orb)?;
    Ok(rapport)
}
