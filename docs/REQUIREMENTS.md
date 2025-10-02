# Vane 需求明细结论（v1）

> 产出方式：三角色 Agent Team 研讨（检索引擎架构师 / 跨平台集成专家 / 产品与需求分析师）。
> 2026-08-09 v1 定稿（两轮）；v1.1 同日经第三轮定向复议纳入变更：**默认支持中文分词 + 自定义词表**。
> 本文档为 M0–M2 的需求合同，MoSCoW 清单即合同：任何新增需求必须挤掉同级旧需求。

---

## 0. 一句话定位

**一个 Rust 核心、四处可嵌（桌面 / Node / Go / 浏览器）的轻量级向量+BM25 混合检索库**
——sqlite-vec 的嵌入式形态 + Tantivy 级 BM25 + 一体化 RRF 混合排序。

差异化依据：浏览器端"向量+BM25 混合检索一体化、开箱即用"的成熟方案目前几乎空白
（唯一正面竞品 Orama 为纯 JS 实现，性能天花板低且无 Rust 核心供 Node/Go 复用）。

---

## 1. 目标用户与 P0 场景

| 优先级 | 场景 | 规模 | 说明 |
|---|---|---|---|
| P0-1 | AI Agent / 桌面 AI 应用的本地记忆库（Node/桌面） | 1万~50万条 | 外部 embedding API 产向量，每轮对话前 hybrid search 取上下文 |
| P0-2 | RAG 应用的本地/边缘检索层（Node+Go） | 数万~百万 chunk | 离线灌库、同进程检索、亚秒延迟、目录便于备份 |
| P0-3 | 浏览器端隐私语义搜索（WASM，PKM/笔记类 Web 应用） | ≤5万条（M2 验收边界） | **用户数据主权型正式产品**，非玩具 demo；OPFS 数据被驱逐构成产品事故，必须提供快照导出 |

目标用户是应用开发者，不是 infra 工程师：体验基准是 `npm install` / `go get` 即用。

---

## 2. 功能需求（MoSCoW）

### Must have
- Collection 管理（建/删/列）；文档 = 文本 + 向量 + JSON metadata
- 写入自动构建 BM25 倒排；向量索引：暴力扫描（M0）+ HNSW（M1）
- **分词器 collection 级可配置（v1.1 变更）**：内置 `standard`（unicode+lowercase+stemmer）/ `cjk_bigram`（零词典兜底）/ `jieba`（中文精确分词，**默认捆绑精简词典**）
- **自定义词表（v1.1 变更）**：所有分词器支持用户注入词条（可选词频），用户词条优先级高于内置词典、无条件胜出
- 混合搜索：vector-only / text-only / hybrid（**RRF 默认**，k=60）
- 目录多段文件持久化 + manifest 原子切换，崩溃不损坏；`export()` 单文件导出
- 增量写入与删除（tombstone + 段合并）
- 写入可见性语义：**显式 flush/commit 可见 + auto-commit 默认开启（1s 或 N 条触发）**，API 语义兼容未来 NRT
- Node（napi-rs）、Go（cgo staticlib）、浏览器（wasm-bindgen）三侧绑定（按里程碑分期交付）

### Should have
- Metadata 过滤（等值/范围，**pre-filter 位图进 HNSW 遍历**，拒绝 post-filter 为主）
- SQ8 向量量化（内存降 4 倍）
- 事务性批量写入（一次 commit 内可见性一致）
- 多字段文本（title/body 分开建 BM25，字段加权）
- 词表热更新 API：`setUserDict()`（暂存）+ `reindex()`（原子生效），见 §3.3
- jieba 完整词典（含词性标注）作为 native 可选 feature

### Could have
- mmap 只读打开模式（永不进入核心路径）
- 索引快照导出/导入
- 韩文/日文专用分词词典（当前中日共用词典，日韩切分质量不承诺）

### Won't have（明确划掉）
- ❌ 内置 embedding 生成（模型+tokenizer+推理运行时使包体积与维护面爆炸；examples 提供 OpenAI/ollama/transformers.js 5 行接入样板）
- ❌ GPU 加速 ❌ 分布式/副本/服务端模式 ❌ SQL 接口 ❌ 多用户/权限/网络协议

