# Fusion 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development。步骤用 checkbox。
>
> **Goal:** 实现 SPEC §8.2 两种融合算法——RRF（k=60 冻结）与 linear（minmax 归一化 + alpha 加权），作为 L1 阶段独立模块，仅依赖 `vane_core::types` 中的 `ScoredDoc` 与常量 `RRF_K`，零外部 crate。
>
> **Architecture:** `vane_core::fusion` 为纯函数模块，无状态、无 IO、无 `cfg`。三组公开项：`FusionCandidate` + `rrf_fuse`、`LinearInput` + `minmax_normalize`、`linear_fuse`。输入候选的 `rank` 由调用方按 `score` 降序预先编号（从 0 起），融合算法不重新排序输入、只对结果降序输出。RRF 使用 `HashMap<u64, f32>` 累加各路 `1/(k+rank)`；linear 取两路 `docid` 并集，缺路记 `score=0.0`。
>
> **Tech Stack:** `std` + `vane_core::types`。禁止引入任何外部 crate（包括 `hashbrown`、`ahash`）。
>
> **SPEC 引用:** §8.2（融合）、§14 不变量 I-5（核心零平台分支）。
>
> **前置依赖:** 00-workspace（`ScoredDoc`、`Result`、常量 `RRF_K=60`）。
>
> **验收标准:**
> 1. `cargo test -p vane-core fusion` 全绿。
> 2. `cargo check --target wasm32-unknown-unknown -p vane-core` 通过（I-5）。
> 3. `cargo clippy -p vane-core` 无 warning。
> 4. 签名与 `docs/plans/m0/README.md` 的 `03-fusion 产出` 节完全一致（结构体字段、函数参数顺序、可见性）。
> 5. RRF 结果按分数降序；linear 结果按分数降序；同分时 `docid` 升序作为稳定 tie-break。
> 6. 不在任一路出现的 `docid` 不出现在 RRF 结果；linear 两路取并集、缺路记 0。

---

## Global Constraints

- **RRF k=60 冻结**（SPEC §8.2 / 全局约束表）。本模块不持有 k 的默认值——`rrf_fuse(paths, k)` 由调用方传入，调用方应传 `vane_core::types::RRF_K`。模块内不定义 `RRF_K` 常量（避免重复定义）。测试中显式用 `60` 验证"冻结"语义，并加一条用例确认 `RRF_K == 60`。
- **linear 归一化仅 minmax**，按当次候选集归一化（SPEC §8.2）。不提供 z-score / softmax 等其他归一化。
- **API 默认路径不出现 alpha**（SPEC §8.2）：本模块不提供 `alpha` 默认值，`linear_fuse` 必须显式接收 `alpha`。默认融合策略由 07-api-core 的 `FusionSpec::Rrf` 决定，不在本模块编码。
- **不引入外部依赖**：仅 `std` + `vane_core::types`。`Cargo.toml` 不新增 dependency。
- **核心零 `cfg`**（I-5）：`fusion/mod.rs` 不得出现任何 `cfg(...)`。
- **数值稳定性**：minmax 在 `max == min` 时（空集、单元素、全相同分数）所有归一化值记 `0.0`（避免除零；SPEC 未规定具体值，选 0.0 保证 linear 缺路语义一致）。
- **稳定性**：结果按 `score` 降序；同分按 `docid` 升序。使用 `sort_by` 而非 `sort_by_key` 以显式控制次序。
- **rank 语义**：`FusionCandidate.rank` 从 0 开始；`1/(k+rank)` 中 `rank=0` 给最高分项 `1/k`。调用方负责保证每路 `rank` 已按 `score` 降序连续编号。

---

## File Structure

```
crates/vane-core/
└── src/
    ├── lib.rs              # 00-workspace 产出；本计划不改其结构，仅确认 fusion 模块挂载点
    ├── types.rs            # 00-workspace 产出；本计划只读消费 ScoredDoc / RRF_K
    └── fusion/
        ├── mod.rs          # 本计划唯一新增文件：全部公开项 + 实现
        └── tests.rs        # 内联单元测试（#[cfg(test)] 模块也可，但独立文件更清晰）
```

