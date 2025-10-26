# 02-tombstone-merge 代码审查

> 审查者：review agent（只读，未运行 cargo）
> 基线：BASE=919936f → HEAD=8cdf747（407bafb / fc2f98d / e9e9016）
> 审查对象：`crates/vane-core/src/merge/mod.rs`（新增 314 行）、`bm25.rs`（+17）、`segment/mod.rs`（+10）、`api/collection.rs`（+257/-20）、`api/tests.rs`、`merge/tests.rs`（新增 238）、`tests/tombstone_merge.rs`（新增 237）、`vane-node/tests/integration.rs`

## 逐维度结论

### 1. delete tombstone 正确性 ✅
- `CollectionInner.tombstones: RwLock<HashMap<String, RoaringBitmap>>`（collection.rs:52-53）存 ulid → 绝对 docid 位图。
- `delete(ids)`（collection.rs:712-744）：逐段 `local_docid_by_external` 反查 → `abs = base + local` → `bm.insert(abs as u32)`，返回命中数。abs>u32::MAX 跳过（与 search 一致）。`break` 一个 id 只命中一个段。正确。
- search 并入 filter（collection.rs:511-536）：每段构建 `alive_bm = [base, base+count) − tombstones[ulid]`，传 `brute_search`/`HnswReader::search`/`InvertedIndexReader::search`。tombstone 为 EXCLUSION、filter 为 INCLUSION，转 alive 集语义正确。02 用户 filter 恒 None，`merged_filter = alive_ref.or(filter_bm)`。正确。

### 2. MergeTask posting remap（B-1）✅
- `step()`（merge/mod.rs:159-223）：从 `InvertedIndexReader::iter_terms()` 读 term→TermEntry，遍历 `blk.postings`，用 `remap.get(&p.docid)`（old_abs → new_abs）重写 docid，累积到 `inv_terms: HashMap<String, HashMap<u64,u32>>`。
- `finalize_merge`（merge/mod.rs:240-282）：`inv_terms` → `Vec<Posting>` 按 docid 排序 → `TermPostings{doc_freq, postings}` → `InvertedData{docid_base=target_docid_base, field_lengths, ...}` → `write_inverted`。
- 关键核查：M0 flush 期 `inv_builder.add_document(global_docid, ...)`（collection.rs:251）存的是**绝对 docid**（base+local），与 `remap` 键 `abs = src_base + local` 一致 → remap 命中正确。
- field_lengths 索引：`step()` 按 new_local 顺序 push（merge/mod.rs:210），`finalize_merge` 直接作为 `InvertedData.field_lengths`（索引 = local docid，与 bm25.rs:119 注释一致）。正确。
- 不重新分词：MergeTask 持 `Arc<dyn Tokenizer>` 但 `step()` 内从不调用 tokenize（`#[allow(dead_code)]` 标注，merge/mod.rs:62）。符合 B-1。

### 3. iter_terms 扩展非破坏 ✅
- 新增 `InvertedIndexReader::iter_terms()`（bm25.rs:487-490）、`docid_base()`（bm25.rs:478）、`field_lengths()`（bm25.rs:483）——均为新增 pub 方法，不改既有签名。
- `InvertedIndexReader::search` 签名不变（bm25.rs:532-537：`search(&self, query_tokens, topk, filter: Option<&RoaringBitmap>)`）。
- `TermEntry`/`Block`/`Posting` 字段 M0 已 pub，merge 直接 `e.blocks.iter().flat_map(...)`。非破坏。

### 4. 图重建（I-3）✅
- `step()` 首个有 vector 字段的段创建 `HnswWriter::new(dim, metric, 16, 200)`（merge/mod.rs:179-184），每文档 `hw.insert(new_local as u32, v)`（merge/mod.rs:205-208）。
- `finalize_merge`：`hw.build()` → `write_hnsw`（merge/mod.rs:284-291）。从零重建，不读旧图。
- delete 不动 hnsw.bin：`delete()` 只写内存 tombstones 位图，不调任何段文件写。Task 7 测试 `graph_rebuilt_only_during_merge` 字节断言通过。

