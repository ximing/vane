# M2 Scoping 报告 — SPEC v1.2 修订提案 + Phase Zero 安全清理 + 模块分解预览

> 产出：M2 只读架构 scoping SubAgent（2026-08-09）
> 起点：HEAD `6548a3`（M1 完成，main 干净，340 测试绿），SPEC v1.1
> 约束：全程只读；每条结论附 `file:line` 或 `SPEC §节号`；不臆测。

---

## 第一部分：SPEC v1.2 修订提案（精确文本）

### 修订 A — 冷启动懒加载

#### A.0 事实认定与冻结边界判定

**核心结论：懒加载不触及 §4 冻结的对外 IDL 签名。**

- SPEC §4（lines 69-131）冻结的是语言无关 IDL 的 6 动词 + 4 管理函数及其参数结构（`open/Db.collection/.../Collection.search/...`、`OpenOptions/SearchQuery/Hit/Filter/...`）。`SegmentReader` 不在 §4 IDL 清单内，是 `crates/vane-core` 内部读期句柄。
- M1-SUMMARY §3.2（`docs/plans/m1/M1-SUMMARY.md:73`）所述"属签名变更，须 SPEC 修订"系指 `SegmentReader::open` 行为变更（从全加载改为懒加载）触发 §13.1 冷启动承诺的实测口径变更——**不是 §4 IDL 签名变更**。本报告据此将修订定位为「core 内部行为 + §13.1 承诺 + §6.2 语义注释」，属轻量修订。

#### A.1 当前全加载路径（代码定位）

`crates/vane-core/src/segment/mod.rs:340` `SegmentReader::open` 当前一次性加载全部数据：

| 加载项 | 代码位置 | 体积量级（10万×384维） |
|---|---|---|
| header.bin | `mod.rs:343-344` `read_all` + `decode_header` | ~KB |
| vectors.bin | `mod.rs:348-368` 全量 `read_all` + `chunks_exact(4)` collect | **~154MB**（冷启动主因，见 cold-start 报告 `11-cold-start-bench-report.md:46`） |
| idmap.bin | `mod.rs:376` `load_id_map` → `mod.rs:401-408` | ~MB |
| stored.bin | `mod.rs:379` `load_stored` → `mod.rs:392-399` | ~MB |
| scalars.col | 由 `ScalarReader::open`（`mod.rs:585`）独立加载，不在 `SegmentReader::open` | ~MB |
| hnsw.bin | 由 `HnswReader::open`（`hnsw/mod.rs:523`）独立加载，不在 `SegmentReader::open` | ~60MB |
| inverted.bin | 由 `InvertedIndexReader` 独立加载 | ~MB |

- `dim` 当前从 vectors 长度反推：`mod.rs:369-373` `(vectors.len() / doc_count) as u32`。懒加载下 vectors 不在 open 时加载，dim 须改为从 header 或 vectors.bin 头部读取。
- `vectors()` 访问器：`mod.rs:413` `pub fn vectors(&self) -> &[f32]`，`&self` 返回 `&[f32]`。

#### A.2 vectors() 消费方（懒加载影响面）

| 消费方 | 位置 | 调用形态 | 懒加载兼容性 |
|---|---|---|---|
| HNSW 搜索 | `api/collection.rs:766` `hr.search(qv, want, ef, merged_filter, base, reader.vectors())` | 借 `&[f32]` 导航 | 首次 search 触发 load；需 `OnceLock` 保 `&self` 签名 |
| 暴力回退 | `api/collection.rs:773,781` `brute_search(reader.vectors(), reader.dim(), ...)` | 同上 | 同上 |
| reindex 重建 | `api/reindex.rs:142` `reader.vectors()[(local*dim)..]` | 切片借 | reindex 本需全量，首次访问触发 load |
| merge 段合并 | `merge/mod.rs:199` `reader.vectors()[(local*dim)..]` | 切片借 | 同上 |
| flush 新段 HNSW 构建 | `api/collection.rs:384` `reader.vectors()` | 新段刚写，内存已有 | 无影响（新段不走懒加载路径，或走也立即命中） |
| reindex 新段 HNSW 构建 | `api/reindex.rs:178` `new_reader_tmp.vectors()` | 同上 | 无影响 |

**结论：所有消费方均借 `&[f32]` 只读，无写入。懒加载用 `std::sync::OnceLock<Vec<f32>>`（或 `Mutex<Option<Vec<f32>>>`）即可保持 `vectors(&self) -> &[f32]` 签名不变，消费方零改动。**

#### A.3 懒加载设计提案

