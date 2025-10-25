# 02-tombstone-merge 实装报告

> 模块：delete tombstone + 段合并 + compact()（L1，建在 01-hnsw + 00-text-persistence 之上）
> 完成日期：2026-08-09
> 提交：407bafb / fc2f98d / e9e9016

## Task 清单与改动

### Task 1：delete 追加 tombstone（内存 + 查询过滤）✅
- `CollectionInner` 增 `tombstones: RwLock<HashMap<String, RoaringBitmap>>`（ulid → 绝对 docid 位图）+ `compacting: Mutex<bool>`。
- `delete(ids)`：遍历 snapshot 段，经 `SegmentReader::local_docid_by_external` 反查 → 定位 ulid + abs docid → 插入位图，返回命中数。
- search 内每段构建 `alive_bm = [base, base+count) − tombstone`（tombstone 为 EXCLUSION，search filter 为 INCLUSION，故转 alive 集），传 `brute_search`/`HnswReader::search`/`InvertedIndexReader::search`。03 计划正式 compile_filter 统一（02 手动并入）。
- 测试：`delete_hides_doc_from_search` / `delete_hides_doc_from_vector_search` / `delete_unknown_id_returns_zero`。

### Task 2：tombstone 持久化经 WAL —— 跳过（后置 04）⏭️
- 按编排者澄清：02 不实现 WAL（04 负责）。Task 2 reopen 测试跳过。
- 02 仅做内存 tombstone；reopen 后 tombstone 丢失（预期行为，测试 `tombstone_not_persisted_without_wal` 显式断言此 02 阶段语义）。
- 04 后续把 `Wal::append(AddTombstone)` 接到 delete，`wal::recover` 重放注入 `CollectionInner.tombstones`。

### Task 3：MergeTask 单段合并 ✅
- 新增 `crates/vane-core/src/merge/mod.rs`：`MergeTask`/`MergeContext`/`pick_merge_candidates`/`finalize_merge`/`delete_segment_dir`。
- `step`：open SegmentReader + InvertedIndexReader → 合并 tombstone（header.bin ∪ 内存注入）→ 遍历 docid 跳过 tombstone → `add_doc` + `set_text`（原文复用，B-1/00）+ `HnswWriter::insert` → posting remap（读 `iter_terms` 重写 docid）。
- `finalize_merge`：writer.finalize + write_inverted + write_hnsw + 新段 tombstone 恒空（物理清除）。
- 测试：`merge_single_segment_drops_tombstoned_docs`（5 docs − 2 tombstoned = 3，原文可读，倒排命中 3，hnsw.bin 存在）。

### Task 4：多段合并 + docid 重映射 ✅
- MergeTask 跨 step 累积 `inv_terms`/`field_lengths`/`hnsw_writer`，`target_docid` 连续递增。
- 测试：`merge_multi_segments_remaps_docid_contiguous`（2 段 6 docs，新 docid 连续 0..6）+ `merge_progress_and_completion`。

### Task 5：compact() + 旧段删除 ✅
- `compact()`：重入保护（`compacting` 标志，E_BUSY）→ `merge_segments(全部 ulid)` → manifest 原子切换 → 内存快照 retain 旧段 + 删旧段目录 + 加新段。
- 单段且无 tombstone 时 no-op。
- 测试：`compact_merges_all_segments_and_removes_old`（3 段 → 1 段，搜索仍命中）/ `compact_physically_clears_tombstone`（compact 后 tombstone 物理清除，再 delete 新 id 仍工作）。

### Task 6：段数超 10 自动合并 ✅
- flush 末尾 `if segment_count() > SEGMENT_MAX { auto_merge_two_smallest() }`。
- `auto_merge_two_smallest`：`pick_merge_candidates` 排序（tombstone 比例降序 + doc_count 升序）→ 取前 2 → `merge_segments`。失败不阻塞 flush（记 stderr）。
- 测试：`flush_auto_merges_when_exceeding_segment_max`（11 次 flush → ≤10 段）。

### Task 7：不变量 I-3 ✅
- 测试 `graph_rebuilt_only_during_merge`：delete 后 hnsw.bin 字节不变；compact 后旧段 hnsw.bin 删除、新段有新 hnsw.bin。

## 偏离与裁决

### 1. posting 迭代方法新增（非破坏扩展）✅
- M0 `InvertedIndexReader` 未暴露 raw postings 迭代（`terms` 字段私有）。
- 新增 `pub fn iter_terms() -> impl Iterator<Item = (&str, &TermEntry)>` + `docid_base()` + `field_lengths()`。
- `TermEntry`/`Block`/`Posting` 字段 M0 已 pub，merge 直接 `e.blocks.iter().flat_map(|b| b.postings.iter())`。
- 属 M1 扩展，非 M0 冻结 pub API 破坏。

