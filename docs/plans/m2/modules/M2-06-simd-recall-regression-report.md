# M2-06 SIMD 双变体召回回归——报告

## 1. 概述

在 M2-05 产出的 simd/scalar 双 wasm 变体上各跑一遍 recall@10≥0.95 五档选择率
回归（SPEC §8.4/§13.2-1），并跨变体比对 topK Jaccard≥0.99（硬断言），防 SIMD
数值路径分歧导致召回退化。

M2-05 已确认 f32 距离循环未向量化（FP 非结合性；SIMD 仅加速 roaring 位图），
故两变体向量/文本召回数值一致，预期 Jaccard=1.0。本模块验证：
1. wasm32 召回路径正确（无 native 专属 bug）；
2. §8.4 双变体无分歧（硬门禁）；
3. CI 可重复。

## 2. 交付物

| 文件 | 类型 | 说明 |
|---|---|---|
| `crates/vane-wasm/tests/common/mod.rs` | Create | 共享召回回归模块（simd/scalar 复用）；`#[path]` 引入 M1 `recall_fixture.rs` |
| `crates/vane-wasm/tests/recall_regression_simd.rs` | Create | simd 变体测试二进制（`mod common;`） |
| `crates/vane-wasm/tests/recall_regression_scalar.rs` | Create | scalar 变体测试二进制（`mod common;`） |
| `scripts/run-wasm-recall.sh` | Create | CI 编排：双产物构建 + wasm-bindgen-test(node) + Jaccard 比对 |
| `scripts/wasm-recall-jaccard.mjs` | Create | 跨变体 Jaccard≥0.99 硬断言（node） |
| `.cargo/config.toml` | Create | `wasm-bindgen-test-runner` 作为 wasm32 测试运行器 |
| `.github/workflows/ci.yml` | Modify | 新增 `wasm-recall` job |

## 3. 方法论

### 3.1 复用 M1 recall 方法论（I-8 薄壳）

测试逻辑完全复用 M1 `crates/vane-core/tests/recall_regression.rs` + `recall_fixture.rs`：
- 1000 文档 × 128 维 cosine，确定性伪随机（Knuth 乘法哈希）。
- 五档选择率 0.1%/1%/10%/50%/99% × 三模式 vector/text/hybrid。
- 基线 = `Collection::search_brute_baseline`（暴力双路 + RRF），被测 = `Collection::search`（HNSW + 自适应回退）。
- recall@10 = |hnsw_top10 ∩ baseline_top10| / min(10, |baseline_top10|)。

通过 `#[path = "../../../vane-core/tests/recall_fixture.rs"] mod recall_fixture;` 直接
引入 core fixture，零拷贝、零改 core。wasm 产物仅作运行载体（I-8）。

### 3.2 双变体区分

两测试二进制 `recall_regression_simd.rs` / `recall_regression_scalar.rs` 内容同源
（均 `mod common;`），变体区分由 CI 构建时 RUSTFLAGS 决定：
- simd：`RUSTFLAGS="-Ctarget-feature=+simd128"`
- scalar：默认（无 simd128）

### 3.3 Jaccard 探针

`recall_jaccard_probe` 测试对固定查询集（5 查询 × 3 模式 × 2 档 10%/50% = 30 个 topK 集）
产出 `JACCARD_PROBE <json>` 行（console.log），由 `scripts/run-wasm-recall.sh` 捕获
两变体输出后，`scripts/wasm-recall-jaccard.mjs` 按 (q, mode, tier) 配对计算 Jaccard，
硬断言 min Jaccard ≥ 0.99。

## 4. 双产物召回回归结果

### 4.1 simd 变体（RUSTFLAGS=-Ctarget-feature=+simd128）

五档 × 三模式 recall@10（min_recall across 10 queries）：

