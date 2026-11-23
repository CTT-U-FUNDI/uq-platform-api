// ====================
// 🔎 METADONNEES D'IMAGE — lecture reelle des segments
// ====================
//
// Ce module ne devine rien : il ouvre les segments de metadonnees tels qu'ils
// sont ecrits dans le fichier (EXIF/TIFF, XMP, chunks PNG, marqueurs JPEG) et
// les rend exploitables par `ai_detector` et par le moteur de confiance P2P.
//
// Choix assumee : la recherche de signatures n'est jamais faite sur l'octet
// stream entier. Un fichier de 3 Mo contient des donnees comprimees qui peuvent
// contenir par hasard la suite « Flux » ou « imagen » ; cela produirait de
// fausses accusations d'image generee par IA. On ne scrute que les champs de
// metadonnees, et pour les generateurs, que les champs `logiciel / createur /
// marque / modele / generation`.
//
// Le format `zTXt` de PNG est compresse (Deflate) : sans deflateur embarque dans
// ce binaire, on ne lit que le mot-cle et le fait que le texte est present.
// C'est declare dans le resultat (`ztxt_compresse`) au lieu d'etre invente.

use std::io::Cursor;

/// Une balise de metadonnee : son nom (tag EXIF, mot-cle PNG, nom de champ
/// XMP) et sa valeur lisible.
#[derive(Debug, Clone)]
pub struct ChampMeta {
    pub nom: String,
    pub valeur: String,
    /// Famille du champ : celle ou une signature de generateur IA est probante.
    pub famille: FamilleChamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FamilleChamp {
    Logiciel,
    Createur,
    Appareil,
    Horodatage,
    Position,
    TexteLibre,
    Technique,
}

impl FamilleChamp {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Logiciel => "logiciel",
            Self::Createur => "createur",
            Self::Appareil => "appareil",
            Self::Horodatage => "horodatage",
            Self::Position => "position",
            Self::TexteLibre => "texte",
            Self::Technique => "technique",
        }
    }
}

/// Ce que le fichier declare réellement sur sa fabrication.
#[derive(Debug, Clone, Default)]
pub struct MetadonneesImage {
    pub format: String,
    pub largeur: Option<u32>,
    pub hauteur: Option<u32>,
    pub octets: usize,
    pub exif_present: bool,
    pub xmp_present: bool,
    pub c2pa_present: bool,
    /// Un texte PNG est compresse (zTXt) : sa valeur n'a pas pu etre lue ici.
    pub texte_compresse: bool,
    pub gps_present: bool,
    pub champs: Vec<ChampMeta>,
    /// Tables de quantification JPEG telles que stockees dans le fichier
    /// (une table de 64 valeurs par DQT 8 bits).
    pub tables_quantification: Vec<Vec<u16>>,
    /// Le fichier porte un en-tete JFIF ecrit par une logitheque, sans EXIF
    /// d'appareil photo.
    pub jfif_present: bool,
    pub commentaire: Option<String>,
}

impl MetadonneesImage {
    /// Concatenation des valeurs lues, utilisee pour la chasse aux signatures.
    pub fn texte_metadonnees(&self) -> String {
        let mut tampon = String::new();
        for champ in &self.champs {
            tampon.push_str(champ.famille.as_str());
            tampon.push(':');
            tampon.push_str(&champ.nom);
            tampon.push('=');
            tampon.push_str(&champ.valeur);
            tampon.push('\n');
        }
        tampon
    }

    /// Les seuls champs ou le nom d'un generateur vaut preuve : fabrication,
    /// outil, auteur. « Appareil photo = Canon EOS » n'est pas une signature IA.
    pub fn champs_fabrication(&self) -> Vec<&ChampMeta> {
        self.champs
            .iter()
            .filter(|champ| {
                matches!(
                    champ.famille,
                    FamilleChamp::Logiciel
                        | FamilleChamp::Createur
                        | FamilleChamp::Appareil
                        | FamilleChamp::TexteLibre
                )
            })
            .collect()
    }

    pub fn valeur(&self, recherche: &str) -> Option<&str> {
        self.champs
            .iter()
            .find(|champ| champ.nom.eq_ignore_ascii_case(recherche))
            .map(|champ| champ.valeur.as_str())
    }
}

