# Vane 技术规范（SPEC v1.3）

> 依据 `docs/REQUIREMENTS.md` v1.1 形式化。本文档与需求合同的关系：REQUIREMENTS 回答"做什么/为什么"，
> 本 SPEC 回答"精确怎么做"——所有接口签名、格式布局、状态机、数值门禁以本文档为准。
> 变更纪律：任何与本文件的偏离必须走 spec 修订（版本号 + changelog），不允许代码先行。

---

## 1. 范围与引用

- 适用里程碑：M0–M2。各条款标注生效里程碑：`[M0]` `[M1]` `[M2]`；未标注者自 M0 生效。
- 上游文档：REQUIREMENTS.md v1.1（三方研讨结论与仲裁记录）。
- 本文档不包含：embedding 生成、GPU、分布式、SQL、服务端模式（Won't-have，见 REQUIREMENTS §2）。

## 2. 术语

| 术语 | 定义 |
|---|---|
| Collection | 一个检索集合 = 一份 schema + 若干 segment + 一个 manifest |
| Segment | 不可变索引单元，含 HNSW 图、倒排、列式标量块、向量数据，写一次后只读 |
| Manifest | `manifest.json`，指向当前有效 segment 集合的唯一权威指针，靠 `rename` 原子切换 |
| 快照（Snapshot） | 读查询持有的段列表视图（Arc swap 获得），生命周期内不变 |
| flush | 将内存 buffer 段物化为新 segment 并切换 manifest 的操作 |
| reindex | 用新分词器身份重建全部 segment 的后台增量过程（§7.4） |
| NRT 语义 | 写入 flush 后对新快照原子可见；非"写入即可见" |

---

## 3. 数据模型

### 3.1 Schema [M0]

Collection 创建时声明，创建后仅允许附录式扩展（新增字段），禁止修改/删除既有字段。

```
Field :=
  | { type: "text" }                       // 进 BM25 倒排，可多字段
  | { type: "vector", dim: u32, metric: "cosine" | "l2" | "dot" }
  | { type: "scalar", kind: "int" | "float" | "bool" | "keyword" }   // 可过滤
```

约束：
- 每个 collection **恰好一个** vector 字段（M0–M2 限制，避免多向量融合语义爆炸）。
- text 字段 ≥1 时可省略（纯向量 collection 合法）；scalar 字段任意多个。
- `dim` 上限 4096；metric 默认 `cosine`。

### 3.2 文档

```
Document := { id: string, <field>: value, ... }
```

- `id`：外部字符串主键，≤512 字节；collection 内唯一；`add` 语义为 **幂等 upsert by id**。
- 内部映射为 `u64 docid`（段内单调分配），映射表随段持久化。
- 单文档序列化后 ≤ 16MB；vector 维度必须等于 schema `dim`，否则报 `E_SCHEMA`（§10）。

### 3.3 规模红线

| 项 | 值 | 说明 |
|---|---|---|
| M0/M1 优化目标 | 10 万文档 | 全指标按此承诺 |
| M0/M1 不塌红线 | 50 万文档 | 不承诺延迟，承诺不崩不错 |
| M2 设计上限 | 100 万文档 | 恢复承诺 |
| 浏览器端（M2） | 5 万文档验收边界 | 架构按 50 万设计，代码禁止写死 5 万假设 |
| 段数硬上限 | 10 | 超限强制合并，小段（<1 万文档）优先 |

---

## 4. 公共 API（语言无关 IDL）

三侧绑定共用本 IDL；binding 是无逻辑薄壳，行为测试全部跑在 Rust core。**本节约定的签名 M0 冻结**。

### 4.1 函数清单（6 动词 + 4 管理函数）