挂载约定：00-workspace 已在 lib.rs 一次性预声明全部 9 模块（含 `pub mod fusion;`），本计划不修改 lib.rs（B1 裁决）。本计划仅写 `fusion/mod.rs` 与 `fusion/tests.rs`。

---

## 任务清单（bite-sized TDD）

### Task 0: 模块挂载确认

**Files:** `crates/vane-core/src/fusion/mod.rs`（新建空壳）、`crates/vane-core/src/fusion/tests.rs`（新建空文件）

**Interfaces:**
- Consumes from 00-workspace: 无（仅确认挂载点）
- Produces: 模块文件骨架

- [ ] **Step 1:** 确认 00-workspace 已在 lib.rs 一次性预声明全部 9 模块（含 `pub mod fusion;`），本计划不修改 lib.rs（B1 裁决）。直接进入 Step 2。
- [ ] **Step 2:** 新建 `crates/vane-core/src/fusion/mod.rs`，写入最小可编译内容：
  ```rust
  //! SPEC §8.2 融合算法：RRF(k=60) + linear(minmax)。
  //! 纯函数模块，无状态、无 IO、无 cfg。

  use vane_core::types::ScoredDoc;

  pub struct FusionCandidate {
      pub docid: u64,
      pub rank: u32,
      pub score: f32,
  }

  pub struct LinearInput {
      pub docid: u64,
      pub score: f32,
  }
  ```
- [ ] **Step 3:** 新建 `crates/vane-core/src/fusion/tests.rs`，写入：
  ```rust
  // fusion/tests.rs — Task 1 起填充真实测试
  ```
- [ ] **Step 4:** 在 `mod.rs` 末尾追加 `#[cfg(test)] mod tests;`（指向 `tests.rs`）。
- [ ] **Step 5:** 运 `cargo check -p vane-core`，确认 fusion 模块编译通过（空 tests.rs）。

---

### Task 1: `rrf_fuse` —— RRF 融合主算法

**Files:** `crates/vane-core/src/fusion/mod.rs`（实现）、`crates/vane-core/src/fusion/tests.rs`（测试）

**Interfaces:**
- Consumes from 00-workspace: `ScoredDoc`
- Produces: `FusionCandidate`（结构体，Task 0 已声明字段）、`pub fn rrf_fuse(paths: &[Vec<FusionCandidate>], k: u32) -> Vec<ScoredDoc>`

**算法（SPEC §8.2）：**
- `score(d) = Σ_path 1/(k + rank_path(d))`
- `k = 60`（由调用方传 `RRF_K`；本函数不硬编码）
- 任一路未出现的 `docid` 对该路贡献 0
- 结果按 `score` 降序、同分 `docid` 升序

**TDD 步骤：**

