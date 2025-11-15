//! vane-wasm：浏览器 wasm deliverable 胶水 crate（SPEC §12.1）。
//!
//! M2-01：真实检索/管理 API 的 wasm-bindgen 胶水 + SIMD 探针占位。
//! 内部调 `vane_core::api`（薄壳 I-8，无检索逻辑）。
//! `vane_open` 用 `MemoryVfs`（内存 VFS，OPFS 在 M2-02）。
//!
//! 句柄 uint64 + 全局注册表（与 vane-ffi 同构），sync 函数（Worker 内同步）。
//! 参数/返回 JSON 序列化（SPEC §9.2 binding 薄壳原则）。
//!
//! 依赖 vane-core default features（不含 jieba/dict-zh——词典永不进 wasm，红线）。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use wasm_bindgen::prelude::*;

use vane_core::api::{
    Collection, CollectionOptions, Db, Doc, FusionSpec, Hit, OpenOptions, PersistenceMode,
    ReindexHandle, ScalarValue, SearchMode, SearchQuery,
};
use vane_core::persistence::AutoCommitConfig;
use vane_core::tokenizer::{BuiltinTokenizer, UserDictEntry};
use vane_core::types::{FieldDef, Metric, ScalarKind, Schema, VaneError};
use vane_core::vfs::memory::MemoryVfs;
use vane_core::vfs::Vfs;

#[cfg(feature = "worker")]
pub mod dict_loader;
pub mod simd_probe;
pub mod vfs;
#[cfg(feature = "worker")]
pub mod worker;

// =========================================================================
// 句柄注册表（与 vane-ffi 同构，wasm32 单线程，RwLock 即线程局部）
// =========================================================================

struct RegistryEntry {
    db: Option<Arc<Db>>,
    col: Option<Arc<Collection>>,
    #[allow(dead_code)]
    reindex: Option<ReindexHandle>,
}

impl RegistryEntry {
    fn new_db(db: Arc<Db>) -> Self {
        Self {
            db: Some(db),
            col: None,
            reindex: None,
        }
    }
    fn new_col(col: Arc<Collection>) -> Self {
        Self {
            db: None,
            col: Some(col),
            reindex: None,
        }
    }
    fn new_reindex(rh: ReindexHandle) -> Self {
        Self {
            db: None,
            col: None,
            reindex: Some(rh),
        }
    }
}

static REGISTRY: RwLock<Option<HashMap<u64, RegistryEntry>>> = RwLock::new(None);
static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);

fn alloc_handle(entry: RegistryEntry) -> Result<u64, VaneError> {
    let h = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
    let mut reg = REGISTRY
        .write()
        .map_err(|_| VaneError::Io("registry lock poisoned".into()))?;
    reg.get_or_insert_with(HashMap::new).insert(h, entry);
    Ok(h)
}

fn lookup_db(h: u64) -> Result<Option<Arc<Db>>, VaneError> {
    let reg = REGISTRY
        .read()
        .map_err(|_| VaneError::Io("registry lock poisoned".into()))?;
    Ok(reg
        .as_ref()
        .and_then(|m| m.get(&h))
        .and_then(|e| e.db.clone()))
}

fn lookup_col(h: u64) -> Result<Option<Arc<Collection>>, VaneError> {
    let reg = REGISTRY
        .read()
        .map_err(|_| VaneError::Io("registry lock poisoned".into()))?;
    Ok(reg
        .as_ref()
        .and_then(|m| m.get(&h))
        .and_then(|e| e.col.clone()))
}

fn remove_handle(h: u64) -> Result<bool, VaneError> {
    let mut reg = REGISTRY
        .write()
        .map_err(|_| VaneError::Io("registry lock poisoned".into()))?;
    Ok(reg
        .as_mut()
        .and_then(|m| m.remove(&h).map(|_| true))
        .unwrap_or(false))
}

// =========================================================================
// 错误转换：VaneError → JsValue（throw on JS side）
// =========================================================================

fn err_to_js(e: VaneError) -> JsValue {
    JsValue::from(format!("{}: {}", e.name(), e))
}

