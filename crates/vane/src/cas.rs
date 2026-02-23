use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::error::VaneCliError;
use crate::extract::CanonicalDoc;

pub struct Cas {
    root: PathBuf,
}

impl Cas {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn get_extract(&self, key: &str) -> Option<Vec<CanonicalDoc>> {
        let bytes = fs::read(self.extract_docs_path(key)).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    pub fn put_extract(&self, key: &str, docs: &[CanonicalDoc]) -> Result<(), VaneCliError> {
        let dir = self.extract_dir(key);
        fs::create_dir_all(&dir).map_err(|e| io_err("create extract cas dir", &dir, e))?;
        let payload = serde_json::to_vec(docs)
            .map_err(|e| VaneCliError::new(format!("serialize extract cas: {e}")))?;
        write_key_name(&dir, key)?;
        atomic_write(&self.extract_docs_path(key), &payload)
    }

    pub fn get_embed(&self, key: &str) -> Option<Vec<f32>> {
        let bytes = fs::read(self.embed_vector_path(key)).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    pub fn put_embed(&self, key: &str, v: &[f32]) -> Result<(), VaneCliError> {
        let dir = self.embed_dir(key);
        fs::create_dir_all(&dir).map_err(|e| io_err("create embed cas dir", &dir, e))?;
        let payload = serde_json::to_vec(v)
            .map_err(|e| VaneCliError::new(format!("serialize embed cas: {e}")))?;
        write_key_name(&dir, key)?;
        atomic_write(&self.embed_vector_path(key), &payload)
    }

    pub fn touch(&self, extract_key: &str, embed_keys: &[String], now: u64) {
        let _ = self.write_last_seen(extract_key, true, now);
        let _ = self.write_embed_keys(extract_key, embed_keys);
        for key in embed_keys {
            let _ = self.write_last_seen(key, false, now);
        }
    }

    pub fn last_seen(&self, key: &str) -> Option<u64> {
        read_last_seen(&self.extract_dir(key)).or_else(|| read_last_seen(&self.embed_dir(key)))
    }

    pub fn stored_embed_keys(&self, extract_key: &str) -> Vec<String> {
        let bytes = fs::read(self.extract_dir(extract_key).join("embed_keys.json")).ok();
        bytes
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn list_extract_keys(&self) -> Vec<String> {
        list_cas_keys(&self.root.join("extract"))
    }

    pub fn list_embed_keys(&self) -> Vec<String> {
        list_cas_keys(&self.root.join("embed"))
    }

    pub fn delete_extract(&self, key: &str) -> Result<(), VaneCliError> {
        remove_cas_dir(&self.extract_dir(key))
    }

    pub fn delete_embed(&self, key: &str) -> Result<(), VaneCliError> {
        remove_cas_dir(&self.embed_dir(key))
    }

    fn write_last_seen(&self, key: &str, extract: bool, now: u64) -> Result<(), VaneCliError> {
        let dir = if extract {
            self.extract_dir(key)
        } else {
            self.embed_dir(key)
        };
        fs::create_dir_all(&dir).map_err(|e| io_err("create cas dir", &dir, e))?;
        write_key_name(&dir, key)?;
        atomic_write(&dir.join("last_seen"), now.to_string().as_bytes())
    }

    fn write_embed_keys(
        &self,
        extract_key: &str,
        embed_keys: &[String],
    ) -> Result<(), VaneCliError> {
        let dir = self.extract_dir(extract_key);
        fs::create_dir_all(&dir).map_err(|e| io_err("create extract cas dir", &dir, e))?;
        let payload = serde_json::to_vec(embed_keys)
            .map_err(|e| VaneCliError::new(format!("serialize embed_keys: {e}")))?;
        atomic_write(&dir.join("embed_keys.json"), &payload)
    }

    fn extract_dir(&self, key: &str) -> PathBuf {
        self.root.join("extract").join(safe_key(key))
    }

    fn embed_dir(&self, key: &str) -> PathBuf {
        self.root.join("embed").join(safe_key(key))
    }

    fn extract_docs_path(&self, key: &str) -> PathBuf {
        self.extract_dir(key).join("docs.json")
    }

    fn embed_vector_path(&self, key: &str) -> PathBuf {
        self.embed_dir(key).join("vector.json")
    }
}

/// SHA-256 hex of `file_sha256 + extractor + extractor_ver + chunk_strategy_id`.
pub fn extract_key(
    file_sha256: &str,
    extractor: &str,
    extractor_ver: &str,
    chunk_strategy_id: &str,
) -> String {
    let mut h = Sha256::new();
    h.update(file_sha256.as_bytes());
    h.update([0u8]);
    h.update(extractor.as_bytes());
    h.update([0u8]);
    h.update(extractor_ver.as_bytes());
    h.update([0u8]);
    h.update(chunk_strategy_id.as_bytes());
    hex(&h.finalize())
}

/// SHA-256 hex of `chunk.text` UTF-8 + embed_model_id.
pub fn embed_key(chunk_text: &str, embed_model_id: &str) -> String {
    let mut h = Sha256::new();
    h.update(chunk_text.as_bytes());
    h.update([0u8]);
    h.update(embed_model_id.as_bytes());
    hex(&h.finalize())
}

fn read_last_seen(dir: &Path) -> Option<u64> {
    let s = fs::read_to_string(dir.join("last_seen")).ok()?;
    s.trim().parse().ok()
}

fn list_cas_keys(dir: &Path) -> Vec<String> {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        let recorded = fs::read_to_string(path.join("key"))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        out.push(recorded.unwrap_or(name));
    }
    out
}

fn write_key_name(dir: &Path, key: &str) -> Result<(), VaneCliError> {
    atomic_write(&dir.join("key"), key.as_bytes())
}

fn remove_cas_dir(dir: &Path) -> Result<(), VaneCliError> {
    if dir.exists() {
        fs::remove_dir_all(dir).map_err(|e| io_err("delete cas object", dir, e))?;
    }
    Ok(())
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), VaneCliError> {
    let dir = path
        .parent()
        .ok_or_else(|| VaneCliError::new(format!("cas path has no parent: {}", path.display())))?;
    fs::create_dir_all(dir).map_err(|e| io_err("create cas parent", dir, e))?;
    let tmp = dir.join(format!(
        ".{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("cas")
    ));
    {
        let mut f = File::create(&tmp).map_err(|e| io_err("create cas temp", &tmp, e))?;
        f.write_all(bytes)
            .map_err(|e| io_err("write cas temp", &tmp, e))?;
        f.sync_all().map_err(|e| io_err("sync cas temp", &tmp, e))?;
    }
    fs::rename(&tmp, path).map_err(|e| {
        VaneCliError::new(format!(
            "rename {} -> {}: {e}",
            tmp.display(),
            path.display()
        ))
    })
}

fn io_err(op: &str, path: &Path, err: std::io::Error) -> VaneCliError {
    VaneCliError::new(format!("{op} {}: {err}", path.display()))
}

fn safe_key(key: &str) -> String {
    key.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn hex(digest: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(digest.len() * 2);
    for &b in digest {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}
