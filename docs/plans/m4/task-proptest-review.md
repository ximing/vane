# M4 阶段一 b：proptest 3 不变量 — task reviewer 审查

> SubAgent：task reviewer（opus，只读，禁编辑源码）
> 审查对象：commit `f849c7b`（4 files +631 -2）
> 输入：`docs/plans/m4/phase0-design.md` §3.3（brief）+ `task-proptest-report.md`（report）+ `task-proptest-review-package.md`（diff）
> 日期：2026-08-11

## Spec 合规：✅

| §3.3 要求 | 实现 | 判定 |
|---|---|---|
| 3 不变量（检索排序稳定合法 / persist round-trip 一致 / merge 不丢文档） | `search_returns_stable_topk` / `persist_roundtrip_consistent` / `merge_preserves_live_docs` | ✅ 全覆盖 |
| Strategy `arb_doc`/`arb_doc_batch`/`arb_query` 骨架 | `arb_doc_bodies`/`arb_merge_bodies`/`arb_query_components`（Debug tuple 变体 + char 生成替 string_regex） | ✅ 合理适配（见 workaround 定性） |
| NaN 过滤（`.prop_filter` 或测试内处理） | `arb_finite_f32` 用 `.prop_filter("finite", |x| x.is_finite())` 过滤 NaN+Inf | ✅ 比 spec 要求更强（含 Inf） |
| `proptest-regressions/` 提交 | `crates/vane-core/proptest-regressions/.gitkeep` 已提交（空目录占位） | ✅ |
| proptest dev-dep 不进 wasm/native 生产构建 | `Cargo.toml [dev-dependencies]`；`cargo check --target wasm32-unknown-unknown -p vane-core` 0.33s 不含 proptest | ✅ |

## Findings

### Critical：无

无 `assert!(true)` / 裸 `is_ok()` 不检内容 / 断言与目标无关的 vacuous 项。3 不变量的断言均检查实质内容（len 界、score 单调性、id 集合、tag JSON、capture 一致性）。

### Important

**I-1. `proptest_invariants.rs:229-233` | 不变量 1a 仅有上界 `hits1.len() <= min(topK, total)`，无下界 / 非空 guard | Vector/Hybrid 模式下"search 返 0 hits" bug 可假绿**

不变量 1a 断言 `hits1.len() <= upper`（upper = min(topK, total)）。这是**上界**（结果数不超过 topK），正确但**不充分**：
- 若 search 返 0 hits（bug），`0 <= upper` 恒真 → 1a 通过。
- 1b `hits1.windows(2)` 在空 Vec 上迭代为空 → score 单调性断言**不执行** → 通过。
- 1c `cap1 == cap2`（两空 Vec 相等）→ 通过。

三者**联合 vacuous**：search 返空的 bug 不会被抓到。

**为何非 Critical**：当前 256 cases 实跑过（report 输出 3 passed），且不变量 2 的 `prop_assert_eq!(hits.len(), total, ...)` 证实 Vector 模式 search 能返非空（total>=1），说明 search 正常工作。vacuous 是**潜在风险**（future bug 返空会漏检），非当前假绿。断言本身检内容（非 `assert!(true)`）。

**失败场景**：未来改动使 Vector/Hybrid search 返空（如 HNSW 构建 bug、cosine 全 NaN 回退空），不变量 1 全过假绿。

**建议（不阻塞，但合并前补）**：对 Vector/Hybrid 模式加下界——
- 最弱：`prop_assert!(!hits1.is_empty() || docs.is_empty(), "Vector/Hybrid must return hits when docs exist")`（仅对非 Text 模式）；
- 最强：Vector 模式 `prop_assert_eq!(hits1.len(), upper)`（应返满 topK）。
- Text 模式允许 0 hits（随机 text 无 BM25 匹配合法），不加上界。

### Minor

**M-1. `Cargo.toml:72-73` | 注释称"默认 features 不含 regex/regex-syntax"不准确 | 误导但 deny 合规**

注释写"默认 features 不含 regex/regex-syntax"，但 proptest 1.11.0 的 **default features 含 `string-regex`，直接拉 `regex-syntax`**（Cargo.lock:239 确认 `regex-syntax` 是 proptest 的直接 dep）。注释与 report 自审（lines 128-146 正确指出 regex-syntax 被拉）矛盾。

**实质合规**：`deny.toml:16` 仅 ban `{ name = "regex", wrappers = [...] }`，**不 ban `regex-syntax`**（独立 crate，零依赖）。`cargo deny check` → `bans ok`。测试用 `arb_letter`（char 生成）非 `string_regex`，regex-syntax 被拉入但运行时不走该路径。

**建议**：修正注释为"regex-syntax ≠ regex（banned），deny 绿；运行时用 char 生成非 string_regex"；或 `proptest = { version = "1", default-features = false }` 真正不拉 regex-syntax（更干净，但需验证 default-features=false 不影响 Strategy API）。

