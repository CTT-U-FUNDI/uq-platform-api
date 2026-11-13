// ====================
// 👤 PROFILE ROUTES — stats publiques, likes, vues, abonnés, notation
// ====================
//
// Règle : chaque publication CONSERVE ses données. Les compteurs du
// profil se lisent dans les tables persistantes (publication_likes,
// publication_views, followers, profile_ratings), qui survivent à la
// purge 7 jours des publications elles-mêmes.
//
// Score de confiance sur 100 (barème de la spécification) :
//   ancienneté 20 · KYC 20 · réputation 20 · activité 20 · communauté 20
// Badge « Certifié UQ » : score 100 + KYC + aucune sanction + > 1 an.
// Le numéro UQ n'est JAMAIS renvoyé : pseudo public uniquement.

use actix_web::{web, HttpRequest, HttpResponse};
use rusqlite::Connection;
use serde_json::json;

use crate::database::Database;

const SEUILS_ANCIENNETE_SECS: [i64; 4] = [
    6 * 30 * 24 * 3600, // 6 mois
    12 * 30 * 24 * 3600, // 1 an
    24 * 30 * 24 * 3600, // 2 ans
    i64::MAX,
];
const POINTS_ANCIENNETE: [u32; 4] = [5, 10, 15, 20];
const POINTS_KYC: u32 = 20;
const AVIS_MINIMUM_PTS_NOTE: i64 = 10;
const MAX_PT_PUBLICATIONS: u32 = 10;
const MAX_PT_VENTES: u32 = 5;
const MAX_PT_REPONSES: u32 = 5;
const MAX_PT_ABONNES: u32 = 10;
const MAX_PT_LIKES: u32 = 5;
const MAX_PT_VUES: u32 = 5;
const PAS_ABONNES: i64 = 100;
const PAS_LIKES: i64 = 100;
const PAS_VUES: i64 = 1000;
/// Signalements confirmes par la moderation : 10 points retirés chacun, dans la
/// limite de 30. Le total reste sur 100, la sanction ne fait pas un score negatif.
const POINTS_PAR_SIGNALEMENT: u32 = 10;
const PLAFOND_SIGNALEMENTS: u32 = 30;
/// Un emetteur dont 3 signalements ont ete juges abusifs perd 5 points par
/// dossier, plafonnes a 15 : une erreur isolee n'est pas sanctionnee.
const SEUIL_SIGNALEMENTS_ABUSIFS: i64 = 3;
const POINTS_PAR_ABUS: u32 = 5;
const PLAFOND_ABUS: u32 = 15;
const INDEX_ANCRE: &str = "uq_index.db";
const CHEMIN_INDEX: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/uq_index.db");
/// Ordre de recherche du registre : le répertoire courant du serveur d'abord
/// (comme `data/users/*.db` et `uq_platform_db.db`), l'emplacement d'origine en
/// repli. Le relatif-d'abord permet à un bac de test isolé de lire sa propre
/// copie sans jamais toucher au registre de production.
const CHEMINS_REGISTRE: [&str; 2] = [INDEX_ANCRE, CHEMIN_INDEX];
const ANCIENNETE_CERTIFICATION_SECS: i64 = 12 * 30 * 24 * 3600;

fn normaliser(uq: &str) -> String {
    uq.trim().to_uppercase()
}

/// Map pure du registre : UQ -> date d'inscription.
fn carte_registre(rows: Vec<(String, i64)>) -> std::collections::HashMap<String, i64> {
    rows.into_iter()
        .map(|(uq, cree_le)| (normaliser(&uq), cree_le.max(0)))
        .collect()
}

/// Dates d'inscription lues là où les comptes existent vraiment : la table
/// `uq_index` de uq_index.db. La table `accounts` de uq_platform_db.db n'est
/// que le miroir d'un seed JSON et reste vide.
fn lire_inscriptions(conn: &rusqlite::Connection, uqs: &[&str]) -> Vec<(String, i64)> {
    let marqueurs = uqs.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT uq_number, COALESCE(created_at, 0) FROM uq_index WHERE uq_number IN ({marqueurs})"
    );
    let Ok(mut stmt) = conn.prepare(&sql) else {
        return vec![];
    };
    let Ok(rows) = stmt.query_map(rusqlite::params_from_iter(uqs.iter()), |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1).unwrap_or(0)))
    }) else {
        return vec![];
    };
    rows.filter_map(|x| x.ok()).collect()
}

/// Dates d'inscription du registre, dans le premier chemin de
/// `CHEMINS_REGISTRE` qui répond.
fn inscriptions(uqs: &[&str]) -> std::collections::HashMap<String, i64> {
    let mut sortie = std::collections::HashMap::new();
    if uqs.is_empty() {
        return sortie;
    }
    for chemin in CHEMINS_REGISTRE {
        let Ok(conn) = rusqlite::Connection::open(chemin) else {
            continue;
        };
        let bruts = lire_inscriptions(&conn, uqs);
        if !bruts.is_empty() {
            sortie = carte_registre(bruts);
            break;
        }
    }
    sortie
}

/// Ouvre le registre sans le créer : un chemin absent est ignoré.
fn connexion_registre() -> Option<rusqlite::Connection> {
    CHEMINS_REGISTRE
        .into_iter()
        .filter(|chemin| std::path::Path::new(chemin).exists())
        .find_map(|chemin| rusqlite::Connection::open(chemin).ok())
}

/// Ventes reçues par compte, en une passe. Deux sources, disjointes : les
/// transactions en points de la plateforme et les transferts de devise du
/// registre (`wallet_transfers`). Le chemin fiat réel ne passe pas par la
/// mémoire de la plateforme : sans le registre, un vendeur qui encaisse reste
/// à 0 vente et les 5 points d'activité — donc le badge — sont inatteignables.
fn ventes_par_compte(db: &Database) -> std::collections::HashMap<String, i64> {
    let mut compteurs = db.compter_transactions_recues();
    let Some(conn) = connexion_registre() else {
        return compteurs;
    };
    let Ok(mut stmt) = conn.prepare(
        "SELECT recipient_uq, COUNT(*) FROM wallet_transfers GROUP BY recipient_uq",
    ) else {
        return compteurs;
    };
    let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
    else {
        return compteurs;
    };
    for (destinataire, n) in rows.flatten() {
        *compteurs.entry(normaliser(&destinataire)).or_insert(0) += n;
    }
    compteurs
}