---

## 3. 核心架构决策（三方共识）

### 3.1 向量索引：分段 HNSW
- 每个 segment 一个独立 HNSW 图，段内不可变；删除 = tombstone 位图（roaring），段合并时新图从零重建（**图从不原地删除**，无需独立图重建机制，拒绝"删除>20% 提示用户重建"的运维话术）
- 多段并行搜索后归并；段数硬上限 ~10，超限强制合并（小段 <1万文档优先合并，合并不阻塞读）
- 暴力扫描保留为**自适应回退**：过滤后候选集 < 2×k 时切换暴力精确扫描
- 实现路线：基于 instant-distance fork 或自研（~800 行）；拒绝 hnsw_rs（绑 rayon、不便序列化）、usearch（C++ 内核进不了 WASM）、IVF-PQ（需训练、PQ 重排序依赖随机读）、DiskANN（OPFS 延迟模型不适用）

### 3.2 BM25：Block-Max WAND + 可插拔分词（v1.1 重写）
- posting = (docid_delta, tf) 变长编码 + 128-doc 跳块，块内 max score 支持 Block-Max WAND top-k
- Tokenizer 全可插拔 trait，collection 级配置，三内置：`standard` / `cjk_bigram` / `jieba`
- **jieba 实现路线（v1.1）**：保留 jieba 算法内核（前缀 DAG + HMM 未登录词识别），**词典重做**——从开源词表剪枝保留 ~20 万高频词，双数组 Trie 序列化 + zstd 压缩后约 1–1.5MB；只许裁剪词典，不许改动算法（验收：与 jieba-rs 原版同词典切分 100% 一致）
- **中英混排**：按 unicode script 边界切 run，CJK run 走 jieba，Latin/digit run 走 lowercase+stemmer，position 全程连续保证跨语言 phrase query 正确
- 借鉴 Tantivy 段设计但不用其本体

### 3.3 分词器一致性与词表变更语义（v1.1 新增，仲裁结论）
- 分词器身份 = `(算法版本, 内置词典版本, 用户词表 SHA256)` 三元组，写入 collection 元数据与每个段头部；查询期校验，不匹配拒绝查询并提示 reindex
- **词表变更"暂存不生效"语义**（架构师与产品方冲突的仲裁）：`setUserDict()` 仅登记新词表，collection 进入 `needsReindex` 状态；**新写入仍用旧分词器**，全库任意时刻只有一套分词身份，杜绝新旧段混排导致的静默错误；`reindex()` 显式触发全量重建（复用段合并管线，后台增量执行，旧段全程只读服务，完成后原子切换），新词表此时才生效
- 明确拒绝：①"仅影响新段"的惰性方案（检索结果静默错误）；②自动全量重建（50 万文档分钟级阻塞是事故）；③按词表版本查询期合并（精度玄学化）
- 内置词典独立**日历版本化**（如 `2026.08`），与库 semver 解耦；词典升级打开老库仅警告不强制重建，CHANGELOG 必须标注"影响检索结果"

### 3.4 混合融合：RRF 默认
- 默认 `fusion: "rrf"`（k=60，零校准成本）；`fusion: { linear: alpha }` 仅作显式选项（要求调用方指定 min-max 归一化）
- **API 默认路径不出现 alpha 参数**（RRF 不可调权重，避免概念错位）
- CI 硬门禁：**hybrid recall@10 ≥ 0.95**，口径为相对"暴力双路 + RRF"基线；召回回归测试覆盖 0.1%~99% 过滤选择率

### 3.5 存储层：VFS trait 是架构脊柱
- `trait Vfs { create / read_at / write_at / append / sync / rename / delete / list }` 同步接口 + LRU 页缓存（默认 32MB）；`rename` 是 manifest 原子切换的唯一原语
- **整个引擎零 mmap 依赖**（native 首版也不用）——显式 read 进页缓存，换全平台同一代码路径；mmap 只读模式仅作 Could-have 附加
- 四后端：std fs（native）/ OPFS SyncAccessHandle（WASM Worker 主）/ IndexedDB（WASM 降级，适配层放 binding crate 不污染 core）/ Memory
- 文件组织：目录多段 + manifest.json 版本指针；WAL 做薄（仅段添加/删除/tombstone 元操作日志）；所有段文件带 version header，格式变更必须写迁移器
- **VFS trait 签名 M0 冻结**——它是 Go cgo、WASM、IndexedDB 降级三方的共同接缝

