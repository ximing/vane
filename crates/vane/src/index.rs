use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use vane_core::api::{
    Collection, CollectionOptions, Db, Doc, Hit, OpenOptions, ScalarValue, SearchQuery,
};
use vane_core::persistence::AutoCommitConfig;
use vane_core::tokenizer::BuiltinTokenizer;
use vane_core::types::{FieldDef, Metric, ScalarKind, Schema, VaneError};
use vane_core::vfs::std_fs::StdFsVfs;

use crate::error::VaneCliError;
use crate::extract::CanonicalDoc;

const COLLECTION_NAME: &str = "docs";
const COMPACT_DELETE_THRESHOLD: u64 = 1000;
const COMPACT_DEAD_RATIO: f64 = 0.2;

pub struct ProjectIndex {
    db: Db,
    col: Collection,
    dim: u32,
    model_id: String,
    tokenizer_fallback: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embed_model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dim: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chunk_strategy_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokenizer_fallback: Option<String>,
    /// Base URL that produced the collection currently on disk (`db/` or `db.prev`).
    /// Query embedding restores this while a new overlay is waiting to swap (§7.4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embed_base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rebuild: Option<RebuildProgress>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reindex_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RebuildProgress {
    pub done: u64,
    pub total: u64,
}

impl ProjectState {
    pub fn load(path: &Path) -> Result<Self, VaneCliError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let bytes = fs::read(path).map_err(|e| io_err("read state.json", path, e))?;
        serde_json::from_slice(&bytes)
            .map_err(|e| VaneCliError::new(format!("parse {}: {e}", path.display())))
    }

    pub fn save_atomic(&self, path: &Path) -> Result<(), VaneCliError> {
        let payload = serde_json::to_vec_pretty(self)
            .map_err(|e| VaneCliError::new(format!("serialize state.json: {e}")))?;
        atomic_write(path, &payload)
    }
}

pub fn project_dir(home: &Path, project_id: &str) -> PathBuf {
    home.join("rag").join("projects").join(project_id)
}

pub fn project_db_path(home: &Path, project_id: &str) -> PathBuf {
    project_dir(home, project_id).join("db")
}

pub fn project_db_new_path(home: &Path, project_id: &str) -> PathBuf {
    project_dir(home, project_id).join("db.new")
}

pub fn project_db_prev_path(home: &Path, project_id: &str) -> PathBuf {
    project_dir(home, project_id).join("db.prev")
}

pub fn state_path(home: &Path, project_id: &str) -> PathBuf {
    project_dir(home, project_id).join("state.json")
}

/// `{project_id}:{rel_path}#{chunk_index}`
pub fn doc_id(project_id: &str, rel_path: &str, chunk_index: u32) -> String {
    format!("{project_id}:{rel_path}#{chunk_index}")
}

pub fn index_doc(
    project_id: &str,
    root: &str,
    doc: &CanonicalDoc,
    vector: Option<Vec<f32>>,
) -> Doc {
    let mut meta = HashMap::new();
    meta.insert("root".into(), ScalarValue::Keyword(root.into()));
    meta.insert("path".into(), ScalarValue::Keyword(doc.path.clone()));
    meta.insert(
        "modality".into(),
        ScalarValue::Keyword(doc.modality.clone()),
    );
    meta.insert(
        "extractor".into(),
        ScalarValue::Keyword(doc.extractor.clone()),
    );
    meta.insert(
        "chunk_index".into(),
        ScalarValue::Int(i64::from(doc.chunk_index)),
    );
    meta.insert("start_byte".into(), ScalarValue::Int(doc.start_byte as i64));
    meta.insert("end_byte".into(), ScalarValue::Int(doc.end_byte as i64));
    Doc {
        id: doc_id(project_id, &doc.path, doc.chunk_index),
        text: Some(doc.text.clone()),
        vector,
        meta: Some(meta),
    }
}

pub fn open_or_create(
    home: &Path,
    project_id: &str,
    dim: u32,
    model_id: &str,
) -> Result<ProjectIndex, VaneCliError> {
    let state_file = state_path(home, project_id);
    let mut state = ProjectState::load(&state_file)?;
    let prefer_cjk = state.tokenizer_fallback.as_deref() == Some("cjk_bigram");
    let idx = open_or_create_at(
        &project_db_path(home, project_id),
        dim,
        model_id,
        prefer_cjk,
    )?;
    state.embed_model_id = Some(model_id.to_string());
    state.dim = Some(dim);
    if idx.tokenizer_fallback {
        state.tokenizer_fallback = Some("cjk_bigram".into());
    }
    state.save_atomic(&state_file)?;
    Ok(idx)
}

