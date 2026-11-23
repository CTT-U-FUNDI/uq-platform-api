// ============================================================
// AI DETECTOR — Detection des images generees par une IA
// ============================================================
//
// Le detecteur travaille sur les champs de metadonnees reels extraits par
// `crate::image_meta`, jamais sur l'octet stream brut du fichier. La version
// precedente cherchait « Flux », « imagen » ou « synthetic » dans les 8
// premiers Ko du fichier : une photo normale contient des donnees JPEG
// compressees ou, pire, un texte en espagnol (« imagen » = image), et le
// signal etait affiche avec 0,90 de confiance. Une accusation d'image generee
// ne peut pas reposer sur un hasard d'octets.
//
// Regles actuelles :
//  * un nom de generateur ne compte que dans un champ de fabrication
//    (logiciel, createur, appareil, description, paquet XMP) ;
//  * la correspondance est ancoree sur les frontieres de mot ;
//  * C2PA n'est PAS une preuve d'IA : ce sont des credentials de contenu,
//    producteur d'appareil photo comme modele. Le signal devient
//    « provenance declaree », gravite informative ;
//  * sans metadonnee, le verdict est « inconnu », pas « sur » : l'absence de
//    trace ne prouve rien.

use std::fs;
use std::path::Path;

use crate::image_meta::MetadonneesImage;

/// Resultat de la detection.
#[derive(Debug, Clone)]
pub struct DetectionResult {
    pub is_ai_generated: bool,
    /// 0,0 = aucune trace ; 1,0 = generateur nomme noir sur blanc.
    pub confidence: f32,
    pub signals: Vec<String>,
    /// `ai_detected` | `inconnu` | `provenance_declaree` | `error`
    pub verdict: String,
}

impl DetectionResult {
    pub fn ai_detected(signals: Vec<String>, confidence: f32) -> Self {
        Self {
            is_ai_generated: true,
            confidence,
            signals,
            verdict: "ai_detected".to_string(),
        }
    }

    /// Image sans metadonnee de fabrication : rien n'est prouve dans un sens
    /// ni dans l'autre.
    pub fn inconnu(motif: &str) -> Self {
        Self {
            is_ai_generated: false,
            confidence: 0.0,
            signals: vec![motif.to_string()],
            verdict: "inconnu".to_string(),
        }
    }

    pub fn error(msg: &str) -> Self {
        Self {
            is_ai_generated: false,
            confidence: 0.0,
            signals: vec![msg.to_string()],
            verdict: "error".to_string(),
        }
    }

    pub fn as_json(&self) -> serde_json::Value {
        serde_json::json!({
            "ai_generee": self.is_ai_generated,
            "confiance": (self.confidence * 100.0).round() / 100.0,
            "signaux": self.signals,
            "verdict": self.verdict,
        })
    }
}

/// Generateurs d'images dont le nom apparait dans les metadonnees de sortie.
/// Nom officiel, puis variantes rencontrees dans les champs `Software`.
const GENERATEURS: &[(&str, &[&str])] = &[
    ("DALL·E", &["dall-e", "dall·e", "dall_e", "dalle", "openai"]),
    ("Midjourney", &["midjourney", "mid_journey"]),
    (
        "Stable Diffusion",
        &[
            "stable diffusion",
            "stable-diffusion",
            "stablediffusion",
            "sd-xl",
            "sdxl",
            "sd-webui",
            "automatic1111",
        ],
    ),
    ("ComfyUI", &["comfyui", "comfy-ui", "comfy_frontend"]),
    ("NovelAI", &["novelai"]),
    ("Adobe Firefly", &["firefly", "adobe firefly"]),
    ("Ideogram", &["ideogram"]),
    (
        "FLUX (Black Forest Labs)",
        &[
            "flux.1",
            "flux-1",
            "fluxdev",
            "flux1",
            "black forest labs",
            "bfl.ai",
        ],
    ),
    (
        "Google Imagen",
        &["imagen 2", "imagen3", "imagen 3", "google imagen"],
    ),
    ("Microsoft CogView/MAI", &["cogview", "mai-image"]),
    ("Recraft", &["recraft"]),
    ("Leonardo AI", &["leonardo.ai", "leonardo ai"]),
    ("Bing Image Creator", &["bing image creator"]),
    ("Wan / Hunyuan", &["wan2.1", "hunyuan-dit", "hunyuan image"]),
];

