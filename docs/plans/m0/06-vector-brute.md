# Vector-Brute 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development`。步骤用 checkbox。
> 本计划属于 M0 L1 批次（与 01-vfs / 02-tokenizer / 03-fusion 并行），仅消费 00-workspace 产出的基础类型，无横向依赖。

**Goal:** 实现 M0 阶段的暴力向量检索（SPEC §8.1 vector 模式），对一段扁平 f32 向量数组做线性扫描，按指定 Metric 计算 score，用最小堆保留 topK，支持位图过滤。M0 不含 HNSW，本模块即 vector 路径的全部实现；M1 HNSW 落地后，本模块仍作为"过滤基数 < 2×topK 时的暴力精确回退"路径保留（SPEC §8.3）。

**Architecture:**
- 纯函数式入口 `brute_search`：无状态、无 IO、无分配副作用（除返回 Vec）。
- 单文件模块 `crates/vane-core/src/vector/mod.rs`，内部拆三个私有函数：`cosine_score` / `l2_score` / `dot_score`，由 `brute_search` 按 `Metric` 分派。
- topK 选择用 `std::collections::BinaryHeap` 最小堆（`Reverse` 包装），堆容量 ≤ topK，避免全排序。
- 过滤：`filter` 为 `Some(bitmap)` 时，只对 `bitmap` 中出现的 docid 扫描；`local_index = docid - docid_base` 必须落在 `[0, doc_count)` 内，越界的位图项静默跳过（防御性，不报错——调用方可能传跨段合并后的位图）。

**Tech Stack:**
- 纯算法，无外部依赖：`std` + `vane_core::types`（`ScoredDoc`, `Metric`）。
- 依赖 `roaring` crate（已在 00-workspace 加入 workspace，作为 filter 位图类型）。
- 不引入 `ndarray` / 任何 SIMD 库（§13.3 依赖黑名单；SIMD 是 M2 的事）。

**SPEC 引用:**
- §8.1 vector 模式：暴力扫描（M0），结果按向量距离排序。
- §3.1 Metric：`cosine` / `l2` / `dot`。
- §8.3 filter：位图过滤；M0 filter 传 `None`，但参数与语义预留。
- §13.1 暴力 hybrid topK=10 P99 < 150ms（10万×384维）——本模块是达成该承诺的核心路径。

**前置依赖:**
- 00-workspace（`ScoredDoc`, `Metric`, `VaneError`, `Result`, `DIM_MAX`）。

**验收标准:**
1. `cargo test -p vane-core vector::` 全绿。
2. `cargo check --target wasm32-unknown-unknown -p vane-core` 通过（核心零平台分支，不变量 I-5）。
3. `cargo clippy -p vane-core -- -D warnings` 通过。
4. 10万×384维 cosine topK=10 单次扫描在 dev 机器上 < 150ms（criterion benchmark，回归 >10% 报警由 10-ci-gates 接入）。
5. 三种 Metric 的 score 语义符合下表，结果按 score 降序、同分按 docid 升序（确定性输出）。
6. 边界用例全覆盖：空 vectors / topK=0 / topK>doc_count / dim 不匹配 / 零向量 / filter 位图。

### Score 语义（统一约定）

所有 Metric 统一为"score 越大越相似"，结果按 score 降序。这样 fusion 与排序逻辑无需感知 metric 差异。

| Metric | score 定义 | 范围 | 说明 |
|---|---|---|---|
| `Cosine` | `(a·b) / (\|a\|·\|b\|)` | [-1, 1] | 零向量（\|a\|=0 或 \|b\|=0）score = 0.0（视为无信息，不参与 NaN） |
| `L2` | `-\|a-b\|`（负欧氏距离） | (-∞, 0] | 距离越小 score 越大；完全相同 score = 0.0 |
| `Dot` | `a·b` | (-∞, +∞) | 未归一化点积，依赖向量本身模长 |

同分（浮点等值）时按 docid 升序排列，保证输出确定性。

---

## Global Constraints

遵守 `docs/plans/m0/README.md` 全局约束表，重点：

| 约束 | 本模块落实 |
|---|---|
| core 禁 `std::fs`/`std::net`/mmap | 本模块无 IO，天然满足 |
| 核心算法零 `cfg(target)` | 不出现任何 `cfg`；SIMD 留给 M2 |
| 依赖黑名单 | 不引入 ndarray / 任何 SIM D 库 |
| dim 上限 4096 | `brute_search` 入口校验 `dim <= DIM_MAX`，否则 `E_SCHEMA`（实际由上层 schema 校验，这里做防御性二次校验） |
| topK 上限 1000 | `brute_search` 不强制（上层 SearchQuery 校验）；本模块对任意 topK 正确工作 |
| 不变量 I-5 核心零平台分支 | 单文件纯算法，无 cfg |

NaN 防御：输入 f32 理论上不应含 NaN（向量来自段文件 / 查询构造）。本模块在 score 计算后做一次 `is_finite` 断言（debug 断言 + release 视为 -∞ 不入堆），防止 NaN 污染堆序。

---

## File Structure

```
crates/vane-core/src/
├── lib.rs                  # 00-workspace 产出，已 pub mod vector;
└── vector/
    └── mod.rs              # 本计划全部产出
```

