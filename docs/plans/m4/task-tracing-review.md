# M4 Phase 5a — tracing feature 审查

- **审查者**：task reviewer SubAgent（opus，只读，禁编辑源码）
- **审查对象**：commit `dae29c6`（`feat(core): tracing feature（cfg 门控零开销埋点，默认 off）（M4 阶段五 a）`）
- **范围**：6 files +345（Cargo.toml +12 / Cargo.lock +32 / collection.rs +70 / page_cache.rs +12 / wal/mod.rs +3 / report +216）
- **输入**：brief `phase0-design.md` §3.5 / report `task-tracing-report.md` / review package `task-tracing-review-package.md`

---

## A. Spec 合规

| 检查项 | 结果 | 依据 |
|---|---|---|
| tracing optional dep + `cfg(feature="tracing")` 门控 + 默认 off | ✅ | `Cargo.toml:44` `tracing = { version = "0.1", optional = true }`；`Cargo.toml:77` `tracing = ["dep:tracing"]`；不在任何 default feature 链 |
| 9 埋点覆盖 §3.5 表 | ⚠️ 基本覆盖，1 字段缺 | 7 spec 行覆盖 6 全 + 1 部分（索引大小缺 bytes，见 Minor-1） |
| 不建 telemetry.rs（§3.5 推荐直接各模块 cfg） | ✅ | 无 `telemetry.rs`；埋点直接在各模块 `#[cfg(feature="tracing")]` + `tracing::info!/debug!` |
| vane-wasm 不启用 tracing | ✅ | `vane-wasm/Cargo.toml:14` `vane-core = { workspace = true, features = ["zstd-decode"] }`——仅 zstd-decode，无 tracing；`default = []` |

**Spec 合规：✅**（1 处 Minor spec 覆盖缺，不阻塞）

---

## B. 代码质量

### B.1 cfg 门控正确性——全门控 ✅

`grep -rn 'tracing::' crates/vane-core/src/` 确认 11 处宏调用（8 `info!/info_span!` + 3 `debug!`），**每一处均被前行 `#[cfg(feature = "tracing")]` 门控**：

| 文件:行 | 宏 | 前一行 cfg |
|---|---|---|
| `collection.rs:465` | `info!(… "flush done")` | `:464` ✅ |
| `collection.rs:534` | `info!(… "merge start")` | `:533` ✅ |
| `collection.rs:654` | `info!(… "merge done")` | `:653` ✅ |
| `collection.rs:713` | `info_span!(… "search")` | `:712` ✅ |
| `collection.rs:721` | `web_time::Instant::now()`（埋点用） | `:720` ✅ |
| `collection.rs:1031` | `info!(elapsed_us, hits, "search done")` | `:1030` ✅ |
| `collection.rs:1184` | `info!(… "dict state transition")` | `:1183` ✅ |
| `collection.rs:1243` | `info!(… "dict state transition")` | `:1242` ✅ |
| `collection.rs:1428` | `info!(… "dict state transition")` | `:1427` ✅ |
| `page_cache.rs:54` | `debug!(hit=true, …)` | `:53` ✅ |
| `page_cache.rs:66` | `debug!(hit=false, …)` | `:65` ✅ |
| `wal/mod.rs:68` | `debug!(?record, …)` | `:67` ✅ |

`#[cfg]` 块内的 `let _span` / `let _search_start` 绑定也在同一 gate 下——tracing off 时这两个变量不存在，后续 `tracing::info!(elapsed_us = _search_start.elapsed()…)` 也被 gate 消除，**无未引用变量编译错误**。`dep:tracing` 隔离确保 tracing crate 本身不进 off 编译单元。

**cfg 门控正确性定性：全门控，无泄漏。**

### B.2 埋点实质——non-vacuous ✅

逐条核对埋点 emit 的字段值（非空 `info!()` / `debug!()`）：

| 埋点 | emit 字段 | 判定 |
|---|---|---|
| search span | `top_k`, `mode=?`, `segment_count`, `allow_hnsw`（4 字段） | non-vacuous ✅ |
| search done | `elapsed_us`, `hits`（2 字段） | non-vacuous ✅ |
| flush done | `collection`, `segment_ulid`, `doc_count`, `segment_count`（4 字段） | non-vacuous ✅（缺 bytes，见 Minor-1） |
| merge start | `collection`, `sources`, `target_docid_base`, `full_merge`（4 字段） | non-vacuous ✅ |
| merge done | `collection`, `new_segment_ulid`, `new_doc_count`, `segment_count`（4 字段） | non-vacuous ✅ |
| PageCache hit | `hit=true`, `path`, `page_idx`（3 字段） | non-vacuous ✅ |
| PageCache miss | `hit=false`, `path`, `page_idx`, `bytes`（4 字段） | non-vacuous ✅ |
| Wal::append | `?record`（Debug，记录值） | non-vacuous ✅ |
| dict×3 transitions | `collection`, `from=?`, `to=?`, `dict_entries`（3-4 字段） | non-vacuous ✅ |