**M-2. `proptest_invariants.rs:7` `#![allow(dead_code)]` 文件级 | 掩盖潜在死 helper，已复核全调用 | 可接受但脆**

文件级 `#![allow(dead_code)]` 会掩盖任何未调用的 helper。reviewer 逐个复核 15 个 helper 的调用链：
- `arb_letter`→`arb_word`/`arb_doc_bodies`(tag)/`arb_merge_bodies`(tag) ✓
- `arb_word`→`arb_text` ✓
- `arb_text`→`arb_query_components`/`arb_doc_bodies`/`arb_merge_bodies` ✓
- `arb_finite_f32`→`arb_vector` ✓
- `arb_vector`→`arb_query_components`/`arb_doc_bodies`/`arb_merge_bodies` ✓
- `arb_query_components`→不变量 1 ✓
- `arb_doc_bodies`→不变量 1, 2 ✓
- `arb_merge_bodies`→不变量 3 ✓
- `build_docs`→不变量 1, 2 ✓
- `build_merge_scenario`→不变量 3 ✓
- `build_query`→不变量 1 ✓
- `build_schema`→不变量 1, 2, 3 ✓
- `vector_query_all`→不变量 2, 3 ✓
- `capture`→不变量 1, 2 ✓

**无死 helper**。allow(dead_code) 的理由（proptest! 宏闭包边界导致 rustc 无法追踪调用）成立。但若未来新增 helper 未用，此属性会静默隐藏——建议改用 `#[allow(dead_code)]` 逐函数标，或加注释提醒维护者复核。

**M-3. `persist_roundtrip_consistent ~110s/256 cases` | 性能非缺陷 | defer**

根因：256 cases × 2 `Db::open` × jieba 冷加载 ~150ms = ~77s + 测试本身 ~33s ≈ 110s。SPEC 要求 `Db::open` 加载词典（非设计缺陷）。`--all-features` 启 jieba feature 触发冷加载。proptest 本身（Strategy+断言）<1ms/case。

**判定**：Minor 性能，defer。CI test job（report：workspace test exit 0 无回归）容纳。若 CI timeout 压力：
- 短期：`PROPTEST_CASES=64` env 或 `proptest! { #![proptest_config(Config { cases: 64, ..Config::default() })] }` 降 cases；
- 中期：不变量 2 单测可 `cfg(not(feature="dict-zh"))` 跳 jieba（但 `--all-features` 仍会跑）；
- 长期：`Db::open` 共享 jieba dict 缓存（架构变更，超出本 task）。

## #[test] 实跑定性：✅ 3 不变量都加了 #[test] + 256 cases 实跑确认

proptest! 宏 gotcha 复核：
- `proptest_invariants.rs:208` `#[test]` 在 `fn search_returns_stable_topk` 前 ✓
- `proptest_invariants.rs:263` `#[test]` 在 `fn persist_roundtrip_consistent` 前 ✓
- `proptest_invariants.rs:341` `#[test]` 在 `fn merge_preserves_live_docs` 前 ✓

report 真实输出（lines 78-86）：
```
running 3 tests
test search_returns_stable_topk ... ok
test merge_preserves_live_docs ... ok
test persist_roundtrip_consistent has been running for over 60 seconds
test persist_roundtrip_consistent ... ok
test result: ok. 3 passed; 0 failed; ...; finishing in 110.38s
```
3 tests 被 `cargo test --list` 识别 + 实跑 + 256 cases 各过（110s 排除"0 tests 假绿"——0 tests 不会跑 110s）。**非 0 tests 假绿**。

## Debug tuple workaround 定性：✅ 合理，非致断言失真

- `Doc`（types.rs:114 `pub struct Doc {` 无 `#[derive(Debug)]`）和 `SearchQuery` 未 derive Debug（M0 冻结 pub API，implementer 未碰）。
- proptest! 宏要求 Strategy Value 类型实现 `Debug`（打印失败输入）。
- **解法**：Strategy 返 Debug tuple（`(String, Vec<f32>, char)` / `(String, Vec<f32>, u32, SearchMode)` / `(String, Vec<f32>, char, bool)`——全有 Debug），测试体内 `build_docs`/`build_query` 构造 API 类型。
- **断言不失真**：assertion 检的是**真实 API Hit 对象**（`h.id`/`h.score`/`h.fields`，Hit types.rs:102 derive Debug），非 Debug tuple。Debug tuple 仅影响失败输入打印（可从 tuple 重建输入，复现性 OK）。
- 断言路径：`col.search(&q).unwrap()` → `Vec<Hit>` → `capture(&hits)` → `Vec<(String, f32, Option<String>)>` → `prop_assert_eq!`。全链检真实 API 行为。

