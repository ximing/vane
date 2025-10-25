# 01-hnsw 修复复审（R-hnsw-vec，fix 循环第 1 轮）

> 复审对象：BASE=9c372f7 → HEAD=919936f（commit `fix(hnsw): R-hnsw-vec ...`）
> 复审依据：原审查报告 `01-hnsw-review.md` 阻塞项 B-1 + 修复报告 `01-hnsw-fix-report.md`
> 复审日期：2026-08-09

## 维度逐条结论

### 1. hnsw.bin graph-only 闭环 —— ✅

- **`write_hnsw` 移除向量写入**：`crates/vane-core/src/hnsw/mod.rs:483-493`（旧）每节点 `dim*4 LE f32` 写入块已整体删除。新代码节点循环只写 `{ local_docid(4) | level(1) | 各层 num_neighbors(4)+neighbors }`。doc 注释改为「graph-only——不写向量」（`mod.rs:455`）。✅
- **`HnswReader::open` 不再读 Node.vector**：`mod.rs` 旧 L591-601 的向量反序列化块（`vlen = dim*4`、`Vec::with_capacity(dim)`、`f32::from_le_bytes` 循环）已删除。`Node` 构造不再含 `vector` 字段（`mod.rs:583-587`）。✅
- **`Node` 结构无 vector 字段**：`mod.rs:60-65`，`pub struct Node { local_docid, level, neighbors }`——`pub vector: Vec<f32>` 已移除。✅
- **新增字节断言真校验**：`hnsw_bin_is_graph_only_no_embedded_vectors`（`crates/vane-core/src/hnsw/tests.rs:206-248`）计算预期 graph-only 大小 `expected = 33(头) + Σ{4+1+Σ(4+neighbors*4)}`，并 `assert_eq!(buf.len(), expected)`，失败消息还提示「vectors would add {nodes*dim*4} bytes」。这是真字节级断言——若有向量尾部，`buf.len()` 会比 `expected` 大恰好 `nodes*dim*4` 字节。✅

### 2. search 借向量 —— ✅

- **`HnswReader::search` 增 `vectors: &[f32]` 参数**：`mod.rs:631`（末位参数）。`#[allow(clippy::too_many_arguments)]`（8 参，`mod.rs:624`）。✅
- **距离计算用 `vectors[local_docid*dim..]`**：新增 `node_vector(vectors, local_docid, dim)` 辅助（`mod.rs:779-785`），按 `local_docid * dim` 索引切片。`search_layer` 入口/邻居遍历均用 `node_vector(vectors, n.local_docid, self.dim)`（`mod.rs:710, 735`），结果 score 用 `node_vector(vectors, n.local_docid, self.dim)` + `metric_score`（`mod.rs:663-665`）。✅
- **不再用 `self.nodes[e].vector`**：grep 确认 `mod.rs` 中 HnswReader 路径零 `.vector` 引用（HnswWriter 路径改用 `self.vectors[idx]`，`mod.rs:375,400`）。✅
- **`search_layer` 同步增参**：`mod.rs:694`，`#[allow(clippy::too_many_arguments)]`（`mod.rs:684`）。✅

### 3. api 传参 —— ✅

- **collection.rs search 传 `reader.vectors()`**：`crates/vane-core/src/api/collection.rs:381`——`hr.search(qv, want, ef, filter_bm, base, reader.vectors())`。注释标明 R-hnsw-vec 共享单一副本（`collection.rs:380`）。✅
- **HnswReader 不持有 vectors**：`mod.rs:510-518` `pub struct HnswReader { dim, metric, m, ef_construction, entry_point, max_level, nodes }`——无 vectors 字段。与 SegmentReader 共用 `reader.vectors()` 借用的同一份 `&[f32]`，零冗余。✅

### 4. 零冗余确认 —— ✅

- **HnswReader 内存仅 graph**：见上，结构体无 vectors 字段。`open` 只读 hnsw.bin（graph-only），不读 vectors.bin。✅
- **vectors 仅 SegmentReader 一份**：api 层把 `reader.vectors()`（SegmentReader 已加载的 vectors.bin 单一副本）以 `&[f32]` 形式传入 HnswReader::search，无第二份副本。`HnswWriter.vectors`（`mod.rs:211`）仅写期构建用，build 后随 Writer 丢弃，不落盘、不进 Reader。✅

