// ============================================================
// PREUVE DE PAIEMENT P2P — analyse d'une capture, verdict, apprentissage
// ============================================================
//
// L'acheteur (et le vendeur) recopient sur la feuille les valeurs visibles de
// leur capture — montant, devise, numéro, nom. Le moteur croise ces valeurs
// avec ce que la commande impose, puis cherche les traces d'une image
// fabriquée : générateur IA nommé dans les métadonnées, outil de retouche,
// export sans aucune trace d'appareil, horodatage antérieur à la commande,
// image sans matière textuelle.
//
// Deux règles commandent tout le reste :
//  * une capture douteuse n'accuse personne. Sans métadonnée, un signal reste
//    faible ; seul un générateur nommément déclaré (confiance >= 0,9) est un
//    veto, parce que le fichier l'écrit lui-même ;
//  * l'humain garde la main. Un verdict `Rejet` bloquant est toujours
//    surmontable par une dérogation assumée, tracée et notifiée en clair à
//    l'autre partie — une vraie capture mal lisible ne doit pas immobiliser
//    de l'argent.
//
// Le moteur apprend : chaque preuve est stockée avec les forces de ses
// signaux, chaque issue (commande libérée = preuve tenait, litige = preuve
// mauvaise) est écrite sur la ligne, et `recalibrer` ajuste les pondérations
// à la main sur ces issues.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

/// Pondération par défaut de chaque signal. La valeur est l'emprise du signal
/// dans le score, pas sa gravité : `generateur_ia` a un veto de toute façon.
pub const POIDS_DEFAUT: &[(&str, f64)] = &[
    ("montant", 0.40),
    ("generateur_ia", 0.45),
    ("numero", 0.20),
    ("nom", 0.15),
    ("devise", 0.10),
    ("retouche", 0.14),
    ("export_editeur", 0.10),
    ("horodatage", 0.12),
    ("sans_trace", 0.06),
    ("image_legere", 0.08),
    ("texte_absent", 0.16),
];

/// En dessous de ce nombre de cas annotés, les pondérations apprises ne
/// valent rien : on garde les priori.
pub const ECHANTILLON_MIN: i64 = 8;
/// Verdict `Valide` à partir de là, `Rejet` en dessous de `SEUIL_REJET`.
pub const SEUIL_VALIDE: f64 = 0.60;
pub const SEUIL_REJET: f64 = 0.25;
const PAS_APPRENTISSAGE: f64 = 0.25;
const TOLERANCE_MONTANT: f64 = 0.01;
pub const IMAGE_MAX_CARACTERES: usize = 1_200_000;

#[derive(Debug, Clone)]
pub struct Signal {
    pub nom: &'static str,
    /// -1 = rassurant, 0 = muet, +1 = accusateur.
    pub force: f64,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Valide,
    Doute,
    Rejet,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Valide => "valide",
            Self::Doute => "doute",
            Self::Rejet => "rejet",
        }
    }
}

/// Issue du test de connaissance du nom : la contrepartie recopie le nom que
/// son opérateur affiche après le paiement, nom que la plateforme ne lui a
/// jamais montré. Comparé au nom enregistré, le résultat se range ici.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NomRecu {
    /// Le nom recopié couvre le nom enregistré : l'écran a été regardé.
    Conforme,
    /// Recouvrement partiel : reçu mal lisible ou nom enregistré incomplet.
    Partiel,
    /// Faute de comparaison possible : pas de nom enregistré, ou saisie vide.
    Inverifiable,
    /// Le nom recopié désigne quelqu'un d'autre : la preuve est écartée.
    Divergent,
}

impl NomRecu {
    /// Code stocké dans la base. Zéro reste la valeur par défaut des lignes
    /// plus anciennes, donc l'invérifiable.
    pub fn as_i64(self) -> i64 {
        match self {
            Self::Conforme => 2,
            Self::Partiel => 1,
            Self::Inverifiable => 0,
            Self::Divergent => -1,
        }
    }