### 5. 原文复用（B-1/00）✅
- `step()`：`writer.set_text(reader.text(local).unwrap_or(""))?`（merge/mod.rs:201）。从源段 `SegmentReader::text` 读原文写入新段。`merge_single_segment_drops_tombstoned_docs` 断言 `reader.text(0) == "hello rust"`。

### 6. compact + auto-merge ⚠️（见维度 8 的 base 碰撞）
- `compact()`（collection.rs:760-785）：`compacting` 标志重入保护 → E_BUSY → `run_compact()` → 全段或单段有 tombstone 才合并。0 段/1 段无 tombstone no-op。正确。
- `merge_segments`（collection.rs:362-436）：MergeTask → finalize → `manifest_store.save_atomic`（I-6）→ 内存快照 swap（retain 非合并段 + push 新段）→ `delete_segment_dir` 删旧段 → 缓存（snapshot/seg_offsets/inverted_readers/hnsw_readers/tombstones）更新。
- auto-merge（collection.rs:329-356）：flush 末尾 `segment_count() > SEGMENT_MAX` → `auto_merge_two_smallest` → `pick_merge_candidates` 排序（tombstone 比例降序 + doc_count 升序）取前 2。失败记 stderr 不阻塞 flush。
- **潜在问题**：`target_docid_base = 0` 硬编码（collection.rs:365）。compact()（合并全部段）安全；auto_merge（合并 2/N 段）当非合并段中存在 base=0 的段时会碰撞（详见维度 8）。

### 7. docid 重映射 ✅
- `target_docid` 从 `target_docid_base` 起连续递增（merge/mod.rs:155、213）。跨 step 累积，多段合并新 docid 连续从 0 起。`merge_multi_segments_remaps_docid_contiguous` 断言新段 docid_base=0、docids=0..6。正确。

### 8. next_docid 重置（裁决项）—— 明确结论

**结论：next_docid 不重置是正确且无害的；真正需要裁决的是 `target_docid_base=0` 在 partial auto-merge 下的碰撞风险。**

分两个路径分析：

**(a) compact()（合并全部段）—— 安全，无需重置 next_docid**
- 合并后旧段全部移除，新段 base=0、count=N。snapshot 仅剩新段。
- `next_docid` 保持旧值（stale-high，≥ 所有旧 base+count）。后续 `add()` 从 stale-high 继续分配 → 新段 flush 后 base=stale-high，与新段 [0,N) 无重叠。
- stale-high 不影响正确性：docid 经 `seg_offsets[ulid] → base` 映射，非全局连续；alive_bm 按段 [base, base+count) 构建；tombstone 按 ulid 存绝对 docid。03 filter 若用绝对 docid 位图，按段 base 偏移同样成立。
- **若重置反而会出错**：compact 不持 `write_state` 锁，若 buffer 中有未 flush 的 doc（docid 已分配），重置 next_docid 下行会与之碰撞。故保守不重置是正确的。

**(b) auto_merge_two_smallest（合并 2/N 段）—— 潜在碰撞 ❌**
- `target_docid_base=0` 硬编码（collection.rs:365）。新段占据 [0, new_count)。
- 若非合并段中存在 base=0 的段（即首次 flush 创建的段，doc_count 较大时不会被选入"最小两段"），则新段 [0,new_count) 与旧段 [0, old_count) **docid 空间重叠**。
- 危害：`search` 回填 Hit.fields 时（collection.rs:664-688）按 `sd.docid.checked_sub(base)` 遍历段，第一个 base=0 的段命中即 break。若新段（被 push 到 snap 末尾）的 hit docid=0，会误命中旧段（在前）的 local 0 → 返回错误的 external_id 与 stored_json。fusion 期若按 docid 去重还会丢一条文档。
- 测试 `flush_auto_merges_when_exceeding_segment_max` 未触发：11 段等量（各 1 doc），stable sort 取前两段（base 0、1）→ base=0 段被合并移除，无碰撞。生产环境段大小不均时会暴露。
- **这不是 next_docid 重置问题**，而是 `target_docid_base=0` 对 partial merge 不安全。修复方向：partial merge 时 `target_docid_base` 取一个不与任何非合并段重叠的新基址（如 `max(非合并段 base+count)` 或复用 `next_docid` 并推进），或 auto_merge 强制包含 base 最小的段（脆弱，不推荐）。