/// 把 VaneError 写入 JsValue 并返回（用于 Result 提前返回）。
type JsResult<T> = std::result::Result<T, JsValue>;

// =========================================================================
// JSON 解析辅助（与 vane-ffi convert 同构，I-8 薄壳）
// =========================================================================

fn parse_open_opts(v: &serde_json::Value) -> Result<OpenOptions, VaneError> {
    let persistence = match v.get("persistence").and_then(|v| v.as_str()) {
        Some("best-effort") => PersistenceMode::BestEffort,
        _ => PersistenceMode::Persistent,
    };
    let auto_commit = parse_auto_commit(v.get("autoCommit"))?;
    let page_cache_mb = v.get("pageCacheMb").and_then(|v| v.as_u64()).unwrap_or(32) as u32;
    Ok(OpenOptions {
        persistence,
        auto_commit,
        page_cache_mb,
    })
}

fn parse_auto_commit(v: Option<&serde_json::Value>) -> Result<AutoCommitConfig, VaneError> {
    match v {
        Some(serde_json::Value::String(s)) if s == "off" => Ok(AutoCommitConfig::Off),
        Some(serde_json::Value::String(_)) => Err(VaneError::InvalidArg(
            "autoCommit string must be 'off'".into(),
        )),
        Some(o) => Ok(AutoCommitConfig::On {
            interval_ms: o.get("intervalMs").and_then(|v| v.as_u64()).unwrap_or(1000) as u32,
            max_docs: o.get("maxDocs").and_then(|v| v.as_u64()).unwrap_or(1000) as u32,
        }),
        None => Ok(AutoCommitConfig::default()),
    }
}

fn parse_collection_opts(v: &serde_json::Value) -> Result<CollectionOptions, VaneError> {
    let tokenizer = match v.get("tokenizer").and_then(|v| v.as_str()) {
        Some("cjk_bigram") => BuiltinTokenizer::CjkBigram,
        Some("jieba") => BuiltinTokenizer::Jieba,
        _ => BuiltinTokenizer::Standard,
    };
    let user_dict = v
        .get("userDict")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().map(parse_dict_entry).collect::<Result<_, _>>())
        .transpose()?
        .unwrap_or_default();
    let auto_commit = parse_auto_commit(v.get("autoCommit"))?;
    Ok(CollectionOptions {
        tokenizer,
        user_dict,
        auto_commit,
    })
}

fn parse_dict_entry(v: &serde_json::Value) -> Result<UserDictEntry, VaneError> {
    match v {
        serde_json::Value::String(s) => Ok(UserDictEntry::Word(s.clone())),
        o => Ok(UserDictEntry::WordWithFreq {
            term: o
                .get("term")
                .and_then(|v| v.as_str())
                .ok_or_else(|| VaneError::InvalidArg("userDict.term missing".into()))?
                .to_string(),
            freq: o.get("freq").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
        }),
    }
}

fn parse_schema(v: &serde_json::Value) -> Result<Schema, VaneError> {
    let fields_arr = v
        .get("fields")
        .and_then(|v| v.as_array())
        .ok_or_else(|| VaneError::InvalidArg("schema.fields must be an array".into()))?;
    let mut fields = Vec::new();
    for entry in fields_arr {
        let name = entry
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| VaneError::InvalidArg("field.name missing".into()))?
            .to_string();
        let fd = parse_field(entry)?;
        fields.push((name, fd));
    }
    Schema::new(fields)
}

fn parse_field(v: &serde_json::Value) -> Result<FieldDef, VaneError> {
    let t = v
        .get("type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| VaneError::InvalidArg("field.type missing".into()))?;
    Ok(match t {
        "text" => FieldDef::Text,
        "vector" => FieldDef::Vector {
            dim: v
                .get("dim")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| VaneError::InvalidArg("vector.dim missing".into()))?
                as u32,
            metric: match v.get("metric").and_then(|v| v.as_str()) {
                Some("l2") => Metric::L2,
                Some("dot") => Metric::Dot,
                _ => Metric::Cosine,
            },
        },
        "scalar" => FieldDef::Scalar {
            kind: match v.get("kind").and_then(|v| v.as_str()) {
                Some("int") => ScalarKind::Int,
                Some("float") => ScalarKind::Float,
                Some("bool") => ScalarKind::Bool,
                _ => ScalarKind::Keyword,
            },
        },
        other => return Err(VaneError::InvalidArg(format!("unknown field type {other}"))),
    })
}