/// KYC réel d'un compte : son cadenas (`est_verifie` sans verrouillage), même
/// autorité que `permissions::compute_permissions`. La colonne
/// `uq_index.kyc_verified` n'est écrite par aucun code : toujours 0.
fn kyc_verifie(uq: &str) -> bool {
    let chemin = format!("data/users/{}.db", uq.replace("-", "_").to_lowercase());
    let Ok(conn) = rusqlite::Connection::open(&chemin) else {
        return false;
    };
    conn.query_row(
        "SELECT 1 FROM cadenas_verification WHERE id = 1 AND est_verifie = 1 AND verrouille = 0",
        [],
        |_| Ok(()),
    )
    .is_ok()
}

fn compte_valide(_conn: &Connection, uq: &str) -> bool {
    inscriptions(&[uq]).contains_key(&normaliser(uq))
}

/// Accepte un UQ ou un pseudo public (« uq@jean », « jean ») et rend le UQ.
/// Le client n'a jamais besoin de connaître le numéro : la route /api/profile
/// peut être appelée avec le pseudo affiché.
fn resoudre_identifiant(saisie: &str) -> Option<String> {
    let brut = normaliser(saisie);
    if crate::auth_guard::ressemble_a_un_uq(&brut) {
        return Some(brut);
    }
    crate::uq_public_alias::resoudre_destinataire(saisie)
        .map(|uq| normaliser(&uq))
        .filter(|uq| crate::auth_guard::ressemble_a_un_uq(uq))
}

fn identites(uqs: &[String]) -> Vec<serde_json::Value> {
    uqs.iter()
        .map(|uq| {
            let (pseudo, name) = crate::uq_public_alias::get_public_identity(uq);
            json!({
                "pseudo": pseudo,
                "name": name,
                "avatar_url": crate::uq_public_alias::get_public_avatar_url(uq),
            })
        })
        .collect()
}

fn lire_uqs(conn: &Connection, sql: &str, cible: &str, limite: i64, decalage: i64) -> Vec<String> {
    let mut stmt = match conn.prepare(sql) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    stmt.query_map(rusqlite::params![cible, limite, decalage], |r| r.get::<_, String>(0))
        .map(|rows| rows.filter_map(|x| x.ok()).collect())
        .unwrap_or_default()
}

// ====================
// AGREGATS
// ====================

#[derive(Clone)]
struct Agrégats {
    publications: i64,
    likes_recus: i64,
    vues_recues: i64,
    commentaires_recus: i64,
    commentaires_ecrits: i64,
    ventes: i64,
    abonne: i64,
    abonnements: i64,
    note_moyenne: f64,
    nb_avis: i64,
    anciennete_secs: i64,
    kyc: bool,
    fraude: bool,
    /// Signalements de chat confirmes par la moderation contre ce compte.
    signalements_confirmes: i64,
    /// Signalements emis par ce compte et rejetes comme abusifs.
    signalements_abusifs: i64,
}

fn agregats(db: &Database, conn: &Connection, uq: &str) -> Agrégats {
    let count = |sql: &str| -> i64 {
        conn.query_row(sql, [uq], |r| r.get::<_, i64>(0)).unwrap_or(0)
    };
    let mut publications = 0i64;
    let mut commentaires_recus = 0i64;
    if let Ok(mut stmt) = conn.prepare("SELECT json FROM publications") {
        if let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(0)) {
            for raw in rows.flatten() {
                if let Ok(record) = serde_json::from_str::<crate::database::PublicationRecord>(&raw) {
                    if record.author == uq {
                        publications += 1;
                        commentaires_recus += record.comments.len() as i64;
                    }
                }
            }
        }
    }
    let commentaires_ecrits = {
        let mut n = 0i64;
        if let Ok(mut stmt) = conn.prepare("SELECT json FROM publications") {
            if let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(0)) {
                for raw in rows.flatten() {
                    if let Ok(record) = serde_json::from_str::<crate::database::PublicationRecord>(&raw) {
                        n += record
                            .comments
                            .iter()
                            .filter(|c| c.author == uq)
                            .count() as i64;
                    }
                }
            }
        }
        n
    };
    let ventes = ventes_par_compte(db)
        .get(&normaliser(uq))
        .copied()
        .unwrap_or(0);
    let (note_moyenne, nb_avis) = conn
        .query_row(
            "SELECT COALESCE(AVG(note), 0.0), COUNT(*) FROM profile_ratings WHERE uq_note = ?1",
            [uq],
            |r| Ok((r.get::<_, f64>(0)?, r.get::<_, i64>(1)?)),
        )
        .unwrap_or((0.0, 0));
    let cree_le = inscriptions(&[uq]).get(&normaliser(uq)).copied().unwrap_or(0);
    let kyc = kyc_verifie(uq);
    let fraude = db
        .get_sanctions(1000)
        .iter()
        .any(|s| {
            ["account_id", "uq", "id"]
                .iter()
                .filter_map(|k| s.get(*k).and_then(|v| v.as_str()))
                .any(|v| normaliser(v) == uq)
        });
    // Signalements de chat : stockés dans le registre central, pas dans la base
    // du compte. Un registre indisponible laisse les compteurs a zero plutôt que
    // de faire echouer le profil.
    let (signalements_confirmes, signalements_abusifs) =
        crate::chat_reports::compteurs_signalements(uq);
    Agrégats {
        publications,
        likes_recus: count("SELECT COUNT(*) FROM publication_likes WHERE uq_auteur = ?1"),
        vues_recues: count("SELECT COUNT(*) FROM publication_views WHERE uq_auteur = ?1"),
        commentaires_recus,
        commentaires_ecrits,
        ventes,
        abonne: count("SELECT COUNT(*) FROM followers WHERE uq_followed = ?1"),
        abonnements: count("SELECT COUNT(*) FROM followers WHERE uq_follower = ?1"),
        note_moyenne,
        nb_avis,
        anciennete_secs: (chrono::Utc::now().timestamp() - cree_le).max(0),
        kyc,
        fraude,
        signalements_confirmes,
        signalements_abusifs,
    }
}

