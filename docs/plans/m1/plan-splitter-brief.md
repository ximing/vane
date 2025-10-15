# 阶段一：M1 计划拆分（plan-splitter 派发简报）

> 这是你（plan-splitter SubAgent）的需求文档。你是软件架构师，产出 M1 的实现计划集。
> 产出目录：`docs/plans/m1/`（已存在 EXECUTION-NOTES.md / 00-cleanup.md / 本文件，不要覆盖）。

## 背景

Vane 是 Rust 核心、四处可嵌（桌面/Node/Go/浏览器）的向量+BM25 混合检索库。M0 已完成（commit 538db51）：暴力向量+BM25(Block-Max WAND)+RRF/linear+持久化+flush 语义+Node napi 绑定+standard/cjk_bigram 分词+VFS trait+wasm32 CI 门禁+benchmark CI+demo。204 测试全绿。

本任务把 SPEC §15 的 M1 范围拆成若干独立、可 TDD 执行的实现计划。

## 必读（开工前通读）

- `docs/REQUIREMENTS.md` v1.1（需求合同，§2 MoSCoW、§3 架构决策、§7 里程碑、§8 风险登记册）
- `docs/SPEC.md` v1.0（技术规范——§3.1/§3.3/§4 IDL/§5 分词器/§6 存储格式/§7.4 词表状态机/§8 查询/§9 FFI/§10 错误码/§11 并发/§12 分发/§13 门禁/§14 不变量/§15 里程碑对照）
- `docs/plans/m0/README.md`（**M0 Global Interface Contracts——M0 既有 pub API 的单一事实源**。M1 计划必须核查与这些签名的真实对接）
- `docs/plans/m0/M0-SUMMARY.md`（M0 交付清单/遗留问题/M1 建议）
- `docs/plans/m0/EXECUTION-NOTES.md`（M0 遗留 FF1~FF6 + parked 项；阶段零已处理 FF1/FF2/FF3/corpus-test/FF6）
- **M0 实际代码**（`crates/vane-core/src/`）——以 git 上 M0 实际 pub API 为准，不假设。逐模块 Read 确认签名。

## M1 范围（SPEC §15 + REQUIREMENTS §7）

1. **分段 HNSW**：段内不可变图，多段并行搜索归并；暴力扫描作自适应回退（过滤候选<2×k）；段数硬上限 10，超限强制合并；图从不原地删（I-3），段合并时新图从零重建。实现路线：instant-distance fork 或自研（~800 行）。
2. **删除 tombstone + 段合并**：roaring tombstone（M0 header.bin 已预留 tombstone 字段）；段合并可切片增量任务（native 后台 / WASM 写间隙小步）；compact() 实装。
3. **metadata pre-filter**：位图进 HNSW 遍历 + WAND 推进；低选择率（位图基数<2×topK）暴力回退。delete/compact/Filter 实装（M0 占位 E_UNSUPPORTED）。
4. **薄 WAL 崩溃恢复**：仅段增删/tombstone 元操作日志（SPEC §6.4）。
5. **jieba-lite 分词**：jieba 算法内核（前缀 DAG + HMM）+ 精简词典 ~20 万词（DAT + zstd，≤1.5MB gzip）；算法与 jieba-rs 完全一致仅裁剪词典；中英混排按 script 边界切 run。
6. **自定义词表 + setUserDict/reindex 状态机**（SPEC §7.4）：Stable→PendingReindex→Rebuilding→Stable；暂存不生效；reindex 复用段合并管线后台增量；旧段只读服务；原子切换。
7. **词典分发**：Node `@vane/dict-zh` 平台无关数据包作主包正式 dependency（禁 postinstall，≤1.5MB gzip CI 门禁）；Go `go:embed dict.bin.gz`（<2MB CI 门禁，`//go:build vane_nodict` tag）；词典独立日历版本化，三渠道版本哈希一致才发版。
8. **Go cgo 绑定**：staticlib + zig cc 全平台预编译 .a；`CGO_ENABLED=0` 清晰报错引导 wazero；wazero 二等备选同 API build tag 切换。**若燃尽图告急可后移——分词 Must 不让位（REQUIREMENTS §7 风险 #15）。**
9. **冷启动实测背书**：打开 10 万库 <1s；>2s 降级分级指标（元数据<1s、首次查询<3s）。
10. **recall@10≥0.95 真实回归**：相对"暴力双路+RRF"基线，CI 硬门禁；五档选择率（0.1%/1%/10%/50%/99%）。

## 关键验收锚点（计划须编码进验收标准）

- **中文分词四项验收**（SPEC §13.2-2）：① 200 句与 jieba-rs 原版切分 100% 一致；② 中文维基 500 篇+50 查询，jieba-lite 相对完整版 nDCG@10 差<2%、相对 bigram 提升≥15%；③ 20 生造词注入 userDict 后单 token 入索引、短语命中 100%；④ 缺词典自动降级 bigram + console.warn 不抛错（WASM 侧 E_DICT_UNAVAILABLE 禁止到达）。
- **词表状态机**（§7.4）：禁止新旧分词身份混排检索、自动全量重建、查询期多版本合并。
- **体积门禁**（§13.2-3）：核心 wasm gzip ≤800KB（含 jieba 代码、不含词典）；`@vane/dict-zh` ≤1.5MB；Go embed <2MB。
- **不变量 I-1~I-8** 全覆盖（I-3 图不原地删、I-4 单一分词身份在 M1 有真实测试）。

