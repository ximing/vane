# M4 阶段五 b — inspect API Task Review

> Reviewer：task reviewer SubAgent（opus，只读，禁编辑源码）
> 审查对象：commit 684a112 `feat(core): inspect API（Db::stats/segment_info + 健康检查）（M4 阶段五 b）`
> 输入：phase0-design.md §3.6 / task-inspect-report.md / task-inspect-review-package.md
> 审查日期：2026-08-12

## A. Spec 合规

**✅ 合规。**

### 3 新 pub 方法签名

| 方法 | §3.6 spec 签名 | 实现（db.rs:189-239） | 匹配 |
|---|---|---|---|
| `stats` | `pub fn stats(&self) -> DbStats` | `pub fn stats(&self) -> super::inspect::DbStats` | ✅（经 `pub use inspect::*` 重导出为 `Db::stats -> DbStats`） |
| `segment_info` | `pub fn segment_info(&self) -> Vec<SegmentInfo>` | `pub fn segment_info(&self) -> Vec<super::inspect::SegmentInfo>` | ✅ |
| `collection_segment_info` | `pub fn collection_segment_info(&self, name: &str) -> Option<Vec<SegmentInfo>>` | 同 | ✅ |

### 7 structs/enums 字段 + derive

| struct/enum | §3.6 字段 | 实现（inspect.rs:29-106） | derive | 匹配 |
|---|---|---|---|---|
| `DbStats` | db_path / collections / dict_available / executor_kind | 同 | Debug, Clone | ✅ |
| `CollectionStats` | name / segment_count / total_docs / live_docs / tombstoned_docs / index_bytes / dict_state / tokenizer_id / health | 同 | Debug, Clone | ✅ |
| `SegmentInfo` | ulid / doc_count / docid_base / tombstoned_count / format_versions / file_sizes / health | 同 | Debug, Clone | ✅ |
| `FormatVersions` | header / vectors / stored / idmap / scalars / inverted / hnsw | 同 | Debug, Clone | ✅ |
| `SegmentFileSizes` | header / vectors / stored / idmap / scalars / inverted / hnsw(Option) | 同 | Debug, Clone | ✅ |
| `Health` | Healthy / Degraded / Corrupt | 同 | Debug, Clone, Copy, PartialEq, Eq | ✅ |
| `ExecutorKind` | Serial / Rayon | 同 | Debug, Clone, Copy, PartialEq, Eq | ✅ |

字段类型核对：
- `SegmentInfo.tombstoned_count: u64` ← `roaring::RoaringBitmap::len()` 返 u64 ✅
- `SegmentInfo.doc_count: u32` / `docid_base: u64` ← `SegmentMeta` 字段类型匹配（segment/mod.rs:13-15） ✅
- `CollectionStats.dict_state: DictState` ← `api::types::DictState`（Copy, 已存在） ✅
- `CollectionStats.tokenizer_id: TokenizerId` ← `types::TokenizerId`（已存在） ✅

### 模块声明

`api/mod.rs:13-14`：`pub mod inspect; pub use inspect::*;` — 匹配 §3.6 ✅。无 name conflict（grep 确认 DbStats/CollectionStats/SegmentInfo/FormatVersions/SegmentFileSizes/Health/ExecutorKind 在 inspect.rs 外无同名 pub 项）。

### 健康检查（§3.6 表）

| 健康标志 | §3.6 表 spec | 实现（inspect.rs） | 匹配 |
|---|---|---|---|
| 词典降级 | collection tokenizer=Jieba 且 `DbInner.jieba_dict` None → Degraded | `build_collection_stats` line 144-158：`if matches!(col_inner.tokenizer_kind, BuiltinTokenizer::Jieba) && !dict_available { worst(Degraded) }` | ✅ |
| 段损坏 | `SegmentReader::open` 失败 → Corrupt | `segment_health` line 235-250：`match SegmentReader::open(vfs, seg_dir) { Ok(_) => ..., Err(_) => Health::Corrupt }` | ✅ |
| hnsw 缺失 | `hnsw_readers[i]` None → Degraded | `segment_health` line 238-243：`Some(None) => true / None => true → Degraded` | ✅ |
| dict_state | `CollectionInner.dict_state` | `build_collection_stats` line 120：`*col_inner.dict_state.read().unwrap()` | ✅ |
| executor_kind | platform cfg 推断 | `executor_kind()` line 209-218：`cfg!(all(not(target_arch="wasm32"), feature="executor-native"))` | ✅ |
| collection health | worst of segments + 词典降级 | `worst_health = worst(worst_health, seg_health)` 逐段，再 `worst(worst_health, Degraded)` for 词典 | ✅ |