- [ ] **Step 1 —— 写测试（先红）：** 在 `tests.rs` 追加：
  ```rust
  use vane_core::types::RRF_K;

  fn fc(docid: u64, rank: u32, score: f32) -> FusionCandidate {
      FusionCandidate { docid, rank, score }
  }

  fn scored_eq(a: &[ScoredDoc], b: &[(u64, f32)]) -> bool {
      a.len() == b.len()
          && a.iter().zip(b.iter()).all(|(x, y)| x.docid == y.0 && (x.score - y.1).abs() < 1e-6)
  }

  #[test]
  fn rrf_two_paths_basic() {
      // 两路，docid 0/1 在两路都出现，rank 不同
      let path_a = vec![fc(0, 0, 9.0), fc(1, 1, 8.0)];
      let path_b = vec![fc(1, 0, 7.0), fc(0, 1, 6.0)];
      let out = rrf_fuse(&[path_a, path_b], 60);
      // doc0: 1/(60+0) + 1/(60+1) = 1/60 + 1/61
      // doc1: 1/(60+1) + 1/(60+0) = 同上
      let s0 = 1.0 / 60.0 + 1.0 / 61.0;
      let s1 = s0; // 对称
      // 同分 -> docid 升序 -> [0, 1]
      assert!(scored_eq(&out, &[(0, s0), (1, s1)]));
  }

  #[test]
  fn rrf_single_path() {
      let path = vec![fc(10, 0, 5.0), fc(20, 1, 4.0), fc(30, 2, 3.0)];
      let out = rrf_fuse(&[path], 60);
      let s10 = 1.0 / 60.0;
      let s20 = 1.0 / 61.0;
      let s30 = 1.0 / 62.0;
      assert!(scored_eq(&out, &[(10, s10), (20, s20), (30, s30)]));
  }

  #[test]
  fn rrf_doc_absent_in_one_path() {
      // doc0 只在 path_a，doc1 只在 path_b
      let path_a = vec![fc(0, 0, 9.0)];
      let path_b = vec![fc(1, 0, 7.0)];
      let out = rrf_fuse(&[path_a, path_b], 60);
      let s0 = 1.0 / 60.0;
      let s1 = 1.0 / 60.0;
      // 同分 -> docid 升序
      assert!(scored_eq(&out, &[(0, s0), (1, s1)]));
  }

  #[test]
  fn rrf_doc_absent_in_all_paths_excluded() {
      // 没有任何路包含的 docid 不应出现（构造上不可能进入结果，验证防回归）
      let path_a = vec![fc(0, 0, 9.0)];
      let path_b = vec![fc(1, 0, 7.0)];
      let out = rrf_fuse(&[path_a, path_b], 60);
      assert!(out.iter().all(|d| d.docid == 0 || d.docid == 1));
      assert_eq!(out.len(), 2);
  }

  #[test]
  fn rrf_empty_paths() {
      // 空路径切片 -> 空结果
      let out = rrf_fuse(&[], 60);
      assert!(out.is_empty());
  }

  #[test]
  fn rrf_empty_vecs_in_paths() {
      // 两路都为空 vec
      let out = rrf_fuse(&[vec![], vec![]], 60);
      assert!(out.is_empty());
  }

  #[test]
  fn rrf_result_sorted_desc() {
      // 构造不对称 rank，验证降序
      let path_a = vec![fc(0, 0, 9.0), fc(1, 1, 8.0), fc(2, 2, 7.0)];
      let path_b = vec![fc(2, 0, 7.0), fc(1, 1, 6.0), fc(0, 2, 5.0)];
      let out = rrf_fuse(&[path_a, path_b], 60);
      // doc0: 1/60 + 1/62
      // doc1: 1/61 + 1/61
      // doc2: 1/62 + 1/60
      // doc0 == doc2 > doc1
      let s0 = 1.0 / 60.0 + 1.0 / 62.0;
      let s1 = 2.0 / 61.0;
      let s2 = s0;
      assert!(scored_eq(&out, &[(0, s0), (2, s2), (1, s1)]));
  }

  #[test]
  fn rrf_k_is_60_frozen() {
      // SPEC §8.2：RRF_K 冻结为 60
      assert_eq!(RRF_K, 60);
  }

  #[test]
  fn rrf_duplicate_docid_within_single_path() {
      // 调用方契约：每路 docid 唯一。但若违反，累加行为应可预测（同一路多次出现按多次计入）。
      // 此用例锁定当前实现行为：同路重复 docid 会被累加两次。
      let path = vec![fc(0, 0, 5.0), fc(0, 1, 4.0)];
      let out = rrf_fuse(&[path], 60);
      let s = 1.0 / 60.0 + 1.0 / 61.0;
      assert_eq!(out.len(), 1);
      assert!(scored_eq(&out, &[(0, s)]));
  }
  ```
  运行 `cargo test -p vane-core fusion`，确认编译失败（`rrf_fuse` 未定义）。

