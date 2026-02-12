# M4 阶段一 b：proptest property-based 不变量测试 — 实现报告

> SubAgent：implementer（阶段一 b）
> 分支：`feat/m4-prod-readiness`
> commit：`f849c7b`
> 日期：2026-08-11
> 设计来源：`docs/plans/m4/phase0-design.md` §3.3

## 状态

**DONE_WITH_CONCERNS** — 3 不变量 256 cases 全过；deny 绿；wasm 不受影响。
concern：`persist_roundtrip_consistent` 在 `--all-features` 下 256 cases 耗时 ~110s，
因 jieba dict 每 `Db::open` 冷加载 ~150ms（非 proptest 本身开销）。

## 3 不变量实现摘要

### 不变量 1：检索排序稳定合法（`search_returns_stable_topk`）

**Strategy**：`arb_doc_bodies(DIM=4, MAX_DOCS=8)` × `arb_query_components(DIM=4)`
- 文档体：`(text: String, vector: Vec<f32>, tag: char)` — 1..8 条，测试体内 `build_docs` 造 `Doc`（顺序 id `d0..d{n-1}` 保唯一 + meta tag scalar）。
- 查询体：`(text: String, vector: Vec<f32>, top_k: u32, mode: SearchMode)` — topK 1..=8，mode 三选一（Hybrid/Vector/Text）。
- f32 经 `is_finite()` 过滤 NaN/Inf；向量 `prop_filter("not_all_zero")` 避 cosine 0/0 退化。

**断言**（非 vacuous）：
- 1a：`hits1.len() <= min(topK, total_docs)` — 结果数有上界。
- 1b：`hits1.windows(2)` 中 `w[0].score >= w[1].score` — score 单调非递增；全部 `score.is_finite()`。
- 1c：`capture(hits1) == capture(hits2)` — 同 query 二次检索 `(id, score, tag)` 完全一致。

### 不变量 2：persist round-trip 一致（`persist_roundtrip_consistent`）

**Strategy**：`arb_doc_bodies(DIM=4, MAX_DOCS=8)` — 同不变量 1 文档体。

**流程**：MemoryVfs → `Db::open` → `collection` → `add` → `flush` → 基线 `search`（Vector, topK=total）→ `close` → `Db::open`（reopen）→ `search`。

**断言**：
- 2a：`hits2.len() == total` + `got_ids == expected_ids`（HashSet）— external_id 全回填。
- 2b：每条 hit 的 `fields["tag"]` 存在且为合法 JSON 字符串（`len>=3` + 首尾 `"`）— stored.bin meta JSON round-trip。
- 2c：`capture(hits2) == baseline` — search 结果集 `(id, score, tag)` 与基线完全一致。

### 不变量 3：merge 不丢文档（`merge_preserves_live_docs`）

**Strategy**：`arb_merge_bodies(DIM=4, MAX_DOCS=8)` × `chunk_size in 1u32..=4u32`
- merge 体：`(text, vector, tag, delete_flag: bool)` — 测试体内 `build_merge_scenario` 造 `(Vec<Doc>, Vec<bool>)`。

**流程**：MemoryVfs → `Db::open` → 按 `chunk_size` 分批 `add+flush`（多段）→ `delete`（标志位 true 的）→ `compact` → `search_brute_baseline`（Vector, topK=total）。

**断言**（用 `search_brute_baseline` 绕过 HNSW 近似，避假红/绿 — 1a merge_fuzz review M2 建议）：
- 3a：`hits.len() == expected_live.len()` + 全部 `hit_ids.contains(id)` — 活文档全可见。
- 3b：全部 `delete_ids` 不在 `hit_ids` — tombstoned 不可见。
- 3c：`hits.len() == hit_ids.len()` — 无重复 docid。

## 文件清单

| 文件 | 动作 | 说明 |
|---|---|---|
| `crates/vane-core/Cargo.toml` | 修改 | `[dev-dependencies]` 加 `proptest = "1"` |
| `crates/vane-core/tests/proptest_invariants.rs` | 新增 | 3 不变量 + Strategy + helper（421 行） |
| `crates/vane-core/proptest-regressions/.gitkeep` | 新增 | proptest 失败 seed 持久化目录（空 — 无失败） |
| `Cargo.lock` | 修改 | proptest v1.11.0 入 lock（+206 行） |

## 各门禁真实输出

### 1. `cargo fmt --all -- --check`
```
（无 diff 输出，exit 0）
```

### 2. `cargo clippy --all-targets --all-features --workspace --exclude vane-fuzz -- -D warnings`
```
    Checking vane-core v0.2.0 (/Users/ximing/project/mygithub/vane/crates/vane-core)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.10s
```
无 warning 无 error。

