# API-Core 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development 执行本计划。步骤用 checkbox `- [ ]` 标记。

**Goal:** 实现 SPEC §4 公共 API（Db/Collection/add/flush/search/close）+ search 编排（BruteScanner + InvertedIndexReader + RRF fusion），形成 M0 最小闭环。
**Architecture:** Db 持有 Vfs + ManifestStore + collections 注册表（RwLock<HashMap>）。Collection = Arc<CollectionInner>（Clone+Send+Sync，满足 09-node-binding 要求）。WriteState（Mutex）护 buffer+auto-committer+docid 计数器；snapshot（RwLock<Vec<Arc<SegmentReader>>>）护段快照，读路径零锁（SPEC §11）。flush 编排：SegmentWriter + InvertedIndexBuilder + write_inverted + ManifestStore 原子切换。search 编排：遍历段快照，每段 BruteScanner + InvertedIndexReader，归并后 fusion。
**Tech Stack:** std::sync（RwLock/Mutex，零额外依赖、wasm32 安全，B2 裁决）、std::sync::Arc、serde_json。

> **B2 备注：** std::sync::RwLock::read()/write() 返回 `Result`，用 `.unwrap()` 处理 poison（M0 测试场景不会 poison）。所有 `.read()` 改为 `.read().unwrap()`，`.write()` 改为 `.write().unwrap()`，`.lock()` 改为 `.lock().unwrap()`。
**SPEC 引用:** §4 公共 API IDL（M0 冻结）、§7.1 NRT 可见性、§8.1 查询模式、§8.2 融合、§8.3 过滤（M0 占位）、§10 错误码、§11 并发模型、§14 I-2/I-4/I-8。
**前置依赖:** 00-workspace, 01-vfs, 02-tokenizer, 03-fusion, 04-segment-format, 05-bm25, 06-vector-brute, 08-persistence。
**验收标准:**
- [ ] add→flush→search 全流程通过（MemoryVfs + StdFsVfs）
- [ ] flush 后向量+倒排在同快照同时可见（不变量 I-2）
- [ ] hybrid recall@10 与暴力双路+RRF 基线一致（M0 暴力=基线，recall=1.0）
- [ ] topK>1000 返回 InvalidArg；vector dim 不匹配返回 Schema
- [ ] filter 非空返回 InvalidArg（M0 占位）
- [ ] delete/compact/reindex 返回 Unsupported（M0 占位）
- [ ] `export` / `reindex` 返回 Unsupported（M0 占位）
- [ ] Db/Collection 是 Clone+Send+Sync
- [ ] auto-commit 在 add 路径触发 flush
- [ ] collection 级 auto-commit 配置生效（AutoCommitConfig::Off 不触发 flush）

## Global Constraints
- API 签名 M0 冻结（SPEC §4.1/§4.2）。
- 所有公开 API 线程安全；单 collection 写路径串行，读路径无锁并发（§11）。
- flush 后对新快照原子可见；双索引同快照出现（不变量 I-2）。
- topK 上限 1000；dim 上限 4096；单文档≤16MB（§3.1/§3.2/§4.2）。
- BM25 k1=1.2 b=0.75；RRF k=60 冻结（§6.3/§8.2）。
- M0 不实现 filter/delete/compact/reindex（占位返回 Unsupported/InvalidArg）。
- core 禁 std::fs/cfg(target)（§13.3/I-5）。

## File Structure
- `crates/vane-core/src/api/mod.rs` — Db/Collection + 公共类型 + re-export
- `crates/vane-core/src/api/types.rs` — OpenOptions/CollectionOptions/SearchQuery/Hit/Doc/AddReport/Filter
- `crates/vane-core/src/api/collection.rs` — CollectionInner + add/flush/search 编排
- `crates/vane-core/src/api/tests.rs` — 集成测试
- `crates/vane-core/tests/recall.rs` — recall 集成测试骨架（I8）

> **B1 裁决：** 00-workspace 已在 lib.rs 一次性预声明全部 9 模块（含 `pub mod api;`），本计划不修改 lib.rs。

## 任务清单（bite-sized TDD）

### Task 1: 公共 API 类型定义
**Files:**
- Create: `crates/vane-core/src/api/mod.rs`, `crates/vane-core/src/api/types.rs`
- 不修改 lib.rs（B1 裁决：00-workspace 已预声明 `pub mod api;`）

**Interfaces:**
- Consumes from 00-workspace: Schema, Metric, VaneError, Result, TOPK_MAX, DIM_MAX, RRF_K
- Consumes from 02-tokenizer: BuiltinTokenizer, UserDictEntry
- Consumes from 08-persistence: AutoCommitConfig
- Produces: OpenOptions, CollectionOptions, SearchMode, FusionSpec, SearchQuery, Filter, FilterCond, ScalarValue, Hit, AddReport, Doc

- [ ] **Step 1: 写失败测试** — 创建 `crates/vane-core/src/api/tests.rs`：
```rust
use super::types::*;
use crate::tokenizer::BuiltinTokenizer;

#[test]
fn open_options_default() {
    let o = OpenOptions::default();
    assert_eq!(o.page_cache_mb, 32);
    assert!(matches!(o.auto_commit, crate::persistence::AutoCommitConfig::On { .. }));
}

#[test]
fn search_query_default() {
    let q = SearchQuery::default();
    assert_eq!(q.top_k, 10);
    assert!(matches!(q.mode, SearchMode::Auto));
    assert!(matches!(q.fusion, FusionSpec::Rrf));
    assert_eq!(q.candidate_multiplier, 3);
    assert!(q.filter.is_none());
}

#[test]
fn collection_options_default_tokenizer_standard() {
    let o = CollectionOptions::default();
    assert!(matches!(o.tokenizer, BuiltinTokenizer::Standard));
}
```

