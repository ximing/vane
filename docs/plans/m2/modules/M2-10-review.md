# M2-10 100万规模 + Executor — 评审报告

**评审对象**：Executor trait + RayonExecutor/SerialExecutor + default_executor() 工厂 + Db.executor 字段 + 多段搜索并行归并 + 100万 #[ignore] 压测。
**评审范围**：vane-core（BASE c2d2ac6..HEAD da4abf5）。
**评审方式**：只读代码审查，未跑 cargo（编排者已确认 workspace 编译 + 测试绿）。
**评审日期**：2026-08-10。

## 结论

**PASS_WITH_FINDINGS** — 0 Blocker，0 Important，5 Minor。

并发正确性成立（无数据竞争），I-5 cfg 隔离守住（api/merge 零 cfg(target)），join_all 接口偏差技术合理（dyn-compatibility），归并不破 I-2 双索引原子性。发现均为 Minor（文档同步 / 门禁延后 / TDD 缺口），不阻塞合入。

---

## 1. I-5 cfg 隔离（关键不变量）— PASS

**证据**：
- `crates/vane-core/src/executor/mod.rs:43,46,62,65,83,87` — `cfg(all(not(target_arch="wasm32"), feature="executor-native"))` 及其补集，集中在 executor/mod.rs。
- `crates/vane-core/src/api/db.rs:24,50,56` — 仅 `executor: Arc<dyn ...>` 字段 + `default_executor()` 工厂调用，**零 `cfg(target)`**。
- `crates/vane-core/src/api/collection.rs:751-889` — 并行归并全段零 `cfg(target)`。
- `crates/vane-core/src/merge/` — grep `cfg(target` 空，零 cfg。
- rayon 使用：`grep -rn 'rayon' crates/vane-core/src/` 仅 `executor/mod.rs`（collection.rs:881 是注释文字，非 use）。
- 核心算法（hnsw/bm25/fusion/filter/segment）零 cfg(target)。

**pre-existing（非 M2-10 引入）**：
- `vfs/mod.rs:18`、`vfs/std_fs.rs`（6 处）、`vfs/tests.rs:67`、`segment/tests.rs:153` 均有 `cfg(not(target_arch="wasm32"))`。这些是 M0/M1 既存，计划 §4 test 4 已承认 vfs 合法；`segment/tests.rs:153` 计划描述遗漏（见 M-4）。

**裁决**：I-5 守住。M2-10 未在 executor/mod.rs 之外引入任何新 `cfg(target_arch)`。

## 2. 并发正确性（核心）— PASS

### 2.1 RayonExecutor::join_all 用法正确
`executor/mod.rs:48-54`：`rayon::scope(|s| { for task in tasks { s.spawn(move |_| task()); } })`。spawn 闭包 `move`，scope 阻塞至全部 task 完成，panic 经 rayon::scope 传播。语义 = 并行执行 + 阻塞至完成。正确。

### 2.2 归并逻辑与串行等价
`collection.rs:758-889`：
1. 预分配 `seg_results: Vec<Arc<Mutex<(Vec<ScoredDoc>, Vec<ScoredDoc>)>>>`，每段独立槽（collection.rs:761-763）。
2. 每段一个 task，task 内完成 vector 路 + text 路搜索，写**自己的**槽 `results_slot`（collection.rs:875-877）。
3. `join_all` 后串行 `extend_from_slice` 进 `vec_candidates`/`text_candidates`（collection.rs:885-889）。
4. 后续 `sort_by(score desc)` + `truncate(topk/cand)` + RRF/Linear 融合（collection.rs:892-949）**与 M1 串行版完全一致**。

**等价性论证**：原串行 `for` 循环每段 `append` hits 进 candidates，最后统一 sort/truncate；新版每段写独立槽后统一 extend + sort/truncate。归并算子（extend + sort + truncate）对无序输入产出相同全局 topK（同分时 sort 不稳定，但原串行 `append` 顺序 = snap 迭代顺序，新版 extend 顺序 = seg_results 数组顺序 = snap 迭代顺序，**顺序一致**）。故并行结果 = 串行结果。PASS。

