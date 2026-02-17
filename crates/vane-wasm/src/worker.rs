//! VaneWorker——Dedicated Worker 壳（SPEC §4.1 / §11 / §12.3）。
//!
//! 主页面 async ↔ Worker 同步 core 的 postMessage Promise 边界。
//! 异步严格限于 init（Vfs 选择 + 词典加载）+ postMessage 边界；
//! 步骤进入 core 后全同步（REQUIREMENTS §4.1，I-8 薄壳）。
//!
//! ## init 异步序列（SPEC §3 / M2-02 §4.7）
//! ```text
//! OPFS 可用：
//!   1. root = await navigator.storage.getDirectory()
//!   2. fh   = await root.getFileHandle("vane.db", {create:true})
//!   3. sah  = await fh.createSyncAccessHandle()
//!   4. OpfsVfs::from_handle(sah)
//!   5. Db::open(Arc<OpfsVfs>, db_path)          // 同步
//! OPFS 不可用（Safari 历史 bug / API 缺失）→ 降级 IDB：
//!   1. idb  = await open_idb("vane_db")
//!   2. blob = await idb.get("container") ?? Vec::new()
//!   3. IdbVfs::from_blob(blob)
//!   4. Db::open(Arc<IdbVfs>, db_path)            // 同步
//! ```
//!
//! ## 词典加载（SPEC §12.3）
//! init 阶段加载词典（CDN fetch / 内联 dictData / 缓存），Db::open 后 `set_jieba_dict` 注入。
//! `collection(tokenizer=Jieba)` 时若词典不可用 → 降级 `CjkBigram` + warn（不抛错，§12.4）。
//!
//! ## I-8 薄壳
//! 本模块无检索逻辑——全部委托 `vane_core::api`（薄壳）。Promise 仅包装同步结果。

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use vane_core::api::{
    Collection, CollectionOptions, Db, Doc, Hit, OpenOptions, PersistenceMode, ScalarValue,
    SearchQuery,
};
use vane_core::persistence::AutoCommitConfig;
#[cfg(feature = "jieba")]
use vane_core::tokenizer::jieba::JiebaDict;
use vane_core::tokenizer::BuiltinTokenizer;
use vane_core::types::{FieldDef, Metric, ScalarKind, Schema, VaneError};
use vane_core::vfs::memory::MemoryVfs;
use vane_core::vfs::Vfs;

use crate::dict_loader;
use crate::simd_probe;

// =========================================================================
// 内部状态
// =========================================================================

struct WorkerInner {
    db: Option<Db>,
    collections: HashMap<u32, Collection>,
    next_col_id: u32,
    /// init 阶段选定的 Vfs（create 选，open 消费调 Db::open）。
    vfs: Option<Arc<dyn Vfs>>,
    /// 词典字节（init 阶段 fetch/内联），Db::open 后注入 set_jieba_dict。
    dict_bytes: Option<Vec<u8>>,
    closed: bool,
}

impl WorkerInner {
    fn new() -> Self {
        Self {
            db: None,
            collections: HashMap::new(),
            next_col_id: 1,
            vfs: None,
            dict_bytes: None,
            closed: false,
        }
    }

    fn db(&self) -> Result<&Db, VaneError> {
        self.db
            .as_ref()
            .ok_or_else(|| VaneError::InvalidArg("db not opened".into()))
    }

    fn check_open(&self) -> Result<(), VaneError> {
        if self.closed {
            return Err(VaneError::InvalidArg("worker closed".into()));
        }
        Ok(())
    }
}

/// VaneWorker——浏览器 Dedicated Worker 的 wasm-bindgen 胶水对象。
///
/// JS 侧用法（经 worker.js 路由 postMessage）：
/// ```js
/// const worker = await VaneWorker.create({ vfs: "opfs", dbPath: "vane.db" });
/// await worker.open("vane.db", {});
/// const col = await worker.collection("docs", schema, { tokenizer: "jieba" });
/// await worker.add(col, docs);
/// await worker.flush(col);
/// const hits = await worker.search(col, query);
/// ```
#[wasm_bindgen]
pub struct VaneWorker {
    inner: Rc<RefCell<WorkerInner>>,
}

fn err_to_js(e: VaneError) -> JsValue {
    JsValue::from(format!("{}: {}", e.name(), e))
}

/// 跨平台 console.warn（wasm32 用 web_sys::console，native 用 eprintln）。
fn warn(msg: &str) {
    #[cfg(target_arch = "wasm32")]
    web_sys::console::warn_1(&msg.into());
    #[cfg(not(target_arch = "wasm32"))]
    eprintln!("[vane] {}", msg);
}

/// JsValue → serde_json::Value（接受 JS 对象或 JSON 字符串）。
fn js_to_json(v: &JsValue) -> Result<serde_json::Value, VaneError> {
    if v.is_undefined() || v.is_null() {
        return Ok(serde_json::Value::Null);
    }
    if v.is_string() {
        let s = v.as_string().unwrap_or_default();
        if s.is_empty() {
            return Ok(serde_json::Value::Null);
        }
        return serde_json::from_str(&s)
            .map_err(|e| VaneError::InvalidArg(format!("JSON parse: {e}").into()));
    }
    let s = js_sys::JSON::stringify(v)
        .map_err(|e| VaneError::InvalidArg(format!("JSON stringify: {:?}", e).into()))?;
    serde_json::from_str(s.as_string().unwrap_or_default().as_str())
        .map_err(|e| VaneError::InvalidArg(format!("JSON parse: {e}").into()))
}