/// Barème spec : composantes séparées, total sur 100.
fn score(a: &Agrégats) -> serde_json::Value {
    let anciennete = SEUILS_ANCIENNETE_SECS
        .iter()
        .zip(POINTS_ANCIENNETE.iter())
        .find(|(seuil, _)| a.anciennete_secs <= **seuil)
        .map(|(_, p)| *p)
        .unwrap_or(POINTS_ANCIENNETE[POINTS_ANCIENNETE.len() - 1]);
    let kyc = if a.kyc { POINTS_KYC } else { 0 };
    let reputation = if a.nb_avis >= AVIS_MINIMUM_PTS_NOTE {
        ((a.note_moyenne * 4.0).min(20.0)).round() as u32
    } else {
        0
    };
    let publications = (a.publications as u32).min(MAX_PT_PUBLICATIONS);
    let ventes = (a.ventes as u32).min(MAX_PT_VENTES);
    let reponses = (a.commentaires_ecrits as u32).min(MAX_PT_REPONSES);
    let communaute_abonnes = ((a.abonne / PAS_ABONNES).max(0) as u32).min(MAX_PT_ABONNES);
    let communaute_likes = ((a.likes_recus / PAS_LIKES).max(0) as u32).min(MAX_PT_LIKES);
    let communaute_vues = ((a.vues_recues / PAS_VUES).max(0) as u32).min(MAX_PT_VUES);
    // Sanctions issues de la moderation du chat, soustraites du total.
    let sanction_subie = (a.signalements_confirmes.max(0) as u32 * POINTS_PAR_SIGNALEMENT)
        .min(PLAFOND_SIGNALEMENTS);
    let sanction_emise = if a.signalements_abusifs >= SEUIL_SIGNALEMENTS_ABUSIFS {
        (a.signalements_abusifs as u32 * POINTS_PAR_ABUS).min(PLAFOND_ABUS)
    } else {
        0
    };
    let sanction = sanction_subie + sanction_emise;
    let total = (anciennete + kyc + reputation + publications + ventes + reponses
        + communaute_abonnes + communaute_likes + communaute_vues)
        .saturating_sub(sanction);
    let certifie = total == 100
        && a.kyc
        && !a.fraude
        && sanction == 0
        && a.anciennete_secs > ANCIENNETE_CERTIFICATION_SECS;
    json!({
        "total": total,
        "composantes": {
            "anciennete": anciennete,
            "kyc": kyc,
            "reputation": reputation,
            "publications": publications,
            "ventes": ventes,
            "reponses": reponses,
            "abonnes": communaute_abonnes,
            "likes": communaute_likes,
            "vues": communaute_vues,
        },
        "sanction": {
            "signalements_confirmes": sanction_subie,
            "signalements_abusifs": sanction_emise,
            "total": sanction,
        },
        "certifie": certifie,
        "badge": if certifie { "Certifié UQ" } else { "" },
    })
}

// ====================
// HANDLERS
// ====================

/// Deux colonnes (cle, compteur) lues en un seul `GROUP BY`.
fn groupe_de_compteurs(conn: &Connection, sql: &str) -> std::collections::HashMap<String, i64> {
    let mut sortie = std::collections::HashMap::new();
    let Ok(mut stmt) = conn.prepare(sql) else {
        return sortie;
    };
    let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))) else {
        return sortie;
    };
    for (cle, valeur) in rows.flatten() {
        sortie.insert(normaliser(&cle), valeur);
    }
    sortie
}