### 2. 标量重写后置（Q-7）⏭️
- 02 merge 暂跳过标量重写（`set_scalar` 由 03 实装，03 在 02 之后）。
- 02 merge 处理：vectors + 原文(text) + 倒排(posting remap) + hnsw 重建。标量在 03/06 补。

### 3. Task 2 跳过（WAL 边界）⏭️
- 02 不引入 Wal 类型，不写 wal.log。tombstone 仅内存。04 后续接入。

### 4. MergeTask tombstone 注入方式
- README 契约 `MergeTask::new` 签名不含 tombstone 参数（保持契约不变）。
- 新增扩展方法 `set_tombstones(HashMap<String, RoaringBitmap>)`：api compact 注入内存 tombstone，与源段 header.bin tombstone 取并集。
- 单元测试 fixture 直接写 header.bin tombstone（走 `reader.meta().tombstones` 路径）；api compact 走 `set_tombstones` 内存路径。两者并集覆盖。
- 测试 `merge_with_injected_memory_tombstones` 验证并集语义。

### 5. tokenizer 字段 Box → Arc<dyn Tokenizer>
- `CollectionInner.tokenizer` 从 `Box<dyn Tokenizer>` 改为 `Arc<dyn Tokenizer>`（私有字段，非 pub API 破坏）。
- 原因：MergeTask::new 需 `Arc<dyn Tokenizer>`（M-2 契约），compact 复用 collection 当前 tokenizer 需克隆。Arc 可 clone，Box 不可。
- `build_tokenizer` 仍返回 `Box`，create_new 内 `Arc::<dyn Tokenizer>::from(box)` 转换。

### 6. delete_segment_dir 递归删除
- Vfs::delete 仅删单文件（MemoryVfs/StdFsVfs 一致）。段目录含 header/vectors/stored/idmap/scalars/inverted/hnsw 多文件。
- 实现 `delete_segment_dir`：经 `Vfs::list` 递归收集文件路径，逐个 `vfs.delete`（core 禁 std::fs）。

## 自证门禁结果（全绿）

| 门禁 | 结果 |
|---|---|
| `cargo test --workspace --all-features` | ✅ 218 lib + 8 tombstone_merge + 5 merge + 其他全过 |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | ✅ 零告警 |
| `cargo check --target wasm32-unknown-unknown -p vane-core` | ✅ 零 cfg |
| `cargo fmt --all -- --check` | ✅ |
| `bash scripts/check-no-std-fs.sh` | ✅ OK |
| `bash crates/vane-node/scripts/check-thin.sh` | ✅ I-8 clean |
| `cargo bench --no-run -p vane-core` | ✅ 编译通过 |

## 提交 hash

- `407bafb` bm25/segment: M1 扩展 postings 迭代与 external_id 反查
- `fc2f98d` merge: MergeTask 段合并 + posting remap + 图重建（Task 3/4，B-1/I-3）
- `e9e9016` api: delete tombstone + compact + auto-merge（Task 1/5/6/7）

## 测试摘要

新增 13 测试（5 merge 单元 + 8 tombstone_merge 集成），更新 2 处 M0 占位测试（api::tests + vane-node integration）。全 workspace 218+ 测试绿。

## 遗留/疑问

1. **WAL 接入（04）**：delete 当前仅内存 tombstone，reopen 后丢失。04 需把 `Wal::append(AddTombstone)` 接到 delete，`recover` 重放注入 `CollectionInner.tombstones`。compact 后 `Wal::truncate`（B-2）。
2. **filter compile_filter（03）**：02 search 手动把 tombstone 并入 alive_bm，每段 O(count) 构建。03 接入 compile_filter 后统一 AND 用户 filter + tombstone，并补 `should_fallback_brute`（低选择率暴力回退）。02 的 alive_bm 构建 03 可优化为预编译缓存。
3. **reindex（06）**：`MergeTask::new` 持 `tokenizer` 仅为管线复用契约；06 reindex 传新 tokenizer 且倒排走 `InvertedIndexBuilder::add_document` 重新分词（非 posting remap）。02 的 `inv_terms` 累积路径 06 不复用，但 `MergeTask` 框架（step/finalize_merge/manifest 切换/旧段删除）可复用。
4. **compact 后 docid_base 重置为 0**：当前 compact 把新段 docid_base 设为 0，与 `next_docid`（write_state 全局计数器）可能冲突——compact 后新写入的 docid 从 `next_docid`（旧值）继续，与新段 docid_base=0 不连续。这不影响正确性（docid 经 ulid→offset 映射，非全局连续），但若 03 filter 依赖全局连续 docid 需注意。编排者裁决：是否 compact 后重置 `next_docid` 为新段 doc_count？当前未重置（保守，避免并发写期 compact 的 docid 碰撞——但 02 compact 同步执行无并发）。