### index_bytes：read_at 探测 EOF（Vfs trait 不改）

- `probe_file_size`（inspect.rs:295-311）：offset=0 起循环读 8KB buffer，n=0 即 EOF，累计推算 size。文件不存在（read_at 返 Err）→ 返 0。
- `SegmentFileSizes`：header/vectors/stored/idmap/scalars/inverted 用 u64（缺失→0），hnsw 用 `Option<u64>`（None=无 hnsw.bin）。
- `index_bytes` = 各段 `file_sizes.total()` 之和（含 hnsw.unwrap_or(0)）。
- Vfs trait 未改（无 size() 方法添加） ✅

## B. 代码质量

### 新 pub API 正确性——✅ 正确

**stats()（db.rs:189-211）**：
- 读 `collections` RwLock → 遍历 `HashMap<String, Arc<CollectionInner>>` → 调 `build_collection_stats(name, col_inner, vfs, dict_available)` → 按 name 排序保证确定性 → 构造 `DbStats`。正确。

**build_collection_stats()（inspect.rs:111-171）**：
- 读 `snapshot`（段快照 Vec<Arc<SegmentReader>>），逐段读 `reader.meta()` 取 `doc_count`/`ulid`。
- `total_docs` = 各段 `meta.doc_count` 之和。正确（含 tombstoned，spec 要求）。
- `tombstoned_docs` = `tombstones.get(&ulid).map(|b| b.len())` 之和。正确（按段 ULID 匹配 tombstone 位图）。
- `live_docs` = `total_docs - tombstoned_docs`。正确（u64 减法；tombstoned ≤ total_docs 因 tombstone 仅记当前快照内段）。
- `index_bytes` = 各段 `probe_segment_file_sizes().total()` 之和。正确。
- `segment_count` = `snap.len()`。正确。
- `dict_state` = `*col_inner.dict_state.read()`。正确。
- `tokenizer_id` = `col_inner.tokenizer_id.read().clone()`。正确。
- `health` = worst of 各段 health，再 worst 词典降级。正确。

**build_segment_info()（inspect.rs:174-206）**：
- 遍历 `snapshot`，逐段读 `meta.ulid/doc_count/docid_base`，`tombstones.get(&ulid).len()` 取 `tombstoned_count`。
- `file_sizes` = `probe_segment_file_sizes`，`format_versions` = `read_format_versions`，`health` = `segment_health`。正确。

**collection_segment_info()（db.rs:232-239）**：
- `collections.get(name)?` → None 返 None，Some 调 `build_segment_info`。正确。

### pub(crate) 可见性变更——✅ acceptable，非冻结 API 改动

collection.rs 3 字段 private→pub(crate)：
- `snapshot: RwLock<Vec<Arc<SegmentReader>>>`
- `hnsw_readers: RwLock<Vec<Option<Arc<HnswReader>>>>`
- `dict_state: RwLock<DictState>`

**判定依据**：
1. `CollectionInner` 本身是 `pub(crate) struct`（非 pub），外部 crate 无法访问此类型。
2. 字段从 private → pub(crate) 只影响 crate 内部可见性（inspect 模块可读），不暴露到 crate 外。
3. 不改任何 pub fn/struct 签名，不改 pub trait 方法。
4. pub(crate) 不是 pub API surface 的一部分——它仅是 crate 内部的模块间通信机制。

**结论**：pub(crate) widening 是内部可见性调整，**不算冻结 pub API 改动**。acceptable。

### 不改冻结 pub API——✅ 确认

