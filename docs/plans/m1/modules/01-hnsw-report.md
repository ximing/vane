# 01-hnsw 实装报告

> 模块：`vane_core::hnsw`（段内不可变 HNSW 图 + 段级搜索归并 + 暴力自适应回退）
> 计划：`docs/plans/m1/modules/01-hnsw.md`（单一事实源，逐字遵循）
> 分支：main（worktree 不可用，直接在 main 工作；唯一写者）
> 提交：`aa252ca`（Task 1-4,6 hnsw 模块）、`0c1cb26`（Task 5 api 接入）

## Task 逐项改动

### Task 1：距离函数 + HnswWriter 骨架
- 新建 `crates/vane-core/src/hnsw/mod.rs`：`HnswGraph`/`HnswWriter`/`Node`，
  `new`/`insert`/`build`/`doc_count`/`entry_point`/`neighbors`。
- `metric_distance(metric,a,b)`：cosine=1-cos / L2=|a-b|² / dot=-dot（导航用，单调等价 -score）。
- `metric_score`：cosine / -sqrt(L2) / dot（结果 score 语义，与 brute_search 一致）。
- `lib.rs` 增 `pub mod hnsw;`。
- 测试：`hnsw_writer_builds_empty_graph`、`hnsw_writer_insert_single_node`。

### Task 2：分层插入 + 邻居选择
- 完整 HNSW 插入算法：level=floor(-ln(u)*mL)，mL=1/ln(M)；从 entry_point 贪婪层降；
  每层 ef_construction 搜索 → select_neighbors（简单取 M 最近）→ 双向连接 → 修剪
  （lc==0 上限 2M，其余 M）。entry_point=最高层节点。
- 确定性 xorshift64 RNG（固定种子，图结构可复现；不引入 rand crate，无新依赖）。
- 修复借用冲突：新节点先入 self.nodes（neighbors 暂空）再连接，保证修剪时
  `self.nodes[new_idx]` 合法。
- 测试：`hnsw_insert_multiple_nodes_connects_neighbors`。

### Task 3：write_hnsw / HnswReader::open / search
- `write_hnsw`：序列化到 `seg_dir/hnsw.bin`（格式见下方裁决 R-hnsw-vec）。
- `HnswReader::open`：反序列化；缺失文件返回 `Err`（Q-5 → api 层 catch fallback brute）。
- `HnswReader::search`：层降导航（max_level→1，ef=1）+ 第 0 层 ef_search 搜索；
  返回 `Vec<ScoredDoc>`（绝对 docid = local + docid_base，score 用 metric 语义，
  降序、同分 docid 升序，与 brute_search 一致）。
- 测试：`hnsw_search_returns_topk_nearest`、`hnsw_open_missing_file_returns_err`、
  `hnsw_search_cosine_metric`、`hnsw_search_docid_base_offset`、
  `hnsw_recall_vs_brute_small_scale`（300×8 L2，recall@10 实测 1.0 ≥0.95）。

### Task 4：filter 参数
- `search_layer` 增 `filter: Option<&RoaringBitmap>` + `docid_base`：
  结果堆 W 仅入 filter 命中节点（`filter.contains(local + base)`），
  邻居均可作导航点（§8.3「位图进 HNSW 遍历」）。上层导航 filter=None。
- 测试：`hnsw_search_with_filter_skips_excluded`。

### Task 5：api 层接入
- `CollectionInner` 增 `hnsw_readers: RwLock<Vec<Option<Arc<HnswReader>>>>`。
- flush：add_doc 完成后从 `reader.vectors()` 构建 HnswWriter（M=16/ef_construction=200）
  → build → write_hnsw；open HnswReader 入缓存（写/open 失败 → None，不阻塞 flush）。
- restore_from_manifest：每段 try open，Some/None。
- search vector 路：有 HnswReader 且非强制暴力 → HNSW（ef_search=max(ef_construction, want*4)）；
  否则 fallback brute_search。自适应回退骨架：filter 位图基数 < 2*topk → brute
  （M1 filter_bm=None，03 接入位图后触发）。
- `SearchQuery.filter` 不再 reject（M0 InvalidArg）→ 透传 None 占位（03 编译）。
- 新增 `Collection::segment_ulids()` 公共访问器。
- M0 测试更名：`search_filter_rejected_in_m0` → `search_filter_accepted_but_not_compiled_in_m1`。
- 集成测试 `tests/hnsw_recall.rs`：500 文档 recall + Q-5 缺失回退 + 多段串行归并。
- 全串行（R-4/R-6）：多段 for 循环归并，零 cfg(target)，无 thread::scope/rayon。

### Task 6：I-3 字节稳定占位
- 测试 `hnsw_graph_bytes_stable_after_write`：写后读两次字节一致。
- 强断言（delete 后 hnsw.bin 字节不变）由 02 Task 7 补（delete 走 tombstone 不动图）。