/// Badges de tout le flux en une passe : le flux de publications affiche le
/// même badge que le profil, sans recalculer les agrégats pour chaque post.
pub fn badges_collectifs(
    db: &Database,
    conn: &Connection,
    auteurs: &[String],
) -> std::collections::HashMap<String, String> {
    let mut resultat = std::collections::HashMap::new();
    let cibles: std::collections::HashSet<String> =
        auteurs.iter().map(|a| normaliser(a)).filter(|a| !a.is_empty()).collect();
    if cibles.is_empty() {
        return resultat;
    }

    let lire = |map: &std::collections::HashMap<String, i64>, cle: &str| -> i64 {
        map.get(cle).copied().unwrap_or(0)
    };

    // Une seule lecture des publications : total par auteur + commentaires.
    let mut publications = std::collections::HashMap::<String, i64>::new();
    let mut recus = std::collections::HashMap::<String, i64>::new();
    let mut ecrits = std::collections::HashMap::<String, i64>::new();
    if let Ok(mut stmt) = conn.prepare("SELECT json FROM publications") {
        if let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(0)) {
            for raw in rows.flatten() {
                let Ok(record) = serde_json::from_str::<crate::database::PublicationRecord>(&raw)
                else {
                    continue;
                };
                let auteur = normaliser(&record.author);
                if cibles.contains(&auteur) {
                    *publications.entry(auteur.clone()).or_insert(0) += 1;
                    *recus.entry(auteur.clone()).or_insert(0) += record.comments.len() as i64;
                }
                for commentaire in &record.comments {
                    let cle = normaliser(&commentaire.author);
                    if cibles.contains(&cle) {
                        *ecrits.entry(cle).or_insert(0) += 1;
                    }
                }
            }
        }
    }

    let likes = groupe_de_compteurs(
        conn,
        "SELECT uq_auteur, COUNT(*) FROM publication_likes GROUP BY uq_auteur",
    );
    let vues = groupe_de_compteurs(
        conn,
        "SELECT uq_auteur, COUNT(*) FROM publication_views GROUP BY uq_auteur",
    );
    let abonnes = groupe_de_compteurs(
        conn,
        "SELECT uq_followed, COUNT(*) FROM followers GROUP BY uq_followed",
    );
    let abonnements = groupe_de_compteurs(
        conn,
        "SELECT uq_follower, COUNT(*) FROM followers GROUP BY uq_follower",
    );

    let mut notes = std::collections::HashMap::<String, (f64, i64)>::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT uq_note, AVG(note), COUNT(*) FROM profile_ratings GROUP BY uq_note",
    ) {
        if let Ok(rows) = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, f64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        }) {
            for (cle, moyenne, avis) in rows.flatten() {
                notes.insert(normaliser(&cle), (moyenne, avis));
            }
        }
    }

    // Sanctions de chat, en deux passes pour toute la file : le meme barème que
    // `agregats()`, sinon le classement et la fiche d'un compte se contrediraient.
    let signalements_subis = groupe_de_compteurs(
        conn,
        "SELECT target_uq, COUNT(*) FROM chat_reports WHERE statut = 'confirmee' GROUP BY target_uq",
    );
    let signalements_abusifs = groupe_de_compteurs(
        conn,
        "SELECT reporter_uq, COUNT(*) FROM chat_reports WHERE statut = 'rejetee' GROUP BY reporter_uq",
    );

    let fraudeurs: std::collections::HashSet<String> = db
        .get_sanctions(1000)
        .iter()
        .flat_map(|sanction| {
            ["account_id", "uq", "id"]
                .iter()
                .filter_map(|cle| sanction.get(*cle).and_then(|v| v.as_str()).map(normaliser))
                .collect::<Vec<String>>()
        })
        .collect();
    let ventes_recues = ventes_par_compte(db);
    let uqs: Vec<&str> = cibles.iter().map(String::as_str).collect();
    let registre = inscriptions(&uqs);

    for auteur in &cibles {
        let cree_le = registre.get(auteur.as_str()).copied().unwrap_or(0);
        let kyc = kyc_verifie(auteur);
        let (note_moyenne, nb_avis) = notes.get(auteur).copied().unwrap_or((0.0, 0));
        let agregat = Agrégats {
            publications: lire(&publications, auteur),
            likes_recus: lire(&likes, auteur),
            vues_recues: lire(&vues, auteur),
            commentaires_recus: lire(&recus, auteur),
            commentaires_ecrits: lire(&ecrits, auteur),
            ventes: lire(&ventes_recues, auteur),
            abonne: lire(&abonnes, auteur),
            abonnements: lire(&abonnements, auteur),
            note_moyenne,
            nb_avis,
            anciennete_secs: (chrono::Utc::now().timestamp() - cree_le).max(0),
            kyc,
            fraude: fraudeurs.contains(auteur),
            signalements_confirmes: lire(&signalements_subis, auteur),
            signalements_abusifs: lire(&signalements_abusifs, auteur),
        };
        let badge = score(&agregat)["badge"]
            .as_str()
            .unwrap_or("")
            .to_string();
        if !badge.is_empty() {
            resultat.insert(auteur.clone(), badge);
        }
    }
    resultat
}


