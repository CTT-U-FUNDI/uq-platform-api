// Metriques image pures Rust — port de _lumiere/_netnete/_reflets (vision_server.py).
// Entree : image RGB decodee (w*h*3). Sortie : scores normalises.

/// Luminance moyenne 0..255 -> 0..100.
pub fn lumiere(rgb: &[u8], w: usize, h: usize) -> f32 {
    if rgb.is_empty() || w == 0 { return 0.0; }
    let mut sum = 0u64;
    let n = (w * h).min(rgb.len() / 3);
    for i in 0..n {
        let r = rgb[i * 3] as u32; let g = rgb[i * 3 + 1] as u32; let b = rgb[i * 3 + 2] as u32;
        sum += ((77 * r + 150 * g + 29 * b) >> 8) as u64;
    }
    (sum as f32 / n as f32 * 100.0 / 255.0 * 100.0).round() / 100.0
}

/// Nettete : variance du Laplacien sur la luminance (echantillonne 1 px/4).
pub fn nettete(rgb: &[u8], w: usize, h: usize) -> f32 {
    if w < 3 || h < 3 { return 0.0; }
    let lum = |x: usize, y: usize| -> f32 {
        let i = (y * w + x) * 3;
        (77.0 * rgb[i] as f32 + 150.0 * rgb[i + 1] as f32 + 29.0 * rgb[i + 2] as f32) / 256.0
    };
    let mut vals = Vec::with_capacity((w / 4) * (h / 4));
    let mut y = 2;
    while y + 2 < h {
        let mut x = 2;
        while x + 2 < w {
            let lap = 4.0 * lum(x, y)
                - lum(x - 1, y) - lum(x + 1, y)
                - lum(x, y - 1) - lum(x, y + 1);
            vals.push(lap);
            x += 4;
        }
        y += 4;
    }
    if vals.is_empty() { return 0.0; }
    let mean = vals.iter().sum::<f32>() / vals.len() as f32;
    let var = vals.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / vals.len() as f32;
    var.sqrt().round()
}

/// Reflets : ratio de pixels quasi-blancs (lum > 250).
pub fn reflets(rgb: &[u8], w: usize, h: usize) -> f32 {
    if rgb.is_empty() { return 0.0; }
    let n = (w * h).min(rgb.len() / 3);
    let mut blancs = 0u64;
    for i in 0..n {
        if rgb[i * 3] > 250 && rgb[i * 3 + 1] > 250 && rgb[i * 3 + 2] > 250 { blancs += 1; }
    }
    (blancs as f32 / n as f32 * 10000.0).round() / 10000.0
}

/// Variantes niveaux de gris (1 octet/pixel) pour les analyses document.
pub fn lumiere_gray(g: &[u8], w: usize, h: usize) -> f32 { lumiere_rgb_from_gray(g, w, h) }
pub fn lumiere_rgb_from_gray(g: &[u8], w: usize, h: usize) -> f32 {
    if g.is_empty() || w == 0 { return 0.0; }
    let n = (w * h).min(g.len());
    let sum: u64 = g[..n].iter().map(|&v| v as u64).sum();
    (sum as f32 / n as f32 * 100.0 / 255.0).round() / 100.0
}
pub fn nettete_gray(g: &[u8], w: usize, h: usize) -> f32 {
    if w < 3 || h < 3 { return 0.0; }
    let lum = |x: usize, y: usize| g[y * w + x] as f32;
    let mut vals = Vec::with_capacity((w / 4) * (h / 4));
    let mut y = 2;
    while y + 2 < h {
        let mut x = 2;
        while x + 2 < w {
            vals.push(4.0 * lum(x, y) - lum(x-1,y) - lum(x+1,y) - lum(x,y-1) - lum(x,y+1));
            x += 4;
        }
        y += 4;
    }
    if vals.is_empty() { return 0.0; }
    let mean = vals.iter().sum::<f32>() / vals.len() as f32;
    (vals.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / vals.len() as f32).sqrt().round()
}
pub fn reflets_gray(g: &[u8], w: usize, h: usize) -> f32 {
    if g.is_empty() { return 0.0; }
    let n = (w * h).min(g.len());
    let blancs = g[..n].iter().filter(|&&v| v > 250).count() as f32;
    (blancs / n as f32 * 10000.0).round() / 10000.0
}
// alias utilises par proto.rs
pub use lumiere_gray as lumiere_rgb;
