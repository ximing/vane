# Vane M1 计划集 SPEC 契约审查报告

> 审查者：SPEC 契约视角审查 SubAgent
> 审查日期：2026-08-09
> 审查对象：`docs/plans/m1/README.md` + `modules/01..12-*.md`（12 份）+ `plan-split-report.md`
> 审查基准：`docs/SPEC.md` v1.0、`docs/REQUIREMENTS.md` v1.1、`docs/plans/m0/README.md`、M0 实际代码 `crates/vane-core/src/`（逐文件核对）
> 模式：只读审查，不改任何文件

---

## 1. 结论

### Verdict: CHANGES_REQUESTED

存在 2 项阻塞级问题（R-3 TokenizerId 词典版本注入方式 + 04-wal WAL truncate 逻辑缺陷），须修正后方可进入实施。其余为 minor 级，可在实施中同步修正。

### 阻塞项（按严重度排序）

| # | 严重度 | 计划 | 问题 | 修复方向 |
|---|---|---|---|---|
| B-1 | 阻塞 | 05-jieba-lite / 06-userdict-reindex | R-3 方案 A（JiebaTokenizer 二次哈希叠加 `dict.version() + sha256_prefix()`）使 TokenizerId 依赖词典**内容**，词典升级时 TokenizerId 变化 → E_TOKENIZER_MISMATCH → 实质强制重建，违反 REQUIREMENTS §3.3「词典升级打开老库仅警告不强制重建」 | `builtin_dict_version(Jieba)` 改为编译期**格式**版本常量（如 `b"jieba-fmt-v1"`），非日历内容版本；`JiebaTokenizer::id()` 直接用 `compute_tokenizer_id(Jieba, user_dict)`，不做二次哈希 |
| B-2 | 阻塞 | 04-wal | WAL `truncate` 在每次 flush 的 manifest 切换后**清空整个 WAL 文件**，会丢失尚未被 compact 消费的 `AddTombstone` 记录。序列 flush→delete→flush→崩溃 后 reopen 丢失 tombstone → 已删除文档复活（数据损坏） | truncate 改为选择性：仅清除已被 manifest 提交的 AddSegment/DeleteSegment 记录，保留 AddTombstone 直到 compact/merge 物理清除 tombstone 后才清除；或 flush 不 truncate，仅 compact truncate |

### Major 项（实施前须明确，不阻塞计划批准）

| # | 计划 | 问题 |
|---|---|---|
| M-1 | 01-hnsw | 计划提及 `cfg(not(target_arch="wasm32"))` 包 `thread::scope` 作为并行搜索选项——违反 I-5（cfg 只在 Executor/VFS）。须明确 M1 全串行，无 thread::scope，无 cfg（R-6 倾向已正确，但计划正文仍含矛盾表述） |
| M-2 | 02/06 | `MergeTask::new` 签名在 README（无 tokenizer 参数）与 02/06 计划正文（"MergeTask 持 `Box<dyn Tokenizer>`"）不一致。README 是单一事实源，须补 tokenizer 参数 |
| M-3 | 02 | Task 2 测试代码仍调用 `col.sync_tombstones()`，但正文已裁决"采用替代方案（WAL 持久化）"。测试代码须同步更新 |

---

## 2. R-3 / R-4 / R-6 独立结论

### R-3：TokenizerId 词典版本注入方式 — 编排者判定成立 ✅

**编排者判定**：方案 A（二次哈希叠加 `dict.version()`）违反 REQUIREMENTS §3.3 + I-4，应推翻。正解：`builtin_dict_version(Jieba)` = 编译期常量。

**独立核查过程**：

1. **SPEC §5.4 公式**：`TokenizerId := sha256( algorithm_version ‖ builtin_dict_version ‖ user_dict_bytes )`。公式中 `builtin_dict_version` 是组成部分，但 SPEC 未定义其语义（是"格式版本"还是"内容日历版本"）。

2. **REQUIREMENTS §3.3**（注：任务指令称"SPEC §3.3"，实为 REQUIREMENTS §3.3；SPEC §3.3 是"规模红线"）：「内置词典独立日历版本化（如 `2026.08`）……词典升级打开老库**仅警告不强制重建**」。这是硬约束：词典内容升级不得强制 reindex。

3. **M0 代码 `id.rs` 第 23 行注释**：`/// - jieba：M0 占位空串；M1 接入 jieba-lite 后填词典日历版本（如 b"jieba-lite-2026.08"）`。模块文档注释第 4-5 行：`任何分词算法变更（……jieba 词典版本）必须递增对应 version 标签，从而产生新 TokenizerId 触发 reindex`。**M0 注释本身将"日历版本"注入 `builtin_dict_version`，与 REQUIREMENTS §3.3 直接矛盾**。

