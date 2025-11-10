# M2-06 SIMD 双变体召回回归

## 1. 目标
SIMD128 与 scalar 两个 wasm 变体各跑一遍 recall@10≥0.95 五档选择率回归（SPEC §8.4/§13.2-1），防 SIMD 数值路径分歧导致召回退化。

SPEC 节号：§8.4（两变体各跑召回回归）、§13.2-1（hybrid recall@10≥0.95 五档）。

## 2. 涉及文件
- **Create** `crates/vane-wasm/tests/recall_regression_simd.rs`：simd 产物召回回归（wasm-bindgen-test）。
- **Create** `crates/vane-wasm/tests/recall_regression_scalar.rs`：scalar 产物召回回归。
- **Modify** `.github/workflows/ci.yml`：增 `wasm-recall` job——双产物构建 + 浏览器跑召回回归（用 wasm-bindgen-test-runner 或 headless browser）。
- **Consumes** M1 `crates/vane-core/tests/recall_regression.rs` 测试方法论与 fixture（五档选择率 0.1%/1%/10%/50%/99% × 三模式 vector/text/hybrid）。

## 3. 接口契约
### Consumes from
- M1 `recall_regression` 测试方法论（M1 README §12，五档×三模式，HNSW vs 暴力双路+RRF 基线）。
- M2-05 双产物（simd/scalar `.wasm`）。
- M0 `vane_core::vector::brute_search`（基线口径）、M1 `HnswReader::search`（被测路径）。

### Produces for
- CI `wasm-recall` job：双产物 recall@10≥0.95 硬门禁。
- 回归报告：simd vs scalar 召回差异（预期 0，若 >0 评估数值分歧来源）。

## 4. TDD 测试清单
1. **simd 产物五档回归**：`recall_regression_simd` 跑 0.1%/1%/10%/50%/99% 五档 × vector/text/hybrid 三模式，每档 recall@10 ≥0.95（相对暴力双路+RRF 基线）。
2. **scalar 产物五档回归**：同测试 1，scalar 产物。
3. **两变体结果一致**：simd 与 scalar 同查询同基线，topK 结果集 Jaccard ≥0.99（允许排序微差异，但召回集一致）。
4. **数值分歧硬断言**（reviewer B-M9）：测试 3 的 Jaccard ≥0.99 为**硬断言**（非诊断步骤），失败即 CI 阻断；失败时附诊断信息（分歧查询列表 + 数值差异量级），评估是否 SIMD 路径 bug（预期 f32 SIMD 误差 <1e-6 不影响召回）。
5. **CI 集成**：`wasm-recall` job 在 PR 上跑双产物回归，失败阻断。

## 5. 验收标准
- 双产物五档×三模式 recall@10 ≥0.95 全绿（CI 硬门禁）。
- 两变体 Jaccard ≥0.99（或分歧可解释为 f32 数值精度，非 bug）。
- CI `wasm-recall` job 稳定运行（headless browser 矩阵：Chrome）。

## 6. 前置依赖
- M2-05（双产物构建）。

## 7. 不变量覆盖
- **§8.4 SIMD 双变体召回回归**：本模块直接落实。测试 1+2 守护。
- **§13.2-1 recall@10≥0.95**：测试 1+2 五档硬门禁。
- **I-8 binding 薄壳**：召回测试在 core 方法论，wasm 产物仅作为运行载体。
