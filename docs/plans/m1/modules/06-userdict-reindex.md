# 06-userdict-reindex：自定义词表 + setUserDict/reindex 状态机（§7.4）

> SPEC 引用：§7.4（词表变更与 reindex 状态机）、§5.3（自定义词表）、§5.4（TokenizerId）、§4.1（ReindexHandle）、§4.2（CollectionOptions.userDict）。
> 前置依赖：M0 `api`/`persistence`/`tokenizer`；05-jieba-lite（JiebaTokenizer 新身份）；02-tombstone-merge（reindex 复用 MergeTask 管线）；**00-text-persistence**（`SegmentReader::text` 读原文重新分词，B-1 前置）。
> M1 README 契约：api 扩展（DictState/ReindexHandle/set_user_dict/reindex）。

## Goal

实装 §7.4 词表状态机：Stable → setUserDict → PendingReindex → reindex → Rebuilding → Stable。setUserDict 暂存不生效（新写入用旧分词身份）；reindex 复用 MergeTask 管线后台增量重建全量段；旧段只读服务；完成后 manifest 原子切换新词表生效。ReindexHandle 可轮询可阻塞。

## Architecture

- **状态机**（`DictState`）：`CollectionInner` 增 `dict_state: RwLock<DictState>` + `pending_dict: RwLock<Vec<UserDictEntry>>`（暂存词表）。
  - Stable：正常读写。
  - PendingReindex：setUserDict 后；新 add 仍用旧 tokenizer；search 响应可携带 needsReindex（绑定层查询 `dict_state()`）。
  - Rebuilding：reindex() 后；旧段只读服务；写路径 E_BUSY（见下「Rebuilding 期写路径 E_BUSY」）。
  - Stable：reindex 完成原子切换后。
- **reindex 编排**（B-1 修订）：
  1. 校验 state == PendingReindex（否则 InvalidArg 或 no-op）。
  2. state → Rebuilding；构建新 tokenizer（用 pending_dict + jieba 词典）+ 新 TokenizerId。
  3. 对每段：**从旧段 `SegmentReader::text` 读原文**（00 产出），用**新分词器**重新 tokenize → `InvertedIndexBuilder::add_document` 重建倒排（**非 posting remap**——reindex 分词器变了，必须重新分词）。vectors/hnsw 不变（向量与分词无关），但段需新 ULID（段不可变 I-1），vectors.bin/hnsw.bin 可直接复制或重写。原文写入新段（`set_text`，供未来再次 reindex）。标量 `set_scalar` 重写。
  4. 全部新段就绪 → manifest 原子切换（旧段 ULID 替换为新 ULID）→ 旧段目录删除 → state → Stable → tokenizer/tokenizer_id 更新为新身份。
  5. WAL 记录段增删（compact 路径 truncate）。
- **Rebuilding 期写路径 E_BUSY（Q-6）**：M1 选择 Rebuilding 期写路径返回 E_BUSY（保守，比 SPEC §7.4 更严格——SPEC 仅说「查询仍命中旧段」，未明确禁止写入）。SPEC 允许未来放宽为旧身份写入。
- **ReindexHandle**：持有 `Arc<ReindexInner>`，inner 含 `Mutex<ReindexState>`（progress + 完成标志 + 错误）+ MergeTask 句柄。`progress()` 读进度；`wait()` 阻塞直到完成（native 用 Condvar，WASM 轮询）。
- **禁止行为**（SPEC §7.4）：新旧分词身份混排检索（Rebuilding 期查询命中旧段，新段未切换前不参与查询）、自动全量重建（reindex 必须显式触发）、查询期多版本词表合并。

## 涉及文件

- **Modify**：
  - `crates/vane-core/src/api/types.rs`（DictState 枚举；ReindexHandle 结构）
  - `crates/vane-core/src/api/collection.rs`（CollectionInner 增 dict_state/pending_dict；set_user_dict/reindex/dict_state 实装；reindex 签名 `Result<()>` → `Result<ReindexHandle>`）
  - `crates/vane-core/src/api/db.rs`（restore_from_manifest 恢复 dict_state=Stable）
- **Create**：
  - `crates/vane-core/src/api/reindex.rs`（ReindexHandle + ReindexInner + 后台任务）
  - `crates/vane-core/src/api/reindex_tests.rs`
- **Test**：
  - `crates/vane-core/src/api/reindex_tests.rs`
  - `crates/vane-core/tests/userdict_reindex.rs`（集成）

## Interfaces

### Consumes from M0（已核查 git HEAD）