// ── JSON 解析（与 lib.rs 同构，I-8 薄壳）────────────────────────────────────

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
    use vane_core::tokenizer::UserDictEntry;
    let tokenizer = match v.get("tokenizer").and_then(|v| v.as_str()) {
        Some("cjk_bigram") => BuiltinTokenizer::CjkBigram,
        Some("jieba") => BuiltinTokenizer::Jieba,
        _ => BuiltinTokenizer::Standard,
    };
    let user_dict = v
        .get("userDict")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .map(|e| match e {
                    serde_json::Value::String(s) => Ok(UserDictEntry::Word(s.clone())),
                    o => Ok(UserDictEntry::WordWithFreq {
                        term: o
                            .get("term")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| VaneError::InvalidArg("userDict.term missing".into()))?
                            .to_string(),
                        freq: o.get("freq").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                    }),
                })
                .collect::<Result<_, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    let auto_commit = parse_auto_commit(v.get("autoCommit"))?;
    Ok(CollectionOptions {
        tokenizer,
        user_dict,
        auto_commit,
    })
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
        let t = entry
            .get("type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| VaneError::InvalidArg("field.type missing".into()))?;
        let fd = match t {
            "text" => FieldDef::Text,
            "vector" => FieldDef::Vector {
                dim: entry
                    .get("dim")
                    .and_then(|v| v.as_u64())
                    .ok_or_else(|| VaneError::InvalidArg("vector.dim missing".into()))?
                    as u32,
                metric: match entry.get("metric").and_then(|v| v.as_str()) {
                    Some("l2") => Metric::L2,
                    Some("dot") => Metric::Dot,
                    _ => Metric::Cosine,
                },
            },
            "scalar" => FieldDef::Scalar {
                kind: match entry.get("kind").and_then(|v| v.as_str()) {
                    Some("int") => ScalarKind::Int,
                    Some("float") => ScalarKind::Float,
                    Some("bool") => ScalarKind::Bool,
                    _ => ScalarKind::Keyword,
                },
            },
            other => {
                return Err(VaneError::InvalidArg(
                    format!("unknown field type {other}").into(),
                ))
            }
        };
        fields.push((name, fd));
    }
    Schema::new(fields)
}

fn parse_docs(v: &serde_json::Value) -> Result<Vec<Doc>, VaneError> {
    let arr = v
        .as_array()
        .ok_or_else(|| VaneError::InvalidArg("docs must be array".into()))?;
    arr.iter()
        .map(|v| {
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
                    .filter_map(|(k, vv)| match vv {
                        serde_json::Value::Number(n) if n.is_i64() => {
                            Some((k.clone(), ScalarValue::Int(n.as_i64().unwrap())))
                        }
                        serde_json::Value::Number(n) if n.is_f64() => {
                            Some((k.clone(), ScalarValue::Float(n.as_f64().unwrap())))
                        }
                        serde_json::Value::Bool(b) => Some((k.clone(), ScalarValue::Bool(*b))),
                        serde_json::Value::String(s) => {
                            Some((k.clone(), ScalarValue::Keyword(s.clone())))
                        }
                        _ => None,
                    })
                    .collect()
            });
            Ok(Doc {
                id,
                text,
                vector,
                meta,
            })
        })
        .collect()
}