**埋点实质定性：non-vacuous。** 全部 emit 有意义字段值，无空 `info!()` / no-op。唯一缺：flush done 缺 `bytes` 字段（§3.5 索引大小 metric，见 Minor-1）。

### B.3 wasm off 体积不变——确认 ✅

代码审查复核：
- 所有埋点 `#[cfg(feature = "tracing")]` 门控（B.1 确认）→ tracing off 时编译期消除
- `dep:tracing` 隔离 → tracing crate 不进 off 编译单元
- vane-wasm 不透传 tracing feature（A 确认）→ wasm deliverable 永不含 tracing

report 称 `grep -c tracing = 0`（wasm-objdump vane_core.wasm）+ vane-wasm 349261B（与 Phase 2b 基线持平）。代码审查与之一致——tracing off 时确无 tracing 符号入 wasm 路径。

**wasm off 不变定性：确认。**

### B.4 web_time / web-time dep 设置 ✅

编排者 grep `web_time`（下划线）未在 Cargo.toml 见——因 dep 名是 `web-time`（连字符），Rust 在代码中自动转下划线 `web_time::`。

核实结果：
- **(a)** `web-time = "1"` 在 `Cargo.toml:33` `[dependencies]`（连字符），非 `[target-dependencies]`
- **(b)** **regular dep**（非 optional，非 gated on tracing）。PRE-EXISTING——`persistence/mod.rs:152,160,189`（AutoCommitter `last_flush: web_time::Instant`）+ `segment/ulid.rs:13`（`use web_time::{SystemTime, UNIX_EPOCH}`，M2-01）早已使用。**tracing 任务未引入此 dep，仅复用**
- **(c)** tracing off 时 web-time 仍入 wasm build——**但它在 tracing 任务之前就已存在**（AutoCommitter/ULID 用），故无 tracing 引入的体积增量。report 的"体积不变"成立，不是因 optional/gated，而是因 web-time 早就在 build 里
- **(d)** web-time 传递依赖：wasm32 → js-sys（performance.now()），native → std。均不在 deny 黑名单（regex/tokio/prost/tonic/openssl/lindera/ndarray/wee_alloc/dashmap/parking_lot）。cargo deny check bans ok 已确认

**web_time/web-time dep 设置定性：在 Cargo.toml + regular（pre-existing）+ 不影响 off 体积（早就在 build）。无 issue。**

### B.5 RAII guard 错误路径——Minor

`run_search` 流程：
- `:684-708` topK 超限 / (None,None) → early return（**span 建立前**，参数校验 fast-fail）
- `:712-719` `info_span!("search", …)` 创建 RAII guard
- `:720-721` `_search_start = web_time::Instant::now()`
- `:722-731` dim 不匹配 → early return（**span 建立后**，`_span` drop 记 span exit，但无 `elapsed_us` done 事件）
- `:1030-1034` 成功路径 `info!(elapsed_us, hits, "search done")`

错误路径（dim 不匹配 / `?` 传播）：span RAII guard drop → subscriber 可观测 span 持续期；但显式 `elapsed_us` done 事件仅成功路径 emit。

**定性：Minor。** 理由：
1. span 本身在错误路径仍记录 exit（RAII drop），subscriber 可算 span 持续期——错误路径 latency 非完全不可观测
2. `elapsed_us` done 是成功路径的便捷指标事件，缺它不影响 span 完整性
3. tracing 默认 off，仅 tracing-on 用户受影响
4. defer `tracing::instrument` 属性宏（已在 dep 但未用）到 Phase 6 合理

### B.6 不改冻结 pub API ✅

diff 确认：所有埋点在函数体内 `#[cfg]` 块，不改 pub fn 签名 / struct 定义 / 错误码 / 持久化格式。`git show --stat dae29c6` 确认仅触 5 源文件 + Cargo.lock + report，未碰 SPEC.md / CI yml / 段格式文件。

---

## C. 已知 concerns 定性