### 2.3 无数据竞争
- **seg_results 槽**：每 task 写独占槽（`seg_results[i]`），无跨 task 共享写；Mutex 仅防 task 内 panic 后读槽，实际无竞争。
- **reader/inv_reader/hnsw_reader**：每 task 持不同段的 Arc clone（collection.rs:781-783），无同段并发写；`&self` 方法只读。
- **tokenizer_arc**（collection.rs:769-770）：`Arc<dyn Tokenizer>` 共享，`tokenize(&self)` 只读，Tokenizer: Send+Sync（RwLock 持有证明）。
- **query_vec/query_text/filter_bm_arc**：Arc clone 共享，只读。
- **SegmentReader::vectors() OnceLock**：每段 reader 仅被一个 task 访问（单次 search 内）；跨并发 search 访问同段 OnceLock 由 `OnceLock::get_or_init` Sync 安全（M2-07 验证）。

**裁决**：无数据竞争，无阻塞级问题。

### 2.4 I-2 双索引原子不破
段内 vector/text 在**同 task** 产出（collection.rs:799-874），段快照在 flush 时原子切换、搜索期只读不变；跨段归并在 `join_all` 结束后串行合并（collection.rs:885-889）。双索引一致性保持。PASS。

## 3. join_all 接口偏差 — 合理（Minor 文档同步）

**偏差**：计划 §3 契约 `fn scope<R>(&self, f: impl FnOnce(&Scope) -> R) -> R` + `Scope::spawn` → 实装 `fn join_all(&self, tasks: Vec<Box<dyn FnOnce() + Send>>)`（executor/mod.rs:34）。

**合理性**：`scope<R>` 含 generic `R`，不可 dyn-compatible，`Arc<dyn Executor>` 无法持有（Db.executor 字段需求）。`join_all` 无 generic，dyn-compatible 成立（tests.rs:120-128 编译期验证 `Send+Sync`）。功能等价：调用方预分配 per-segment 结果槽，tasks 写槽，join_all 阻塞至完成后串行归并。偏差技术正确。

**M-1（Minor）**：计划 §3 / SPEC §11 契约仍为 `scope<R>` + `Scope::spawn`，未同步为 `join_all`。建议更新计划 §3 与 SPEC §11 接口契约描述，或在计划中标注偏差裁决（报告 concern 1 已述，但计划文本未改）。

## 4. Db::open 调用方不破 — PASS

- `api/db.rs:43-56`：`Db::open` body 增 `let executor = default_executor();` + 字段初始化，**签名不变**（3 参：vfs, path, opts）。
- 调用方：`vane-ffi/src/lib.rs:523`、`vane-node/src/db.rs:35`、`vane-wasm/src/lib.rs:373`、`vane-wasm/src/worker.rs:689` 均 `Db::open(vfs, path, opts)`，未传 executor（内部默认构造）。
- executor 由 `default_executor()` 工厂在 open 内部构造，调用方无感。PASS。

## 5. rayon 依赖 — PASS

- `Cargo.toml:37`：`rayon = { version = "1", optional = true }`。
- `Cargo.toml:60`：`executor-native = ["dep:rayon"]`。
- `deny.toml` bans 列表无 rayon（grep 空）。
- vane-wasm/Cargo.toml 不传 executor-native（grep 空，PASS — rayon 不进 wasm）。
- vane-ffi/vane-node 亦未传 executor-native（见 M-2）。

## 6. 100万压测 — PASS（门禁延后）

`tests/million_scale.rs`：
- 默认（非 ignore）：3 个 1万 docs 测试（no_crash、matches_serial、compact）。
- `#[ignore]`：`parallel_search_100k`（10万）+ `million_scale_full_pipeline`（100万，P99<5000ms + hits.len==10）。
- recall≥0.80（million_scale.rs:209）：对比 parallel-HNSW vs serial-brute，是 scale smoke 阈值，**非 §13.2-1 recall_regression 门禁 ≥0.95**（后者是独立门禁，不混淆）。0.80 对 HNSW-approximate vs brute 合理。

**M-3（Minor）**：计划 §4 test 8 + §5 验收要求 100万 recall@10 ≥0.95（五档），但 `million_scale_full_pipeline`（million_scale.rs:274-340）仅断言 P99<5s + hits.len==10，**无 recall 断言**。因 100万 标 `#[ignore]`（构建耗时），recall 门禁延后至 CI heavy job。建议启用 100万 测试时补 recall 断言（vs brute 基线，五档选择率），或显式标注该门禁延后。

## 7. 不变量覆盖