    pub fn de(valeur: i64) -> Self {
        match valeur {
            2 => Self::Conforme,
            1 => Self::Partiel,
            -1 => Self::Divergent,
            _ => Self::Inverifiable,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Conforme => "conforme",
            Self::Partiel => "partiel",
            Self::Inverifiable => "inverifiable",
            Self::Divergent => "divergent",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Analyse {
    pub verdict: Verdict,
    /// Conformité calculée, 0 = tout accuse, 1 = tout rassure.
    pub conformite: f64,
    /// Un générateur d'IA est nommé dans le fichier : verdict forcé.
    pub veto: bool,
    /// Verdict du test de connaissance du nom du bénéficiaire.
    pub nom_ok: NomRecu,
    pub signaux: Vec<Signal>,
}

impl Analyse {
    /// Les raisons lisibles, dans l'ordre de gravité décroissante.
    pub fn raisons(&self) -> Vec<String> {
        let mut marques: Vec<(f64, String)> = self
            .signaux
            .iter()
            .filter(|signal| signal.force.abs() >= 0.2)
            .map(|signal| (signal.force.abs(), signal.detail.clone()))
            .collect();
        marques.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        marques.into_iter().map(|(_, raison)| raison).collect()
    }

    pub fn as_json(&self) -> Value {
        json!({
            "verdict": self.verdict.as_str(),
            "conformite": ((self.conformite * 100.0).round() / 100.0),
            "veto": self.veto,
            "nom_verifie": self.nom_ok.as_str(),
            "raisons": self.raisons(),
            "signaux": self.signaux.iter().map(|signal| json!({
                "nom": signal.nom,
                "force": (signal.force * 100.0).round() / 100.0,
                "detail": signal.detail,
            })).collect::<Vec<_>>(),
        })
    }
}

/// Ce que la commande exige de voir sur la capture.
#[derive(Debug, Clone)]
pub struct Attendu {
    pub montant: f64,
    pub devise: String,
    pub numero: String,
    pub nom: String,
    /// Le nom attendu est le nom officiel enregistré du compte destinataire.
    /// S'il ne s'agit que de l'affichage public, la comparaison ne prouve rien
    /// et le test de connaissance est déclaré invérifiable.
    pub nom_officiel: bool,
    /// Époque de création de la commande, en secondes.
    pub cree_le: i64,
}

/// Ce que l'humain a recopié sur sa capture.
#[derive(Debug, Clone)]
pub struct Saisie {
    pub montant: f64,
    pub devise: String,
    pub numero: String,
    pub nom: String,
}

fn maintenant_secondes() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ============================ Poids appris ============================

pub fn poids_par_defaut() -> HashMap<String, f64> {
    POIDS_DEFAUT
        .iter()
        .map(|(nom, poids)| (nom.to_string(), *poids))
        .collect()
}

/// Pondérations en cours d'utilisation : les priori tant que les cas annotés
/// sont trop rares pour dire quoi que ce soit.
pub fn charger_poids(conn: &Connection) -> HashMap<String, f64> {
    let lu = conn.query_row(
        "SELECT poids, echantillons FROM p2p_proof_model WHERE id = 1",
        [],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
    );
    match lu {
        Ok((texte, echantillons)) if echantillons >= ECHANTILLON_MIN => {
            let valeurs: Value = serde_json::from_str(&texte).unwrap_or(Value::Null);
            let mut poids = poids_par_defaut();
            if let Some(objet) = valeurs.as_object() {
                for (nom, valeur) in objet {
                    if let Some(n) = valeur.as_f64() {
                        poids.insert(nom.clone(), n);
                    }
                }
            }
            poids
        }
        _ => poids_par_defaut(),
    }
}

/// Score de conformité à partir des seules forces des signaux.
pub fn conformite_des(signaux: &[Signal], poids: &HashMap<String, f64>) -> f64 {
    let mut suspect = 0.0f64;
    let mut rassurant = 0.0f64;
    for signal in signaux {
        let emprise = poids.get(signal.nom).copied().unwrap_or(0.10);
        if signal.force > 0.0 {
            suspect += emprise * signal.force;
        } else if signal.force < 0.0 {
            rassurant += emprise * (-signal.force);
        }
    }
    // Le lissage evite qu'une preuve parfaite affiche 1,00 : le moteur n'a
    // jamais lu le texte de l'image, il ne peut pas etre certain.
    rassurant / (rassurant + suspect + 0.35)
}

// ============================ Signaux ============================

/// Chiffres seuls, sans préfixe international ni séparateurs.
fn chiffres(texte: &str) -> String {
    texte.chars().filter(|c| c.is_ascii_digit()).collect()
}

fn normaliser_nom(texte: &str) -> Vec<String> {
    const ACCENTUES: &[(char, &str)] = &[
        ('à', "a"),
        ('á', "a"),
        ('â', "a"),
        ('ä', "a"),
        ('ã', "a"),
        ('å', "a"),
        ('è', "e"),
        ('é', "e"),
        ('ê', "e"),
        ('ë', "e"),
        ('ì', "i"),
        ('í', "i"),
        ('î', "i"),
        ('ï', "i"),
        ('ò', "o"),
        ('ó', "o"),
        ('ô', "o"),
        ('ö', "o"),
        ('õ', "o"),
        ('ù', "u"),
        ('ú', "u"),
        ('û', "u"),
        ('ü', "u"),
        ('ñ', "n"),
        ('ç', "c"),
    ];
    texte
        .to_lowercase()
        .chars()
        .map(|c| {
            ACCENTUES
                .iter()
                .find(|(base, _)| *base == c)
                .map(|(_, rendu)| rendu.to_string())
                .unwrap_or_else(|| c.to_string())
        })
        .collect::<String>()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|mot| mot.len() > 1)
        .map(|mot| mot.to_string())
        .collect()
}

/// Deux numéros de mobile money ne s'écrivent pas pareil d'un bord à l'autre
/// (09… d'un côté, +243… de l'autre). On compare le corps du numéro : zéro
/// de préfixe retiré, neuf derniers chiffres.
fn noyau_numero(texte: &str) -> String {
    let mut digits = chiffres(texte);
    while digits.starts_with('0') && digits.len() > 6 {
        digits.remove(0);
    }
    if digits.len() <= 9 {
        return digits;
    }
    digits[digits.len() - 9..].to_string()
}

fn numero_correspond(saisi: &str, attendu: &str) -> f64 {
    if chiffres(saisi).len() < 5 || chiffres(attendu).len() < 5 {
        return 0.0;
    }
    let a = noyau_numero(saisi);
    let b = noyau_numero(attendu);
    if a == b {
        return -1.0;
    }
    if a.len() >= 6 && b.len() >= 6 && (a.ends_with(&b) || b.ends_with(&a)) {
        return -0.6;
    }
    0.9
}

/// Les outils de retouche et d'export courants : une trace ici n'est pas une
/// accusation d'IA, mais un reçu payé « recomposé » laisse des traces.
const OUTILS_DE_RETOUCHE: &[&str] = &[
    "photoshop",
    "gimp",
    "lightroom",
    "snapseed",
    "picsart",
    "canva",
    "pixlr",
    "luminar",
    "affinity",
    "paint.net",
    "inpaint",
    "crop",
    "recadr",
];

fn horodatage_de(meta: &crate::image_meta::MetadonneesImage) -> Option<i64> {
    let brut = meta
        .valeur("DateTimeOriginal")
        .or_else(|| meta.valeur("ModifyDate"))
        .or_else(|| meta.valeur("DateTime"))?;
    let chiffres = chiffres(brut);
    if chiffres.len() < 14 {
        return None;
    }
    let annee: i64 = chiffres[0..4].parse().ok()?;
    let mois: i64 = chiffres[4..6].parse().ok()?;
    let jour: i64 = chiffres[6..8].parse().ok()?;
    let heure: i64 = chiffres[8..10].parse().ok()?;
    let minute: i64 = chiffres[10..12].parse().ok()?;
    // Approximation suffisante : on compare des jours, pas des secondes.
    Some(
        (annee - 1970) * 31_557_600
            + (mois - 1) * 2_629_800
            + (jour - 1) * 86_400
            + heure * 3_600
            + minute * 60,
    )
}

/// Contraste moyen d'une image réduite : un reçu ou une capture de mobile
/// money est fait de texte, donc de transitions fréquentes. Une image
/// recomposée à la main ou un fond unie n'en a presque pas.
fn contraste_moyen(octets: &[u8]) -> Option<f64> {
    let image = image::load_from_memory(octets).ok()?;
    let gris = image.to_luma8();
    let petit = image::imageops::resize(&gris, 120, 120, image::imageops::FilterType::Triangle);
    let pixels = petit.as_raw();
    let largeur = petit.width() as usize;
    let mut transitions = 0.0f64;
    let mut comparisons = 0usize;
    for ligne in 0..(petit.height() as usize) {
        for colonne in 1..largeur {
            let index = ligne * largeur + colonne;
            transitions += (pixels[index] as f64 - pixels[index - 1] as f64).abs();
            comparisons += 1;
        }
    }
    if comparisons == 0 {
        return None;
    }
    Some(transitions / comparisons as f64)
}

/// Analyse complète d'une preuve. `octets` est le fichier décodé, jamais une
/// donnée interprétée à la légère.
pub fn analyser(
    octets: &[u8],
    saisi: &Saisie,
    attendu: &Attendu,
    poids: &HashMap<String, f64>,
) -> Analyse {
    let mut signaux: Vec<Signal> = Vec::new();

    // --- ce que la capture doit dire -------------------------------------
    let ecart = if attendu.montant > 0.0 {
        (saisi.montant - attendu.montant).abs() / attendu.montant
    } else {
        0.0
    };
    let force_montant = if saisi.montant <= 0.0 || attendu.montant <= 0.0 {
        0.0
    } else if ecart <= TOLERANCE_MONTANT {
        -1.0
    } else if ecart <= 0.05 {
        0.0
    } else if ecart <= 0.25 {
        0.6
    } else {
        1.0
    };
    signaux.push(Signal {
        nom: "montant",
        force: force_montant,
        detail: if force_montant < 0.0 {
            format!(
                "montant conforme : {:.2} {} attendus, {:.2} déclarés",
                attendu.montant, attendu.devise, saisi.montant
            )
        } else {
            format!(
                "montant écarté de {:.0} % : la commande porte {:.2} {}, la capture annonce {:.2}",
                ecart * 100.0,
                attendu.montant,
                attendu.devise,
                saisi.montant
            )
        },
    });

    let force_devise = if saisi.devise.is_empty() || attendu.devise.is_empty() {
        0.0
    } else if saisi.devise.eq_ignore_ascii_case(&attendu.devise) {
        -1.0
    } else {
        0.7
    };
    signaux.push(Signal {
        nom: "devise",
        force: force_devise,
        detail: if force_devise < 0.0 {
            format!("devise conforme : {}", attendu.devise)
        } else {
            format!(
                "devise différente : la commande est en {}, la capture annonce {}",
                attendu.devise, saisi.devise
            )
        },
    });

    let force_numero = numero_correspond(&saisi.numero, &attendu.numero);
    signaux.push(Signal {
        nom: "numero",
        force: force_numero,
        detail: if force_numero > 0.2 {
            "le numéro de la capture ne correspond pas à celui de la commande".to_string()
        } else if force_numero < 0.0 {
            "numéro conforme à celui de la commande".to_string()
        } else {
            "numéro non vérifiable (absent de la commande ou de la saisie)".to_string()
        },
    });

    let attendu_mots = normaliser_nom(&attendu.nom);
    let saisi_mots = normaliser_nom(&saisi.nom);
    // Le recouvrement note le test de connaissance. Un nom masqué par
    // l'opérateur (« A**** O**** ») ne laisse aucun mot comparable : le signal
    // reste muet plutôt que d'accuser une capture pourtant vraie.
    let recouvrement = if attendu_mots.is_empty() || saisi_mots.is_empty() {
        -1.0
    } else {
        attendu_mots
            .iter()
            .filter(|mot| saisi_mots.contains(mot))
            .count() as f64
            / attendu_mots.len() as f64
    };
    let nom_ok = if !attendu.nom_officiel || recouvrement < 0.0 {
        NomRecu::Inverifiable
    } else if recouvrement >= 0.6 {
        NomRecu::Conforme
    } else if recouvrement >= 0.3 {
        NomRecu::Partiel
    } else {
        NomRecu::Divergent
    };
    let force_nom = match nom_ok {
        NomRecu::Conforme => -1.0,
        NomRecu::Partiel => -0.2,
        NomRecu::Inverifiable => 0.0,
        NomRecu::Divergent => 0.8,
    };
    signaux.push(Signal {
        nom: "nom",
        force: force_nom,
        detail: match nom_ok {
            NomRecu::Conforme => {
                "le nom recopié est le nom enregistré du compte destinataire".to_string()
            }
            NomRecu::Partiel => {
                "le nom recopié ne recouvre qu'une partie du nom enregistré".to_string()
            }
            NomRecu::Inverifiable => {
                if attendu.nom_officiel {
                    "nom invérifiable : rien de comparable entre la saisie et le reçu".to_string()
                } else {
                    "nom invérifiable : la contrepartie n'a pas enregistré son nom officiel"
                        .to_string()
                }
            }
            NomRecu::Divergent => {
                "le nom recopié ne désigne pas le compte destinataire".to_string()
            }
        },
    });

    // --- ce que le fichier dit de lui-même ---------------------------------
    let meta = crate::image_meta::extraire(octets);
    let detection = match &meta {
        Some(meta) => crate::ai_detector::detect_ai_metadonnees(meta),
        None => crate::ai_detector::detect_ai_octets(octets),
    };
    let force_ia = if detection.is_ai_generated {
        detection.confidence as f64
    } else {
        0.0
    };
    let veto = force_ia >= 0.9;
    signaux.push(Signal {
        nom: "generateur_ia",
        force: force_ia,
        detail: if force_ia > 0.0 {
            format!(
                "le fichier se déclare fabriqué par un générateur ({})",
                detection.signals.join(" · ")
            )
        } else {
            "aucun générateur d'images déclaré dans le fichier".to_string()
        },
    });

    let retouche = meta.as_ref().map_or(String::new(), |meta| {
        meta.champs_fabrication()
            .iter()
            .filter(|champ| {
                OUTILS_DE_RETOUCHE
                    .iter()
                    .any(|outil| champ.valeur.to_lowercase().contains(outil))
            })
            .map(|champ| format!("{} = {}", champ.nom, champ.valeur))
            .collect::<Vec<_>>()
            .join(" · ")
    });
    signaux.push(Signal {
        nom: "retouche",
        force: if retouche.is_empty() { 0.0 } else { 1.0 },
        detail: if retouche.is_empty() {
            "aucune trace de logiciel de retouche".to_string()
        } else {
            format!("logiciel de retouche déclaré : {}", retouche)
        },
    });

    if let Some(meta) = &meta {
        let sans_trace = !meta.exif_present && !meta.xmp_present && meta.champs.is_empty();
        signaux.push(Signal {
            nom: "sans_trace",
            force: if sans_trace { 1.0 } else { -0.4 },
            detail: if sans_trace {
                "aucune métadonnée : image recadrée ou ré-exportée, ce qui reste une information et non une preuve"
                    .to_string()
            } else {
                "métadonnées présentes dans le fichier".to_string()
            },
        });

        let export = meta.format == "jpeg" && meta.jfif_present && !meta.exif_present;
        signaux.push(Signal {
            nom: "export_editeur",
            force: if export { 1.0 } else { 0.0 },
            detail: if export {
                "JPEG signé d'une logithèque d'export (JFIF) sans EXIF d'appareil ou de téléphone"
                    .to_string()
            } else {
                "pas de signature d'export d'éditeur".to_string()
            },
        });

        let horodatage = horodatage_de(meta);
        let force_horodatage = match horodatage {
            Some(instant) if instant < attendu.cree_le - 172_800 => 1.0,
            Some(instant) if instant > maintenant_secondes() + 7_200 => 0.8,
            Some(_) => -0.6,
            None => 0.0,
        };
        signaux.push(Signal {
            nom: "horodatage",
            force: force_horodatage,
            detail: match (horodatage, force_horodatage) {
                (Some(_), f) if f > 0.9 => {
                    "l'horodatage du fichier est antérieur à l'ouverture de la commande".to_string()
                }
                (Some(_), f) if f > 0.5 => "l'horodatage du fichier est dans le futur".to_string(),
                (Some(_), _) => "horodatage compatible avec la commande".to_string(),
                (None, _) => "pas d'horodatage lisible dans le fichier".to_string(),
            },
        });

        let legere = meta.octets < 12_000
            || meta
                .largeur
                .zip(meta.hauteur)
                .map_or(false, |(l, h)| l < 320 || h < 320);
        signaux.push(Signal {
            nom: "image_legere",
            force: if legere { 1.0 } else { -0.3 },
            detail: if legere {
                "image trop petite pour porter un reçu lisible".to_string()
            } else {
                format!(
                    "dimensions exploitables ({}×{}, {})",
                    meta.largeur.unwrap_or(0),
                    meta.hauteur.unwrap_or(0),
                    meta.octets
                )
            },
        });
    } else {
        signaux.push(Signal {
            nom: "sans_trace",
            force: 0.6,
            detail: "structure du fichier illisible : aucune métadonnée n'a pu être vérifiée"
                .to_string(),
        });
    }

    match contraste_moyen(octets) {
        Some(contraste) if contraste < 2.0 => signaux.push(Signal {
            nom: "texte_absent",
            force: 1.0,
            detail: format!(
                "l'image ne contient presque aucun détail (contraste moyen {:.1}) : aucun texte de reçu reconnaissable",
                contraste
            ),
        }),
        Some(_) => signaux.push(Signal {
            nom: "texte_absent",
            force: -0.5,
            detail: "l'image porte du détail, comme une capture avec du texte".to_string(),
        }),
        None => signaux.push(Signal {
            nom: "texte_absent",
            force: 0.0,
            detail: "pixels non analysables (format décalé)".to_string(),
        }),
    }

    let conformite = conformite_des(&signaux, poids);
    let verdict = if veto || nom_ok == NomRecu::Divergent {
        Verdict::Rejet
    } else if conformite >= SEUIL_VALIDE {
        Verdict::Valide
    } else if conformite < SEUIL_REJET {
        Verdict::Rejet
    } else {
        Verdict::Doute
    };

    Analyse {
        verdict,
        conformite,
        veto,
        nom_ok,
        signaux,
    }
}

// ============================ Stockage ============================

/// Tables de la branche preuve : les preuves elles-mêmes, et le modèle de
/// pondération réévalué à chaque issue nouvelle.
pub fn assurer_tables(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS p2p_proofs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            order_id INTEGER NOT NULL,
            role TEXT NOT NULL,
            uq TEXT NOT NULL,
            montant_declare REAL NOT NULL DEFAULT 0,
            devise_declare TEXT NOT NULL DEFAULT '',
            numero_declare TEXT NOT NULL DEFAULT '',
            nom_declare TEXT NOT NULL DEFAULT '',
            image TEXT NOT NULL DEFAULT '',
            format TEXT NOT NULL DEFAULT '',
            conformite REAL NOT NULL DEFAULT 0,
            verdict TEXT NOT NULL DEFAULT '',
            veto INTEGER NOT NULL DEFAULT 0,
            signaux TEXT NOT NULL DEFAULT '[]',
            reasons TEXT NOT NULL DEFAULT '[]',
            derogue INTEGER NOT NULL DEFAULT 0,
            nom_ok INTEGER NOT NULL DEFAULT 0,
            issue TEXT NOT NULL DEFAULT '',
            issue_at INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_p2p_proofs_order ON p2p_proofs(order_id, id);
        CREATE TABLE IF NOT EXISTS p2p_proof_model (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            poids TEXT NOT NULL DEFAULT '{}',
            echantillons INTEGER NOT NULL DEFAULT 0,
            updated_at INTEGER NOT NULL DEFAULT 0
        );
        INSERT OR IGNORE INTO p2p_proof_model (id, poids, echantillons, updated_at)
        VALUES (1, '{}', 0, 0);
        CREATE TABLE IF NOT EXISTS p2p_preuve_tentatives (
            order_id INTEGER NOT NULL,
            uq TEXT NOT NULL,
            stale INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (order_id, uq)
        );",
    )
    .map_err(|e| format!("Preuves P2P : migration impossible ({})", e))?;
    ajouter_colonne(conn, "p2p_proofs", "derogue", "INTEGER NOT NULL DEFAULT 0")?;
    ajouter_colonne(conn, "p2p_proofs", "nom_ok", "INTEGER NOT NULL DEFAULT 0")?;
    ajouter_colonne(
        conn,
        "p2p_proofs",
        "image_sha256",
        "TEXT NOT NULL DEFAULT ''",
    )
}

/// Les tables se créent une fois ; une colonne ajoutée depuis doit quand même
/// apparaître sur un fichier plus ancien. `ALTER TABLE` ne sait pas tester, on
/// lit le schéma d'abord.
fn ajouter_colonne(
    conn: &Connection,
    table: &str,
    colonne: &str,
    type_: &str,
) -> Result<(), String> {
    let existe: bool = conn
        .query_row(
            &format!(
                "SELECT COUNT(*) FROM pragma_table_info('{}') WHERE name = ?1",
                table
            ),
            params![colonne],
            |row| row.get::<_, i64>(0),
        )
        .map(|n| n > 0)
        .unwrap_or(false);
    if existe {
        return Ok(());
    }
    conn.execute(
        &format!("ALTER TABLE {} ADD COLUMN {} {}", table, colonne, type_),
        [],
    )
    .map(|_| ())
    .map_err(|e| format!("Preuves P2P : colonne {} impossible ({})", colonne, e))
}

/// Tentatives de capture PÉRIMÉE (> 1 min) pour une commande : max 3.
/// Une capture ancienne rejouée ne doit pas boucler indéfiniment.
pub fn tentatives_stale(conn: &Connection, commande: i64, uq: &str) -> i64 {
    conn.query_row(
        "SELECT stale FROM p2p_preuve_tentatives WHERE order_id = ?1 AND UPPER(uq) = UPPER(?2)",
        params![commande, uq],
        |row| row.get(0),
    )
    .unwrap_or(0)
}

pub fn incr_tentative_stale(conn: &Connection, commande: i64, uq: &str) {
    let _ = conn.execute(
        "INSERT INTO p2p_preuve_tentatives (order_id, uq, stale) VALUES (?1, ?2, 1)
         ON CONFLICT(order_id, uq) DO UPDATE SET stale = stale + 1",
        params![commande, uq],
    );
}

/// Une preuve ne dépasse jamais ce plafond : le même garde-fou que les
/// photos du chat s'applique ici aussi.
pub fn decoder_image(data_uri: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    let texte = data_uri.trim();
    if texte.len() > IMAGE_MAX_CARACTERES {
        return Err("Image trop lourde pour une preuve (650 Ko maximum)".to_string());
    }
    let encodage = texte
        .find(",")
        .map(|index| &texte[index + 1..])
        .unwrap_or(texte);
    if !texte.starts_with("data:image/") {
        return Err("Format d'image invalide : envoie une capture en data:image".to_string());
    }
    base64::engine::general_purpose::STANDARD
        .decode(encodage.trim())
        .map_err(|e| format!("Capture indécodable ({})", e))
}

pub struct PreuveEnregistree {
    pub identifiant: i64,
    pub analyse: Analyse,
}

pub fn inserer_preuve(
    conn: &Connection,
    commande: i64,
    role: &str,
    uq: &str,
    saisi: &Saisie,
    octets: &[u8],
    analyse: &Analyse,
) -> Result<PreuveEnregistree, String> {
    let horodatage = maintenant_secondes();
    let signaux = serde_json::to_string(
        &analyse
            .signaux
            .iter()
            .map(|signal| json!({ "nom": signal.nom, "force": signal.force, "detail": signal.detail }))
            .collect::<Vec<_>>(),
    )
    .unwrap_or("[]".to_string());
    let raisons = serde_json::to_string(&analyse.raisons()).unwrap_or("[]".to_string());
    let b64 = {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(octets)
    };
    let image_sha256 = {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(octets))
    };
    let deja_utilisee: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM p2p_proofs WHERE image_sha256 = ?1",
            params![image_sha256],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if deja_utilisee > 0 {
        return Err(
            "Cette capture a déjà été utilisée comme preuve sur une autre transaction.".to_string(),
        );
    }
    let data_uri = format!(
        "data:image/{};base64,{}",
        crate::image_meta::format_de(octets),
        b64
    );
    conn.execute(
        "INSERT INTO p2p_proofs
            (order_id, role, uq, montant_declare, devise_declare, numero_declare, nom_declare,
             image, format, conformite, verdict, veto, signaux, reasons, created_at, nom_ok, image_sha256)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)",
        params![
            commande,
            role,
            uq,
            saisi.montant,
            saisi.devise,
            saisi.numero,
            saisi.nom,
            data_uri,
            crate::image_meta::format_de(octets),
            analyse.conformite,
            analyse.verdict.as_str(),
            if analyse.veto { 1 } else { 0 },
            signaux,
            raisons,
            horodatage,
            analyse.nom_ok.as_i64(),
            image_sha256,
        ],
    )
    .map_err(|e| format!("Preuve impossible à enregistrer ({})", e))?;
    Ok(PreuveEnregistree {
        identifiant: conn.last_insert_rowid(),
        analyse: analyse.clone(),
    })
}

