# M1 计划拆分报告

> 产出日期：2026-08-09
> 拆分者：plan-splitter SubAgent
> 依据：`docs/plans/m1/plan-splitter-brief.md` + `docs/SPEC.md` v1.0 + `docs/REQUIREMENTS.md` v1.1 + M0 git HEAD 实际代码
> 产出：`docs/plans/m1/README.md` + `docs/plans/m1/modules/01..12-*.md`（12 份）

---

## 1. 计划清单

| # | 文件 | 一句话摘要 | 批次 |
|---|---|---|---|
| 01 | `modules/01-hnsw.md` | 段内不可变 HNSW 图 + 段级搜索归并 + 暴力自适应回退 | L0 |
| 02 | `modules/02-tombstone-merge.md` | delete tombstone + 段合并 + compact() | L1 |
| 03 | `modules/03-pre-filter.md` | metadata 过滤位图进 HNSW+WAND + 低选择率暴力回退 + scalars.col | L2 |
| 04 | `modules/04-wal.md` | 薄 WAL 元操作日志 + 崩溃恢复 | L2 |
| 05 | `modules/05-jieba-lite.md` | jieba DAG+HMM + 精简词典 DAT+zstd + 中英混排 | L0 |
| 06 | `modules/06-userdict-reindex.md` | setUserDict + reindex 状态机 + ReindexHandle | L2 |
| 07 | `modules/07-dict-distribution-node.md` | @vane/dict-zh 数据包 + 主包 dependency + 体积门禁 | L1 |
| 08 | `modules/08-dict-distribution-go.md` | go:embed + vane_nodict tag + DictVersion | L2 |
| 09 | `modules/09-go-cgo-binding.md` | vane-ffi C ABI + Go cgo staticlib + zig cc + wazero（**可后移**） | L0 |
| 10 | `modules/10-ci-m1.md` | M1 CI 门禁扩展（recall/wasm 体积/词典体积/Go 矩阵/冷启动） | L4 |
| 11 | `modules/11-cold-start-bench.md` | 冷启动 <1s 实测背书 + 分级降级指标 | L3 |
| 12 | `modules/12-recall-regression.md` | recall@10≥0.95 五档选择率回归 job | L3 |

---

## 2. 依赖拓扑（一句话）

L0（01-hnsw / 05-jieba-lite / 09-go-cgo-binding 三路并行）→ L1（02-tombstone-merge / 07-dict-node 两路）→ L2（03-pre-filter / 04-wal / 06-userdict-reindex / 08-dict-go 四路）→ L3（11-cold-start / 12-recall 两路）→ L4（10-ci-m1 收尾）；最大并行度 4 路（L2）。

完整依赖图见 `docs/plans/m1/README.md` mermaid。

---

## 3. 与 M0 API 对接核查结果

逐模块 Read 了 `crates/vane-core/src/` 全部文件（types.rs / vfs/mod.rs / tokenizer/{mod,id,standard,cjk_bigram}.rs / segment/{mod,header}.rs / bm25.rs / vector/mod.rs / persistence/mod.rs / api/{types,db,collection,mod}.rs）+ `crates/vane-node/src/` + `crates/vane-ffi/src/lib.rs` + `.github/workflows/` + `deny.toml` + workspace `Cargo.toml`。核查结论：

### 3.1 M0 冻结签名对接无误

- **Vfs trait**（8 方法）：04-wal、01-hnsw、02-tombstone-merge 均经 Vfs 读写，不改签名。
- **Schema / brute_search / InvertedIndexReader::search(filter)**：03-pre-filter 直接复用 M0 已支持的 filter 参数（brute_search / InvertedIndexReader::search 均已接 `Option<&RoaringBitmap>`），无需改 M0。
- **SegmentReader / SegmentWriter**：02/03 复用 open/add_doc/finalize；scalars.col 通过**新增** `SegmentWriter::set_scalar` 扩展（不改 add_doc 签名）；ScalarReader 新增类型。
- **SegmentMeta.tombstones: RoaringBitmap**：M0 已预留（header.bin 已含 tombstone 字段，M0 写空），02-tombstone-merge 直接用。
- **ManifestStore / CollectionMeta**：02/04/06 复用 save_atomic/add_segment/load。
- **compute_tokenizer_id**：05-jieba-lite 不改其公开签名，JiebaTokenizer 内部二次哈希叠加词典版本（方案 A）。

### 3.2 M0 占位对接