**对 03 filter 的影响**：03 若编译全局绝对 docid 位图作为用户 filter，同样依赖各段 base 不重叠。当前 partial merge 的 base 碰撞会污染 03 filter。故建议在 03 开工前修复 partial merge 的 base 分配。

### 9. Task 2 跳过（WAL 边界）✅
- 02 未引入 `Wal` 类型，未写 wal.log。`delete()` 仅写内存位图。
- `tombstone_not_persisted_without_wal` 测试（tombstone_merge.rs:192-211）显式断言 reopen 后 tombstone 丢失（d0 复活），作为 02 阶段预期语义。
- 04 接入路径清晰：`Wal::append(AddTombstone)` → `wal::recover` 重放注入 `CollectionInner.tombstones`；compact 后 `Wal::truncate`（B-2）。报告遗留 #1 已记录。

### 10. 标量后置（Q-7）✅
- `step()` 未调用 `set_scalar`（03 实装）。02 merge 处理 vectors + text + 倒排 remap + hnsw 重建，标量后置 03。符合 Q-7。

### 11. 不变量 ✅
- **I-1（段不可变）**：delete 仅写内存位图，不改 header.bin/vectors.bin/任何段文件。merge 写**新段**，不改源段。
- **I-2（合并后向量+倒排+图同快照）**：`finalize_merge` 依次落盘 writer.finalize（vectors/stored/idmap/header）→ write_inverted → write_hnsw，manifest 原子切换后内存快照一次性 swap（collection.rs:410-435 单个 write 锁临界区）。
- **I-3（图不原地删）**：见维度 4。
- **I-6（manifest 原子切换）**：`manifest_store.save_atomic`（collection.rs:397）在内存 swap 之前。旧段删除在 swap 之后（尽力清理，失败不回滚 manifest）。

### 12. M0 签名零破坏 ✅
- 新增：`SegmentReader::local_docid_by_external`、`InvertedIndexReader::{iter_terms, docid_base, field_lengths}`、`Collection::segment_count`、`merge` 模块。均为新增 pub，非破坏。
- `SegmentReader::external_id`/`stored_json`/`text`/`vectors`/`dim`、`SegmentWriter::add_doc`/`set_text`/`finalize`、`InvertedIndexReader::search`、`brute_search`、`HnswReader::search` 签名均未变。
- `CollectionInner.tokenizer` 从 `Box<dyn Tokenizer>` 改 `Arc<dyn Tokenizer>`（collection.rs:39）——私有字段，非 pub API；`build_tokenizer` 仍返回 `Box`，`create_new` 内 `Arc::<dyn Tokenizer>::from(box)` 转换。非破坏。

### 13. 范围合规 ✅
- 仅实装 delete/compact/merge。`reindex()`/`db.export()` 仍 `Err(Unsupported)`（collection.rs:786-790）。未引入 Wal 类型、未实装 compile_filter（03）、未做 reindex（06）。
- core 未引 std::fs（`delete_segment_dir` 经 `Vfs::list`+`Vfs::delete`）。
- 零 cfg（除 `#[cfg(test)] mod tests` 与 `#[allow(dead_code)]` 标注 tokenizer 字段）。
- 无黑名单依赖。

## 其他观察（minor）