pub fn preuve_de(conn: &Connection, identifiant: i64) -> Result<Option<Value>, String> {
    conn.query_row(
        "SELECT id, order_id, role, uq, verdict, conformite, veto, reasons, created_at
         FROM p2p_proofs WHERE id = ?1",
        params![identifiant],
        |row| {
            Ok(json!({
                "id": row.get::<_, i64>(0)?,
                "order_id": row.get::<_, i64>(1)?,
                "role": row.get::<_, String>(2)?,
                "uq": row.get::<_, String>(3)?,
                "verdict": row.get::<_, String>(4)?,
                "conformite": row.get::<_, f64>(5)?,
                "veto": row.get::<_, i64>(6)? == 1,
                "raisons": serde_json::from_str::<Value>(&row.get::<_, String>(7)?).unwrap_or(json!([])),
                "created_at": row.get::<_, i64>(8)?,
            }))
        },
    )
    .optional()
    .map_err(|e| format!("Lecture de preuve impossible ({})", e))
}

/// Les preuves d'une commande, dans l'ordre, sans les images : la liste sert à
/// l'affichage. Une image se relit à l'unité, pour la partie concernée.
pub fn lister_preuves(conn: &Connection, commande: i64) -> Result<Vec<Value>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, role, uq, montant_declare, devise_declare, numero_declare, nom_declare,
                    verdict, conformite, veto, reasons, signaux, issue, derogue, created_at,
                    nom_ok
             FROM p2p_proofs WHERE order_id = ?1 ORDER BY id",
        )
        .map_err(|e| format!("Lecture des preuves impossible ({})", e))?;
    let lignes = stmt
        .query_map(params![commande], |row| {
            Ok(json!({
                "id": row.get::<_, i64>(0)?,
                "role": row.get::<_, String>(1)?,
                "uq": row.get::<_, String>(2)?,
                "montant_declare": row.get::<_, f64>(3)?,
                "devise_declare": row.get::<_, String>(4)?,
                "numero_declare": row.get::<_, String>(5)?,
                "nom_declare": row.get::<_, String>(6)?,
                "verdict": row.get::<_, String>(7)?,
                "conformite": row.get::<_, f64>(8)?,
                "veto": row.get::<_, i64>(9)? == 1,
                "raisons": serde_json::from_str::<Value>(&row.get::<_, String>(10)?).unwrap_or(json!([])),
                "signaux": serde_json::from_str::<Value>(&row.get::<_, String>(11)?).unwrap_or(json!([])),
                "issue": row.get::<_, String>(12)?,
                "derogue": row.get::<_, i64>(13)? == 1,
                "created_at": row.get::<_, i64>(14)?,
                "nom_ok": row.get::<_, i64>(15)?,
                "nom_verifie": NomRecu::de(row.get::<_, i64>(15)?).as_str(),
            }))
        })
        .map_err(|e| format!("Lecture des preuves impossible ({})", e))?;
    Ok(lignes.filter_map(|l| l.ok()).collect())
}