diff 验证（`git diff 3758620..684a112 -- collection.rs db.rs mod.rs`）：
- **collection.rs**：仅 3 行 visibility 改动（private→pub(crate)），无类型/签名变更。
- **db.rs**：仅新增（+3 pub 方法 +1 private helper `dict_available_internal`），无现有 pub fn 改动。
- **mod.rs**：仅新增 2 行（`pub mod inspect; pub use inspect::*;`），无现有 re-export 改动。
- **inspect.rs**：新文件，纯新增。
- **DbInner**：无改动（字段已 pub(crate)）。
- **Vfs trait**：未改（无 size()）。
- **SPEC.md / CI yml**：未碰。

确认：无现有 pub fn/struct/trait 签名被改。纯新增。

### 健康检查逻辑——✅ 正确

**SegmentReader::open 失败→Corrupt**（inspect.rs:235-250）：
- `SegmentReader::open`（segment/mod.rs:370-448）读 header.bin（`decode_header` 校验 magic+version）、idmap.bin（`decode_kv_map` 校验 magic+version）、vectors.bin 头探测（校验 magic + version∈{1,2}）、stored.bin 头探测（校验 magic + version∈{1,2}）。任一失败返 `Err`。
- `segment_health` 匹配 `Err(_) => Health::Corrupt`。正确——覆盖 magic/version/decode 校验失败。

**hnsw_readers[i] None→Degraded**（inspect.rs:238-243）：
- `hnsw_readers.get(i)` 返 `Option<&Option<Arc<HnswReader>>>`。
- `Some(Some(_))` → hnsw 存在 → false（Healthy）。
- `Some(None)` → hnsw 缺失 → true（Degraded）。
- `None` → 索引越界（snapshot 比 hnsw_readers 长）→ true（Degraded，防御性）。
- 正确。

**Jieba+dict 不可用→Degraded**（inspect.rs:144-158）：
- `if matches!(col_inner.tokenizer_kind, BuiltinTokenizer::Jieba) && !dict_available { worst(Degraded) }`。
- `BuiltinTokenizer::Jieba` 变体在 jieba feature off 时仍存在（非 cfg 门控，tokenizer/mod.rs:33），编译无问题。
- jieba feature off 时 `dict_available` 恒 false（`dict_available_internal` 返 false），但 Jieba collection 在 feature off 时无法创建（`build_tokenizer` 返 DictUnavailable）。条件等价但防御性保留。正确。

**collection = worst of segments + 词典降级**：
- 逐段 `worst_health = worst(worst_health, seg_health)`，最后与词典降级 `worst(worst_health, Degraded)`。正确——Corrupt > Degraded > Healthy（`worst` 函数 inspect.rs:342-348 验证）。

### Vfs size 方案——✅ 正确

**probe_file_size（inspect.rs:295-311）**：
- 8KB buffer 循环读至 n=0（EOF）。文件不存在（read_at Err）→ 返 0（total 仍 0）。
- 不 panic on empty/missing 文件 ✅
- 性能：大文件（154MB vectors.bin）需 ~19k 次 read_at 调用——inspect 非热路径，可接受（§3.6 取舍）。
- SegmentFileSizes.hnsw=Option<u64> 区分无文件（None）vs 存在（Some）：`if sz == 0 { None } else { Some(sz) }`。基本正确——但 0 字节文件与缺失文件都映射 None（见 Minor 4）。

### 20 单测 non-vacuous——✅ 全部 non-vacuous

逐测核对：