fn parse_docs(v: &serde_json::Value) -> Result<Vec<Doc>, VaneError> {
    let arr = v
        .as_array()
        .ok_or_else(|| VaneError::InvalidArg("docs must be array".into()))?;
    arr.iter().map(parse_doc).collect()
}

fn parse_doc(v: &serde_json::Value) -> Result<Doc, VaneError> {
    let id = v
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| VaneError::InvalidArg("doc.id missing".into()))?
        .to_string();
    let text = v.get("text").and_then(|v| v.as_str()).map(String::from);
    let vector = v.get("vector").and_then(|v| v.as_array()).map(|a| {
        a.iter()
            .filter_map(|x| x.as_f64().map(|f| f as f32))
            .collect::<Vec<_>>()
    });
    let meta = v.get("meta").and_then(|v| v.as_object()).map(|o| {
        o.iter()
            .filter_map(|(k, vv)| parse_scalar(vv).map(|sv| (k.clone(), sv)))
            .collect()
    });
    Ok(Doc {
        id,
        text,
        vector,
        meta,
    })
}

fn parse_scalar(v: &serde_json::Value) -> Option<ScalarValue> {
    match v {
        serde_json::Value::Number(n) if n.is_i64() => Some(ScalarValue::Int(n.as_i64().unwrap())),
        serde_json::Value::Number(n) if n.is_f64() => Some(ScalarValue::Float(n.as_f64().unwrap())),
        serde_json::Value::Bool(b) => Some(ScalarValue::Bool(*b)),
        serde_json::Value::String(s) => Some(ScalarValue::Keyword(s.clone())),
        _ => None,
    }
}

fn parse_search_query(v: &serde_json::Value) -> Result<SearchQuery, VaneError> {
    let text = v.get("text").and_then(|v| v.as_str()).map(String::from);
    let vector = v.get("vector").and_then(|v| v.as_array()).map(|a| {
        a.iter()
            .filter_map(|x| x.as_f64().map(|f| f as f32))
            .collect::<Vec<_>>()
    });
    if text.is_none() && vector.is_none() {
        return Err(VaneError::InvalidArg("text or vector required".into()));
    }
    let top_k = v.get("topK").and_then(|v| v.as_u64()).unwrap_or(10) as u32;
    let mode = match v.get("mode").and_then(|v| v.as_str()) {
        Some("hybrid") => SearchMode::Hybrid,
        Some("vector") => SearchMode::Vector,
        Some("text") => SearchMode::Text,
        _ => SearchMode::Auto,
    };
    let fusion = match v.get("fusion") {
        Some(serde_json::Value::String(s)) if s == "rrf" => FusionSpec::Rrf,
        Some(o) => FusionSpec::Linear {
            alpha: o
                .get("linear")
                .and_then(|l| l.get("alpha"))
                .and_then(|v| v.as_f64())
                .unwrap_or(0.5) as f32,
        },
        None => FusionSpec::Rrf,
    };
    // filter 暂不支持（与 vane-ffi 一致，M2 后续模块接入）。
    if v.get("filter").is_some_and(|f| !f.is_null()) {
        return Err(VaneError::InvalidArg("filter not supported in wasm".into()));
    }
    let filter = None;
    let candidate_multiplier = v
        .get("candidateMultiplier")
        .and_then(|v| v.as_u64())
        .unwrap_or(3) as u32;
    Ok(SearchQuery {
        text,
        vector,
        top_k,
        mode,
        fusion,
        filter,
        candidate_multiplier,
    })
}

