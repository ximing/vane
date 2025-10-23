# 01-hnsw 修复报告（R-hnsw-vec，fix 循环第 1 轮）

> 修复对象：审查阻塞项 B-1（R-hnsw-vec — hnsw.bin 嵌入向量）
> 修复日期：2026-08-09
> 方案：编排者裁决方案 B（零冗余——search 借用 SegmentReader.vectors() 单一副本）

## 问题

`write_hnsw` 在每节点末尾追加 `dim*4` f32 向量，`HnswReader::open` 读入 `Node.vector`，`search`/`search_layer` 用 `self.nodes[e].vector` 算导航距离。导致向量在 vectors.bin + hnsw.bin 双存（10 万×384≈321MB 逼近 §13.1 500MB、50 万≈1.6GB 违反 §3.3「50 万不塌红线」会 OOM），违反 SPEC §6.2 + README 契约（hnsw.bin = graph-only）。

## 修复改动

### 1. hnsw.bin 改 graph-only（`crates/vane-core/src/hnsw/mod.rs`）

- **模块文档**（L10-21）：格式说明移除 `vector(dim*4 LE f32)` 行，改为 graph-only 契约 + R-hnsw-vec 修复说明。
- **`Node` 结构**（L60-66）：移除 `pub vector: Vec<f32>` 字段。Reader/Graph 节点不再持有向量。
- **`HnswWriter`**（L204-215）：新增 `vectors: Vec<Vec<f32>>` 字段（按 node_idx 索引，写期构建用，不落盘）。
  - `insert`（L244-263）：`new_node` 不再含 vector；`self.vectors.push(vector.to_vec())` 与 `self.nodes.push` 成对。
  - `insert` 修剪段（L288-307）：`self.nodes[nb].vector.clone()` → `self.vectors[nb].clone()`；`self.nodes[c].vector.clone()` → `self.vectors[c].clone()`。
  - `search_layer`（写期，L399-420）：`&n.vector` / `&en.vector` → `&self.vectors[ep]` / `&self.vectors[e]`。
  - `build`（L383-394）：`HnswGraph` 不再带 vectors（vectors 字段丢弃）。
- **`write_hnsw`**（L468-501）：移除每节点 `vector(dim*4 LE f32)` 写入块（原 L483-493）。doc 注释改为「graph-only——不写向量」。
- **`HnswReader::open`**（L543-611）：移除每节点向量反序列化块（原 L591-601）。`Node` 构造不再含 vector。
- **`HnswReader` 结构**（L529-537）：无 `vector` 字段（本就无此字段，Node 改了即生效）。

### 2. HnswReader::search 增 `vectors: &[f32]` 参数（`mod.rs` L685-742）

- **`search` 签名**（L685-694）：增 `vectors: &[f32]` 末位参数。`#[allow(clippy::too_many_arguments)]`（8 参）。
- **`search` 体**：`search_layer` 调用传 `vectors`；结果 score 用 `node_vector(vectors, n.local_docid, self.dim)` 取向量。
- **`search_layer`（读期，L754-815）**：增 `vectors: &[f32]` 参数。`&n.vector`/`&en.vector` → `node_vector(vectors, n.local_docid, self.dim)`。`#[allow(clippy::too_many_arguments)]`。
- **`node_vector` 辅助**（L823-828）：`fn node_vector(vectors: &[f32], local_docid: u32, dim: u32) -> &[f32]`，按 `local_docid * dim` 索引切片。

### 3. api 层传 vectors（`crates/vane-core/src/api/collection.rs` L378-382）

- `hr.search(qv, want, ef, filter_bm, base)` → `hr.search(qv, want, ef, filter_bm, base, reader.vectors())`。
- 注释：R-hnsw-vec——向量不进 hnsw.bin，由 SegmentReader.vectors() 传入共享单一副本。
- flush/restore 不变（HnswReader::open 仍 graph-only；缺失 hnsw.bin → None → brute fallback，Q-5 不变）。