// ====================
// POINT D'ENTREE
// ====================

/// Forme reelle du fichier, lue dans ses premiers octets.
pub fn format_de(octets: &[u8]) -> &'static str {
    if octets.starts_with(b"\xFF\xD8\xFF") {
        "jpeg"
    } else if octets.starts_with(b"\x89PNG\r\n\x1a\n") {
        "png"
    } else if octets.len() > 12 && &octets[0..4] == b"RIFF" && &octets[8..12] == b"WEBP" {
        "webp"
    } else if octets.starts_with(b"GIF8") {
        "gif"
    } else if octets.len() > 12 && &octets[4..8] == b"ftyp" {
        "iso-media"
    } else if octets.starts_with(b"BM") {
        "bmp"
    } else if octets.starts_with(b"\x00\x00\x01\x00") {
        "ico"
    } else {
        "inconnu"
    }
}

/// Lit tout ce que le fichier declare. `None` seulement si l'octet stream
/// n'est pas une image reconnaissable.
pub fn extraire(octets: &[u8]) -> Option<MetadonneesImage> {
    let format = format_de(octets);
    if format == "inconnu" {
        return None;
    }
    let mut meta = MetadonneesImage {
        format: format.to_string(),
        octets: octets.len(),
        ..Default::default()
    };
    // Dimensions : en-tete seule, pas de decompressement des pixels.
    if let Ok(reader) = image::ImageReader::new(Cursor::new(octets)).with_guessed_format() {
        if let Ok((largeur, hauteur)) = reader.into_dimensions() {
            meta.largeur = Some(largeur);
            meta.hauteur = Some(hauteur);
        }
    }
    match format {
        "jpeg" => lire_jpeg(octets, &mut meta),
        "png" => lire_png(octets, &mut meta),
        // Les autres conteneurs ne sont pas decortiques ici : le bilan portera
        // la mention « metadonnees non lues dans ce format », jamais un vide
        // presenting comme une absence de traces.
        _ => meta.champs.push(ChampMeta {
            nom: "format".to_string(),
            valeur: format.to_string(),
            famille: FamilleChamp::Technique,
        }),
    }
    meta.c2pa_present = meta
        .champs
        .iter()
        .any(|champ| champ.valeur.to_lowercase().contains("c2pa"));
    Some(meta)
}

// ====================
// JPEG
// ====================

fn lire_jpeg(octets: &[u8], meta: &mut MetadonneesImage) {
    let mut position = 2usize; // apres SOI
    while position + 4 <= octets.len() {
        if octets[position] != 0xFF {
            position += 1;
            continue;
        }
        let marqueur = octets[position + 1];
        // 0xFF00 est un octet echappe dans les data, pas un marqueur.
        if marqueur == 0x00 || marqueur == 0xFF {
            position += 1;
            continue;
        }
        // RST / SOI / EOI : pas de longueur.
        if (0xD0..=0xD9).contains(&marqueur) {
            position += 2;
            continue;
        }
        if marqueur == 0xDA {
            // debut des scan data : on arrete la marche des marqueurs.
            break;
        }
        let longueur = ((octets[position + 2] as usize) << 8) | octets[position + 3] as usize;
        if longueur < 2 {
            break;
        }
        let debut = position + 4;
        let fin = (position + 2 + longueur).min(octets.len());
        let segment = &octets[debut.min(fin)..fin];
        match marqueur {
            0xE0 => {
                if segment.starts_with(b"JFIF") {
                    meta.jfif_present = true;
                }
            }
            0xE1 => {
                if segment.starts_with(b"Exif\x00\x00") {
                    meta.exif_present = true;
                    lire_tiff(&segment[6..], meta);
                } else if segment.starts_with(b"http://ns.adobe.com/xap/1.0/") {
                    meta.xmp_present = true;
                    let xmp = String::from_utf8_lossy(&segment[28..]);
                    champs_xmp(&xmp, meta);
                }
            }
            // APP2 peut porter l'IFD de balisage couleur ou du XMP segmente.
            0xE2 if segment.starts_with(b"ICC_PROFILE") => {}
            0xFE => {
                let texte = String::from_utf8_lossy(segment).trim().to_string();
                if !texte.is_empty() {
                    meta.commentaire = Some(texte.clone());
                    meta.champs.push(ChampMeta {
                        nom: "Comment".to_string(),
                        valeur: borne(&texte),
                        famille: FamilleChamp::TexteLibre,
                    });
                }
            }
            0xDB => collecter_dqt(segment, meta),
            // EXIF peut aussi voyager en APP12 (Ducky) ou APP13 : on lit le
            // texte imprimable, utile pour les outils d'export.
            0xEC | 0xED | 0xEE => {
                let texte = texte_imprimable(segment);
                if texte.len() > 3 {
                    meta.champs.push(ChampMeta {
                        nom: format!("app{:x}", marqueur),
                        valeur: borne(&texte),
                        famille: FamilleChamp::TexteLibre,
                    });
                }
            }
            _ => {}
        }
        position = debut + (longueur - 2);
    }
}

