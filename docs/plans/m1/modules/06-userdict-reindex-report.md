# 06-userdict-reindex 实装报告

> 模块：setUserDict + reindex 状态机（§7.4）+ ReindexHandle
> 提交：`1d02c86`
> 日期：2026-08-09

## Task 改动

### Task 1：DictState 枚举 + set_user_dict 暂存
- `api/types.rs`：新增 `DictState { Stable, PendingReindex, Rebuilding }`。
- `CollectionInner`：增 `dict_state: RwLock<DictState>` + `pending_dict: RwLock<Vec<UserDictEntry>>` + `tokenizer_kind: BuiltinTokenizer`。
- `Collection::set_user_dict`：校验非 Rebuilding（E_BUSY）+ DictTooLarge → 覆盖 pending_dict → state=PendingReindex。
- `Collection::dict_state()`/`tokenizer_id()` 访问器。
- **偏离**：`tokenizer`/`tokenizer_id` 从 CollectionInner 直接字段改为 `RwLock<...>` 包装，以支持 reindex 原子替换（CollectionInner 在 Arc 中，需 interior mutability）。所有 access site（flush/search/merge_segments/db.rs 幂等校验）同步改为 `.read().unwrap()`。

### Task 2：reindex 签名变更 + ReindexHandle
- `api/reindex.rs`（新建）：`ReindexHandle { inner: Arc<ReindexInner> }` + `ReindexInner { state: Mutex<RebuildState>, condvar: Condvar }`。`progress()`/`wait()` 实装。
- **签名变更**（R-2 已批准）：`reindex()` 从 M0 `Result<()>` 落实为 SPEC §4.1 `Result<ReindexHandle>`。
- M1 同步执行（R-4/R-6）：`reindex()` 内同步完成重建后返回 `ReindexHandle::completed()`（progress=1.0, wait 立即返回）。后台化留 M2 Executor。
- 非 PendingReindex 状态返回 `InvalidArg`；compact 进行中返回 `E_BUSY`。

### Task 3：reindex 重建倒排（新分词身份）
- **reindex 路径**：**独立路径，非复用 MergeTask**。原因：MergeTask 走 posting remap（同分词器 docid 重映射），reindex 需用新分词器重新 tokenize 原文（B-1：分词器变了必须重新分词）。两者管线结构相似但倒排重建逻辑不同。
- `reindex_segment()`（api/reindex.rs）：逐段从旧段 `SegmentReader::text` 读原文（00 前置）→ 新分词器 tokenize → `InvertedIndexBuilder::add_document`。vectors/hnsw/idmap/stored/scalars 经 SegmentWriter 重写。
- **tombstone 处理**：reindex **不跳过 tombstone 文档**（与 compact 区分）。docid 顺序不变 → tombstone 位图（绝对 docid）对新段同样有效，仅需 re-key ULID。理由：reindex 只换分词身份，不做物理清除；若跳过 tombstone 会导致 docid 重排，破坏 tombstone 位图语义。
- **HNSW**：重建（非复制）。同 docid 顺序插入 → 功能等价图。计划允许「复制或重写」，此处重写（SegmentWriter 产出新段后，从新段 vectors 重建 HnswWriter）。
- 测试策略：standard 分词器不消费 user_dict 做切分（仅影响 TokenizerId），故 reindex 后 tokenization 不变但 tokenizer_id 已变。jieba 场景的切分改善验证留 10-ci-m1 的 jieba feature job。

### Task 4：Rebuilding 期旧段只读服务 + 写路径 E_BUSY
- `add`/`flush`/`delete`/`compact` 检查 `dict_state == Rebuilding` → `Err(VaneError::Busy)`（Q-6）。
- search 不受影响（旧段只读）。
- `set_state_for_test` 测试辅助（模拟 Rebuilding 窗口）。

### Task 5：reindex 完成原子切换 + WAL
- manifest `save_atomic`（ULID 替换 + `tokenizer_id`/`user_dict` 更新，I-6）。
- 旧段目录经 `delete_segment_dir` 删除（Vfs::list 递归 + Vfs::delete）。
- 内存快照/inverted_readers/hnsw_readers/scalar_readers/tombstones 全部更新。
- `tokenizer`/`tokenizer_id` RwLock 写锁替换为新身份。
- state → Stable。
- **WAL**：M1 未接 04-wal（04 模块尚未实装）。reindex 的段增删经 manifest 原子切换保证一致性；WAL 记录留 04 模块接入时补。

### Task 6：不变量 I-4（单一分词身份）
- `single_tokenizer_identity_throughout_reindex`：PendingReindex 旧身份 → reindex 后新身份 → 全库段头 tokenizer_id 一致。
- `add_after_reindex_uses_new_tokenizer`：reindex 后新 add + flush 的段使用新身份。