```
open(path: string, opts?: OpenOptions) -> Db
Db.collection(name: string, schema: Schema & CollectionOptions) -> Collection   // 幂等：同名同 schema 返回既有
Db.collections() -> [string]
Db.export(destPath: string) -> Result<()>          // 打包单文件快照
Db.close() -> Result<()>

Collection.add(docs: [Document]) -> Result<AddReport>          // 批量幂等 upsert
Collection.flush() -> Result<()>                               // 可见性边界
Collection.search(query: SearchQuery) -> Result<[Hit]>
Collection.delete(ids: [string]) -> Result<u64>                // 返回 tombstone 数
Collection.compact() -> Result<()>                             // 手动触发段合并
Collection.reindex() -> Result<ReindexHandle>                  // §7.4
```

### 4.2 参数结构

```
OpenOptions := {
  persistence?: "persistent" | "best-effort"   // 默认 persistent；WASM 侧映射 navigator.storage.persist()
  autoCommit?: { intervalMs?: u32 = 1000, maxDocs?: u32 = 1000 } | "off"   // 默认开启
  pageCacheMb?: u32 = 32
}

CollectionOptions := {
  tokenizer?: "standard" | "cjk_bigram" | "jieba"   // 默认 "standard"
  userDict?: [ string | { term: string, freq?: u32 } ]
  // M2 WASM: dictData?: bytes   —— 内联词典注入，离线/自托管场景
}

SearchQuery := {
  text?: string, vector?: [f32],        // 至少其一；两者皆给 = 混合
  topK?: u32 = 10,                      // 上限 1000
  mode?: "hybrid" | "vector" | "text",  // 缺省按入参推断；显式指定优先
  fusion?: "rrf" | { linear: { alpha: f32, norm: "minmax" } },   // 默认 "rrf"
  filter?: Filter,                       // §8.3
  candidateMultiplier?: u32 = 3          // RRF 两路各取 topK × multiplier
}

Hit := { id: string, score: f32, fields?: {…} }   // rrf 模式下 score 为 RRF 分；linear 模式为融合分

Filter := { <scalarField>: { eq?: v, in?: [v], gte?: v, lte?: v } }   // 多字段为 AND

AddReport := { accepted: u64, visibleAfterFlush: true }
ReindexHandle := { progress(): f32, wait(): Result<()> }   // 可轮询可阻塞
```

### 4.3 三侧签名映射

| IDL | JS（Node/WASM） | Go |
|---|---|---|
| 返回值 + 错误 | `Promise<T>`，reject 携带 `VaneError`（含 code） | `(T, error)`，error 实现 `VaneError` 接口 |
| `bytes` | `Uint8Array` / `Buffer` | `[]byte` |
| camelCase | `userDict`, `topK` | `UserDict`, `TopK`（结构体字段） |
| 阻塞性 | 全部 async | 全部阻塞（goroutine 安全） |

并发承诺：**所有公开 API 线程/goroutine 安全**；单 collection 写路径内部串行（单写者），读路径无锁并发。

---

## 5. 分词器规范

### 5.1 内置分词器

| 名称 | 管线 | 词典 | 生效 |
|---|---|---|---|
| `standard` | unicode 分词 → lowercase → Porter stemmer | 无 | M0 |
| `cjk_bigram` | CJK 连续 run 切二元组；非 CJK 走 standard 管线 | 无 | M0 |
| `jieba` | 前缀 DAG 最大概率切分 + HMM 未登录词识别；词典见 §5.2 | 精简词典 ~20 万词 | M1 |

**中英混排统一规则**：先按 unicode script 边界切 run；CJK run 进 jieba/bigram，Latin/digit run 进 lowercase+stemmer；token position 全程连续递增（跨语言 phrase query 正确性依赖此不变量）。

### 5.2 jieba 词典（`jieba-lite`）

- 来源：jieba 开源词表剪枝，保留 ~20 万高频词 + 全部单字 + 词频；**算法与 jieba-rs 完全一致，仅裁剪词典**。
- 物理格式：`dict.bin` = zstd 压缩的双数组 Trie（DAT）序列化 blob，头部 16 字节：`magic(4) | format_version(4) | sha256(8 前缀)`。
- 分发与版本：独立日历版本（`YYYY.MM`），与库 semver 解耦；三侧分发通道见 §12。
- HMM 参数（转移矩阵，压缩后 ~200KB）随 `dict.bin` 同包。