/// Un champ de fabrication qui ne porte qu'une logithèque de retouche courante
/// (pas un generateur) est une information, pas une fraude.
const OUTILS_NEUTRES: &[&str] = &[
    "photoshop",
    "gimp",
    "lightroom",
    "snapseed",
    "darktable",
    "canva",
    "winimage",
    "picsart",
    "preview",
    "jpeglib",
    "libjpeg",
    "pnglib",
    "imageio",
    "pillow",
    "opencv",
    "ffmpeg",
    "whatsapp",
    "telegram",
    "signal",
    "chrome",
    "screenshot",
    "capture",
];

/// Ancre sur les frontieres de mot : « fluxratio » ne doit pas matcher « flux ».
/// Le tiret compte comme separateur — « SD-WebUI » et « flux.1-dev » restent des
/// signatures valides — alors que le soulignement reste collant (identifiants).
fn contient_signature(texte: &str, variante: &str) -> bool {
    let texte = texte.to_lowercase();
    let cible = variante.to_lowercase();
    if cible.is_empty() {
        return false;
    }
    let bord = |c: Option<char>| {
        c.map(|car| !(car.is_alphanumeric() || car == '_'))
            .unwrap_or(true)
    };
    let caracteres: Vec<char> = texte.chars().collect();
    let cible_car: Vec<char> = cible.chars().collect();
    if cible_car.is_empty() || cible_car.len() > caracteres.len() {
        return false;
    }
    for debut in 0..=(caracteres.len() - cible_car.len()) {
        if caracteres[debut..debut + cible_car.len()] == cible_car[..] {
            let avant = debut
                .checked_sub(1)
                .and_then(|i| caracteres.get(i))
                .copied();
            if bord(avant) && bord(caracteres.get(debut + cible_car.len()).copied()) {
                return true;
            }
        }
    }
    false
}

/// Detection sur des metadonnees deja lues.
pub fn detect_ai_metadonnees(meta: &MetadonneesImage) -> DetectionResult {
    let champs = meta.champs_fabrication();
    if champs.is_empty() {
        return DetectionResult::inconnu("aucune metadonnee de fabrication dans le fichier");
    }
    let mut signaux = Vec::new();
    let mut confiance = 0.0f32;

    for champ in &champs {
        for (generateur, variantes) in GENERATEURS {
            if variantes
                .iter()
                .any(|variante| contient_signature(&champ.valeur, variante))
            {
                signaux.push(format!(
                    "generateur {} declare dans le champ {} (« {} »)",
                    generateur, champ.nom, champ.valeur
                ));
                // Un nom explicite dans Software/CreatorTool est une preuve
                // directe ; dans une description libre, c'est plus faible.
                confiance = confiance.max(match champ.famille {
                    crate::image_meta::FamilleChamp::TexteLibre => 0.75,
                    _ => 0.95,
                });
            }
        }
        for outil in OUTILS_NEUTRES {
            if contient_signature(&champ.valeur, outil) {
                signaux.push(format!(
                    "outil de retouche ou d'export : {} dans {}",
                    outil, champ.nom
                ));
                confiance = confiance.min(0.0); // n'augmente pas le score IA
            }
        }
    }

    // C2PA : credentials de contenu. Ce n'est pas une signature d'IA, mais la
    // provenance merite d'etre montree au validateur.
    if meta.c2pa_present {
        signaux.push(
            "credentials de contenu (C2PA) presents : origine declaree par le fichier".to_string(),
        );
    }

    if confiance > 0.0 {
        // Les mentions neutres seules ne doivent pas rester au tableau.
        let signaux_ia: Vec<String> = signaux
            .into_iter()
            .filter(|signal| signal.starts_with("generateur"))
            .collect();
        if !signaux_ia.is_empty() {
            return DetectionResult::ai_detected(signaux_ia, confiance);
        }
    }
    if meta.c2pa_present {
        return DetectionResult {
            is_ai_generated: false,
            confidence: 0.0,
            signals: vec![
                "credentials de contenu (C2PA) presents : origine declaree par le fichier"
                    .to_string(),
            ],
            verdict: "provenance_declaree".to_string(),
        };
    }
    DetectionResult::inconnu("aucun nom de generateur dans les metadonnees")
}