### 3.6 并发模型
- 不可变段 + **单写者 + 无锁读**（Arc swap 段快照）；写并发需求为零
- `trait Executor` 抽象并行：native = rayon，WASM = 串行；核心算法零 `cfg` 污染
- 段合并抽象为**可切片增量任务**：native 后台跑，WASM 在写间隙小步推进，同一状态机两平台复用
- 拒绝 SharedArrayBuffer 多线程 WASM（COOP/COEP 部署要求转嫁用户）

---

## 4. 跨平台与集成决策（三方共识）

### 4.1 WASM / 浏览器
- 唯一目标 `wasm32-unknown-unknown`，不引入 wasi；core 不直接依赖 `std::fs/std::net`
- **core 保持同步 IO**：SyncAccessHandle 在 Worker 内部本步同步；异步性只存在于"主页面 ↔ Worker"postMessage 边界，由 JS 壳层包成 Promise——不为浏览器把 core 异步化
- 持久化：OPFS 主（强制 Dedicated Worker 架构）+ IndexedDB 降级；暴露 `persistence: 'persistent' | 'best-effort'`，调用 `navigator.storage.persist()`；文档声明"浏览器存储非可靠存储，关键数据用 `export()` 快照导出"
- SIMD128 双变体构建（wasm 无法运行时分支，init 时 `WebAssembly.validate` 探针选产物，用户只下载其一）；召回回归测试两变体各跑一遍
- **体积门禁（v1.1 修订）**：核心引擎 gzip ≤ 800KB（CI 硬红线，口径 = **含 jieba 算法代码、不含词典数据**）｜全功能变体（stemmer+SQ8）≤ 1.2MB；**词典永不打进 wasm 产物**，独立 fetch（默认 CDN URL + sha256 校验 + OPFS 缓存，二次启动零网络）；词典资源单列预算（任一渠道 ≤ 2MB gzip）；500KB 为 M2 优化目标而非门禁
- **中文降级行为**：WASM 侧词典 fetch 失败时自动降级 `cjk_bigram` 并 console.warn，**不抛错**；离线场景支持内联 Buffer 注入词典
- 依赖黑名单：regex、tokio 全套、prost/tonic、openssl、lindera（>10MB 词典格式）、ndarray、wee_alloc（已停维护，用 dlmalloc/lol_alloc）
- CI 门禁从 M0 第一天生效：`cargo check --target wasm32-unknown-unknown -p vane-core`，core 出现 std::fs 即构建失败

### 4.2 Node.js
- napi-rs（N-API v6+），**直连 core 不过 C ABI**；拒绝 Neon（V8 ABI）与直接加载 WASM
- 异步桥接：napi `AsyncTask`/`ThreadsafeFunction` + core 内部线程池，不桥接 tokio
- 并发模型：**每进程单 DB 实例、多查询并发**为主场景；句柄注册表粗粒度 RwLock 即可
- 分发：npm 主包 + 分平台 optionalDependencies 子包；主包内运行时 fallback 加载；CI 跑 npm/yarn/pnpm/bun 四包管理器安装矩阵
- **中文词典分发（v1.1）**：独立平台无关数据包 `@vane/dict-zh`（仅含预编译 `dict.bin`）作主包**正式 dependency**，随装随有、离线可用；**拒绝 postinstall 下载**（pnpm 禁脚本/企业断网/`--ignore-scripts` 静默失效）；另提供 `@vane/slim`（无词典依赖，中文退化 bigram 并文档明示）
- **M0 预编译产物 4 个**：`linux-x64-gnu`、`darwin-arm64`、`darwin-x64`、`win32-x64-msvc`（P0-1 是桌面场景，不能没有 Windows）；musl/linux-arm64 顺延 M1；永不要求用户从源码构建

