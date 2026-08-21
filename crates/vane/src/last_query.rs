use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::cas::Cas;
use crate::error::VaneCliError;
use crate::fsutil::atomic_write;
use crate::live::LiveSet;
use crate::search::{parse_doc_id, read_by_id};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CachedHit {
    pub id: String,
    pub path: String,
    pub root: String,
    pub score: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LastQuery {
    pub query: String,
    pub at: u64,
    /// None = fused --all scope.
    pub scope_root: Option<String>,
    #[serde(default)]
    pub hits: Vec<CachedHit>,
}

pub struct ReadOutcome {
    pub hit: CachedHit,
    pub chunk_index: u32,
    pub text: String,
}

#[derive(Debug, PartialEq)]
pub enum ReadError {
    OutOfRange { n: usize, k: usize },
    Empty,
    Stale { n: usize },
}

pub fn last_query_path(home: &Path) -> PathBuf {
    home.join("run").join("last_query.json")
}

pub fn save_last_query(home: &Path, q: &LastQuery) -> Result<(), VaneCliError> {
    let payload = serde_json::to_vec_pretty(q)
        .map_err(|e| VaneCliError::new(format!("serialize last_query.json: {e}")))?;
    atomic_write(&last_query_path(home), &payload, "last_query.json")
}

/// Missing or corrupt cache is "no cache", never an error (spec §7.2).
pub fn load_last_query(home: &Path) -> Option<LastQuery> {
    let bytes = std::fs::read(last_query_path(home)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// 1-based hit lookup shared by chunk and --file reads.
pub fn hit_at(q: &LastQuery, n: usize) -> Result<&CachedHit, ReadError> {
    if q.hits.is_empty() {
        return Err(ReadError::Empty);
    }
    if n == 0 || n > q.hits.len() {
        return Err(ReadError::OutOfRange { n, k: q.hits.len() });
    }
    Ok(&q.hits[n - 1])
}

/// Resolve hit `n` (1-based) of `q` to its chunk text via LiveSet + CAS.
/// No daemon involved (spec §2.4).
pub fn read_outcome(home: &Path, q: &LastQuery, n: usize) -> Result<ReadOutcome, ReadError> {
    let hit = hit_at(q, n)?.clone();
    let (pid, _, chunk_index) = parse_doc_id(&hit.id).map_err(|_| ReadError::Stale { n })?;
    let live = LiveSet::load_for_project(home, &pid).map_err(|_| ReadError::Stale { n })?;
    let cas = Cas::new(home.join("rag").join("cas"));
    let chunk = read_by_id(&cas, &live, "", Path::new(&hit.root), &hit.id)
        .map_err(|_| ReadError::Stale { n })?;
    Ok(ReadOutcome {
        hit,
        chunk_index,
        text: chunk.text,
    })
}