/// Open or create a Vane db at `db_dir` without writing `state.json`.
pub fn open_or_create_at(
    db_dir: &Path,
    dim: u32,
    model_id: &str,
    prefer_cjk: bool,
) -> Result<ProjectIndex, VaneCliError> {
    fs::create_dir_all(db_dir).map_err(|e| io_err("create project db dir", db_dir, e))?;
    open_at(db_dir, dim, model_id, prefer_cjk)
}

/// Open the serving collection for search/read. Never creates `db/`.
///
/// Mid-swap (`db` → `db.prev`, `db.new` → `db`) a query must not `mkdir`
/// an empty `db/` — that would make `rename(db.new, db)` fail and skip
/// rollback. If `db/` is missing or does not match `dim`/`model_id`, retry
/// briefly and open `db.prev` when present.
pub fn open_existing(
    home: &Path,
    project_id: &str,
    dim: u32,
    model_id: &str,
    prefer_cjk: bool,
) -> Result<ProjectIndex, VaneCliError> {
    const ATTEMPTS: u32 = 8;
    const DELAY: Duration = Duration::from_millis(15);
    let db = project_db_path(home, project_id);
    let prev = project_db_prev_path(home, project_id);
    let mut last = VaneCliError::new(format!(
        "no existing collection at {} or {}",
        db.display(),
        prev.display()
    ));
    for i in 0..ATTEMPTS {
        if is_populated_dir(&db) {
            match open_at(&db, dim, model_id, prefer_cjk) {
                Ok(idx) => return Ok(idx),
                Err(e) => last = e,
            }
        }
        if is_populated_dir(&prev) {
            match open_at(&prev, dim, model_id, prefer_cjk) {
                Ok(idx) => return Ok(idx),
                Err(e) => last = e,
            }
        }
        if i + 1 < ATTEMPTS {
            thread::sleep(DELAY);
        }
    }
    Err(last)
}

fn is_populated_dir(path: &Path) -> bool {
    path.is_dir()
        && fs::read_dir(path)
            .ok()
            .is_some_and(|mut it| it.next().is_some())
}

fn open_at(
    db_dir: &Path,
    dim: u32,
    model_id: &str,
    prefer_cjk: bool,
) -> Result<ProjectIndex, VaneCliError> {
    let db_path = db_dir
        .to_str()
        .ok_or_else(|| VaneCliError::new(format!("db path is not utf-8: {}", db_dir.display())))?;

    let open_opts = OpenOptions {
        auto_commit: AutoCommitConfig::Off,
        ..OpenOptions::default()
    };
    let vfs = Arc::new(StdFsVfs::new());
    let db = Db::open(vfs, db_path, open_opts).map_err(core_err)?;

    let schema = docs_schema(dim)?;
    let (col, tokenizer_fallback) = if prefer_cjk {
        (
            open_collection(&db, schema, BuiltinTokenizer::CjkBigram).map_err(core_err)?,
            true,
        )
    } else {
        match open_collection(&db, schema.clone(), BuiltinTokenizer::Jieba) {
            Ok(col) => (col, false),
            Err(e) if is_tokenizer_retry(&e) => (
                open_collection(&db, schema, BuiltinTokenizer::CjkBigram).map_err(core_err)?,
                true,
            ),
            Err(e) => return Err(core_err(e)),
        }
    };

    Ok(ProjectIndex {
        db,
        col,
        dim,
        model_id: model_id.to_string(),
        tokenizer_fallback,
    })
}

/// `db` → `db.prev`, `db.new` → `db`. Leaves `db.prev` so the caller can
/// update `state.json` first, then delete prev (§7.4).
pub fn swap_new_db(home: &Path, project_id: &str) -> Result<(), VaneCliError> {
    let db = project_db_path(home, project_id);
    let new = project_db_new_path(home, project_id);
    let prev = project_db_prev_path(home, project_id);
    if !new.exists() {
        return Err(VaneCliError::new(format!(
            "rebuild swap missing {}",
            new.display()
        )));
    }
    if prev.exists() {
        fs::remove_dir_all(&prev).map_err(|e| io_err("remove leftover db.prev", &prev, e))?;
    }
    if db.exists() {
        fs::rename(&db, &prev).map_err(|e| {
            VaneCliError::new(format!(
                "rename {} -> {}: {e}",
                db.display(),
                prev.display()
            ))
        })?;
    }
    if let Err(e) = fs::rename(&new, &db) {
        if prev.exists() && !db.exists() {
            let _ = fs::rename(&prev, &db);
        }
        return Err(VaneCliError::new(format!(
            "rename {} -> {}: {e}",
            new.display(),
            db.display()
        )));
    }
    Ok(())
}

