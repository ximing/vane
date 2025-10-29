# 06-userdict-reindex 代码审查

> 基线 BASE=7d5722a..HEAD，diff：`git diff 7d5722a..HEAD -- crates/`
> 编排者门禁已确认 274 测试绿 + clippy/wasm32/fmt/no-std-fs/thin 全过
> 日期：2026-08-09

## 逐维度结论

### 1. §7.4 状态机 — ✅

- 全路径 Stable→setUserDict→PendingReindex→reindex→Rebuilding→Stable 实装：`collection.rs:928-957`（reindex 入口：校验 PendingReindex → Rebuilding → run_reindex → Stable）。
- PendingReindex 期间新写入用旧身份：`flush`/`add` 经 `tokenizer_id.read().unwrap().clone()`（`collection.rs:237`）取旧 id；`pending_reindex_new_writes_use_old_tokenizer`（reindex_tests.rs:67）验证。
- Rebuilding 旧段只读服务：reindex 同步执行期间 snapshot 未切换前指向旧段，search 不检查 dict_state 可继续读旧段。
- 完成后 manifest 原子切换：`update_manifest_after_reindex`（reindex.rs:214）`save_atomic`（I-6）+ 内存快照/tombstone re-key + tokenizer 替换。
- 「放弃」路径（再次 setUserDict 覆盖暂存）经 `set_user_dict_overwrites_pending` 测试（reindex_tests.rs:90）。

### 2. I-4 单一分词身份 — ⚠️（核心发现，见「阻塞/裁决项 #1」）

- 任意时刻一 collection 一套 TokenizerId：**存在一个非原子窗口**。`run_reindex`（collection.rs:1003-1075）的完成顺序为：
  1. `[snapshot/offsets/inv/hnsw/scalar/tombstones 写锁块]` 切换 snapshot 到新段 → **释放所有写锁**；
  2. 删除旧段目录；
  3. `tokenizer.write() = new_tokenizer`；
  4. `tokenizer_id.write() = new_tokenizer_id`；
  5. `dict_state = Stable`。
- 步骤 1 与步骤 3/4 之间存在窗口：并发 `search`（SPEC §4.3 承诺读路径无锁并发，且 Rebuilding 期允许查询）先取 `snapshot.read()`（已指向**新段**，段头新 id），再取 `tokenizer.read()`（仍**旧**分词器）tokenize 查询 → 新旧身份混排检索，违反 I-4 / §7.4「禁止行为：新旧分词身份混排检索」。
- search 持锁顺序已核实（`collection.rs:697`）：snapshot.read() 先于 tokenizer.read()，故若 reindex 在同一写锁块内同时持有 snapshot.write()+tokenizer.write()，search 会阻塞在 snapshot.read() 直至两者皆释放——这正是修复路径。
- PendingReindex 新写入旧身份：仅校验 `col.tokenizer_id()`（reindex_tests.rs:82），**未验证新 flush 段段头 id**（minor 测试缺口，但 flush 取 id 路径正确，间接成立）。
- 禁止自动全量重建：reindex 显式触发，非 PendingReindex 返回 InvalidArg（reindex_tests.rs:135）。✅
- 禁止查询期多版本合并：Rebuilding 期 snapshot 未切换前查询命中旧段。✅（但见上窗口）

### 3. R-2 签名 — ✅

- `reindex() -> Result<ReindexHandle>`（collection.rs:938），非 `Result<()>`。ReindexHandle `progress()`/`wait()`（reindex.rs:64-78）实装。M0 占位 `Err(Unsupported)` 已落实为 SPEC §4.1。

### 4. reindex 路径（重建倒排 vs 复制图）— ✅（含说明）

- **只重建倒排、不重建图**：倒排用新分词器重新 tokenize 原文 from `SegmentReader::text`（reindex.rs:149-161），`InvertedIndexBuilder::add_document`，非 posting remap。✅ B-1/00 前置满足。
- 新段 ULID（I-1 段不可变）：`SegmentWriter::new` 产出新 ULID（reindex.rs:130）。✅
- scalars/stored/idmap 复用：`set_scalar`/`add_doc`/`set_text` 重写（reindex.rs:147-156）。✅
- **vectors/hnsw**：计划允许「复制或重写」，实装选择**重写**（reindex.rs:171-196：从新段 vectors 重建 HnswWriter）。功能等价（同 docid 顺序、同 vectors、同参数），但 HNSW 层级分配若用非确定 RNG，新图拓扑可能与旧图不同（不影响正确性，仅 recall 分布微异）。
  - 与审查维度「vectors/hnsw 复制」的措辞不符，但计划明确允许重写，非偏离。
  - **裁决建议**：若要严格「图不变」，M2 可改为文件级复制 hnsw.bin/vectors.bin；M1 重写可接受。