/// Les tables de quantification : 0x21 (8 bits), 0x22 (16 bits).
fn collecter_dqt(segment: &[u8], meta: &mut MetadonneesImage) {
    let mut index = 0usize;
    while index + 1 < segment.len() {
        let element = segment[index];
        let precision = element >> 4;
        let tableau = (element & 0x0F) as usize;
        index += 1;
        let taille = if precision == 1 { 128 } else { 64 };
        if index + taille > segment.len() {
            break;
        }
        // Zig-zag natif du fichier : la comparaison avec les tables standard se
        // fait dans le meme ordre, sans re-permutation.
        let mut valeurs = Vec::with_capacity(taille);
        for i in 0..taille {
            valeurs.push(if precision == 1 {
                ((segment[index + i * 2] as u16) << 8) | segment[index + i * 2 + 1] as u16
            } else {
                segment[index + i] as u16
            });
        }
        index += taille;
        while meta.tables_quantification.len() <= tableau {
            meta.tables_quantification.push(Vec::new());
        }
        meta.tables_quantification[tableau] = valeurs;
    }
}

// ====================
// PNG
// ====================

fn lire_png(octets: &[u8], meta: &mut MetadonneesImage) {
    let mut position = 8usize;
    while position + 8 <= octets.len() {
        let longueur = u32::from_be_bytes([
            octets[position],
            octets[position + 1],
            octets[position + 2],
            octets[position + 3],
        ]) as usize;
        let type_chunk = &octets[position + 4..position + 8];
        let debut = position + 8;
        let fin = debut + longueur;
        if fin + 4 > octets.len() {
            break;
        }
        let corps = &octets[debut..fin];
        match type_chunk {
            b"tEXt" => {
                if let Some(sep) = corps.iter().position(|b| *b == 0) {
                    let mot_cle = String::from_utf8_lossy(&corps[..sep]).into_owned();
                    let valeur = String::from_utf8_lossy(&corps[sep + 1..]).into_owned();
                    meta.champs.push(ChampMeta {
                        nom: mot_cle.clone(),
                        valeur: borne(&valeur),
                        famille: famille_de_nom(&mot_cle),
                    });
                }
            }
            b"iTXt" => {
                // mot-cle\0 drapeau compression\0 methode\0 langue\0 traduction\0 texte
                if let Some(sep) = corps.iter().position(|b| *b == 0) {
                    let mot_cle = String::from_utf8_lossy(&corps[..sep]).into_owned();
                    let apres = &corps[sep + 1..];
                    let compresse = apres.first().copied().unwrap_or(0) == 1;
                    let texte = if compresse {
                        meta.texte_compresse = true;
                        "<texte compresse>".to_string()
                    } else {
                        deux_zeres(apres)
                            .map(|suite| String::from_utf8_lossy(suite).into_owned())
                            .unwrap_or_default()
                    };
                    if mot_cle == "XML:com.adobe.xmp" && !compresse {
                        meta.xmp_present = true;
                        champs_xmp(&texte, meta);
                    } else {
                        meta.champs.push(ChampMeta {
                            nom: mot_cle.clone(),
                            valeur: borne(&texte),
                            famille: famille_de_nom(&mot_cle),
                        });
                    }
                }
            }
            b"zTXt" => {
                meta.texte_compresse = true;
                if let Some(sep) = corps.iter().position(|b| *b == 0) {
                    meta.champs.push(ChampMeta {
                        nom: String::from_utf8_lossy(&corps[..sep]).into_owned(),
                        valeur: "<compressé deflate, non lu dans ce binaire>".to_string(),
                        famille: FamilleChamp::TexteLibre,
                    });
                }
            }
            b"eXIf" => {
                meta.exif_present = true;
                lire_tiff(corps, meta);
            }
            _ => {}
        }
        position = fin + 4;
    }
}