/// L'image d'une preuve, pour la seule partie qui l'a déposée et son
/// vis-à-vis (garde faite en amont par la route).
pub fn image_de(conn: &Connection, identifiant: i64) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT image FROM p2p_proofs WHERE id = ?1",
        params![identifiant],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .map_err(|e| format!("Lecture de l'image impossible ({})", e))
}

/// Ecrit l'issue d'une commande sur ses preuves : c'est le seul label dont
/// dispose le moteur pour apprendre.
pub fn marquer_issue(conn: &Connection, commande: i64, issue: &str) -> Result<usize, String> {
    let horodatage = maintenant_secondes();
    conn.execute(
        "UPDATE p2p_proofs SET issue = ?1, issue_at = ?2
             WHERE order_id = ?3 AND issue = ''",
        params![issue, horodatage, commande],
    )
    .map_err(|e| format!("Issue de preuve impossible ({})", e))
}

/// L'acheteur passe outre un verdict écarté : la trace en dit explicitement le
/// nom. Le vendeur voit la mention, et l'issue de la commande instructera le
/// moteur sur ce choix.
pub fn marquer_derogue(conn: &Connection, commande: i64, uq: &str) -> Result<usize, String> {
    conn.execute(
        "UPDATE p2p_proofs SET derogue = 1
             WHERE id = (
                 SELECT id FROM p2p_proofs
                  WHERE order_id = ?1 AND UPPER(uq) = UPPER(?2)
                  ORDER BY id DESC LIMIT 1
             )",
        params![commande, uq],
    )
    .map_err(|e| format!("Derogation de preuve impossible ({})", e))
}