1. **open 阶段（冷启动）只加载元数据**：header.bin + idmap.bin（外部 id 反查必需）+ manifest。vectors / stored / hnsw 延后。
2. **dim 来源**：在 `header.bin` 增加 `dim: u32 LE` 字段（header.bin 当前布局见 `segment/header.rs:4-7`，不含 dim）。**这是 header.bin 格式变更，需 per-file format_version bump（见修订 B 的 per-file 机制；header.bin v1→v2 加 dim 字段，旧 v1 header 缺 dim 则回退从 vectors.bin 头读）**。或：不改 header，改从 vectors.bin 头部读 dim（vectors.bin v1 头仅 magic+version 8 字节，v2 可加 dim 4 字节）。**推荐后者**：dim 本就属于 vectors.bin 语义，header 不必冗余。
3. **vectors 懒加载**：`SegmentReader.vectors` 字段改为 `OnceLock<Vec<f32>>`；`vectors()` 首次调用时 `read_all` + decode 填充，后续直接返回 `&[f32]`。需 `&self` 内部可变 → `OnceLock` 满足（`OnceLock::get_or_init`）。
4. **stored 懒加载**：`stored` 字段改为 `OnceLock<HashMap<...>>`；`stored_json()` / `text()` 首次调用触发 `load_stored`。
5. **hnsw 懒加载**：`HnswReader::open`（`hnsw/mod.rs:523`）当前全加载 nodes。HNSW 搜索需全图 nodes，懒加载收益有限（hnsw.bin ~60MB），但可推迟到首次 vector search。由 `api/collection.rs:403,575` 的 `HnswReader::open` 调用点改为延后到首次 search（或保持 open 时加载——因 search 必然紧随，收益小）。**建议 M2 首期只懒加载 vectors+stored，hnsw 维持 open 时加载；若 <1s 仍不达再懒加载 hnsw。**

#### A.4 §13.1 冷启动承诺修订文本

**SPEC §13.1 原文本（lines 409）：**
```
| 冷启动（打开 10 万库） | < 1s（M1 实测背书；>2s 则降级为分级指标：元数据 <1s、首次查询 <3s） |
```

**SPEC §13.1 新文本：**
```
| 冷启动（打开 10 万库） | 元数据 open < 1s（vectors/stored 懒加载，M2 实测背书）；首次向量查询触发 vectors 加载，<3s（降级分级保留为 fallback） |
```

**修订理由**：M1 实测 open 1573ms（`11-cold-start-bench-report.md:46`），全加载模型下元数据与 vectors 加载耦合，"<1s"不可达，走降级分级。M2 懒加载将 open 仅读 header+idmap+manifest（MB 级），<1s 可达；首次查询触发 vectors 加载（154MB），仍承诺 <3s（与降级档一致）。降级分级保留为 fallback：若懒加载后元数据 open 仍 >1s（极多段/极慢 IO），退回降级口径。

#### A.5 §6.2 目录布局语义注释修订文本

**SPEC §6.2 原文本（lines 198-212）** 目录布局表后无懒加载语义说明。在 `stored.bin` 行（line 211）后追加注释行。

**SPEC §6.2 新文本（在 line 212 `stored.bin` 行后、line 213 空行前插入）：**
```
    // 懒加载语义（M2）：SegmentReader::open 仅读 header.bin + idmap.bin + manifest；
    // vectors.bin / stored.bin / hnsw.bin 首次访问时按需加载（OnceLock，core 内部，
    // 不改 §4 IDL 签名）。冷启动承诺见 §13.1。
```

**修订理由**：明确懒加载边界，避免实现者误以为 open 须全加载。注释行不改变布局，仅语义说明。

#### A.6 对 hnsw 搜索路径与 reindex/merge 的影响评估

- **HNSW 搜索**（`hnsw/mod.rs:624` `HnswReader::search` 借 `reader.vectors()`，`api/collection.rs:766`）：`OnceLock` 首次 `vectors()` 调用触发 load，后续零成本。`HnswReader` 自身 nodes 仍 open 时加载（A.3-5）。无签名变更。
- **reindex**（`api/reindex.rs:111` open 旧段 → `:142` 读 vectors → `:146` stored_json → `:149` text → `:177-178` 新段 vectors → `:198` 新段 open）：reindex 本需全量原文+向量重建，懒加载不阻碍（首次访问触发 load）。无改动。
- **merge**（`merge/mod.rs:132` open 源段 → `:199` vectors → `:203` stored_json → `:206` text）：同 reindex，无改动。
- **compact 全合并**：同 merge。
- **风险**：若 `OnceLock` 初始化并发触发（多读线程同时首查），`OnceLock::get_or_init` 保证只加载一次（`std::sync` 原语，无新依赖，符合黑名单）。冷启动后首次查询延迟略增（+vectors 加载 ~1s 量级），已在 §13.1 <3s 承诺内。

---

### 修订 B — stored.bin zstd + per-file format_version