`crates/vane-core/src/vector/mod.rs` 顶层结构：

```rust
//! 暴力向量检索（SPEC §8.1 vector 模式，M0）。
//!
//! 纯函数式：无状态、无 IO。消费 ScoredDoc / Metric，产出 brute_search。

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use vane_core::types::{Metric, ScoredDoc, DIM_MAX};

// ---- 公开入口 ----
pub fn brute_search(...) -> Vec<ScoredDoc>;

// ---- 私有：单向量 score ----
fn cosine_score(a: &[f32], b: &[f32]) -> f32;
fn l2_score(a: &[f32], b: &[f32]) -> f32;
fn dot_score(a: &[f32], b: &[f32]) -> f32;

// ---- 私有：堆元素包装（处理 f32 无 Ord + NaN）----
struct HeapEntry(Keyf32, u64);   // (score, docid)
struct Keyf32(f32);              // totalEq + totalOrd，NaN 视为 -∞
```

`lib.rs` 中 `pub mod vector;`（00-workspace 已在 lib.rs 一次性预声明全部 9 模块（含 `pub mod vector;`），本计划不修改 lib.rs（B1 裁决））。

> **注：** `VaneError`/`Result` 未在 `brute_search` 签名中使用（返回 `Vec` 而非 `Result`），不导入以避免 unused import 警告。错误码由上层 SearchQuery 校验产出。

---

## 任务清单（bite-sized TDD）

### Task 1: score 函数（cosine / l2 / dot）

**Files:** `crates/vane-core/src/vector/mod.rs`（新建）

**Interfaces:**
- Consumes from 00-workspace: `Metric`, `ScoredDoc`（本任务仅用 `Metric`，`ScoredDoc` 在 Task 2 用）
- Produces: 私有 `cosine_score` / `l2_score` / `dot_score`，以及 `Keyf32` / `HeapEntry` 包装类型

**目标：** 三种度量各自的 score 计算，含维度校验（a.len() == b.len()）、零向量处理（cosine）、NaN 防御。先写测试再写实现。

- [ ] **Step 1: 新建文件骨架与包装类型**

  创建 `crates/vane-core/src/vector/mod.rs`：

  ```rust
  //! 暴力向量检索（SPEC §8.1 vector 模式，M0）。

  use std::cmp::Reverse;
  use std::collections::BinaryHeap;

  use vane_core::types::{Metric, ScoredDoc, DIM_MAX};

  /// f32 的全序包装：NaN 视为 -∞（最小），保证 BinaryHeap 可用。
  /// 这是 score 排序的唯一真相源，避免 f32 无 Ord 导致堆污染。
  #[derive(Debug, Clone, Copy)]
  struct Keyf32(f32);

  impl Keyf32 {
      fn val(self) -> f32 { self.0 }
  }

  impl PartialEq for Keyf32 {
      fn eq(&self, other: &Self) -> bool { self.0.to_bits() == other.0.to_bits() }
  }
  impl Eq for Keyf32 {}

  impl PartialOrd for Keyf32 {
      fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
          Some(self.cmp(other))
      }
  }
  impl Ord for Keyf32 {
      fn cmp(&self, other: &Self) -> std::cmp::Ordering {
          // NaN 视为最小
          match (self.0.is_nan(), other.0.is_nan()) {
              (true, true) => std::cmp::Ordering::Equal,
              (true, false) => std::cmp::Ordering::Less,
              (false, true) => std::cmp::Ordering::Greater,
              (false, false) => self.0.total_cmp(&other.0),
          }
      }
  }

  /// 最小堆元素：(score, docid)。包 Reverse 后 BinaryHeap 行为为最小堆（堆顶=最小 score）。
  type MinHeap = BinaryHeap<Reverse<(Keyf32, u64)>>;
  ```