/// GET /api/profile/stats/{uq} — stats agrégées + score + badge (public).
pub async fn profile_stats(req: HttpRequest, path: web::Path<String>) -> HttpResponse {
    let _ = &req;
    let uq = match resoudre_identifiant(&path.into_inner()) {
        Some(uq) => uq,
        None => {
            return HttpResponse::BadRequest()
                .json(json!({"status":"error","message":"Identifiant de compte invalide"}))
        }
    };
    let db = Database::shared("uq_platform_db.json");
    // Calcul de total_reseaux : invitations envoyees et acceptees
    let total_reseaux: i64 = if let Ok(conn) = db.conn() {
        conn.query_row(
            "SELECT COUNT(*) FROM followers WHERE uq_follower = ?1 AND status = 'accepted'",
            rusqlite::params![uq],
            |row| row.get(0),
        ).unwrap_or(0)
    } else {
        0
    };

    // Reputation : 5 couleurs
    let reputation = {
        let conn_opt = db.conn().ok();
        conn_opt.and_then(|c| {
            c.query_row(
                "SELECT vert, blanc, orange, rouge, noir, vert_depuis_rouge FROM reputation WHERE uq = ?1",
                rusqlite::params![uq],
                |row| Ok(serde_json::json!({
                    "vert": row.get::<_, i64>(0)?,
                    "blanc": row.get::<_, i64>(1)?,
                    "orange": row.get::<_, i64>(2)?,
                    "rouge": row.get::<_, i64>(3)?,
                    "noir": row.get::<_, i64>(4)?,
                    "vert_depuis_rouge": row.get::<_, i64>(5)?,
                }))
            ).ok()
        }).unwrap_or(serde_json::json!({
            "vert": 0, "blanc": 0, "orange": 0, "rouge": 0, "noir": 0, "vert_depuis_rouge": 0
        }))
    };

    let conn = match db.connexion() {
        Ok(c) => c,
        Err(e) => return HttpResponse::InternalServerError().json(json!({"status":"error","message":e})),
    };
    if !compte_valide(&conn, &uq) {
        return HttpResponse::NotFound().json(json!({"status":"error","message":"Compte introuvable"}));
    }
    let a = agregats(&db, &conn, &uq);
    let scores = score(&a);
    let (pseudo, name) = crate::uq_public_alias::get_public_identity(&uq);
    let lecteur = crate::auth_guard::exiger_auth(&req).ok().map(|c| normaliser(&c.sub));
    let suit = lecteur
        .as_ref()
        .map(|l| {
            conn.query_row(
                "SELECT 1 FROM followers WHERE uq_follower = ?1 AND uq_followed = ?2",
                rusqlite::params![l, uq],
                |_| Ok(()),
            )
            .is_ok()
        })
        .unwrap_or(false);
    let amis = lecteur
        .as_ref()
        .map(|l| {
            conn.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM followers
                    WHERE uq_follower = ?1 AND uq_followed = ?2
                 ) AND EXISTS(
                    SELECT 1 FROM followers
                    WHERE uq_follower = ?2 AND uq_followed = ?1
                 )",
                rusqlite::params![l, uq],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0)
                == 1
        })
        .unwrap_or(false);
    let preferences_path = format!("data/users/{}.db", uq.replace("-", "_").to_lowercase());
    let mut audiences = std::collections::HashMap::new();
    if let Ok(preferences_conn) = rusqlite::Connection::open(&preferences_path) {
        if let Ok(mut statement) = preferences_conn.prepare(
            "SELECT key, value FROM settings
             WHERE key IN (
                'privacy_description', 'privacy_city', 'privacy_origin_city',
                'privacy_real_name',
                'privacy_occupation', 'privacy_marital_status',
                'privacy_mobile_money_number', 'privacy_trading_currency'
             )",
        ) {
            if let Ok(rows) = statement.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            }) {
                audiences.extend(rows.flatten());
            }
        }
    }
    let own_profile = lecteur.as_deref() == Some(uq.as_str());
    let visible_to_reader = |key: &str| {
        own_profile
            || match audiences.get(key).map(String::as_str).unwrap_or("private") {
                "public" => true,
                "friends" => amis,
                _ => false,
            }
    };
    let profile_fields =
        crate::uq_index::UqIndex::shared("uq_index.db").get_full_profile(&uq);
    let public_alias = rusqlite::Connection::open(&preferences_path)
        .and_then(|connection| {
            connection.query_row(
                "SELECT display_name FROM uq_aliases WHERE real_uq = ?1 AND active = 1 LIMIT 1",
                rusqlite::params![uq],
                |row| row.get::<_, String>(0),
            )
        })
        .ok()
        .filter(|alias| !alias.trim().is_empty());
    let display_name = if own_profile || visible_to_reader("privacy_real_name") {
        name
    } else {
        public_alias.unwrap_or_else(|| pseudo.clone())
    };
    let public_fields = [
        ("privacy_description", "description", "description"),
        ("privacy_city", "ville_actuelle", "city"),
        ("privacy_origin_city", "ville_provenance", "origin_city"),
        ("privacy_occupation", "emploi", "occupation"),
        ("privacy_marital_status", "statut_marital", "marital_status"),
        (
            "privacy_mobile_money_number",
            "mobile_money_number",
            "mobile_money_number",
        ),
        ("privacy_trading_currency", "display_currency", "trading_currency"),
    ];
    let mut public_profile = serde_json::Map::new();
    for (preference, field, output) in public_fields {
        if !visible_to_reader(preference) {
            continue;
        }
        if let Some(value) = profile_fields.get(field).filter(|value| !value.trim().is_empty()) {
            public_profile.insert(output.to_string(), json!(value));
        }
    }
    let (demande_envoyee_par_moi, demande_envoyee_vers_moi, _est_ami_expose) = if let Some(ref lecteur_uq) = lecteur {
        let env: i64 = conn.query_row(
            "SELECT COUNT(*) FROM followers WHERE uq_follower = ?1 AND uq_followed = ?2 AND status = 'pending'",
            rusqlite::params![lecteur_uq, uq],
            |row| row.get(0),
        ).unwrap_or(0);
        let recu: i64 = conn.query_row(
            "SELECT COUNT(*) FROM followers WHERE uq_follower = ?1 AND uq_followed = ?2 AND status = 'pending'",
            rusqlite::params![uq, lecteur_uq],
            |row| row.get(0),
        ).unwrap_or(0);
        let ami: i64 = conn.query_row(
            "SELECT COUNT(*) FROM followers WHERE uq_follower = ?1 AND uq_followed = ?2 AND status = 'accepted'",
            rusqlite::params![lecteur_uq, uq],
            |row| row.get(0),
        ).unwrap_or(0);
        (env > 0, recu > 0, ami > 0)
    } else {
        (false, false, false)
    };
    let couleur_anneau = {
        let rep = reputation.as_object();
        let v = rep.and_then(|o| o.get("vert")).and_then(|x| x.as_i64()).unwrap_or(0);
        let b = rep.and_then(|o| o.get("blanc")).and_then(|x| x.as_i64()).unwrap_or(0);
        let o2 = rep.and_then(|o| o.get("orange")).and_then(|x| x.as_i64()).unwrap_or(0);
        let r = rep.and_then(|o| o.get("rouge")).and_then(|x| x.as_i64()).unwrap_or(0);
        let n = rep.and_then(|o| o.get("noir")).and_then(|x| x.as_i64()).unwrap_or(0);
        crate::reputation_routes::couleur_dominante(v, b, o2, r, n)
    };

    HttpResponse::Ok().json(json!({
        "status": "success",
        "pseudo": pseudo,
        "name": display_name,
        "avatar_url": crate::uq_public_alias::get_public_avatar_url(&uq),
        "public_profile": public_profile,
        "couleur_anneau": couleur_anneau,
        "est_ami": amis,
        "demande_envoyee_par_moi": demande_envoyee_par_moi,
        "demande_envoyee_vers_moi": demande_envoyee_vers_moi,
        "stats": {
            "total_publications": a.publications,
            "total_likes_recus": a.likes_recus,
            "reputation": reputation,
            "total_vues_recues": a.vues_recues,
            "total_commentaires_recus": a.commentaires_recus,
            "total_commentaires_ecrits": a.commentaires_ecrits,
            "total_abonnes": a.abonne,
            "total_reseaux": total_reseaux,
            "total_abonnements": a.abonnements,
            "note_moyenne": (a.note_moyenne * 10.0).round() / 10.0,
            "nb_avis": a.nb_avis,
            "ventes": a.ventes,
            "anciennete_jours": a.anciennete_secs / 86400,
            "kyc_verifie": a.kyc,
        },
        "score_confiance": scores,
        "vous_suivez_deja": suit,
    }))
}