## Node/FFI 同步
- Node：`VaneReindexHandle` napi struct（progress/wait）+ `ReindexTask` Output 改 `ReindexHandle` + `set_user_dict`/`dict_state` 异步方法。`parse_dict_entry` 改 pub。
- FFI：vane-ffi 仍为 M0 占位（09-go-cgo-binding 模块负责实装）。`vane_reindex`/`vane_reindex_progress`/`vane_reindex_wait` 的 C ABI 落地留 09。

## 状态机测试结果
- `set_user_dict_enters_pending_reindex`：Stable → PendingReindex ✓
- `pending_reindex_new_writes_use_old_tokenizer`：I-4 旧身份 ✓
- `reindex_returns_handle_and_progresses`：PendingReindex → Rebuilding → Stable ✓
- `reindex_on_stable_returns_invalid_arg`：Stable 非 PendingReindex → InvalidArg ✓
- `rebuilding_writes_rejected_with_busy`：Q-6 E_BUSY（add/flush/delete/compact）✓
- `set_user_dict_during_rebuilding_returns_busy`：Rebuilding 期 set_user_dict → E_BUSY ✓
- `single_tokenizer_identity_throughout_reindex`：I-4 全库新身份 ✓

## I-4 验证
- PendingReindex 期新写入用旧 TokenizerId（验证 `col.tokenizer_id()` == old_id）。
- reindex 完成后 `col.tokenizer_id()` != old_id。
- 全库 `snapshot_readers()` 段头 `tokenizer_id` == `col.tokenizer_id()`。
- reindex 后新 add + flush 的段也使用新身份。

## reindex 路径（复用 MergeTask 或独立）
**独立路径**。`reindex_segment()` 在 `api/reindex.rs` 独立实装，不调用 `MergeTask`。
- MergeTask：posting remap（同分词器，docid 重映射）+ 图重建 + 物理清除 tombstone。
- reindex：新分词器重新 tokenize 原文（B-1）+ 图重写 + 保留 tombstone（docid 顺序不变）。
两者管线结构相似但倒排重建逻辑根本不同，独立实装更清晰。

## 偏离与裁决
1. **tokenizer/tokenizer_id 改 RwLock**：CollectionInner 在 Arc 中，reindex 需原子替换 tokenizer/tokenizer_id。改为 `RwLock<Arc<dyn Tokenizer>>` / `RwLock<TokenizerId>` 提供 interior mutability。所有 access site 同步改。非 M0 冻结 pub API 变更（字段为 pub(crate)）。
2. **reindex 不跳过 tombstone**：计划未明确 reindex 对 tombstone 的处理。裁决：保留（docid 顺序不变，tombstone 位图有效），与 compact（物理清除）区分。测试 `reindex_preserves_tombstone` 验证。
3. **HNSW 重建而非复制**：计划允许「复制或重写」。选择重写（SegmentWriter 产出新段后从 vectors 重建 HnswWriter），避免 Vfs 文件级复制的复杂性。
4. **WAL 未接**：04-wal 模块尚未实装。reindex 的段增删一致性经 manifest 原子切换保证。WAL 记录留 04 接入。
5. **ReindexHandle::failed 未使用**：M1 同步执行下 reindex 失败直接返回 Err（不返回 handle）。`failed()` 构造器标记 `#[allow(dead_code)]`，留 M2 后台化时使用。

## 自证门禁
```
cargo test --workspace --all-features          → 243+31 passed, 0 failed, 1 ignored
cargo clippy --workspace --all-targets --all-features -- -D warnings  → clean
cargo check --target wasm32-unknown-unknown -p vane-core  → clean（零 cfg）
cargo fmt --all -- --check                     → clean
bash scripts/check-no-std-fs.sh                → OK
bash crates/vane-node/scripts/check-thin.sh    → OK
cargo bench --no-run -p vane-core              → OK
```

## 提交 hash
- `1d02c86`：api: setUserDict + reindex 状态机 + ReindexHandle（06-userdict-reindex）

## 遗留/疑问
1. **WAL 接入**：reindex 的段增删（DeleteSegment + AddSegment）应在 04-wal 实装后记录，compact 后 truncate。当前仅靠 manifest 原子切换，崩溃恢复可能残留孤儿新段（manifest 未切换前崩溃）。04 模块接入时补 WAL 记录 + recover 清理。
2. **FFI C ABI**：`vane_reindex`/`vane_reindex_progress`/`vane_reindex_wait` 留 09-go-cgo-binding 实装。core 侧 ReindexHandle 已就绪。
3. **jieba feature 场景**：standard 分词器不消费 user_dict 做切分，reindex 后 tokenization 不变。jieba 场景的切分改善验证留 10-ci-m1 的 jieba feature job。