/// Aprres deux `0` consecutifs (langue puis traduction translitteree).
fn deux_zeres(corps: &[u8]) -> Option<&[u8]> {
    let premier = corps.iter().position(|b| *b == 0)?;
    let second = corps[premier + 1..].iter().position(|b| *b == 0)?;
    Some(&corps[premier + 1 + second + 1..])
}

// ====================
// TIFF / EXIF
// ====================

struct BlocTiff<'a> {
    octets: &'a [u8],
    petit: bool,
}

impl<'a> BlocTiff<'a> {
    fn u16(&self, off: usize) -> Option<u16> {
        let f = off.checked_add(2)?;
        if f > self.octets.len() {
            return None;
        }
        let (a, b) = (self.octets[off] as u16, self.octets[off + 1] as u16);
        Some(if self.petit {
            a | (b << 8)
        } else {
            (a << 8) | b
        })
    }

    fn u32(&self, off: usize) -> Option<u32> {
        let f = off.checked_add(4)?;
        if f > self.octets.len() {
            return None;
        }
        let lu = |i: usize| self.octets[i] as u32;
        Some(if self.petit {
            lu(off) | (lu(off + 1) << 8) | (lu(off + 2) << 16) | (lu(off + 3) << 24)
        } else {
            (lu(off) << 24) | (lu(off + 1) << 16) | (lu(off + 2) << 8) | lu(off + 3)
        })
    }
}

/// Taille en octets d'un type TIFF (1=BYTE 2=ASCII 3=SHORT 4=LONG 5=RATIONAL
/// 7=UNDEFINED 9=SLONG 10=SRATIONAL 13=IFD).
fn taille_type(type_: u16) -> usize {
    match type_ {
        1 | 2 | 6 | 7 => 1,
        3 | 8 => 2,
        4 | 9 | 13 => 4,
        5 | 10 => 8,
        11 | 12 => 4,
        _ => 0,
    }
}

fn lire_tiff<'a>(octets: &'a [u8], meta: &mut MetadonneesImage) {
    if octets.len() < 8 {
        return;
    }
    let petit = match &octets[0..2] {
        b"II" => true,
        b"MM" => false,
        _ => return,
    };
    let bloc = BlocTiff { octets, petit };
    if bloc.u16(2) != Some(42) {
        return;
    }
    let Some(premier) = bloc.u32(4) else { return };
    marcher_ifd(&bloc, premier, meta, 0);
}

const MAX_IFD: usize = 2;

fn marcher_ifd(bloc: &BlocTiff<'_>, off: u32, meta: &mut MetadonneesImage, profondeur: usize) {
    if profondeur > MAX_IFD || off as usize + 2 > bloc.octets.len() {
        return;
    }
    let Some(nombre) = bloc.u16(off as usize) else {
        return;
    };
    let nombre = nombre.min(512) as usize;
    let mut curseur = off as usize + 2;
    for _ in 0..nombre {
        if curseur + 12 > bloc.octets.len() {
            return;
        }
        let Some(tag) = bloc.u16(curseur) else { return };
        let Some(type_) = bloc.u16(curseur + 2) else {
            return;
        };
        let Some(compte) = bloc.u32(curseur + 4) else {
            return;
        };
        let unitaire = taille_type(type_);
        if unitaire == 0 {
            curseur += 12;
            continue;
        }
        let taille = compte as usize * unitaire;
        let adresse = if taille > 4 {
            match bloc.u32(curseur + 8) {
                Some(valeur) if valeur as usize + taille <= bloc.octets.len() => {
                    Some(valeur as usize)
                }
                _ => None,
            }
        } else {
            Some(curseur + 8)
        };
        if let Some(adresse) = adresse {
            traiter_valeur(bloc, tag, type_, compte, adresse, taille, meta, profondeur);
        }
        // Le pointeur GPS vaut mention meme sans lecture des coordonnees.
        if tag == 0x8825 {
            meta.gps_present = true;
        }
        curseur += 12;
    }
}