### 5. Q-6 E_BUSY — ✅

- Rebuilding 期 add/flush/delete/compact 检查 `dict_state==Rebuilding` → `Err(VaneError::Busy)`（collection.rs:171/222/879/877）。`rebuilding_writes_rejected_with_busy`（reindex_tests.rs:147）覆盖四路径。注明「比 SPEC §7.4 严格」在计划/报告均有记录。✅

### 6. set_user_dict — ✅

- 暂存新词表进 PendingReindex，不即时生效：`pending_dict.write()` + `state=PendingReindex`（collection.rs:928-936）。✅
- user_dict 变化产生新 TokenizerId 经 `compute_tokenizer_id(kind, &pending)`（collection.rs:970），底层 `sha256(algorithm_version ‖ builtin_dict_version ‖ user_dict_bytes)`（id.rs:62）——R-3 满足：词典**内容**升级不进 id（builtin_dict_version 不变），user_dict_bytes 进 id。✅
- DictTooLarge 上限 10 万（reindex_tests.rs:105）。✅

### 7. M1 同步执行 — ✅

- reindex 同步完成，返回 `ReindexHandle::completed()`（collection.rs:1077，progress=1.0）。无后台线程、无 cfg。`reindex_returns_handle_and_progresses` 验证 wait 后 progress==1.0（reindex_tests.rs:131）。✅

### 8. tombstone 保留 — ✅

- reindex 不跳过 tombstone 文档（reindex.rs:99-101 注释），docid 顺序不变，tombstone 位图按 `old_ulids[i]→new_ulids[i]` re-key（collection.rs:1057-1064）。`reindex_preserves_tombstone`（userdict_reindex.rs:149）验证 delete 后 reindex 不复活。✅
- re-key 顺序对应正确：`new_segments` 按 `old_ulids` 顺序构建，`new_ulids[i]` 对应 `old_ulids[i]`。✅

### 9. M0 签名零破坏 — ✅

- `reindex Result<()>→Result<ReindexHandle>`：R-2 已批准的 SPEC IDL 落实。
- `set_user_dict`/`dict_state`/`tokenizer_id()`/`snapshot_readers()`/`set_state_for_test`：新增。
- `tokenizer`/`tokenizer_id` 从直接字段改 `RwLock<...>`：pub(crate) 字段，非公开 API；所有 access site（flush/search/merge/db.rs 幂等校验）同步改 `.read().unwrap()`。✅
- db.rs 幂等校验 `existing.tokenizer_id` 改 `existing.tokenizer_id.read().unwrap()`（db.rs:77）。✅
- 其余 M0 签名不变。✅

### 10. Node 绑定同步 — ✅

- `VaneReindexHandle` napi struct + `progress()`/`wait()`（collection.rs:120-145 of vane-node）。`ReindexTask::Output = ReindexHandle`，`JsValue = VaneReindexHandle`。
- `set_user_dict`/`dict_state` 异步方法绑定（vane-node collection.rs:223-238）。`parse_dict_entry` 改 pub。
- Node 构建门禁绿（thin check + integration test 改为 InvalidArg 断言）。✅
- FFI 留 09：vane-ffi 仍 M0 占位，`vane_reindex`/`vane_reindex_progress`/`vane_reindex_wait` C ABI 留 09-go-cgo-binding。Go cgo「可后移」(SPEC §15)，Node「不可后移」——**Node 已同步**。✅ 合理。

### 11. WAL 边界 — ✅（边界清晰）

- 06 不写 WAL（04 未实装），仅 manifest `save_atomic` 保证一致性（reindex.rs:235）。
- 崩溃恢复：manifest 切换前崩溃 → 新段为孤儿（manifest 仍指旧段），旧段完整 → 04 接入时补 WAL 记录 + recover 清理。报告「遗留/疑问 #1」明确标注。✅
- **次要**：reindex 中途失败（非崩溃，如第 3 段 reindex_segment 报错）时，已写入的前 N 个新段未清理也未进 manifest → 孤儿段残留。run_reindex 失败回退 state=PendingReindex（collection.rs:960-965）但未删除已建新段。M1 同步下可接受（04 接入后统一 recover），但建议 06 在失败路径 best-effort 清理已建新段目录（非阻塞）。