/// La derniere preuve posee par une partie, avec son verdict : c'est le sésame
/// qui ouvre le bouton « J'ai payé ».
pub fn derniere_preuve_de(
    conn: &Connection,
    commande: i64,
    uq: &str,
) -> Result<Option<Value>, String> {
    conn.query_row(
        "SELECT id, role, verdict, conformite, veto, derogue, nom_ok
             FROM p2p_proofs
             WHERE order_id = ?1 AND UPPER(uq) = UPPER(?2)
             ORDER BY id DESC LIMIT 1",
        params![commande, uq],
        |row| {
            Ok(json!({
                "id": row.get::<_, i64>(0)?,
                "role": row.get::<_, String>(1)?,
                "verdict": row.get::<_, String>(2)?,
                "conformite": row.get::<_, f64>(3)?,
                "veto": row.get::<_, i64>(4)? == 1,
                "derogue": row.get::<_, i64>(5)? == 1,
                "nom_ok": row.get::<_, i64>(6)?,
                "nom_verifie": NomRecu::de(row.get::<_, i64>(6)?).as_str(),
            }))
        },
    )
    .optional()
    .map_err(|e| format!("Lecture de la derniere preuve impossible ({})", e))
}

/// Recalcul des pondérations à partir des cas annotés. Deux époques, pas qui
/// décroie : le modèle bouge peu à chaque commande terminée et reste lisible.
pub fn recalibrer(conn: &Connection) -> Result<Value, String> {
    let mut stmt = conn
        .prepare("SELECT signaux, issue FROM p2p_proofs WHERE issue IN ('completed','litige') ORDER BY id")
        .map_err(|e| format!("Apprentissage : lecture impossible ({})", e))?;
    let lignes = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| format!("Apprentissage : lecture impossible ({})", e))?;
    let mut cas: Vec<(Vec<Signal>, f64)> = Vec::new();
    for ligne in lignes {
        let (texte, issue) =
            ligne.map_err(|e| format!("Apprentissage : ligne illisible ({})", e))?;
        let valeurs: Value = match serde_json::from_str(&texte) {
            Ok(valeurs) => valeurs,
            Err(_) => continue,
        };
        let mut signaux = Vec::new();
        if let Some(tableau) = valeurs.as_array() {
            for entree in tableau {
                let nom = entree.get("nom").and_then(|v| v.as_str()).unwrap_or("");
                let force = entree.get("force").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let detail = entree
                    .get("detail")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if let Some(connu) = connaitre_nom(nom) {
                    signaux.push(Signal {
                        nom: connu,
                        force,
                        detail,
                    });
                }
            }
        }
        let cible = if issue == "completed" { 1.0 } else { 0.0 };
        if !signaux.is_empty() {
            cas.push((signaux, cible));
        }
    }
    if (cas.len() as i64) < ECHANTILLON_MIN {
        return Ok(json!({ "echantillons": cas.len(), "poids": poids_par_defaut_json() }));
    }

    let mut poids = poids_par_defaut();
    for epoch in 0..2 {
        let pas = PAS_APPRENTISSAGE * (0.6f64.powi(epoch));
        for (signaux, cible) in &cas {
            let note = conformite_des(signaux, &poids);
            let erreur = note - cible;
            for signal in signaux {
                let priorite = priorite_de(signal.nom);
                let courant = poids.get(signal.nom).copied().unwrap_or(priorite);
                let nouveau =
                    (courant + pas * erreur * signal.force).clamp(priorite * 0.25, priorite * 4.0);
                poids.insert(signal.nom.to_string(), nouveau);
            }
        }
    }

    let horodatage = maintenant_secondes();
    let texte = serde_json::to_string(&poids_json(&poids)).unwrap_or("{}".to_string());
    conn.execute(
        "UPDATE p2p_proof_model SET poids = ?1, echantillons = ?2, updated_at = ?3 WHERE id = 1",
        params![texte, cas.len() as i64, horodatage],
    )
    .map_err(|e| format!("Apprentissage : écriture impossible ({})", e))?;
    Ok(
        json!({ "echantillons": cas.len(), "poids": serde_json::from_str::<Value>(&texte).unwrap_or(Value::Null) }),
    )
}