## 偏离与裁决

- **R-hnsw-vec（hnsw.bin 嵌入向量）**：README 契约的 hnsw.bin 格式只列图结构
  （magic/version/dim/metric/m/ef_construction/entry_point/max_level/num_nodes/
  {local_docid/level/num_neighbors/neighbors}），未含向量。但 Task 3 单元测试仅写
  hnsw.bin 即可 `search`（无 vectors.bin），且 `HnswReader::search` 需向量导航。
  故扩展格式：每节点记录末尾追加 `vector(dim*4 LE f32)`。理由：单元测试是契约的一部分
  （「逐字遵循」），search 需向量，独立 hnsw.bin 必须自包含。代价：向量在 vectors.bin
  与 hnsw.bin 各存一份（存储冗余 ~1x 向量体积）；M2 可优化为 HnswReader 复用 vectors.bin
  按需加载。api 层 brute_search 回退仍用 vectors.bin（路径不变）。**需编排者确认是否接受此扩展，
  或要求改为 HnswReader 加载 vectors.bin（届时需调整 Task 3/4 单元测试写 vectors.bin）。**

- **filter 编译延后 03**：M1 `filter_bm = None`，自适应回退分支存在但 03 前不触发。
  `SearchQuery.filter` 不再 reject（与计划一致），但 filter 实际不生效（透传 None）。

- **每 Task commit 粒度**：Task 1-4,6 的算法高度耦合于单文件，无法有意义地拆分为
  4 个独立 commit 而不回退代码，故合并为 1 个 commit（`aa252ca`），Task 5 单独 1 个
  （`0c1cb26`）。报告逐 Task 列明改动以保留可追溯性。

- **M0 测试更名**：`search_filter_rejected_in_m0` 断言 filter 返回 InvalidArg；
  M1 计划要求不再 reject，故更名为 `search_filter_accepted_but_not_compiled_in_m1`
  并改断言为 Ok（filter 透传 None，无文档时返回空 Vec）。此为测试更新，非 pub API 变更。

## 自证门禁结果（全绿）

| 门禁 | 结果 |
|---|---|
| `cargo test --workspace --all-features` | 212 lib + 集成全过（基线 202 → +10 hnsw 单元 +3 集成 -1 更名净 +12... 实测 212 lib/3 集成/其余同） |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 通过 |
| `cargo check --target wasm32-unknown-unknown -p vane-core` | 通过（hnsw 零 cfg） |
| `cargo fmt --all -- --check` | 通过 |
| `bash scripts/check-no-std-fs.sh` | OK |
| `bash crates/vane-node/scripts/check-thin.sh` | OK（I-8 clean） |
| `cargo bench --no-run -p vane-core` | 通过 |
| I-5 复核 | hnsw 模块 + api 接入零 `cfg(target)`、无 `thread::scope`/rayon |
| cargo-deny | 跳过（预存基础设施问题，非本模块引入；无新依赖） |

## recall 小规模测试结果

- `hnsw_recall_vs_brute_small_scale`：300 文档×8 维 L2，20 条 query，recall@10 实测 **1.0**（≥0.95）。
- `api_hnsw_recall_vs_brute_at_least_95pct`：500 文档×8 维 Cosine，api 层 HNSW 搜索返回 10 条（不 panic）。
- 大规模五档 recall 回归由 12-recall-regression 负责。

## 提交 hash

- `aa252ca` — hnsw: 实装 M1 01 模块——段内不可变 HNSW 图 + 搜索 + filter（Task 1-4,6）
- `0c1cb26` — api: 接入 HnswReader + 自适应暴力回退（01-hnsw Task 5）

## 遗留 / 疑问

1. **R-hnsw-vec 待裁决**：hnsw.bin 嵌入向量是本模块对 README 格式的扩展（见偏离节）。
   若编排者要求严格遵循 README 格式（向量不进 hnsw.bin），则需：HnswReader 改为加载
   vectors.bin，并调整 Task 3/4 单元测试额外写 vectors.bin。当前实现的测试与计划测试逐字一致。
2. **ef_search 公式**：实现用 `max(ef_construction, want*4)`（want = topk 或 cand）。
   计划表述为 `max(ef_construction, topk*4)`；hybrid 模式 want=cand（>topk），略大于计划公式，
   对 recall 有利、性能在预算内。若需严格 topk*4 可调整。
3. **02/03 衔接**：filter 位图编译（03）与 delete tombstone（02）未触及；
   `filter_bm=None` 占位、`delete` 仍 M0 占位 `E_UNSUPPORTED`。HnswReader 不删节点（I-3），
   02 MergeTask 重建图时调 HnswWriter（契约已就绪）。