**判定**：workaround 合理，不致断言失真。

## persist_roundtrip ~110s 定性：Minor 性能，defer

见 M-3。非缺陷，SPEC 要求 `Db::open` 加载词典。CI 容纳。defer 除非 CI timeout 压力。

## proptest deny 合规定性：✅ regex-syntax ≠ regex，合规

- `deny.toml:16` ban 的是 `{ name = "regex", wrappers = ["napi-derive-backend", "criterion"] }`——仅 ban `regex` crate，且限 wrapper。
- proptest 1.11.0 拉的是 `regex-syntax`（独立 crate，零依赖），**非** `regex`。
- `cargo tree -p proptest` 依赖链无 `regex` crate（report 自审 lines 128-146 确认）。
- `cargo deny check` → `bans ok`（report lines 96-103）。唯一 warning 是 pre-existing `unused-wrapper`（napi-derive-backend 未在当前构建，非本次引入）。

**判定**：合规。regex-syntax 非 regex，不触黑名单。（但 Cargo.toml 注释不准确——见 M-1。）

## ⚠️ 无法从 diff 验证项

1. **HNSW order 跨 reopen 确定性**：不变量 2c `prop_assert_eq!(after, baseline)` 假设 reopen 后 search 结果**顺序**与基线一致。256 cases 过说明实践中确定（同 HNSW 图 + 同 query → 同序），但无法从 diff 证明 HNSW 永远 order-stable 跨 reopen。若 HNSW 有未持久化随机种子，未来 flake 风险。需读 HNSW search 代码确认。
2. **CI test job timeout 容纳 +110s**：report 称 workspace test exit 0 无回归，但无法从 diff 验证 CI config 的 test job timeout 是否 >110s + 其他测试总时。信任 report（不重跑门禁）。
3. **proptest 默认 256 cases 实跑数**：report 输出 + 110s 时序强烈暗示 256 cases 跑了（1 case 不会跑 110s），但无法从 diff 独立确认精确 case 数。`#[test]` 在位 + report 输出 = 充分证据。

## API 签名核对（reviewer 只读复核）

| 测试调用 | 源码位置 | 确认 |
|---|---|---|
| `Db::open(vfs, "db", OpenOptions::default())` | api/db.rs | ✓ |
| `db.collection("docs", schema, CollectionOptions::default())` | api/collection.rs | ✓ |
| `col.add(&docs)` → `Result<AddReport>` | collection.rs:253 | ✓ |
| `col.flush()` | collection.rs:303 | ✓ |
| `col.search(&q)` → `Result<Vec<Hit>>` | collection.rs:637 | ✓ |
| `col.search_brute_baseline(&q)` → `Result<Vec<Hit>>`（`#[doc(hidden)]`，绕 HNSW 走 brute） | collection.rs:648 | ✓ 存在且语义正确（不变量 3 用 brute baseline 验活文档全集，合理） |
| `col.delete(&ids)` | collection.rs:1019 | ✓ |
| `col.compact()` | collection.rs:1076 | ✓ |
| `db.close()` | db.rs:178 | ✓ |
| `db.collections()` → `Vec<String>` | db.rs:162 | ✓ |
| `Hit { id: String, score: f32, fields: Option<HashMap<String, String>> }` | types.rs:103-107 | ✓ |
| `Doc` 无 Debug derive | types.rs:114 | ✓（确认 workaround 必要） |

全部 API 调用真实存在，无虚构方法。`search_brute_baseline` 是 `#[doc(hidden)]` 非 IDL 但 pub，用于测试/bench 基线——不变量 3 用它绕 HNSW 近似验活文档全集，是 M2 merge_fuzz review 建议的正确用法。

## 总体：不进 fix 循环（建议合并前补 I-1）

**判定**：无 Critical，1 Important（I-1 潜在 vacuous 风险），3 Minor。3 不变量断言**非 vacuous**（检实质内容），256 cases 实跑确认（非 0 tests 假绿），Debug workaround 不致失真，deny 合规。

**是否进 fix 循环**：**否**——测试正确性无缺陷，可合并。但**建议合并前补 I-1**（不变量 1 加 Vector/Hybrid 非空 guard，关闭"search 返空"假绿风险），5 行改动。Minor 项（M-1 注释修正、M-2 allow(dead_code) 收窄、M-3 性能优化）可 defer。

**reviewer 立场**：implementer 的 proptest 实现质量良好——Strategy 设计合理（零 regex 运行时路径 + NaN/Inf 过滤 + 非全零 guard），断言用 `prop_assert!`/`prop_assert_eq!`（支持 shrinking），不变量 3 用 brute baseline 绕 HNSW 近似（正确），`#[test]` gotcha 已处理。唯一实质 gap 是不变量 1 缺下界，建议补上。