### 5.3 自定义词表

- 注入点：collection 创建参数 `userDict`，或运行期 `setUserDict`（§7.4 暂存语义）。
- 词条格式：字符串（缺省 freq = 内置词典最高频值，保证 DAG 优先命中）或 `{term, freq}`。
- 优先级：**用户词 > 内置词；同为用户词则 freq 高者优先；歧义消解完全保持 jieba 原版行为**（不另发明规则）。
- 上限：10 万词条；超限报 `E_DICT_TOO_LARGE`。

### 5.4 分词器身份

```
TokenizerId := sha256( algorithm_version ‖ builtin_dict_version ‖ user_dict_bytes )
```

写入 collection 元数据与每个 segment 头部。**任何时刻一个 collection 只有一套生效分词身份**（不变量 I-4，§14）。

**`builtin_dict_version` 语义澄清（v1.1）**：`builtin_dict_version` 是**编译期词典格式 spec 版本**常量（如 `b"jieba-fmt-v1"`），仅当 DAT 结构 / HMM 参数**格式**变更时递增；词典**内容**升级（增删词条、日历版本变化）**不改变** `builtin_dict_version`。词典运行时日历版本（如 `2026.08`）+ 内容 sha256 前缀仅供 §12.3 三渠道一致性校验与 §3.3 升级警告，**不进 TokenizerId**。

效果：
- 词典内容升级 → `builtin_dict_version` 不变 → TokenizerId 不变 → 旧段可继续查询（REQUIREMENTS §3.3「仅警告不强制重建」满足）。
- 词典格式升级 → `builtin_dict_version` 递增 → TokenizerId 变化 → reindex 触发（合理：格式不兼容需重建）。

---

## 6. 存储格式

### 6.1 VFS trait（M0 冻结签名）

```rust
trait Vfs: Send + Sync {
    fn create(&self, path: &str) -> Result<()>;
    fn read_at(&self, path: &str, buf: &mut [u8], offset: u64) -> Result<usize>;
    fn write_at(&self, path: &str, buf: &[u8], offset: u64) -> Result<()>;
    fn append(&self, path: &str, buf: &[u8]) -> Result<u64>;   // 返回写入起始 offset
    fn sync(&self, path: &str) -> Result<()>;
    fn rename(&self, from: &str, to: &str) -> Result<()>;      // manifest 原子切换唯一原语
    fn delete(&self, path: &str) -> Result<()>;
    fn list(&self, dir: &str) -> Result<Vec<String>>;
}
```

- core 对 IO 的全部认知仅此接口；**core crate 禁止出现 `std::fs` / `std::net` / mmap**（CI 门禁，§13.3）。
- 四后端：`std-fs`（native）、`opfs`（WASM Worker 主）、`idb`（WASM 降级，适配层在 binding crate）、`memory`（测试/纯内存）。Memory + OPFS 后端自 M0 起跑同一测试套件。
- 页缓存：LRU buffer pool，默认 32MB，页大小 64KB（对齐 IndexedDB chunk 模拟粒度）。

### 6.2 目录布局

```
<db>/
├── manifest.json            // { version, segments: [...], collections: {...} }
├── wal.log                  // 薄 WAL：仅段增删/tombstone 追加元操作 [M1]
└── segments/
    └── seg_<ulid>/
        ├── header.bin       // magic | format_version | tokenizer_id | docid_range | tombstone_bitmap
        ├── vectors.bin      // f32 定长连续排布（docid 序）
        ├── hnsw.bin         // 段内 HNSW 图（自研格式，fork instant-distance 演进）[M1]
        ├── inverted.bin     // 倒排：词典块 + posting 块（§6.3）
        ├── scalars.col      // 列式标量块，按字段分区
        └── stored.bin       // 原文/JSON meta；format_version v1=裸 JSON（M0/M1 产物，双模读取保留），v2=zstd 块压缩（M2 起，native/node 写，wasm 读）
        // 懒加载语义（M2）：SegmentReader::open 仅读 header.bin + idmap.bin + manifest；
        // vectors.bin / stored.bin / hnsw.bin 首次访问时按需加载（OnceLock，core 内部，不改 §4 IDL 签名）。冷启动承诺见 §13.1。
```