| # | 测试 | 断言具体性 | 判定 |
|---|---|---|---|
| 1 | stats_returns_db_path_and_executor | `db_path == "testdb"`（具体）；`executor_kind matches Serial\|Rayon`（对 2 变体 enum 略弱，但 test 19 充分覆盖） | non-vacuous |
| 2 | stats_returns_one_collection_with_correct_counts | `collections.len()==1` / `name=="docs"` / `segment_count==1` / `total_docs==2` / `live_docs==2` / `tombstoned_docs==0` / `dict_state==Stable` | non-vacuous |
| 3 | stats_index_bytes_nonzero | `index_bytes > 0`（具体值断言） | non-vacuous |
| 4 | stats_health_healthy_for_fresh_segment | `health == Health::Healthy`（精确枚举匹配） | non-vacuous |
| 5 | segment_info_returns_one_segment_with_correct_fields | `len()==1` / `ulid 非空` / `doc_count==2` / `docid_base==0` / `tombstoned_count==0` | non-vacuous |
| 6 | segment_info_format_versions_correct | `header==HEADER_FORMAT_V1` / `vectors==VECTORS_FORMAT_V2` / `stored ∈ {V1,V2}` / `idmap==IDMAP_FORMAT_V1` / `scalars==SCALARS_FORMAT_V1` / `inverted==FORMAT_VERSION` / `hnsw==HNSW_FORMAT_V1`（与写入常量逐字比对） | non-vacuous |
| 7 | segment_info_file_sizes_nonzero | `header>0` / `vectors>0` / `stored>0` / `idmap>0` / `inverted>0` / `hnsw.is_some()`（具体大小断言） | non-vacuous |
| 8 | segment_info_health_healthy | `health == Health::Healthy` | non-vacuous |
| 9 | collection_segment_info_returns_some_for_existing | `is_some()` + `len()==1` | non-vacuous |
| 10 | collection_segment_info_returns_none_for_missing | `is_none()`（负面测试，最小但非 vacuous） | non-vacuous |
| 11 | stats_multiple_collections | `len()==2` / 排序 `col1<col2` / `segment_count==0` both | non-vacuous |
| 12 | stats_empty_db | `db_path=="emptydb"` / `collections.is_empty()` / `dict_available` 与 `jieba_dict_available()` 一致 | non-vacuous |
| 13 | segment_info_empty_collection | `infos.is_empty()`（未 flush 无段） | non-vacuous |
| 14 | probe_file_size_empty_file_returns_zero | `probe_file_size(nonexistent) == 0` | non-vacuous |
| 15 | probe_file_size_correct_for_known_content | 写 100 字节 → `probe_file_size == 100` | non-vacuous |
| 16 | read_version_field_missing_file_returns_zero | `read_version_field(nonexistent) == 0` | non-vacuous |
| 17 | read_version_field_correct_for_vane_magic | 写 MAGIC+V2 → `read_version_field == VECTORS_FORMAT_V2` | non-vacuous |
| 18 | worst_function_ordering | 6 组 `worst(a,b)` 覆盖全排列 | non-vacuous |
| 19 | executor_kind_consistent_with_cfg | `kind == Rayon` when cfg matches, else `Serial` | non-vacuous |
| 20 | stats_after_tombstone_delete | `total_docs==2` / `tombstoned_docs==1` / `live_docs==1` / `total_tombstoned==1` | non-vacuous |

**结论**：20 测全部断言具体字段值（段数、文档数、health 枚举、format_version 常量、file_size>0、tombstone 计数），无仅 `is_some()`/`is_ok()` 的空测。test 1 的 `executor_kind matches` 略弱但 test 19 充分覆盖。无 Critical vacuous 测。

### inspect 重新 open 段——✅ acceptable

`segment_health`（inspect.rs:235）调 `SegmentReader::open(vfs, seg_dir)` 重新打开段做健康检查。

**判定**：acceptable（遵循 spec 表）。
- §3.6 **表 spec**（normative）要求"SegmentReader::open 失败 → Corrupt"——必须 open 才能判 Corrupt。
- §3.6 **取舍建议**（non-normative）说"不主动重新 open 校验（性能）"——是建议，非要求。
- 实现遵循 normative 表 spec。inspect 非热路径，重新 open 可接受。
- 重新 open 能检测段在初始 open 后的损坏（文件被外部篡改）——比"仅信任 snapshot 中已有的 reader"更真实。
- `SegmentReader::open` 是轻量操作（M2-07 懒加载：仅读 header+idmap+头探测，不读 payload）。

## C. 已知 concerns

### C1. inspect 重新 open 段（见 B）——acceptable
性能取舍。inspect 非热路径。遵循 normative spec 表。不改。