| M0 占位 | M1 实装计划 | 状态 |
|---|---|---|
| `Collection::delete` → E_UNSUPPORTED | 02-tombstone-merge Task 1 | 实装 |
| `Collection::compact` → E_UNSUPPORTED | 02-tombstone-merge Task 5 | 实装 |
| `Collection::reindex` → E_UNSUPPORTED (`Result<()>`) | 06-userdict-reindex Task 2 | **签名变更**（见 R-2） |
| `Db::export` → E_UNSUPPORTED | — | **保留占位**（见 R-1） |
| `build_tokenizer(Jieba)` → DictUnavailable | 05-jieba-lite（新增 `build_jieba_tokenizer` 工厂，不改 build_tokenizer） | 实装 |
| vane-ffi `src/lib.rs` 空占位 | 09-go-cgo-binding | 实装 |

### 3.3 Node 绑定扩展点

- `VaneCollection` 需增 `compact()` napi 方法（M0 未暴露）。
- `reindex` 返回值从 `AsyncTask<ReindexTask>` (Output=()) 改为返回 `VaneReindexHandle` napi struct。
- `convert.rs::parse_search_query` 移除 M0 filter reject（`filter not supported in M0`），改为解析 filter。
- `convert.rs::parse_collection_opts` 增 jieba 词典自动加载 + 降级逻辑。

---

## 4. 需编排者裁决的疑点

### R-1：export() 归属 M1 还是 M2（SPEC 矛盾）

- **brief 原文**："M0 占位待 M1 实装：delete/compact/reindex/export"——将 export 列入 M1。
- **SPEC §15 M2 行**："export 快照导出"明确列入 M2。
- **REQUIREMENTS §2 Must have**："export() 单文件导出"——Must 但未标里程碑。
- **裁决建议**：以 SPEC §15 为准（SPEC 是技术规范单一事实源），export 保留 M0 占位（E_UNSUPPORTED），M2 实装。M1 计划集不覆盖 export 实装。若编排者坚持 brief 口径，需新增 `13-export.md` 计划（但会扩大 M1 范围，与风险 #15 冲突）。
- **当前处理**：M1 README 将 export 列入"仅 API 占位"区，不实装。

### R-2：reindex() 签名变更（M0 占位 → SPEC IDL 落实）

- **M0 实际**：`pub fn reindex(&self) -> Result<()>`（占位，M0 README 标注 "ReindexHandle 留 M1"）。
- **SPEC §4.1 冻结 IDL**：`Collection.reindex() -> Result<ReindexHandle>`（§4 注：本约签名 M0 冻结）。
- **分析**：M0 的 `Result<()>` 是对 SPEC IDL 的临时偏离（占位），M1 落实为 `Result<ReindexHandle>` 是**回归 SPEC 冻结签名**，非破坏 M0 冻结签名。M0 README 已明确预留此变更。
- **裁决建议**：批准 06 计划 Task 2 的签名变更（`Result<()>` → `Result<ReindexHandle>`），同步 Node 绑定（ReindexTask Output 改 ReindexHandle + 新增 VaneReindexHandle napi struct）。不视为红线违反。
- **影响面**：api/collection.rs、crates/vane-node/src/collection.rs、crates/vane-ffi（vane_reindex 返回 handle）。

### R-3：TokenizerId 词典版本注入方式

- **M0**：`compute_tokenizer_id(kind, user_dict)` 不接收词典实例；`builtin_dict_version(Jieba)` 返回 `b""`（占位）。
- **SPEC §5.4**：TokenizerId = sha256(algorithm_version ‖ **builtin_dict_version** ‖ user_dict_bytes)，jieba 的 builtin_dict_version 应含词典日历版本。
- **方案 A（推荐，不改 M0 签名）**：`JiebaTokenizer::new` 内部在 `compute_tokenizer_id` 之上二次哈希叠加 `dict.version() + dict.sha256_prefix()`。`compute_tokenizer_id` 公开签名不变。
- **方案 B（否决）**：改 `compute_tokenizer_id` 签名——破坏 M0 冻结。
- **裁决建议**：批准方案 A。05 计划 Task 7 已采用。

### R-4：rayon 依赖（Executor 抽象）

- **SPEC §11**：native Executor = rayon；wasm = 串行。
- **现状**：M0 无 Executor，无 rayon 依赖。deny.toml 黑名单不含 rayon（允许）。
- **M1 决策**：01-hnsw 计划用 `std::thread::scope`（native 并行）+ 串行（wasm）替代 rayon，避免引入依赖 + cfg 复杂度。02-tombstone-merge 的 MergeTask 同样用同步执行（M1 先同步，后台化留 Executor 扩展点）。
- **裁决建议**：批准 M1 不引入 rayon，用 std::thread::scope。若 M2 需更优并行再评估 rayon。SPEC §11 的 "rayon" 是建议非硬约束（"native 实现 = rayon" 是实现路线描述）。