所有文件以 4 字节 `magic` + 4 字节 `format_version` 开头，**每文件独立 version 递增**（per-file format_version，非全库共用常量）。格式变更必须：① version 递增；② 提供迁移器或双模读取；③ 冻结 corpus 兼容测试通过（§13.3）。stored.bin v1→v2 采用双模读取（不做原地迁移），旧 v1 段只读服务至段合并自然清除。

### 6.3 倒排布局

- 词典块：FST 或有序数组二分（M0 允许后者，格式版本预留）。
- posting：`(docid_delta, tf)` 变长编码（vbyte），每 128 文档一跳块；跳块头记录块内 `max_score`（BM25 上界），供 Block-Max WAND top-k 剪枝。
- BM25 参数：`k1 = 1.2, b = 0.75`（冻结，进 format_version 语义）。
- 删除：tombstone roaring bitmap 存 header.bin；查询期过滤；段合并时物理清除。

### 6.4 写入与崩溃恢复

1. 写入进内存 buffer 段；`flush`（或 auto-commit 触发）时：构建新 segment 文件集 → `sync` 每个文件 → `append` WAL 记录 → 新 manifest 写临时文件 → `sync` → **`rename` 原子切换**。
2. 崩溃恢复：manifest 永远指向最后一个完整状态；WAL 重放仅恢复 tombstone 追加与段增删元操作；半成品 segment 文件按 ULID 不在 manifest 中即判定垃圾，启动时清理。
3. **无 mmap 依赖**：native 同样走显式 read + 页缓存（全平台同一代码路径；mmap 只读模式仅作 Could-have 附加）。

---

## 7. 一致性与可见性

### 7.1 写入可见性（NRT）

- `add()` 返回时数据**不保证可搜**；`flush()` 后对新快照**原子可见**——向量与 BM25 两类索引在同一次 manifest 切换中同时出现（不变量 I-2）。
- auto-commit 默认开启：`intervalMs=1000` 或 `maxDocs=1000` 先到先触发。API 语义兼容未来真 NRT（内存 buffer 段可查），升级不改签名。

### 7.2 删除

- `delete(ids)` = 追加 tombstone（即时进 WAL，flush 后随段生效）；tombstone 比例进可观测指标；`compact()` 或自动分层合并时物理清除。
- **HNSW 图从不原地删除**；段合并时新图从零重建（不变量 I-3）。无独立"图重建"API，无"删除 >20% 提示用户"话术。

### 7.3 段合并

- 策略：分层合并简化版；段数硬上限 10，超限强制合并；小段（<1 万文档）优先。
- 调度：合并为**可切片增量任务**（每片处理 N 个 posting 块/图节点后 yield）；native 由 Executor 投后台，WASM 在写间隙小步推进；合并全程不阻塞读（快照不可变），只可能延迟写。
- 合并触发与写路径解耦；持续小批量写入场景文档建议"攒批 ≥100 条"。

### 7.4 词表变更与 reindex [M1]

状态机：

```
Stable ──setUserDict()──> PendingReindex ──reindex()──> Rebuilding ──完成──> Stable
   ▲                        │ 新写入仍用旧分词身份          │ 旧段只读服务
   └──── 放弃（再次 setUserDict 覆盖暂存词表）◄─────────────┘ 原子切换
```

- `PendingReindex` 状态下：写入/查询正常，全部使用**旧**分词身份；`search` 响应头携带 `needsReindex: true`。
- `Rebuilding`：复用段合并管线，全量段逐一以新分词器重建；完成前查询仍命中旧段；完成后 manifest 一次切换，新词表原子生效。
- 禁止行为：新旧分词身份混排检索、自动触发全量重建、查询期多版本词表合并。