### C2. report 在 code commit——acceptable
`task-inspect-report.md` 与 code 同 commit（684a112）。minor scope 偏差，5a 同模式。acceptable。

## Findings

### Critical
无。

### Important
无。

### Minor

| # | file:line | 一句话 | 失败场景 |
|---|---|---|---|
| M1 | db.rs:217-227 | `segment_info()` 不按 collection name 排序，跨 collection 输出顺序非确定（HashMap 迭代）；`stats()` 排序了，`segment_info()` 没有——不一致 | FFI `vane_db_segment_info` JSON 输出顺序非确定，可能导致 FFI 层字符串比较 flaky |
| M2 | inspect.rs:230-251 | "段文件部分缺失但可读 → Degraded"（§3.6 Health::Degraded 注释）未实现；仅检查 open 失败（Corrupt）和 hnsw 缺失（Degraded），inverted.bin/scalars.col 缺失但 open 成功时返 Healthy | 段 inverted.bin 被删但 header.bin 完好时，health=Healthy 而非 Degraded（spec 注释暗示应 Degraded）；实际 SegmentWriter::finalize 原子写所有文件，此场景罕见 |
| M3 | inspect.rs tests | 无 Corrupt/Degraded health 路径测试——20 测全测 Healthy；`segment_health` 的 Err→Corrupt 和 hnsw None→Degraded 分支未覆盖 | 段损坏或 hnsw 缺失时 health 值未经测试验证（逻辑正确但无回归守护） |
| M4 | inspect.rs:261-269 | `SegmentFileSizes.hnsw`：`sz==0 → None` 将"文件不存在"与"文件存在但 0 字节"都映射 None；spec 说 Option 区分"无文件 vs 存在" | 0 字节 hnsw.bin（无效/损坏）被报为 None（无 hnsw.bin）而非 Some(0)；实践中 hnsw.bin 至少有 8 字节头，此场景不发生 |
| M5 | inspect.rs:117-141 | `build_collection_stats` 同时持 5 个 read lock（snapshot/hnsw_readers/tombstones/dict_state/tokenizer_id）然后做 VFS I/O（probe_segment_file_sizes + segment_health→open），阻塞写者 | StdFsVfs 下 inspect 期间写路径等待；非热路径可接受，与现有 search 持锁模式一致 |
| M6 | db.rs:250-259 | `dict_available_internal` 与 `jieba_dict_available` 逻辑重复（均读 `inner.jieba_dict.read().is_some()`）；前者加 cfg(not(jieba))→false 分支 | 维护时两处需同步修改；可重构为一个 cfg-gated 方法 |
| M7 | inspect.rs:144-158 | `#[cfg(feature="jieba")]` 和 `#[cfg(not(feature="jieba"))]` 两个块逻辑完全相同（Jieba+!dict_available→Degraded），可合并为一个无 cfg 块 | 无功能影响；冗余 cfg 分支，`BuiltinTokenizer::Jieba` 非 cfg 门控 |

## 新 pub API 正确性定性

**✅ 正确。** 三个 pub 方法（stats/segment_info/collection_segment_info）正确遍历 collections/segments，读内部状态（SegmentMeta.doc_count/ulid/docid_base、tombstones 位图、hnsw_readers、dict_state、tokenizer_id），构造返回 structs。字段值正确：segment_count=snap.len()、total_docs=各段 doc_count 之和、live_docs=total-tombstoned、index_bytes=各段 file_sizes.total() 之和、health=worst of segments + 词典降级。probe_file_size 和 read_version_field 内部函数经独立单测验证（test 14-17）。

## pub(crate) 可见性变更定性

**✅ acceptable，非冻结 API 改动。** CollectionInner 是 `pub(crate) struct`（非 pub），其字段从 private→pub(crate) 仅影响 crate 内部可见性，不暴露到 crate 外。pub(crate) 不属于 pub API surface。这不是冻结 API 改动。

## 不改冻结 pub API 定性

