# 12-recall-regression 实装报告

> SPEC §13.2-1（hybrid recall@10 ≥0.95，五档选择率 0.1%/1%/10%/50%/99%）/
> §8.4（召回回归覆盖五档）/§8.1（低选择率 <2×topK 暴力回退 100% 召回）。
> 前置：01-hnsw（HnswReader）、03-pre-filter（compile_filter / should_fallback_brute）。
> 状态：**完成，门禁全绿**。

## Task 改动

### Task 1：fixture 生成（1000 文档 × 128 维 + 五档标量）

- 新增 `crates/vane-core/tests/recall_fixture.rs`（`recall_regression.rs` 子模块，非独立 test 二进制）。
- 1000 文档 × 128 维 cosine 向量，Knuth 乘法哈希确定性生成（`wrapping_mul(2654435761)`，无 RNG 种子依赖，结果可复现）。
- `cat` 标量字段每文档唯一值（`c0..c999`），`FilterCond::In` 精确选档：0.1%→1 doc、1%→10、10%→100、50%→500、99%→990。
- `body` 文本字段含 `term{i%50}` 分布，使 text 路返回有意义 topK。
- 10 个查询向量（独立 seed 区分文档/查询空间）。
- `recall_at_10` 口径（见下方裁决 R-1）。

### Task 2：基线计算（暴力双路+RRF）— search_brute_baseline

- `api/collection.rs`：抽取 `fn run_search(&self, query, allow_hnsw: bool)` 共享 `search` 的全部逻辑（mode 推断 / dim 校验 / filter 编译 / 自适应回退 / Hit 回填）。
- `search(query)` → `run_search(query, true)`；`search_brute_baseline(query)` → `run_search(query, false)`。
- `allow_hnsw=false` 时 vector 路恒走 `brute_search`（跳过 HnswReader），text 路与 fusion 逻辑不变 → 100% 召回基线。
- `search_brute_baseline` 标注 `#[doc(hidden)]`，非对外 IDL（测试/bench 辅助）。
- **未改 M0 冻结 pub API**（仅新增方法 + 私有重构）。

### Task 3：五档选择率 × 三模式 recall@10 ≥ 0.95

- `tests/recall_regression.rs`：三个 `#[test]`（vector / text / hybrid）各跑五档 × 10 查询，断言 `recall ≥ 0.95`。
- 基线 = `col.search_brute_baseline(q)`；被测 = `col.search(q)`（内部 HnswReader + 自适应回退）。
- filter 透传：`SearchQuery.filter = Some(tier_filter(tier))`，经 `compile_filter` 编译为 roaring 位图，传各段 HnswReader/brute/InvertedIndexReader。

### Task 4：低选择率（0.1%）暴力回退验证

- `recall_low_selectivity_uses_brute_fallback`：0.1% 档（1 doc）位图基数 1 < 2×topK=20 → `should_fallback_brute` 触发 → api search 走暴力 → recall=1.0。

### CI 接入

- `.github/workflows/ci.yml` recall job：先跑 M0 `recall.rs` 冒烟（trivially 1.0），再跑 M1 `recall_regression.rs` 真实门禁。

## 五档 × 三模式 recall 实测数值

> 1000 文档 × 128 维 cosine，M=16/ef_construction=200/ef_search=max(200, topk×4)=200。
> 每档 × 10 查询取 min_recall（最差查询）。

| 选择率 | 命中 doc 数 | vector min_recall | text min_recall | hybrid min_recall |
|--------|------------|-------------------|-----------------|-------------------|
| 0.1%   | 1          | 1.000             | 1.000           | 1.000             |
| 1%     | 10         | 1.000             | 1.000           | 1.000             |
| 10%    | 100        | 1.000             | 1.000           | 1.000             |
| 50%    | 500        | 1.000             | 1.000           | 1.000             |
| 99%    | 990        | 1.000             | 1.000           | 1.000             |

**全部 15 组 min_recall = 1.000 ≥ 0.95 门禁通过。**