fn hits_to_json(hits: &[Hit]) -> serde_json::Value {
    serde_json::Value::Array(
        hits.iter()
            .map(|h| {
                let fields = h
                    .fields
                    .as_ref()
                    .map(|m| {
                        let mut o = serde_json::Map::new();
                        for (k, val) in m {
                            o.insert(k.clone(), serde_json::Value::String(val.clone()));
                        }
                        serde_json::Value::Object(o)
                    })
                    .unwrap_or(serde_json::Value::Null);
                serde_json::json!({ "id": h.id, "score": h.score, "fields": fields })
            })
            .collect(),
    )
}

// =========================================================================
// wasm-bindgen 导出（SPEC §9 / M1 README §09 契约对齐，I-8 薄壳）
// =========================================================================

/// 返回 vane 包版本（CARGO_PKG_VERSION）。
#[wasm_bindgen]
pub fn vane_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// 打开 Vane 数据库。path 为逻辑路径（MemoryVfs 内存键），opts_json 为 OpenOptions JSON。
/// 成功返回 Db 句柄（u64）；失败 throw JsValue。
///
/// M2-01：用 MemoryVfs（内存 VFS）。M2-02 接入 OPFS VFS（feature-gated）。
#[wasm_bindgen]
pub fn vane_open(path: &str, opts_json: &str) -> JsResult<u64> {
    let opts: OpenOptions = if opts_json.is_empty() {
        OpenOptions::default()
    } else {
        let v: serde_json::Value = serde_json::from_str(opts_json)
            .map_err(|e| err_to_js(VaneError::InvalidArg(format!("opts_json parse: {e}"))))?;
        parse_open_opts(&v).map_err(err_to_js)?
    };
    let vfs: Arc<dyn Vfs> = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs, path, opts).map_err(err_to_js)?;
    let h = alloc_handle(RegistryEntry::new_db(Arc::new(db))).map_err(err_to_js)?;
    Ok(h)
}

/// 创建或获取 collection。schema_json 为 Schema JSON，opts_json 为 CollectionOptions JSON。
/// 成功返回 Collection 句柄（u64）。
#[wasm_bindgen]
pub fn vane_collection(db_h: u64, name: &str, schema_json: &str, opts_json: &str) -> JsResult<u64> {
    let db = lookup_db(db_h)
        .map_err(err_to_js)?
        .ok_or_else(|| VaneError::NotFound(format!("db handle {db_h} not found")))
        .map_err(err_to_js)?;
    let schema_v: serde_json::Value = serde_json::from_str(schema_json)
        .map_err(|e| err_to_js(VaneError::InvalidArg(format!("schema_json parse: {e}"))))?;
    let schema = parse_schema(&schema_v).map_err(err_to_js)?;
    let opts: CollectionOptions = if opts_json.is_empty() {
        CollectionOptions::default()
    } else {
        let v: serde_json::Value = serde_json::from_str(opts_json)
            .map_err(|e| err_to_js(VaneError::InvalidArg(format!("opts_json parse: {e}"))))?;
        parse_collection_opts(&v).map_err(err_to_js)?
    };
    let col = db.collection(name, schema, opts).map_err(err_to_js)?;
    let h = alloc_handle(RegistryEntry::new_col(Arc::new(col))).map_err(err_to_js)?;
    Ok(h)
}

/// 追加文档。docs_json 为 Doc[] JSON。返回 accepted 数量。
#[wasm_bindgen]
pub fn vane_add(col_h: u64, docs_json: &str) -> JsResult<u64> {
    let col = lookup_col(col_h)
        .map_err(err_to_js)?
        .ok_or_else(|| VaneError::NotFound(format!("collection handle {col_h} not found")))
        .map_err(err_to_js)?;
    let v: serde_json::Value = serde_json::from_str(docs_json)
        .map_err(|e| err_to_js(VaneError::InvalidArg(format!("docs_json parse: {e}"))))?;
    let docs = parse_docs(&v).map_err(err_to_js)?;
    let report = col.add(&docs).map_err(err_to_js)?;
    Ok(report.accepted)
}