### R-5：stored.bin zstd 压缩归属

- **M0 SUMMARY**："stored.bin 未做 zstd 压缩（I10 裁决）……M1 补 zstd 块压缩"。
- **M1 范围**：brief 的 M1 范围 10 项未提及 stored.bin zstd。
- **裁决建议**：stored.bin zstd 延后 M2（与 export 同批）。M1 不动 stored.bin 格式（避免 corpus 兼容测试断链）。若编排者要求 M1 做，需单独计划 + 评估 zstd 依赖对 wasm32 体积影响（core 加 zstd 会撑爆 800KB 红线——除非用纯 Rust ruzstd 仅解码，但压缩仍需 zstd 编码器）。

### R-6：HnswReader 并行搜索的 cfg 降级

- **问题**：`std::thread::scope` 在 wasm32 目标下可编译但实际单线程（wasm 无线程）。SPEC §11 要求 cfg 仅在 Executor 实现。
- **当前处理**：01-hnsw 计划用 `cfg(not(target_arch="wasm32"))` 包 `thread::scope`，wasm32 串行 fallback。这是搜索并行处的 cfg，非核心算法 cfg。
- **裁决建议**：接受此 cfg 作为 Executor 抽象的雏形（M1 不正式抽象 Executor trait，M2 再抽）。或要求 01 计划先全串行（无 cfg），M2 再并行。**倾向后者**——M1 先串行搜索（10 万×384 HNSW 搜索本就快，串行 <50ms 可达），避免 cfg 污染。01 计划 Task 5 验收时若性能达标则保持串行。

---

## 5. SPEC 覆盖自审

| SPEC §15 M1 交付项 | 计划 | 覆盖 |
|---|---|---|
| 分段 HNSW + 暴力回退 | 01-hnsw | ✅ |
| tombstone + 段合并 | 02-tombstone-merge | ✅ |
| metadata pre-filter | 03-pre-filter | ✅ |
| 薄 WAL | 04-wal | ✅ |
| jieba-lite + 词典 | 05-jieba-lite | ✅ |
| 自定义词表 + setUserDict/reindex | 06-userdict-reindex | ✅ |
| Node 词典分发 | 07-dict-distribution-node | ✅ |
| Go 词典分发 | 08-dict-distribution-go | ✅ |
| Go cgo 绑定（可后移） | 09-go-cgo-binding | ✅ |
| 冷启动实测背书 | 11-cold-start-bench | ✅ |
| recall≥0.95 真实回归 | 12-recall-regression | ✅ |
| CI 门禁扩展 | 10-ci-m1 | ✅ |
| export 快照 | — | ⚠️ R-1（建议 M2） |

**验收锚点覆盖**：
- §13.2-1 recall@10≥0.95 五档 → 12 + 10
- §13.2-2 中文分词四项 → 05（①②③④）+ 10（CI job）
- §13.2-3 体积门禁 → 10（wasm/dict）
- §7.4 状态机 → 06
- 冷启动 → 11
- 不变量 I-1~I-8 → README 矩阵全覆盖（I-3 图不删 / I-4 单一分词身份 / I-7 FFI 内存 / I-8 薄壳 在 M1 有真实测试）

---

## 6. Placeholder 扫描

`grep -rn "TBD\|TODO\|适当处理" docs/plans/m1/modules/ docs/plans/m1/README.md` → 0 命中。所有计划含真实测试代码与实现签名。

---

## 7. 降级顺序（燃尽图告急）

1. **不让位**：05-jieba-lite、01-hnsw、02-tombstone-merge、06-userdict-reindex（Must + recall 门禁依赖）。
2. **可后移**：09-go-cgo-binding → 08-dict-distribution-go → 10-ci-m1 的 Go 矩阵部分。
3. **不可后移**：10-ci-m1 的 recall 回归 + wasm 体积 + 词典体积门禁（质量合同）。

---

## 8. 结论

12 份模块计划 + README 索引产出完成，覆盖 SPEC §15 M1 全部范围（export 除外，见 R-1）。与 M0 pub API 对接经逐文件核查无误，无 M0 冻结签名破坏（reindex 签名变更是 SPEC IDL 落实，见 R-2）。6 项疑点待编排者裁决。