### 4.3 Go
- **cgo + staticlib 为一等公民**：CI 预编译全平台 `.a`（Linux 交叉用 zig cc），Go 模块按 GOOS/GOARCH 嵌入；性能最优、部署形态符合 Go 用户直觉
- **中文词典分发（v1.1）**：`go:embed` 内嵌 `dict.bin.gz`（+1.5MB 换单文件部署体验不被破坏）；提供 `//go:build vane_nodict` 裁剪 tag（退化 bigram）；embed 天然钉版，`vane.DictVersion()` 可查
- **wazero 为文档化的二等备选**（仲裁结论：检索主路径 wazero 劣化 2~4 倍、无 SIMD 时 3~5 倍，不满足一等公民性能门槛；且 core 的 wasm 测试矩阵已被浏览器路径占用）
  - 同一 Go API，build tag 切换（`-tags wazero`）；不承诺与 cgo 版版本同步
  - `CGO_ENABLED=0` 时给清晰编译错误 + 指向 wazero 包，不做静默降级
  - wazero 形态把 wasm32 target 提前到 M1 进 CI 发布流（仅 CI target，非浏览器交付物）

### 4.4 FFI 层（C ABI 约定，cbindgen）
- 句柄：`uint64_t` 不透明句柄 + 全局注册表（规避 cgo pointer 规则、防 use-after-free），拒绝裸指针
- 错误：`int32_t` 状态码 + `vane_last_error_message()` 拉取详情
- 内存铁律：**谁分配谁释放，跨边界只借不还**；批量结果用回调或 arena 一次性 free
- 三层结构：core（干净 Rust API）/ vane-ffi（C ABI → Go）/ vane-node（napi-rs 直连）

### 4.5 Workspace 与发布
```
vane/
├── crates/{vane-core, vane-ffi, vane-node, vane-wasm}
├── bindings/{go, node}
```
core 不依赖任何 binding crate，feature 划分 `std`/`wasm`；单 CI workflow 矩阵全平台；三端版本号严格同步（crates.io + npm + Go module proxy）

---

## 5. API 形态

极简 JSON 文档风格，6 动词，拒绝 SQL 与链式 builder。三侧同名函数 + 同构参数 + 同错误码枚举；binding 层是无逻辑薄壳，行为测试全部跑在 Rust 核心上；**binding 契约 M0 冻结**（共享一份 IDL 式定义）。

```js
const db  = await Vane.open("./mydb", { persistence: "persistent" });
const col = db.collection("docs", {
  dim: 384,
  tokenizer: "jieba",            // v1.1: "standard" | "cjk_bigram" | "jieba"
  userDict: ["布地奈德", { term: "PD-1抑制剂", freq: 100 }],  // v1.1: 可选自定义词表
});
await col.add([{ id, text, vector, meta }]);            // 批量幂等 upsert；默认 auto-commit
await col.flush();                                       // 显式可见性边界
await col.search({ text, vector, topK: 10,
                   mode: "hybrid",                       // "vector" | "text" | "hybrid"
                   fusion: "rrf",                        // "rrf"(默认) | { linear: 0.5 }
                   filter: { lang: "zh" } });
await col.setUserDict(["新词"]);                         // v1.1: 暂存，needsReindex=true，不即时生效
await col.reindex();                                     // v1.1: 显式重建，新词表原子生效（后台增量，旧段只读服务）
await col.delete(["id1"]);  await col.compact();         // 手动合并入口
await db.export("./backup.vane");  await db.close();
```
签名映射规定死：JS `await search(...)` ⇔ Go `Search(...) (Result, error)`，语义等价。

---

## 6. 非功能需求（量化指标）

