# 12-recall-regression 代码审查

> 审查者：code-reviewer（只读，未运行 cargo）。
> 基线 BASE=72bb641，HEAD=7d5722a。
> 范围：`crates/vane-core/tests/recall_regression.rs`（+244）、`crates/vane-core/tests/recall_fixture.rs`（+140）、`crates/vane-core/src/api/collection.rs`（+23/-2）、`.github/workflows/ci.yml`（recall job）。
> 编排者集成门禁已确认 7 测试绿 + clippy/wasm32/fmt/no-std-fs 全过。

## 逐维度结论

### 1. recall 口径 ✅

`recall_fixture.rs:128-140` 实装 `recall_at_10 = |hnsw_top10 ∩ baseline_top10| / min(10, |baseline_top10|)`，按 external_id 比对，分母取 `base_ids.len()`（因 `baseline.iter().take(10)` 已 cap 至 10）= 标准 IR recall@k。符合 SPEC §13.2-1「相对暴力双路+RRF 基线」口径。

- 0.1% 档（1 doc）：|relevant|=1，被找到 → 1/1=1.0。合理（标准 IR：唯一相关文档被检索到即满召回）。
- 基线空（无相关文档）→ 记 1.0（vacuously perfect），已注释说明。合理。
- **裁决 R-1 偏离计划原文（/10）但正确**：计划 Task 4 硬约束 0.1% 档 recall=1.0，与 /10 公式矛盾；实装采用标准 IR 口径，与 SPEC §8.1「低选择率 100% 召回」一致。高选择率档 baseline 恒 ≥10，分母仍 = 10，门禁严格性未降低。

### 2. 五档选择率构造 ✅

`recall_fixture.rs:104-110` `cats_for_tier`：`n = round(1000 * tier).clamp(1, 1000)`，从 `c0` 起连续取 → 0.1%=1、1%=10、10%=100、50%=500、99%=990 doc。`tier_filter`（:114-120）构造 `Filter{fields: [("cat", FilterCond::In([...))]}`，经 `api/collection.rs:595-601` 的 `compile_filter`（03 模块）编译为 roaring 位图，**非手动位图**。每文档 cat 唯一值（`c0..c999`）保证选档精确。

### 3. 三模式覆盖 ✅

`recall_regression.rs` 各一个 `#[test]`：
- `recall_vector_5_selectivity_tiers`（:155-173）
- `recall_text_5_selectivity_tiers`（:175-193）
- `recall_hybrid_5_selectivity_tiers`（:195-213）

每测试遍历五档 × 10 查询，断言 `r >= RECALL_THRESHOLD(0.95)`。hybrid 基线 = `search_brute_baseline`（brute 双路 + rrf_fuse，见维度 4）。text 模式 baseline 与被测均用 `InvertedIndexReader::search`（WAND 精确 topK）→ 恒等 → recall=1.0（合理，text 路无近似）。

### 4. 基线正确性 ✅

`collection.rs:516-524`：`search_brute_baseline` 标注 `#[doc(hidden)]`，调 `run_search(query, false)`。`run_search`（:528-）共享 search 全部逻辑，`allow_hnsw=false` 时 vector 路恒走 `brute_search`（:658-668，跳过 HnswReader 分支），text 路 `InvertedIndexReader::search` 不变，fusion 走 rrf_fuse。**与 HNSW 路径独立**（基线完全绕过 HnswReader）。

辅助测试 `search_brute_baseline_returns_topk_without_hnsw`（:23-46）验证基线返回 topK=10 且按 RRF 分降序；`search_brute_baseline_matches_search_when_no_hnsw_segment`（:48-65）验证 text 模式下基线 == search（recall=1.0）。

### 5. HNSW 路径真正 exercised ✅（核心结论）

**flush 写 hnsw.bin**：`collection.rs:281-310` flush 中 `write_hnsw` + `HnswReader::open`，成功则 push `Some(hr)`（:332-337）。fixture 单次 flush 1000 doc → 段有 hnsw.bin → `hnsw_reader=Some`。

**自适应回退阈值**（`filter/mod.rs:149-151`）：`bitmap.len() < 2*topk`（topk=10 → 阈值 20）。

各档实际走查（vector 路）：
| 档 | 命中 doc 数 | `force_brute`（<20?） | `use_hnsw = allow_hnsw && !force_brute` | 实走路径 |
|----|-----|-----|-----|-----|
| 0.1% | 1 | true | false | brute（回退） |
| 1% | 10 | true（10<20） | false | **brute（回退）** |
| 10% | 100 | false | true | **HNSW** |
| 50% | 500 | false | true | **HNSW** |
| 99% | 990 | false | true | **HNSW** |

`collection.rs:641-668` 确认：`use_hnsw=true` 且 `hnsw_reader=Some` → 走 `hr.search(qv, want, ef, merged_filter, base, reader.vectors())`（:646），非 brute。`ef = ef_construction().max(want*4) = max(200, 40)=200`（:644）。