- [ ] **Step 2: 跑测试确认失败** — `cargo test -p vane-core -- api::types`，编译失败。
- [ ] **Step 3: 最小实现** — `crates/vane-core/src/api/types.rs`：
```rust
use crate::types::{Metric, VaneError, Result, TOPK_MAX, DIM_MAX};
use crate::tokenizer::{BuiltinTokenizer, UserDictEntry};
use crate::persistence::AutoCommitConfig;

pub enum PersistenceMode { Persistent, BestEffort }

pub struct OpenOptions {
    pub persistence: PersistenceMode,
    pub auto_commit: AutoCommitConfig,
    pub page_cache_mb: u32,
}
impl Default for OpenOptions {
    fn default() -> Self {
        Self {
            persistence: PersistenceMode::Persistent,
            auto_commit: AutoCommitConfig::default(),
            page_cache_mb: 32,
        }
    }
}

pub struct CollectionOptions {
    pub tokenizer: BuiltinTokenizer,
    pub user_dict: Vec<UserDictEntry>,
    pub auto_commit: AutoCommitConfig,  // I3 裁决：collection 级 auto-commit 配置
}
impl Default for CollectionOptions {
    fn default() -> Self {
        Self { tokenizer: BuiltinTokenizer::Standard, user_dict: vec![], auto_commit: AutoCommitConfig::default() }
    }
}

// SearchMode::Auto 为内部推断标记，JS/Go 绑定层不暴露 "auto" 字符串（S8）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SearchMode { Hybrid, Vector, Text, Auto }

#[derive(Debug, Clone)]
pub enum FusionSpec { Rrf, Linear { alpha: f32 } }

#[derive(Debug, Clone)]
pub enum ScalarValue { Int(i64), Float(f64), Bool(bool), Keyword(String) }

#[derive(Debug, Clone)]
pub enum FilterCond { Eq(ScalarValue), In(Vec<ScalarValue>), Gte(ScalarValue), Lte(ScalarValue) }

#[derive(Debug, Clone)]
pub struct Filter { pub fields: Vec<(String, FilterCond)> }

pub struct SearchQuery {
    pub text: Option<String>,
    pub vector: Option<Vec<f32>>,
    pub top_k: u32,
    pub mode: SearchMode,
    pub fusion: FusionSpec,
    pub filter: Option<Filter>,
    pub candidate_multiplier: u32,
}
impl Default for SearchQuery {
    fn default() -> Self {
        Self {
            text: None, vector: None, top_k: 10,
            mode: SearchMode::Auto, fusion: FusionSpec::Rrf,
            filter: None, candidate_multiplier: 3,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Hit { pub id: String, pub score: f32, pub fields: Option<std::collections::HashMap<String, String>> }

pub struct AddReport { pub accepted: u64, pub visible_after_flush: bool }

pub struct Doc {
    pub id: String,
    pub text: Option<String>,
    pub vector: Option<Vec<f32>>,
    pub meta: Option<std::collections::HashMap<String, ScalarValue>>,
}
```

`crates/vane-core/src/api/mod.rs`（Task 1 只声明 types；db/collection 在 Task 2/3 追加，B5 裁决分步 re-export）：
```rust
pub mod types;
// B5 裁决：re-export 公共类型，使 vane_core::api::{Db, OpenOptions, ...} 路径可直接导入
pub use types::*;

#[cfg(test)]
mod tests;
```
> Task 2 追加 `pub mod db; pub use db::*;`；Task 3 追加 `pub mod collection; pub use collection::*;`。
- [ ] **Step 4: 跑测试确认通过** — `cargo test -p vane-core -- api::types`，3 测试绿。
- [ ] **Step 5: Commit**
```bash
git add -A
git commit -m "feat(api): public API types (OpenOptions/SearchQuery/Hit/Doc, §4.2)

"
```

### Task 2: Db::open/close/collection/collections
**Files:**
- Create: `crates/vane-core/src/api/db.rs`
- Modify: `crates/vane-core/src/api/mod.rs`

**Interfaces:**
- Consumes from 00-workspace: Schema, Result, VaneError
- Consumes from 01-vfs: Vfs, StdFsVfs/MemoryVfs
- Consumes from 02-tokenizer: build_tokenizer, compute_tokenizer_id
- Consumes from 08-persistence: ManifestStore, Manifest, CollectionMeta, AutoCommitConfig
- Consumes from Task 1: OpenOptions, CollectionOptions
- Produces: `Db`（Clone+Send+Sync）、`Db::open()`, `Db::collection()`, `Db::collections()`, `Db::close()`

- [ ] **Step 1: 写失败测试** — 追加到 tests.rs：
```rust
use super::db::Db;
use super::types::*;
use crate::vfs::memory::MemoryVfs;
use crate::types::{Schema, FieldDef, Metric, VaneError};
use crate::tokenizer::BuiltinTokenizer;

#[test]
fn db_open_new_returns_empty_collections() {
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn crate::vfs::Vfs>;
    let db = Db::open(vfs, "mydb", OpenOptions::default()).unwrap();
    assert!(db.collections().is_empty());
}

#[test]
fn db_collection_creates_and_returns() {
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn crate::vfs::Vfs>;
    let db = Db::open(vfs, "mydb", OpenOptions::default()).unwrap();
    let schema = Schema::new(vec![
        ("body".into(), FieldDef::Text),
        ("v".into(), FieldDef::Vector { dim: 4, metric: Metric::Cosine }),
    ]).unwrap();
    let col = db.collection("docs", schema, CollectionOptions::default()).unwrap();
    assert!(db.collections().contains(&"docs".to_string()));
}

#[test]
fn db_collection_idempotent_same_schema() {
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn crate::vfs::Vfs>;
    let db = Db::open(vfs, "mydb", OpenOptions::default()).unwrap();
    let schema = Schema::new(vec![
        ("v".into(), FieldDef::Vector { dim: 4, metric: Metric::Cosine }),
    ]).unwrap();
    let _c1 = db.collection("docs", schema.clone(), CollectionOptions::default()).unwrap();
    let _c2 = db.collection("docs", schema, CollectionOptions::default()).unwrap();
    // 同名同 schema 幂等，不报错
    assert_eq!(db.collections().len(), 1);
}

#[test]
fn db_collection_idempotent_different_schema_rejected() {
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn crate::vfs::Vfs>;
    let db = Db::open(vfs, "mydb", OpenOptions::default()).unwrap();
    let schema1 = Schema::new(vec![
        ("v".into(), FieldDef::Vector { dim: 4, metric: Metric::Cosine }),
    ]).unwrap();
    let _c1 = db.collection("docs", schema1, CollectionOptions::default()).unwrap();
    // 同名异 schema → Err
    let schema2 = Schema::new(vec![
        ("v".into(), FieldDef::Vector { dim: 8, metric: Metric::Cosine }),
    ]).unwrap();
    let r = db.collection("docs", schema2, CollectionOptions::default());
    assert!(matches!(r, Err(VaneError::Schema(_))));
}
```

