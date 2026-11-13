// ====================
// 💾 STORAGE KYC — Sauvegarde/Chargement
// ====================

use std::fs;
use std::path::Path;
use crate::kyc::queue::KycQueue;
use crate::kyc::training::TrainingRecord;

const QUEUE_FILE: &str = "data/kyc/queue.json";
const TRAINING_FILE: &str = "data/kyc/training.json";
const PHOTOS_DIR: &str = "data/kyc/photos";
const VIDEOS_DIR: &str = "data/kyc/videos";

pub struct KycStore;

/// `data/kyc/*` n'existe qu'apres la premiere sauvegarde. Ecrire sans creer le
/// repertoire ferait echouer la premiere soumission en silence : la route
/// repondrait « dossier soumis » alors que rien n'est enregistre.
fn ecrire<T: AsRef<[u8]>>(chemin: &str, contenu: T) -> Result<(), String> {
    if let Some(repertoire) = Path::new(chemin).parent() {
        fs::create_dir_all(repertoire).map_err(|e| e.to_string())?;
    }
    fs::write(chemin, contenu).map_err(|e| e.to_string())
}

impl KycStore {
    pub fn init() -> Result<(), String> {
        fs::create_dir_all("data/kyc").map_err(|e| e.to_string())?;
        fs::create_dir_all(PHOTOS_DIR).map_err(|e| e.to_string())?;
        fs::create_dir_all(VIDEOS_DIR).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn load_queue() -> KycQueue {
        if Path::new(QUEUE_FILE).exists() {
            if let Ok(content) = fs::read_to_string(QUEUE_FILE) {
                if let Ok(queue) = serde_json::from_str::<KycQueue>(&content) {
                    return queue;
                }
            }
        }
        KycQueue::new()
    }

    pub fn save_queue(queue: &KycQueue) -> Result<(), String> {
        let json = serde_json::to_string_pretty(queue).map_err(|e| e.to_string())?;
        ecrire(QUEUE_FILE, json)
    }

    pub fn save_photo(uq_number: &str, data: &[u8]) -> Result<String, String> {
        let filename = format!("{}/{}_photo.jpg", PHOTOS_DIR, uq_number.replace("-", "_").to_lowercase());
        ecrire(&filename, data)?;
        Ok(filename)
    }

    pub fn save_video(uq_number: &str, data: &[u8]) -> Result<String, String> {
        let filename = format!("{}/{}_video.mp4", VIDEOS_DIR, uq_number.replace("-", "_").to_lowercase());
        ecrire(&filename, data)?;
        Ok(filename)
    }

    /// Selfie guide du parcours KYC : meme repertoire que la photo, nom distinct.
    pub fn save_selfie(uq_number: &str, data: &[u8]) -> Result<String, String> {
        let filename = format!("{}/{}_selfie.jpg", PHOTOS_DIR, uq_number.replace("-", "_").to_lowercase());
        ecrire(&filename, data)?;
        Ok(filename)
    }

    pub fn save_training_record(record: &TrainingRecord) -> Result<(), String> {
        let mut records: Vec<TrainingRecord> = Vec::new();
        if Path::new(TRAINING_FILE).exists() {
            if let Ok(content) = fs::read_to_string(TRAINING_FILE) {
                records = serde_json::from_str(&content).unwrap_or_default();
            }
        }
        records.push(record.clone());
        let json = serde_json::to_string_pretty(&records).map_err(|e| e.to_string())?;
        ecrire(TRAINING_FILE, json)
    }
}

impl KycStore {
    /// Supprime un dossier ET ses fichiers apres decision
    pub fn supprimer_dossier(uq_number: &str) -> Result<(), String> {
        // 1. Supprimer la photo
        let photo = format!("{}/{}_photo.jpg",
            PHOTOS_DIR, uq_number.replace("-", "_").to_lowercase());
        let _ = fs::remove_file(&photo);

        // 2. Supprimer la video
        let video = format!("{}/{}_video.mp4",
            VIDEOS_DIR, uq_number.replace("-", "_").to_lowercase());
        let _ = fs::remove_file(&video);

        // 3. Supprimer de la file
        let mut queue = Self::load_queue();
        queue.dossiers.retain(|d| d.uq_number != uq_number);
        Self::save_queue(&queue)?;

        Ok(())
    }
}