#### B.0 SPEC 与实现张力认定

**张力 1（stored.bin zstd）**：
- SPEC §6.2 line 211：`stored.bin       // 原文/JSON meta（zstd 块压缩）`
- 实现 `segment/mod.rs:208-226`（finalize 写 stored.bin）：写裸 JSON，无压缩。注释 `mod.rs:210`：`I10: M0 写裸数据（zstd 块压缩延后 M1，format_version 不变）`。
- M1-SUMMARY §3.2（`M1-SUMMARY.md:75`）：`stored.bin zstd 压缩：M1 保持裸 JSON（避免 core 加 zstd 撑爆 800KB + I-5 禁 cfg 隔离）；M2 评估 per-file format_version + zstd`。
- **张力确认**：SPEC 写明 zstd，实现裸 JSON。M2 须消解。

**张力 2（per-file format_version）**：
- SPEC §6.2 line 214：`所有文件以 4 字节 magic + 4 字节 format_version 开头。格式变更必须：① version 递增；② 提供迁移器或双模读取；③ 冻结 corpus 兼容测试通过（§13.3）。`
- 实现 `types.rs:15`：`pub const FORMAT_VERSION: u32 = 1;` 全局单常量，header/vectors/stored/idmap/scalars/hnsw/inverted 共用。
- corpus_compat.rs:221-280 验证所有文件头 magic+version=1，但共用同一常量——**无法单文件 bump**。要 stored.bin 独立升 v2，须引入 per-file format_version 常量。

#### B.1 per-file format_version 设计

引入每文件独立常量（`types.rs`）：

```rust
// per-file format_version（SPEC §6.2：所有文件 magic+format_version 开头，独立递增）
pub const HEADER_FORMAT_V1: u32 = 1;
pub const VECTORS_FORMAT_V1: u32 = 1;
pub const STORED_FORMAT_V1: u32 = 1;   // 裸 JSON（M0/M1 产物，双模读取保留）
pub const STORED_FORMAT_V2: u32 = 2;   // zstd 块压缩（M2 起）
pub const IDMAP_FORMAT_V1: u32 = 1;
pub const SCALARS_FORMAT_V1: u32 = 1;
pub const HNSW_FORMAT_V1: u32 = 1;
// inverted.bin 由 05-bm25 模块自管，同理 per-file 化
```

保留 `FORMAT_VERSION` 常量作为「全库 schema/语义版本」用于 manifest 层（若需），但段文件头不再共用它。每文件编解码点（`segment/mod.rs:decode_stored:505`、`decode_kv_map:460`、`header.rs:40`、`hnsw/mod.rs:533`、`ScalarReader::decode_scalars:648`）改用对应 per-file 常量校验。

#### B.2 stored.bin v2 zstd 块压缩格式

```
stored.bin v2 布局：
magic(4)="VANE" | format_version(4 LE)=2 | raw_payload_len(4 LE) |
zstd_block_len(4 LE) | zstd_block_bytes...
// zstd_block = zstd-compress(raw_payload)
// raw_payload = v1 布局（count + {docid|text_len|text|meta_len|meta}...）
```

- 编码（finalize，`segment/mod.rs:211-228`）：先构 raw_payload（同 v1），再 zstd 压缩成块，写 v2 头。
- 解码（`decode_stored:505`）：读 version；v1 走原路径 `decode_stored`；v2 读 zstd_block → ruzstd 解压 → 对 raw_payload 走 v1 解码逻辑。

#### B.3 zstd 编解码依赖与 wasm 800KB 门禁评估

**现状**（`crates/vane-core/Cargo.toml`）：
- `ruzstd = { version = "0.5", optional = true }`（纯 Rust，**仅解码**）
- `jieba = ["ruzstd"]`：ruzstd 当前 gated 在 jieba feature 后，wasm32 check 不启用 jieba → ruzstd 不进 wasm 产物。
- `zstd = "0.13"` 仅在 `crates/vane-dict-zh/Cargo.toml` 的 `[dev-dependencies]`（词典生成脚本用，不进任何运行时 crate）。

**编码侧（写期）**：
- `zstd` crate（0.13，zstd-sys C 库）体积大且 C 库 wasm32 编译风险高，**不可进 wasm 核心**。
- 方案：新增 vane-core feature `zstd-encode`，引入 `zstd = "0.13"` 作为 optional dep；native/node 启用，wasm32 check 不启用。**wasm 端 flush 写 stored.bin 时不压缩（落 v1 裸 JSON）**——浏览器端 stored 体积小（5万文档验收边界，REQUIREMENTS §4.1），不压缩可接受。

