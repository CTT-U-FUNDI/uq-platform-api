// Detection de visages YuNet (ONNX) — equivalent cv2.FaceDetectorYN.
// Sortie par visage : [x, y, w, h, x_re, y_re, x_le, y_le, x_nez, y_nez,
//                       x_bouche_r, y_bouche_r, x_bouche_l, y_bouche_l, score]

use ort::session::Session;
use std::sync::Mutex;
use ort::value::Tensor;

pub struct YuNet {
    session: Mutex<Session>,
    score_seuil: f32,
    nms_seuil: f32,
    top_k: usize,
}

impl YuNet {
    pub fn charger(chemin: &str) -> Result<Self, String> {
        let session = Session::builder()
            .map_err(|e| e.to_string())?
            .commit_from_file(chemin)
            .map_err(|e| format!("YuNet load {}: {}", chemin, e))?;
        Ok(Self { session: Mutex::new(session), score_seuil: 0.6, nms_seuil: 0.3, top_k: 5000 })
    }

    /// Detecte les visages dans une image RGB (largeur w, hauteur h).
    /// Retourne les visages tries par score decroissant.
    pub fn detecter(&self, rgb: &[u8], w: u32, h: u32) -> Result<Vec<Visage>, String> {
        // Blob NCHW : 1x3xHxW, normalise 0..1, RGB.
        let mut blob = vec![0f32; (w * h * 3) as usize];
        for i in 0..(w * h) as usize {
            blob[i] = rgb[i * 3] as f32 / 255.0;
            blob[(w * h) as usize + i] = rgb[i * 3 + 1] as f32 / 255.0;
            blob[2 * (w * h) as usize + i] = rgb[i * 3 + 2] as f32 / 255.0;
        }
        let shape = [1usize, 3, h as usize, w as usize];
        let input = Tensor::from_array(([1usize, 3, h as usize, w as usize], blob))
            .map_err(|e| e.to_string())?;
        let mut session = self.session.lock().map_err(|_| "mutex".to_string())?;
        let outputs = session.run(ort::inputs!["input" => input])
            .map_err(|e| e.to_string())?;
        let (vals, dims) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| e.to_string())?;
        // Sortie YuNet : [1, N, 15] ou [1, 15, N] selon le modele.
        let (n, stride) = if dims.len() == 3 && dims[1] < dims[2] {
            (dims[1], dims[2])
        } else {
            (dims[2], dims[1])
        };
        let mut visages = Vec::new();
        for i in 0..n.min(self.top_k) {
            let base = if dims[1] < dims[2] { i * 15 } else { i };
            let get = |c: usize| -> f32 {
                if dims[1] < dims[2] { vals[base + c] } else { vals[c * stride + i] }
            };
            let score = get(14);
            if score < self.score_seuil { continue; }
            let x = get(0); let y = get(1); let bw = get(2); let bh = get(3);
            if bw <= 0.0 || bh <= 0.0 { continue; }
            visages.push(Visage {
                x, y, w: bw, h: bh, score,
                oeil_droit: (get(4), get(5)),
                oeil_gauche: (get(6), get(7)),
                nez: (get(8), get(9)),
                bouche_r: (get(10), get(11)),
                bouche_l: (get(12), get(13)),
            });
        }
        // NMS simple par IoU + tri score.
        visages.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
        let mut gardes: Vec<Visage> = Vec::new();
        for v in visages {
            if gardes.iter().all(|g| iou(g, &v) < self.nms_seuil) {
                gardes.push(v);
            }
        }
        Ok(gardes)
    }
}

pub struct Visage {
    pub x: f32, pub y: f32, pub w: f32, pub h: f32, pub score: f32,
    pub oeil_droit: (f32, f32), pub oeil_gauche: (f32, f32),
    pub nez: (f32, f32), pub bouche_r: (f32, f32), pub bouche_l: (f32, f32),
}

fn iou(a: &Visage, b: &Visage) -> f32 {
    let x1 = a.x.max(b.x); let y1 = a.y.max(b.y);
    let x2 = (a.x + a.w).min(b.x + b.w); let y2 = (a.y + a.h).min(b.y + b.h);
    let inter = (x2 - x1).max(0.0) * (y2 - y1).max(0.0);
    let union = a.w * a.h + b.w * b.h - inter;
    if union <= 0.0 { 0.0 } else { inter / union }
}