- [ ] **Step 2 —— 写实现（转绿）：** 在 `mod.rs` 追加：
  ```rust
  use std::collections::HashMap;

  /// RRF 融合（SPEC §8.2）。
  ///
  /// `score(d) = Σ_path 1/(k + rank_path(d))`
  ///
  /// - `paths`：每路候选，`rank` 由调用方按 `score` 降序从 0 起编号。
  /// - `k`：RRF 平滑常数，SPEC 冻结为 60；调用方应传 [`vane_core::types::RRF_K`]。
  /// - 返回值按 `score` 降序，同分按 `docid` 升序。
  /// - 不在任何路出现的 `docid` 不会出现在结果中。
  pub fn rrf_fuse(paths: &[Vec<FusionCandidate>], k: u32) -> Vec<ScoredDoc> {
      let mut acc: HashMap<u64, f32> = HashMap::new();
      for path in paths {
          for c in path {
              // k 为 u32，rank 为 u32，相加不会溢出（u32::MAX 远超任何合理候选规模）
              let contrib = 1.0f32 / (k as f32 + c.rank as f32);
              *acc.entry(c.docid).or_insert(0.0) += contrib;
          }
      }
      let mut out: Vec<ScoredDoc> = acc
          .into_iter()
          .map(|(docid, score)| ScoredDoc { docid, score })
          .collect();
      // 降序 score，升序 docid（稳定 tie-break）
      out.sort_by(|a, b| {
          b.score
              .partial_cmp(&a.score)
              .unwrap_or(std::cmp::Ordering::Equal)
              .then_with(|| a.docid.cmp(&b.docid))
          });
      out
  }
  ```
  运行 `cargo test -p vane-core fusion`，确认全绿。

- [ ] **Step 3 —— 追加测试：** Task 0 已创建空 tests.rs，本步直接追加真实测试。

- [ ] **Step 4 —— clippy + wasm32 check：**
  - `cargo clippy -p vane-core -- -D warnings`
  - `cargo check --target wasm32-unknown-unknown -p vane-core`
  两者皆须通过。`HashMap` 在 wasm32 可用（std 提供）。

- [ ] **Step 5 —— 不变量自查：**
  - 无 `cfg` 出现（`grep -n "cfg(" crates/vane-core/src/fusion/mod.rs` 应空）。
  - 无外部 crate 引入（`use` 仅 `std::collections::HashMap` 与 `vane_core::types::ScoredDoc`）。

---

### Task 2: `minmax_normalize` —— 按候选集归一化

**Files:** `crates/vane-core/src/fusion/mod.rs`、`crates/vane-core/src/fusion/tests.rs`

**Interfaces:**
- Consumes from 00-workspace: `ScoredDoc`
- Produces: `LinearInput`（Task 0 已声明字段）、`pub fn minmax_normalize(scored: &[ScoredDoc]) -> Vec<LinearInput>`

**算法（SPEC §8.2）：**
- `norm(s) = (s - min) / (max - min)`，`min`/`max` 取自当次候选集
- 候选集为空 -> 返回空 vec
- `max == min`（单元素或全相同分数）-> 所有归一化值记 `0.0`（避免除零）
- 输出顺序与输入一致（不排序；排序由 `linear_fuse` 负责）

**TDD 步骤：**