---

## 8. 查询语义

### 8.1 模式

| mode | 召回路径 | 排序 |
|---|---|---|
| `vector` | HNSW 段级并行搜索 → 归并；过滤候选 < 2×topK 时暴力精确回退 | 向量距离 |
| `text` | Block-Max WAND top-k | BM25 |
| `hybrid` | 两路各取 `topK × candidateMultiplier` 候选 | 融合（§8.2） |

### 8.2 融合

- 默认 **RRF**：`score(d) = Σ_path 1/(k + rank_path(d))`，`k = 60`（冻结）。
- `linear`：`alpha × norm(vector_score) + (1-alpha) × norm(bm25_score)`，norm 仅支持 `minmax`（按当次候选集归一化）。linear 为显式选项，文档标注"分数跨语料不可比，调参责任在调用方"。
- API 默认路径不出现 `alpha`。

### 8.3 过滤（pre-filter）

- `filter` 编译为 roaring 位图，**作为参数传入 HNSW 遍历**（访问邻居时检查）与 WAND 推进（skip 不在位图的文档）。
- 禁止以 post-filter 为主策略；位图基数 < 2×topK 时向量路自动切换暴力精确扫描（100% 召回）。
- 标量条件：`eq / in / gte / lte`，字段间 AND；不支持 OR/NOT（M0–M2，避免规划器复杂度）。

### 8.4 质量门禁

- **hybrid recall@10 ≥ 0.95**（口径：相对"暴力双路 + RRF"基线），CI 硬门禁；召回回归覆盖 0.1% / 1% / 10% / 50% / 99% 五档过滤选择率。
- SIMD128 与 scalar 两个 wasm 变体各跑一遍召回回归（防 SIMD 数值路径分歧）。

---

## 9. FFI 规范（C ABI，`vane-ffi`，cbindgen 生成）

### 9.1 约定

- **句柄**：所有对象（Db / Collection / ReindexHandle）对外为 `uint64_t` 不透明句柄，core 内全局注册表 `std::sync::RwLock<HashMap<u64, Arc<…>>>`（v1.1：原 `DashMap` 与依赖黑名单冲突，统一 `std::sync`）；`vane_*_close(h)` 注销。禁止裸指针出边界。
- **错误**：所有函数返回 `int32_t`（0=OK，负值=错误码，§10）；详情经 `vane_last_error_message(h) -> char*` 获取，调用方负责 `vane_string_free`。
- **内存铁律**：谁分配谁释放，跨边界只借不还。宿主传入 buffer 仅在调用期间借用；C 侧返回的 buffer 由对应 `vane_*_free` 释放。批量结果：arena 一次分配 + 一次 free。
- **并发**：句柄内部 `RwLock`；API 文档承诺 goroutine 安全并发调用。

### 9.2 函数面（与 §4.1 一一对应）

```
vane_open(path_ptr, path_len, opts_json, out_handle*) -> i32
vane_collection(db_h, name, schema_json, out_handle*) -> i32
vane_add(col_h, docs_json) -> i32
vane_flush(col_h) -> i32
vane_search(col_h, query_json, out_arena*) -> i32
vane_delete(col_h, ids_json, out_count*) -> i32
vane_compact / vane_reindex / vane_export / vane_close / vane_string_free / vane_last_error_message
```

**v1.1 补列**（ReindexHandle 必需 + M1 词典分发扩展）：
```
vane_reindex_progress(h, out_progress*) -> i32   // ReindexHandle.progress() 落 FFI（§4.1 IDL 要求）
vane_reindex_wait(h) -> i32                       // ReindexHandle.wait() 落 FFI
vane_load_dict(h, dict_ptr, dict_len) -> i32      // M1 词典分发：注入 jieba 词典字节（Node/Go 侧调用）
vane_dict_version(out_ptr, out_len*) -> i32       // M1 词典分发：查当前词典日历版本 + sha256 前缀
```