**解码侧（读期）**：
- v2 stored.bin（native 写）可能被 wasm 端读取（corpus 跨平台迁移场景）。wasm 须支持 v2 解码 → ruzstd（纯 Rust 解码）。
- 方案：将 ruzstd 从 `jieba` feature 解耦，新设 `zstd-decode` feature（或直接默认启用 ruzstd，非 optional）。**wasm32 构建须启用 ruzstd 以支持 v2 解码**。
- **体积评估**：ruzstd 0.5 纯 Rust 解码器，gzip 后估计 +30~60KB。当前核心 wasm 557KB gzip（M1-SUMMARY §2），+ruzstd 约 590~620KB，仍在 800KB 门禁内。**须 M2 实测确认**（vane-wasm cdylib 建立 后 `cargo bloat` 测）。

#### B.4 I-5 不变量张力与仲裁建议

**张力**：SPEC §14 I-5（line 435）`核心算法代码无 cfg(target)；cfg 只允许出现在 Executor 与 VFS 实现处`。stored.bin zstd 编码若在 `segment/mod.rs` 写期用 `#[cfg(feature = "zstd-encode")]` 分支，严格读法违反 I-5（cfg 出现在 segment 模块，非 VFS/Executor）。

**仲裁建议（供用户批准）**：I-5 释义澄清——
- I-5 禁的是 **`cfg(target=...)` 平台分支**进核心算法（检索/HNSW/BM25/fusion）。
- `cfg(feature=...)` 用于**存储编解码的可选能力**（如 zstd 编码），属「能力开关」非「平台分支」，且 feature 在构建期由 binding crate 选定（与 VFS 后端选择同构）。
- 建议 SPEC §14 I-5 补注释：`cfg(feature) 用于存储编解码能力开关（如 zstd-encode）允许出现在 segment 编解码处；cfg(target) 平台分支仍仅限 VFS/Executor。`

**替代方案（若用户不放宽 I-5）**：将压缩抽象为 `trait StoredCodec { fn encode(&self, raw: &[u8]) -> Result<Vec<u8>>; fn decode(&self, bytes: &[u8]) -> Result<Vec<u8>>; }`，core 仅依赖 trait，zstd 实现放 vane-ffi/vane-node binding 注入。**成本**：core 需在 SegmentWriter 构造时接收 `Arc<dyn StoredCodec>`，穿透 SegmentWriter/new/finalize 签名，改动面大；且 wasm 端仍需 ruzstd 实现 trait。**不推荐**，建议走 I-5 释义澄清。

#### B.5 旧 v1 stored.bin 双模读取与迁移策略

依 SPEC §6.2 line 214 三要求：

1. **version 递增**：stored.bin v1（裸 JSON）→ v2（zstd 块）。见 B.1。
2. **双模读取**（非迁移器）：`decode_stored`（`segment/mod.rs:505`）按 version 分支：
   - v1：现有路径（`mod.rs:505-562`）。
   - v2：读 raw_payload_len + zstd_block → ruzstd 解压 → 对 raw_payload 走 v1 解码逻辑。
   - 不做「v1→v2 原地迁移」：旧 v1 段保持 v1 格式只读服务；新 flush 段写 v2。段合并时新段写 v2（原文从源段读出重新落盘）。
3. **corpus 兼容测试**（§13.3）：`tests/corpus_compat.rs` 现有 `corpus_format_compat_roundtrip`（line 152）冻结 v1 roundtrip。须新增：
   - `corpus_stored_v2_roundtrip`：写 v2（enable `zstd-encode`）→ close → open → search 基线一致。
   - `corpus_stored_v1_read_compat`：用 v1 fixture（或 M1 产物）→ 新版本 open → 读回一致。
   - `corpus_stored_v2_decode_in_wasm`：v2 corpus → wasm32 check 解码路径编译通过（ruzstd 进 wasm）。

#### B.6 SPEC §6.2 修订文本

**SPEC §6.2 原文本（line 211 + line 214）：**
```
└── stored.bin       // 原文/JSON meta（zstd 块压缩）
...
所有文件以 4 字节 `magic` + 4 字节 `format_version` 开头。格式变更必须：① version 递增；② 提供迁移器或双模读取；③ 冻结 corpus 兼容测试通过（§13.3）。
```

**SPEC §6.2 新文本：**
```
└── stored.bin       // 原文/JSON meta；format_version v1=裸 JSON（M0/M1 产物，双模读取保留），v2=zstd 块压缩（M2 起，native/node 写，wasm 读）
...
所有文件以 4 字节 `magic` + 4 字节 `format_version` 开头，**每文件独立 version 递增**（per-file format_version，非全库共用常量）。格式变更必须：① version 递增；② 提供迁移器或双模读取；③ 冻结 corpus 兼容测试通过（§13.3）。stored.bin v1→v2 采用双模读取（不做原地迁移），旧 v1 段只读服务至段合并自然清除。
```