### 4. 测试更新

- **`hnsw/tests.rs`**：
  - `hnsw_search_returns_topk_nearest`（L38-53）：构造 `vectors: Vec<f32>` flat slice 传给 search。
  - `hnsw_search_with_filter_skips_excluded`（L56-75）：同上。
  - `hnsw_search_cosine_metric`（L120-139）：insert 时同步累积 vectors slice。
  - `hnsw_search_docid_base_offset`（L141-157）：同上。
  - `hnsw_recall_vs_brute_small_scale`（L159-204）：vectors 本就是 flat slice，search 调用加 `&vectors`。
  - **新增** `hnsw_bin_is_graph_only_no_embedded_vectors`（L206-243）：断言 hnsw.bin 字节大小 == graph-only 预期（头 33 + 每节点 {local_docid(4)+level(1)+各层 num_neighbors(4)+neighbors}），即不含 `dim*4` 向量尾部。
- **`tests/hnsw_recall.rs`**：
  - `api_hnsw_recall_vs_brute_at_least_95pct` → **改名** `api_hnsw_vector_search_returns_results`（L9-47）。名实相符：仅断言返回 10 条 + score 降序（真实 recall 五档回归由 12-recall-regression 负责）。注释更新说明 graph-only + api 传 vectors。

### 5. README 契约更新（`docs/plans/m1/README.md`）

- §01-hnsw `HnswGraph` 注释（L131-133）：标注 graph-only + 向量由 api 传。
- `write_hnsw` 注释（L148-151）：标注「graph-only——不写向量」+ R-hnsw-vec 说明。
- `HnswReader::open` 注释（L156）：标注「仅读 graph-only hnsw.bin（不读 vectors.bin）」。
- `HnswReader::search` 签名（L157-168）：增 `vectors: &[f32]` 参数 + 说明。
- §03-pre-filter 消费处（L289）：`HnswReader::search(filter)` → `HnswReader::search(filter, vectors)` 注明 vectors 由 api 传入。

## 自证门禁结果

| 门禁 | 结果 |
|---|---|
| `cargo test --workspace --all-features` | ✅ 213 + 2 + 3 + 1 + 3 + 19 + 4 = 245 passed, 0 failed, 1 ignored |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | ✅ 零 warning |
| `cargo check --target wasm32-unknown-unknown -p vane-core` | ✅ 通过（零 cfg） |
| `cargo fmt --all -- --check` | ✅ 通过 |
| `bash scripts/check-no-std-fs.sh` | ✅ OK |
| `bash crates/vane-node/scripts/check-thin.sh` | ✅ OK |
| `cargo bench --no-run -p vane-core` | ✅ 编译通过 |
| **hnsw 单元测试** | ✅ 11 passed（含新增 graph-only 字节断言） |
| **hnsw 集成测试** | ✅ 3 passed（含改名后的 api_hnsw_vector_search_returns_results） |
| **recall@10（hnsw_recall_vs_brute_small_scale）** | ✅ **1.0**（300×8 L2，20 query，≥0.95） |
| **hnsw.bin 不含向量** | ✅ `hnsw_bin_is_graph_only_no_embedded_vectors` 断言字节大小 == graph-only 预期 |

## 提交

- hash: 见 `git log -1`（commit message: `fix(hnsw): R-hnsw-vec hnsw.bin graph-only, search 借用 SegmentReader.vectors 导航`）
- 分支: main（与 M1 既定提交流一致：aa252ca/0c1cb26/9c372f7 均在 main）

## 遗留

- 无。R-hnsw-vec 阻塞项已清除，hnsw.bin graph-only + 向量单一副本（SegmentReader.vectors()）零冗余。
- 非阻塞观察（审查已记录，不在本次修复范围）：HnswWriter/Reader search_layer 逻辑重复可后续提取；select_neighbors 简单策略可由 12-regression 验证后决定升级。