```rust
// crates/vane-core/src/api/collection.rs
pub(crate) struct CollectionInner {
    pub(crate) name: String,
    pub(crate) schema: Schema,
    pub(crate) tokenizer: Box<dyn crate::tokenizer::Tokenizer>,
    pub(crate) tokenizer_id: CoreTokenizerId,
    // ... vfs/db_path/segments_dir/write_state/snapshot/seg_offsets/inverted_readers
}
impl Collection {
    pub fn reindex(&self) -> Result<()>;  // M0 占位，M1 改签名
}
// crates/vane-core/src/tokenizer/mod.rs
pub fn build_tokenizer(kind: BuiltinTokenizer, user_dict: &[UserDictEntry]) -> Result<Box<dyn Tokenizer>>;
pub fn compute_tokenizer_id(kind: BuiltinTokenizer, user_dict: &[UserDictEntry]) -> TokenizerId;
// crates/vane-core/src/persistence/mod.rs
pub struct CollectionMeta { pub tokenizer_id: TokenizerId, pub user_dict: Vec<UserDictEntry>, ... }
```

### Consumes from 05-jieba-lite

```rust
pub fn build_jieba_tokenizer(dict: Arc<JiebaDict>, user_dict: &[UserDictEntry]) -> Result<Box<dyn Tokenizer>>;
// JiebaTokenizer::id() 含词典版本 + sha256_prefix
```

### Consumes from 02-tombstone-merge

```rust
pub struct MergeTask { ... }
impl MergeTask {
    pub fn new(
        sources: Vec<String>,
        target_docid_base: u64,
        tokenizer_id: TokenizerId,
        schema: Schema,
        tokenizer: std::sync::Arc<dyn vane_core::tokenizer::Tokenizer>,  // M-2：reindex 传新 tokenizer
    ) -> Self;
    pub fn step(&mut self, ctx: &MergeContext) -> Result<bool>;
    pub fn progress(&self) -> f32;
}
pub fn finalize_merge(task: MergeTask, ctx: &MergeContext) -> Result<SegmentMeta>;
```
**M-2**：MergeTask::new 签名含 `tokenizer` 参数。reindex 传**新 tokenizer**（倒排走 `InvertedIndexBuilder::add_document` 重新分词，非 posting remap）；compact 传当前 tokenizer（倒排走 posting remap）。02 计划已统一此签名。

### Consumes from 00-text-persistence

```rust
impl SegmentReader {
    pub fn text(&self, local_docid: u64) -> Option<&str>;  // reindex 读原文重新分词（B-1 前置）
}
```

### Produces（见 README § 06-userdict-reindex 契约）

## TDD 任务清单

### Task 1：DictState 枚举 + set_user_dict 暂存

**测试**（`crates/vane-core/src/api/reindex_tests.rs`）：
```rust
use super::*;
use crate::api::*;
use crate::tokenizer::UserDictEntry;
use crate::vfs::memory::MemoryVfs;
use std::sync::Arc;

#[test]
fn set_user_dict_enters_pending_reindex() {
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col = setup_col(&db);
    assert_eq!(col.dict_state(), DictState::Stable);
    col.set_user_dict(&[UserDictEntry::Word("新词".into())]).unwrap();
    assert_eq!(col.dict_state(), DictState::PendingReindex);
}

#[test]
fn pending_reindex_new_writes_use_old_tokenizer() {
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col = setup_col(&db);
    let old_id = col.tokenizer_id().clone();
    col.set_user_dict(&[UserDictEntry::Word("新词".into())]).unwrap();
    // PendingReindex 期 add 仍用旧身份
    col.add(&[Doc { id: "d1".into(), text: Some("新词".into()), vector: None, meta: None }]).unwrap();
    assert_eq!(col.tokenizer_id(), &old_id, "new writes must use old tokenizer (I-4)");
}
```
验证失败：`set_user_dict`/`dict_state` 不存在。
最小实现：`api/types.rs` 增 `DictState { Stable, PendingReindex, Rebuilding }`；`CollectionInner` 增 `dict_state: RwLock<DictState>` + `pending_dict: RwLock<Vec<UserDictEntry>>`；`set_user_dict` 校验 state==Stable||PendingReindex（Rebuilding 时 E_BUSY）→ 覆盖 pending_dict → state=PendingReindex。`dict_state()` 读锁返回。
commit：`api: add DictState and set_user_dict staging (§7.4)`。

### Task 2：reindex 签名变更 + ReindexHandle 骨架