4. **方案 A 的效果链**：`dict.version()`="2026.08" + `sha256_prefix()`=内容哈希 → TokenizerId 随词典内容变化 → 词典升级（新词剪裁）→ 新 TokenizerId ≠ 旧段 TokenizerId → 查询时 E_TOKENIZER_MISMATCH（SPEC §10 -6）→ 查询被拒绝 → 用户被迫 reindex 才能查询 → **实质强制重建**，违反 §3.3「仅警告不强制重建」。

5. **I-4 关系**：方案 A 使词典升级后 collection 出现两套 TokenizerId（collection 新身份 vs 旧段旧身份），违反 I-4「任意时刻一 collection 一套生效分词身份」。虽 E_TOKENIZER_MISMATCH 阻止查询（非"混排检索"），但状态本身违反 I-4 的单一身份约束。

6. **编排者正解核查**：`builtin_dict_version(Jieba)` = 编译期**格式**版本常量（如 `b"jieba-fmt-v1"`）。语义：仅当 DAT 结构 / HMM 参数格式变更时递增，词典内容升级（增删词条）不变。效果：
   - 词典内容升级 → `builtin_dict_version` 不变 → TokenizerId 不变 → 无 E_TOKENIZER_MISMATCH → §3.3 满足（仅 CHANGELOG 警告）。
   - 词典格式升级 → `builtin_dict_version` 递增 → TokenizerId 变化 → reindex 触发（合理：格式不兼容需重建）。
   - `compute_tokenizer_id` 公开签名不变（仍 `(kind, user_dict) -> TokenizerId`），仅 `id.rs` 内部 `builtin_dict_version(Jieba)` 返回值从 `b""` 改为常量。无二次哈希，`JiebaTokenizer::id()` 直接用 `compute_tokenizer_id(Jieba, user_dict)`。

**结论**：**编排者判定完全成立**。方案 A 须推翻。05 计划 Task 7 须修订（删除二次哈希逻辑，改为 `builtin_dict_version(Jieba)` 返回格式常量）。06 计划无直接影响（reindex 仍由 `user_dict_bytes` 变化触发 TokenizerId 变更，路径正确）。同时须修正 M0 `id.rs` 第 23 行注释（"日历版本"→"格式版本"）及模块文档注释第 4-5 行。

---

### R-4：rayon 依赖 / Executor 抽象 — 延后 acceptable，但须文档化 ⚠️

**问题**：SPEC §11「native 实现 = rayon」，M1 不引入 rayon。

**核查**：

1. **"延后并行到 M2"是否构成 Must 降级**：REQUIREMENTS §2 Must 清单为「HNSW（M1）」「混合搜索」——未将"并行搜索"列为 Must。SPEC §3.1/§8.1「HNSW 段级并行搜索 → 归并」是架构决策描述，非功能 Must。10 万×384 维 ≤10 段串行 HNSW 搜索（每段 ~3-5ms × 10 = 30-50ms）可满足 §13.1 P99 <50ms。**不构成 Must 降级**。

2. **SPEC §11 偏离**：§11 明确「native 实现 = rayon」。M1 不引入 Executor trait、不用 rayon，是对 §11 的阶段性偏离。但 §11 的核心约束是「cfg 只允许出现在 Executor 与 VFS 实现处，核心算法零 cfg」（I-5）。只要 M1 不在核心算法引入 cfg（即全串行），I-5 不违反。rayon 本身不在 deny.toml 黑名单（已核查），M1 不用是自愿选择非禁止。

3. **风险**：若 10 万×384 维 ≤10 段串行搜索实测 >50ms P99，则须引入并行。但 11-cold-start-bench 会实测背书，届时可决策。

**结论**：M1 不引入 rayon、不抽象 Executor **acceptable**，但须在 README 或计划中明确标注「M1 串行搜索，Executor+rayon 延后 M2」作为已知阶段性偏离。若 11-bench 实测超 50ms 则须在 M1 内引入并行（通过 Executor trait，非直接 thread::scope）。

---

### R-6：HnswReader 并行搜索的 cfg 降级 — 须全串行，禁 thread::scope ✅

**问题**：01-hnsw 计划提及 `cfg(not(target_arch="wasm32"))` 包 `thread::scope`，wasm32 串行 fallback。

**核查**：

1. **I-5 约束**：「核心算法零 `cfg(target)`；cfg 只允许出现在 Executor 与 VFS 实现处」。hnsw 模块是核心算法，在其中放 `cfg(not(target_arch="wasm32"))` **直接违反 I-5**。

2. **wasm32 可行性**：`std::thread::scope` 在 wasm32-unknown-unknown 可编译但运行时 panic（wasm 无线程）。因此必须有 cfg 守卫才能在 wasm32 运行——但这又违反 I-5。死结。