参数/返回一律 JSON 序列化（binding 薄壳原则；性能敏感的 `vane_search` 允许 M1 评估升级为定长二进制，需 spec 修订）。

### 9.3 Node 例外

Node **不经过 C ABI**，`vane-node` 用 napi-rs 直连 core（N-API v6+）；异步经 `AsyncTask` 提交 core 内部线程池，不桥接 tokio。

---

## 10. 错误码

| code | 名称 | 含义 |
|---|---|---|
| 0 | OK | — |
| -1 | E_IO | VFS 层读写失败 |
| -2 | E_SCHEMA | schema 不符（维度错/字段类型错/未知字段） |
| -3 | E_NOT_FOUND | collection / 文档 id 不存在 |
| -4 | E_CORRUPT | 段/manifest 校验失败（magic、version、sha256） |
| -5 | E_VERSION | 格式版本过新且无迁移器 |
| -6 | E_TOKENIZER_MISMATCH | 查询期分词身份与段不符（提示 reindex 状态） |
| -7 | E_DICT_TOO_LARGE | 用户词表超 10 万词条 |
| -8 | E_DICT_UNAVAILABLE | jieba 词典未加载且已声明 `tokenizer:"jieba"`（注：WASM 侧禁止到达此错误——自动降级 bigram + warn，见 §12.4） |
| -9 | E_BUSY | reindex/compact 进行中，冲突操作 |
| -10 | E_UNSUPPORTED | 平台能力缺失（如无 OPFS 且未启用 idb 降级） |
| -11 | E_INVALID_ARG | 参数非法（topK>1000、filter 作用于非标量字段等） |

三侧绑定透传 code，不得吞并/重编。

---

## 11. 并发与执行模型

- `trait Executor { fn spawn(&self, task: Task); fn scope(&self, …); }`：native 实现 = rayon；wasm 实现 = 同线程串行。**`cfg` 只允许出现在 Executor 与 VFS 实现处**，核心算法零 cfg（不变量 I-5）。
- 读路径零锁：快照经 `Arc<SegmentSet>` swap 获取。
- 写路径：单写者队列；flush/合并/reindex 互斥（E_BUSY）。
- 明确不支持：SharedArrayBuffer 多线程 WASM（COOP/COEP 部署成本）；tokio 进 core。

---

## 12. 构建、分发与平台矩阵

### 12.1 Workspace

```
crates/vane-core   # 检索核心；feature: std(默认) / wasm / jieba(M1)
crates/vane-ffi    # cdylib + staticlib + cbindgen
crates/vane-node   # napi-rs
crates/vane-wasm   # wasm-bindgen + Worker 胶水 [M2]
bindings/go        # cgo 包装 + 预编译 .a
bindings/node      # npm 包骨架
```

### 12.2 目标矩阵

| 产物 | target | 里程碑 |
|---|---|---|
| Node prebuilt（M0 即 4 个） | x86_64-linux-gnu / aarch64-apple-darwin / x86_64-apple-darwin / x86_64-pc-windows-msvc | M0 |
| Node prebuilt 追加 | linux-musl ×2、linux-arm64、win-arm64（可选） | M1 |
| Go staticlib | 同 Node 矩阵（zig cc 交叉） | M1 |
| wasm32-unknown-unknown | core CI check 门禁 | **M0 起** |
| wasm 双变体（simd128 默认 / scalar fallback，init 探针选择） | 对外交付 | M2 |

### 12.3 词典分发（jieba-lite）[M1/M2]

| 侧 | 通道 | 约束 |
|---|---|---|
| Node | `@vane/dict-zh` 平台无关数据包，主包**正式 dependency**；`@vane/slim` 无词典变体 | 禁止 postinstall 下载；包体 ≤1.5MB gzip（CI 门禁） |
| Go | `go:embed dict.bin.gz`；`//go:build vane_nodict` 裁剪 tag；`vane.DictVersion()` | embed 二进制增量 <2MB（CI 门禁） |
| WASM [M2] | 默认 CDN URL fetch → sha256 校验 → OPFS 缓存；支持 `dictData` 内联注入 | fetch 失败自动降级 bigram + console.warn，**不抛错** |