/// Delete `db.prev` after `state.json` has the new embed_model_id/dim.
pub fn remove_db_prev(home: &Path, project_id: &str) -> Result<(), VaneCliError> {
    let prev = project_db_prev_path(home, project_id);
    if prev.exists() {
        fs::remove_dir_all(&prev)
            .map_err(|e| io_err("remove db.prev after state update", &prev, e))?;
    }
    Ok(())
}

impl ProjectIndex {
    pub fn dim(&self) -> u32 {
        self.dim
    }

    pub fn model_id(&self) -> &str {
        &self.model_id
    }

    pub fn tokenizer_fallback(&self) -> bool {
        self.tokenizer_fallback
    }

    pub fn add_docs(&self, docs: &[Doc]) -> Result<(), VaneCliError> {
        self.col.add(docs).map(|_| ()).map_err(core_err)
    }

    pub fn delete_ids(&self, ids: &[String]) -> Result<u64, VaneCliError> {
        self.col.delete(ids).map_err(core_err)
    }

    pub fn flush(&self) -> Result<(), VaneCliError> {
        self.col.flush().map_err(core_err)
    }

    pub fn compact(&self) -> Result<(), VaneCliError> {
        self.col.compact().map_err(core_err)
    }

    pub fn maybe_compact(&self, deletes: u64, live: u64, dead: u64) -> Result<bool, VaneCliError> {
        if !should_compact(deletes, live, dead) {
            return Ok(false);
        }
        self.compact()?;
        Ok(true)
    }

    pub fn search(&self, query: &SearchQuery) -> Result<Vec<Hit>, VaneCliError> {
        self.col.search(query).map_err(core_err)
    }

    pub fn close(&self) -> Result<(), VaneCliError> {
        self.db.close().map_err(core_err)
    }
}

pub fn maybe_compact(
    idx: &ProjectIndex,
    deletes: u64,
    live: u64,
    dead: u64,
) -> Result<bool, VaneCliError> {
    idx.maybe_compact(deletes, live, dead)
}

pub fn should_compact(deletes: u64, live: u64, dead: u64) -> bool {
    if deletes >= COMPACT_DELETE_THRESHOLD {
        return true;
    }
    live > 0 && (dead as f64) / (live as f64) >= COMPACT_DEAD_RATIO
}

fn open_collection(
    db: &Db,
    schema: Schema,
    tokenizer: BuiltinTokenizer,
) -> Result<Collection, VaneError> {
    db.collection(
        COLLECTION_NAME,
        schema,
        CollectionOptions {
            tokenizer,
            user_dict: Vec::new(),
            auto_commit: AutoCommitConfig::Off,
        },
    )
}

fn docs_schema(dim: u32) -> Result<Schema, VaneCliError> {
    Schema::new(vec![
        ("body".into(), FieldDef::Text),
        (
            "embedding".into(),
            FieldDef::Vector {
                dim,
                metric: Metric::Cosine,
            },
        ),
        (
            "root".into(),
            FieldDef::Scalar {
                kind: ScalarKind::Keyword,
            },
        ),
        (
            "path".into(),
            FieldDef::Scalar {
                kind: ScalarKind::Keyword,
            },
        ),
        (
            "modality".into(),
            FieldDef::Scalar {
                kind: ScalarKind::Keyword,
            },
        ),
        (
            "extractor".into(),
            FieldDef::Scalar {
                kind: ScalarKind::Keyword,
            },
        ),
        (
            "chunk_index".into(),
            FieldDef::Scalar {
                kind: ScalarKind::Int,
            },
        ),
        (
            "start_byte".into(),
            FieldDef::Scalar {
                kind: ScalarKind::Int,
            },
        ),
        (
            "end_byte".into(),
            FieldDef::Scalar {
                kind: ScalarKind::Int,
            },
        ),
    ])
    .map_err(core_err)
}

fn is_tokenizer_retry(err: &VaneError) -> bool {
    matches!(
        err,
        VaneError::DictUnavailable(_) | VaneError::TokenizerMismatch(_)
    ) || (matches!(err, VaneError::Schema(_)) && format!("{err}").contains("tokenizer"))
}

fn core_err(err: VaneError) -> VaneCliError {
    VaneCliError::new(err.to_string())
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), VaneCliError> {
    let dir = path.parent().ok_or_else(|| {
        VaneCliError::new(format!("state.json path has no parent: {}", path.display()))
    })?;
    fs::create_dir_all(dir).map_err(|e| io_err("create state.json parent", dir, e))?;
    let tmp = dir.join(format!(
        "{}.tmp",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("state.json")
    ));
    {
        let mut f = File::create(&tmp).map_err(|e| io_err("create state.json temp", &tmp, e))?;
        f.write_all(bytes)
            .map_err(|e| io_err("write state.json temp", &tmp, e))?;
        f.sync_all()
            .map_err(|e| io_err("sync state.json temp", &tmp, e))?;
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
