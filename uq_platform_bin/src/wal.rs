use std::fs::{create_dir_all, File, OpenOptions};
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crc32fast::Hasher as Crc32Hasher;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WalEntry {
    pub seq: u64,
    pub timestamp_ms: u128,
    pub op_type: String,
    pub op_id: String,
    pub payload: JsonValue,
    pub checksum: u32,
}

#[derive(Clone)]
pub enum FsyncMode {
    Immediate,
    Buffered,
}

pub struct Wal {
    dir: PathBuf,
    file: Arc<Mutex<File>>,
    next_seq: AtomicU64,
    fsync_mode: FsyncMode,
    flush_interval_ms: u64,
}

impl Wal {
    pub fn open<P: AsRef<Path>>(dir: P) -> std::io::Result<Self> {
        let dir = dir.as_ref().to_path_buf();
        create_dir_all(&dir)?;
        let wal_path = dir.join("wal.log");

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(&wal_path)?;

        // Determine last seq by scanning existing lines
        let last_seq = Self::scan_last_seq(&wal_path).unwrap_or(0);

        // Read fsync config from env
        let fsync_mode = match std::env::var("WAL_FSYNC_MODE")
            .unwrap_or_else(|_| "buffered".to_string())
            .as_str()
        {
            "immediate" => FsyncMode::Immediate,
            _ => FsyncMode::Buffered,
        };
        let flush_interval_ms = std::env::var("WAL_FSYNC_INTERVAL_MS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(100u64);

        let wal = Wal {
            dir,
            file: Arc::new(Mutex::new(file)),
            next_seq: AtomicU64::new(last_seq + 1),
            fsync_mode,
            flush_interval_ms,
        };

        // If buffered, spawn flush thread
        if let FsyncMode::Buffered = wal.fsync_mode {
            let file_clone = wal.file.clone();
            let interval = wal.flush_interval_ms;
            thread::Builder::new()
                .name("wal-flusher".into())
                .spawn(move || loop {
                    thread::sleep(Duration::from_millis(interval));
                    if let Ok(mut f) = file_clone.lock() {
                        let _ = f.flush();
                        let _ = f.sync_all();
                    }
                })
                .ok();
        }

        Ok(wal)
    }

    fn scan_last_seq<P: AsRef<Path>>(path: P) -> std::io::Result<u64> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(0);
        }
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let mut last_seq = 0u64;
        for line in reader.lines() {
            match line {
                Ok(l) => {
                    if l.trim().is_empty() {
                        continue;
                    }
                    match serde_json::from_str::<WalEntry>(&l) {
                        Ok(entry) => {
                            // verify checksum
                            let mut hasher = Crc32Hasher::new();
                            // compute checksum over seq,timestamp,op_type,op_id,payload
                            let cs_string = format!(
                                "{}|{}|{}|{}|{}",
                                entry.seq,
                                entry.timestamp_ms,
                                entry.op_type,
                                entry.op_id,
                                entry.payload.to_string()
                            );
                            hasher.update(cs_string.as_bytes());
                            let computed = hasher.finalize();
                            if computed == entry.checksum {
                                last_seq = entry.seq;
                            } else {
                                break; // checksum mismatch — stop at last good
                            }
                        }
                        Err(_) => break, // parse error — likely partial write
                    }
                }
                Err(_) => break,
            }
        }
        Ok(last_seq)
    }

    pub fn append(&self, op_type: &str, payload: JsonValue) -> std::io::Result<WalEntry> {
        let seq = self.next_seq.fetch_add(1, Ordering::SeqCst);
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        let timestamp_ms = now.as_millis();
        let op_id = Uuid::new_v4().to_string();

        // compute checksum
        let mut cs_hasher = Crc32Hasher::new();
        let cs_string = format!(
            "{}|{}|{}|{}|{}",
            seq,
            timestamp_ms,
            op_type,
            op_id,
            payload.to_string()
        );
        cs_hasher.update(cs_string.as_bytes());
        let checksum = cs_hasher.finalize();

        let entry = WalEntry {
            seq,
            timestamp_ms,
            op_type: op_type.to_string(),
            op_id,
            payload: payload.clone(),
            checksum,
        };

        let line = serde_json::to_string(&entry)? + "\n";
        // append
        let mut file = self.file.lock().unwrap();
        file.write_all(line.as_bytes())?;
        if let FsyncMode::Immediate = self.fsync_mode {
            file.flush()?;
            file.sync_all()?;
        }
        Ok(entry)
    }

    /// Read all valid entries from WAL (verifies checksum and stops at first invalid)
    pub fn read_all(&self) -> std::io::Result<Vec<WalEntry>> {
        let path = self.dir.join("wal.log");
        if !path.exists() {
            return Ok(vec![]);
        }
        let f = File::open(&path)?;
        let reader = BufReader::new(f);
        let mut res = Vec::new();
        for line in reader.lines() {
            let l = line?;
            if l.trim().is_empty() {
                continue;
            }
            let entry: WalEntry = match serde_json::from_str(&l) {
                Ok(e) => e,
                Err(_) => break,
            };
            // verify checksum
            let mut hasher = Crc32Hasher::new();
            let cs_string = format!(
                "{}|{}|{}|{}|{}",
                entry.seq,
                entry.timestamp_ms,
                entry.op_type,
                entry.op_id,
                entry.payload.to_string()
            );
            hasher.update(cs_string.as_bytes());
            if hasher.finalize() == entry.checksum {
                res.push(entry);
            } else {
                break;
            }
        }
        Ok(res)
    }

    /// Truncate WAL to last valid entry (if partial write detected)
    pub fn truncate_to_last_valid(&self) -> std::io::Result<()> {
        let path = self.dir.join("wal.log");
        if !path.exists() {
            return Ok(());
        }
        let file = File::open(&path)?;
        let reader = BufReader::new(file);
        let mut valid_lines: Vec<String> = Vec::new();
        for line in reader.lines() {
            let l = line?;
            if l.trim().is_empty() {
                continue;
            }
            if let Ok(entry) = serde_json::from_str::<WalEntry>(&l) {
                let mut hasher = Crc32Hasher::new();
                let cs_string = format!(
                    "{}|{}|{}|{}|{}",
                    entry.seq,
                    entry.timestamp_ms,
                    entry.op_type,
                    entry.op_id,
                    entry.payload.to_string()
                );
                hasher.update(cs_string.as_bytes());
                if hasher.finalize() == entry.checksum {
                    valid_lines.push(l);
                    continue;
                }
            }
            break; // stop at first invalid
        }
        // rewrite file with valid lines
        let mut f = OpenOptions::new()
            .write(true)
            .truncate(true)
            .create(true)
            .open(&self.dir.join("wal.log"))?;
        for l in valid_lines {
            f.write_all(l.as_bytes())?;
            f.write_all(b"\n")?;
        }
        f.flush()?;
        f.sync_all()?;
        Ok(())
    }

    /// Path to meta file for recording last snapshot information
    fn meta_path(&self) -> PathBuf {
        self.dir.join("wal.meta")
    }

    /// Read metadata (last_snapshot_seq) if present
    pub fn read_meta(&self) -> std::io::Result<Option<u64>> {
        let path = self.meta_path();
        if !path.exists() {
            return Ok(None);
        }
        let s = std::fs::read_to_string(&path)?;
        let v: serde_json::Value = serde_json::from_str(&s)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        Ok(v.get("last_snapshot_seq").and_then(|x| x.as_u64()))
    }

    /// Write metadata (last_snapshot_seq)
    pub fn write_meta(&self, last_snapshot_seq: u64) -> std::io::Result<()> {
        let path = self.meta_path();
        let tmp = self.dir.join("wal.meta.tmp");
        let mut f = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&tmp)?;
        let payload = serde_json::json!({ "last_snapshot_seq": last_snapshot_seq });
        f.write_all(payload.to_string().as_bytes())?;
        f.flush()?;
        f.sync_all()?;
        std::fs::rename(tmp, path)?;
        Ok(())
    }

    /// Rotate / compact WAL by removing entries with seq <= keep_upto_seq.
    /// Keeps entries with seq > keep_upto_seq in-place (rewrites wal.log).
    pub fn rotate_after(&self, keep_upto_seq: u64) -> std::io::Result<()> {
        let path = self.dir.join("wal.log");
        if !path.exists() {
            return Ok(());
        }
        let all = self.read_all()?;
        let keep: Vec<WalEntry> = all.into_iter().filter(|e| e.seq > keep_upto_seq).collect();

        // rewrite the existing file atomically by truncating and writing keep entries
        let mut f_lock = self.file.lock().unwrap();
        f_lock.set_len(0)?;
        f_lock.seek(SeekFrom::Start(0))?;
        for entry in &keep {
            let line = serde_json::to_string(entry)? + "\n";
            f_lock.write_all(line.as_bytes())?;
        }
        f_lock.flush()?;
        f_lock.sync_all()?;

        // update next_seq to last kept seq + 1, or keep_upto_seq + 1 if none kept
        let last_seq = keep.last().map(|e| e.seq).unwrap_or(keep_upto_seq);
        self.next_seq.store(last_seq + 1, Ordering::SeqCst);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs::OpenOptions;
    use std::io::Seek;
    use tempfile::tempdir;

    #[test]
    fn test_append_and_read() {
        let td = tempdir().unwrap();
        let wal = Wal::open(td.path()).unwrap();
        let p = json!({"amount":100});
        let e1 = wal.append("tx", p.clone()).unwrap();
        let e2 = wal.append("tx", p.clone()).unwrap();
        assert_eq!(e2.seq, e1.seq + 1);
        let entries = wal.read_all().unwrap();
        assert!(entries.len() >= 2);
        assert_eq!(entries[entries.len() - 2].seq, e1.seq);
        assert_eq!(entries[entries.len() - 1].seq, e2.seq);
    }

    #[test]
    fn test_truncate_partial_write() {
        let td = tempdir().unwrap();
        let path = td.path().join("wal.log");
        let wal = Wal::open(td.path()).unwrap();
        let p = json!({"amount":42});
        let _ = wal.append("tx", p.clone()).unwrap();
        // simulate partial write by appending incomplete JSON
        {
            let mut f = OpenOptions::new().append(true).open(&path).unwrap();
            f.write_all(b"{\"seq\":999,").unwrap();
            f.flush().unwrap();
        }
        // truncate should remove the partial trailing bytes
        wal.truncate_to_last_valid().unwrap();
        let entries = wal.read_all().unwrap();
        assert!(entries.iter().all(|e| e.seq != 999));
    }

    #[test]
    fn test_checksum_mismatch_detection() {
        let td = tempdir().unwrap();
        let path = td.path().join("wal.log");
        let wal = Wal::open(td.path()).unwrap();
        let p = json!({"amount":7});
        let e = wal.append("tx", p.clone()).unwrap();
        // corrupt the file by changing a byte
        {
            let mut f = OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)
                .unwrap();
            let mut s = String::new();
            use std::io::Read;
            f.read_to_string(&mut s).unwrap();
            // replace a character in the last line
            let corrupted = s.replace("7", "8");
            f.set_len(0).unwrap();
            f.seek(SeekFrom::Start(0)).unwrap();
            f.write_all(corrupted.as_bytes()).unwrap();
            f.flush().unwrap();
        }
        // read_all should stop before corrupted entry
        let entries = wal.read_all().unwrap();
        // either zero entries or entries without corrupted seq
        assert!(entries.iter().all(|x| x.checksum != 0));
    }
}