词典独立日历版本化；库 release 前校验三渠道所钉词典版本哈希一致，不一致阻断发版。

### 12.4 版本与发布

crates.io / npm / Go module 三端版本号严格同步，单一 release 脚本驱动；产物命名 `libvane-{version}-{triple}.{a|node|wasm}`。

---

## 13. 非功能指标与 CI 门禁

### 13.1 性能承诺（native/Node，10 万×384 维）

| 指标 | 承诺 |
|---|---|
| hybrid topK=10 P99 | < 50ms（HNSW [M1]）/ < 150ms（暴力 [M0]） |
| 批量 add | ≥ 5k docs/s（含索引构建） |
| 内存（全加载） | < 500MB；SQ8 后 < 200MB [M2] |
| 冷启动（打开 10 万库） | 元数据 open < 1s（vectors/stored 懒加载，M2 实测背书）；首次向量查询触发 vectors 加载，<3s（降级分级保留为 fallback） |
| 词典冷加载 | < 150ms（预编译 dict.bin 零拷贝反序列化）[M1] |
| WASM 端 | 上述延迟放宽 3~5 倍（暂估值，M2 前出实测预算表）[M2] |

### 13.2 质量门禁（CI 硬卡）

1. hybrid recall@10 ≥ 0.95（相对暴力双路+RRF 基线），五档选择率回归。
2. 中文分词（M1）：① 200 句测试集与 jieba-rs 原版切分 100% 一致；② 中文维基 500 篇 + 50 查询，jieba-lite 相对 bigram nDCG@10 不退步（≥0%，M2 实测 +0.4%——bigram 在真实维基为强基线，数学上限≈7.5%）；相对 bigram ≥15% 提升由代表性边界歧义语料（合成 trap corpus，M1 实测 +84%）承载；相对完整版 nDCG@10 差 <2% 由 ① 的 200 句 100% 切分一致性覆盖（切分一致→nDCG 差 0%）；③ 20 个生造词注入 userDict 后单 token 入索引、短语命中 100%；④ 缺词典自动降级不抛错。
3. 体积：核心 wasm gzip ≤ 800KB（含 jieba 代码、不含词典）；全功能 ≤ 1.2MB；`@vane/dict-zh` ≤ 1.5MB；Go embed 增量 < 2MB；500KB 为 M2 优化目标非门禁。
4. 平台四包管理器（npm/yarn/pnpm/bun）安装矩阵通过。

### 13.3 工程纪律门禁

- `cargo check --target wasm32-unknown-unknown -p vane-core`：core 出现 `std::fs` 即失败（M0 第一天起）。
- `cargo-deny` 依赖审查 + `cargo bloat` 周报；依赖黑名单：regex / tokio 全套 / prost / tonic / openssl / lindera / ndarray / wee_alloc。
- 冻结 corpus 格式兼容测试：旧版本写出的库必须被新版本打开。
- benchmark CI：性能回退 >10% 报警。

---

## 14. 不变量清单（测试必须覆盖）

- **I-1 段不可变**：segment 文件写一次后只读；任何更新 = 新段 + manifest 切换。
- **I-2 双索引原子可见**：flush 后向量与倒排在同一快照同时出现；不存在"半可见"状态。
- **I-3 图不原地删**：HNSW 图节点删除只经 tombstone；图重建仅发生在段合并。
- **I-4 单一分词身份**：任意时刻一 collection 一套 TokenizerId；新写入在 reindex 完成前必须用旧身份。
- **I-5 核心零平台分支**：core 算法代码无 `cfg(target)`；平台差异仅在 VFS/Executor 实现。
  - 注：`cfg(feature)` 用于存储编解码能力开关（如 zstd-encode）允许出现在 segment 编解码处；`cfg(target)` 平台分支仍仅限 VFS/Executor 实现。