| tier | vector | text | hybrid |
|---|---|---|---|
| 0.1% | 1.000 | 1.000 | 1.000 |
| 1% | 1.000 | 1.000 | 1.000 |
| 10% | 1.000 | 1.000 | 1.000 |
| 50% | 1.000 | 1.000 | 1.000 |
| 99% | 1.000 | 1.000 | 1.000 |

0.1% 档暴力回退 recall=1.0（§8.1）✓。全部 ≥ 0.95 ✓。

### 4.2 scalar 变体（默认 RUSTFLAGS）

| tier | vector | text | hybrid |
|---|---|---|---|
| 0.1% | 1.000 | 1.000 | 1.000 |
| 1% | 1.000 | 1.000 | 1.000 |
| 10% | 1.000 | 1.000 | 1.000 |
| 50% | 1.000 | 1.000 | 1.000 |
| 99% | 1.000 | 1.000 | 1.000 |

0.1% 档暴力回退 recall=1.0 ✓。全部 ≥ 0.95 ✓。

## 5. 两变体 Jaccard（硬断言）

30 个 topK 集跨变体比对：

```
Jaccard comparison: 30 queries, min Jaccard = 1.000000
PASS: all queries Jaccard >= 0.99
```

min Jaccard = **1.000000**（与 M2-05 预期一致：f32 未向量化，两变体数值完全一致）。

## 6. CI `wasm-recall` job

```yaml
wasm-recall:
  needs: test
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v4
    - uses: dtolnay/rust-toolchain@stable
      with:
        targets: wasm32-unknown-unknown
    - uses: actions/setup-node@v4
      with:
        node-version: '20'
    - name: Install wasm-bindgen-cli
      run: cargo install wasm-bindgen-cli --locked --version 0.2.127
    - name: Run dual-variant recall regression (simd + scalar + Jaccard)
      run: bash scripts/run-wasm-recall.sh
```

本地已跑通完整 node 路径（双产物构建 + wasm-bindgen-test-runner + Jaccard 比对）。
远程 CI 标注：ubuntu-latest + node 20 + wasm-bindgen-cli 0.2.127。

## 7. 自证门禁结果

| # | 门禁 | 结果 |
|---|---|---|
| 1 | `cargo test --workspace --all-features` 全绿 | ✅ 459 passed, 0 failed, 1 ignored（无回退） |
| 2 | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | ✅ clean |
| 3 | `cargo fmt --all -- --check` | ✅ clean |
| 4 | simd 产物五档×三模式 recall@10≥0.95 | ✅ 全 1.000 |
| 5 | scalar 产物五档×三模式 recall@10≥0.95 | ✅ 全 1.000 |
| 6 | 两变体 Jaccard ≥0.99 | ✅ min Jaccard = 1.000000 |
| 7 | CI `wasm-recall` job 写入 + 本地 node 路径跑通 | ✅ |

补充：`cargo clippy --target wasm32-unknown-unknown -p vane-wasm --all-targets -- -D warnings` clean。

## 8. 遗留 / Concerns

- **CI 远程未实跑**：本地 node 路径全绿；远程 ubuntu CI 首次运行需验证
  wasm-bindgen-cli 版本对齐（锁 0.2.127 与 lockfile 一致）。若远程 wasm-bindgen
  编译耗时过长，可考虑缓存 binary。
- **测试耗时**：单变体 wasm32 召回回归 ~14s（1000 文档 × 150 次搜索），
  双变体 ~28s，CI 可接受。
- **Jaccard 探针覆盖**：30 个 topK 集（5 查询 × 3 模式 × 2 档）足够检测变体分歧；
  未覆盖全部 50 档×查询组合（避免探针输出过大），但 recall 回归本身覆盖全五档。
- **f32 未向量化结论**：Jaccard=1.0 证实 M2-05 结论——simd128 flag 仅影响 roaring
  位图路径，不影响 f32 距离计算，故召回数值无分歧。若未来 core 引入手写 SIMD f32
  距离（如 WASM intrinsics），本门禁会捕获分歧。
