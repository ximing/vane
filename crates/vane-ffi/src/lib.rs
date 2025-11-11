//! M2-11：vane-ffi C ABI 实装（SPEC §9，M1 README §09 契约逐字落实）。
//!
//! 句柄 uint64_t + 全局注册表 `std::sync::RwLock<HashMap<u64, Arc<...>>>`（非 dashmap）。
//! 所有函数返回 i32（0=OK，负=错误码 SPEC §10），详情经 `vane_last_error_message`。
//! 内存铁律 I-7：谁分配谁释放，跨边界只借不还，arena 一次 free。
//!
//! 三类句柄：Db / Collection / ReindexHandle，由全局原子计数器分配 u64。
//! `vane_close(h)` 注销；注销后使用 = E_NOT_FOUND（-3），非 UB。
//!
//! # Safety
//! 所有 `extern "C"` 函数接受原始指针参数。调用方（C/Go cgo）须保证指针有效且
//! 生命周期覆盖调用期间。Rust 侧不标记 `unsafe` 因 C 调用方无 `unsafe` 概念；
//! 内部对 null 指针做防御性检查（返 E_INVALID_ARG），但非 null 指针的合法性由
//! 调用方负责。

#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use vane_core::api::{
    Collection, CollectionOptions, Db, Doc, FusionSpec, Hit, OpenOptions, PersistenceMode,
    ReindexHandle, ScalarValue, SearchMode, SearchQuery,
};
use vane_core::persistence::AutoCommitConfig;
use vane_core::tokenizer::{BuiltinTokenizer, UserDictEntry};
use vane_core::types::{FieldDef, Metric, ScalarKind, Schema, VaneError};
use vane_core::vfs::std_fs::StdFsVfs;
use vane_core::vfs::Vfs;

// ---- 句柄注册表 ----

/// 句柄类型标签（内部诊断用，不跨 FFI 边界）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
enum HandleKind {
    Db,
    Collection,
    Reindex,
}

struct RegistryEntry {
    #[allow(dead_code)]
    kind: HandleKind,
    /// 持有句柄资源的类型擦除指针。Db/Collection 存 Arc clone，Reindex 存 owned。
    db: Option<Arc<Db>>,
    col: Option<Arc<Collection>>,
    reindex: Option<ReindexHandle>,
}

impl RegistryEntry {
    fn new_db(db: Arc<Db>) -> Self {
        Self {
            kind: HandleKind::Db,
            db: Some(db),
            col: None,
            reindex: None,
        }
    }
    fn new_col(col: Arc<Collection>) -> Self {
        Self {
            kind: HandleKind::Collection,
            db: None,
            col: Some(col),
            reindex: None,
        }
    }
    fn new_reindex(rh: ReindexHandle) -> Self {
        Self {
            kind: HandleKind::Reindex,
            db: None,
            col: None,
            reindex: Some(rh),
        }
    }
}

static REGISTRY: RwLock<Option<HashMap<u64, RegistryEntry>>> = RwLock::new(None);
static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);

fn alloc_handle(entry: RegistryEntry) -> u64 {
    let h = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
    let mut reg = REGISTRY.write().unwrap();
    reg.get_or_insert_with(HashMap::new).insert(h, entry);
    h
}

fn lookup_db(h: u64) -> Option<Arc<Db>> {
    REGISTRY
        .read()
        .unwrap()
        .as_ref()
        .and_then(|m| m.get(&h))
        .and_then(|e| e.db.clone())
}

fn lookup_col(h: u64) -> Option<Arc<Collection>> {
    REGISTRY
        .read()
        .unwrap()
        .as_ref()
        .and_then(|m| m.get(&h))
        .and_then(|e| e.col.clone())
}

/// ReindexHandle 内部持 Arc，但未暴露 Clone。为支持注册表 lookup 后调用
/// progress/wait，我们在注册表存 owned，操作时在 read lock 内通过闭包调用。
fn with_reindex_handle<R>(h: u64, f: impl FnOnce(&ReindexHandle) -> R) -> Option<R> {
    let reg = REGISTRY.read().unwrap();
    reg.as_ref()
        .and_then(|m| m.get(&h))
        .and_then(|e| e.reindex.as_ref().map(f))
}

fn remove_handle(h: u64) -> bool {
    REGISTRY
        .write()
        .unwrap()
        .as_mut()
        .and_then(|m| m.remove(&h).map(|_| true))
        .unwrap_or(false)
}