**结论**：10%/50%/99% 三档（vector 与 hybrid 的 vector 子路）真正经 HnswReader.search，HNSW 被实际 exercised。0.1%/1% 档走 brute 回退（recall=1.0 by design，符合 §8.1）。**未出现"全部 brute 假测 HNSW"的情况。**

⚠️ 次要：1% 档（10 doc）实际也走 brute 回退（10<20），但 `recall_low_selectivity_uses_brute_fallback` 测试仅显式覆盖 0.1% 档，报告表格将 1% recall=1.0 列入而未注明其为 brute 回退。不影响门禁正确性，但 1% 档并非 HNSW 召回证据——HNSW 召回仅由 10%/50%/99% 三档背书。

### 6. 断言严格性 ✅

`recall_regression.rs:166-171/187-192/208-213`：每档×每查询 `assert!(r >= RECALL_THRESHOLD)`，`RECALL_THRESHOLD=0.95`（`recall_fixture.rs:30`）。未调低断言。Task 4（:228-232）显式 `assert_eq!(r, 1.0)` 卡 0.1% 档暴力回退。

### 7. recall 全 1.0 评估 ⚠️（非阻塞，已自证）

1000 doc + ef_search=200（远大于 topK=10）下 HNSW 达精确解，15 组 min_recall 全 1.0。报告「遗留/疑问」已诚实承认门禁未真正"压力测试" HNSW 近似能力，并给出三选项（增大 fixture / 降 ef_search / 聚类簇向量分布），大规模压力延至 11-cold-start-bench（非门禁）。

**评估**：当前 1000 doc 符合计划「CI 秒级」约束（2.1s），门禁能防"HNSW 完全坏掉"的回归（如 ef 配置错、filter 传错导致 0 召回），但无法捕捉 HNSW 召回率从 0.99 退化到 0.96 这类近似质量退化。建议作为 11-cold-start-bench 的可选 stress bench（更大规模 + 更低 ef_search + 聚类向量分布），**非本模块阻塞项**。

### 8. M0 recall.rs 替换与 CI 指向 ✅

- M0 `tests/recall.rs` 保留作冒烟（trivially 1.0，:82 `let recall = 1.0`），未删除——符合计划「裁决：保留」。
- `ci.yml:51-54` recall job 两步：`cargo test --test recall -p vane-core`（M0 冒烟）+ `cargo test --test recall_regression -p vane-core`（M1 真实门禁）。指向正确。

### 9. M0 签名零破坏 ✅

- `search_brute_baseline` 为新增 `#[doc(hidden)]` 方法（:516-524），非对外 IDL。
- `search` pub 签名不变（:509-511，体内改为调 `run_search(query, true)`）。
- `run_search` 为私有 `fn`，共享逻辑重构未破坏 pub API。M0 `search` 调用方零影响。

### 10. 确定性 ✅

`recall_fixture.rs:67-77` `deterministic_vector` 用 Knuth 乘法哈希（`wrapping_mul(2654435761)`），无 RNG 种子依赖，纯函数式可复现。文档/查询向量用不同 seed 组合（`i*31`/`i*13+7` + seed_b 0/1）区分空间。CI 友好（2.1s，秒级）。

### 11. CI 接入 ✅

`.github/workflows/ci.yml` recall job 增 `Run recall regression gate (M1, §13.2-1)` step（:53-54）。已接入。

## 额外发现（非审查维度，供编排者裁决）

- **SPEC §8.4 第二条未覆盖**：SPEC §8.4 要求「SIMD128 与 scalar 两个 wasm 变体各跑一遍召回回归（防 SIMD 数值路径分歧）」。当前 `recall_regression` 仅在 native target 跑，ci.yml 未加 wasm 变体召回回归 step。可能属 10-ci-m1 或 wasm SIMD 模块职责，**非本模块阻塞**，但需编排者确认由谁兜底。

## 阻塞项

无。

## 需编排者裁决的疑点

1. **R-1 分母口径**：实装 `min(10, |baseline_top10|)`（标准 IR），偏离计划原文 `/10`。需确认接受（报告已申请裁决；审查者认为标准 IR 口径正确，与 SPEC §8.1 自洽，建议接受）。
2. **1% 档归类**：1% 档（10 doc）实际走 brute 回退而非 HNSW，但报告表格未注明。是否需要在报告/注释中显式标注「1% 档亦属 brute 回退」？审查者认为非阻塞（门禁正确），但透明度可改进。
3. **recall 全 1.0 的门禁强度**：是否需在本模块内增强（更大 fixture / 降 ef_search / 聚类向量），还是按计划延至 11-cold-start-bench？审查者建议延至 11-cold-start-bench（符合计划「CI 秒级」约束）。
4. **§8.4 wasm 双变体召回回归**：由本模块还是 10-ci-m1 / wasm 模块兜底？