- [ ] **Step 2: 写测试（先于实现）**

  在文件末尾加 `#[cfg(test)] mod tests`，覆盖：

  ```rust
  #[cfg(test)]
  mod tests {
      use super::*;

      #[test]
      fn cosine_identical_vectors_is_one() {
          let a = [1.0_f32, 0.0, 0.0];
          let b = [1.0_f32, 0.0, 0.0];
          let s = cosine_score(&a, &b);
          assert!((s - 1.0).abs() < 1e-6, "got {s}");
      }

      #[test]
      fn cosine_orthogonal_is_zero() {
          let a = [1.0_f32, 0.0];
          let b = [0.0_f32, 1.0];
          assert!(cosine_score(&a, &b).abs() < 1e-6);
      }

      #[test]
      fn cosine_opposite_is_minus_one() {
          let a = [1.0_f32, 0.0];
          let b = [-1.0_f32, 0.0];
          assert!((cosine_score(&a, &b) - (-1.0)).abs() < 1e-6);
      }

      #[test]
      fn cosine_zero_vector_returns_zero() {
          // 零向量：|a|=0，无法归一化，约定返回 0.0（无信息），不得 NaN
          let a = [0.0_f32, 0.0, 0.0];
          let b = [1.0_f32, 2.0, 3.0];
          assert_eq!(cosine_score(&a, &b), 0.0);
          assert_eq!(cosine_score(&b, &a), 0.0);
          assert_eq!(cosine_score(&a, &a), 0.0);
      }

      #[test]
      fn cosine_dim_mismatch_panics_in_debug() {
          // debug 下断言维度一致；release 下行为未定义但不得 UB
          // 这里用 debug_assert_eq，release 跳过（上层保证维度，本层防御）
          let a = [1.0_f32, 0.0];
          let b = [1.0_f32];
          debug_assert_eq!(a.len(), b.len());
      }

      #[test]
      fn l2_identical_is_zero() {
          let a = [1.0_f32, 2.0, 3.0];
          assert_eq!(l2_score(&a, &a), 0.0); // score = -|a-b| = 0
      }

      #[test]
      fn l2_distance_negated() {
          let a = [0.0_f32, 0.0];
          let b = [3.0_f32, 4.0];
          // |a-b| = 5, score = -5
          assert!((l2_score(&a, &b) - (-5.0)).abs() < 1e-6);
      }

      #[test]
      fn l2_larger_distance_lower_score() {
          // 距离越大 score 越小（越负）
          let a = [0.0_f32];
          let near = [1.0_f32];
          let far = [10.0_f32];
          assert!(l2_score(&a, &near) > l2_score(&a, &far));
      }

      #[test]
      fn dot_basic() {
          let a = [1.0_f32, 2.0, 3.0];
          let b = [4.0_f32, 5.0, 6.0];
          // 4+10+18 = 32
          assert!((dot_score(&a, &b) - 32.0).abs() < 1e-5);
      }

      #[test]
      fn dot_orthogonal_is_zero() {
          let a = [1.0_f32, 0.0];
          let b = [0.0_f32, 5.0];
          assert!(dot_score(&a, &b).abs() < 1e-6);
      }

      #[test]
      fn dot_can_be_negative() {
          let a = [1.0_f32, -1.0];
          let b = [-1.0_f32, 1.0];
          // -1 + -1 = -2
          assert!((dot_score(&a, &b) - (-2.0)).abs() < 1e-6);
      }

      #[test]
      fn keyf32_orders_nan_as_min() {
          let nan = Keyf32(f32::NAN);
          let neg = Keyf32(-1.0_f32);
          assert!(nan < neg);
          assert!(neg > nan);
      }

      #[test]
      fn keyf32_eq_bitwise() {
          assert_ne!(Keyf32(0.0_f32), Keyf32(-0.0_f32)); // bits 不同 -> 不等
          assert_eq!(Keyf32(1.5), Keyf32(1.5));
      }
  }
  ```

- [ ] **Step 3: 实现三个 score 函数**

  ```rust
  /// cosine 相似度 = (a·b) / (|a|·|b|)。零向量返回 0.0。
  ///
  /// 维度校验：debug_assert a.len() == b.len()（上层保证；本层防御性）。
  fn cosine_score(a: &[f32], b: &[f32]) -> f32 {
      debug_assert_eq!(a.len(), b.len(), "cosine_score: dim mismatch");
      let mut dot = 0.0_f32;
      let mut na = 0.0_f32;
      let mut nb = 0.0_f32;
      for i in 0..a.len() {
          dot += a[i] * b[i];
          na += a[i] * a[i];
          nb += b[i] * b[i];
      }
      let denom = na.sqrt() * nb.sqrt();
      if denom == 0.0_f32 || !denom.is_finite() {
          return 0.0_f32; // 零向量或溢出，无信息
      }
      dot / denom
  }

  /// L2 score = -|a-b|（负欧氏距离，越大越相似）。
  fn l2_score(a: &[f32], b: &[f32]) -> f32 {
      debug_assert_eq!(a.len(), b.len(), "l2_score: dim mismatch");
      let mut sum_sq = 0.0_f32;
      for i in 0..a.len() {
          let d = a[i] - b[i];
          sum_sq += d * d;
      }
      -sum_sq.sqrt()
  }

  /// dot score = a·b（未归一化点积）。
  fn dot_score(a: &[f32], b: &[f32]) -> f32 {
      debug_assert_eq!(a.len(), b.len(), "dot_score: dim mismatch");
      let mut s = 0.0_f32;
      for i in 0..a.len() {
          s += a[i] * b[i];
      }
      s
  }
  ```

- [ ] **Step 4: 运行测试，确认全绿**

  ```bash
  cargo test -p vane-core vector::tests
  cargo clippy -p vane-core -- -D warnings
  ```

  若 `vane_core::types` 路径解析失败，确认 00-workspace 已在 `lib.rs` 暴露 `pub mod types;` 且 `Metric`/`ScoredDoc`/`VaneError`/`DIM_MAX` 可见。本计划假设 00-workspace 已合并。

- [ ] **Step 5: wasm32 check**

  ```bash
  cargo check --target wasm32-unknown-unknown -p vane-core
  ```

  确认无 `cfg`、无平台分支。

---

### Task 2: brute_search topK（最小堆 + filter + docid_base 偏移）

**Files:** `crates/vane-core/src/vector/mod.rs`

**Interfaces:**
- Consumes from 00-workspace: `ScoredDoc`, `Metric`, `VaneError`, `Result`, `DIM_MAX`
- Produces: `pub fn brute_search(...)`（签名见模块契约）

**目标：** 实现公开入口。遍历向量数组，按 metric 分派 score 函数，用最小堆保留 topK。filter 为 `Some` 时只扫描位图中的 docid。