/// POST /api/publications/{id}/like — ajout idempotent, compte renvoyé.
pub async fn publication_like_add(req: HttpRequest, path: web::Path<String>) -> HttpResponse {
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let id = path.into_inner();
    let db = Database::shared("uq_platform_db.json");
    let liker = normaliser(&claims.sub);
    let conn = match db.connexion() {
        Ok(c) => c,
        Err(e) => return HttpResponse::InternalServerError().json(json!({"status":"error","message":e})),
    };
    let auteur: Option<String> = conn
        .query_row(
            "SELECT json_extract(json, '$.author') FROM publications WHERE id = ?1",
            [&id],
            |r| r.get(0),
        )
        .ok();
    let Some(auteur) = auteur else {
        return HttpResponse::NotFound().json(json!({"status":"error","message":"Publication introuvable"}));
    };
    let deja_liké = conn
        .query_row(
            "SELECT 1 FROM publication_likes WHERE publication_id = ?1 AND uq_liker = ?2",
            rusqlite::params![&id, &liker],
            |_| Ok(()),
        )
        .is_ok();
    if !deja_liké {
        let _ = conn.execute(
            "INSERT INTO publication_likes (publication_id, uq_liker, uq_auteur, created_at) VALUES (?1,?2,?3,?4)",
            rusqlite::params![&id, &liker, normaliser(&auteur), chrono::Utc::now().timestamp()],
        );
    }
    let likes: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM publication_likes WHERE publication_id = ?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    HttpResponse::Ok().json(json!({"status":"success","likes":likes,"deja_like":deja_liké}))
}

/// DELETE /api/publications/{id}/like — retrait, compte renvoyé.
pub async fn publication_like_remove(req: HttpRequest, path: web::Path<String>) -> HttpResponse {
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let id = path.into_inner();
    let db = Database::shared("uq_platform_db.json");
    let liker = normaliser(&claims.sub);
    let conn = match db.connexion() {
        Ok(c) => c,
        Err(e) => return HttpResponse::InternalServerError().json(json!({"status":"error","message":e})),
    };
    let supprimes = conn
        .execute(
            "DELETE FROM publication_likes WHERE publication_id = ?1 AND uq_liker = ?2",
            rusqlite::params![&id, &liker],
        )
        .unwrap_or(0);
    let likes: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM publication_likes WHERE publication_id = ?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    HttpResponse::Ok().json(json!({"status":"success","likes":likes,"retire":supprimes > 0}))
}

/// POST /api/publications/{id}/view — anonyme possible, aucune vérif.
pub async fn publication_view_record(req: HttpRequest, path: web::Path<String>) -> HttpResponse {
    let id = path.into_inner();
    let db = Database::shared("uq_platform_db.json");
    let conn = match db.connexion() {
        Ok(c) => c,
        Err(e) => return HttpResponse::InternalServerError().json(json!({"status":"error","message":e})),
    };
    let auteur: Option<String> = conn
        .query_row(
            "SELECT json_extract(json, '$.author') FROM publications WHERE id = ?1",
            [&id],
            |r| r.get(0),
        )
        .ok();
    let Some(auteur) = auteur else {
        return HttpResponse::NotFound().json(json!({"status":"error","message":"Publication introuvable"}));
    };
    let anonyme = crate::auth_guard::exiger_auth(&req).ok().map(|c| normaliser(&c.sub));
    let _ = conn.execute(
        "INSERT INTO publication_views (publication_id, uq_viewer, uq_auteur, created_at) VALUES (?1,?2,?3,?4)",
        rusqlite::params![&id, anonyme, normaliser(&auteur), chrono::Utc::now().timestamp()],
    );
    let vues: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM publication_views WHERE publication_id = ?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    HttpResponse::Ok().json(json!({"status":"success","vues":vues}))
}

/// POST /api/follow/{uq} — suivre (pas soi-même), compte renvoyé.
pub async fn follow_add(req: HttpRequest, path: web::Path<String>) -> HttpResponse {
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let cible = match resoudre_identifiant(&path.into_inner()) {
        Some(uq) => uq,
        None => return HttpResponse::BadRequest().json(json!({"status":"error","message":"Identifiant de compte invalide"})),
    };
    let suiveur = normaliser(&claims.sub);
    if suiveur == cible {
        return HttpResponse::BadRequest().json(json!({"status":"error","message":"Impossible de se suivre soi-même"}));
    }
    let db = Database::shared("uq_platform_db.json");
    let conn = match db.connexion() {
        Ok(c) => c,
        Err(e) => return HttpResponse::InternalServerError().json(json!({"status":"error","message":e})),
    };
    if !compte_valide(&conn, &cible) {
        return HttpResponse::NotFound().json(json!({"status":"error","message":"Compte introuvable"}));
    }
    let insere = conn
        .execute(
            "INSERT OR IGNORE INTO followers (uq_follower, uq_followed, created_at) VALUES (?1,?2,?3)",
            rusqlite::params![&suiveur, &cible, chrono::Utc::now().timestamp()],
        )
        .unwrap_or(0)
        > 0;
    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM followers WHERE uq_followed = ?1", [&cible], |r| r.get(0))
        .unwrap_or(0);
    HttpResponse::Ok().json(json!({"status":"success","total_abonnes":total,"deja_suivi":!insere}))
}

/// DELETE /api/follow/{uq} — ne plus suivre.
pub async fn follow_remove(req: HttpRequest, path: web::Path<String>) -> HttpResponse {
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let cible = match resoudre_identifiant(&path.into_inner()) {
        Some(uq) => uq,
        None => return HttpResponse::BadRequest().json(json!({"status":"error","message":"Identifiant de compte invalide"})),
    };
    let suiveur = normaliser(&claims.sub);
    let db = Database::shared("uq_platform_db.json");
    let conn = match db.connexion() {
        Ok(c) => c,
        Err(e) => return HttpResponse::InternalServerError().json(json!({"status":"error","message":e})),
    };
    let _ = conn.execute(
        "DELETE FROM followers WHERE uq_follower = ?1 AND uq_followed = ?2",
        rusqlite::params![&suiveur, &cible],
    );
    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM followers WHERE uq_followed = ?1", [&cible], |r| r.get(0))
        .unwrap_or(0);
    HttpResponse::Ok().json(json!({"status":"success","total_abonnes":total}))
}