3. **plan-splitter R-6 倾向**（M1 先全串行，无 cfg）是**唯一正确解**。01 计划正文仍含矛盾表述（"hnsw 模块零 cfg" vs "若编译失败则用 cfg(not(target_arch=\"wasm32\")) 包 thread::scope"），须统一为「M1 全串行搜索，无 thread::scope，无 cfg」。

4. **std::thread::scope 替代 rayon 是否偏离 §11**：在 hnsw 模块直接用 `thread::scope` 绕过 Executor 抽象，且需 cfg 守卫——既偏离 §11（不经 Executor），又违反 I-5（cfg 在核心算法）。**不可接受**。若 M1 需并行，须先抽象 Executor trait（cfg 仅在 Executor 实现），再经 Executor 调度。M1 全串行避免此问题。

**结论**：**M1 须全串行搜索**。01-hnsw 计划须删除所有 `thread::scope` / `cfg(not(target_arch="wasm32"))` 相关表述，明确「M1 串行，M2 经 Executor trait 引入并行」。

---

## 3. 逐计划审查

### 01-hnsw — ⚠️ APPROVED_WITH_MINOR

| 维度 | 结论 | 证据 |
|---|---|---|
| HnswReader::search 签名 | ✅ | `(query, topk, ef_search, filter: Option<&RoaringBitmap>, docid_base) -> Vec<ScoredDoc>`，与 README 契约一致；filter 参数与 SPEC §8.3「位图进 HNSW 遍历」对齐 |
| hnsw.bin 格式 | ✅ | `magic(4) \| format_version(4 LE) \| dim(4 LE) \| metric(1) \| ...`，遵守 SPEC §6.2「所有文件以 4 字节 magic + 4 字节 format_version 开头」。LE 编码与 M0 header.rs 一致（M0 用 `to_le_bytes`） |
| 自适应回退（§8.1 候选<2×k） | ✅ | 回退由 api 层判定调 `brute_search`（M0 已支持 filter），不在 hnsw 模块内。03 计划 `should_fallback_brute` 实装判定。与 SPEC §8.1「过滤候选 <2×topK 时暴力精确回退」一致 |
| M0 API 对接 | ✅ | `Metric`/`ScoredDoc`/`Vfs`/`brute_search`/`SegmentReader`/`SegmentWriter` 签名均与 M0 代码一致（已核查） |
| I-3 图不删 | ✅ | Task 6 测试字节稳定；图重建仅段合并（02 Task 3/7） |
| **cfg 纪律** | ❌ → M-1 | 计划正文矛盾：声称"hnsw 零 cfg"又提 `cfg(not(target_arch="wasm32"))` 包 `thread::scope`。须统一为全串行（R-6） |
| M=16/ef_construction=200 | ✅ | SPEC §3.1 默认值（README 契约注明） |

### 02-tombstone-merge — ⚠️ APPROVED_WITH_MINOR

| 维度 | 结论 | 证据 |
|---|---|---|
| delete 返回 tombstone 数（§4.1） | ✅ | `delete(&self, ids: &[String]) -> Result<u64>`，M0 占位签名一致（已核查：M0 `delete(_ids: &[String]) -> Result<u64>` 返回 Unsupported） |
| compact 实装 | ✅ | Task 5；E_BUSY 若 reindex 进行中 |
| MergeTask 可切片（§7.3） | ⚠️ | SPEC §7.3「每片处理 N 个 posting 块/图节点后 yield」；计划 `step()` 处理「一个源段全部数据」，粒度粗于 SPEC。M1 同步执行可接受，但须注明切片粒度留 M2 细化 |
| I-3 图重建仅段合并 | ✅ | Task 3 MergeTask 重建 HNSW 图；Task 7 delete 不动 hnsw.bin |
| I-2 双索引原子可见 | ✅ | 合并后向量+倒排+图同快照出现（manifest 切换） |
| 段数硬上限 10（§3.3） | ✅ | Task 6 flush 后检查 `snapshot.len() > SEGMENT_MAX` 自动合并 |
| **MergeTask::new 签名** | ❌ → M-2 | README 契约 `new(sources, target_docid_base, tokenizer_id, schema)` 无 tokenizer 参数；02 正文「MergeTask 持 Box<dyn Tokenizer>，由调用方传入」；06 正文「MergeTask 需接受 Box<dyn Tokenizer>」。须在 README 补 tokenizer 参数 |
| **Task 2 测试代码** | ❌ → M-3 | 测试仍调 `col.sync_tombstones()`，正文已裁决改用 WAL。测试代码须更新 |
| M0 API 对接 | ✅ | SegmentMeta.tombstones: RoaringBitmap（已核查存在）；encode_header/decode_header（已核查 pub）；InvertedIndexBuilder/write_inverted（已核查） |

### 03-pre-filter — ✅ APPROVED