- [ ] **Step 1: 写测试（先于实现）**

  在 `tests` 模块追加：

  ```rust
  use vane_core::types::{Metric, ScoredDoc};
  use roaring::RoaringBitmap;

  fn approx_eq(a: f32, b: f32) -> bool { (a - b).abs() < 1e-5 }

  #[test]
  fn brute_cosine_topk_basic() {
      // 4 个 2 维向量，query=[1,0]
      // cosine: v0=[1,0]->1.0, v1=[0,1]->0.0, v2=[-1,0]->-1.0, v3=[1,1]->0.7071
      let vectors: Vec<f32> = vec![
          1.0, 0.0,
          0.0, 1.0,
          -1.0, 0.0,
          1.0, 1.0,
      ];
      let query = [1.0_f32, 0.0];
      let res = brute_search(&vectors, 2, &query, Metric::Cosine, 2, None, 0);
      assert_eq!(res.len(), 2);
      // 降序：v0 (1.0) > v3 (0.7071)
      assert_eq!(res[0].docid, 0);
      assert!(approx_eq(res[0].score, 1.0));
      assert_eq!(res[1].docid, 3);
      assert!(approx_eq(res[1].score, 1.0_f32 / 2.0_f32.sqrt()));
  }

  #[test]
  fn brute_l2_topk_order() {
      // query=[0,0]，最近的是 v0=[1,0]（dist=1），次近 v1=[2,0]（dist=2）
      let vectors: Vec<f32> = vec![1.0, 0.0, 2.0, 0.0, 5.0, 0.0];
      let query = [0.0_f32, 0.0];
      let res = brute_search(&vectors, 2, &query, Metric::L2, 2, None, 100);
      assert_eq!(res.len(), 2);
      assert_eq!(res[0].docid, 100); // docid_base 偏移
      assert!(approx_eq(res[0].score, -1.0));
      assert_eq!(res[1].docid, 101);
      assert!(approx_eq(res[1].score, -2.0));
  }

  #[test]
  fn brute_dot_topk() {
      let vectors: Vec<f32> = vec![1.0, 1.0, 2.0, 2.0]; // v0 dot q=1, v1 dot q=2
      let query = [1.0_f32, 1.0];
      let res = brute_search(&vectors, 2, &query, Metric::Dot, 2, None, 0);
      assert_eq!(res[0].docid, 1);
      assert!(approx_eq(res[0].score, 4.0));
      assert_eq!(res[1].docid, 0);
      assert!(approx_eq(res[1].score, 2.0));
  }

  #[test]
  fn brute_filter_only_scanned_docs_in_bitmap() {
      let vectors: Vec<f32> = vec![1.0, 0.0, 1.0, 0.0, 1.0, 0.0]; // 3 个相同向量
      let query = [1.0_f32, 0.0];
      let mut bm = RoaringBitmap::new();
      bm.insert(1); // 只扫 local_index=1 -> docid = 1000+1 = 1001
      let res = brute_search(&vectors, 2, &query, Metric::Cosine, 2, Some(&bm), 1000);
      assert_eq!(res.len(), 1);
      assert_eq!(res[0].docid, 1001);
  }

  #[test]
  fn brute_filter_bitmap_out_of_range_skipped() {
      // 位图含 docid 超出段范围（local_index >= doc_count），静默跳过
      let vectors: Vec<f32> = vec![1.0, 0.0]; // 1 个向量
      let query = [1.0_f32, 0.0];
      let mut bm = RoaringBitmap::new();
      bm.insert(0);
      bm.insert(999); // 越界
      let res = brute_search(&vectors, 2, &query, Metric::Cosine, 2, Some(&bm), 0);
      assert_eq!(res.len(), 1);
      assert_eq!(res[0].docid, 0);
  }

  #[test]
  fn brute_docid_base_offset_applied() {
      let vectors: Vec<f32> = vec![1.0, 0.0, 1.0, 0.0];
      let query = [1.0_f32, 0.0];
      let res = brute_search(&vectors, 2, &query, Metric::Cosine, 2, None, 42);
      assert_eq!(res[0].docid, 42);
      assert_eq!(res[1].docid, 43);
  }

  #[test]
  fn brute_topk_full_results_when_eq_doc_count() {
      let vectors: Vec<f32> = vec![1.0, 0.0, 0.0, 1.0];
      let query = [1.0_f32, 0.0];
      let res = brute_search(&vectors, 2, &query, Metric::Cosine, 5, None, 0);
      assert_eq!(res.len(), 2); // 只有 2 个 doc，topK=5 也只返回 2
  }

  #[test]
  fn brute_results_sorted_desc_by_score() {
      // 随机乱序向量，验证输出严格降序（允许同分按 docid 升序）
      let vectors: Vec<f32> = vec![
          0.1, 0.0,
          1.0, 0.0,
          0.5, 0.0,
          -1.0, 0.0,
      ];
      let query = [1.0_f32, 0.0];
      let res = brute_search(&vectors, 2, &query, Metric::Cosine, 4, None, 0);
      assert_eq!(res.len(), 4);
      for w in res.windows(2) {
          assert!(w[0].score >= w[1].score, "not desc: {:?} vs {:?}", w[0], w[1]);
      }
      assert_eq!(res[0].docid, 1); // 最相似
      assert_eq!(res[3].docid, 3); // 最不相似
  }

  #[test]
  fn brute_tie_break_by_docid_ascending() {
      // 两个相同向量，同分，docid 小的排前
      let vectors: Vec<f32> = vec![1.0, 0.0, 1.0, 0.0];
      let query = [1.0_f32, 0.0];
      let res = brute_search(&vectors, 2, &query, Metric::Cosine, 2, None, 10);
      assert_eq!(res[0].docid, 10);
      assert_eq!(res[1].docid, 11);
      assert!(approx_eq(res[0].score, res[1].score));
  }
  ```