- [ ] **Step 1 —— 写测试（先红）：** 在 `tests.rs` 追加：
  ```rust
  fn sd(docid: u64, score: f32) -> ScoredDoc {
      ScoredDoc { docid, score }
  }

  fn li_eq(a: &[LinearInput], b: &[(u64, f32)]) -> bool {
      a.len() == b.len()
          && a.iter().zip(b.iter()).all(|(x, y)| x.docid == y.0 && (x.score - y.1).abs() < 1e-6)
  }

  #[test]
  fn minmax_basic() {
      let scored = vec![sd(0, 10.0), sd(1, 5.0), sd(2, 0.0)];
      let out = minmax_normalize(&scored);
      // min=0, max=10 -> [1.0, 0.5, 0.0]
      assert!(li_eq(&out, &[(0, 1.0), (1, 0.5), (2, 0.0)]));
  }

  #[test]
  fn minmax_preserves_input_order() {
      // 输入未排序，输出应保持输入顺序
      let scored = vec![sd(7, 1.0), sd(3, 5.0), sd(9, 3.0)];
      let out = minmax_normalize(&scored);
      // min=1, max=5
      // doc7: (1-1)/(5-1)=0
      // doc3: (5-1)/4=1
      // doc9: (3-1)/4=0.5
      assert!(li_eq(&out, &[(7, 0.0), (3, 1.0), (9, 0.5)]));
  }

  #[test]
  fn minmax_empty() {
      let out = minmax_normalize(&[]);
      assert!(out.is_empty());
  }

  #[test]
  fn minmax_single_element() {
      // max==min -> 归一化为 0.0
      let scored = vec![sd(42, 3.14)];
      let out = minmax_normalize(&scored);
      assert!(li_eq(&out, &[(42, 0.0)]));
  }

  #[test]
  fn minmax_all_equal_scores() {
      let scored = vec![sd(0, 2.5), sd(1, 2.5), sd(2, 2.5)];
      let out = minmax_normalize(&scored);
      assert!(li_eq(&out, &[(0, 0.0), (1, 0.0), (2, 0.0)]));
  }

  #[test]
  fn minmax_negative_scores() {
      // 向量距离可能为负（如 dot）；验证 minmax 通用
      let scored = vec![sd(0, -1.0), sd(1, -5.0)];
      let out = minmax_normalize(&scored);
      // min=-5, max=-1
      // doc0: (-1 - (-5)) / 4 = 1.0
      // doc1: (-5 - (-5)) / 4 = 0.0
      assert!(li_eq(&out, &[(0, 1.0), (1, 0.0)]));
  }

  #[test]
  fn minmax_nan_safe_input_rejected() {
      // 调用方契约：不含 NaN。若含 NaN，partial_cmp 在排序中视为 Equal，
      // 但 min/max 用 fold 会传播 NaN。此处锁定行为：含 NaN 时结果未定义，
      // 本测试仅确认不 panic（minmax 不对 NaN 做特殊处理）。
      let scored = vec![sd(0, f32::NAN)];
      let _ = minmax_normalize(&scored);
  }
  ```
  运行测试，确认编译失败（`minmax_normalize` 未定义）。

- [ ] **Step 2 —— 写实现（转绿）：** 在 `mod.rs` 追加：
  ```rust
  /// minmax 归一化（SPEC §8.2；按当次候选集）。
  ///
  /// - `norm(s) = (s - min) / (max - min)`，min/max 取自输入候选集。
  /// - 候选集为空 -> 返回空 vec。
  /// - `max == min`（单元素或全相同分数）-> 所有归一化值记 `0.0`。
  /// - 输出顺序与输入一致；不排序。
  pub fn minmax_normalize(scored: &[ScoredDoc]) -> Vec<LinearInput> {
      if scored.is_empty() {
          return Vec::new();
      }
      let mut min = scored[0].score;
      let mut max = scored[0].score;
      for d in &scored[1..] {
          if d.score < min {
              min = d.score;
          }
          if d.score > max {
              max = d.score;
          }
      }
      let range = max - min;
      scored
          .iter()
          .map(|d| LinearInput {
              docid: d.docid,
              score: if range == 0.0 || range.is_nan() {
                  0.0
              } else {
                  (d.score - min) / range
              },
          })
          .collect()
  }
  ```
  运行测试，确认全绿。

  > 说明：`range == 0.0` 覆盖单元素与全相同分数；`range.is_nan()` 覆盖含 NaN 的输入（虽非契约，但避免除零传播 NaN）。

- [ ] **Step 3 —— clippy + wasm32 check：** 同 Task 1 Step 4。

- [ ] **Step 4 —— 不变量自查：**
  - `minmax_normalize` 不调用 `sort`（顺序保持）。
  - 无 `cfg`。

---

### Task 3: `linear_fuse` —— alpha 加权融合

**Files:** `crates/vane-core/src/fusion/mod.rs`、`crates/vane-core/src/fusion/tests.rs`

**Interfaces:**
- Consumes from 00-workspace: `ScoredDoc`
- Produces: `pub fn linear_fuse(vec_scores: &[LinearInput], text_scores: &[LinearInput], alpha: f32) -> Vec<ScoredDoc>`

**算法（SPEC §8.2）：**
- `fused(d) = alpha × vec_score(d) + (1 - alpha) × text_score(d)`
- 两路取 `docid` 并集；缺路记 `score = 0.0`
- 结果按 `score` 降序、同分 `docid` 升序
- `alpha` 由调用方传入（API 默认路径不出现 alpha，本函数不提供默认值）