| 维度 | 结论 | 证据 |
|---|---|---|
| Filter 编译为位图（§8.3） | ✅ | `compile_filter(filter, schema, segments, scalars, tombstones) -> Result<RoaringBitmap>` |
| 低选择率暴力回退（§8.3） | ✅ | `should_fallback_brute(bm, topk) = bm.cardinality() < 2*topk`，与 SPEC「位图基数 <2×topK」一致 |
| scalars.col 列式块（§6.2） | ✅ | ScalarReader 新增类型；SegmentWriter::set_scalar 新增方法（不改 M0 add_doc 签名，已核查 M0 无 set_scalar） |
| eq/in/gte/lte + AND（§8.3） | ✅ | Task 2；不支持 OR/NOT（§8.3 M0-M2 限制） |
| tombstone 并入 filter | ✅ | Task 5 `bm.and_not(&segment_tombstones)` |
| M0 API 对接 | ✅ | Filter/FilterCond/ScalarValue（已核查 api/types.rs）；brute_search filter 参数（已核查 7 参数含 filter+docid_base）；InvertedIndexReader::search filter 参数（已核查含 `Option<&RoaringBitmap>`） |
| SearchQuery.filter 解禁 | ✅ | M0 `parse_search_query` 对 filter reject（已核查 convert.rs 第 239-243 行）；M1 移除 reject 改为编译 |

### 04-wal — ❌ CHANGES_REQUESTED（B-2）

| 维度 | 结论 | 证据 |
|---|---|---|
| WalRecord 仅段增删/tombstone（§6.4） | ✅ | `AddSegment / DeleteSegment / AddTombstone` 三变体，与 SPEC §6.4「仅段增删/tombstone元操作」一致 |
| 崩溃恢复（§6.4） | ✅ | Task 3/4 tombstone 重放 + 半成品段清理 |
| I-6 manifest 原子性 | ✅ | Task 6 测试崩溃后 manifest 完整 |
| **WAL truncate 逻辑** | ❌ → B-2 | `truncate = Vfs::create（重置空文件）` 清空整个 WAL。flush 后 truncate 会丢失未消费的 AddTombstone 记录。序列：flush(seg1)→truncate→delete(d1)→WAL[AddTombstone(seg1,d1)]→flush(seg2)→truncate（AddTombstone 丢失！）→崩溃→reopen→tombstone 丢失→d1 复活。**数据损坏**。SPEC §6.4「WAL 重放仅恢复未提交元操作」——AddTombstone 虽即时生效但持久化仅靠 WAL，truncate 后不可恢复 |
| WASM 同步 IO | ✅ | WAL 经 Vfs trait，WASM Worker 内同步 |
| M0 API 对接 | ✅ | Vfs append/sync/read_at（已核查）；Manifest/ManifestStore（已核查） |

**B-2 修复建议**：truncate 改为选择性重写——flush 的 manifest 切换后，重写 WAL 仅保留 AddTombstone 记录（丢弃已提交的 AddSegment）。compact/merge 的 manifest 切换后，AddTombstone 随旧段物理清除，此时可清空 WAL。或更简单：flush 不 truncate（仅 compact truncate），WAL 累积 AddSegment 记录直到 compact（100 条 AddSegment 记录体积可忽略）。

### 05-jieba-lite — ❌ CHANGES_REQUESTED（B-1）

| 维度 | 结论 | 证据 |
|---|---|---|
| DAT+zstd 词典（§5.2） | ✅ | dict.bin 格式：16 字节头 + DAT + HMM 参数；ruzstd 解码（非黑名单，纯 Rust wasm32 安全） |
| 四项验收（§13.2-2） | ✅ | ①200 句 jieba-rs 对照（Task 7+10-ci）；②nDCG 维基（10-ci）；③生造词（Task 6）；④缺词典降级（Task 8） |
| 中英混排（§5.1） | ✅ | Task 5 CJK run 进 DAG+HMM，Latin run 进 standard 管线，position 连续 |
| feature 隔离 | ✅ | `cfg(feature="jieba")` 是 feature cfg 非 target cfg，不违反 I-5；wasm32 构建不启用 jieba feature |
| 词典永不进 wasm | ✅ | jieba feature 默认关；vane-dict-zh 独立 crate |
| jieba-rs dev-dependency | ⚠️ | 仅测试用，非 core 运行时。须验证 jieba-rs 不传递依赖黑名单 crate（regex/ndarray 等） |
| **TokenizerId（§5.4，R-3）** | ❌ → B-1 | 方案 A 二次哈希叠加 `dict.version() + sha256_prefix()` 使 TokenizerId 依赖词典内容，违反 REQUIREMENTS §3.3。须改为 `builtin_dict_version(Jieba)` = 格式常量，无二次哈希 |
| M0 API 对接 | ✅ | Tokenizer trait/Token/UserDictEntry/build_tokenizer/compute_tokenizer_id（已核查）；`builtin_dict_version(Jieba)` 当前返回 `b""`（已核查 id.rs 第 24-29 行） |
| dict.bin 16 字节头 | ✅ | `magic(4) + format_version(4) + sha256_prefix(8)` = 16 字节，与 SPEC §5.2 一致 |