- [ ] **Step 2: 实现 brute_search**

  ```rust
  /// 暴力向量扫描（SPEC §8.1 vector 模式；M0 无 HNSW）。
  ///
  /// - `vectors`: 扁平 f32 数组，doc i 的向量 = `vectors[i*dim .. (i+1)*dim]`
  /// - `dim`: 单向量维度；必须 `query.len() == dim` 且 `vectors.len() % dim == 0`
  /// - `query`: 查询向量
  /// - `metric`: 距离度量（cosine / l2 / dot）
  /// - `topk`: 返回前 topk 个；实际返回数 `min(topk, 命中文档数)`
  /// - `filter`: `Some(bitmap)` 时只扫描位图中的 docid；
  ///   `local_index = docid - docid_base`，越界项静默跳过
  /// - `docid_base`: 段内起始 docid，结果 docid = `docid_base + local_index`
  ///
  /// 返回：按 score 降序，同分按 docid 升序。空输入返回空 Vec。
  ///
  /// 错误：`dim == 0` 或 `dim > DIM_MAX` 或 `query.len() != dim` 返回 `E_SCHEMA`；
  ///       `vectors.len()` 不是 `dim` 的整数倍返回 `E_SCHEMA`。
  pub fn brute_search(
      vectors: &[f32],
      dim: u32,
      query: &[f32],
      metric: Metric,
      topk: usize,
      filter: Option<&roaring::RoaringBitmap>,
      docid_base: u64,
  ) -> Vec<ScoredDoc> {
      // ---- 维度校验（不 panic，返回 Vec 空或由上层 Result 包装）----
      // 注：本函数签名返回 Vec（非 Result），因为 07-api-core 调用时上层
      // 已做 schema 校验。这里做防御性早返回：非法输入返回空 Vec，
      // 避免后续索引越界 panic。严格错误码由上层 SearchQuery 校验产出。
      if dim == 0 || dim > DIM_MAX {
          return Vec::new();
      }
      let dim = dim as usize;
      if query.len() != dim || vectors.len() % dim != 0 {
          return Vec::new();
      }
      if topk == 0 {
          return Vec::new();
      }
      let doc_count = vectors.len() / dim;
      if doc_count == 0 {
          return Vec::new();
      }

      // ---- score 分派 ----
      let score_fn: fn(&[f32], &[f32]) -> f32 = match metric {
          Metric::Cosine => cosine_score,
          Metric::L2 => l2_score,
          Metric::Dot => dot_score,
      };

      // ---- 最小堆保留 topK ----
      // 堆元素 Reverse<(Keyf32, u64)>：BinaryHeap 是最大堆，Reverse 后堆顶=最小 score。
      // 堆满（size > topk）时弹出最小，保留 topK 个最大。
      let mut heap: BinaryHeap<Reverse<(Keyf32, u64)>> = BinaryHeap::with_capacity(topk + 1);

      let push_if_better = |heap: &mut BinaryHeap<Reverse<(Keyf32, u64)>>, score: f32, docid: u64| {
          // NaN 防御：score 非 finite 视为 -∞，仍可入堆但不会胜出
          let key = if score.is_finite() { Keyf32(score) } else { Keyf32(f32::NEG_INFINITY) };
          heap.push(Reverse((key, docid)));
          if heap.len() > topk {
              heap.pop(); // 弹出最小
          }
      };

      match filter {
          None => {
              for i in 0..doc_count {
                  let v = &vectors[i * dim..(i + 1) * dim];
                  let s = score_fn(v, query);
                  push_if_better(&mut heap, s, docid_base + i as u64);
              }
          }
          Some(bm) => {
              // 只扫描位图中的 docid；local_index = docid - docid_base
              for docid in bm.iter() {
                  // 越界静默跳过（防御性：调用方可能传跨段合并位图）
                  if docid < docid_base {
                      continue;
                  }
                  let local = docid - docid_base;
                  if local as usize >= doc_count {
                      continue;
                  }
                  let v = &vectors[local as usize * dim..(local as usize + 1) * dim];
                  let s = score_fn(v, query);
                  push_if_better(&mut heap, s, docid);
              }
          }
      }

      // ---- 堆 -> 有序 Vec（降序，同分 docid 升序）----
      let mut out: Vec<ScoredDoc> = Vec::with_capacity(heap.len());
      while let Some(Reverse((key, docid))) = heap.pop() {
          out.push(ScoredDoc { docid, score: key.val() });
      }
      // pop 出来是升序（最小堆），反转得降序
      out.reverse();

      // 同分按 docid 升序：stable sort 在 score 降序基础上对同分组保持 docid 升序
      // （因为上面遍历 docid 升序入堆，pop 出来同分是降序，reverse 后恢复升序）
      // 这里用 sort_by 做显式保证，避免依赖入堆顺序的隐式不变量。
      out.sort_by(|a, b| {
          // score 降序（b.score vs a.score）；同分 docid 升序（a.docid vs b.docid）
          match Keyf32(b.score).cmp(&Keyf32(a.score)) {
              std::cmp::Ordering::Equal => a.docid.cmp(&b.docid),
              other => other,
          }
      });

      out
  }
  ```

  关于 `push_if_better` 闭包借用 `topk`：`topk` 是 `usize`（Copy），闭包按值捕获即可。`BinaryHeap` 类型在闭包里以 `&mut` 借用，闭包签名需标注生命周期或用 `|heap: &mut ..., ...|`。上面的写法在闭包内捕获 `topk`（Copy）没问题；但 Rust 闭包捕获 `topk` 是不可变借用，而 `topk` 后续不再使用，编译通过。若编译器抱怨，改为内联循环体（不抽闭包），逻辑等价。

  若闭包写法编译失败，替换为内联（等价、更稳）：

  ```rust
  // 内联版本（不抽闭包）：
  None => {
      for i in 0..doc_count {
          let v = &vectors[i * dim..(i + 1) * dim];
          let s = score_fn(v, query);
          let key = if s.is_finite() { Keyf32(s) } else { Keyf32(f32::NEG_INFINITY) };
          heap.push(Reverse((key, docid_base + i as u64)));
          if heap.len() > topk { heap.pop(); }
      }
  }
  Some(bm) => {
      for docid in bm.iter() {
          if docid < docid_base { continue; }
          let local = docid - docid_base;
          if local as usize >= doc_count { continue; }
          let v = &vectors[local as usize * dim..(local as usize + 1) * dim];
          let s = score_fn(v, query);
          let key = if s.is_finite() { Keyf32(s) } else { Keyf32(f32::NEG_INFINITY) };
          heap.push(Reverse((key, docid)));
          if heap.len() > topk { heap.pop(); }
      }
  }
  ```
  实现时优先用内联版本，避免闭包借用纠纷。