**注**：`proptest!` 宏将测试体包入传给 `TestRunner::run` 的闭包，rustc `dead_code` 分析无法穿越闭包追踪 helper 调用 → 文件级 `#![allow(dead_code)]` 消除假告警（helper 实际被闭包内测试体调用）。不影响 clippy 其他门禁。

### 3. `cargo test -p vane-core --all-features --test proptest_invariants`
```
running 3 tests
test search_returns_stable_topk ... ok
test merge_preserves_live_docs ... ok
test persist_roundtrip_consistent has been running for over 60 seconds
test persist_roundtrip_consistent ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 110.38s
```
3 不变量默认 256 cases 各过。`persist_roundtrip_consistent` 耗时 ~110s（256 cases × 每例 `Db::open` reopen 加载 jieba dict ~150ms × 2 次 open）。

### 4. `cargo test --workspace --all-features --exclude vane-fuzz`
```
（exit code 0 — 全 workspace 测试无回归）
```
含 proptest 3 不变量 + 现有全部集成/单元测试。tail -40 截取尾部零测试文件 + doc-tests 全 ok。

### 5. `cargo deny check`
```
warning[unused-wrapper]: wrapper for banned crate was not encountered
   ┌─ deny.toml:16:36
   │  { name = "regex", wrappers = ["napi-derive-backend", "criterion"] },
   │                                    unmatched wrapper

advisories ok, bans ok, licenses ok, sources ok
```
**bans ok** — proptest 传递依赖不触黑名单。`unused-wrapper` 是 pre-existing warning（当前构建未引 napi-derive-backend），非本次引入。

### 6. `cargo check --target wasm32-unknown-unknown -p vane-core`
```
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.33s
```
proptest 是 dev-dep，不进 wasm32 构建（dev-deps 不参与 `cargo check --target`）。

## commit

```
f849c7b test(core): proptest 3 不变量（检索稳定/round-trip/merge 不丢）（M4 阶段一 b）

 4 files changed, 631 insertions(+), 2 deletions(-)
 Cargo.lock                                     | 206 +++++++++++-
 crates/vane-core/Cargo.toml                    |   6 +
 crates/vane-core/proptest-regressions/.gitkeep |   0
 crates/vane-core/tests/proptest_invariants.rs  | 421 +++++++++++++++++++++++++
```

`git status` 确认只动上述 4 文件（不含 SPEC/CI/fault.rs/crash_recovery.rs/vane-fuzz）。

## 自审

### proptest 是否拉 regex？

**否**。proptest v1.11.0（默认 features）传递依赖链：
```
proptest v1.11.0
├── bit-set, bit-vec, bitflags
├── num-traits → libm
├── rand, rand_core, getrandom → cfg-if, libc
├── rand_chacha → ppv-lite86 → zerocopy → zerocopy-derive (proc-macro)
├── rand_xorshift
├── regex-syntax v0.8.11   ← 独立 crate，非 "regex"
├── rusty-fork → fnv, quick-error, tempfile, wait-timeout
├── tempfile → fastrand, once_cell, rustix
└── unarray
```

- `regex-syntax` 是独立 regex 解析器 crate（零依赖），**非** deny.toml 黑名单的 `regex` crate。
- `cargo tree -p proptest | grep "regex v"` 为空 — proptest 不拉 `regex` crate。
- deny.toml 黑名单 `{ name = "regex", wrappers = [...] }` 只 ban `regex` crate，不 ban `regex-syntax`。

### deny 是否绿？

**是**。`cargo deny check` 输出 `advisories ok, bans ok, licenses ok, sources ok`。唯一 warning 是 pre-existing `unused-wrapper`（napi-derive-backend 未在当前构建中），非本次引入。

### 依赖链是否触黑名单？

**否**。proptest 传递依赖：bit-set / bit-vec / bitflags / num-traits / libm / rand / rand_core / getrandom / cfg-if / libc / rand_chacha / ppv-lite86 / zerocopy / zerocopy-derive / proc-macro2 / quote / syn / unicode-ident / rand_xorshift / regex-syntax / rusty-fork / fnv / quick-error / tempfile / fastrand / once_cell / rustix / errno / wait-timeout / unarray / autocfg。

无一匹配 deny.toml `[bans] deny` 列表（regex / tokio / prost / tonic / openssl / lindera / ndarray / wee_alloc / dashmap / parking_lot）。

### proptest! 宏 `#[test]` 语法