**数值约定：**
- `alpha` 通常 ∈ [0, 1]，但本函数不做范围校验（SPEC 未规定校验；若调用方传越界值，行为可预测——线性外推）。07-api-core 可在上层加 `InvalidArg` 校验，不在本模块。
- 缺路记 0 的语义：若 `vec_scores` 不含某 `docid`，该 `docid` 的 `vec_score` 视为 0；`text_scores` 同理。

**TDD 步骤：**

- [ ] **Step 1 —— 写测试（先红）：** 在 `tests.rs` 追加：
  ```rust
  fn li(docid: u64, score: f32) -> LinearInput {
      LinearInput { docid, score }
  }

  #[test]
  fn linear_basic_overlap() {
      let vec_scores = vec![li(0, 1.0), li(1, 0.5)];
      let text_scores = vec![li(0, 0.2), li(1, 0.8)];
      let out = linear_fuse(&vec_scores, &text_scores, 0.5);
      // doc0: 0.5*1.0 + 0.5*0.2 = 0.6
      // doc1: 0.5*0.5 + 0.5*0.8 = 0.65
      // 降序: [1(0.65), 0(0.6)]
      assert!(scored_eq(&out, &[(1, 0.65), (0, 0.6)]));
  }

  #[test]
  fn linear_alpha_one_ignores_text() {
      let vec_scores = vec![li(0, 0.9), li(1, 0.1)];
      let text_scores = vec![li(0, 1.0), li(1, 1.0)];
      let out = linear_fuse(&vec_scores, &text_scores, 1.0);
      // doc0: 1.0*0.9 + 0.0*1.0 = 0.9
      // doc1: 1.0*0.1 + 0.0*1.0 = 0.1
      assert!(scored_eq(&out, &[(0, 0.9), (1, 0.1)]));
  }

  #[test]
  fn linear_alpha_zero_ignores_vec() {
      let vec_scores = vec![li(0, 1.0), li(1, 1.0)];
      let text_scores = vec![li(0, 0.3), li(1, 0.7)];
      let out = linear_fuse(&vec_scores, &text_scores, 0.0);
      // doc0: 0*1.0 + 1*0.3 = 0.3
      // doc1: 0*1.0 + 1*0.7 = 0.7
      assert!(scored_eq(&out, &[(1, 0.7), (0, 0.3)]));
  }

  #[test]
  fn linear_disjoint_docids_union() {
      // 两路 docid 完全不重叠 -> 并集，缺路记 0
      let vec_scores = vec![li(0, 1.0)];
      let text_scores = vec![li(1, 1.0)];
      let out = linear_fuse(&vec_scores, &text_scores, 0.5);
      // doc0: 0.5*1.0 + 0.5*0.0 = 0.5
      // doc1: 0.5*0.0 + 0.5*1.0 = 0.5
      // 同分 -> docid 升序 -> [0, 1]
      assert!(scored_eq(&out, &[(0, 0.5), (1, 0.5)]));
  }

  #[test]
  fn linear_partial_overlap() {
      let vec_scores = vec![li(0, 1.0), li(1, 0.5), li(2, 0.0)];
      let text_scores = vec![li(1, 1.0), li(2, 0.5), li(3, 0.0)];
      let out = linear_fuse(&vec_scores, &text_scores, 0.5);
      // doc0: 0.5*1.0 + 0.5*0.0 = 0.5   (只在 vec)
      // doc1: 0.5*0.5 + 0.5*1.0 = 0.75  (两路都有)
      // doc2: 0.5*0.0 + 0.5*0.5 = 0.25  (两路都有)
      // doc3: 0.5*0.0 + 0.5*0.0 = 0.0   (只在 text)
      // 降序: [1, 0, 2, 3]
      assert!(scored_eq(&out, &[(1, 0.75), (0, 0.5), (2, 0.25), (3, 0.0)]));
  }

  #[test]
  fn linear_both_empty() {
      let out = linear_fuse(&[], &[], 0.5);
      assert!(out.is_empty());
  }

  #[test]
  fn linear_vec_empty() {
      let text_scores = vec![li(0, 0.4), li(1, 0.6)];
      let out = linear_fuse(&[], &text_scores, 0.5);
      // doc0: 0.5*0 + 0.5*0.4 = 0.2
      // doc1: 0.5*0 + 0.5*0.6 = 0.3
      assert!(scored_eq(&out, &[(1, 0.3), (0, 0.2)]));
  }

  #[test]
  fn linear_text_empty() {
      let vec_scores = vec![li(0, 0.4), li(1, 0.6)];
      let out = linear_fuse(&vec_scores, &[], 0.5);
      // doc0: 0.5*0.4 + 0.5*0 = 0.2
      // doc1: 0.5*0.6 + 0.5*0 = 0.3
      assert!(scored_eq(&out, &[(1, 0.3), (0, 0.2)]));
  }

  #[test]
  fn linear_tie_break_docid_asc() {
      // 构造同分场景
      let vec_scores = vec![li(5, 0.2), li(3, 0.2)];
      let text_scores = vec![li(5, 0.2), li(3, 0.2)];
      let out = linear_fuse(&vec_scores, &text_scores, 0.5);
      // 两 doc 同分 0.2 -> docid 升序 [3, 5]
      assert!(scored_eq(&out, &[(3, 0.2), (5, 0.2)]));
  }

  #[test]
  fn linear_does_not_provide_alpha_default() {
      // 编译期检查：linear_fuse 必须显式接收 alpha（签名层面保证，无默认值）。
      // 此测试仅作为文档锚点：确认函数签名第三参数为 f32 且无 Default。
      // 若未来有人加 #[derive(Default)] 或默认参数，此测试需更新。
      let _: fn(&[LinearInput], &[LinearInput], f32) -> Vec<ScoredDoc> = linear_fuse;
  }
  ```
  运行测试，确认编译失败（`linear_fuse` 未定义）。