### 12. 不变量 — ✅

- I-1（段不可变）：新段新 ULID，旧段删除非原地改。✅
- I-4：见维度 2 ⚠️。
- I-6（manifest 原子）：`save_atomic`（rename 原语）。✅
- I-5（零 cfg）：reindex.rs/collection.rs 无 `cfg(target)`。core 禁 std::fs：reindex 用 Vfs trait。✅

### 13. 测试质量 — ✅

- 9 单元（reindex_tests.rs）+ 5 集成（userdict_reindex.rs）= 14。
- 覆盖：状态机三态迁移、I-4 全库段头一致、E_BUSY 四写路径、原子切换 ULID 替换 + 旧段删除、reopen 新身份、tombstone 保留、多段逐段重建、reindex 后新 add 用新身份。✅
- 缺口（minor）：PendingReindex 新 flush 段段头 id 未直接验证（仅验证 col.tokenizer_id()）；Rebuilding 并发 search 命中旧段无测试（M1 同步窗口短，难测）。

### 14. 06 陈旧注释 — ✅

- 计划 `06-userdict-reindex.md:68` 已为 `// JiebaTokenizer::id() 直接用 compute_tokenizer_id，不含词典版本（R-3）`——正确。05 实装 `tokenizer/jieba/mod.rs:7` 注释 `JiebaTokenizer::id() 直接用 compute_tokenizer_id(Jieba, user_dict)` 一致。无陈旧「含词典版本」措辞。✅

## 阻塞/裁决项

### #1（需编排者裁决）reindex 完成阶段非原子，存在 I-4 混排窗口

- **现象**：`run_reindex` 释放 snapshot 写锁后、更新 `tokenizer`/`tokenizer_id` 前，并发 search 可命中新段（新 id）但用旧分词器 tokenize 查询 → 新旧身份混排检索，违反 I-4 / §7.4 禁止行为。
- **影响**：M1 测试用 standard 分词器（不消费 user_dict 做切分），tokenization 不变，**无可见效果、不被测试捕获**；jieba feature 场景下会出现 recall 回退（查询 token 与段倒排 token 体系不匹配）。
- **触发条件**：跨线程并发 search 在 reindex 收尾窗口（snapshot 已切换、tokenizer 未切换）执行。SPEC §4.3 明确读路径无锁并发、Rebuilding 期允许查询，故窗口可达。
- **建议修复**（非侵入）：将 `tokenizer.write()`/`tokenizer_id.write()` 移入现有 snapshot 写锁块内（在切换 snapshot 的同时更新 tokenizer/tokenizer_id，再统一释放）。search 持锁顺序为 snapshot.read()→tokenizer.read()，同序无死锁；search 要么见（旧段+旧分词器）要么见（新段+新分词器），无混排。
- **裁决建议**：M1 门禁已绿、standard 场景无影响，可判 APPROVED_WITH_MINOR，但**必须在 jieba feature gate / M2 前修复**。若编排者认为 I-4 红线不可放过任何窗口，则判 CHANGES_REQUESTED。

### #2（minor，非阻塞）reindex 失败路径孤儿段未清理

- `run_reindex` 失败回退 PendingReindex，但已写入的前 N 个新段目录未删除。04 接入后统一 recover 可覆盖；建议 06 失败路径 best-effort 清理已建 `new_segments` 目录。

## 结论

- **verdict：APPROVED_WITH_MINOR**
- 阻塞项：无硬阻塞；#1 为需编排者裁决的唯一疑点（I-4 混排窗口，M1 standard 场景不可见，jieba/M2 前须修）。
- **Node 绑定同步**：✅ 已同步。`VaneReindexHandle` napi struct（progress/wait）+ `set_user_dict`/`dict_state` 异步方法已绑定，Node 构建门禁绿。FFI 留 09 合理（Go cgo 可后移，Node 不可后移——Node 已就位）。
- **reindex 路径（复制图 vs 重建）**：✅ 正确。倒排用新分词器重新 tokenize 原文（非 posting remap）；vectors/hnsw 选择重写（计划允许「复制或重写」），功能等价。唯一隐患是 HNSW 层级分配非确定 RNG 可能致图拓扑微异（不影响正确性）。
- 需编排者裁决疑点：仅 #1（I-4 完成阶段原子性）。