- **M-minor-1（panic 安全）**：`compact()` 的 `compacting` 标志释放模式（collection.rs:762-778）非真正 finally——若 `run_compact()` panic，`*guard = false` 不执行，`compacting` 永远 true 导致后续 compact 全部 E_BUSY。注释"改用显式 finally 模式"与实现不符。建议用 Drop guard 或 `catch_unwind`。低危（panic 本身已是非正常路径）。
- **M-minor-2（header.bin tombstone 语义未定义）**：`step()` 把 `reader.meta().tombstones` 当**绝对 docid**处理（`tombs.contains(abs as u32)`，merge/mod.rs:170-172）。但测试 fixture `write_segment` 写入的是 local 值（base=0 时 abs==local，掩盖歧义）。02 生产从不写 header.bin tombstone（I-1），故不影响 02 正确性。但 04 WAL replay / 未来模块写 header tombstone 时需明确 abs 语义，否则 base>0 段会出错。建议 SPEC 或代码注释明确"header.bin tombstones 存绝对 docid"。
- **M-minor-3（delete O(segments × ids × doc_count)）**：`local_docid_by_external` 线性遍历 id_map（segment/mod.rs:337-342），`delete` 逐段逐 id 调用。小规模可接受；大规模 delete 性能差。03/06 可优化为段级 external_id→local 反向索引。非阻塞。
- **M-minor-4（finalize_merge hnsw 写失败降级）**：`write_hnsw` 失败仅 eprintln 不返回 Err（merge/mod.rs:285-290）。新段会缺 hnsw.bin → search 期 fallback brute（Q-5 一致）。但 manifest 已登记新段，语义上"合并成功但图缺失"——可接受（与 01-hnsw 的降级策略一致），建议日志级别提升。
- **M-minor-5（auto_merge 失败静默）**：flush 后 auto_merge 失败仅 stderr（collection.rs:313-316），不向上抛。SEGMENT_MAX 是软上限，可接受。但生产环境无观测会盲区。建议接 SPEC §7.2 的 tombstone 比例指标时一并暴露 segment_count metric。

## 阻塞项

无硬阻塞。但**维度 8(b) 的 partial auto-merge base 碰撞**是真实潜在正确性缺陷，需编排者裁决（见下）。

## 需编排者裁决的疑点

1. **partial auto-merge 的 target_docid_base**（维度 8b）：当前硬编码 0，在"非合并段含 base=0 段"时产生 docid 空间重叠，污染 search 回填与 03 filter。三个选项：
   - (A) 02 内修复：partial merge 时 `target_docid_base = max(非合并段 base+count)`（或复用 `next_docid` 并推进），保留 compact() 全合并时 base=0。
   - (B) 推迟到 03：03 接入 compile_filter 时统一处理 base 分配，02 auto_merge 暂时限制"仅当 base=0 段在候选时合并"。
   - (C) 接受风险：文档化为 02 已知限制，M2 重做段管理时统一。
   - 推荐 (A)：改动小（`merge_segments` 按 source 是否覆盖全部段选择 base），闭合 02 边界。

2. **header.bin tombstone 的 abs/local 语义**（M-minor-2）：需在 04 WAL 开工前明确，建议写入 SPEC §6.3 或 SegmentMeta 字段注释。

3. **compact() panic 安全**（M-minor-1）：是否 02 内修 Drop guard，或接受"panic 致 compacting 卡死"的权衡。

## verdict

**APPROVED_WITH_MINOR**

delete/compact/merge 核心逻辑正确，B-1 posting remap、I-3 图重建、原文复用、M0 签名零破坏、范围合规均通过。唯一需裁决的是 partial auto-merge 的 `target_docid_base=0` 碰撞风险（维度 8b）——非 02 测试覆盖范围，但生产可触发；建议编排者在 03 开工前定夺修复方案（推荐选项 A）。

## 对 next_docid 重置的明确结论

**无需重置 next_docid。** compact() 全合并路径下 stale-high 完全无害（新段 base=0 与后续新段 base=stale-high 不重叠），且重置会与 buffer 中已分配 docid 碰撞。partial auto-merge 的问题根源是 `target_docid_base=0` 而非 next_docid——修 target_docid_base 即可，不要动 next_docid。