| 指标 | 承诺值 | 备注 |
|---|---|---|
| 数据规模 | M0/M1：10 万优化目标、**50 万不塌红线**；M2：100 万 | 浏览器端 M2 验收边界 5 万（架构按 50 万设计，代码不写死 5 万假设） |
| 查询延迟 | 10万×384维 hybrid topK=10 P99 < 50ms（HNSW）/ < 150ms（暴力）；WASM 端放宽 3~5 倍 | WASM 具体数值 M2 前出实测预算表 |
| 召回率 | hybrid recall@10 ≥ 0.95（相对暴力双路+RRF 基线），CI 硬门禁 | 覆盖 0.1%~99% 过滤选择率 |
| 内存 | 10万×384维全加载 < 500MB；SQ8 后 < 200MB | 向量 154MB + 图 ~60MB + 倒排 |
| WASM 体积 | 核心 ≤800KB gzip（红线，含 jieba 算法代码、**不含词典数据**）；全功能 ≤1.2MB；词典资源任一渠道 ≤2MB gzip | 500KB 为 M2 优化目标；词典永不进 wasm 产物 |
| 中文词典 | 精简词典 ~20 万词，DAT+zstd ≤1.5MB gzip；预编译 `dict.bin` 冷加载 <150ms（零拷贝反序列化） | 自定义词表合并走 sha256 两级缓存 |
| 启动 | 打开 10 万文档库 < 1s（**待 M1 冷启动实测背书**；若 >2s 则降级为分级指标：元数据 <1s、首次查询 <3s） | 无 mmap 换全平台同路径的代价项 |
| 写入吞吐 | 批量 add ≥ 5k docs/s；持续小批量场景注明"建议攒批 ≥100 条" | WASM 端合并切片粒度待 benchmark |
| 可见性 | flush 后对新读快照原子可见（双索引同快照出现）；auto-commit 默认 1s | API 语义兼容未来 NRT |

---

## 7. 里程碑

- **M0（4~6 周，最小闭环）**：Rust 核心（暴力向量 + BM25 + RRF + 持久化 + flush 语义）+ Node napi 绑定（4 平台预编译）。分词器：实现 `standard` + `cjk_bigram`，**`tokenizer` 字段与 API 占位从第一天存在**（v1.1），jieba 不进 M0（词典三侧分发要动 CI 流水线，塞入会两线起火）。VFS trait / CI wasm32 门禁 / benchmark CI / binding 契约全部从第一天建立。**浏览器仅 CI 内部目标，不对外交付**。Demo：1 万条维基摘要（英文语料），hybrid / vector-only / text-only 三列排序对比 + 对比 sqlite-vec+FTS5 手写方案的代码量。
- **M1**：HNSW（分段）、删除 tombstone + 合并、metadata pre-filter、Go cgo 绑定（wazero 备选进 CI）、薄 WAL 崩溃恢复；**v1.1 新增：`jieba` 分词器 + 精简词典 + 自定义词表 + `setUserDict`/`reindex` + Node/Go 两侧词典分发打通**。验收：recall ≥0.95 回归、冷启动实测背书、中文分词四项验收（切分与 jieba 原版 100% 一致；精简词典相对完整版 nDCG@10 差异 <2%、相对 bigram 提升 ≥15%；自定义词表生造词单 token 入索引、短语命中 100%；缺词典自动降级 bigram 不抛错）。若燃尽图告急，**Go 绑定允许后移——分词是用户点名的 Must，保用户承诺优先**。
- **M2**：浏览器交付（OPFS 主 + IndexedDB 降级 + Worker 壳 + SIMD 双变体）、WASM 侧词典 CDN fetch + OPFS 缓存、SQ8、快照导出、100 万规模承诺恢复、jieba 完整词典（native feature）。Demo：纯前端页面拖入 markdown 文件夹本地混合搜索（含中文）。

开源协议：**Apache-2.0**（专利授权条款，企业嵌入法务阻力最小）。examples 覆盖 OpenAI / ollama / transformers.js 三种 embedding 接入。

---

## 8. TOP 风险登记册（合并三方）