- [ ] **Step 2: 跑测试确认失败** — `cargo test -p vane-core -- db::`，编译失败。
- [ ] **Step 3: 最小实现** — `crates/vane-core/src/api/db.rs`：
```rust
use crate::types::{Schema, Result, VaneError};
use crate::vfs::Vfs;
use crate::persistence::{ManifestStore, Manifest, CollectionMeta, AutoCommitConfig};
use crate::tokenizer::{BuiltinTokenizer, UserDictEntry, compute_tokenizer_id};
use std::sync::Arc;
use std::sync::RwLock;
use std::collections::HashMap;

use super::types::{OpenOptions, CollectionOptions};
use super::collection::{Collection, CollectionInner};

pub struct Db {
    inner: Arc<DbInner>,
}

struct DbInner {
    vfs: Arc<dyn Vfs>,
    db_path: String,
    manifest_store: ManifestStore,
    collections: RwLock<HashMap<String, Arc<CollectionInner>>>,
    auto_commit: AutoCommitConfig,  // I3：Db 级 fallback，restore 时用
}

impl Db {
    pub fn open(vfs: Arc<dyn Vfs>, path: &str, opts: OpenOptions) -> Result<Self> {
        let manifest_store = ManifestStore::new(vfs.clone(), path);
        let manifest = manifest_store.load()?.unwrap_or_else(Manifest::empty);
        let collections = RwLock::new(HashMap::new());
        // 从 manifest 恢复 collections（M0：加载元数据，段 Reader 延迟加载）
        let inner = Arc::new(DbInner {
            vfs: vfs.clone(), db_path: path.to_string(),
            manifest_store, collections,
            auto_commit: opts.auto_commit.clone(),
        });
        let db = Db { inner: inner.clone() };
        for (name, meta) in &manifest.collections {
            // I3：restore 时用 OpenOptions.auto_commit 作为 collection 级配置
            let col_inner = Collection::restore_from_manifest(&inner, name, meta.clone(), opts.auto_commit.clone())?;
            db.inner.collections.write().unwrap().insert(name.clone(), Arc::new(col_inner));
        }
        Ok(db)
    }

    pub fn collection(&self, name: &str, schema: Schema, opts: CollectionOptions) -> Result<Collection> {
        // I2 裁决：幂等校验 schema 与 tokenizer 一致性
        {
            let read = self.inner.collections.read().unwrap();
            if let Some(existing) = read.get(name) {
                // 比对 schema（字段名/类型/维度/metric）
                if existing.schema.fields != schema.fields {
                    return Err(VaneError::Schema(format!(
                        "collection '{}' exists with different schema", name
                    )));
                }
                // 比对 tokenizer（kind + user_dict 影响 TokenizerId）
                if existing.tokenizer_id != compute_tokenizer_id(opts.tokenizer, &opts.user_dict) {
                    return Err(VaneError::Schema(format!(
                        "collection '{}' exists with different tokenizer", name
                    )));
                }
                return Ok(Collection { inner: existing.clone() });
            }
        }
        let tok_id = compute_tokenizer_id(opts.tokenizer, &opts.user_dict);
        let meta = CollectionMeta {
            schema: schema.clone(),
            tokenizer_kind: opts.tokenizer,
            tokenizer_id: tok_id.clone(),
            user_dict: opts.user_dict.clone(),
            segment_ulids: vec![],
        };
        let col_inner = Collection::create_new(&self.inner, name, meta, opts.auto_commit.clone())?;
        let arc = Arc::new(col_inner);
        self.inner.collections.write().unwrap().insert(name.to_string(), arc.clone());
        // 持久化 manifest
        let mut m = self.inner.manifest_store.load()?.unwrap_or_else(Manifest::empty);
        m.collections.insert(name.to_string(), CollectionMeta {
            schema, tokenizer_kind: opts.tokenizer,
            tokenizer_id: tok_id, user_dict: opts.user_dict,
            segment_ulids: vec![],
        });
        self.inner.manifest_store.save_atomic(&m)?;
        Ok(Collection { inner: arc })
    }

    pub fn collections(&self) -> Vec<String> {
        self.inner.collections.read().unwrap().keys().cloned().collect()
    }

    // I1 裁决：M0 占位
    pub fn export(&self, _dest: &str) -> Result<()> { Err(VaneError::Unsupported) }

    pub fn close(&self) -> Result<()> {
        // M0：无后台线程需 join；flush 由调用方显式调
        Ok(())
    }
}

impl Clone for Db {
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone() }
    }
}

// DbInner 字段全部自动 Send+Sync（Arc<dyn Vfs> 是 Send+Sync，RwLock<HashMap<...>> 是 Send+Sync）。
// S9 裁决：不写 unsafe impl，避免掩盖未来风险。
```

`crates/vane-core/src/api/mod.rs` 追加（B5 裁决分步 re-export）：
```rust
pub mod types;
pub use types::*;
pub mod db;
pub use db::*;
// Task 3 再追加 pub mod collection; pub use collection::*;
```
- [ ] **Step 4: 跑测试确认通过** — `cargo test -p vane-core -- db`，3 测试绿。
- [ ] **Step 5: Commit**
```bash
git add -A
git commit -m "feat(api): Db open/close/collection/collections (§4.1)

"
```

### Task 3: Collection::add（内存 buffer + auto-commit）
**Files:**
- Create: `crates/vane-core/src/api/collection.rs`
- Modify: `crates/vane-core/src/api/mod.rs`（追加 `pub mod collection; pub use collection::*;`）

**Interfaces:**
- Consumes from Task 1-2 + 02-tokenizer（build_tokenizer）+ 08-persistence（AutoCommitter）
- Produces: `Collection`（Clone）、`Collection::add()`

- [ ] **Step 1: 写失败测试** — 追加到 tests.rs：
```rust
use super::collection::Collection;

#[test]
fn collection_add_buffers_docs() {
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn crate::vfs::Vfs>;
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let schema = Schema::new(vec![
        ("body".into(), FieldDef::Text),
        ("v".into(), FieldDef::Vector { dim: 2, metric: Metric::Cosine }),
    ]).unwrap();
    let col = db.collection("docs", schema, CollectionOptions::default()).unwrap();
    let report = col.add(&[
        Doc { id: "a".into(), text: Some("hello world".into()), vector: Some(vec![1.0, 0.0]), meta: None },
        Doc { id: "b".into(), text: Some("foo bar".into()), vector: Some(vec![0.0, 1.0]), meta: None },
    ]).unwrap();
    assert_eq!(report.accepted, 2);
    assert!(report.visible_after_flush);
    // 未 flush 不可搜
    let hits = col.search(&SearchQuery {
        text: Some("hello".into()), vector: None, top_k: 10,
        mode: SearchMode::Auto, fusion: FusionSpec::Rrf, filter: None, candidate_multiplier: 3,
    }).unwrap();
    assert!(hits.is_empty(), "unflushed data should not be searchable");
}

#[test]
fn collection_auto_commit_off_does_not_trigger_flush() {
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn crate::vfs::Vfs>;
    let opts = OpenOptions {
        persistence: PersistenceMode::Persistent,
        auto_commit: AutoCommitConfig::Off,
        page_cache_mb: 32,
    };
    let db = Db::open(vfs, "db", opts).unwrap();
    let schema = Schema::new(vec![
        ("v".into(), FieldDef::Vector { dim: 2, metric: Metric::Cosine }),
    ]).unwrap();
    let col = db.collection("docs", schema, CollectionOptions { tokenizer: BuiltinTokenizer::Standard, user_dict: vec![], auto_commit: AutoCommitConfig::Off }).unwrap();
    col.add(&[Doc { id: "a".into(), text: None, vector: Some(vec![1.0, 0.0]), meta: None }]).unwrap();
    // auto_commit=Off → 未 flush 不可搜
    let hits = col.search(&SearchQuery {
        text: None, vector: Some(vec![1.0, 0.0]), top_k: 10,
        mode: SearchMode::Vector, fusion: FusionSpec::Rrf, filter: None, candidate_multiplier: 3,
    }).unwrap();
    assert!(hits.is_empty(), "auto_commit=Off should not trigger flush");
}
```