/// Les noms de signaux vivent dans le binaire ; la base n'en porte que des
/// chaînes. Un nom inconnu est abandonné : il n'a pas de priori à apprendre.
fn connaitre_nom(nom: &str) -> Option<&'static str> {
    POIDS_DEFAUT
        .iter()
        .find(|(connue, _)| *connue == nom)
        .map(|(connue, _)| *connue)
}

fn priorite_de(nom: &str) -> f64 {
    POIDS_DEFAUT
        .iter()
        .find(|(connue, _)| *connue == nom)
        .map(|(_, poids)| *poids)
        .unwrap_or(0.10)
}

fn poids_json(poids: &HashMap<String, f64>) -> serde_json::Map<String, Value> {
    poids
        .iter()
        .map(|(nom, valeur)| (nom.clone(), json!((valeur * 1000.0).round() / 1000.0)))
        .collect()
}

fn poids_par_defaut_json() -> Value {
    Value::Object(poids_json(&poids_par_defaut()))
}

// ============================ Tests ============================

#[cfg(test)]
mod tests {
    use super::*;

    /// PNG de bruit : contrasté, donc porteur de « détail » comme une capture
    /// d'écran, et suffisamment lourd pour ne pas passer pour une vignette.
    fn png_brute() -> Vec<u8> {
        let img = image::GrayImage::from_fn(600, 800, |x, y| {
            image::Luma([((x * 31 + y * 17) % 251) as u8])
        });
        let mut curseur = std::io::Cursor::new(Vec::new());
        img.write_to(&mut curseur, image::ImageFormat::Png).unwrap();
        curseur.into_inner()
    }

    /// PNG d'un seul gris : ce que rendrait un fond recomposé sans texte.
    fn png_plate() -> Vec<u8> {
        let img = image::GrayImage::from_pixel(600, 800, image::Luma([250u8]));
        let mut curseur = std::io::Cursor::new(Vec::new());
        img.write_to(&mut curseur, image::ImageFormat::Png).unwrap();
        curseur.into_inner()
    }

    /// Insère un chunk tEXt avant le IEND : le fichier déclare lui-même sa
    /// fabrication, exactement comme un export de générateur.
    fn png_avec_champ(kle: &str, valeur: &str) -> Vec<u8> {
        let png = png_brute();
        let mut corps = kle.as_bytes().to_vec();
        corps.push(0);
        corps.extend_from_slice(valeur.as_bytes());

        let mut chunk = Vec::new();
        chunk.extend_from_slice(&(corps.len() as u32).to_be_bytes());
        chunk.extend_from_slice(b"tEXt");
        chunk.extend_from_slice(&corps);
        let mut somme = crc32fast::Hasher::new();
        somme.update(b"tEXt");
        somme.update(&corps);
        chunk.extend_from_slice(&somme.finalize().to_be_bytes());

        let queue = png.len() - 12;
        let mut sortie = png[..queue].to_vec();
        sortie.extend_from_slice(&chunk);
        sortie.extend_from_slice(&png[queue..]);
        sortie
    }