fn traiter_valeur(
    bloc: &BlocTiff<'_>,
    tag: u16,
    type_: u16,
    compte: u32,
    adresse: usize,
    taille: usize,
    meta: &mut MetadonneesImage,
    profondeur: usize,
) {
    // Sous-IFD : EXIF (0x8769), GPS (0x8825), Interop (0xA005).
    if (tag == 0x8769 || tag == 0x8825 || tag == 0xA005) && type_ == 4 && profondeur < MAX_IFD {
        if let Some(sous) = bloc.u32(adresse) {
            marcher_ifd(bloc, sous, meta, profondeur + 1);
        }
        return;
    }
    let valeur = match type_ {
        2 => {
            let brut = &bloc.octets[adresse..(adresse + taille).min(bloc.octets.len())];
            let fin = brut.iter().position(|b| *b == 0).unwrap_or(brut.len());
            String::from_utf8_lossy(&brut[..fin]).trim().to_string()
        }
        3 if compte == 1 => bloc.u16(adresse).map(|v| v.to_string()).unwrap_or_default(),
        4 if compte == 1 => bloc.u32(adresse).map(|v| v.to_string()).unwrap_or_default(),
        5 | 10 => {
            // RATIONAL : deux 32 bits, suffisant pour exposer une valeur brute.
            let numerateur = bloc.u32(adresse).unwrap_or(0);
            let denominateur = bloc.u32(adresse + 4).unwrap_or(1).max(1);
            format!("{}/{}", numerateur, denominateur)
        }
        // Binaire (vignette, XMP embarque) : on n'affiche que le volume.
        _ => format!("<{} octets>", taille),
    };
    if valeur.is_empty() || valeur.starts_with('<') {
        return;
    }
    let (nom, famille) = nom_tag(tag);
    if nom.is_empty() {
        return;
    }
    meta.champs.push(ChampMeta {
        nom: nom.to_string(),
        valeur: borne(&valeur),
        famille,
    });
}

fn nom_tag(tag: u16) -> (&'static str, FamilleChamp) {
    use FamilleChamp as F;
    match tag {
        0x010E => ("ImageDescription", F::TexteLibre),
        0x010F => ("Make", F::Appareil),
        0x0110 => ("Model", F::Appareil),
        0x0112 => ("Orientation", F::Technique),
        0x0131 => ("Software", F::Logiciel),
        0x0132 => ("DateTime", F::Horodatage),
        0x9003 => ("DateTimeOriginal", F::Horodatage),
        0x9004 => ("DateTimeDigitized", F::Horodatage),
        0x9005 => ("TimeSource", F::Technique),
        0x8298 => ("Copyright", F::Createur),
        0xA299 => ("Copyright2003", F::Createur),
        0x02BC => ("XMP-embedded", F::TexteLibre),
        0x013B => ("Artist", F::Createur),
        0x900D => ("CreatorProfile", F::Createur),
        0x9033 => ("BodySerialNumber", F::Technique),
        0x9201 => ("ShutterSpeedValue", F::Technique),
        0x9202 => ("ApertureValue", F::Technique),
        0x9209 => ("Flash", F::Technique),
        0xA434 => ("LensModel", F::Technique),
        0x011A => ("XResolution", F::Technique),
        0x011B => ("YResolution", F::Technique),
        _ => ("", F::Technique),
    }
}

// ====================
// XMP
// ====================

/// Les paquets XMP sont du XML : on en tire les champs qui portent une
/// empreinte de fabrication, et on keep le paquet entier borne pour la chasse
/// aux signatures.
fn champs_xmp(xmp: &str, meta: &mut MetadonneesImage) {
    for (balise, famille) in [
        ("xmp:CreatorTool", FamilleChamp::Logiciel),
        ("xmp:MetadataDate", FamilleChamp::Horodatage),
        ("xmp:ModifyDate", FamilleChamp::Horodatage),
        ("xmp:CreateDate", FamilleChamp::Horodatage),
        ("tiff:Software", FamilleChamp::Logiciel),
        ("tiff:Make", FamilleChamp::Appareil),
        ("tiff:Model", FamilleChamp::Appareil),
        ("dc:creator", FamilleChamp::Createur),
        ("dc:title", FamilleChamp::TexteLibre),
        ("photoshop:AuthorsPosition", FamilleChamp::Createur),
        ("crs:Version", FamilleChamp::Logiciel),
        ("dct:created", FamilleChamp::Horodatage),
        ("GCamera:Microphone", FamilleChamp::Technique),
    ] {
        for valeur in valeurs_xmp(xmp, balise) {
            if valeur.is_empty() {
                continue;
            }
            meta.champs.push(ChampMeta {
                nom: balise.rsplit(':').next().unwrap_or(balise).to_string(),
                valeur: borne(&valeur),
                famille,
            });
        }
    }
    meta.champs.push(ChampMeta {
        nom: "xmp".to_string(),
        valeur: borne(xmp),
        famille: FamilleChamp::TexteLibre,
    });
}