| # | concern | 定性 | 理由 |
|---|---|---|---|
| 1 | CI 无 wasm32-size-tracing-on job | **acceptable defer** | task scope 明示"不碰 CI yml"；Phase 6 加 job + tracing on 对比断言。report §4.3 手动验证 +15KB ≤ 800KB |
| 2 | check-wasm-size.sh 无 grep 断言 | **acceptable defer** | §3.5 推荐加 `grep -c tracing = 0` 断言；report §4.2 手动验证 grep=0。Phase 6 持久化脚本断言 |
| 3 | tracing-subscriber 不进 core | **acceptable 设计取舍** | §3.5 明示"subscriber 是消费侧，core 只 emit"；vane-ffi/vane-node 按需加 dev-dep |
| 4 | web_time wasm elapsed | **无 issue**（见 B.4） | web-time pre-existing regular dep，tracing 复用同一 Instant 源，无新依赖，不影响 off 体积 |
| 5 | RAII guard 错误路径 | **Minor defer**（见 B.5） | span RAII 仍记 exit；elapsed_us done 缺错误路径，defer `tracing::instrument` |

---

## Findings 汇总

### Critical
（无）

### Important
（无）

### Minor

1. **`api/collection.rs:464-471` | flush done 埋点缺 `bytes` 字段（§3.5 索引大小 metric 部分未覆盖）| 用户无法从 tracing 事件观测段文件大小增长（容量规划指标缺失）**
   - spec §3.5 表行"索引大小"：`info!(segment_ulid, bytes = n, "segment persisted")`
   - impl flush done：`info!(collection, segment_ulid, doc_count, segment_count, "flush done")`——有 `segment_ulid` 但无 `bytes`
   - 根因：Vfs trait 无 `size()` 方法（§3.6 确认），flush 热路径计算 bytes 需 `read_at` 探测 EOF（性能差）或遍历段目录文件
   - 建议：defer 至 inspect API（§3.6 `segment_info` 的 `file_sizes`）或 Phase 6 补 `bytes`（用 `SegmentReader` 已知的 format 字段推算，避免 Vfs 探测）

2. **`report §2` | 埋点计数 "2 个 debug!" 应为 "3 个 debug!" | 文档不一致（非代码问题）**
   - 实际 debug! 调用：PageCache hit(`page_cache.rs:54`) + PageCache miss(`page_cache.rs:66`) + Wal::append(`wal/mod.rs:68`) = **3 个**
   - report §2 称"8 个 info!/info_span! + 2 个 debug!"——debug! 少算 1 个
   - 代码无影响，仅 report 计数笔误

3. **review package diff | report 内容引用 commit `86a2b81` 但实际 report 文件引用 `dae29c6` | review package 非完全最新**
   - review package diff 的 report 新增内容（`:468-683`）写 `86a2b81`，但实际 `task-tracing-report.md:5,162` 写 `dae29c6`
   - 推测：commit amend（86a2b81→dae29c6）后 report 更新了 hash 字段，但 review package diff 在 amend 前生成
   - 代码 diff（Cargo.toml/collection.rs/page_cache.rs/wal/mod.rs/Cargo.lock）与实际文件一致，仅 report 文件的 hash 字段有差异

---

## 无法从 diff 验证项

| 项 | 原因 | trust |
|---|---|---|
| wasm-objdump grep -c tracing = 0 | 需 wasm 工具链构建 | trust report §4.2 + 代码审查一致（全门控，无泄漏路径） |
| vane-wasm 349261B 体积不变 | 需构建 | trust report §4.1 + 代码审查一致（vane-wasm 不透传 tracing） |
| cargo deny check bans ok | 需 cargo-deny | trust report §5.2 + deny.toml 黑名单核对（tracing/web-time 不在列） |
| cargo test --all-features EXIT=0 | 需跑测试 | trust report §6 + 代码审查（cfg-gated，off 时零行为漂移） |
| cargo clippy -D warnings rc=0 | 需跑 clippy | trust report §6 |

---

## 总体

**不进 fix 循环。**

- Spec 合规 ✅（1 Minor spec 覆盖缺：flush done 缺 bytes，可 defer）
- cfg 门控全正确，无泄漏
- 埋点 non-vacuous，全部 emit 有意义字段
- wasm off 体积不变确认（代码审查 + report 一致）
- web-time dep 设置无 issue（pre-existing regular dep，tracing 复用，不影响 off 体积）
- RAII guard 错误路径 Minor（span 仍记 exit，elapsed_us defer）
- 5 项 concerns 全 acceptable defer / 无 issue / Minor
- 3 项 Minor（flush done 缺 bytes / report 计数笔误 / review package staleness）均不阻塞

**建议：接受当前实现，Minor-1（bytes）记入 Phase 6 backlog。**
