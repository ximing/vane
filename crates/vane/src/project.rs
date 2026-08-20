use std::path::{Path, PathBuf};

use crate::error::VaneCliError;

/// SHA-256 of the canonical root's UTF-8 bytes, hex-encoded, first 16 characters.
pub fn project_id(canonical_root: &Path) -> String {
    use sha2::{Digest, Sha256};
    let raw = canonical_root.to_string_lossy();
    let digest = Sha256::digest(raw.as_bytes());
    let mut out = String::with_capacity(16);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for &b in digest.iter().take(8) {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

/// Walk `start` and its parents looking for `.vane.toml`.
pub fn find_vane_toml_dir(start: &Path) -> Option<PathBuf> {
    let mut cur = if start.is_file() {
        start.parent()?.to_path_buf()
    } else {
        start.to_path_buf()
    };
    loop {
        if cur.join(".vane.toml").is_file() {
            return Some(cur);
        }
        if !cur.pop() {
            return None;
        }
    }
}

/// Where `vane query` should search when the user does not pass `--root`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryScope {
    All,
    Root(PathBuf),
}

/// `.vane.toml` walking up from cwd, else the longest registered prefix, else all roots.
pub fn resolve_query_scope(cwd: &Path, registered: &[PathBuf], force_global: bool) -> QueryScope {
    if force_global {
        return QueryScope::All;
    }
    if let Some(dir) = find_vane_toml_dir(cwd) {
        return QueryScope::Root(dir);
    }
    match find_current_root(cwd, registered) {
        Some(root) => QueryScope::Root(root),
        None => QueryScope::All,
    }
}

/// Longest registered root that is a path prefix of `cwd`.
pub fn find_current_root(cwd: &Path, roots: &[PathBuf]) -> Option<PathBuf> {
    roots
        .iter()
        .filter(|root| is_path_prefix(root, cwd))
        .max_by_key(|root| root.components().count())
        .cloned()
}

/// Refuse equal, nested, or wrapping roots. `/a` vs `/ab` is allowed.
pub fn reject_nested(existing: &[PathBuf], new: &Path) -> Result<(), VaneCliError> {
    for old in existing {
        if is_path_prefix(old, new) {
            return Err(VaneCliError::new(format!(
                "path {} is inside existing root {}",
                new.display(),
                old.display()
            )));
        }
        if is_path_prefix(new, old) {
            return Err(VaneCliError::new(format!(
                "path {} would contain existing root {}",
                new.display(),
                old.display()
            )));
        }
    }
    Ok(())
}

fn is_path_prefix(prefix: &Path, path: &Path) -> bool {
    let mut path_c = path.components();
    for c in prefix.components() {
        match path_c.next() {
            Some(pc) if pc == c => {}
            _ => return false,
        }
    }
    true
}