- [ ] **Step 3: 运行全部测试**

  ```bash
  cargo test -p vane-core vector::
  cargo clippy -p vane-core -- -D warnings
  cargo check --target wasm32-unknown-unknown -p vane-core
  ```

  确认 Task 1 + Task 2 全绿。

- [ ] **Step 4: 微基准（手动验证性能承诺）**

  写一个 `#[ignore]` 的性能测试，验证 10万×384维 < 150ms（dev 机器粗验，正式 benchmark 在 10-ci-gates 接入 criterion）：

  ```rust
  #[test]
  #[ignore]
  fn perf_100k_384_cosine_top10() {
      let dim = 384_usize;
      let n = 100_000_usize;
      let vectors: Vec<f32> = (0..(n * dim))
          .map(|i| ((i as u32).wrapping_mul(2654435761) as f32) / (u32::MAX as f32))
          .collect();
      let query: Vec<f32> = (0..dim).map(|i| i as f32 / dim as f32).collect();
      let start = std::time::Instant::now();
      let res = brute_search(&vectors, dim as u32, &query, Metric::Cosine, 10, None, 0);
      let elapsed = start.elapsed();
      assert_eq!(res.len(), 10);
      eprintln!("100k x 384 cosine top10: {:?}", elapsed);
      assert!(elapsed.as_millis() < 150, "P99 预算超限: {:?}", elapsed);
  }
  ```

  ```bash
  cargo test -p vane-core vector::tests::perf_100k_384_cosine_top10 -- --ignored --nocapture
  ```

  若超 150ms：优先检查是否未开 release 优化（`cargo test --release`）；确认 release 下达标。dev 默认 debug 会慢 5-10 倍，性能测试必须 `--release`。

- [ ] **Step 5: 提交**

  ```bash
  git add crates/vane-core/src/vector/mod.rs
  git commit -m "feat(vector): brute_search with cosine/l2/dot + topK min-heap + filter

  SPEC §8.1 vector 模式 M0 实现。score 统一为越大越相似：
  cosine=相似度, l2=-距离, dot=点积。最小堆保留 topK，
  位图过滤支持越界跳过。

  "
  ```

---

### Task 3: 边界与错误用例

**Files:** `crates/vane-core/src/vector/mod.rs`（追加测试）

**Interfaces:**
- Consumes from 00-workspace: `Metric`, `ScoredDoc`
- Produces: 无新接口，仅补充边界测试覆盖

**目标：** 覆盖所有边界条件，确保非法输入不 panic、返回空 Vec 或正确结果。