    fn commande() -> Attendu {
        Attendu {
            montant: 2_800.0,
            devise: "CDF".to_string(),
            numero: "0821 55 44 33".to_string(),
            nom: "Mbaye Diao".to_string(),
            nom_officiel: true,
            cree_le: maintenant_secondes() - 600,
        }
    }

    fn saisie_conforme() -> Saisie {
        Saisie {
            montant: 2_800.0,
            devise: "CDF".to_string(),
            numero: "+243821554433".to_string(),
            nom: "MBAYE DIAO".to_string(),
        }
    }

    fn force_de(analyse: &Analyse, nom: &str) -> f64 {
        analyse
            .signaux
            .iter()
            .find(|signal| signal.nom == nom)
            .map(|signal| signal.force)
            .unwrap()
    }

    fn signal(nom: &'static str, force: f64) -> Signal {
        Signal {
            nom,
            force,
            detail: nom.to_string(),
        }
    }

    #[test]
    fn preuve_conforme_declarative_est_validee() {
        let analyse = analyser(
            &png_brute(),
            &saisie_conforme(),
            &commande(),
            &poids_par_defaut(),
        );
        assert_eq!(analyse.verdict, Verdict::Valide, "{:?}", analyse.raisons());
        assert!(!analyse.veto);
        assert!(analyse.conformite > SEUIL_VALIDE);
    }

    #[test]
    fn generateur_nomme_dans_le_fichier_impose_le_veto() {
        let analyse = analyser(
            &png_avec_champ("Software", "ComfyUI 0.3.10"),
            &saisie_conforme(),
            &commande(),
            &poids_par_defaut(),
        );
        assert!(analyse.veto);
        assert_eq!(analyse.verdict, Verdict::Rejet);
        assert!(
            analyse
                .raisons()
                .iter()
                .any(|raison| raison.contains("ComfyUI")),
            "{:?}",
            analyse.raisons()
        );
    }

    #[test]
    fn valeurs_recopiees_a_cote_rejettent_la_preuve() {
        let saisi = Saisie {
            montant: 9_400.0,
            devise: "USD".to_string(),
            numero: "0700 11 22 99".to_string(),
            nom: "Maurice Bey".to_string(),
        };
        let analyse = analyser(&png_brute(), &saisi, &commande(), &poids_par_defaut());
        assert_eq!(analyse.verdict, Verdict::Rejet, "{:?}", analyse.raisons());
        assert!(
            !analyse.veto,
            "le veto reste reserve au fichier qui s accuse"
        );
    }

    #[test]
    fn montant_legerement_faute_donne_restera_a_trancher_humainement() {
        let saisi = Saisie {
            montant: 2_890.0,
            ..saisie_conforme()
        };
        let analyse = analyser(&png_brute(), &saisi, &commande(), &poids_par_defaut());
        assert_eq!(analyse.verdict, Verdict::Doute, "{:?}", analyse.raisons());
    }

    #[test]
    fn image_sans_texte_est_signalee_sans_accuser() {
        let analyse = analyser(
            &png_plate(),
            &saisie_conforme(),
            &commande(),
            &poids_par_defaut(),
        );
        assert!(force_de(&analyse, "texte_absent") > 0.9);
        assert!(!analyse.veto);
        assert_eq!(analyse.verdict, Verdict::Doute, "{:?}", analyse.raisons());
    }

    #[test]
    fn numero_se_retrouve_malgre_le_format_international() {
        // Un numéro conforme rassure (force negative), un faux accuse.
        assert!(numero_correspond("+243 82 155 44 33", "0821 55 44 33") <= -0.4);
        assert_eq!(numero_correspond("0821554433", "0821 55 44 33"), -1.0);
        assert!(numero_correspond("0999 88 77 66", "0821 55 44 33") > 0.0);
        assert_eq!(numero_correspond("", "0821 55 44 33"), 0.0);
    }

    #[test]
    fn nom_supporte_accents_et_casse() {
        let attendu = Attendu {
            nom: "Aïcha Ndiaye".to_string(),
            ..commande()
        };
        let saisi = Saisie {
            nom: "AICHA NDIAYE".to_string(),
            ..saisie_conforme()
        };
        let analyse = analyser(&png_brute(), &saisi, &attendu, &poids_par_defaut());
        assert!(
            force_de(&analyse, "nom") < 0.0,
            "le nom doit compter comme rassurant"
        );
    }

