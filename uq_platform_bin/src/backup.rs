// Sauvegarde SQLite automatique : VACUUM INTO au demarrage puis toutes les 6h.
// Dossier : UQ_BACKUP_DIR (defaut ./backups). Garde les 7 dernieres.
use std::path::PathBuf;
use std::time::Duration;

fn dossier() -> PathBuf {
    std::env::var("UQ_BACKUP_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("backups"))
}

fn nom_fichier() -> String {
    let t = chrono::Utc::now().format("%Y-%m-%d_%H%M%S");
    format!("uq_backup_{}.db", t)
}

fn faire_backup(db_path: &str) -> Result<PathBuf, String> {
    let dir = dossier();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let dest = dir.join(nom_fichier());
    let src = rusqlite::Connection::open(db_path).map_err(|e| e.to_string())?;
    src.execute_batch(&format!(
        "VACUUM INTO '{}'",
        dest.display().to_string().replace('\'', "''")
    ))
    .map_err(|e| e.to_string())?;
    // Garde les 7 plus recentes.
    let mut fichiers: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension().map(|x| x == "db").unwrap_or(false)
                && p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("uq_backup_")
        })
        .collect();
    fichiers.sort();
    while fichiers.len() > 7 {
        let _ = std::fs::remove_file(fichiers.remove(0));
    }
    Ok(dest)
}

pub fn demarrer(db_path: String) {
    tokio::spawn(async move {
        // Au demarrage puis toutes les 6h.
        let mut interval = tokio::time::interval(Duration::from_secs(6 * 3600));
        loop {
            interval.tick().await;
            match faire_backup(&db_path) {
                Ok(p) => eprintln!("[BACKUP] {}", p.display()),
                Err(e) => eprintln!("[BACKUP] erreur : {}", e),
            }
        }
    });
}