**测试**：
```rust
#[test]
fn reindex_returns_handle_and_progresses() {
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col = setup_col_with_docs(&db);
    col.set_user_dict(&[UserDictEntry::Word("新词".into())]).unwrap();
    let handle = col.reindex().unwrap();  // 签名变更：Result<ReindexHandle>
    let p0 = handle.progress();
    handle.wait().unwrap();
    assert_eq!(col.dict_state(), DictState::Stable);
}
```
验证失败：`reindex` 返回 `Result<()>`（M0 占位），无 ReindexHandle。
最小实现：`api/reindex.rs` 定义 `ReindexHandle { inner: Arc<ReindexInner> }` + `ReindexInner { state: Mutex<RebuildState>, condvar: Condvar }`；`RebuildState { progress: f32, done: bool, error: Option<VaneError> }`。`reindex()`：校验 PendingReindex → state=Rebuilding → 构建 ReindexHandle → 同步执行（M1 先同步，后台化留 Executor）MergeTask 逐段重建 → 完成后 state=Stable。`progress()`/`wait()` 实装。
**签名变更说明**（报告 R-2）：`reindex()` 从 M0 `Result<()>` 落实为 SPEC §4.1 `Result<ReindexHandle>`。M0 README 标注 "ReindexHandle 留 M1"。Node 绑定 ReindexTask 适配（Output 改 ReindexHandle，JsValue 新增 VaneReindexHandle napi struct）。
commit：`api: change reindex signature to Result<ReindexHandle> (SPEC §4.1)`。

### Task 3：reindex 重建倒排（新分词身份）

**测试**（`crates/vane-core/tests/userdict_reindex.rs`）：
```rust
#[test]
fn reindex_rebuilds_inverted_with_new_tokenizer() {
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
    let col = setup_col_with_docs(&db);  // 含 "机器学习" 文档
    // 原身份（standard）切 "机器学习" 可能不正确
    let hits_before = col.search(&SearchQuery { text: Some("机器学习".into()), top_k: 10, mode: SearchMode::Text, ..Default::default() }).unwrap();
    // setUserDict 注入 "机器学习" 词
    col.set_user_dict(&[UserDictEntry::Word("机器学习".into())]).unwrap();
    let handle = col.reindex().unwrap();
    handle.wait().unwrap();
    // reindex 后新身份切 "机器学习" 为单 token，命中
    let hits_after = col.search(&SearchQuery { text: Some("机器学习".into()), top_k: 10, mode: SearchMode::Text, ..Default::default() }).unwrap();
    assert!(hits_after.len() >= 1);
    // tokenizer_id 已变（新身份）
    assert_ne!(col.tokenizer_id(), &old_id);
}
```
最小实现：reindex 内部：构建新 tokenizer（build_tokenizer 或 build_jieba_tokenizer）+ 新 TokenizerId → 对每段：**从 `SegmentReader::text` 读原文**（00 产出）→ 用新 tokenizer 重新 tokenize → `InvertedIndexBuilder::add_document` 重建倒排（非 posting remap）→ write_inverted 新段 → vectors/hnsw 不变（复制或重写）→ `set_text` 原文写入新段 → `set_scalar` 标量重写（从源段 ScalarReader 读，重映射 docid，Q-7 同 02 merge）→ manifest 替换 ULID → 更新 CollectionInner.tokenizer/tokenizer_id。
**测试前提注意**（可行性 reviewer m5）：M0 `StandardTokenizer::new(user_dict)` 不消费 user_dict 做切分（仅影响 TokenizerId）。故 standard + userDict 变更只改 id 不改切分，reindex 后 tokenization 不变，`hits_after.len()>=1` 虽过但无意义。reindex 真正有意义的场景是 jieba + userDict（jieba feature 启用）。M1 测试策略：Task 3 用 standard 验证**管线不崩 + 身份切换**（`tokenizer_id` 已变），jieba 场景的切分改善验证留 10-ci-m1 的 jieba feature job。
commit：`api: reindex rebuilds inverted from original text with new tokenizer (B-1/00)`。

### Task 4：Rebuilding 期旧段只读服务 + 写路径 E_BUSY

**测试**：
```rust
#[test]
fn rebuilding_writes_rejected_with_busy() {
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col = setup_col_with_docs(&db);
    col.set_user_dict(&[UserDictEntry::Word("新词".into())]).unwrap();
    // 模拟 Rebuilding（手动设 state 或在 reindex 执行中测试）
    // M1 同步执行 reindex，Rebuilding 窗口短；用异步/手动注入测试
    col.set_state_for_test(DictState::Rebuilding);
    let r = col.add(&[Doc { id: "x".into(), text: None, vector: None, meta: None }]);
    assert!(matches!(r, Err(crate::types::VaneError::Busy)));
    // 查询仍可用（旧段只读）
    let hits = col.search(&SearchQuery { text: Some("hello".into()), top_k: 10, mode: SearchMode::Text, ..Default::default() });
    assert!(hits.is_ok());
}
```
最小实现：`Collection::add`/`flush`/`delete`/`compact` 检查 `dict_state==Rebuilding` → `Err(VaneError::Busy)`。search 不受影响（旧段只读）。`set_state_for_test` 测试辅助。
commit：`api: reject writes during Rebuilding with E_BUSY (§7.4)`。