fn pagination(query: &web::Query<std::collections::HashMap<String, String>>) -> (i64, i64) {
    let limite = query
        .get("limit")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(20)
        .clamp(1, 100);
    let decalage = query
        .get("offset")
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|v| *v >= 0)
        .unwrap_or(0);
    (limite, decalage)
}

/// GET /api/follow/{uq}/followers?limit&offset — abonnés (pseudo only).
pub async fn follow_followers(
    path: web::Path<String>,
    query: web::Query<std::collections::HashMap<String, String>>,
) -> HttpResponse {
    let cible = match resoudre_identifiant(&path.into_inner()) {
        Some(uq) => uq,
        None => return HttpResponse::BadRequest().json(json!({"status":"error","message":"Identifiant de compte invalide"})),
    };
    let (limite, decalage) = pagination(&query);
    let db = Database::shared("uq_platform_db.json");
    let conn = match db.connexion() {
        Ok(c) => c,
        Err(e) => return HttpResponse::InternalServerError().json(json!({"status":"error","message":e})),
    };
    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM followers WHERE uq_followed = ?1", [&cible], |r| r.get(0))
        .unwrap_or(0);
    let uqs = lire_uqs(
        &conn,
        "SELECT uq_follower FROM followers WHERE uq_followed = ?1 ORDER BY created_at DESC LIMIT ?2 OFFSET ?3",
        &cible,
        limite,
        decalage,
    );
    HttpResponse::Ok().json(json!({
        "status":"success","total":total,"limit":limite,"offset":decalage,
        "abonnes": identites(&uqs),
    }))
}

/// GET /api/follow/{uq}/following?limit&offset — abonnements (pseudo only).
pub async fn follow_following(
    path: web::Path<String>,
    query: web::Query<std::collections::HashMap<String, String>>,
) -> HttpResponse {
    let cible = match resoudre_identifiant(&path.into_inner()) {
        Some(uq) => uq,
        None => return HttpResponse::BadRequest().json(json!({"status":"error","message":"Identifiant de compte invalide"})),
    };
    let (limite, decalage) = pagination(&query);
    let db = Database::shared("uq_platform_db.json");
    let conn = match db.connexion() {
        Ok(c) => c,
        Err(e) => return HttpResponse::InternalServerError().json(json!({"status":"error","message":e})),
    };
    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM followers WHERE uq_follower = ?1", [&cible], |r| r.get(0))
        .unwrap_or(0);
    let uqs = lire_uqs(
        &conn,
        "SELECT uq_followed FROM followers WHERE uq_follower = ?1 ORDER BY created_at DESC LIMIT ?2 OFFSET ?3",
        &cible,
        limite,
        decalage,
    );
    HttpResponse::Ok().json(json!({
        "status":"success","total":total,"limit":limite,"offset":decalage,
        "abonnements": identites(&uqs),
    }))
}