- [ ] **Step 1: 追加边界测试**

  在 `tests` 模块追加：

  ```rust
  #[test]
  fn brute_empty_vectors_returns_empty() {
      let query = [1.0_f32, 0.0];
      let res = brute_search(&[], 2, &query, Metric::Cosine, 10, None, 0);
      assert!(res.is_empty());
  }

  #[test]
  fn brute_topk_zero_returns_empty() {
      let vectors: Vec<f32> = vec![1.0, 0.0, 0.0, 1.0];
      let query = [1.0_f32, 0.0];
      let res = brute_search(&vectors, 2, &query, Metric::Cosine, 0, None, 0);
      assert!(res.is_empty());
  }

  #[test]
  fn brute_topk_exceeds_doc_count_returns_all() {
      // topK=10 但只有 3 个 doc，返回 3 个（降序）
      let vectors: Vec<f32> = vec![
          1.0, 0.0,
          0.5, 0.0,
          0.1, 0.0,
      ];
      let query = [1.0_f32, 0.0];
      let res = brute_search(&vectors, 2, &query, Metric::Cosine, 10, None, 0);
      assert_eq!(res.len(), 3);
      // 降序
      assert!(res[0].score >= res[1].score);
      assert!(res[1].score >= res[2].score);
      assert_eq!(res[0].docid, 0);
      assert_eq!(res[1].docid, 1);
      assert_eq!(res[2].docid, 2);
  }

  #[test]
  fn brute_dim_zero_returns_empty() {
      let query: [f32; 0] = [];
      let res = brute_search(&[], 0, &query, Metric::Cosine, 10, None, 0);
      assert!(res.is_empty());
  }

  #[test]
  fn brute_dim_exceeds_max_returns_empty() {
      // DIM_MAX = 4096；dim=4097 应被拒
      let dim = 4097_u32;
      let query = vec![0.0_f32; dim as usize];
      let vectors = vec![0.0_f32; dim as usize];
      let res = brute_search(&vectors, dim, &query, Metric::Cosine, 1, None, 0);
      assert!(res.is_empty(), "dim > DIM_MAX should return empty");
  }

  #[test]
  fn brute_dim_just_at_max_ok() {
      let dim = 4096_u32;
      let query = vec![1.0_f32; dim as usize];
      let vectors = vec![1.0_f32; dim as usize]; // 1 个向量
      let res = brute_search(&vectors, dim, &query, Metric::Cosine, 1, None, 0);
      assert_eq!(res.len(), 1);
      assert!(approx_eq(res[0].score, 1.0));
  }

  #[test]
  fn brute_query_dim_mismatch_returns_empty() {
      // query.len() != dim
      let vectors: Vec<f32> = vec![1.0, 0.0, 0.0, 1.0]; // dim=2, 2 docs
      let query = [1.0_f32, 0.0, 0.0]; // len=3 != 2
      let res = brute_search(&vectors, 2, &query, Metric::Cosine, 2, None, 0);
      assert!(res.is_empty());
  }

  #[test]
  fn brute_vectors_not_multiple_of_dim_returns_empty() {
      // vectors.len()=5 不是 dim=2 的整数倍
      let vectors: Vec<f32> = vec![1.0, 0.0, 0.0, 1.0, 0.5];
      let query = [1.0_f32, 0.0];
      let res = brute_search(&vectors, 2, &query, Metric::Cosine, 2, None, 0);
      assert!(res.is_empty());
  }

  #[test]
  fn brute_filter_empty_bitmap_returns_empty() {
      let vectors: Vec<f32> = vec![1.0, 0.0, 0.0, 1.0];
      let query = [1.0_f32, 0.0];
      let bm = RoaringBitmap::new(); // 空
      let res = brute_search(&vectors, 2, &query, Metric::Cosine, 2, Some(&bm), 0);
      assert!(res.is_empty());
  }

  #[test]
  fn brute_filter_all_out_of_range_returns_empty() {
      let vectors: Vec<f32> = vec![1.0, 0.0]; // 1 doc
      let query = [1.0_f32, 0.0];
      let mut bm = RoaringBitmap::new();
      bm.insert(100);
      bm.insert(200);
      let res = brute_search(&vectors, 2, &query, Metric::Cosine, 2, Some(&bm), 0);
      assert!(res.is_empty());
  }

  #[test]
  fn brute_filter_with_docid_base_offset() {
      // docid_base=50；位图含 docid=51 -> local=1
      let vectors: Vec<f32> = vec![
          0.0, 0.0, 0.0, 0.0,   // local 0: 零向量
          1.0, 0.0, 0.0, 0.0,   // local 1
      ];
      let query = [1.0_f32, 0.0, 0.0, 0.0];
      let mut bm = RoaringBitmap::new();
      bm.insert(50); // local 0
      bm.insert(51); // local 1
      let res = brute_search(&vectors, 4, &query, Metric::Cosine, 2, Some(&bm), 50);
      assert_eq!(res.len(), 2);
      assert_eq!(res[0].docid, 51); // cosine=1.0
      assert_eq!(res[1].docid, 50); // 零向量 cosine=0.0
  }

  #[test]
  fn brute_filter_below_docid_base_skipped() {
      // 位图含 docid < docid_base，静默跳过
      let vectors: Vec<f32> = vec![1.0, 0.0]; // 1 doc
      let query = [1.0_f32, 0.0];
      let mut bm = RoaringBitmap::new();
      bm.insert(0);  // < docid_base=100，跳过
      bm.insert(100); // local 0
      let res = brute_search(&vectors, 2, &query, Metric::Cosine, 2, Some(&bm), 100);
      assert_eq!(res.len(), 1);
      assert_eq!(res[0].docid, 100);
  }

  #[test]
  fn brute_single_vector_returns_one() {
      let vectors: Vec<f32> = vec![1.0, 0.0];
      let query = [1.0_f32, 0.0];
      let res = brute_search(&vectors, 2, &query, Metric::Cosine, 1, None, 0);
      assert_eq!(res.len(), 1);
      assert_eq!(res[0].docid, 0);
      assert!(approx_eq(res[0].score, 1.0));
  }

  #[test]
  fn brute_all_three_metrics_on_same_data() {
      // 同一份数据跑三种 metric，确保都返回 topK 且不 panic
      let vectors: Vec<f32> = vec![
          1.0, 0.0,
          0.0, 1.0,
          1.0, 1.0,
          -1.0, -1.0,
      ];
      let query = [1.0_f32, 1.0];
      for metric in [Metric::Cosine, Metric::L2, Metric::Dot] {
          let res = brute_search(&vectors, 2, &query, metric, 2, None, 0);
          assert_eq!(res.len(), 2, "metric {:?} returned wrong len", metric);
          assert!(res[0].score >= res[1].score, "metric {:?} not desc", metric);
      }
  }

  #[test]
  fn brute_nan_in_query_does_not_panic() {
      // 防御性：query 含 NaN（不应发生，但要保证不 panic、不污染堆序）
      let vectors: Vec<f32> = vec![1.0, 0.0, 0.0, 1.0];
      let query = [f32::NAN, 0.0];
      let res = brute_search(&vectors, 2, &query, Metric::Cosine, 2, None, 0);
      // 不 panic 即可；结果数仍为 min(topk, doc_count)
      assert!(res.len() <= 2);
  }
  ```