/// Attribut `cle="valeur"` ou element `<cle>valeur</cle>`.
fn valeurs_xmp(xmp: &str, cle: &str) -> Vec<String> {
    let mut trouve = Vec::new();
    let motif = format!("{}=\"", cle);
    let mut base = 0usize;
    while let Some(pos) = xmp[base..].find(&motif) {
        let debut = base + pos + motif.len();
        if let Some(fin) = xmp[debut..].find('"') {
            trouve.push(xmp[debut..debut + fin].trim().to_string());
        }
        base = debut;
    }
    let ouvrant = format!("<{}", cle);
    let mut base = 0usize;
    while let Some(pos) = xmp[base..].find(&ouvrant) {
        let debut = base + pos + ouvrant.len();
        let Some(fermeture_balise) = xmp[debut..].find('>') else {
            break;
        };
        let corps = debut + fermeture_balise + 1;
        let fermante = format!("</{}", cle);
        let Some(fin) = xmp[corps..].find(&fermante) else {
            break;
        };
        trouve.push(xmp[corps..corps + fin].trim().to_string());
        base = corps;
    }
    trouve
}

// ====================
// OUTILS
// ====================

fn famille_de_nom(mot_cle: &str) -> FamilleChamp {
    let cle = mot_cle.to_lowercase();
    if cle.contains("software") || cle.contains("generator") || cle.contains("producer") {
        FamilleChamp::Logiciel
    } else if cle.contains("author")
        || cle.contains("creator")
        || cle.contains("artist")
        || cle.contains("copyright")
    {
        FamilleChamp::Createur
    } else if cle.contains("make") || cle.contains("model") || cle.contains("camera") {
        FamilleChamp::Appareil
    } else if cle.contains("date") || cle.contains("time") {
        FamilleChamp::Horodatage
    } else if cle.contains("desc") || cle.contains("comment") || cle.contains("keywords") {
        FamilleChamp::TexteLibre
    } else {
        FamilleChamp::Technique
    }
}

/// Valeur courte et sans caracteres de controle : les champs sont exposes au
/// membre puis stockes en base, ils ne doivent pas trainer un binaire.
fn borne(valeur: &str) -> String {
    valeur
        .chars()
        .filter(|c| !c.is_control() || *c == ' ')
        .take(400)
        .collect()
}

fn texte_imprimable(segment: &[u8]) -> String {
    let mut sortie = String::new();
    let mut suite = 0usize;
    for b in segment {
        if b.is_ascii_graphic() || *b == b' ' {
            sortie.push(*b as char);
            suite += 1;
        } else if suite > 4 {
            break;
        } else {
            sortie.clear();
            suite = 0;
        }
    }
    if suite > 4 {
        sortie
    } else {
        String::new()
    }
}