- [ ] **Step 2 —— 写实现（转绿）：** 在 `mod.rs` 追加：
  ```rust
  use std::collections::HashMap;

  /// linear 融合（SPEC §8.2）。
  ///
  /// `fused(d) = alpha × vec_score(d) + (1 - alpha) × text_score(d)`
  ///
  /// - 两路取 `docid` 并集；缺路记 `score = 0.0`。
  /// - `alpha` 由调用方传入，本函数不提供默认值（SPEC §8.2：API 默认路径不出现 alpha）。
  /// - 结果按 `score` 降序，同分按 `docid` 升序。
  /// - `alpha` 范围校验由上层负责（本函数不校验，行为可预测的线性外推）。
  pub fn linear_fuse(
      vec_scores: &[LinearInput],
      text_scores: &[LinearInput],
      alpha: f32,
  ) -> Vec<ScoredDoc> {
      let mut acc: HashMap<u64, (f32, f32)> = HashMap::new();
      for v in vec_scores {
          acc.entry(v.docid).or_insert((0.0, 0.0)).0 = v.score;
      }
      for t in text_scores {
          acc.entry(t.docid).or_insert((0.0, 0.0)).1 = t.score;
      }
      let mut out: Vec<ScoredDoc> = acc
          .into_iter()
          .map(|(docid, (v, t))| ScoredDoc {
              docid,
              score: alpha * v + (1.0 - alpha) * t,
          })
          .collect();
      out.sort_by(|a, b| {
          b.score
              .partial_cmp(&a.score)
              .unwrap_or(std::cmp::Ordering::Equal)
              .then_with(|| a.docid.cmp(&b.docid))
          });
      out
  }
  ```
  运行测试，确认全绿。

  > 实现说明：`HashMap<u64, (f32, f32)>` 同时持有两路分数，缺路默认 `(0.0, 0.0)`。`or_insert` 保证只路出现的 docid 另一路为 0。重复 docid 在同一路内的行为：后写覆盖（调用方契约：每路 docid 唯一；违反时取最后一个值）。

- [ ] **Step 3 —— clippy + wasm32 check：** 同 Task 1 Step 4。