- **I-6 manifest 原子性**：任何崩溃后 manifest 指向完整状态；孤儿段文件可安全清理。
- **I-7 FFI 内存铁律**：谁分配谁释放，跨边界只借不还；句柄注销后使用 = 明确错误而非 UB。
- **I-8 binding 薄壳**：三侧绑定无检索逻辑；行为差异视为 bug。

---

## 15. 里程碑验收对照

| 里程碑 | 交付 | 验收锚点 |
|---|---|---|
| M0（4–6 周） | 暴力向量 + BM25 + RRF + 持久化 + flush 语义 + Node 4 平台 prebuilt；分词 standard/bigram + tokenizer API 占位 | §13.2-1（暴力口径）、§13.2-3 核心档、demo 三列排序对比 + 对比 sqlite-vec+FTS5 代码量 |
| M1 | 分段 HNSW、tombstone+合并、pre-filter、Go cgo（可后移，分词不让位）、薄 WAL、**jieba-lite + userDict + setUserDict/reindex + Node/Go 词典分发** | §13.2-1/2 全量、冷启动实测背书、§7.4 状态机用例 |
| M2 | 浏览器交付（OPFS+IDB 降级+Worker 壳+SIMD 双变体）、词典 CDN fetch、SQ8、export 快照、100 万规模 | §13.1 WASM 放宽档、降级不抛错用例、浏览器 5 万文档验收 |

---

## Changelog

- **v1.0**（2026-08-09）：自 REQUIREMENTS v1.1 形式化，含第三轮复议结论（默认中文分词 + 自定义词表 + 词表暂存/reindex 语义仲裁）。
- **v1.1**（2026-08-09）：M1 计划审查闭环后三处修订。S1 §5.4 澄清 `builtin_dict_version` = 编译期词典格式 spec 版本常量（非日历内容版本），词典内容升级不改变 TokenizerId（满足 REQUIREMENTS §3.3「仅警告不强制重建」）。S2 §9.1 FFI 句柄注册表 `DashMap` → `std::sync::RwLock<HashMap>`（消除与依赖黑名单冲突）。S3 §9.2 补列 `vane_reindex_progress` / `vane_reindex_wait`（ReindexHandle IDL 落实）+ `vane_load_dict` / `vane_dict_version`（M1 词典分发扩展）。
- **v1.2**（2026-08-09）：M2 scoping 检查点后三处修订（用户批准）。S1 §13.1 冷启动承诺改为「元数据 open <1s（vectors/stored 懒加载，M2 实测背书）；首次向量查询触发 vectors 加载 <3s」，消解 M1 实测 1573ms 未达 <1s 的遗留（SegmentReader OnceLock 懒加载，不改 §4 IDL 签名）。S2 §6.2 stored.bin 引入 per-file format_version（每文件独立递增，替代全局共用常量）+ v1(裸JSON)/v2(zstd) 双模读取（不做原地迁移）；补懒加载语义注释。S3 §14 I-5 释义澄清：`cfg(feature)` 能力开关（如 zstd-encode）允许出现在 segment 编解码处，`cfg(target)` 平台分支仍仅限 VFS/Executor。
- **v1.3**（2026-08-10）：M2-13 真实维基 nDCG corpus 落地后一处修订（用户批准）。S1 §13.2-2 ② 修订：真实中文维基 500 篇上 jieba-lite 相对 bigram nDCG@10 门禁从「提升 ≥15%」改为「不退步（≥0%，实测 +0.4%）」——bigram 在真实维基为强基线（nDCG≈0.93，数学上限≈7.5%），+15% 仅在合成边界陷阱语料可达（M1 实测 +84%，由代表性边界歧义 corpus 承载该硬门禁）；相对完整版 <2% 由 200 句 100% 切分一致性覆盖。