// ====================
// TESTS
// ====================

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb};

    /// PNG 2x2 sans metadonnee : le parseur ne doit rien inventer.
    fn png_nu() -> Vec<u8> {
        let image = ImageBuffer::<Rgb<u8>, Vec<u8>>::from_pixel(2, 2, Rgb([10, 20, 30]));
        let mut tampon = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut tampon), image::ImageFormat::Png)
            .unwrap();
        tampon
    }

    fn jpeg_de(couleur: Rgb<u8>) -> Vec<u8> {
        let image = ImageBuffer::<Rgb<u8>, Vec<u8>>::from_pixel(64, 48, couleur);
        let mut tampon = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut tampon), image::ImageFormat::Jpeg)
            .unwrap();
        tampon
    }

    #[test]
    fn format_reconnu_sans_deviner() {
        assert_eq!(format_de(&png_nu()), "png");
        assert_eq!(format_de(&jpeg_de(Rgb([1, 2, 3]))), "jpeg");
        assert_eq!(format_de(b"ceci n'est pas une image"), "inconnu");
        assert!(extraire(b"pas une image").is_none());
    }

    #[test]
    fn png_nu_ne_signale_aucune_signature() {
        let meta = extraire(&png_nu()).expect("png lu");
        assert_eq!(meta.format, "png");
        assert_eq!(meta.largeur, Some(2));
        assert_eq!(meta.hauteur, Some(2));
        assert!(!meta.exif_present);
        assert!(!meta.c2pa_present);
        assert!(meta.champs_fabrication().is_empty());
    }

    #[test]
    fn chunk_text_png_est_lu() {
        let mut octets = png_nu();
        // chunk tEXt "Software = Midjourney v6"
        let corps = b"Software\0Midjourney v6";
        let mut chunk = Vec::new();
        chunk.extend_from_slice(&(corps.len() as u32).to_be_bytes());
        chunk.extend_from_slice(b"tEXt");
        chunk.extend_from_slice(corps);
        chunk.extend_from_slice(&[0u8; 4]); // CRC non verifie par le parseur
                                            // insertion avant IEND
        let pos = octets.len() - 12;
        octets.splice(pos..pos, chunk);
        let meta = extraire(&octets).expect("png lu");
        assert_eq!(meta.valeur("Software"), Some("Midjourney v6"));
        let fabrication = meta.champs_fabrication();
        assert!(fabrication
            .iter()
            .any(|champ| champ.famille == FamilleChamp::Logiciel));
    }

    #[test]
    fn jpeg_libre_porte_sa_table_de_quantification() {
        let meta = extraire(&jpeg_de(Rgb([80, 80, 80])).as_slice()).expect("jpeg lu");
        assert!(meta.jfif_present);
        assert!(!meta.exif_present);
        assert!(!meta.tables_quantification.is_empty());
    }

    #[test]
    fn xmp_attribut_et_element() {
        let mut meta = MetadonneesImage::default();
        champs_xmp(
            r#"<x:xmpmeta><xmp:CreatorTool="DALL-E 3"/><tiff:Software><tiff:Software>ComfyUI</tiff:Software></tiff:Software></x:xmpmeta>"#,
            &mut meta,
        );
        assert!(meta.champs.iter().any(|champ| champ.valeur == "DALL-E 3"));
        assert!(meta.champs.iter().any(|champ| champ.valeur == "ComfyUI"));
    }

    #[test]
    fn horodatage_exif_recupere() {
        let mut octets = png_nu();
        // eXIf chunk : TIFF LE, 1 entree DateTimeOriginal (0x9003) ASCII
        let mut tiff = Vec::new();
        tiff.extend_from_slice(b"II");
        tiff.extend_from_slice(&42u16.to_le_bytes());
        tiff.extend_from_slice(&8u32.to_le_bytes()); // premier IFD a 8
        tiff.extend_from_slice(&1u16.to_le_bytes()); // 1 entree
        tiff.extend_from_slice(&0x9003u16.to_le_bytes());
        tiff.extend_from_slice(&2u16.to_le_bytes()); // ASCII
        tiff.extend_from_slice(&20u32.to_le_bytes()); // 20 octets
        let valeur = b"2026:11:03 14:05:00\0";
        let offset = (8 + 2 + 12 + 4 + valeur.len()) as u32;
        tiff.extend_from_slice(&offset.to_le_bytes());
        tiff.extend_from_slice(&0u32.to_le_bytes()); // IFD suivant
        tiff.extend_from_slice(valeur);
        let mut chunk = Vec::new();
        chunk.extend_from_slice(&(tiff.len() as u32).to_be_bytes());
        chunk.extend_from_slice(b"eXIf");
        chunk.extend_from_slice(&tiff);
        chunk.extend_from_slice(&[0u8; 4]);
        let pos = octets.len() - 12;
        octets.splice(pos..pos, chunk);
        let meta = extraire(&octets).expect("png avec exif");
        assert!(meta.exif_present);
        assert_eq!(meta.valeur("DateTimeOriginal"), Some("2026:11:03 14:05:00"));
    }
}