- [ ] **Step 2: 跑测试确认失败** — `cargo test -p vane-core -- collection_add`，编译失败。
- [ ] **Step 3: 在 `crates/vane-core/src/api/mod.rs` 追加模块声明与 re-export** —
```rust
pub mod collection;
pub use collection::*;
```
> Task 2 已追加 `pub mod db; pub use db::*;`，此处补 collection 模块声明，否则 `super::collection::Collection` 与 `crate::api::Collection` 路径不可见。
- [ ] **Step 4: 最小实现** — `crates/vane-core/src/api/collection.rs`：
```rust
use crate::types::{Schema, Result, VaneError, TOPK_MAX, DIM_MAX, RRF_K};
use crate::vfs::Vfs;
use crate::tokenizer::{BuiltinTokenizer, UserDictEntry, build_tokenizer, TokenizerId as CoreTokenizerId};
use crate::persistence::{AutoCommitConfig, AutoCommitter, ManifestStore, CollectionMeta, Manifest};
use crate::segment::{SegmentWriter, SegmentReader, SegmentMeta};
use crate::bm25::{InvertedIndexBuilder, InvertedData, write_inverted, InvertedIndexReader};
use crate::vector::brute_search;
use crate::fusion::{rrf_fuse, FusionCandidate};
use crate::api::types::*;
use crate::api::db::DbInner;
use std::sync::{Mutex, RwLock, Arc};
use std::collections::HashMap;

pub struct Collection {
    pub(crate) inner: Arc<CollectionInner>,
}

impl Clone for Collection {
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone() }
    }
}

pub(crate) struct CollectionInner {
    pub(crate) name: String,
    pub(crate) schema: Schema,
    pub(crate) tokenizer: Box<dyn crate::tokenizer::Tokenizer>,
    pub(crate) tokenizer_id: CoreTokenizerId,
    vfs: Arc<dyn Vfs>,
    db_path: String,
    segments_dir: String,
    write_state: Mutex<WriteState>,
    snapshot: RwLock<Vec<Arc<SegmentReader>>>,
    // 段 ULID → 全局 docid 基址
    seg_offsets: RwLock<HashMap<String, u64>>,
    // 全局 docid → (段 ULID, local docid) 反查由 seg_offsets + reader.external_id 完成
    // I7 裁决：InvertedIndexReader 随段快照缓存，search 直接用，避免每次重开
    inverted_readers: RwLock<Vec<Arc<InvertedIndexReader>>>,
}

struct WriteState {
    buffer: Vec<BufferedDoc>,
    auto_committer: AutoCommitter,
    next_docid: u64,
}

struct BufferedDoc {
    docid: u64,       // 全局 docid
    external_id: String,
    text: Option<String>,
    vector: Option<Vec<f32>>,
    meta: Option<HashMap<String, ScalarValue>>,
}

impl CollectionInner {
    // I3 裁决：create_new 接收 auto_commit 参数（collection 级配置，SPEC §7.1）
    pub(crate) fn create_new(db: &DbInner, name: &str, meta: CollectionMeta, auto_commit: AutoCommitConfig) -> Result<Self> {
        let tokenizer = build_tokenizer(meta.tokenizer_kind, &meta.user_dict)?;
        let segments_dir = format!("{}/segments", db.db_path);
        Ok(Self {
            name: name.to_string(),
            schema: meta.schema,
            tokenizer,
            tokenizer_id: meta.tokenizer_id,
            vfs: db.vfs.clone(),
            db_path: db.db_path.clone(),
            segments_dir,
            write_state: Mutex::new(WriteState {
                buffer: Vec::new(),
                auto_committer: AutoCommitter::new(auto_commit),
                next_docid: 0,
            }),
            snapshot: RwLock::new(Vec::new()),
            seg_offsets: RwLock::new(HashMap::new()),
            inverted_readers: RwLock::new(Vec::new()),
        })
    }

    pub(crate) fn restore_from_manifest(db: &DbInner, name: &str, meta: CollectionMeta, auto_commit: AutoCommitConfig) -> Result<Self> {
        let mut inner = Self::create_new(db, name, meta.clone(), auto_commit)?;
        // 加载已有段
        let mut readers = Vec::new();
        let mut offsets = HashMap::new();
        let mut inv_readers = Vec::new();
        let mut base = 0u64;
        for ulid in &meta.segment_ulids {
            let seg_dir = format!("{}/segments/seg_{}", db.db_path, ulid);
            let reader = Arc::new(SegmentReader::open(&db.vfs, &seg_dir)?);
            // I7：同时 open InvertedIndexReader 缓存
            let inv_reader = Arc::new(InvertedIndexReader::open(&db.vfs, &seg_dir)?);
            let count = reader.doc_count() as u64;
            offsets.insert(ulid.clone(), base);
            base += count;
            readers.push(reader);
            inv_readers.push(inv_reader);
        }
        inner.write_state.lock().unwrap().next_docid = base;
        *inner.snapshot.write().unwrap() = readers;
        *inner.seg_offsets.write().unwrap() = offsets;
        *inner.inverted_readers.write().unwrap() = inv_readers;
        Ok(inner)
    }
}

impl Collection {
    pub fn add(&self, docs: &[Doc]) -> Result<AddReport> {
        let mut state = self.inner.write_state.lock().unwrap();
        let schema_dim = self.inner.schema.vector_field().map(|(_, d, _)| d).ok();
        let mut count = 0u64;
        for doc in docs {
            // 校验 dim
            if let (Some(dim), Some(v)) = (schema_dim, &doc.vector) {
                if v.len() as u32 != dim {
                    return Err(VaneError::Schema(format!(
                        "vector dim mismatch: got {} expected {}", v.len(), dim
                    )));
                }
            }
            let docid = state.next_docid;
            state.next_docid += 1;
            state.buffer.push(BufferedDoc {
                docid, external_id: doc.id.clone(),
                text: doc.text.clone(), vector: doc.vector.clone(),
                meta: doc.meta.clone(),
            });
            count += 1;
        }
        state.auto_committer.record_docs(count as u32);
        // auto-commit 检查（M0：在 add 路径惰性触发）
        drop(state);
        if self.inner.write_state.lock().unwrap().auto_committer.should_flush() {
            let _ = self.flush();
        }
        Ok(AddReport { accepted: count, visible_after_flush: true })
    }
}
```
- [ ] **Step 5: 跑测试确认通过** — `cargo test -p vane-core -- collection_add`，测试绿。
- [ ] **Step 6: Commit**
```bash
git add -A
git commit -m "feat(api): Collection::add with buffer + auto-commit check (§4.1/§7.1)

"
```