### 5. recall 无回归 —— ✅

- `hnsw_recall_vs_brute_small_scale`（`tests.rs:159-204`）测试通过，断言 `recall >= 0.95` 仍成立（实测 1.0，报告称）。算法路径未改：层降 ef=1 导航 + 第 0 层 ef_search 搜索，距离单调等价未变。仅向量来源从 `self.nodes[e].vector` 切到 `node_vector(vectors, ...)`，数学等价。✅

### 6. Q-5 fallback 不破坏 —— ✅

- `HnswReader::open` 缺失 hnsw.bin → `read_all_vfs` 失败 → `Err`（`mod.rs:525`）。本次未改 open 错误路径。✅
- api flush/restore 链路未碰（diff 仅改 search 调用一行）。`Some(hr)` → HNSW、`None` → brute 分支不变。✅
- 集成测试 `m0_corpus_without_hnsw_bin_falls_back_to_brute` 通过（实测）。✅

### 7. 测试名实相符 —— ✅

- `api_hnsw_vector_search_returns_results`（`crates/vane-core/tests/hnsw_recall.rs:9-50`）：断言返回 10 条 + score 降序（`hnsw_recall.rs:48-50` 的 windows(2) 检查）。名「returns_results」与断言一致。注释明确真实 recall 五档回归交 12-recall-regression。✅ 名实相符，原审查维度 11 瑕疵已闭环。

### 8. README 契约更新 —— ✅

- `docs/plans/m1/README.md`：
  - `HnswGraph` 注释标 graph-only + 向量由 api 传（L130-133）。✅
  - `write_hnsw` 注释标「graph-only——不写向量」+ 格式行无 vector（L145-149）。✅
  - `HnswReader::open` 注释标「仅读 graph-only hnsw.bin（不读 vectors.bin）」（L157）。✅
  - `HnswReader::search` 签名含 `vectors: &[f32]` 参数 + 说明（L160-170）。✅
  - §03-pre-filter 消费处 `HnswReader::search(filter, vectors)`（L295）。✅

### 9. 无新回归 —— ✅

- **245 测试全绿**：实测 `cargo test --workspace --all-features` = 213+2+3+1+3+19+4 = 245 passed, 1 ignored, 0 failed。与报告一致。✅
- **clippy**：`cargo clippy --workspace --all-targets --all-features -- -D warnings` 零 warning。✅
- **wasm32**：`cargo check --target wasm32-unknown-unknown -p vane-core` 通过（零 cfg，grep 确认 mod.rs/collection.rs 无 `cfg(target)`）。✅
- **fmt**：`cargo fmt --all -- --check` 通过。✅
- **no-std-fs**：`scripts/check-no-std-fs.sh` = OK。✅
- **thin**：`crates/vane-node/scripts/check-thin.sh` = OK。✅
- **M0 冻结签名未碰**：diff 仅改 collection.rs 一行（search 调用加末位参数），`brute_search`/`SegmentReader::vectors`/`dim`/`add_doc` 签名零变更。✅

## 非阻塞观察（记录，不要求本模块修）

- `HnswReader::open` 头长度校验 `buf.len() < 29`（`mod.rs:527`）偏松（实际头 33 字节），pre-existing 代码非本次引入，后续解析有完整 truncated 校验，不影响正确性。
- `HnswWriter::search_layer` 与 `HnswReader::search_layer` 逻辑重复，可后续提取（原审查已记录）。

## verdict

**APPROVED**

- 阻塞项 B-1（R-hnsw-vec）已正确闭环：hnsw.bin 改 graph-only（write_hnsw 不写向量、HnswReader::open 不读向量、Node 无 vector 字段、新增字节断言真校验）；HnswReader::search 增 `vectors: &[f32]` 参数借 SegmentReader 单一副本导航；api 层传 `reader.vectors()`；零冗余（HnswReader 内存仅 graph）。
- recall 无回归（1.0），Q-5 fallback 不破坏，测试名实相符，README 契约更新，245 测试 + clippy/wasm32/fmt/no-std-fs 全绿，M0 冻结签名未碰。
- 无未闭环项。可进 02-merge。