### 06-userdict-reindex — ⚠️ APPROVED_WITH_MINOR

| 维度 | 结论 | 证据 |
|---|---|---|
| 状态机（§7.4） | ✅ | Stable→PendingReindex→Rebuilding→Stable；Task 1/2/4/5 覆盖 |
| ReindexHandle（§4.1） | ✅ | `progress()/wait()`；R-2 签名变更 `Result<()>` → `Result<ReindexHandle>` 已批准 |
| I-4 单一分词身份 | ✅ | Task 6 验证全库单一 TokenizerId；PendingReindex 新写入用旧身份（Task 1） |
| reindex 只重建倒排不重建图 | ✅ | 裁决合理：向量与分词无关，HNSW 图不变。新段 ULID（段不可变），vectors/hnsw 复制 |
| Rebuilding 写路径 E_BUSY | ⚠️ | SPEC §7.4 未明确禁止 Rebuilding 期写入（仅说「查询仍命中旧段」）。计划选择 E_BUSY 比 SPEC 更严格。可接受（保守策略），但须注明 |
| **MergeTask 签名** | ❌ → M-2 | 消费 02 的 MergeTask 须传 tokenizer，README 契约缺参数 |
| M0 API 对接 | ✅ | CollectionInner 字段（已核查 pub(crate)）；CollectionMeta（已核查 5 字段）；compute_tokenizer_id（已核查） |
| reindex 获取 JiebaDict | ⚠️ | 07 计划 DbInner 增 `jieba_dict: Option<Arc<JiebaDict>>`，06 未明确说明 reindex 如何获取 dict 实例（隐含经 DbInner）。须显式化 |

### 07-dict-distribution-node — ✅ APPROVED

| 维度 | 结论 | 证据 |
|---|---|---|
| @vane/dict-zh 正式 dependency（§12.3） | ✅ | 禁 postinstall；平台无关数据包 |
| 体积 ≤1.5MB gzip（§13.2-3） | ✅ | Task 4 CI 门禁 |
| 缺词典降级 bigram + warn（§13.2-2 ④） | ✅ | Task 3 |
| 词典冷加载 <150ms（§13.1） | ✅ | Task 5 criterion bench |
| DbInner.jieba_dict 扩展 | ✅ | pub(crate) 内部结构扩展，非 M0 冻结签名破坏 |
| M0 API 对接 | ✅ | 消费 05 的 JiebaDict::load / build_jieba_tokenizer |

### 08-dict-distribution-go — ✅ APPROVED

| 维度 | 结论 | 证据 |
|---|---|---|
| go:embed dict.bin.gz（§12.3） | ✅ | `//go:build !vane_nodict` + `//go:build vane_nodict` 降级 |
| embed <2MB（§12.3） | ✅ | Task 4 CI 门禁 |
| DictVersion()（§12.3） | ✅ | Task 1 |
| 三渠道哈希一致 | ✅ | Task 5 CI 校验 Node/Go 一致 |
| C ABI 扩展 | ⚠️ | `vane_load_dict`/`vane_dict_version` 不在 SPEC §9.2 函数面。合理扩展但须 spec amendment |

### 09-go-cgo-binding — ✅ APPROVED

| 维度 | 结论 | 证据 |
|---|---|---|
| C ABI 句柄（§9.1） | ✅ | uint64_t + `std::sync::RwLock<HashMap>`（非 dashmap，黑名单合规） |
| 错误码透传（§10） | ✅ | Task 4 i32 状态码 + last_error_message |
| I-7 内存铁律 | ✅ | Task 1 close 两次返回错误非 UB；arena 一次 free |
| I-8 binding 薄壳 | ✅ | Go 测试仅验证调用链，行为测试在 core |
| wazero build tag（§4.3） | ✅ | Task 6 `//go:build wazero` + CGO_ENABLED=0 报错引导 |
| SPEC §9.1 "DashMap" 矛盾 | ✅ | SPEC §9.1 原文「DashMap<u64, Arc<…>>」与黑名单冲突；计划正确用 std::sync（M0 B2 裁决）。SPEC 应修订 |
| FFI 函数扩展 | ⚠️ | `vane_reindex_progress`/`vane_reindex_wait`/`vane_load_dict`/`vane_dict_version` 超出 §9.2。ReindexHandle 的 progress/wait 是 §4.1 IDL 要求（§9.2 遗漏），合理。load_dict/version 是 M1 词典分发扩展，须 spec amendment |
| M0 API 对接 | ✅ | Db/Collection 全部 pub API（已核查）；StdFsVfs（已核查 `#[cfg(not(target_arch="wasm32"))]`） |
| vane-ffi 空占位 | ✅ | 已核查 `crates/vane-ffi/src/lib.rs` 仅一行注释 |