proptest 1.x 的 `proptest!` 宏**不自动**加 `#[test]`。须在宏块内显式 `#[test]`：
```rust
proptest! {
    #[test]
    fn my_test(x in 0..10) { ... }
}
```
不加 `#[test]` 则生成普通函数（非测试），`cargo test --list` 显示 0 tests。已验证。

### Doc/SearchQuery 未 derive Debug

`Doc` 和 `SearchQuery` 未 derive `Debug`（M0 冻结 pub API，不改）。proptest! 宏需值类型实现 `Debug` 以打印失败输入。**解法**：Strategy 返回 `Debug` 可格式化元组（`String`/`Vec<f32>`/`char`/`u32`/`SearchMode`/`bool` 全有 Debug），测试体内经 `build_docs` / `build_query` 构造 API 类型。

### 不变量 2 耗时 concern

`persist_roundtrip_consistent` 在 `--all-features` 下 256 cases 耗时 ~110s。根因：每例 2 次 `Db::open`（首次 + reopen），`--all-features` 启用 jieba feature → 每 `Db::open` 调 `load_default_jieba_dict()`（~150ms 冷加载），256 × 2 × ~150ms ≈ 77s + 测试本身 ~33s ≈ 110s。非 proptest 框架开销，非 jieba 设计缺陷（SPEC 要求 `Db::open` 加载词典）。proptest 本身（Strategy 生成 + 断言）<1ms/case。CI test job 现有 timeout 应可容纳（workspace 全测试 exit 0 无回归）。

## 结论

3 不变量（检索排序稳定合法 / persist round-trip 一致 / merge 不丢文档）proptest 实现完成，256 cases 全过。proptest dev-dep 不触黑名单，cargo deny 绿，wasm 不受影响。commit `f849c7b` 仅含 4 目标文件，无冻结 API/SPEC/CI 改动。

---

## 6. Fix r1（review I-1 + M1）

**fix commit**：`34a9b11`
**fix 提交信息**：`test(core): proptest 不变量 1 加非空 guard + 修 Cargo.toml 注释（M4 阶段一 b fix r1）`
**fix 文件**：`crates/vane-core/tests/proptest_invariants.rs` + `crates/vane-core/Cargo.toml`

### I-1（Important）— 不变量 1 加非空 guard

**问题**：不变量 1a 仅有上界 `hits1.len() <= min(topK, total)`，无下界/非空 guard → 若 search 返 0 hits 的 bug，`0 <= upper`、`windows(2)` 空、`cap1 == cap2` 两空全过 = 假绿。当前非假绿（不变量 2 证实 search 返非空），但潜在风险。

**修**：不变量 1 加 Vector/Hybrid 非空 guard（`proptest_invariants.rs:235-244`）：
```rust
if matches!(q.mode, SearchMode::Vector | SearchMode::Hybrid) {
    prop_assert!(
        !hits1.is_empty(),
        "Vector/Hybrid search returned 0 hits with {} docs, topK={}",
        docs.len(), q.top_k
    );
}
```
- Vector/Hybrid 模式：docs 非空（`arb_doc_bodies` 生成 1..MAX_DOCS）+ query 有效向量 → search 必返 ≥1 hit（cosine 对非零向量有定义，RRF 不过滤结果）。guard 捕获 search 返 0 hits 的 bug。
- Text 模式：query 文本可能不命中任何文档，0 hits 合法，不强制非空。

### M1（Minor）— Cargo.toml 注释修正

**问题**：`Cargo.toml:72` 注释称"默认 features 不含 regex/regex-syntax"不准确——proptest default 拉 `regex-syntax`（Cargo.lock 确认）。

**修**：注释改为"proptest default 拉 regex-syntax（独立 regex 解析器，非 deny 黑名单的 regex crate），不拉 regex crate"。

### 修后自证

- `cargo fmt --all -- --check`：exit 0
- `cargo clippy --all-targets --all-features --workspace --exclude vane-fuzz -- -D warnings`：无 warning
- `cargo test -p vane-core --all-features --test proptest_invariants`：3 passed, 0 failed, 109.82s — 新 guard 不破坏（Vector/Hybrid 全返非空）
- `cargo test --workspace --all-features --exclude vane-fuzz`：exit 0（无回归）
- `cargo deny check`：advisories ok, bans ok, licenses ok, sources ok

### 更新结论

fix r1 消除不变量 1 的假绿风险（Vector/Hybrid 非空 guard）+ 修正 Cargo.toml 注释。3 不变量 256 cases 全过，无回归。M2（#![allow(dead_code)] 逐函数标）defer；M3（persist_roundtrip ~110s 性能）defer。