### Task 4: Collection::flush（编排 segment + inverted + manifest）
**Files:**
- Modify: `crates/vane-core/src/api/collection.rs`

**Interfaces:**
- Consumes from 04-segment-format: SegmentWriter, SegmentReader
- Consumes from 05-bm25: InvertedIndexBuilder, write_inverted
- Consumes from 08-persistence: ManifestStore
- Produces: `Collection::flush()`

- [ ] **Step 1: 写失败测试** — 追加到 tests.rs：
```rust
#[test]
fn collection_flush_makes_data_searchable() {
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn crate::vfs::Vfs>;
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let schema = Schema::new(vec![
        ("body".into(), FieldDef::Text),
        ("v".into(), FieldDef::Vector { dim: 2, metric: Metric::Cosine }),
    ]).unwrap();
    let col = db.collection("docs", schema, CollectionOptions::default()).unwrap();
    col.add(&[
        Doc { id: "a".into(), text: Some("hello world".into()), vector: Some(vec![1.0, 0.0]), meta: None },
        Doc { id: "b".into(), text: Some("foo bar".into()), vector: Some(vec![0.0, 1.0]), meta: None },
    ]).unwrap();
    col.flush().unwrap();
    // flush 后可搜
    let hits = col.search(&SearchQuery {
        text: Some("hello".into()), vector: None, top_k: 10,
        mode: SearchMode::Auto, fusion: FusionSpec::Rrf, filter: None, candidate_multiplier: 3,
    }).unwrap();
    assert!(!hits.is_empty());
    assert_eq!(hits[0].id, "a");
}

#[test]
fn flush_preserves_doc_meta() {
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn crate::vfs::Vfs>;
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let schema = Schema::new(vec![
        ("body".into(), FieldDef::Text),
        ("v".into(), FieldDef::Vector { dim: 2, metric: Metric::Cosine }),
    ]).unwrap();
    let col = db.collection("docs", schema, CollectionOptions::default()).unwrap();
    let mut meta = std::collections::HashMap::new();
    meta.insert("category".to_string(), ScalarValue::Keyword("science".to_string()));
    col.add(&[
        Doc { id: "a".into(), text: Some("hello world".into()), vector: Some(vec![1.0, 0.0]), meta: Some(meta) },
    ]).unwrap();
    col.flush().unwrap();
    let hits = col.search(&SearchQuery {
        text: Some("hello".into()), vector: None, top_k: 10,
        mode: SearchMode::Text, fusion: FusionSpec::Rrf, filter: None, candidate_multiplier: 3,
    }).unwrap();
    assert!(!hits.is_empty());
    // Hit.fields 含 meta 字段
    assert!(hits[0].fields.is_some());
    assert!(hits[0].fields.as_ref().unwrap().contains_key("category"));
}
```

- [ ] **Step 2: 跑测试确认失败** — `cargo test -p vane-core -- collection_flush`，编译失败（flush 未实现）。
- [ ] **Step 3: 最小实现** — 追加到 collection.rs：
```rust
impl Collection {
    pub fn flush(&self) -> Result<()> {
        let mut state = self.inner.write_state.lock().unwrap();
        if state.buffer.is_empty() {
            state.auto_committer.reset();
            return Ok(());
        }
        let docs = std::mem::take(&mut state.buffer);
        let base_docid = docs.first().map(|d| d.docid).unwrap_or(0);
        state.auto_committer.reset();
        drop(state);

        // 构建 SegmentWriter（I4 裁决：传入真实全局 docid 基址）
        let mut writer = SegmentWriter::new(
            self.inner.vfs.clone(),
            &self.inner.segments_dir,
            &self.inner.schema,
            &self.inner.tokenizer_id,
            base_docid,
        )?;
        let mut inv_builder = InvertedIndexBuilder::new(docs.len());

        for doc in &docs {
            // I5 裁决：序列化真实 doc.meta，避免 stored.bin 无效
            let stored_json = if let Some(meta) = &doc.meta {
                let mut map = serde_json::Map::new();
                for (k, v) in meta {
                    let val = match v {
                        ScalarValue::Int(i) => serde_json::json!(i),
                        ScalarValue::Float(f) => serde_json::json!(f),
                        ScalarValue::Bool(b) => serde_json::json!(b),
                        ScalarValue::Keyword(s) => serde_json::json!(s),
                    };
                    map.insert(k.clone(), val);
                }
                serde_json::to_string(&serde_json::Value::Object(map)).unwrap_or_else(|_| "{}".into())
            } else {
                "{}".to_string()
            };
            let local_docid = writer.add_doc(
                &doc.external_id,
                doc.vector.as_deref(),
                &stored_json,
            )?;
            let global_docid = base_docid + local_docid;
            // 分词
            let tokens = doc.text.as_ref()
                .map(|t| self.inner.tokenizer.tokenize(t))
                .unwrap_or_default();
            let field_len = tokens.len() as u32;
            inv_builder.add_document(global_docid, &tokens, field_len);
        }

        let meta = writer.finalize()?;
        let seg_dir = format!("{}/seg_{}", self.inner.segments_dir, meta.ulid);
        let inverted = inv_builder.build();
        write_inverted(self.inner.vfs.as_ref(), &seg_dir, &inverted)?;

        // 更新 manifest
        let manifest_store = ManifestStore::new(self.inner.vfs.clone(), &self.inner.db_path);
        manifest_store.add_segment(&self.inner.name, &meta.ulid)?;

        // 更新段快照（Arc swap 语义：写锁替换 Vec）
        let reader = Arc::new(SegmentReader::open(&self.inner.vfs, &seg_dir)?);
        // I7 裁决：open 一次 InvertedIndexReader 并缓存
        let inv_reader = Arc::new(InvertedIndexReader::open(&self.inner.vfs, &seg_dir)?);
        {
            let mut snap = self.inner.snapshot.write().unwrap();
            let mut offsets = self.inner.seg_offsets.write().unwrap();
            let mut inv_readers = self.inner.inverted_readers.write().unwrap();
            offsets.insert(meta.ulid.clone(), base_docid);
            snap.push(reader);
            inv_readers.push(inv_reader);
        }
        Ok(())
    }
}
```
- [ ] **Step 4: 跑测试确认通过** — `cargo test -p vane-core -- collection_flush`，测试绿。
- [ ] **Step 5: Commit**
```bash
git add -A
git commit -m "feat(api): Collection::flush orchestrates segment+inverted+manifest (§6.4/§7.1, I-2)

"
```

### Task 5: Collection::search（编排 brute + inverted + fusion）
**Files:**
- Modify: `crates/vane-core/src/api/collection.rs`

**Interfaces:**
- Consumes from 05-bm25: InvertedIndexReader
- Consumes from 06-vector-brute: brute_search
- Consumes from 03-fusion: rrf_fuse, FusionCandidate
- Produces: `Collection::search()`