| # | 风险 | 规避 |
|---|---|---|
| 1 | 范围蔓延（最高危） | MoSCoW 即合同；新增需求必须挤掉同级旧需求 |
| 2 | 相对 sqlite-vec+FTS5 组合无优势 | M0 demo 三列排序 + 代码量对比；主打原子混合排序与统一 filter 语义 |
| 3 | 浏览器持久化做不出/被驱逐 | OPFS 抽象 M0 先行；M2 才对外交付；`export()` 快照 + persistence API |
| 4 | WASM 体积失控 | 三级门禁 + 依赖黑名单 + cargo-deny + cargo bloat 周报 |
| 5 | 性能/召回不达标 | benchmark CI 从 M0 建；recall@10 ≥0.95 硬门禁；指标按平台分级 |
| 6 | cgo 交叉编译用户侧爆炸 | 全平台预编译 `.a` + zig cc + CGO_ENABLED=0 清晰报错引导 wazero |
| 7 | 低选择率过滤召回崩溃 | pre-filter 位图进遍历 + 候选集过小暴力回退，双保险 |
| 8 | 文件格式演进断裂 | version header + 冻结 corpus 兼容测试 + 强制迁移器；M0/M1 预留 segment 元数据扩展位 |
| 9 | embedding 不内置的"最后一公里"差评 | examples 5 行接入样板；M2 可选独立集成 crate，核心永不内置 |
| 10 | OPFS 语义陷阱（仅 Worker/Safari 配额） | 文档强制 Worker 架构；Memory+OPFS 双后端同一测试套件 M0 起 |
| 11 | WASM 词典 fetch 失败，中文开箱体验崩（v1.1） | bigram 自动降级不抛错；支持内联 Buffer 自托管词典 |
| 12 | 词表变更后用户感知"搜索坏了"（v1.1） | 暂存不生效 + needsReindex 状态 + 显式 reindex；文档"词表最佳实践"（建库前收齐术语，宜稳不宜勤改） |
| 13 | 精简词典领域切分劣化（古文/方言/长尾专有词）口碑风险（v1.1） | HMM 未登录词兜底 + 自定义词表官方补救通道 + 完整词典 feature；5 万词 benchmark 对全量词典 nDCG 差 >2% 则放宽剪枝阈值 |
| 14 | 词典膨胀借道偷渡（v1.1） | CI 独立门禁：`@vane/dict-zh` 包体积、`dict.bin` 尺寸、Go embed 二进制增量（<2MB）三线分卡 |
| 15 | M1 范围膨胀（分词+HNSW+删除+Go 绑定同 milestone）（v1.1） | 降级顺序预设：Go 绑定可后移，分词 Must 不让位 |

---

## 9. 研讨未完全收敛项（裁决/待实测）

| 议题 | 状态 |
|---|---|
| WASM 体积红线 | **已裁决**：核心 800KB 红线维持（v1.1 口径修订：含分词器代码、不含词典数据）；词典资源单列预算 ≤2MB/渠道；原"2MB 天花板"档取消 |
| Go wazero 定位 | **已裁决**：二等备选（产品方坚持，集成专家的量化数据 2~4 倍劣化支持此结论） |
| M0 预编译产物数 | **已裁决**：4 个含 win32-x64（集成专家坚持，P0-1 桌面场景论据成立） |
| `alpha` 参数形态 | **已裁决**：默认路径无 alpha，`fusion: "rrf" \| { linear: α }` |
| 词表变更语义（v1.1 第 3 轮冲突） | **已裁决（仲裁）**：暂存不生效 + 显式 reindex 原子切换——架构师的"杜绝静默不一致"与产品方的"拒绝自动重建/不炸存量"同时满足 |
| jieba 进 M0 还是 M1（v1.1） | **已裁决**：M1（词典三侧分发动 CI 流水线，M0 塞入两线起火；M0 保留 API 占位 + bigram 顶着） |
| WASM 词典进不进主产物（v1.1） | **已裁决**：永不进。架构师"红线须升 2MB"的前提（词典打进 wasm）被集成专家的外置方案消解，800KB 红线不降不升 |
| 冷启动 <1s 指标 | 待 M1 实测背书，可能降级为分级指标 |
| 持续小批量写入的合并停顿 | 待 M1 benchmark；需求侧已下调承诺（建议攒批 ≥100 条） |
| WASM 首次查询延迟预算 | 待 M2 前出实测预算表，当前"放宽 3~5 倍"为暂估值 |
| 段合并切片粒度 | 待 benchmark 数据，M1 开工前验证项 |