/// Detection par chemin de fichier, conservee pour les appels existants.
pub fn detect_ai(image_path: &str) -> DetectionResult {
    let path = Path::new(image_path);
    if !path.exists() {
        return DetectionResult::error(&format!("Fichier introuvable: {}", image_path));
    }
    match fs::read(path) {
        Ok(octets) => detect_ai_octets(&octets),
        Err(e) => DetectionResult::error(&format!("Lecture impossible: {}", e)),
    }
}

/// Detection sur les octets en memoire — le cas reel d'une preuve recue par
/// HTTP, qui n'est jamais passee par un fichier temporaire.
pub fn detect_ai_octets(octets: &[u8]) -> DetectionResult {
    match crate::image_meta::extraire(octets) {
        Some(meta) => detect_ai_metadonnees(&meta),
        None => DetectionResult::error("format d'image non reconnaissable"),
    }
}

/// Analyse d'un lot de fichiers.
pub fn detect_ai_batch(paths: &[&str]) -> Vec<DetectionResult> {
    paths.iter().map(|p| detect_ai(p)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image_meta::{ChampMeta, FamilleChamp};

    fn meta_avec(champs: Vec<(&str, &str, FamilleChamp)>) -> MetadonneesImage {
        MetadonneesImage {
            format: "png".to_string(),
            octets: 10,
            champs: champs
                .into_iter()
                .map(|(nom, valeur, famille)| ChampMeta {
                    nom: nom.to_string(),
                    valeur: valeur.to_string(),
                    famille,
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn sans_metadonnee_le_verdict_reste_inconnu() {
        let resultat = detect_ai_metadonnees(&MetadonneesImage::default());
        assert_eq!(resultat.verdict, "inconnu");
        assert!(!resultat.is_ai_generated);
    }

    #[test]
    fn logiciel_generateur_detecte() {
        let meta = meta_avec(vec![("Software", "ComfyUI 0.3.10", FamilleChamp::Logiciel)]);
        let resultat = detect_ai_metadonnees(&meta);
        assert!(resultat.is_ai_generated);
        assert!(resultat.confidence >= 0.9);
        assert!(resultat.signals[0].contains("ComfyUI"));
    }

    #[test]
    fn mot_isole_dans_un_texte_libre_detecte_aussi() {
        let meta = meta_avec(vec![(
            "ImageDescription",
            "portrait realise avec Midjourney",
            FamilleChamp::TexteLibre,
        )]);
        let resultat = detect_ai_metadonnees(&meta);
        assert!(resultat.is_ai_generated);
        assert!((resultat.confidence - 0.75).abs() < 0.01);
    }

    #[test]
    fn outil_de_retouche_ne_sonne_pas_l_ia() {
        let meta = meta_avec(vec![
            ("Software", "Photoshop 24.0", FamilleChamp::Logiciel),
            ("Make", "VODACOM Q10", FamilleChamp::Appareil),
        ]);
        let resultat = detect_ai_metadonnees(&meta);
        assert!(!resultat.is_ai_generated);
        assert_eq!(resultat.verdict, "inconnu");
    }

    #[test]
    fn frontieres_de_mot_obligatoires() {
        // « fluxratio » n'est pas FLUX ; « sd-webui » l'est.
        assert!(!contient_signature("rapport fluxratio 2026", "flux"));
        assert!(contient_signature("SD-WebUI v1.9", "sd-webui"));
        assert!(contient_signature("flux.1-dev", "flux.1"));
    }

    #[test]
    fn c2pa_seul_n_est_pas_une_accusation() {
        let meta = meta_avec(vec![(
            "xmp",
            "<x:xmpmeta xmlns:c2pa='http://ns.adobe.com/c2pa-dc/'>",
            FamilleChamp::TexteLibre,
        )]);
        let mut meta = meta;
        meta.c2pa_present = true;
        let resultat = detect_ai_metadonnees(&meta);
        assert!(!resultat.is_ai_generated);
        assert_eq!(resultat.verdict, "provenance_declaree");
    }

    #[test]
    fn fichier_inexistant_en_error() {
        assert_eq!(detect_ai("nope.jpg").verdict, "error");
    }

    #[test]
    fn octets_hors_image_en_error() {
        assert_eq!(detect_ai_octets(b"bonjour").verdict, "error");
    }
}