### 10-ci-m1 — ✅ APPROVED

| 维度 | 结论 | 证据 |
|---|---|---|
| recall 五档（§13.2-1） | ✅ | Task 2 recall_regression job |
| wasm 体积（§13.2-3） | ✅ | Task 1 ≤800KB gzip |
| 词典体积（§13.2-3） | ✅ | Task 3 ≤1.5MB/<2MB |
| Go 交叉矩阵（§12.2） | ✅ | Task 5 zig cc 6 平台 |
| 冷启动（§13.1） | ✅ | Task 6 bench |
| jieba 兼容（§13.2-2 ①） | ✅ | Task 4 200 句 |
| §13.2-2 ② nDCG 维基 | ⚠️ | 计划提及但 fixture「离线生成」，无具体生成方案。须明确 fixture 来源与 CI 执行方式 |
| corpus 兼容测试（§13.3） | ⚠️ | M1 新增 hnsw.bin + scalars.col 真实数据。M0 corpus 无 hnsw.bin——须验证 M1 能打开 M0 corpus（HnswReader::open 缺失文件时 fallback brute）。计划未明确此兼容路径 |

### 11-cold-start-bench — ✅ APPROVED

| 维度 | 结论 | 证据 |
|---|---|---|
| 冷启动 <1s（§13.1） | ✅ | Task 2/3 bench + 分级降级断言 |
| 分级降级（§13.1） | ✅ | metadata <1s + 首次查询 <3s |
| fixture 10 万×384 维 | ✅ | Task 1 生成脚本；100 flush 触发 auto-merge 到 ≤10 段 |
| M0 SegmentReader 签名不改 | ✅ | 懒加载留 M2 |
| M0 API 对接 | ✅ | Db::open/StdFsVfs（已核查） |

### 12-recall-regression — ✅ APPROVED

| 维度 | 结论 | 证据 |
|---|---|---|
| recall@10 ≥0.95 五档（§13.2-1） | ✅ | Task 3 五档×三模式 |
| 基线口径（§13.2-1） | ✅ | 暴力双路+RRF（brute_search + InvertedIndexReader::search + rrf_fuse） |
| 低选择率暴力回退（§8.1） | ✅ | Task 4 0.1% 档 recall=1.0 |
| search_brute_baseline | ✅ | `#[doc(hidden)]` 测试辅助，非对外 IDL。合理 |
| M0 API 对接 | ✅ | brute_search/InvertedIndexReader::search/rrf_fuse（已核查全部 pub 签名） |

---

## 4. M0 API 对接核查汇总

逐文件核对了 `crates/vane-core/src/` 全部相关文件。结论：

| M0 签名 | 计划引用 | 一致性 |
|---|---|---|
| `Vfs` trait 8 方法 | 01/02/04/09 | ✅ 一致 |
| `Metric`/`ScoredDoc`/`Result`/`TokenizerId` | 01/05/12 | ✅ 一致 |
| `MAGIC`/`FORMAT_VERSION` 常量 | 01/04 | ✅ 一致 |
| `brute_search(vectors, dim, query, metric, topk, filter, docid_base)` | 01/03/12 | ✅ 一致（7 参数含 filter+docid_base） |
| `SegmentReader::open/vectors/dim/meta/segment_dir/vfs` | 01/02/03/11 | ✅ 一致 |
| `SegmentWriter::new/add_doc/finalize` | 01/02/03 | ✅ 一致 |
| `SegmentWriter::set_scalar` | 03 新增 | ✅ M0 无此方法（已核查），03 新增扩展不改 add_doc |
| `SegmentMeta.tombstones: RoaringBitmap` | 02 | ✅ 一致（已核查，M0 恒为空） |
| `encode_header`/`decode_header` | 02 | ✅ 一致（pub fn） |
| `InvertedIndexReader::search(query_tokens, topk, filter)` | 03/12 | ✅ 一致（含 filter 第 3 参数） |
| `InvertedIndexBuilder::new/add_document/build` | 02 | ✅ 一致 |
| `write_inverted` | 02 | ✅ 一致 |
| `compute_tokenizer_id(kind, user_dict)` | 05/06 | ✅ 一致（公开签名未改） |
| `builtin_dict_version(Jieba)` 返回 `b""` | 05 | ✅ 一致（M0 占位；R-3 须改为格式常量） |
| `build_tokenizer(Jieba)` → DictUnavailable | 05 | ✅ 一致 |
| `Manifest`/`CollectionMeta`/`ManifestStore` | 02/04/06 | ✅ 一致（CollectionMeta 5 字段） |
| `Collection::delete/compact/reindex` 占位 | 02/06/09 | ✅ 一致（均返回 Unsupported；reindex 签名 `Result<()>` → M1 改 `Result<ReindexHandle>`，R-2 已批准） |
| `CollectionInner` 字段 | 02/06 | ✅ 一致（pub(crate)，tokenizer/tokenizer_id/snapshot 等） |
| `parse_search_query` filter reject | 03 | ✅ 一致（convert.rs 第 239-243 行 reject，M1 移除） |
| vane-node 无 `compact` napi 方法 | 02 | ⚠️ 02 须新增 compact napi 导出（plan-splitter 报告已注明） |
| `lib.rs` 无 hnsw/merge/filter/wal 模块 | 01/02/03/04 | ✅ M1 新增 `pub mod hnsw/merge/filter/wal;` |
| `vane-ffi/src/lib.rs` 空占位 | 09 | ✅ 一致 |
| `deny.toml` 含 dashmap/parking_lot，不含 rayon | 全局 | ✅ 一致 |
| `Cargo.toml` 无 `[features]` 段 | 05 | ✅ M1 新增 `[features] jieba = ["ruzstd"]` |