**SPEC §14 I-5 补注释（line 435 后追加）：**
```
- 注：`cfg(feature)` 用于存储编解码能力开关（如 zstd-encode）允许出现在 segment 编解码处；`cfg(target)` 平台分支仍仅限 VFS/Executor 实现。
```

**修订理由**：消解 SPEC §6.2 zstd 承诺与 M1 裸 JSON 实现的张力；per-file format_version 落实 §6.2 line 214「所有文件 magic+version」语义；I-5 释义澄清为 zstd feature-gate 开绿灯，同时锁死 `cfg(target)` 仍仅限 VFS/Executor。

---

## 第二部分：Phase Zero 安全清理任务清单（SPEC 无关，可立即推进）

> 原则：不写实现代码，只写任务规格（file:line + TDD 测试大纲 + 验收），供后续 developer SubAgent 执行。Phase Zero 不触及 §4 IDL，可在 SPEC v1.2 批准前并行推进。

### 2.1 parked minors 逐项

#### 2.1.1 05-is_cjk 代码复制

- **现状**：`crates/vane-core/src/tokenizer/cjk_bigram.rs:97` `fn is_cjk(c: char) -> bool` 与 `crates/vane-core/src/tokenizer/jieba/mod.rs:113` `fn is_cjk(c: char) -> bool` 逐字相同（同 9 个 unicode range `matches!`）。jieba/mod.rs:4 注释自称"复用 M0 `cjk_bigram::is_cjk`"，实为复制。
- **任务**：提取为 `pub(crate) fn is_cjk(c: char) -> bool`，放 `tokenizer/mod.rs` 或 `cjk_bigram.rs`（`pub(crate)`），两处改调用。
- **TDD 大纲**：
  1. 新增 `tokenizer::is_cjk` 单测（复制 cjk_bigram.rs:208-214 的 6 断言：汉/あ/カ 命中，a/空格/1 不命中）。
  2. 回归：`cjk_bigram` 与 `jieba`（feature=jieba）现有分词测试全绿。
- **验收**：两处 `fn is_cjk` 删除，仅余一处 `pub(crate)`；`cargo test --all-features` 全绿；clippy clean。

#### 2.1.2 05-UserTrie max(freq) 缺省值

- **现状**：`crates/vane-core/src/tokenizer/jieba/mod.rs:40-44`：`UserDictEntry::Word(t) => (t.as_str(), max_freq)`，`max_freq = dict.max_freq()`（`dict.rs:78`，词典最高频值）。`seg.rs:29` 注释"freq 缺省值由调用方传入（= dict.max_freq()）"。SPEC §5.3 line 157：`词条格式：字符串（缺省 freq = 内置词典最高频值，保证 DAG 优先命中）`——与实现一致。
- **任务**：确认实现与 SPEC §5.3 一致（已一致）；补文档注释明确"max_freq = 词典最高频，与 jieba-rs 原版一致"；补单测验证缺省 freq 用户词在 DAG 中优先命中（覆盖内置同词）。
- **TDD 大纲**：
  1. 构造词典 + 用户词 `"人工智能"`（无 freq）→ DAG 切分含 `"人工智能"` 整词（非拆分）。
  2. 用户词 freq = max_freq 时覆盖内置同词低 freq 切分路径。
- **验收**：注释补全；新单测过；`cargo test --features jieba` 全绿。

#### 2.1.3 03-compile_filter schema 校验 + 文档注释

- **现状**：`crates/vane-core/src/filter/mod.rs:27` `compile_filter` 参数 `_schema: &Schema` 前缀下划线（未用）。SPEC §10 line 346 `E_INVALID_ARG`：`filter 作用于非标量字段等`。当前未校验 filter 字段是否在 schema 且为 Scalar——字段不存在时 `sr.has_field(field)` 返回 false（`mod.rs:52`）静默 continue，最终该字段无命中（位图空），不报错。
- **任务**：`compile_filter` 入口校验每个 filter 字段在 schema 且为 `FieldDef::Scalar`，否则 `Err(VaneError::InvalidArg(...))`；`_schema` 改 `schema`；补文档注释说明校验语义。
- **TDD 大纲**：
  1. filter 字段不存在 → `Err(E_INVALID_ARG)`。
  2. filter 字段为 Text/Vector 类型 → `Err(E_INVALID_ARG)`。
  3. filter 字段为 Scalar 但类型不匹配 cond（如 Int 字段用 eq String）→ 现有 `scalar_eq` 跨类型返回 false（不命中，不报错）——确认此行为符合 SPEC（§8.3 仅 eq/in/gte/lte，未要求类型不匹配报错）。
  4. 合法 Scalar 字段 → 现有行为不变。
- **验收**：`_schema` 去 `_`；校验逻辑+注释；新单测过；现有 filter 测试全绿。