// ---- 线程局部错误 ----

thread_local! {
    static LAST_ERROR: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

fn set_error(msg: impl Into<String>) {
    LAST_ERROR.with(|e| *e.borrow_mut() = Some(msg.into()));
}

/// 把 VaneError 写入 thread-local 并返回错误码。
fn fail(e: VaneError) -> i32 {
    let code = e.code();
    set_error(e.to_string());
    code
}

// ---- 辅助：从 C 切片读 bytes ----

unsafe fn slice_from_raw<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
    if ptr.is_null() || len == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(ptr, len) }
    }
}

// ---- 辅助：分配 C arena 字符串（调用方用 vane_string_free 释放） ----

/// 记录已分配的 arena 布局，供 vane_string_free 释放。
/// 用全局 HashMap 记录 (ptr → Layout)，避免 free 时 layout 不匹配。
static ARENA_LAYOUTS: RwLock<Option<HashMap<usize, std::alloc::Layout>>> = RwLock::new(None);

fn arena_alloc_tracked(bytes: &[u8]) -> *mut u8 {
    let layout = match std::alloc::Layout::from_size_align(bytes.len() + 1, 8) {
        Ok(l) => l,
        Err(_) => return std::ptr::null_mut(),
    };
    unsafe {
        let ptr = std::alloc::alloc(layout);
        if ptr.is_null() {
            return std::ptr::null_mut();
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
        *ptr.add(bytes.len()) = 0;
        let mut layouts = ARENA_LAYOUTS.write().unwrap();
        layouts
            .get_or_insert_with(HashMap::new)
            .insert(ptr as usize, layout);
        ptr
    }
}

// ---- JSON 解析辅助（复用 vane-node convert.rs 的 JSON schema，I-8 薄壳） ----

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
    // filter 不在 FFI 薄壳支持范围（与 vane-node M0 一致）；非 null reject。
    if v.get("filter").is_some_and(|f| !f.is_null()) {
        return Err(VaneError::InvalidArg("filter not supported in FFI".into()));
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

// ---- 全局词典版本（vane_load_dict 后设置，vane_dict_version 读取） ----

static DICT_VERSION_INFO: RwLock<Option<(String, [u8; 8])>> = RwLock::new(None);

// =========================================================================
// C ABI 函数（SPEC §9 / M1 README §09 契约逐字落实）
// =========================================================================

/// 打开 Vane 数据库。path 为 UTF-8 路径，opts_json 为 OpenOptions JSON。
/// 成功返回 0，out_handle 写入 Db 句柄；失败返回负错误码。
#[no_mangle]
pub extern "C" fn vane_open(
    path_ptr: *const u8,
    path_len: usize,
    opts_json: *const u8,
    opts_len: usize,
    out_handle: *mut u64,
) -> i32 {
    if out_handle.is_null() {
        return VaneError::InvalidArg("out_handle is null".into()).code();
    }
    let path_bytes = unsafe { slice_from_raw(path_ptr, path_len) };
    let path = match std::str::from_utf8(path_bytes) {
        Ok(s) => s,
        Err(_) => return fail(VaneError::InvalidArg("path is not valid UTF-8".into())),
    };
    let opts_bytes = unsafe { slice_from_raw(opts_json, opts_len) };
    let opts: OpenOptions = if opts_bytes.is_empty() {
        OpenOptions::default()
    } else {
        let v: serde_json::Value = match serde_json::from_slice(opts_bytes) {
            Ok(v) => v,
            Err(e) => return fail(VaneError::InvalidArg(format!("opts_json parse: {e}"))),
        };
        match parse_open_opts(&v) {
            Ok(o) => o,
            Err(e) => return fail(e),
        }
    };
    let vfs: Arc<dyn Vfs> = Arc::new(StdFsVfs::new());
    match Db::open(vfs, path, opts) {
        Ok(db) => {
            let h = alloc_handle(RegistryEntry::new_db(Arc::new(db)));
            unsafe { *out_handle = h };
            0
        }
        Err(e) => fail(e),
    }
}

/// 创建或获取 collection。schema_json 为 Schema JSON，opts_json 为 CollectionOptions JSON。
#[no_mangle]
pub extern "C" fn vane_collection(
    db_h: u64,
    name_ptr: *const u8,
    name_len: usize,
    schema_json: *const u8,
    schema_len: usize,
    opts_json: *const u8,
    opts_len: usize,
    out_handle: *mut u64,
) -> i32 {
    if out_handle.is_null() {
        return VaneError::InvalidArg("out_handle is null".into()).code();
    }
    let db = match lookup_db(db_h) {
        Some(d) => d,
        None => return fail(VaneError::NotFound(format!("db handle {db_h} not found"))),
    };
    let name_bytes = unsafe { slice_from_raw(name_ptr, name_len) };
    let name = match std::str::from_utf8(name_bytes) {
        Ok(s) => s,
        Err(_) => return fail(VaneError::InvalidArg("name is not valid UTF-8".into())),
    };
    let schema_bytes = unsafe { slice_from_raw(schema_json, schema_len) };
    let schema_v: serde_json::Value = match serde_json::from_slice(schema_bytes) {
        Ok(v) => v,
        Err(e) => return fail(VaneError::InvalidArg(format!("schema_json parse: {e}"))),
    };
    let schema = match parse_schema(&schema_v) {
        Ok(s) => s,
        Err(e) => return fail(e),
    };
    let opts_bytes = unsafe { slice_from_raw(opts_json, opts_len) };
    let opts: CollectionOptions = if opts_bytes.is_empty() {
        CollectionOptions::default()
    } else {
        let v: serde_json::Value = match serde_json::from_slice(opts_bytes) {
            Ok(v) => v,
            Err(e) => return fail(VaneError::InvalidArg(format!("opts_json parse: {e}"))),
        };
        match parse_collection_opts(&v) {
            Ok(o) => o,
            Err(e) => return fail(e),
        }
    };
    match db.collection(name, schema, opts) {
        Ok(col) => {
            // Collection 内部 Clone 廉价（Arc），但我们需要 Arc<Collection>。
            // Collection impl Clone → clone 得到新 Collection（共享 inner Arc）。
            // 注册表存 Arc<Collection>：用 Arc 不行（Collection 不是 Clone→Arc）。
            // 改为存 Collection（owned，Clone 廉价）。
            let h = alloc_handle(RegistryEntry::new_col(Arc::new(col)));
            unsafe { *out_handle = h };
            0
        }
        Err(e) => fail(e),
    }
}

/// 追加文档。docs_json 为 Doc[] JSON。
#[no_mangle]
pub extern "C" fn vane_add(col_h: u64, docs_json: *const u8, docs_len: usize) -> i32 {
    let col = match lookup_col(col_h) {
        Some(c) => c,
        None => {
            return fail(VaneError::NotFound(format!(
                "collection handle {col_h} not found"
            )))
        }
    };
    let bytes = unsafe { slice_from_raw(docs_json, docs_len) };
    let v: serde_json::Value = match serde_json::from_slice(bytes) {
        Ok(v) => v,
        Err(e) => return fail(VaneError::InvalidArg(format!("docs_json parse: {e}"))),
    };
    let docs = match parse_docs(&v) {
        Ok(d) => d,
        Err(e) => return fail(e),
    };
    match col.add(&docs) {
        Ok(_) => 0,
        Err(e) => fail(e),
    }
}

/// 刷新缓冲区，持久化段。
#[no_mangle]
pub extern "C" fn vane_flush(col_h: u64) -> i32 {
    let col = match lookup_col(col_h) {
        Some(c) => c,
        None => {
            return fail(VaneError::NotFound(format!(
                "collection handle {col_h} not found"
            )))
        }
    };
    match col.flush() {
        Ok(_) => 0,
        Err(e) => fail(e),
    }
}

/// 搜索。query_json 为 SearchQuery JSON。out_arena 返回 JSON 结果（Hit[]），
/// 调用方须用 vane_string_free 释放。
#[no_mangle]
pub extern "C" fn vane_search(
    col_h: u64,
    query_json: *const u8,
    query_len: usize,
    out_arena: *mut *mut u8,
    out_len: *mut usize,
) -> i32 {
    if out_arena.is_null() || out_len.is_null() {
        return VaneError::InvalidArg("out_arena/out_len is null".into()).code();
    }
    let col = match lookup_col(col_h) {
        Some(c) => c,
        None => {
            return fail(VaneError::NotFound(format!(
                "collection handle {col_h} not found"
            )))
        }
    };
    let bytes = unsafe { slice_from_raw(query_json, query_len) };
    let v: serde_json::Value = match serde_json::from_slice(bytes) {
        Ok(v) => v,
        Err(e) => return fail(VaneError::InvalidArg(format!("query_json parse: {e}"))),
    };
    let query = match parse_search_query(&v) {
        Ok(q) => q,
        Err(e) => return fail(e),
    };
    match col.search(&query) {
        Ok(hits) => {
            let json =
                serde_json::to_vec(&hits_to_json(&hits)).unwrap_or_else(|_| b"[]"[..].to_vec());
            let len = json.len();
            let ptr = arena_alloc_tracked(&json);
            if ptr.is_null() {
                return fail(VaneError::InvalidArg("arena alloc failed".into()));
            }
            unsafe {
                *out_arena = ptr;
                *out_len = len;
            }
            0
        }
        Err(e) => fail(e),
    }
}

/// 删除文档。ids_json 为 string[] JSON。out_count 返回已删除数。
#[no_mangle]
pub extern "C" fn vane_delete(
    col_h: u64,
    ids_json: *const u8,
    ids_len: usize,
    out_count: *mut u64,
) -> i32 {
    if out_count.is_null() {
        return VaneError::InvalidArg("out_count is null".into()).code();
    }
    let col = match lookup_col(col_h) {
        Some(c) => c,
        None => {
            return fail(VaneError::NotFound(format!(
                "collection handle {col_h} not found"
            )))
        }
    };
    let bytes = unsafe { slice_from_raw(ids_json, ids_len) };
    let v: serde_json::Value = match serde_json::from_slice(bytes) {
        Ok(v) => v,
        Err(e) => return fail(VaneError::InvalidArg(format!("ids_json parse: {e}"))),
    };
    let arr = v
        .as_array()
        .ok_or_else(|| VaneError::InvalidArg("ids must be array".into()));
    let arr = match arr {
        Ok(a) => a,
        Err(e) => return fail(e),
    };
    let ids: Vec<String> = arr
        .iter()
        .filter_map(|x| x.as_str().map(String::from))
        .collect();
    match col.delete(&ids) {
        Ok(count) => {
            unsafe { *out_count = count };
            0
        }
        Err(e) => fail(e),
    }
}

/// 触发段合并。
#[no_mangle]
pub extern "C" fn vane_compact(col_h: u64) -> i32 {
    let col = match lookup_col(col_h) {
        Some(c) => c,
        None => {
            return fail(VaneError::NotFound(format!(
                "collection handle {col_h} not found"
            )))
        }
    };
    match col.compact() {
        Ok(_) => 0,
        Err(e) => fail(e),
    }
}

/// 触发 reindex。out_handle 返回 ReindexHandle 句柄。
#[no_mangle]
pub extern "C" fn vane_reindex(col_h: u64, out_handle: *mut u64) -> i32 {
    if out_handle.is_null() {
        return VaneError::InvalidArg("out_handle is null".into()).code();
    }
    let col = match lookup_col(col_h) {
        Some(c) => c,
        None => {
            return fail(VaneError::NotFound(format!(
                "collection handle {col_h} not found"
            )))
        }
    };
    match col.reindex() {
        Ok(rh) => {
            let h = alloc_handle(RegistryEntry::new_reindex(rh));
            unsafe { *out_handle = h };
            0
        }
        Err(e) => fail(e),
    }
}

/// 查询 reindex 进度（0.0..1.0）。
#[no_mangle]
pub extern "C" fn vane_reindex_progress(h: u64, out_progress: *mut f32) -> i32 {
    if out_progress.is_null() {
        return VaneError::InvalidArg("out_progress is null".into()).code();
    }
    match with_reindex_handle(h, |rh| rh.progress()) {
        Some(p) => {
            unsafe { *out_progress = p };
            0
        }
        None => fail(VaneError::NotFound(format!("reindex handle {h} not found"))),
    }
}

/// 阻塞等待 reindex 完成。
#[no_mangle]
pub extern "C" fn vane_reindex_wait(h: u64) -> i32 {
    match with_reindex_handle(h, |rh| rh.wait()) {
        Some(Ok(())) => 0,
        Some(Err(e)) => fail(e),
        None => fail(VaneError::NotFound(format!("reindex handle {h} not found"))),
    }
}

/// 加载 jieba 词典（zstd 压缩 dict.bin 字节）。注入到 db 句柄对应的 Db。
#[no_mangle]
pub extern "C" fn vane_load_dict(h: u64, dict_ptr: *const u8, dict_len: usize) -> i32 {
    let db = match lookup_db(h) {
        Some(d) => d,
        None => {
            // h 可能是 db 句柄；若 not found 也可能是误用 col handle。
            return fail(VaneError::NotFound(format!("db handle {h} not found")));
        }
    };
    let bytes = unsafe { slice_from_raw(dict_ptr, dict_len) };
    if bytes.is_empty() {
        return fail(VaneError::InvalidArg("dict bytes empty".into()));
    }
    let dict = match vane_core::tokenizer::jieba::JiebaDict::load_zstd(bytes) {
        Ok(d) => d,
        Err(e) => return fail(e),
    };
    let version = dict.version().to_string();
    let sha = dict.sha256_prefix();
    db.set_jieba_dict(Arc::new(dict));
    *DICT_VERSION_INFO.write().unwrap() = Some((version, sha));
    0
}

/// 查询词典版本 + sha256 前缀（JSON：{"version":"2026.08","sha256Prefix":"hex16"}）。
/// out_ptr 返回 arena（vane_string_free 释放）。
#[no_mangle]
pub extern "C" fn vane_dict_version(out_ptr: *mut *mut u8, out_len: *mut usize) -> i32 {
    if out_ptr.is_null() || out_len.is_null() {
        return VaneError::InvalidArg("out_ptr/out_len is null".into()).code();
    }
    let guard = DICT_VERSION_INFO.read().unwrap();
    match guard.as_ref() {
        Some((version, sha)) => {
            let sha_hex: String = sha.iter().map(|b| format!("{:02x}", b)).collect();
            let json = serde_json::json!({
                "version": version,
                "sha256Prefix": sha_hex
            });
            let bytes = serde_json::to_vec(&json).unwrap_or_default();
            let len = bytes.len();
            let ptr = arena_alloc_tracked(&bytes);
            if ptr.is_null() {
                return fail(VaneError::InvalidArg("arena alloc failed".into()));
            }
            unsafe {
                *out_ptr = ptr;
                *out_len = len;
            }
            0
        }
        None => fail(VaneError::DictUnavailable),
    }
}

/// 导出数据库快照（M2-12 接入；当前返 E_UNSUPPORTED）。
#[no_mangle]
pub extern "C" fn vane_export(db_h: u64, dest_ptr: *const u8, dest_len: usize) -> i32 {
    let db = match lookup_db(db_h) {
        Some(d) => d,
        None => return fail(VaneError::NotFound(format!("db handle {db_h} not found"))),
    };
    let dest_bytes = unsafe { slice_from_raw(dest_ptr, dest_len) };
    let dest = match std::str::from_utf8(dest_bytes) {
        Ok(s) => s,
        Err(_) => return fail(VaneError::InvalidArg("dest is not valid UTF-8".into())),
    };
    match db.export(dest) {
        Ok(_) => 0,
        Err(e) => fail(e),
    }
}

/// 关闭句柄（Db / Collection / Reindex 均可）。注销后该句柄不可再用。
#[no_mangle]
pub extern "C" fn vane_close(handle: u64) -> i32 {
    if remove_handle(handle) {
        0
    } else {
        // 句柄不存在或已关闭——返回 E_NOT_FOUND（非 UB，I-7）。
        VaneError::NotFound(format!("handle {handle} not found")).code()
    }
}

/// 查询最近一次错误的描述（C 字符串，NUL 终止）。
/// 返回的指针在线程局部有效，直到下次同线程调用任何 vane_* 函数。
/// 调用方不应 free（线程局部缓冲，随线程消亡）。
/// 若无错误返回 null。
///
/// handle 参数当前未使用（错误是线程局部的，不绑定句柄）；保留以匹配 §09 契约。
#[no_mangle]
pub extern "C" fn vane_last_error_message(_handle: u64) -> *const u8 {
    LAST_ERROR.with(|e| {
        let guard = e.borrow();
        match guard.as_ref() {
            Some(msg) => {
                // 返回 NUL 终止的 C 字符串指针。
                // 用 thread_local 存储 CString 以保持指针有效。
                // 简化：每次调用重新分配 thread_local CString。
                LAST_ERROR_CSTRING.with(|c| {
                    *c.borrow_mut() =
                        Some(std::ffi::CString::new(msg.as_str()).unwrap_or_default());
                    c.borrow().as_ref().unwrap().as_ptr() as *const u8
                })
            }
            None => std::ptr::null(),
        }
    })
}

thread_local! {
    static LAST_ERROR_CSTRING: std::cell::RefCell<Option<std::ffi::CString>> = const { std::cell::RefCell::new(None) };
}

/// 释放 vane_search / vane_dict_version 返回的 arena 内存。
/// 传入 null 安全（no-op）。
#[no_mangle]
pub extern "C" fn vane_string_free(ptr: *mut u8) {
    if ptr.is_null() {
        return;
    }
    let mut layouts = ARENA_LAYOUTS.write().unwrap();
    if let Some(map) = layouts.as_mut() {
        if let Some(layout) = map.remove(&(ptr as usize)) {
            unsafe { std::alloc::dealloc(ptr, layout) };
        }
    }
}

// =========================================================================
// 测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "vane-ffi-test-{}-{}",
            std::process::id(),
            NEXT_HANDLE.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn json_ptr(s: &str) -> (*const u8, usize) {
        (s.as_ptr(), s.len())
    }

    #[test]
    fn open_close_roundtrip() {
        let dir = tmp_dir();
        let path = dir.to_str().unwrap();
        let mut handle: u64 = 0;
        let (p, pl) = json_ptr(path);
        let rc = vane_open(p, pl, std::ptr::null(), 0, &mut handle);
        assert_eq!(rc, 0, "open should succeed");
        assert!(handle > 0);

        // close
        let rc = vane_close(handle);
        assert_eq!(rc, 0, "close should succeed");

        // close again → E_NOT_FOUND (-3)
        let rc = vane_close(handle);
        assert_eq!(rc, -3, "double close should return E_NOT_FOUND");

        // use after close → E_NOT_FOUND
        let rc = vane_flush(handle);
        assert_eq!(rc, -3, "use after close should return E_NOT_FOUND");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn collection_add_flush_search() {
        let dir = tmp_dir();
        let path = dir.to_str().unwrap();
        let mut db_h: u64 = 0;
        let (p, pl) = json_ptr(path);
        assert_eq!(vane_open(p, pl, std::ptr::null(), 0, &mut db_h), 0);

        // collection
        let mut col_h: u64 = 0;
        let schema = r#"{"fields":[{"name":"vec","type":"vector","dim":4,"metric":"cosine"},{"name":"body","type":"text"}]}"#;
        let (sp, sl) = json_ptr(schema);
        let (np, nl) = json_ptr("docs");
        assert_eq!(
            vane_collection(db_h, np, nl, sp, sl, std::ptr::null(), 0, &mut col_h),
            0
        );
        assert!(col_h > 0);

        // add
        let docs = r#"[{"id":"a","text":"hello world","vector":[1.0,0.0,0.0,0.0]},{"id":"b","text":"foo bar","vector":[0.0,1.0,0.0,0.0]}]"#;
        let (dp, dl) = json_ptr(docs);
        assert_eq!(vane_add(col_h, dp, dl), 0);

        // flush
        assert_eq!(vane_flush(col_h), 0);

        // search vector
        let query = r#"{"vector":[1.0,0.0,0.0,0.0],"topK":2}"#;
        let (qp, ql) = json_ptr(query);
        let mut arena: *mut u8 = std::ptr::null_mut();
        let mut len: usize = 0;
        assert_eq!(vane_search(col_h, qp, ql, &mut arena, &mut len), 0);
        assert!(!arena.is_null());
        assert!(len > 0);
        let json_str = unsafe { std::slice::from_raw_parts(arena, len) };
        let v: serde_json::Value = serde_json::from_slice(json_str).unwrap();
        assert!(v.is_array());
        let arr = v.as_array().unwrap();
        assert!(!arr.is_empty());
        // top hit should be "a" (cosine similarity 1.0)
        assert_eq!(arr[0].get("id").unwrap().as_str().unwrap(), "a");

        // free arena
        vane_string_free(arena);

        // search text
        let query = r#"{"text":"hello","topK":2}"#;
        let (qp, ql) = json_ptr(query);
        assert_eq!(vane_search(col_h, qp, ql, &mut arena, &mut len), 0);
        assert!(len > 0);
        vane_string_free(arena);

        vane_close(col_h);
        vane_close(db_h);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_and_compact() {
        let dir = tmp_dir();
        let path = dir.to_str().unwrap();
        let mut db_h: u64 = 0;
        let (p, pl) = json_ptr(path);
        assert_eq!(vane_open(p, pl, std::ptr::null(), 0, &mut db_h), 0);

        let mut col_h: u64 = 0;
        let schema = r#"{"fields":[{"name":"vec","type":"vector","dim":2,"metric":"cosine"}]}"#;
        let (sp, sl) = json_ptr(schema);
        let (np, nl) = json_ptr("docs");
        assert_eq!(
            vane_collection(db_h, np, nl, sp, sl, std::ptr::null(), 0, &mut col_h),
            0
        );

        let docs = r#"[{"id":"a","vector":[1.0,0.0]},{"id":"b","vector":[0.0,1.0]}]"#;
        let (dp, dl) = json_ptr(docs);
        assert_eq!(vane_add(col_h, dp, dl), 0);
        assert_eq!(vane_flush(col_h), 0);

        // delete
        let ids = r#"["a"]"#;
        let (ip, il) = json_ptr(ids);
        let mut count: u64 = 0;
        assert_eq!(vane_delete(col_h, ip, il, &mut count), 0);
        assert!(count > 0);

        // compact
        assert_eq!(vane_compact(col_h), 0);

        vane_close(col_h);
        vane_close(db_h);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn last_error_message() {
        // trigger an error: use invalid handle
        let rc = vane_flush(999999);
        assert!(rc < 0);
        let ptr = vane_last_error_message(999999);
        assert!(!ptr.is_null());
        let msg = unsafe {
            let cs = std::ffi::CStr::from_ptr(ptr as *const i8);
            cs.to_string_lossy().into_owned()
        };
        assert!(msg.contains("E_NOT_FOUND") || msg.contains("not found"));
    }

    #[test]
    fn export_returns_unsupported() {
        let dir = tmp_dir();
        let path = dir.to_str().unwrap();
        let mut db_h: u64 = 0;
        let (p, pl) = json_ptr(path);
        assert_eq!(vane_open(p, pl, std::ptr::null(), 0, &mut db_h), 0);
        let dest = "/tmp/vane-ffi-export-test";
        let (dp, dl) = json_ptr(dest);
        let rc = vane_export(db_h, dp, dl);
        assert_eq!(rc, -10, "export should return E_UNSUPPORTED before M2-12");
        vane_close(db_h);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dict_version_unavailable_before_load() {
        // 清除全局 dict version（测试间隔离）
        *DICT_VERSION_INFO.write().unwrap() = None;
        let mut ptr: *mut u8 = std::ptr::null_mut();
        let mut len: usize = 0;
        let rc = vane_dict_version(&mut ptr, &mut len);
        assert_eq!(
            rc, -8,
            "dict_version before load should return E_DICT_UNAVAILABLE"
        );
    }

    #[test]
    fn handle_registry_thread_safety() {
        use std::thread;

        let dir = tmp_dir();
        let path = dir.to_str().unwrap().to_string();
        let mut db_h: u64 = 0;
        let (p, pl) = json_ptr(&path);
        assert_eq!(vane_open(p, pl, std::ptr::null(), 0, &mut db_h), 0);

        let mut col_h: u64 = 0;
        let schema = r#"{"fields":[{"name":"vec","type":"vector","dim":2,"metric":"cosine"}]}"#;
        let (sp, sl) = json_ptr(schema);
        let (np, nl) = json_ptr("docs");
        assert_eq!(
            vane_collection(db_h, np, nl, sp, sl, std::ptr::null(), 0, &mut col_h),
            0
        );

        // 并发 search
        let mut handles = vec![];
        for _ in 0..4 {
            let ch = col_h;
            handles.push(thread::spawn(move || {
                let query = r#"{"vector":[1.0,0.0],"topK":1}"#;
                let (qp, ql) = json_ptr(query);
                let mut arena: *mut u8 = std::ptr::null_mut();
                let mut len: usize = 0;
                let rc = vane_search(ch, qp, ql, &mut arena, &mut len);
                assert_eq!(rc, 0);
                vane_string_free(arena);
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        vane_close(col_h);
        vane_close(db_h);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