- [ ] **Step 2: 运行全部测试**

  ```bash
  cargo test -p vane-core vector::
  ```

  确认所有边界用例通过。若 `brute_nan_in_query_does_not_panic` 失败（NaN 导致堆序错乱），检查 `Keyf32` 的 `Ord` 实现是否把 NaN 归一为 `-∞`（Task 1 Step 1 的实现已处理）。

- [ ] **Step 3: clippy + wasm32 终检**

  ```bash
  cargo clippy -p vane-core -- -D warnings
  cargo check --target wasm32-unknown-unknown -p vane-core
  cargo fmt -- --check
  ```

- [ ] **Step 4: 覆盖率自检（可选）**

  ```bash
  cargo tarpaulin -p vane-core --lib vector --out Html
  ```

  目标：`vector/mod.rs` 行覆盖 ≥ 95%。若 `push_if_better` 闭包版本被内联替代，确认无死代码。

- [ ] **Step 5: 提交**

  ```bash
  git add crates/vane-core/src/vector/mod.rs
  git commit -m "test(vector): boundary cases - empty/topk0/dim mismatch/filter edges

  覆盖空 vectors、topK=0、topK>doc_count、dim=0、dim>DIM_MAX、
  query 维度不匹配、vectors 非整数倍、空位图、位图越界、
  docid<docid_base 跳过、NaN 防御等边界。

  "
  ```

---

## 完成清单（提交前自检）

- [ ] `brute_search` 签名与模块契约完全一致
- [ ] 三种 Metric score 语义符合约定表（cosine=相似度, l2=-距离, dot=点积）
- [ ] 结果按 score 降序、同分 docid 升序（确定性）
- [ ] filter 位图过滤：越界 / docid<base 静默跳过
- [ ] docid_base 偏移正确
- [ ] topK 上限不强制（上层校验），topK=0 / topK>doc_count 行为正确
- [ ] 非法输入（dim=0 / dim>DIM_MAX / 维度不匹配）返回空 Vec，不 panic
- [ ] NaN 防御：不 panic、不污染堆序
- [ ] 核心零 `cfg`、零平台分支（不变量 I-5）
- [ ] `cargo test -p vane-core vector::` 全绿
- [ ] `cargo clippy -p vane-core -- -D warnings` 通过
- [ ] `cargo check --target wasm32-unknown-unknown -p vane-core` 通过
- [ ] 性能：10万×384 cosine top10 release 下 < 150ms（`--ignored` 验证）

## 与下游计划（07-api-core）的对接

07-api-core 的 `Collection::search` 在 vector / hybrid 模式下调用本模块：

```rust
// 07-api-core 伪代码（仅说明调用方式，不在本计划实现）
let scored: Vec<ScoredDoc> = segment_reader.vectors_chunks().flat_map(|chunk| {
    vane_core::vector::brute_search(
        chunk.vectors,       // &[f32] 扁平数组
        chunk.dim,           // u32
        query_vec,           // &[f32]
        metric,              // Metric
        topk * candidate_multiplier as usize,
        filter_bitmap,       // M0 传 None
        chunk.docid_base,    // u64
    )
}).collect();
// 多段结果再归并（也复用最小堆，07-api-core 负责）
```

M0 单段场景下直接调用即可。多段归并的跨段 topK 由 07-api-core 实现（对本模块透明）。

## 与 M1 的衔接预留

- M1 HNSW 落地后，本模块作为"过滤基数 < 2×topK 时的暴力精确回退"保留（SPEC §8.3），签名不变。
- M1 pre-filter 启用时，`filter` 参数从 `None` 变为实际位图，本模块已支持。
- 未来 SIMD 优化（M2）仅替换 `cosine_score` / `l2_score` / `dot_score` 内部实现，公开签名与 score 语义不变。