#### 2.1.4 04-recover 目录扫描

- **现状**：`crates/vane-core/src/wal/mod.rs:140` `recover` 仅重放 WAL `WalRecord::AddSegment`（`mod.rs:171-176`）清理半成品段（ULID 在 WAL 但不在 manifest）。**不扫 `segments/` 目录**：若段文件已写盘但 WAL 未 append 即崩溃（极小概率，但 SPEC §6.4 line 226 要求"半成品 segment 文件按 ULID 不在 manifest 中即判定垃圾，启动时清理"），孤儿段不被清理。
- **任务**：`recover` 末尾加 `Vfs::list("segments")` 扫描，对每个 `seg_<ulid>` 目录，若 ulid 不在 manifest 任何 collection 的 `segment_ulids` 中，调 `merge::delete_segment_dir` 清理。M1-SUMMARY §3.4 标注"S1，非正确性问题"——防御性增强。
- **TDD 大纲**：
  1. 构造 manifest + 一个 manifest 不含的 `seg_<ulid>` 目录（含 header.bin 等文件）→ recover → 目录被删。
  2. manifest 含的段目录 → recover → 保留。
  3. 空 segments 目录 → recover → 无异常。
- **验收**：`recover` 末尾扫描逻辑；新单测过（用 MemoryVfs 或 tempdir + StdFsVfs）；现有 wal 测试全绿。

#### 2.1.5 06-并发测试 jieba 场景

- **现状**：M1 reindex/compact 同步执行（R-4/R-6，`api/reindex.rs:7` 注释），并发测试覆盖 standard 分词器场景，未覆盖 jieba（feature 隔离，并发测试可能未 `--features jieba`）。
- **任务**：新增 jieba 分词器下的并发读写测试：多线程 `search` 与 `setUserDict`/`reindex` 并发，验证 I-4（单一分词身份）不破。
- **TDD 大纲**：
  1. `#[cfg(feature="jieba")]` 测试：jieba collection + N 线程 search + 主线程 setUserDict → reindex → 验证 search 全程不 panic、结果一致（旧身份期间用旧切分）。
  2. jieba collection + 并发 add+flush + search → 无数据竞争（单写者串行，读无锁）。
- **验收**：`cargo test --features jieba` 新测试过；无 UB（miri 或 loom 可选，非门禁）。

#### 2.1.6 02-header.bin tombstone abs/local 语义文档化

- **现状**：`segment/header.rs` 编码（`header.rs:13-30`）将 `meta.tombstones`（`RoaringBitmap`）序列化进 header.bin，但无注释说明存的是**绝对 docid** 还是**段内 local docid**。`wal/mod.rs:14-19` M-minor-2 已文档化 WAL tombstone 存绝对 docid；run期 `CollectionInner.tombstones`（`api/collection.rs:58` 注释"绝对 docid"）也一致。header.bin tombstone 应同一语义（绝对 docid），但 `segment/mod.rs:18` `SegmentMeta.tombstones` 字段注释仅"tombstone 位图（SPEC §6.3）"未标 abs/local。
- **任务**：在 `segment/header.rs` 顶部布局注释（`header.rs:4-7`）与 `SegmentMeta.tombstones` 字段（`segment/mod.rs:18`）补注释："tombstone 存绝对 docid（u32 空间，与 WAL/run-time 一致，M-minor-2）"。
- **TDD 大纲**：无新测试（纯文档化）；回归 `corpus_compat` + header 测试全绿。
- **验收**：注释补全；`cargo test` 全绿。

---

### 2.2 真实中文维基 nDCG corpus

- **背景**：M1 nDCG 验收②（SPEC §13.2-2）用"代表性边界歧义语料"（50 常见 3 字词 + 边界陷阱短语，+84% 达标，见 M1-SUMMARY §2）。M2 须接入真实中文维基 500 篇 + 50 查询 fixture（SPEC §13.2-2 原文要求）。
- **方案**：
  1. **离线获取**：中文维基 dump（`zhwiki-latest-pages-articles.xml.bz2`，通过 `dumps.wikimedia.org` 离线下载，非运行时依赖）。抽取 500 篇正文（按长度 200~2000 字过滤，覆盖科技/历史/地理多领域）。构造 50 查询（短查询 2~4 字，含实体名/概念词/边界歧义词），人工或半自动标注每查询 top-10 相关文档。
  2. **网络可用性**：dump 下载在开发机离线进行，不进 CI 运行时；fixture 提交仓库（500 篇正文 + 50 查询 + relevance judgments）。
  3. **fixture 存放路径**：`crates/vane-core/tests/fixtures/wiki_zh/`（`corpus.json`：500 篇 `{id, text}`；`queries.json`：50 查询 `{qid, text}`；`qrels.json`：`{qid: {docid: rel}}`）。体积控制：500 篇 × 平均 1KB ≈ 500KB，可接受提交。
  4. **替换 M1 代表性语料策略**：M1 的 50 词边界歧义语料保留为**回归对照**（验证 jieba 切分质量不退步），不删除。新增维基 corpus 作为 §13.2-2 主验收。CI jieba nDCG job（M1-SUMMARY §1.3）切换到维基 fixture。
  5. **指标**：jieba-lite 相对完整版 jieba-rs nDCG@10 差 <2%；相对 bigram 提升 ≥15%（SPEC §13.2-2 原口径）。