/// 刷新缓冲区，持久化段。
#[wasm_bindgen]
pub fn vane_flush(col_h: u64) -> JsResult<()> {
    let col = lookup_col(col_h)
        .map_err(err_to_js)?
        .ok_or_else(|| VaneError::NotFound(format!("collection handle {col_h} not found")))
        .map_err(err_to_js)?;
    col.flush().map_err(err_to_js)?;
    Ok(())
}

/// 搜索。query_json 为 SearchQuery JSON。返回 Hit[] JSON 字符串。
#[wasm_bindgen]
pub fn vane_search(col_h: u64, query_json: &str) -> JsResult<String> {
    let col = lookup_col(col_h)
        .map_err(err_to_js)?
        .ok_or_else(|| VaneError::NotFound(format!("collection handle {col_h} not found")))
        .map_err(err_to_js)?;
    let v: serde_json::Value = serde_json::from_str(query_json)
        .map_err(|e| err_to_js(VaneError::InvalidArg(format!("query_json parse: {e}"))))?;
    let query = parse_search_query(&v).map_err(err_to_js)?;
    let hits = col.search(&query).map_err(err_to_js)?;
    let json = serde_json::to_string(&hits_to_json(&hits))
        .map_err(|e| err_to_js(VaneError::InvalidArg(format!("hits serialize: {e}"))))?;
    Ok(json)
}

/// 删除文档。ids_json 为 string[] JSON。返回已删除数。
#[wasm_bindgen]
pub fn vane_delete(col_h: u64, ids_json: &str) -> JsResult<u64> {
    let col = lookup_col(col_h)
        .map_err(err_to_js)?
        .ok_or_else(|| VaneError::NotFound(format!("collection handle {col_h} not found")))
        .map_err(err_to_js)?;
    let v: serde_json::Value = serde_json::from_str(ids_json)
        .map_err(|e| err_to_js(VaneError::InvalidArg(format!("ids_json parse: {e}"))))?;
    let arr = v
        .as_array()
        .ok_or_else(|| VaneError::InvalidArg("ids must be array".into()))
        .map_err(err_to_js)?;
    let ids: Vec<String> = arr
        .iter()
        .filter_map(|x| x.as_str().map(String::from))
        .collect();
    let count = col.delete(&ids).map_err(err_to_js)?;
    Ok(count)
}

/// 触发段合并。
#[wasm_bindgen]
pub fn vane_compact(col_h: u64) -> JsResult<()> {
    let col = lookup_col(col_h)
        .map_err(err_to_js)?
        .ok_or_else(|| VaneError::NotFound(format!("collection handle {col_h} not found")))
        .map_err(err_to_js)?;
    col.compact().map_err(err_to_js)?;
    Ok(())
}

/// 触发 reindex（M1 同步执行）。返回 progress（1.0 表示已完成）。
#[wasm_bindgen]
pub fn vane_reindex(col_h: u64) -> JsResult<f32> {
    let col = lookup_col(col_h)
        .map_err(err_to_js)?
        .ok_or_else(|| VaneError::NotFound(format!("collection handle {col_h} not found")))
        .map_err(err_to_js)?;
    let rh = col.reindex().map_err(err_to_js)?;
    let progress = rh.progress();
    let _ = alloc_handle(RegistryEntry::new_reindex(rh)).map_err(err_to_js)?;
    Ok(progress)
}

/// 导出数据库快照（M2-12 接入；当前返 E_UNSUPPORTED）。
#[wasm_bindgen]
pub fn vane_export(db_h: u64, dest: &str) -> JsResult<()> {
    let db = lookup_db(db_h)
        .map_err(err_to_js)?
        .ok_or_else(|| VaneError::NotFound(format!("db handle {db_h} not found")))
        .map_err(err_to_js)?;
    db.export(dest).map_err(err_to_js)?;
    Ok(())
}

/// 关闭句柄（Db / Collection / Reindex 均可）。注销后该句柄不可再用。
#[wasm_bindgen]
pub fn vane_close(handle: u64) -> JsResult<()> {
    let removed = remove_handle(handle).map_err(err_to_js)?;
    if !removed {
        return Err(err_to_js(VaneError::NotFound(format!(
            "handle {handle} not found"
        ))));
    }
    Ok(())
}