**结论**：M0 API 对接整体无误，无冻结签名被误改（R-2 reindex 已批准除外）。

---

## 5. 跨计划契约一致性

| 契约 | 产出方 | 消费方 | 一致性 |
|---|---|---|---|
| `HnswWriter::new/insert/build` + `write_hnsw` | 01 | 02 | ✅ README 契约与 02 Consumes 一致 |
| `HnswReader::search(query, topk, ef_search, filter, docid_base)` | 01 | 03/12 | ✅ 一致 |
| `MergeTask::new(sources, target_docid_base, tokenizer_id, schema)` | 02 | 06 | ❌ → M-2：06 需传 tokenizer，README 缺参数 |
| `MergeTask::step/progress` + `finalize_merge` | 02 | 06 | ✅ 一致 |
| `WalRecord` 语义 | 04 | 02 | ✅ 02 的 delete/compact 产出对齐 04 的 WalRecord |
| `wal::recover` | 04 | api open 流程 | ✅ 一致 |
| `compile_filter`/`should_fallback_brute` | 03 | 01(api)/12 | ✅ 一致 |
| `ScalarReader`/`SegmentWriter::set_scalar` | 03 | 02(merge 需重写 scalar) | ⚠️ 02 MergeTask 重建段时须处理 scalar 字段（set_scalar），计划未明确提及 |
| `JiebaDict::load/version/sha256_prefix` | 05 | 07/08 | ✅ 一致 |
| `build_jieba_tokenizer(dict, user_dict)` | 05 | 06/07 | ✅ 一致 |
| `DictState`/`ReindexHandle`/`set_user_dict`/`reindex` | 06 | 09(FFI)/07(Node) | ✅ 一致 |
| `vane_load_dict`/`vane_dict_version` C ABI | 09 | 08 | ✅ 08 Consumes 09 的 C ABI |

**签名错配**：仅 M-2（MergeTask::new 缺 tokenizer 参数）。其余跨计划契约一致。

---

## 6. 不变量 I-1~I-8 覆盖

| 不变量 | M1 负责计划 | README 矩阵 | 测试覆盖 | 结论 |
|---|---|---|---|---|
| I-1 段不可变 | 01/02 | ✅ | 01 Task 6 字节稳定；02 合并=新段+manifest | ✅ |
| I-2 双索引原子可见 | 02 | ✅ | 02 合并后向量+倒排+图同快照 | ✅ |
| I-3 图不原地删 | 01/02 | ✅ | 01 Task 6；02 Task 7 delete 不动 hnsw.bin | ✅ |
| I-4 单一分词身份 | 06 | ✅ | 06 Task 1/6 PendingReindex 旧身份 + 原子切换 | ✅ |
| I-5 核心零平台分支 | 10 | ✅ | 10 CI 门禁；05 feature cfg 非 target cfg | ⚠️ 01 的 thread::scope cfg 须删除（M-1） |
| I-6 manifest 原子性 | 04 | ✅ | 04 Task 4/6 崩溃恢复 + 孤儿段清理 | ✅（但 B-2 truncate 缺陷影响 I-6 的 tombstone 持久化） |
| I-7 FFI 内存铁律 | 09 | ✅ | 09 Task 1 close 两次返回错误；arena free | ✅ |
| I-8 binding 薄壳 | 09/07 | ✅ | 09 Go 测试仅调用链；行为在 core | ✅ |

**结论**：不变量矩阵覆盖完整。I-5 受 M-1 影响（01 cfg 表述矛盾），I-6 受 B-2 影响（WAL truncate 丢 tombstone）。修正后全覆盖。

---

## 7. 范围合规

### Won't-have 触碰检查

| Won't-have 项 | 触碰 |
|---|---|
| 内置 embedding 生成 | ❌ 无 |
| GPU 加速 | ❌ 无 |
| 分布式/副本/服务端 | ❌ 无 |
| SQL 接口 | ❌ 无 |
| 多用户/权限/网络协议 | ❌ 无 |