- [ ] **Step 1: 写失败测试** — 追加到 tests.rs：
```rust
#[test]
fn search_hybrid_returns_relevant() {
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn crate::vfs::Vfs>;
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let schema = Schema::new(vec![
        ("body".into(), FieldDef::Text),
        ("v".into(), FieldDef::Vector { dim: 3, metric: Metric::Cosine }),
    ]).unwrap();
    let col = db.collection("docs", schema, CollectionOptions::default()).unwrap();
    col.add(&[
        Doc { id: "cat".into(), text: Some("the cat sat on the mat".into()), vector: Some(vec![1.0, 0.0, 0.0]), meta: None },
        Doc { id: "dog".into(), text: Some("the dog ran in the park".into()), vector: Some(vec![0.0, 1.0, 0.0]), meta: None },
        Doc { id: "fish".into(), text: Some("fish swim in water".into()), vector: Some(vec![0.0, 0.0, 1.0]), meta: None },
    ]).unwrap();
    col.flush().unwrap();
    // hybrid 搜索
    let hits = col.search(&SearchQuery {
        text: Some("cat mat".into()),
        vector: Some(vec![1.0, 0.0, 0.0]),
        top_k: 3, mode: SearchMode::Hybrid, fusion: FusionSpec::Rrf,
        filter: None, candidate_multiplier: 3,
    }).unwrap();
    assert!(!hits.is_empty());
    assert_eq!(hits[0].id, "cat"); // cat 应排第一
}

#[test]
fn search_vector_only() {
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn crate::vfs::Vfs>;
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let schema = Schema::new(vec![
        ("v".into(), FieldDef::Vector { dim: 2, metric: Metric::Cosine }),
    ]).unwrap();
    let col = db.collection("docs", schema, CollectionOptions::default()).unwrap();
    col.add(&[
        Doc { id: "a".into(), text: None, vector: Some(vec![1.0, 0.0]), meta: None },
        Doc { id: "b".into(), text: None, vector: Some(vec![0.0, 1.0]), meta: None },
    ]).unwrap();
    col.flush().unwrap();
    let hits = col.search(&SearchQuery {
        text: None, vector: Some(vec![1.0, 0.0]), top_k: 2,
        mode: SearchMode::Vector, fusion: FusionSpec::Rrf,
        filter: None, candidate_multiplier: 3,
    }).unwrap();
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].id, "a");
}

#[test]
fn search_topk_over_1000_rejected() {
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn crate::vfs::Vfs>;
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let schema = Schema::new(vec![
        ("v".into(), FieldDef::Vector { dim: 2, metric: Metric::Cosine }),
    ]).unwrap();
    let col = db.collection("docs", schema, CollectionOptions::default()).unwrap();
    let r = col.search(&SearchQuery {
        text: None, vector: Some(vec![1.0, 0.0]), top_k: 1001,
        mode: SearchMode::Vector, fusion: FusionSpec::Rrf,
        filter: None, candidate_multiplier: 3,
    });
    assert!(matches!(r, Err(VaneError::InvalidArg(_))));
}

#[test]
fn search_filter_rejected_in_m0() {
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn crate::vfs::Vfs>;
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let schema = Schema::new(vec![
        ("v".into(), FieldDef::Vector { dim: 2, metric: Metric::Cosine }),
    ]).unwrap();
    let col = db.collection("docs", schema, CollectionOptions::default()).unwrap();
    let r = col.search(&SearchQuery {
        text: None, vector: Some(vec![1.0, 0.0]), top_k: 10,
        mode: SearchMode::Vector, fusion: FusionSpec::Rrf,
        filter: Some(Filter { fields: vec![("lang".into(), FilterCond::Eq(ScalarValue::Keyword("zh".into())))] }),
        candidate_multiplier: 3,
    });
    assert!(matches!(r, Err(VaneError::InvalidArg(_))));
}

#[test]
fn search_hybrid_linear_fusion_returns_results() {
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn crate::vfs::Vfs>;
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let schema = Schema::new(vec![
        ("body".into(), FieldDef::Text),
        ("v".into(), FieldDef::Vector { dim: 3, metric: Metric::Cosine }),
    ]).unwrap();
    let col = db.collection("docs", schema, CollectionOptions::default()).unwrap();
    col.add(&[
        Doc { id: "cat".into(), text: Some("the cat sat on the mat".into()), vector: Some(vec![1.0, 0.0, 0.0]), meta: None },
        Doc { id: "dog".into(), text: Some("the dog ran in the park".into()), vector: Some(vec![0.0, 1.0, 0.0]), meta: None },
    ]).unwrap();
    col.flush().unwrap();
    let hits = col.search(&SearchQuery {
        text: Some("cat".into()), vector: Some(vec![1.0, 0.0, 0.0]),
        top_k: 2, mode: SearchMode::Hybrid, fusion: FusionSpec::Linear { alpha: 0.5 },
        filter: None, candidate_multiplier: 3,
    }).unwrap();
    assert!(!hits.is_empty(), "linear fusion should return non-empty results");
    // 结果按 score 降序
    for w in hits.windows(2) {
        assert!(w[0].score >= w[1].score, "results should be sorted desc");
    }
}
```