| 不变量 | 状态 | 证据 |
|--------|------|------|
| I-5 cfg 隔离 | PASS | §1 |
| I-2 双索引原子 | PASS | §2.4 |
| I-3/I-4 merge/reindex | PASS（未触动）| merge/ 零改动，MergeTask 同步执行（报告 concern 4）|

## 8. TDD 覆盖 — PASS（Minor 缺口）

- Executor unit：rayon 并行（tests.rs:13）、rayon 完成性（tests.rs:55）、串行（tests.rs:72）、串行空（tests.rs:97）、工厂（tests.rs:103）、Send+Sync 编译期（tests.rs:120）。
- 多段并行搜索：10k no_crash + matches_serial + compact（million_scale.rs）。
- 100万 scale：#[ignore] 延后。

**M-5（Minor）**：`parallel_search_matches_serial_10k` 对比 parallel-HNSW vs serial-brute（不同模式），测 recall 非 strict 等价。缺一个**同模式**（executor-native on vs off，均 HNSW）结果集相同的严格等价测试。等价性由结构保证（确定性 per-segment 结果 + sort/truncate），风险低，但补测更稳。

## 9. 发现清单

### M-1（Minor）join_all 接口偏差 — 计划/SPEC 契约未同步
- **证据**：`docs/plans/m2/modules/M2-10-million-executor.md` §3（行 24-41）仍为 `scope<R>` + `Scope::spawn`；`executor/mod.rs:34` 实装 `join_all`。
- **影响**：契约文档与实装不一致，下游（M2-14 Demo）按旧契约编码会编译失败。
- **建议**：更新计划 §3 / SPEC §11 契约为 `join_all`，或显式标注偏差裁决。

### M-2（Minor）executor-native 未在 vane-ffi/vane-node 启用 → wrappers 默认串行
- **证据**：`crates/vane-ffi/Cargo.toml:14`（features=["zstd-encode","jieba"]）、`crates/vane-node/Cargo.toml:19`（features=["dict-zh","zstd-encode"]）均无 executor-native。
- **影响**：native wrappers 实际走 SerialExecutor，M2-10 并行搜索对终端用户未生效。并行路径仅 vane-core `--features executor-native` 测试覆盖。
- **裁决**：M2-10 §2 文件清单未含 wrapper Cargo.toml 改动，属计划外 follow-up（报告 concern 3 已述）。additive feature，不影响既有行为。
- **建议**：在 vane-ffi/vane-node Cargo.toml 默认启 `executor-native`，或建档跟踪启用计划。

### M-3（Minor）100万 recall≥0.95 门禁未实现
- **证据**：`million_scale.rs:274-340`（million_scale_full_pipeline）无 recall 断言；计划 §5 验收要求 recall@10 ≥0.95。
- **影响**：100万 recall 门禁未验证（#[ignore] 延后）。
- **建议**：启用 100万 测试时补 recall 断言，或显式标注门禁延后。

### M-4（Minor/Informational）计划 §4 test 4 I-5 grep 描述遗漏 segment/tests.rs
- **证据**：`segment/tests.rs:153` 有 `cfg(not(target_arch="wasm32"))`（pre-existing）；计划 §4 test 4 仅列 executor/mod.rs + vfs。
- **影响**：无代码缺陷（pre-existing test gating，非 M2-10 引入）。计划描述不完整。
- **建议**：计划 test 4 描述补注 segment/tests.rs pre-existing（可选）。

### M-5（Minor）TDD 严格等价测试缺口
- **证据**：`million_scale.rs:165-209` 对比 parallel-HNSW vs serial-brute（不同模式）。
- **影响**：未直接证明 join_all 并行 == 串行（同模式）。等价性结构保证，风险低。
- **建议**：补同模式（executor-native on/off）结果集等价测试。

## 10. 无 Blocker / Important 的理由

- **并发数据竞争**：§2.3 逐项验证无共享可变写，无竞争。
- **I-5**：§1 确认 api/merge 零 cfg(target)，rayon 仅 executor。
- **I-2**：§2.4 确认归并不破双索引原子。
- **join_all 偏差**：§3 技术合理（dyn-compatibility），功能等价，仅文档同步问题（Minor）。
- **wrappers 未启用**：§M-2 属计划外 follow-up，不阻塞 M2-10 交付。

---

**评审人**：task reviewer（只读）
**状态**：PASS_WITH_FINDINGS