✅ 无 Won't-have 触碰。

### M1 范围越界检查

| 超范围项 | 状态 |
|---|---|
| export 快照（M2） | ✅ R-1 已批准保留占位 |
| 浏览器交付（M2） | ❌ 无 |
| SQ8（M2） | ❌ 无 |
| 词典 CDN fetch（M2） | ❌ 无 |
| SIMD 双变体（M2） | ❌ 无 |
| 100 万规模（M2） | ❌ 无 |

✅ 无越界（export 占位除外，已批准）。

### 依赖黑名单合规

| 黑名单依赖 | 引入 |
|---|---|
| regex / tokio / prost / tonic / openssl / lindera / ndarray / wee_alloc | ❌ 无 |
| dashmap / parking_lot | ❌ 无（09 用 std::sync::RwLock） |
| rayon | ❌ 无（R-4 不引入） |

新增依赖：`ruzstd`（05，非黑名单，纯 Rust wasm32 安全）、`jieba-rs`（05 dev-dependency，仅测试）。⚠️ 须验证 jieba-rs 传递依赖不含黑名单 crate。

✅ 黑名单合规。

---

## 8. 需编排者裁决的疑点

| # | 疑点 | 建议 |
|---|---|---|
| Q-1 | **B-2 WAL truncate 修复方案选择**：选择性重写（保留 AddTombstone）vs flush 不 truncate（仅 compact truncate） | 倾向 flush 不 truncate：实现简单，WAL 累积 AddSegment 记录体积可忽略（ULID 字符串），compact 时一次性清空。须 04 计划修订 |
| Q-2 | **M-2 MergeTask::new 签名补 tokenizer 参数**：README 契约须更新为 `new(sources, target_docid_base, tokenizer_id, schema, tokenizer: Box<dyn Tokenizer>)` 或拆为 `with_tokenizer` 方法 | 须 02 计划修订 README 契约 |
| Q-3 | **SPEC §9.1 "DashMap" 与黑名单矛盾**：SPEC 原文「全局注册表 DashMap<u64, Arc<…>>」，但黑名单含 dashmap。M0/M1 正确用 std::sync | 建议 SPEC 修订 §9.1 为「全局注册表 `std::sync::RwLock<HashMap<u64, Arc<…>>>`」 |
| Q-4 | **FFI 函数扩展超出 §9.2**：`vane_reindex_progress`/`vane_reindex_wait`（ReindexHandle 必需，§9.2 遗漏）+ `vane_load_dict`/`vane_dict_version`（M1 词典分发扩展） | 建议 SPEC §9.2 补列 reindex_progress/wait；load_dict/version 作 M1 扩展注明 |
| Q-5 | **corpus 兼容：M0 corpus 无 hnsw.bin**：M1 打开 M0 corpus 时 HnswReader::open 缺失文件的行为未定义 | 建议 01 计划明确：HnswReader::open 缺失 hnsw.bin 时返回 Err，api 层 catch 后 fallback brute_search（与 01 Task 5「hnsw_readers 无该段则 brute」一致） |
| Q-6 | **06 Rebuilding 期写路径 E_BUSY 比 SPEC 更严格**：SPEC §7.4 未禁止 Rebuilding 期写入（仅说查询命中旧段） | 可接受（保守），但建议在计划中注明「M1 选择 Rebuilding 期 E_BUSY，SPEC 允许未来放宽为旧身份写入」 |
| Q-7 | **02 MergeTask 重建段时 scalar 字段处理**：MergeTask 重建段须调 `set_scalar` 重写标量数据，02 计划未明确 | 须 02 计划 Task 3 补 scalar 重写步骤 |
| Q-8 | **§13.2-2 ② nDCG 维基 fixture 来源**：10-ci-m1 提及但无具体生成方案 | 须 10 计划明确 fixture 生成脚本与 CI 执行方式 |

---

## 附录：审查方法说明

- M0 代码核查：逐文件 Read 了 `crates/vane-core/src/` 下 types.rs / vfs/{mod,memory,std_fs,page_cache}.rs / tokenizer/{mod,id,standard,cjk_bigram}.rs / segment/{mod,header,ulid}.rs / bm25.rs / vector/mod.rs / persistence/mod.rs / api/{types,db,collection,mod}.rs / fusion/mod.rs / lib.rs + Cargo.toml + deny.toml + crates/vane-ffi/src/lib.rs + crates/vane-node/src/{collection,convert}.rs。全部签名经 SubAgent 独立核查。
- SPEC 条款引用：以 `docs/SPEC.md` v1.0 节号为准。
- R-3 独立核查：比对了 SPEC §5.4（TokenizerId 公式）、REQUIREMENTS §3.3（词典升级不强制重建）、§7.4/I-4（禁多版本）、M0 代码 id.rs 注释与实现。