- [ ] **Step 4 —— 重复 docid 行为锁定测试：** 在 `tests.rs` 追加：
  ```rust
  #[test]
  fn linear_duplicate_docid_in_same_path_last_wins() {
      // 调用方契约：每路 docid 唯一。违反时同路后写覆盖。
      let vec_scores = vec![li(0, 1.0), li(0, 0.2)];
      let text_scores = vec![li(0, 0.0)];
      let out = linear_fuse(&vec_scores, &text_scores, 1.0);
      // vec 最后值为 0.2，alpha=1 -> 0.2
      assert_eq!(out.len(), 1);
      assert!(scored_eq(&out, &[(0, 0.2)]));
  }
  ```
  运行确认绿。

- [ ] **Step 5 —— 不变量自查：**
  - `grep -n "cfg(" crates/vane-core/src/fusion/mod.rs` 空。
  - `grep -nE "use (hashbrown|ahash|rayon)" crates/vane-core/src/fusion/mod.rs` 空。
  - `cargo check --target wasm32-unknown-unknown -p vane-core` 通过。

---

## 完成态自检清单

执行人在 PR 前逐项确认：

- [ ] `cargo test -p vane-core fusion` 全绿，覆盖：
  - RRF：两路基本、单路、缺一路、全缺排除、空 paths、空 vecs、降序、k=60 冻结、同路重复 docid。
  - minmax：基本、顺序保持、空集、单元素、全相同、负分、NaN 不 panic。
  - linear：基本重叠、alpha=0/1 边界、docid 不完全重叠、并集、单路空、双路空、tie-break、签名无默认 alpha、同路重复 docid。
- [ ] `cargo clippy -p vane-core -- -D warnings` 无 warning。
- [ ] `cargo check --target wasm32-unknown-unknown -p vane-core` 通过（I-5）。
- [ ] `crates/vane-core/src/fusion/mod.rs` 中公开项签名与 `docs/plans/m0/README.md` 的 `03-fusion 产出` 节逐字一致：
  - `pub struct FusionCandidate { pub docid: u64, pub rank: u32, pub score: f32 }`
  - `pub fn rrf_fuse(paths: &[Vec<FusionCandidate>], k: u32) -> Vec<ScoredDoc>`
  - `pub struct LinearInput { pub docid: u64, pub score: f32 }`
  - `pub fn minmax_normalize(scored: &[ScoredDoc]) -> Vec<LinearInput>`
  - `pub fn linear_fuse(vec_scores: &[LinearInput], text_scores: &[LinearInput], alpha: f32) -> Vec<ScoredDoc>`
- [ ] 无外部 crate 引入；`Cargo.toml` 未改动。
- [ ] 无 `cfg(...)` 出现在 `fusion/mod.rs`。
- [ ] RRF 结果降序 + docid 升序 tie-break；linear 同。
- [ ] 不在任一路的 docid 不出现在 RRF 结果；linear 取并集、缺路记 0。

---

## 风险与备注

1. **`HashMap` 迭代顺序非确定**：本模块用 `sort_by` 在最后统一排序，结果顺序确定（不依赖 HashMap 迭代序）。同分 tie-break 用 `docid.cmp` 保证跨运行一致。
2. **`f32` 精度**：RRF `1/(k+rank)` 在 k=60、rank 至数千时数值很小但 f32 精度足够（10 万文档场景 rank < 10^5，1/60 与 1/100060 差异在 f32 有效位内）。linear 的 alpha 加权同理。测试用 `1e-6` 容差。
3. **wasm32 `HashMap`**：std `HashMap` 在 wasm32 可用，使用 `RandomState`（wasm32 下基于 `getrandom` 通过 `js_sys`；00-workspace 已确保 std 可编译到 wasm32）。本模块不引入 `ahash`/`hashbrown`。
4. **alpha 范围校验**：本模块不做。07-api-core 在解析 `FusionSpec::Linear { alpha }` 时应校验 `0.0 <= alpha <= 1.0`，越界返回 `InvalidArg`。本计划在 07-api-core 计划中提示此点。
5. **与 06-vector-brute / 05-bm25 的衔接**：两路检索产出 `Vec<ScoredDoc>`，07-api-core 负责将其转换为 `FusionCandidate`（按 score 降序编号 rank）或 `LinearInput`（经 `minmax_normalize`）。本模块不负责该转换。