### Task 5：reindex 完成原子切换 + WAL

**测试**：
```rust
#[test]
fn reindex_atomic_switch_new_identity_active() {
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
    let col = setup_col_with_docs(&db);
    let old_ulids = col.segment_ulids();
    col.set_user_dict(&[UserDictEntry::Word("新词".into())]).unwrap();
    let handle = col.reindex().unwrap();
    handle.wait().unwrap();
    // 新段 ULID 全替换
    let new_ulids = col.segment_ulids();
    assert_ne!(old_ulids, new_ulids);
    // 旧段目录已删
    for ulid in &old_ulids {
        let files = vfs.list(&format!("db/segments/seg_{}", ulid)).unwrap_or_default();
        assert!(files.is_empty(), "old segment must be deleted");
    }
    // manifest 持久化了新身份
    let db2 = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col2 = db2.collection("c", schema(), CollectionOptions::default()).unwrap();
    assert_eq!(col2.dict_state(), DictState::Stable);
    assert_eq!(col2.tokenizer_id(), col.tokenizer_id());
}
```
最小实现：reindex 完成 → manifest save_atomic（新 ULID 替换旧 + CollectionMeta.tokenizer_id/user_dict 更新）→ Vfs::delete 旧段目录 → WAL append DeleteSegment(旧) + AddSegment(新) + truncate → 更新 snapshot/inverted_readers/hnsw_readers 缓存 → state=Stable。
commit：`api: atomic reindex switch with manifest and WAL`。

### Task 6：不变量 I-4（单一分词身份）

**测试**：
```rust
#[test]
fn single_tokenizer_identity_throughout_reindex() {
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col = setup_col_with_docs(&db);
    let old_id = col.tokenizer_id().clone();
    col.set_user_dict(&[UserDictEntry::Word("新词".into())]).unwrap();
    // PendingReindex：旧身份
    assert_eq!(col.tokenizer_id(), &old_id);
    let handle = col.reindex().unwrap();
    // Rebuilding：查询命中旧段（旧身份），新段未切换前不参与查询
    // （M1 同步执行，Rebuilding 窗口短；验证 wait 后新身份）
    handle.wait().unwrap();
    assert_ne!(col.tokenizer_id(), &old_id);
    // 全库只剩新身份段
    for reader in col.snapshot_readers() {
        assert_eq!(reader.meta().tokenizer_id, *col.tokenizer_id());
    }
}
```
commit：`api: assert single tokenizer identity I-4`。

## 验收标准

- **SPEC §7.4**：状态机 Stable→PendingReindex→Rebuilding→Stable；setUserDict 暂存不生效；reindex 显式触发；Rebuilding 旧段只读服务；原子切换。
- **SPEC §7.4 禁止行为**：新旧身份混排（Task 6 验证）、自动全量重建（reindex 显式）、查询期多版本合并（Rebuilding 命中旧段）。
- **Rebuilding 期 E_BUSY（Q-6）**：M1 选择 Rebuilding 期写路径返回 E_BUSY（比 SPEC §7.4 更严格）；SPEC 允许未来放宽。
- **SPEC §4.1**：reindex 返回 ReindexHandle（progress/wait）。
- **SPEC §5.4/不变量 I-4**：任意时刻一 collection 一套 TokenizerId；新写入在 reindex 完成前用旧身份。
- **B-1/00 前置**：reindex 从旧段 `SegmentReader::text` 读原文重新分词（原文持久化是 reindex 可实现性的前提）。
- **M0 占位对接**：reindex 从 E_UNSUPPORTED 落实为 ReindexHandle。

## 前置依赖

- M0 api/persistence/tokenizer（已合并）。
- 05-jieba-lite（JiebaTokenizer 新身份构建——若 jieba 未启用，reindex 仍可用于 standard/cjk_bigram 的 userDict 变更，Task 3 用 standard 验证管线 + 身份切换）。
- 02-tombstone-merge（MergeTask 管线复用——reindex 传新 tokenizer，倒排走重新分词而非 posting remap）。
- **00-text-persistence**（`SegmentReader::text` 读原文重新分词，B-1 前置——无原文则 reindex 不可实现）。

## Global Constraints

core 禁 std::fs；并发原语 std::sync（ReindexHandle 用 Mutex+Condvar，非 dashmap/parking_lot）；reindex 复用 MergeTask（不另起管线）；manifest 原子切换（I-6）；WAL 记录段增删；reindex 是显式低频操作（性能不苛求，但合并不阻塞读）。