- **TDD 大纲**：
  1. `tests/ndcg_wiki_zh.rs`：加载 fixture → 建 jieba collection → 50 查询 search → 算 nDCG@10。
  2. 对比基线：bigram collection 同 corpus 同查询 nDCG@10；jieba-rs 完整版（dev-dep，feature gated）同 corpus nDCG@10。
  3. 断言：jieba vs bigram 提升 ≥15%；jieba vs jieba-rs 差 <2%。
- **验收**：CI `jieba-nDCG` job 跑维基 fixture 通过；M1 边界歧义语料回归测试保留且通过；fixture 提交仓库。

---

### 2.3 vane-wasm crate 骨架

- **背景**：M1 wasm 体积用 vane-core cdylib 测（557KB gzip，M1-SUMMARY §2），但真实 deliverable 是 `vane-wasm` cdylib + wasm-bindgen（SPEC §12.1 line 369 `crates/vane-wasm   # wasm-bindgen + Worker 胶水 [M2]`）。Phase Zero 先建骨架，测真实体积基线。
- **任务**：
  1. 新建 `crates/vane-wasm` crate：`Cargo.toml`（`crate-type = ["cdylib"]`，dep `vane-core`（default features，不含 jieba/dict-zh）+ `wasm-bindgen`），`src/lib.rs` 占位（`#[wasm_bindgen] pub fn vane_version() -> String { ... }`，不引浏览器 API，不引 OPFS/IDB）。
  2. workspace `Cargo.toml`（根）`members` 追加 `"crates/vane-wasm"`。
  3. CI wasm32-size job（M1 已有）增加 `cargo build --target wasm32-unknown-unknown -p vane-wasm` + 体积测量（`wasm-opt -Oz` + gzip）。
- **TDD 大纲**：
  1. `cargo check --target wasm32-unknown-unknown -p vane-wasm` 编译通过（证明 core 可编 wasm deliverable）。
  2. 体积基线：`vane-wasm` cdylib gzip ≤ 800KB（含 wasm-bindgen 胶水，不含 jieba/词典）。记录基线值，与 vane-core cdylib 557KB 对比。
  3. `check-no-std-fs.sh` 扫描 `crates/vane-wasm/src/` 无 `std::fs`（合法：wasm 不用 std::fs）。
- **初步体积评估**：wasm-bindgen 胶水约 +10~20KB gzip；vane-core 557KB + 胶水 ≈ 570~580KB，远在 800KB 内。ruzstd 若进 wasm（修订 B 解码侧）+30~60KB ≈ 610~640KB，仍达标。**须实测确认**。
- **验收**：`crates/vane-wasm` 骨架建立；workspace 注册；CI wasm32-size job 测 vane-wasm 体积基线并通过 800KB 门禁；不引任何浏览器 API（仅占位函数）。

---

## 第三部分：M2 模块分解预览（供 plan-splitter 细化）