/// POST /api/profile/{uq}/note — {note: 1..5} par noteur, unique.
pub async fn profile_note(req: HttpRequest, path: web::Path<String>, body: web::Json<serde_json::Value>) -> HttpResponse {
    let claims = match crate::auth_guard::exiger_auth(&req) {
        Ok(c) => c,
        Err(r) => return r,
    };
    let cible = match resoudre_identifiant(&path.into_inner()) {
        Some(uq) => uq,
        None => return HttpResponse::BadRequest().json(json!({"status":"error","message":"Identifiant de compte invalide"})),
    };
    let noteur = normaliser(&claims.sub);
    if noteur == cible {
        return HttpResponse::BadRequest().json(json!({"status":"error","message":"Impossible de se noter soi-même"}));
    }
    let Some(note) = body.get("note").and_then(|v| v.as_i64()) else {
        return HttpResponse::BadRequest().json(json!({"status":"error","message":"note requise (1 à 5)"}));
    };
    if !(1..=5).contains(&note) {
        return HttpResponse::BadRequest().json(json!({"status":"error","message":"note hors bornes (1 à 5)"}));
    }
    let db = Database::shared("uq_platform_db.json");
    let conn = match db.connexion() {
        Ok(c) => c,
        Err(e) => return HttpResponse::InternalServerError().json(json!({"status":"error","message":e})),
    };
    if !compte_valide(&conn, &cible) {
        return HttpResponse::NotFound().json(json!({"status":"error","message":"Compte introuvable"}));
    }
    let _ = conn.execute(
        "INSERT INTO profile_ratings (uq_note, uq_noteur, note, created_at) VALUES (?1,?2,?3,?4)
         ON CONFLICT(uq_note, uq_noteur) DO UPDATE SET note=excluded.note, created_at=excluded.created_at",
        rusqlite::params![&cible, &noteur, note, chrono::Utc::now().timestamp()],
    );
    let (moyenne, avis) : (f64, i64) = conn
        .query_row(
            "SELECT COALESCE(AVG(note),0.0), COUNT(*) FROM profile_ratings WHERE uq_note = ?1",
            [&cible],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap_or((0.0, 0));
    HttpResponse::Ok().json(json!({
        "status":"success","note_moyenne":(moyenne*10.0).round()/10.0,"nb_avis":avis
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agregats_base() -> Agrégats {
        Agrégats {
            publications: 0,
            likes_recus: 0,
            vues_recues: 0,
            commentaires_recus: 0,
            commentaires_ecrits: 0,
            ventes: 0,
            abonne: 0,
            abonnements: 0,
            note_moyenne: 0.0,
            nb_avis: 0,
            anciennete_secs: 0,
            kyc: false,
            fraude: false,
            signalements_confirmes: 0,
            signalements_abusifs: 0,
        }
    }

    #[test]
    fn compte_neuf_n_a_que_les_cinq_points_d_anciennete() {
        let score = score(&agregats_base());
        assert_eq!(score["total"], 5);
        assert_eq!(score["composantes"]["anciennete"], 5);
        assert_eq!(score["certifie"], false);
        assert_eq!(score["badge"], "");
    }

    /// Profil sans sanction : 20 (anciennete) + 20 (KYC) + 20 (note 5/5 avec
    /// dix avis) + 10 + 5 + 5 + 10 + 5 + 5 = 100.
    fn profil_parfait() -> Agrégats {
        let mut a = agregats_base();
        a.anciennete_secs = 30 * 30 * 24 * 3600;
        a.kyc = true;
        a.note_moyenne = 5.0;
        a.nb_avis = 10;
        a.publications = 10;
        a.ventes = 5;
        a.commentaires_ecrits = 5;
        a.abonne = 1_000;
        a.likes_recus = 500;
        a.vues_recues = 5_000;
        a
    }

    #[test]
    fn un_signalement_confirme_retire_dix_points() {
        assert_eq!(score(&profil_parfait())["total"], 100);
        let mut a = profil_parfait();
        a.signalements_confirmes = 1;
        let note = score(&a);
        assert_eq!(note["total"], 90);
        assert_eq!(note["sanction"]["signalements_confirmes"], 10);
        assert_eq!(note["certifie"], false, "un compte sanctionne n'est plus certifie");
    }

    #[test]
    fn la_sanction_ne_passe_jamais_sous_zero() {
        let mut a = agregats_base();
        a.signalements_confirmes = 99;
        let note = score(&a);
        assert_eq!(note["sanction"]["total"], PLAFOND_SIGNALEMENTS as i64);
        assert_eq!(note["total"], 0, "cinq points de score face a trente de sanction");
        assert_eq!(note["badge"], "");
    }

    #[test]
    fn trois_signalements_abusifs_sanctionnent_leur_emetteur() {
        let mut a = profil_parfait();
        a.signalements_abusifs = 2;
        assert_eq!(score(&a)["sanction"]["signalements_abusifs"], 0);
        a.signalements_abusifs = 3;
        assert_eq!(
            score(&a)["sanction"]["signalements_abusifs"],
            (3 * POINTS_PAR_ABUS) as i64
        );
        a.signalements_abusifs = 99;
        assert_eq!(score(&a)["sanction"]["signalements_abusifs"], PLAFOND_ABUS as i64);
    }

    #[test]
    fn anciennete_par_paliers() {
        let mut a = agregats_base();
        for (secs, attendu) in [
            (30 * 24 * 3600, 5u32),
            (7 * 30 * 24 * 3600, 10),
            (13 * 30 * 24 * 3600, 15),
            (30 * 30 * 24 * 3600, 20),
        ] {
            a.anciennete_secs = secs;
            assert_eq!(score(&a)["composantes"]["anciennete"], attendu, "{secs} secondes");
        }
    }

    #[test]
    fn reputation_veut_dix_avis_au_moins() {
        let mut a = agregats_base();
        a.note_moyenne = 5.0;
        a.nb_avis = 9;
        assert_eq!(score(&a)["composantes"]["reputation"], 0);
        a.nb_avis = 10;
        assert_eq!(score(&a)["composantes"]["reputation"], 20);
        a.note_moyenne = 3.0;
        assert_eq!(score(&a)["composantes"]["reputation"], 12);
    }

    #[test]
    fn activite_et_communaute_plafonnent() {
        let mut a = agregats_base();
        a.publications = 50;
        a.ventes = 50;
        a.commentaires_ecrits = 50;
        a.abonne = 100_000;
        a.likes_recus = 100_000;
        a.vues_recues = 100_000;
        let score = score(&a);
        assert_eq!(score["composantes"]["publications"], 10);
        assert_eq!(score["composantes"]["ventes"], 5);
        assert_eq!(score["composantes"]["reponses"], 5);
        assert_eq!(score["composantes"]["abonnes"], 10);
        assert_eq!(score["composantes"]["likes"], 5);
        assert_eq!(score["composantes"]["vues"], 5);
        // activité 20 + communauté 20 + les 5 pts d'ancienneté de départ
        assert_eq!(score["total"], 45);
    }

    #[test]
    fn cent_points_donnent_le_badge() {
        let mut a = agregats_base();
        a.anciennete_secs = 30 * 30 * 24 * 3600; // > 2 ans
        a.kyc = true;
        a.note_moyenne = 5.0;
        a.nb_avis = 127;
        a.publications = 45;
        a.ventes = 5;
        a.commentaires_ecrits = 5;
        a.abonne = 10_234;
        a.likes_recus = 1_234;
        a.vues_recues = 12_345;
        let score = score(&a);
        assert_eq!(score["total"], 100);
        assert_eq!(score["certifie"], true);
        assert_eq!(score["badge"], "Certifié UQ");
    }

    #[test]
    fn fraude_annule_la_certification_meme_a_cent_points() {
        let mut a = agregats_base();
        a.anciennete_secs = 30 * 30 * 24 * 3600;
        a.kyc = true;
        a.note_moyenne = 5.0;
        a.nb_avis = 127;
        a.publications = 45;
        a.ventes = 5;
        a.commentaires_ecrits = 5;
        a.abonne = 10_234;
        a.likes_recus = 1_234;
        a.vues_recues = 12_345;
        a.fraude = true;
        let resultat = score(&a);
        assert_eq!(resultat["total"], 100);
        assert_eq!(resultat["certifie"], false);

        //KYC manquant : même sanction
        let mut b = a.clone();
        b.fraude = false;
        b.kyc = false;
        assert_eq!(score(&b)["certifie"], false);
    }

    #[test]
    fn pseudo_resolu_vers_un_uq_valide() {
        assert_eq!(resoudre_identifiant("uq-732837813").as_deref(), Some("UQ-732837813"));
        assert_eq!(resoudre_identifiant("  "), None);
        assert_eq!(resoudre_identifiant("../data/users/x"), None);
    }

    #[test]
    fn registre_reperifie_par_uq_normalise() {
        let carte = carte_registre(vec![
            ("uq-732688531".to_string(), 1_700_000_000),
            ("UQ-00000001".to_string(), -5),
        ]);
        assert_eq!(carte.get("UQ-732688531").copied(), Some(1_700_000_000));
        // Date négative jamais utilisée comme ancienneté.
        assert_eq!(carte.get("UQ-00000001").copied(), Some(0));
        assert_eq!(carte.get("uq-732688531"), None);
    }
}