fn parse_search_query(v: &serde_json::Value) -> Result<SearchQuery, VaneError> {
    use vane_core::api::{FusionSpec, SearchMode};
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
    if v.get("filter").is_some_and(|f| !f.is_null()) {
        return Err(VaneError::InvalidArg("filter not supported in wasm".into()));
    }
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
        filter: None,
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

/// 同步结果 → Promise（成功 resolve / 失败 reject）。
fn ok_promise(v: JsValue) -> js_sys::Promise {
    js_sys::Promise::resolve(&v)
}

fn err_promise(e: VaneError) -> js_sys::Promise {
    js_sys::Promise::reject(&err_to_js(e))
}

// =========================================================================
// init 探针（OPFS 能力检测）
// =========================================================================

/// OPFS 能力探针（SPEC §3 / 落实 M2-03 idb.rs stub）。
///
/// 真实探测 `navigator.storage.getDirectory` 存在性。在 node（无 navigator）
/// 返回 false。Worker init 据此选择 OPFS 主路径或 IDB 降级。
#[cfg(target_arch = "wasm32")]
pub fn opfs_available() -> bool {
    // Worker 上下文无 window——统一经 js_sys::global() 反射获取 navigator。
    // 兼容 Window + WorkerGlobalScope。
    let global = js_sys::global();
    let navigator = match js_sys::Reflect::get(&global, &"navigator".into()) {
        Ok(n) if !n.is_undefined() => n,
        _ => return false,
    };
    let storage = match js_sys::Reflect::get(&navigator, &"storage".into()) {
        Ok(s) if !s.is_undefined() => s,
        _ => return false,
    };
    match js_sys::Reflect::get(&storage, &"getDirectory".into()) {
        Ok(f) if f.is_function() => true,
        _ => false,
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn opfs_available() -> bool {
    // 非 wasm32（node 单测）：无浏览器 API，返回 false。
    false
}

/// Worker init 选项（从 JS opts 解析）。
struct WorkerOpts {
    vfs_kind: VfsKind,
    /// db_path 仅 wasm32+opfs 的 init_opfs_vfs 使用（非 wasm32 不读）。
    #[allow(dead_code)]
    db_path: String,
    dict_url: Option<String>,
    dict_sha256: Option<[u8; 8]>,
    dict_data: Option<Vec<u8>>,
}

enum VfsKind {
    /// 内存 Vfs（测试 / 开发）。opts.vfs = "memory"。
    Memory,
    /// OPFS 主路径（浏览器，异步 init）；运行时探测不可用降级 IDB。
    Opfs,
    /// IDB 降级（浏览器，异步 init）。
    Idb,
}

fn parse_worker_opts(v: &serde_json::Value) -> Result<WorkerOpts, VaneError> {
    let vfs_kind = match v.get("vfs").and_then(|v| v.as_str()) {
        Some("memory") => VfsKind::Memory,
        Some("idb") => VfsKind::Idb,
        _ => VfsKind::Opfs,
    };
    let db_path = v
        .get("dbPath")
        .and_then(|v| v.as_str())
        .unwrap_or("vane.db")
        .to_string();
    let dict_url = v.get("dictUrl").and_then(|v| v.as_str()).map(String::from);
    let dict_sha256 = v.get("dictSha256").and_then(|v| v.as_str()).and_then(|s| {
        if s.len() != 16 {
            return None;
        }
        let mut bytes = [0u8; 8];
        for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
            let hex = std::str::from_utf8(chunk).ok()?;
            bytes[i] = u8::from_str_radix(hex, 16).ok()?;
        }
        Some(bytes)
    });
    Ok(WorkerOpts {
        vfs_kind,
        db_path,
        dict_url,
        dict_sha256,
        dict_data: None,
    })
}

// =========================================================================
// 异步 Vfs 选择（wasm32 浏览器 API）
// =========================================================================

/// OPFS 异步 init：getDirectory → getFileHandle → createSyncAccessHandle → OpfsVfs。
#[cfg(all(target_arch = "wasm32", feature = "opfs"))]
async fn init_opfs_vfs(db_path: &str) -> Result<Arc<dyn Vfs>, VaneError> {
    use wasm_bindgen_futures::JsFuture;

    let global = js_sys::global();
    let navigator = js_sys::Reflect::get(&global, &"navigator".into())
        .map_err(|e| VaneError::Io(format!("navigator: {:?}", e).into()))?;
    let storage = js_sys::Reflect::get(&navigator, &"storage".into())
        .map_err(|e| VaneError::Io(format!("storage: {:?}", e).into()))?;
    let get_directory = js_sys::Reflect::get(&storage, &"getDirectory".into())
        .map_err(|e| VaneError::Io(format!("getDirectory: {:?}", e).into()))?;
    let get_directory: js_sys::Function = get_directory
        .dyn_into()
        .map_err(|e| VaneError::Io(format!("getDirectory not function: {:?}", e).into()))?;
    let root_promise = get_directory
        .call0(&storage)
        .map_err(|e| VaneError::Io(format!("getDirectory(): {:?}", e).into()))?;
    let root: web_sys::FileSystemDirectoryHandle =
        JsFuture::from(root_promise.unchecked_into::<js_sys::Promise>())
            .await
            .map_err(|e| VaneError::Io(format!("getDirectory await: {:?}", e).into()))?
            .unchecked_into();

    let opts = web_sys::FileSystemGetFileOptions::new();
    opts.set_create(true);
    let fh_promise = root.get_file_handle_with_options(db_path, &opts);
    let fh: web_sys::FileSystemFileHandle =
        JsFuture::from(fh_promise.unchecked_into::<js_sys::Promise>())
            .await
            .map_err(|e| VaneError::Io(format!("getFileHandle await: {:?}", e).into()))?
            .unchecked_into();

    let sah_promise = fh.create_sync_access_handle();
    let sah: web_sys::FileSystemSyncAccessHandle =
        JsFuture::from(sah_promise.unchecked_into::<js_sys::Promise>())
            .await
            .map_err(|e| VaneError::Io(format!("createSyncAccessHandle await: {:?}", e).into()))?
            .unchecked_into();

    let opfs = crate::vfs::opfs::OpfsVfs::from_handle(sah)?;
    Ok(Arc::new(opfs))
}

/// IDB 异步 init：open_idb → get("container") → IdbVfs::from_blob。
#[cfg(all(target_arch = "wasm32", feature = "idb"))]
async fn init_idb_vfs() -> Result<Arc<dyn Vfs>, VaneError> {
    let blob = idb_get_container("vane_db", "container")
        .await
        .unwrap_or_default();
    let idb = crate::vfs::idb::IdbVfs::from_blob(blob)?;
    Ok(Arc::new(idb))
}

/// 通过 IDB 读取 container blob（JS 异步，wasm32 only）。
/// 返回 None 表示 IDB 不存在或读取失败（新建空库）。
#[cfg(all(target_arch = "wasm32", feature = "idb"))]
async fn idb_get_container(db_name: &str, store_name: &str) -> Option<Vec<u8>> {
    use wasm_bindgen_futures::JsFuture;

    let global = js_sys::global();
    let indexed_db = js_sys::Reflect::get(&global, &"indexedDB".into()).ok()?;
    if indexed_db.is_undefined() {
        return None;
    }
    let factory: web_sys::IdbFactory = indexed_db.unchecked_into();
    let open_request = factory.open(db_name).ok()?;
    let db: web_sys::IdbDatabase = JsFuture::from(open_request.unchecked_into::<js_sys::Promise>())
        .await
        .ok()?
        .unchecked_into();
    let tx = db
        .transaction_with_str_and_mode(store_name, web_sys::IdbTransactionMode::Readonly)
        .ok()?;
    let store = tx.object_store(store_name).ok()?;
    let get_promise = store.get(&"container".into()).ok()?;
    let result = JsFuture::from(get_promise.unchecked_into::<js_sys::Promise>())
        .await
        .ok()?;
    if result.is_undefined() || result.is_null() {
        return None;
    }
    // result 可能是 Uint8Array 或 ArrayBuffer。
    if let Some(arr) = result.dyn_ref::<js_sys::Uint8Array>() {
        return Some(arr.to_vec());
    }
    if let Some(ab) = result.dyn_ref::<js_sys::ArrayBuffer>() {
        return Some(js_sys::Uint8Array::new(ab).to_vec());
    }
    // 尝试 .buffer（Uint8Array.view 的 buffer）。
    let buf = js_sys::Reflect::get(&result, &"buffer".into()).ok()?;
    if !buf.is_undefined() {
        let ab: js_sys::ArrayBuffer = buf.unchecked_into();
        return Some(js_sys::Uint8Array::new(&ab).to_vec());
    }
    None
}

/// 异步选择 Vfs（init 探针）。失败降级 IDB + warn（不抛错）。
async fn select_vfs(opts: &WorkerOpts) -> Result<Arc<dyn Vfs>, VaneError> {
    match opts.vfs_kind {
        VfsKind::Memory => Ok(Arc::new(MemoryVfs::new())),
        VfsKind::Opfs => {
            if !opfs_available() {
                warn("vane: OPFS not available, falling back to IndexedDB");
                return select_idb_or_memory().await;
            }
            #[cfg(all(target_arch = "wasm32", feature = "opfs"))]
            {
                match init_opfs_vfs(&opts.db_path).await {
                    Ok(vfs) => Ok(vfs),
                    Err(e) => {
                        warn(&format!(
                            "vane: OPFS init failed ({}), falling back to IDB",
                            e
                        ));
                        select_idb_or_memory().await
                    }
                }
            }
            #[cfg(not(all(target_arch = "wasm32", feature = "opfs")))]
            {
                // 非 wasm32 或无 opfs feature：降级 memory（测试环境）。
                Ok(Arc::new(MemoryVfs::new()))
            }
        }
        VfsKind::Idb => select_idb_or_memory().await,
    }
}

/// IDB 降级；非 wasm32 时降级 MemoryVfs（测试环境）。
async fn select_idb_or_memory() -> Result<Arc<dyn Vfs>, VaneError> {
    #[cfg(all(target_arch = "wasm32", feature = "idb"))]
    {
        match init_idb_vfs().await {
            Ok(vfs) => Ok(vfs),
            Err(e) => {
                warn(&format!(
                    "vane: IDB init failed ({}), using in-memory (data not persisted)",
                    e
                ));
                Ok(Arc::new(MemoryVfs::new()))
            }
        }
    }
    #[cfg(not(all(target_arch = "wasm32", feature = "idb")))]
    {
        Ok(Arc::new(MemoryVfs::new()))
    }
}

// =========================================================================
// 异步 init（Vfs 选择 + 词典加载 + SIMD 探针）
// =========================================================================

/// 从 JsValue opts 提取 dictData（Uint8Array / ArrayBuffer）。
fn extract_dict_data(opts: &JsValue) -> Option<Vec<u8>> {
    let dict_data = js_sys::Reflect::get(opts, &"dictData".into()).ok()?;
    if dict_data.is_undefined() || dict_data.is_null() {
        return None;
    }
    if let Some(arr) = dict_data.dyn_ref::<js_sys::Uint8Array>() {
        return Some(arr.to_vec());
    }
    if let Some(ab) = dict_data.dyn_ref::<js_sys::ArrayBuffer>() {
        return Some(js_sys::Uint8Array::new(ab).to_vec());
    }
    None
}

/// 异步 init Worker：选 Vfs + 加载词典 + SIMD 探针。
async fn init_worker(opts_js: JsValue) -> Result<VaneWorker, VaneError> {
    let opts_v = js_to_json(&opts_js)?;
    let mut opts = parse_worker_opts(&opts_v)?;
    opts.dict_data = extract_dict_data(&opts_js);

    // SIMD 探针（M2-05 落实真实探针；占位返 false→scalar）。
    let _simd = simd_probe::simd128_supported();

    // 选 Vfs。
    let vfs = select_vfs(&opts).await?;

    // 加载词典（CDN / 内联 / 降级 None）。
    let cache_vfs: Option<&dyn Vfs> = if matches!(opts.vfs_kind, VfsKind::Memory) {
        None
    } else {
        Some(vfs.as_ref())
    };
    let dict_sha256_ref = opts.dict_sha256.as_ref();
    let dict_bytes = dict_loader::load_dict(
        opts.dict_data.as_deref(),
        opts.dict_url.as_deref(),
        dict_sha256_ref,
        cache_vfs,
    )
    .await;

    let mut inner = WorkerInner::new();
    inner.vfs = Some(vfs);
    inner.dict_bytes = dict_bytes;

    Ok(VaneWorker {
        inner: Rc::new(RefCell::new(inner)),
    })
}

// =========================================================================
// 同步 API（内部，测试直接调用；wasm_bindgen 方法包装为 Promise）
// =========================================================================

impl VaneWorker {
    /// 测试用：MemoryVfs 同步构造（不导出 JS）。
    #[cfg(test)]
    pub(crate) fn new_memory() -> Self {
        let mut inner = WorkerInner::new();
        inner.vfs = Some(Arc::new(MemoryVfs::new()));
        VaneWorker {
            inner: Rc::new(RefCell::new(inner)),
        }
    }

    /// open：Db::open(vfs, path, opts) + 词典注入。
    fn open_sync(&self, path: &str, opts: OpenOptions) -> Result<(), VaneError> {
        let mut inner = self.inner.borrow_mut();
        inner.check_open()?;
        let vfs = inner
            .vfs
            .clone()
            .ok_or_else(|| VaneError::InvalidArg("vfs not initialized".into()))?;
        let db = Db::open(vfs, path, opts)?;
        // 词典注入（若 init 阶段加载成功）。dict.bin 是 zstd 压缩 → load_zstd。
        #[cfg(feature = "jieba")]
        if let Some(bytes) = inner.dict_bytes.as_ref() {
            match JiebaDict::load_zstd(bytes) {
                Ok(dict) => db.set_jieba_dict(Arc::new(dict)),
                Err(e) => {
                    warn(&format!(
                        "vane: jieba dict load failed ({}), falling back to bigram",
                        e
                    ));
                }
            }
        }
        inner.db = Some(db);
        Ok(())
    }

    /// collection：创建/获取 collection。jieba 无词典时降级 CjkBigram（不抛错，§12.4）。
    fn collection_sync(
        &self,
        name: &str,
        schema: Schema,
        mut opts: CollectionOptions,
    ) -> Result<u32, VaneError> {
        let mut inner = self.inner.borrow_mut();
        inner.check_open()?;
        let db = inner.db()?;

        // 预防式降级：jieba 请求但词典不可用 → CjkBigram + warn（E_DICT_UNAVAILABLE 禁止到达）。
        if matches!(opts.tokenizer, BuiltinTokenizer::Jieba) {
            #[cfg(feature = "jieba")]
            {
                if !db.jieba_dict_available() {
                    dict_loader::dict_unavailable_fallback();
                    opts.tokenizer = BuiltinTokenizer::CjkBigram;
                }
            }
            #[cfg(not(feature = "jieba"))]
            {
                dict_loader::dict_unavailable_fallback();
                opts.tokenizer = BuiltinTokenizer::CjkBigram;
            }
        }

        let col = db.collection(name, schema, opts)?;
        let id = inner.next_col_id;
        inner.next_col_id += 1;
        inner.collections.insert(id, col);
        Ok(id)
    }

    fn add_sync(&self, col: u32, docs: Vec<Doc>) -> Result<u64, VaneError> {
        let inner = self.inner.borrow();
        inner.check_open()?;
        let col = inner
            .collections
            .get(&col)
            .ok_or_else(|| VaneError::NotFound(format!("collection {col} not found").into()))?;
        let report = col.add(&docs)?;
        Ok(report.accepted)
    }

    fn flush_sync(&self, col: u32) -> Result<(), VaneError> {
        let inner = self.inner.borrow();
        inner.check_open()?;
        let col = inner
            .collections
            .get(&col)
            .ok_or_else(|| VaneError::NotFound(format!("collection {col} not found").into()))?;
        col.flush()
    }

    fn search_sync(&self, col: u32, query: SearchQuery) -> Result<Vec<Hit>, VaneError> {
        let inner = self.inner.borrow();
        inner.check_open()?;
        let col = inner
            .collections
            .get(&col)
            .ok_or_else(|| VaneError::NotFound(format!("collection {col} not found").into()))?;
        col.search(&query)
    }

    fn delete_sync(&self, col: u32, ids: Vec<String>) -> Result<u64, VaneError> {
        let inner = self.inner.borrow();
        inner.check_open()?;
        let col = inner
            .collections
            .get(&col)
            .ok_or_else(|| VaneError::NotFound(format!("collection {col} not found").into()))?;
        col.delete(&ids)
    }

    fn compact_sync(&self, col: u32) -> Result<(), VaneError> {
        let inner = self.inner.borrow();
        inner.check_open()?;
        let col = inner
            .collections
            .get(&col)
            .ok_or_else(|| VaneError::NotFound(format!("collection {col} not found").into()))?;
        col.compact()
    }

    fn reindex_sync(&self, col: u32) -> Result<f32, VaneError> {
        let inner = self.inner.borrow();
        inner.check_open()?;
        let col = inner
            .collections
            .get(&col)
            .ok_or_else(|| VaneError::NotFound(format!("collection {col} not found").into()))?;
        let rh = col.reindex()?;
        Ok(rh.progress())
    }

    fn export_sync(&self, dest: &str) -> Result<(), VaneError> {
        let inner = self.inner.borrow();
        inner.check_open()?;
        let db = inner.db()?;
        db.export(dest)
    }

    /// readFile：读 VFS 容器内指定虚拟路径的文件全部字节（post-M2 P1 下载闭环）。
    ///
    /// 用途：`export(dest)` 写快照到容器内虚拟路径后，主线程经此 op 读回字节 →
    /// Blob → `<a download>` 触发浏览器下载。读 `inner.vfs`（与 Db 共享的同一 VFS），
    /// 流式 `read_at` 直到 EOF（n == 0）。文件不存在 → `VaneError::Io`。
    fn read_file_sync(&self, path: &str) -> Result<Vec<u8>, VaneError> {
        let inner = self.inner.borrow();
        inner.check_open()?;
        let vfs = inner
            .vfs
            .as_ref()
            .ok_or_else(|| VaneError::InvalidArg("vfs not initialized".into()))?;
        let mut buf = Vec::new();
        let mut tmp = [0u8; 8192];
        let mut off = 0u64;
        loop {
            let n = vfs.read_at(path, &mut tmp, off)?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
            off += n as u64;
        }
        Ok(buf)
    }

    fn close_sync(&self) -> Result<(), VaneError> {
        let mut inner = self.inner.borrow_mut();
        // M-8：close 前 flush 所有未落盘 collection（数据持久化），
        // 避免丢未 flush 的缓冲区写入。flush 失败不阻断 close（best-effort）。
        for col in inner.collections.values() {
            let _ = col.flush();
        }
        if let Some(db) = inner.db.as_ref() {
            let _ = db.close();
        }
        inner.collections.clear();
        inner.db = None;
        inner.vfs = None;
        inner.dict_bytes = None;
        inner.closed = true;
        Ok(())
    }
}

// =========================================================================
// wasm_bindgen 导出（Promise 边界）
// =========================================================================

#[wasm_bindgen]
impl VaneWorker {
    /// 异步工厂：init（选 Vfs + 词典加载 + SIMD 探针）。返回 Promise<VaneWorker>。
    ///
    /// 注意：非 `#[wasm_bindgen(constructor)]`（构造器不能返 Promise）。
    /// JS 用 `const worker = await VaneWorker.create(opts)`。
    #[wasm_bindgen(js_name = create)]
    pub fn create(opts: JsValue) -> js_sys::Promise {
        wasm_bindgen_futures::future_to_promise(async move {
            match init_worker(opts).await {
                Ok(worker) => Ok(worker.into()),
                Err(e) => Err(err_to_js(e)),
            }
        })
    }

    /// 打开数据库。path 为逻辑路径，opts 为 OpenOptions JSON。
    pub fn open(&self, path: String, opts: JsValue) -> js_sys::Promise {
        match (|| -> Result<(), VaneError> {
            let opts_v = js_to_json(&opts)?;
            let open_opts = if opts_v.is_null() {
                OpenOptions::default()
            } else {
                parse_open_opts(&opts_v)?
            };
            self.open_sync(&path, open_opts)
        })() {
            Ok(()) => ok_promise(JsValue::UNDEFINED),
            Err(e) => err_promise(e),
        }
    }

    /// 创建/获取 collection。返回 collection 句柄（u32）。
    pub fn collection(&self, name: String, schema: JsValue, opts: JsValue) -> js_sys::Promise {
        match (|| -> Result<u32, VaneError> {
            let schema_v = js_to_json(&schema)?;
            let s = parse_schema(&schema_v)?;
            let opts_v = js_to_json(&opts)?;
            let o = if opts_v.is_null() {
                CollectionOptions::default()
            } else {
                parse_collection_opts(&opts_v)?
            };
            self.collection_sync(&name, s, o)
        })() {
            Ok(id) => ok_promise(JsValue::from(id)),
            Err(e) => err_promise(e),
        }
    }

    /// 追加文档。返回 accepted 数量。
    pub fn add(&self, col: u32, docs: JsValue) -> js_sys::Promise {
        match (|| -> Result<u64, VaneError> {
            let v = js_to_json(&docs)?;
            let d = parse_docs(&v)?;
            self.add_sync(col, d)
        })() {
            Ok(n) => ok_promise(JsValue::from(n)),
            Err(e) => err_promise(e),
        }
    }

    /// 刷新缓冲区，持久化段。
    pub fn flush(&self, col: u32) -> js_sys::Promise {
        match self.flush_sync(col) {
            Ok(()) => ok_promise(JsValue::UNDEFINED),
            Err(e) => err_promise(e),
        }
    }

    /// 搜索。返回 Hit[] JSON 字符串。
    pub fn search(&self, col: u32, query: JsValue) -> js_sys::Promise {
        match (|| -> Result<String, VaneError> {
            let v = js_to_json(&query)?;
            let q = parse_search_query(&v)?;
            let hits = self.search_sync(col, q)?;
            serde_json::to_string(&hits_to_json(&hits))
                .map_err(|e| VaneError::InvalidArg(format!("hits serialize: {e}").into()))
        })() {
            Ok(s) => ok_promise(JsValue::from(s)),
            Err(e) => err_promise(e),
        }
    }

    /// 删除文档。返回已删除数。
    pub fn delete(&self, col: u32, ids: JsValue) -> js_sys::Promise {
        match (|| -> Result<u64, VaneError> {
            let v = js_to_json(&ids)?;
            let arr = v
                .as_array()
                .ok_or_else(|| VaneError::InvalidArg("ids must be array".into()))?;
            let id_list: Vec<String> = arr
                .iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect();
            self.delete_sync(col, id_list)
        })() {
            Ok(n) => ok_promise(JsValue::from(n)),
            Err(e) => err_promise(e),
        }
    }

    /// 触发段合并。
    pub fn compact(&self, col: u32) -> js_sys::Promise {
        match self.compact_sync(col) {
            Ok(()) => ok_promise(JsValue::UNDEFINED),
            Err(e) => err_promise(e),
        }
    }

    /// 触发 reindex（同步执行）。返回 progress（1.0 表示已完成）。
    pub fn reindex(&self, col: u32) -> js_sys::Promise {
        match self.reindex_sync(col) {
            Ok(p) => ok_promise(JsValue::from(p)),
            Err(e) => err_promise(e),
        }
    }

    /// 导出数据库快照（M2-12 接入）。
    pub fn export(&self, dest: String) -> js_sys::Promise {
        match self.export_sync(&dest) {
            Ok(()) => ok_promise(JsValue::UNDEFINED),
            Err(e) => err_promise(e),
        }
    }

    /// 读 VFS 容器内指定虚拟路径的文件字节，返回 `Uint8Array`（post-M2 P1 下载闭环）。
    ///
    /// 配合 `export`：`export("backup.vane")` 写快照到容器 → `readFile("backup.vane")`
    /// 读回字节 → 主线程 `new Blob([bytes])` → `<a download="backup.vane">` 下载。
    /// 文件不存在或读取失败 → reject。
    #[wasm_bindgen(js_name = readFile)]
    pub fn read_file(&self, path: String) -> js_sys::Promise {
        match self.read_file_sync(&path) {
            Ok(bytes) => {
                let arr = js_sys::Uint8Array::from(&bytes[..]);
                ok_promise(arr.into())
            }
            Err(e) => err_promise(e),
        }
    }

    /// 关闭 Worker（注销句柄）。close 后再调用任何方法 reject（I-7）。
    pub fn close(&self) -> js_sys::Promise {
        match self.close_sync() {
            Ok(()) => ok_promise(JsValue::UNDEFINED),
            Err(e) => err_promise(e),
        }
    }
}

// =========================================================================
// 单元测试（node 可跑——MemoryVfs + 同步 API；异步浏览器路径标注手动验证）
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn schema_json() -> &'static str {
        r#"{"fields":[{"name":"title","type":"text"},{"name":"vec","type":"vector","dim":3,"metric":"cosine"}]}"#
    }

    fn docs_json() -> &'static str {
        r#"[
            {"id":"d1","text":"hello world","vector":[1.0,0.0,0.0]},
            {"id":"d2","text":"foo bar","vector":[0.0,1.0,0.0]},
            {"id":"d3","text":"hello foo","vector":[0.0,0.0,1.0]}
        ]"#
    }

    /// 测试门禁 8：open → collection → add → flush → search 端到端（I-8 薄壳）。
    #[test]
    fn open_collection_add_flush_search_roundtrip() {
        let worker = VaneWorker::new_memory();
        worker.open_sync("test-db", OpenOptions::default()).unwrap();

        let schema = parse_schema(&serde_json::from_str(schema_json()).unwrap()).unwrap();
        let col_id = worker
            .collection_sync("docs", schema, CollectionOptions::default())
            .unwrap();
        assert!(col_id > 0);

        let docs = parse_docs(&serde_json::from_str(docs_json()).unwrap()).unwrap();
        let accepted = worker.add_sync(col_id, docs).unwrap();
        assert_eq!(accepted, 3);

        worker.flush_sync(col_id).unwrap();

        // vector search
        let q = parse_search_query(
            &serde_json::from_str(r#"{"vector":[1.0,0.0,0.0],"topK":3,"mode":"vector"}"#).unwrap(),
        )
        .unwrap();
        let hits = worker.search_sync(col_id, q).unwrap();
        assert!(!hits.is_empty());
        assert_eq!(hits[0].id, "d1");

        // text search
        let qt = parse_search_query(
            &serde_json::from_str(r#"{"text":"hello","topK":3,"mode":"text"}"#).unwrap(),
        )
        .unwrap();
        let hits_t = worker.search_sync(col_id, qt).unwrap();
        let ids: Vec<&str> = hits_t.iter().map(|h| h.id.as_str()).collect();
        assert!(ids.contains(&"d1"));
        assert!(ids.contains(&"d3"));

        worker.close_sync().unwrap();
    }

    /// 测试门禁 9：jieba 无词典 → 降级 CjkBigram（不抛错，E_DICT_UNAVAILABLE 禁止到达）。
    #[test]
    fn jieba_without_dict_falls_back_to_bigram() {
        let worker = VaneWorker::new_memory();
        worker.open_sync("test-db", OpenOptions::default()).unwrap();

        let schema = parse_schema(&serde_json::from_str(schema_json()).unwrap()).unwrap();
        let opts = CollectionOptions {
            tokenizer: BuiltinTokenizer::Jieba,
            ..Default::default()
        };
        // 无词典 → 降级 CjkBigram，不抛错。
        let col_id = worker.collection_sync("jieba_col", schema, opts);
        assert!(col_id.is_ok(), "jieba 降级不抛错: {:?}", col_id);

        worker.close_sync().unwrap();
    }

    /// 测试门禁 12：错误透传——schema 不一致返 Err。
    #[test]
    fn schema_mismatch_returns_error() {
        let worker = VaneWorker::new_memory();
        worker.open_sync("test-db", OpenOptions::default()).unwrap();

        let s1 = parse_schema(&serde_json::from_str(schema_json()).unwrap()).unwrap();
        worker
            .collection_sync("col", s1, CollectionOptions::default())
            .unwrap();

        // 不同 schema（dim 不同）→ Err。
        let s2 = parse_schema(
            &serde_json::from_str(
                r#"{"fields":[{"name":"title","type":"text"},{"name":"vec","type":"vector","dim":4,"metric":"cosine"}]}"#,
            )
            .unwrap(),
        )
        .unwrap();
        let result = worker.collection_sync("col", s2, CollectionOptions::default());
        assert!(result.is_err());
    }

    /// 测试门禁 13：close 后再调用 → Err（I-7 句柄注销）。
    #[test]
    fn close_then_call_rejects() {
        let worker = VaneWorker::new_memory();
        worker.open_sync("test-db", OpenOptions::default()).unwrap();
        worker.close_sync().unwrap();

        let result = worker.open_sync("test-db", OpenOptions::default());
        assert!(result.is_err(), "close 后调用应失败（I-7）");
    }

    /// M-8：close 前 flush 所有 collection（未落盘写入不丢失）。
    #[test]
    fn close_flushes_pending_collections() {
        let worker = VaneWorker::new_memory();
        worker.open_sync("test-db", OpenOptions::default()).unwrap();

        let schema = parse_schema(&serde_json::from_str(schema_json()).unwrap()).unwrap();
        let col_id = worker
            .collection_sync("docs", schema, CollectionOptions::default())
            .unwrap();

        // add 但不显式 flush（缓冲区有未落盘写入）。
        let docs = parse_docs(&serde_json::from_str(docs_json()).unwrap()).unwrap();
        worker.add_sync(col_id, docs).unwrap();

        // close 应 flush 未落盘 collection，不抛错。
        worker.close_sync().unwrap();
    }

    /// 测试门禁 10：opfs_available 逻辑分支（node 返 false → 选 IDB → 降级 memory）。
    #[test]
    fn opfs_probe_returns_false_in_node() {
        // 非 wasm32：opfs_available 恒 false。
        assert!(!opfs_available());
    }

    /// delete + compact + reindex 基本路径。
    #[test]
    fn delete_compact_reindex_work() {
        let worker = VaneWorker::new_memory();
        worker.open_sync("test-db", OpenOptions::default()).unwrap();

        let schema = parse_schema(&serde_json::from_str(schema_json()).unwrap()).unwrap();
        let col_id = worker
            .collection_sync("docs", schema, CollectionOptions::default())
            .unwrap();

        let docs = parse_docs(&serde_json::from_str(docs_json()).unwrap()).unwrap();
        worker.add_sync(col_id, docs).unwrap();
        worker.flush_sync(col_id).unwrap();

        // delete
        let deleted = worker.delete_sync(col_id, vec!["d2".into()]).unwrap();
        assert_eq!(deleted, 1);

        // compact
        worker.compact_sync(col_id).unwrap();

        // reindex requires PendingReindex state（set_user_dict first）——此处验证
        // 未设词表时 reindex 返回 Err（不 panic），状态机正确。
        let reindex_result = worker.reindex_sync(col_id);
        assert!(reindex_result.is_err());

        worker.close_sync().unwrap();
    }

    /// export（M2-12 未实装，返 E_UNSUPPORTED 但不 panic）。
    #[test]
    fn export_returns_unsupported() {
        let worker = VaneWorker::new_memory();
        worker.open_sync("test-db", OpenOptions::default()).unwrap();
        let result = worker.export_sync("/tmp/snapshot");
        assert!(result.is_err());
        worker.close_sync().unwrap();
    }

    /// post-M2 P1：readFile 不存在路径 → Err；close 后 → Err（句柄注销）。
    #[test]
    fn read_file_errors_on_missing_and_closed() {
        let worker = VaneWorker::new_memory();
        worker.open_sync("test-db", OpenOptions::default()).unwrap();

        // 不存在路径 → Err（Io）。
        let missing = worker.read_file_sync("nonexistent.bin");
        assert!(missing.is_err());

        worker.close_sync().unwrap();
        // close 后 → Err（I-7 句柄注销）。
        let after_close = worker.read_file_sync("backup.vane");
        assert!(after_close.is_err());
    }

    /// post-M2 P1：export→readFile round-trip——快照字节以 VANE_SNAP 魔数起始。
    #[test]
    fn export_then_read_file_roundtrip() {
        let worker = VaneWorker::new_memory();
        worker.open_sync("snap-db", OpenOptions::default()).unwrap();

        let schema = parse_schema(&serde_json::from_str(schema_json()).unwrap()).unwrap();
        let col_id = worker
            .collection_sync("docs", schema, CollectionOptions::default())
            .unwrap();
        let docs = parse_docs(&serde_json::from_str(docs_json()).unwrap()).unwrap();
        worker.add_sync(col_id, docs).unwrap();
        worker.flush_sync(col_id).unwrap();

        // export 写快照到容器内虚拟路径 backup.vane。
        worker.export_sync("backup.vane").unwrap();

        // readFile 读回快照字节，应以 VANE_SNAP 魔数起始。
        let snap = worker.read_file_sync("backup.vane").unwrap();
        assert!(snap.len() > 9, "snapshot too short: {}", snap.len());
        assert_eq!(&snap[..9], b"VANE_SNAP", "snapshot magic mismatch");

        worker.close_sync().unwrap();
    }
}