**✅ 确认。** diff 验证：3 个新 pub 方法 + 7 个新 pub struct/enum 均为纯新增；collection.rs 仅 visibility 改动（private→pub(crate)）；db.rs 仅新增方法；mod.rs 仅新增 mod 声明；DbInner/Vfs trait/SPEC.md/CI yml 未碰。无现有 pub fn/struct/trait 签名被改。

## 健康检查逻辑定性

**✅ 正确。** Corrupt（SegmentReader::open 失败，覆盖 header magic/version/idmap decode/vectors+stored 头探测失败）、Degraded（hnsw_readers[i] None 或 Jieba+dict 不可用）、Healthy（open 成功且 hnsw 存在且无词典降级）三档正确。collection 级 health = worst of segments 再 worst 词典降级。`worst` 函数经 test 18 全排列验证。

## Vfs size 方案定性

**✅ 正确。** read_at EOF 探测（8KB buffer 循环至 n=0）不改 Vfs trait（M0 冻结）。不 panic on missing/empty 文件（Err→0）。性能：大文件需多次 read_at，但 inspect 非热路径，可接受。SegmentFileSizes.hnsw=Option<u64> 基本区分无文件 vs 存在（M4：0 字节文件与缺失文件混淆，实践不影响）。

## 20 单测 non-vacuous 定性

**✅ 全部 non-vacuous。** 20 测均断言具体字段值（段数、文档数、health 枚举值、format_version 与写入常量逐字比对、file_size > 0、tombstone 计数），非仅 is_some()/is_ok()。test 1 的 executor_kind matches 略弱（2 变体 enum 的 matches 总真），但 test 19 充分覆盖。无 vacuous 测，无 Critical。

## inspect 重新 open 段定性

**✅ acceptable（遵循 spec 表）。** §3.6 表 spec 是 normative 要求（"open 失败→Corrupt"须 open），§3.6 取舍建议是 non-normative。inspect 非热路径。SegmentReader::open 是轻量操作（M2-07 懒加载）。重新 open 能检测初始 open 后的损坏。遵循 normative spec，不需优化。

## ⚠️ 无法从 diff 验证项

1. **测试执行结果**：20 测通过、342 workspace 测通过、clippy -D warnings 通过、fmt 通过、wasm32 check 无 warning、deny ok——均来自 report 自报，reviewer 未独立重跑（task 要求"不重跑 implementer 已跑门禁"）。
2. **SegmentReader::open 在损坏段上的实际行为**：reviewer 通过阅读 segment/mod.rs:370-448 代码推断 open 失败路径（magic/version/decode 校验），未实际注入损坏段验证。
3. **scalars.col / inverted.bin 是否有 VANE magic 头**：test 6 断言 `format_versions.scalars == SCALARS_FORMAT_V1` / `inverted == FORMAT_VERSION`，若 test 通过则说明有 magic 头；reviewer 未独立 grep 写入路径验证（推断自 test 结果）。
4. **StdFsVfs 下 probe_file_size 的实际 syscall 次数**：性能推断基于代码阅读（8KB/次循环），未实测大文件延迟。
5. **wasm32 体积增量**：report 称 wasm32 check 无 warning，但未给体积数字对比；inspect.rs 纯 Rust 无新依赖，体积增量应可忽略，但未独立验证。

## 总体

**不进 fix 循环。**

实现 spec 合规、pub API 正确、不改冻结 API、健康检查逻辑正确、Vfs size 方案合理、20 测 non-vacuous。7 个 Minor findings 均为改进建议或覆盖缺口，非阻断项：

- M1（segment_info 不排序）：FFI 层可排序，不影响 core 正确性。
- M2（部分缺失→Degraded 未实现）：spec 注释模糊，SegmentWriter 原子写使场景罕见。
- M3（Corrupt/Degraded 路径无测试）：逻辑正确，缺回归守护——建议后续补测。
- M4-M7：代码风格/冗余/锁粒度，非正确性问题。

**建议**：可合入。M1/M3 建议后续小补丁跟进（segment_info 排序 + Corrupt/Degraded health 回归测）。M2/M4/M5/M6/M7 可列为 backlog 改进项。