## 产出要求

### 1. README 索引：`docs/plans/m1/README.md`
- 计划文件清单（# / 文件 / 一句话摘要 / 产出模块）。
- 依赖图（mermaid）+ 拓扑批次（标注可并行批次）。
- **M1 Global Interface Contracts**：M1 新增/扩展的跨计划类型签名（单一事实源），与 M0 既有契约（见 m0/README.md）衔接。任何 M1 计划消费 M0 pub API 须引用 m0/README 的精确签名。
- M1 范围边界（实现 / 仅 API 占位 / 不实现）。
- 全局约束表（同 M0，含 M1 新增：词典永不进 wasm、jieba 算法不动只裁词典等）。
- 不变量覆盖矩阵（I-1~I-8，标注 M1 负责计划与测试要求）。

### 2. 各计划文件：`docs/plans/m1/<NN>-<name>.md`
建议拆分（你可按依赖调整，但须覆盖全部 M1 范围，不得漏）：
- `01-hnsw.md`（分段 HNSW 图 + 段级搜索归并 + 暴力自适应回退）
- `02-tombstone-merge.md`（delete tombstone + 段合并 + compact 实装）
- `03-pre-filter.md`（metadata 过滤位图进 HNSW+WAND + 低选择率暴力回退）
- `04-wal.md`（薄 WAL 元操作日志 + 崩溃恢复）
- `05-jieba-lite.md`（jieba 算法内核 + 精简词典 DAT+zstd + 中英混排）
- `06-userdict-reindex.md`（自定义词表 + setUserDict/reindex 状态机 §7.4）
- `07-dict-distribution-node.md`（@vane/dict-zh 数据包 + 主包 dependency + 体积门禁）
- `08-dict-distribution-go.md`（go:embed dict.bin.gz + vane_nodict tag + DictVersion）
- `09-go-cgo-binding.md`（vane-ffi cbindgen + Go cgo staticlib + zig cc 交叉 + wazero build tag）——标注可后移
- `10-ci-m1.md`（M1 CI 门禁扩展：recall 真实回归 job、wasm 体积门禁、词典体积门禁、Go 交叉编译矩阵、冷启动 bench）
- `11-cold-start-bench.md`（冷启动 <1s 实测背书 + 分级降级指标）
- `12-recall-regression.md`（recall@10≥0.95 五档选择率回归 job，HNSW vs 暴力双路+RRF 基线）

每份计划须含（参照 M0 计划格式）：
- **Goal / Architecture / 涉及文件**（Create/Modify/Test 精确路径）。
- **Interfaces**：Consumes（消费的上游计划/M0 API 精确签名）/ Produces（下游依赖的精确签名）。
- **TDD 任务清单**：bite-sized 步骤（写失败测试→验证失败→最小实现→验证通过→commit），含真实测试代码与实现代码（**禁止 placeholder**：无 TBD/TODO/"适当处理"）。
- **验收标准**：编码上述验收锚点（SPEC 引用编号）。
- **前置依赖**：标注 blockedBy 哪些计划。
- **Global Constraints**：引用 M1 全局约束。

### 3. 自审
- SPEC 覆盖：每个 §15 M1 交付项有对应计划。
- Placeholder 扫描：无 TBD/TODO/缺测试代码。
- 类型一致性：跨计划签名与 M0 README 契约一致（如 SegmentReader/SegmentWriter/brute_search/InvertedIndexReader/Collection API 的扩展点）。
- 与 M0 占位的对接：M0 `delete/compact/reindex/export` 返回 E_UNSUPPORTED，M1 计划须实装这些（reindex 返回 ReindexHandle）。

## 红线

- MoSCoW 即合同：不得新增需求；Won't-have（内置 embedding/GPU/SQL/分布式）不得触碰。
- 词典永不打进 wasm 产物（核心红线 800KB gzip，含 jieba 代码、不含词典数据）。
- core 禁 std::fs/std::net/mmap；cfg 只在 VFS/Executor；不引入 dashmap/parking_lot/黑名单依赖。
- jieba 算法与 jieba-rs 完全一致，只裁词典——不得发明新切分规则。
- 不得改 M0 已冻结的 pub API 签名（Vfs trait、Schema、brute_search 等）；只能扩展。若 M1 必须改 M0 签名，标记为 SPEC 修订提议交编排者，不得自行改。
- 全程中文。

## 报告

完成后把计划集摘要写入 `docs/plans/m1/plan-split-report.md`：计划清单、依赖图（拓扑批次）、与 M0 API 对接核查结果、任何 SPEC 漏洞/矛盾/需裁决项。最终回复只返回：状态、计划文件列表、依赖拓扑一句话、需编排者裁决的疑点。