说明：
- 1000 文档规模 + ef_search=200（远大于 topK=10），HNSW 在该规模下召回达 1.0（精确解）。
- text 模式基线与被测均用 `InvertedIndexReader::search`（WAND 精确 topK），恒等 → recall=1.0。
- hybrid：vector 路 HNSW 精确 + text 路 WAND 精确 → RRF 融合结果与基线一致 → recall=1.0。
- 大规模（10 万）HNSW 召回压力测试在 11-cold-start-bench 跑（非门禁）。

## 偏离与裁决

### R-1：recall@10 分母取 min(10, |baseline_top10|)（非 /10）

**计划原文** recall_at_10 公式为 `hit / 10.0`。**实测问题**：0.1% 档（1 doc）时基线仅返回 1 个文档，`hit=1`，`1/10=0.1 < 0.95`，与 Task 4「0.1% 档 recall=1.0」断言矛盾。

**裁决**：采用标准 IR recall@k 口径 `|retrieved ∩ relevant| / |relevant|`，分母 = `min(10, |baseline_top10|)`。基线为空（无相关文档）时记 recall=1.0（vacuously perfect）。

**依据**：
- SPEC §8.1「低选择率暴力回退 100% 召回」要求 0.1% 档 recall=1.0，与 /10 公式矛盾 → /10 非正确口径。
- 标准 IR recall@k 定义分母为相关文档数（capped at k），非 k 本身。
- 计划 Task 4 断言 `recall=1.0` 是硬约束，公式应适配之。

**影响**：仅测试辅助函数口径调整，不影响门禁严格性（高选择率档 baseline 恒 ≥10，分母仍 = 10）。

### R-2：search_brute_baseline 实装方式（私有重构 vs 复制）

**计划原文** 暗示新增方法内部复制 search 逻辑。**裁决**：抽取 `run_search(query, allow_hnsw)` 私有方法共享逻辑，避免大段复制（~80 行）引入维护负担与行为漂移风险。`search`/`search_brute_baseline` 仅传 `allow_hnsw` 布尔差异。未改 pub API。

## 自证门禁

| 门禁 | 结果 |
|------|------|
| `cargo test --workspace --all-features` | ✅ 全绿（234 + 各 test 二进制，含 recall_regression 7 测试） |
| `cargo test --test recall_regression -p vane-core` | ✅ 7 passed / 0 failed（2.1s） |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | ✅ 无告警 |
| `cargo check --target wasm32-unknown-unknown -p vane-core` | ✅ 通过 |
| `cargo fmt --all -- --check` | ✅ 通过 |
| `bash scripts/check-no-std-fs.sh` | ✅ OK |

## 提交 hash

- `013926a` recall: 五档选择率 × 三模式 recall@10≥0.95 真实回归门禁（12-recall-regression）
- `30879e2` ci: recall job 增 recall_regression 真实门禁（12-recall-regression）

## 遗留 / 疑问

- **recall 全 1.0 是否过轻**：1000 文档 + ef_search=200 下 HNSW 达精确解，门禁未真正"压力测试" HNSW 召回能力。大规模（10 万 × 384 维）召回压力在 11-cold-start-bench 的 bench 中跑（非门禁）。若编排者认为需要更强压力，可考虑：①增大 fixture 规模至 5000-10000；②降低 ef_search；③引入更结构化的向量分布（聚类簇）使 topK 邻域更紧凑。当前 1000 文档符合计划「CI 秒级」约束（2.1s）。
- **R-1 分母口径**：需编排者确认 min(10, baseline_len) 口径被接受（计划原文为 /10，但与 Task 4 矛盾）。若要求严格 /10，则需将 DOC_COUNT 提至 10000 使 0.1% 档 ≥10 doc（CI 仍可接受，但偏离计划「1000 文档」）。
- 无其他疑问。M0 `tests/recall.rs` 保留作冒烟（trivially 1.0），未被删除。