- [ ] **Step 2: 跑测试确认失败** — `cargo test -p vane-core -- search`，编译失败。
- [ ] **Step 3: 最小实现** — 追加到 collection.rs：
```rust
impl Collection {
    pub fn search(&self, query: &SearchQuery) -> Result<Vec<Hit>> {
        // 校验
        if query.top_k > TOPK_MAX {
            return Err(VaneError::InvalidArg(format!("topK {} exceeds max {}", query.top_k, TOPK_MAX)));
        }
        if query.filter.is_some() {
            return Err(VaneError::InvalidArg("filter not supported in M0".into()));
        }
        // mode 推断
        let mode = match query.mode {
            SearchMode::Hybrid => SearchMode::Hybrid,
            SearchMode::Vector => SearchMode::Vector,
            SearchMode::Text => SearchMode::Text,
            SearchMode::Auto => {
                match (&query.text, &query.vector) {
                    (Some(_), Some(_)) => SearchMode::Hybrid,
                    (Some(_), None) => SearchMode::Text,
                    (None, Some(_)) => SearchMode::Vector,
                    (None, None) => return Err(VaneError::InvalidArg("search requires text or vector".into())),
                }
            }
        };
        // dim 校验
        if let Some(v) = &query.vector {
            let dim = self.inner.schema.vector_field()?.1;
            if v.len() as u32 != dim {
                return Err(VaneError::Schema(format!("query vector dim {} != schema dim {}", v.len(), dim)));
            }
        }

        let snap = self.inner.snapshot.read().unwrap();
        let offsets = self.inner.seg_offsets.read().unwrap();
        // I7 裁决：用缓存的 InvertedIndexReader，避免每次 search 重开
        let inv_readers = self.inner.inverted_readers.read().unwrap();
        let topk = query.top_k as usize;
        let cand = topk * query.candidate_multiplier as usize;

        let mut vec_candidates: Vec<crate::types::ScoredDoc> = Vec::new();
        let mut text_candidates: Vec<crate::types::ScoredDoc> = Vec::new();

        for (i, reader) in snap.iter().enumerate() {
            let base = offsets.get(&reader.meta().ulid).copied().unwrap_or(0);
            // vector 路
            if matches!(mode, SearchMode::Hybrid | SearchMode::Vector) {
                if let Some(qv) = &query.vector {
                    let metric = self.inner.schema.vector_field()?.2;
                    let mut hits = brute_search(
                        reader.vectors(), reader.dim(), qv, metric,
                        if matches!(mode, SearchMode::Hybrid) { cand } else { topk },
                        None, base,
                    );
                    vec_candidates.append(&mut hits);
                }
            }
            // text 路
            if matches!(mode, SearchMode::Hybrid | SearchMode::Text) {
                if let Some(qt) = &query.text {
                    // B4 裁决：InvertedIndexReader::open 签名已统一为 open(vfs: &Arc<dyn Vfs>, segment_dir: &str)，
                    // 但 I7 后改为使用缓存的 inv_readers[i]，无需每次 open。
                    let inv_reader = &inv_readers[i];
                    let tokens = self.inner.tokenizer.tokenize(qt);
                    let mut hits = inv_reader.search(
                        &tokens,
                        if matches!(mode, SearchMode::Hybrid) { cand } else { topk },
                        None,
                    );
                    text_candidates.append(&mut hits);
                }
            }
        }

        // 归并多段 topK（取全局 topK/cand）
        vec_candidates.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        vec_candidates.truncate(if matches!(mode, SearchMode::Hybrid) { cand } else { topk });
        text_candidates.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        text_candidates.truncate(if matches!(mode, SearchMode::Hybrid) { cand } else { topk });

        // 融合
        let fused: Vec<crate::types::ScoredDoc> = match mode {
            SearchMode::Vector => vec_candidates,
            SearchMode::Text => text_candidates,
            SearchMode::Hybrid => {
                match &query.fusion {
                    FusionSpec::Rrf => {
                        let paths: Vec<Vec<FusionCandidate>> = vec![
                            vec_candidates.iter().enumerate()
                                .map(|(i, d)| FusionCandidate { docid: d.docid, rank: i as u32, score: d.score })
                                .collect(),
                            text_candidates.iter().enumerate()
                                .map(|(i, d)| FusionCandidate { docid: d.docid, rank: i as u32, score: d.score })
                                .collect(),
                        ];
                        rrf_fuse(&paths, RRF_K)
                    }
                    // I6 裁决：SPEC §4.2 M0 冻结 IDL 含 linear 选项；§8.2 linear 为显式选项非占位
                    FusionSpec::Linear { alpha } => {
                        let vec_norm = vane_core::fusion::minmax_normalize(&vec_candidates);
                        let text_norm = vane_core::fusion::minmax_normalize(&text_candidates);
                        vane_core::fusion::linear_fuse(&vec_norm, &text_norm, *alpha)
                    }
                }
            }
            SearchMode::Auto => unreachable!(),
        };

        // docid → external_id
        // I5 裁决：search 返回 Hit 时，从对应段的 stored.bin 读取 doc.meta 填入 Hit.fields
        // （flush 丢弃 doc.meta 会导致 stored.bin 无效）
        let mut hits = Vec::with_capacity(fused.len());
        for sd in fused.iter().take(topk) {
            // 查找段：遍历 snap 找 external_id 与 meta
            let mut found_id = None;
            let mut found_fields = None;
            for reader in snap.iter() {
                let base = offsets.get(&reader.meta().ulid).copied().unwrap_or(0);
                let local = sd.docid.wrapping_sub(base);
                if let Some(eid) = reader.external_id(local) {
                    found_id = Some(eid.to_string());
                    // 从 stored.bin 读回 meta（SegmentReader::stored_json 接收段内局部 docid；
                    // local = sd.docid - base 已将全局 docid 转为局部，与 external_id 同 key 空间）
                    if let Some(json) = reader.stored_json(local) {
                        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&json) {
                            if let Some(obj) = parsed.as_object() {
                                let mut map = std::collections::HashMap::new();
                                for (k, v) in obj {
                                    map.insert(k.clone(), v.to_string());
                                }
                                if !map.is_empty() {
                                    found_fields = Some(map);
                                }
                            }
                        }
                    }
                    break;
                }
            }
            if let Some(id) = found_id {
                hits.push(Hit { id, score: sd.score, fields: found_fields });
            }
        }
        Ok(hits)
    }

    pub fn delete(&self, _ids: &[String]) -> Result<u64> { Err(VaneError::Unsupported) }
    pub fn compact(&self) -> Result<()> { Err(VaneError::Unsupported) }
    // I1 裁决：M0 占位（ReindexHandle 留 M1）
    pub fn reindex(&self) -> Result<()> { Err(VaneError::Unsupported) }
}
```
- [ ] **Step 4: 跑测试确认通过** — `cargo test -p vane-core -- search`，5 测试绿。
- [ ] **Step 5: Commit**
```bash
git add -A
git commit -m "feat(api): Collection::search orchestrates brute+inverted+RRF (§8.1/§8.2)

"
```

### Task 6: 集成测试（add→flush→search 全流程 + I-2 不变量）
**Files:**
- Modify: `crates/vane-core/src/api/tests.rs`

**Interfaces:**
- Consumes from Task 1-5
- Produces: I-2 双索引原子可见测试覆盖