| # | 模块 | 一句话目标 | 前置依赖 | SPEC 节号 | Phase Zero 已处理 |
|---|---|---|---|---|---|
| M2-00 | Phase Zero 安全清理 | parked minors + wiki nDCG corpus + vane-wasm 骨架 | 无 | §13.2-2/§13.3 | —（本报告定义） |
| M2-01 | vane-wasm cdylib + 体积门禁 | wasm-bindgen 胶水 + SIMD 探针占位 + 800KB 门禁强制 | M2-00 骨架 | §12.1/§12.2/§13.2-3 | 骨架（M2-00） |
| M2-02 | OPFS VFS 后端 | `OpfsVfs` 实现 Vfs trait（SyncAccessHandle，Worker 内同步） | M2-01 | §6.1/§4.1（REQUIREMENTS） | 否 |
| M2-03 | IndexedDB 降级 VFS | `IdbVfs` 适配层（binding crate，OPFS 不可用时降级） | M2-02 | §6.1/§13.2（降级不抛错） | 否 |
| M2-04 | Dedicated Worker 壳 | Worker 胶水 + postMessage Promise 边界 + init 探针 | M2-01/M2-02 | REQUIREMENTS §4.1 | 否 |
| M2-05 | SIMD128 双变体 | wasm simd128 默认 / scalar fallback 两产物 + init 探针选择 | M2-01 | §8.4/§12.2 | 否 |
| M2-06 | SIMD 双变体召回回归 | 两变体各跑 recall@10≥0.95 五档回归 | M2-05 | §8.4/§13.2-1 | 否 |
| M2-07 | 冷启动懒加载 | SegmentReader 按需加载 vectors/stored（OnceLock），open <1s | SPEC v1.2 修订 A 批准 | §13.1/§6.2 | 否（SPEC-gated） |
| M2-08 | stored.bin zstd + per-file format_version | stored v2 zstd 块 + 双模读取 + per-file version 常量 | SPEC v1.2 修订 B 批准 | §6.2/§14 I-5 | 否（SPEC-gated） |
| M2-09 | SQ8 向量量化 | f32→SQ8 量化编码/解码 + 距离计算适配，内存降 4 倍 | M2-07（懒加载改 vectors 访问点） | §13.1（<200MB）/REQUIREMENTS §3 | 否 |
| M2-10 | 100 万规模承诺恢复 | 段合并策略调优 + Executor 并行搜索（rayon）+ 100万压测 | M2-09（SQ8 降内存） | §3.3/§11/§13.1 | 否 |
| M2-11 | Go cgo 绑定 | vane-ffi C ABI 实装 + cgo staticlib + zig cc 交叉 + wazero build tag | vane-ffi 占位已存在（`crates/vane-ffi/src/lib.rs` M0 占位） | §9/§12.2/§12.3 | 否 |
| M2-12 | export 快照导出 | `Db.export(destPath)` 打包单文件快照实装 | M2-02（OPFS 写快照） | §4.1/§15 | 否 |
| M2-13 | 真实维基 nDCG corpus | 500 篇 + 50 查询 fixture + nDCG 验收② | M2-00 corpus 方案 | §13.2-2 | 方案（M2-00） |
| M2-14 | Demo（纯前端 markdown 搜索） | 拖入 md 文件夹本地混合搜索（含中文） | M2-04/M2-05 | §15 | 否 |

**依赖拓扑（高层）**：
- Phase Zero（M2-00）无前置，可立即推进。
- M2-07（懒加载）/ M2-08（stored-zstd）blocked by SPEC v1.2 用户批准。
- M2-09（SQ8）依赖 M2-07（懒加载改 vectors 访问点，SQ8 量化层挂在懒加载的 vectors 加载路径上）。
- M2-10（100万）依赖 M2-09（SQ8 降内存后 100万可行）+ Executor 抽象（§11）。
- M2-02/03/04（浏览器三件套）依赖 M2-01（vane-wasm）。
- M2-06 依赖 M2-05（SIMD 双变体）。
- M2-11（Go cgo）独立链，可与浏览器链并行。

**Phase Zero 已处理标注**：M2-00 涵盖 parked minors、wiki corpus 方案、vane-wasm 骨架；其余模块均未在 Phase Zero 处理，避免重复。

---

## 附：发现的 SPEC 内部矛盾 / 与实现冲突

1. **stored.bin zstd**（本报告修订 B）：SPEC §6.2 line 211 写 zstd，实现裸 JSON。M1 已知张力，M2 消解。
2. **FORMAT_VERSION 全局共用 vs per-file 递增**（本报告修订 B）：SPEC §6.2 line 214 要求每文件 magic+version 且 version 递增，实现 `types.rs:15` 全局单常量——无法单文件 bump。M2 引入 per-file 常量。
3. **I-5 严格读法 vs zstd feature-gate**（本报告修订 B.4）：I-5 字面禁 cfg 出现在 VFS/Executor 之外；stored zstd 编码需 `cfg(feature)` 在 segment 模块。建议 I-5 释义澄清（仅禁 `cfg(target)`，允许 `cfg(feature)` 能力开关）。
4. **§13.1 冷启动 <1s 承诺 vs M1 实测 1573ms**（本报告修订 A）：M1 走降级分级接受，M2 懒加载消解。SPEC §13.1 line 409 措辞"M1 实测背书"与 M1 实测未达 <1s 矛盾，M2 修订为"元数据 open <1s（M2 实测背书）"。
5. **cold-start bench R-11-2/3 遗留**（`11-cold-start-bench-report.md:71-78`）：fixture 生成 ~265s 慢 + benchmark.yml 跑 cold_start bench 拖慢夜间 job。M2 懒加载后 fixture 生成方式可优化（懒加载下 open 不需全加载，fixture 生成仍需写盘，但 gate 断言可简化）。非阻塞。

**无阻塞 M2 的架构债**：所有张力均有明确修订路径（修订 A/B + I-5 澄清），待用户批准 SPEC v1.2 后可推进。