    fn base() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        assurer_tables(&conn).unwrap();
        conn
    }

    fn grain(conn: &Connection, commande: i64, signaux: &[(&str, f64)], issue: &str) {
        let texte = serde_json::to_string(
            &signaux
                .iter()
                .map(|(nom, force)| json!({ "nom": nom, "force": force }))
                .collect::<Vec<_>>(),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO p2p_proofs
                (order_id, role, uq, verdict, conformite, signaux, issue, created_at)
             VALUES (?1, 'buyer', 'UQ-1', 'doute', 0.5, ?2, ?3, 0)",
            params![commande, texte, issue],
        )
        .unwrap();
    }

    #[test]
    fn marquer_issue_et_lister_restituent_les_preuves() {
        let conn = base();
        grain(&conn, 7, &[("montant", -1.0)], "");
        grain(&conn, 7, &[("retouche", 1.0)], "");
        assert_eq!(marquer_issue(&conn, 7, "completed").unwrap(), 2);
        let liste = lister_preuves(&conn, 7).unwrap();
        assert_eq!(liste.len(), 2);
        assert_eq!(liste[0]["issue"].as_str(), Some("completed"));
        // Une seconde issue n'écrase pas la première : la commande est close.
        assert_eq!(marquer_issue(&conn, 7, "litige").unwrap(), 0);
    }

    #[test]
    fn apprentissage_augmente_le_poids_des_signaux_de_litige() {
        let conn = base();
        // Douze preuves tenues, aucune trace de retouche.
        for _ in 0..12 {
            grain(
                &conn,
                1,
                &[("montant", -1.0), ("numero", -1.0), ("retouche", 0.0)],
                "completed",
            );
        }
        // Six litiges, tous avec la retouche déclenchée.
        for _ in 0..6 {
            grain(
                &conn,
                2,
                &[("montant", -1.0), ("numero", -1.0), ("retouche", 1.0)],
                "litige",
            );
        }
        let avant = charger_poids(&conn);
        recalibrer(&conn).unwrap();
        let apres = charger_poids(&conn);

        let priorite = priorite_de("retouche");
        assert_eq!(avant.get("retouche"), Some(&priorite));
        assert!(
            apres["retouche"] > priorite,
            "un signal qui n'apparait que dans les litiges doit peser plus : {:?}",
            apres.get("retouche")
        );
        assert!(apres["retouche"] <= priorite * 4.0, "le poids reste borné");

        // Le but n'est pas un chiffre précis : une preuve qui ressemble à un
        // litige doit être notée plus sévèrement qu'avant l'apprentissage.
        let temoin = vec![
            signal("montant", -1.0),
            signal("numero", -1.0),
            signal("retouche", 1.0),
        ];
        assert!(
            conformite_des(&temoin, &apres) < conformite_des(&temoin, &avant),
            "le moteur doit devenir plus dur sur le signal qui n'apparait que dans les litiges"
        );
    }

    #[test]
    fn trop_peu_de_cas_gardent_les_priorites() {
        let conn = base();
        for _ in 0..3 {
            grain(&conn, 1, &[("retouche", 1.0)], "litige");
        }
        recalibrer(&conn).unwrap();
        let poids = charger_poids(&conn);
        assert_eq!(poids.get("retouche"), Some(&priorite_de("retouche")));
    }

    #[test]
    fn data_uri_decode_et_plafond() {
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(png_brute());
        let octets = decoder_image(&format!("data:image/png;base64,{}", b64)).unwrap();
        assert_eq!(crate::image_meta::format_de(&octets), "png");
        assert!(decoder_image("https://exemple.fr/recu.png").is_err());
    }

    #[test]
    fn image_reutilisee_est_refusee_sur_une_autre_preuve() {
        let conn = base();
        let octets = png_brute();
        let analyse = analyser(
            &octets,
            &saisie_conforme(),
            &commande(),
            &poids_par_defaut(),
        );
        inserer_preuve(&conn, 41, "buyer", "UQ-2", &saisie_conforme(), &octets, &analyse)
            .expect("première preuve");
        let doublon = inserer_preuve(
            &conn,
            42,
            "buyer",
            "UQ-3",
            &saisie_conforme(),
            &octets,
            &analyse,
        );
        assert!(doublon.is_err());
        assert!(doublon.err().unwrap().contains("déjà été utilisée"));
    }

    #[test]
    fn derniere_preuve_et_derogation_visent_la_bonne_partie() {
        let conn = base();
        grain(&conn, 9, &[("montant", 1.0)], "");
        grain(&conn, 9, &[("montant", -1.0)], "");

        // La partie qui n'a rien déposé n'a pas de sésame.
        assert!(derniere_preuve_de(&conn, 9, "UQ-2").unwrap().is_none());

        let derniere = derniere_preuve_de(&conn, 9, "uq-1")
            .unwrap()
            .expect("preuve");
        let premiere = lister_preuves(&conn, 9).unwrap()[0]["id"].as_i64().unwrap();
        assert_ne!(derniere["id"].as_i64(), Some(premiere));
        assert_eq!(derniere["verdict"].as_str(), Some("doute"));
        assert_eq!(derniere["derogue"].as_bool(), Some(false));

        // La dérogation ne marque que la preuve en cours d'utilisation.
        assert_eq!(marquer_derogue(&conn, 9, "UQ-1").unwrap(), 1);
        let preuves = lister_preuves(&conn, 9).unwrap();
        assert_eq!(preuves[0]["derogue"].as_bool(), Some(false));
        assert_eq!(preuves[1]["derogue"].as_bool(), Some(true));
        assert_eq!(marquer_derogue(&conn, 9, "UQ-9").unwrap(), 0);
    }

    #[test]
    fn nom_divergent_ecarte_la_preuve_sans_accuser_le_fichier() {
        // Le test de connaissance : une capture dont le nom désigne quelqu'un
        // d'autre est écartée, mais le veto garde sa signification réservée au
        // fichier qui s'avoue fabriqué. La dérogation assumée reste possible.
        let saisi = Saisie {
            nom: "Joseph Kabila".to_string(),
            ..saisie_conforme()
        };
        let analyse = analyser(&png_brute(), &saisi, &commande(), &poids_par_defaut());
        assert_eq!(analyse.nom_ok, NomRecu::Divergent);
        assert_eq!(analyse.verdict, Verdict::Rejet, "{:?}", analyse.raisons());
        assert!(
            !analyse.veto,
            "la divergence de nom n'est pas un veto de fichier"
        );
    }

    #[test]
    fn nom_recopie_vaut_test_de_connaissance_reussi() {
        let analyse = analyser(
            &png_brute(),
            &saisie_conforme(),
            &commande(),
            &poids_par_defaut(),
        );
        assert_eq!(analyse.nom_ok, NomRecu::Conforme);
        assert_eq!(force_de(&analyse, "nom"), -1.0);
    }

    #[test]
    fn nom_masque_par_l_operateur_ne_peut_accuser_personne() {
        // Certains opérateurs n'affichent que « A**** O**** » : plus aucun mot
        // comparable. Le signal reste muet au lieu d'écarter une vraie capture.
        let saisi = Saisie {
            nom: "A**** O****".to_string(),
            ..saisie_conforme()
        };
        let analyse = analyser(&png_brute(), &saisi, &commande(), &poids_par_defaut());
        assert_eq!(analyse.nom_ok, NomRecu::Inverifiable);
        assert_eq!(force_de(&analyse, "nom"), 0.0);
    }

    #[test]
    fn faute_de_nom_officiel_le_test_reste_muet() {
        // La contrepartie n'a pas enregistré son nom officiel : la comparaison
        // ne porterait sur rien, donc elle ne bloque personne.
        let attendu = Attendu {
            nom_officiel: false,
            ..commande()
        };
        let analyse = analyser(
            &png_brute(),
            &saisie_conforme(),
            &attendu,
            &poids_par_defaut(),
        );
        assert_eq!(analyse.nom_ok, NomRecu::Inverifiable);
        assert_eq!(force_de(&analyse, "nom"), 0.0);
    }

    #[test]
    fn le_verdict_du_nom_se_stocke_et_se_relit() {
        let conn = base();
        let saisi = Saisie {
            nom: "Quelqu Un D Autre".to_string(),
            ..saisie_conforme()
        };
        let analyse = analyser(&png_brute(), &saisi, &commande(), &poids_par_defaut());
        inserer_preuve(&conn, 11, "buyer", "UQ-11", &saisi, &png_brute(), &analyse).unwrap();

        let liste = lister_preuves(&conn, 11).unwrap();
        assert_eq!(liste[0]["nom_ok"].as_i64(), Some(-1));
        assert_eq!(liste[0]["nom_verifie"].as_str(), Some("divergent"));

        let derniere = derniere_preuve_de(&conn, 11, "UQ-11")
            .unwrap()
            .expect("preuve");
        assert_eq!(derniere["nom_ok"].as_i64(), Some(-1));
        assert_eq!(derniere["nom_verifie"].as_str(), Some("divergent"));
    }

    #[test]
    fn le_nom_divergent_ecarte_sans_falsifier_le_score() {
        // Le score reste la mesure des signaux : c'est le verdict qui écarte,
        // donc la dérogation tracée garde la porte ouverte.
        let saisi = Saisie {
            nom: "Joseph Kabila".to_string(),
            ..saisie_conforme()
        };
        let analyse = analyser(&png_brute(), &saisi, &commande(), &poids_par_defaut());
        assert_eq!(
            analyse.conformite,
            conformite_des(&analyse.signaux, &poids_par_defaut())
        );
        assert_eq!(analyse.verdict, Verdict::Rejet);
    }
}