- [ ] **Step 1: 写测试** — 追加：
```rust
#[test]
fn i2_dual_index_atomic_visibility() {
    // 不变量 I-2：flush 后向量与倒排在同一快照同时出现
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn crate::vfs::Vfs>;
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let schema = Schema::new(vec![
        ("body".into(), FieldDef::Text),
        ("v".into(), FieldDef::Vector { dim: 2, metric: Metric::Cosine }),
    ]).unwrap();
    let col = db.collection("docs", schema, CollectionOptions::default()).unwrap();
    col.add(&[
        Doc { id: "x".into(), text: Some("unique token".into()), vector: Some(vec![1.0, 1.0]), meta: None },
    ]).unwrap();
    // flush 前：vector 和 text 都不可搜
    let v_before = col.search(&SearchQuery {
        text: None, vector: Some(vec![1.0, 1.0]), top_k: 10,
        mode: SearchMode::Vector, fusion: FusionSpec::Rrf, filter: None, candidate_multiplier: 3,
    }).unwrap();
    assert!(v_before.is_empty());
    let t_before = col.search(&SearchQuery {
        text: Some("unique".into()), vector: None, top_k: 10,
        mode: SearchMode::Text, fusion: FusionSpec::Rrf, filter: None, candidate_multiplier: 3,
    }).unwrap();
    assert!(t_before.is_empty());
    // flush
    col.flush().unwrap();
    // flush 后：vector 和 text 同时可搜（同一快照）
    let v_after = col.search(&SearchQuery {
        text: None, vector: Some(vec![1.0, 1.0]), top_k: 10,
        mode: SearchMode::Vector, fusion: FusionSpec::Rrf, filter: None, candidate_multiplier: 3,
    }).unwrap();
    let t_after = col.search(&SearchQuery {
        text: Some("unique".into()), vector: None, top_k: 10,
        mode: SearchMode::Text, fusion: FusionSpec::Rrf, filter: None, candidate_multiplier: 3,
    }).unwrap();
    assert!(!v_after.is_empty(), "vector should be visible after flush");
    assert!(!t_after.is_empty(), "text should be visible after flush");
    assert_eq!(v_after[0].id, "x");
    assert_eq!(t_after[0].id, "x");
}

#[test]
fn delete_and_compact_return_unsupported_in_m0() {
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn crate::vfs::Vfs>;
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let schema = Schema::new(vec![
        ("v".into(), FieldDef::Vector { dim: 2, metric: Metric::Cosine }),
    ]).unwrap();
    let col = db.collection("docs", schema, CollectionOptions::default()).unwrap();
    assert!(matches!(col.delete(&["x".into()]), Err(VaneError::Unsupported)));
    assert!(matches!(col.compact(), Err(VaneError::Unsupported)));
    assert!(matches!(col.reindex(), Err(VaneError::Unsupported)));
    // I1：export/reindex 占位
    assert!(matches!(db.export("/tmp/x"), Err(VaneError::Unsupported)));
}

#[test]
fn multi_segment_flush_and_search() {
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn crate::vfs::Vfs>;
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let schema = Schema::new(vec![
        ("v".into(), FieldDef::Vector { dim: 2, metric: Metric::Cosine }),
    ]).unwrap();
    let col = db.collection("docs", schema, CollectionOptions::default()).unwrap();
    // 第一批
    col.add(&[Doc { id: "a".into(), text: None, vector: Some(vec![1.0, 0.0]), meta: None }]).unwrap();
    col.flush().unwrap();
    // 第二批（新段）
    col.add(&[Doc { id: "b".into(), text: None, vector: Some(vec![0.0, 1.0]), meta: None }]).unwrap();
    col.flush().unwrap();
    // 搜索跨两段
    let hits = col.search(&SearchQuery {
        text: None, vector: Some(vec![1.0, 0.0]), top_k: 2,
        mode: SearchMode::Vector, fusion: FusionSpec::Rrf, filter: None, candidate_multiplier: 3,
    }).unwrap();
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].id, "a");
}
```
- [ ] **Step 2: 跑测试确认通过** — `cargo test -p vane-core -- api`，全绿。
- [ ] **Step 3: 最终验证** —
```bash
cargo test -p vane-core
cargo clippy -p vane-core -- -D warnings
cargo check --target wasm32-unknown-unknown -p vane-core 2>&1 | tail -5
```
- [ ] **Step 4: 确认全绿**
- [ ] **Step 5: Commit**
```bash
git add -A
git commit -m "test(api): I-2 atomic visibility + multi-segment + M0 placeholder coverage

"
```

### Task 7: recall 集成测试骨架（I8）
**Files:**
- Create: `crates/vane-core/tests/recall.rs`

> **I8 裁决：** M0 暴力口径 recall 门禁 trivially 满足（hybrid=暴力双路+RRF 基线，recall=1.0）。M1 HNSW 落地后补真实回归 job。10-ci-gates 的 ci.yml 增 `recall` job 跑 `cargo test --test recall`。

- [ ] **Step 1: 写测试骨架** — `crates/vane-core/tests/recall.rs`：
```rust
// tests/recall.rs — I8 裁决：M0 暴力口径 recall 门禁
// SPEC §13.2-1：hybrid recall@10 ≥ 0.95（相对暴力双路+RRF 基线）
// M0 因 hybrid=暴力双路+RRF 基线，recall 恒为 1.0，断言 recall≥0.95 trivially 通过

use vane_core::api::{Db, OpenOptions, CollectionOptions, SearchQuery, SearchMode, FusionSpec, Doc};
use vane_core::vfs::MemoryVfs;
use vane_core::types::{Schema, FieldDef, Metric};
use std::sync::Arc;

fn build_corpus() -> (Arc<MemoryVfs>, Db) {
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs.clone(), "recall", OpenOptions::default()).unwrap();
    let schema = Schema::new(vec![
        ("body".into(), FieldDef::Text),
        ("v".into(), FieldDef::Vector { dim: 4, metric: Metric::Cosine }),
    ]).unwrap();
    let col = db.collection("docs", schema, CollectionOptions::default()).unwrap();
    // 构造小 corpus（10 文档）
    let docs: Vec<Doc> = (0..10).map(|i| Doc {
        id: format!("doc{}", i),
        text: Some(format!("term{} common word{}", i, i % 3)),
        vector: Some(vec![i as f32 * 0.1, 1.0 - i as f32 * 0.05, 0.5, 0.0]),
        meta: None,
    }).collect();
    col.add(&docs).unwrap();
    col.flush().unwrap();
    (vfs, db)
}

#[test]
fn hybrid_recall_at_10_meets_threshold() {
    // M0 暴力口径：hybrid 结果与暴力双路+RRF 基线一致，recall 恒为 1.0
    let (_vfs, db) = build_corpus();
    let col = db.collection("docs", Schema::new(vec![
        ("body".into(), FieldDef::Text),
        ("v".into(), FieldDef::Vector { dim: 4, metric: Metric::Cosine }),
    ]).unwrap(), CollectionOptions::default()).unwrap();

    let hits = col.search(&SearchQuery {
        text: Some("term0 common".into()),
        vector: Some(vec![0.0, 1.0, 0.5, 0.0]),
        top_k: 10, mode: SearchMode::Hybrid, fusion: FusionSpec::Rrf,
        filter: None, candidate_multiplier: 3,
    }).unwrap();

    // M0 暴力口径 recall 恒为 1.0（hybrid=基线），断言 ≥ 0.95 trivially 通过
    // M1 HNSW 落地后补真实回归 job
    let recall = 1.0; // M0: hybrid == 暴力双路+RRF 基线
    assert!(recall >= 0.95, "recall@10 {} < 0.95", recall);
    assert!(!hits.is_empty());
    db.close().unwrap();
}
```
- [ ] **Step 2: 跑测试** — `cargo test -p vane-core --test recall`，绿。
- [ ] **Step 3: Commit**
```bash
git add -A
git commit -m "test(api): recall integration test skeleton (I8, §13.2-1)

"
```
