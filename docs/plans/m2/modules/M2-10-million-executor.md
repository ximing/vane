# M2-10 100万规模 + Executor

## 1. 目标
恢复 100万规模承诺（M0/M1 50万不塌红线 → M2 100万）：引入 `Executor` trait（SPEC §11，native=rayon，wasm=串行），`cfg(target)` 仅在 Executor impl，搜索路径改用 Executor 并行搜各段 → 归并；段合并策略调优；100万压测不崩不错 + 延迟承诺（SPEC §3.3/§11/§13.1）。

SPEC 节号：§3.3（M2 设计上限 100万）、§11（Executor trait + cfg 仅 VFS/Executor）、§13.1（10万优化目标，100万恢复承诺）。

## 2. 涉及文件
- **Create** `crates/vane-core/src/executor/mod.rs`：`Executor` trait + `Scope` + native impl（`cfg(not(target_arch="wasm32"))`，包装 `rayon::scope`）+ wasm impl（`cfg(target_arch="wasm32")`，串行 spawn 立即调用）。
- **Modify** `crates/vane-core/Cargo.toml`：`rayon = { version = "1", optional = true }`；`[features] executor-native = ["dep:rayon"]`（native 启用，wasm 不启）。
- **Modify** `crates/vane-core/src/api/collection.rs`（search 路径，`api/collection.rs:740-790` 区间，reviewer A-M4 文件名笔误修正）：多段搜索改用 `Executor::scope` 并行搜各段 → 归并。Executor 实例由 `Db` 持有（open 时经 `executor::default_executor()` 工厂构造）。
- **Modify** `crates/vane-core/src/api/db.rs`：`Db` 增 `executor: Arc<dyn Executor>` 字段（open 时调 `executor::default_executor()` 构造）。**cfg 集中在 `executor/mod.rs`**（reviewer B-M2）：`pub fn default_executor() -> Arc<dyn Executor>` 工厂函数在 `executor/mod.rs` 内 `cfg(not(target_arch="wasm32"))` 返 `RayonExecutor`、`cfg(target_arch="wasm32")` 返 `SerialExecutor`；`api/db.rs` 仅调工厂，不出现 `cfg(target)`，避免 I-5 风险。
- **Modify** `crates/vane-core/src/merge/mod.rs`（`MergeTask`）：合并调度可选后台化（Executor 投递）；M1 同步执行保留为 wasm 路径。
- **Create** `crates/vane-core/tests/million_scale.rs`：100万压测（fixture 生成 + add/flush/search/compact 全流程，断言不崩 + 延迟）。
- **Modify** `crates/vane-core/benches/`：100万 cold_start + search bench。

## 3. 接口契约
### Consumes from
- M2-09 SQ8（降内存后 100万可行；100万×384 f32=1.5GB 超 §13.1，SQ8 后 ~400MB）。
- M0/M1 `HnswReader::search`（`hnsw/mod.rs:624`）、`compile_filter`（`filter/mod.rs:32`，reviewer A-M1 行号修正）、`MergeTask`（`merge/mod.rs:96`）。
- M2-07 懒加载（`vectors()` 访问点）。

### Produces for
```rust
// crates/vane-core/src/executor/mod.rs
pub trait Executor: Send + Sync {
    fn scope<R>(&self, f: impl FnOnce(&Scope) -> R) -> R;
}
pub struct Scope<'a> { /* rayon::Scope 或 串行 */ }
impl Scope<'_> {
    pub fn spawn(&self, task: impl FnOnce() + Send);
}

// native impl（cfg(not(target_arch="wasm32")), feature=executor-native）
pub struct RayonExecutor;
impl Executor for RayonExecutor { /* rayon::scope */ }

// wasm impl（cfg(target_arch="wasm32")）
pub struct SerialExecutor;
impl Executor for SerialExecutor { /* spawn 立即调用 */ }
```
下游：M2-14 Demo（大规模场景）。

## 4. TDD 测试清单
1. **Executor trait**：`RayonExecutor::scope` 并行执行多任务，结果归并正确（unit test，native）。
2. **SerialExecutor**：`SerialExecutor::scope` 串行执行（spawn 立即调用），wasm 路径。
3. **search 并行归并**：多段 search 用 Executor.scope 并行搜各段，归并 topK 与 M1 串行结果一致（recall 不退）。
4. **I-5 守护**：`grep -rn 'cfg(target_arch' crates/vane-core/src/` 仅命中 `executor/mod.rs`（`default_executor()` 工厂 + 两个 impl 块）+ `vfs/mod.rs:18`（`cfg(not(target_arch="wasm32")) pub mod std_fs;`，**是 target_arch 分支**，reviewer B-M3 修正：测试描述原误称 std_fs 非 target_arch）；`api/db.rs` 仅调 `default_executor()` 工厂无 `cfg(target)`；核心算法（hnsw/bm25/fusion/filter/segment）零 cfg。
5. **rayon 仅 Executor impl**：`grep -rn 'use rayon' crates/vane-core/src/` 仅 `executor/mod.rs`；算法模块零 rayon。
6. **100万 add/flush**：100万×384 维文档分批 add+flush，不崩（OOM 检查，SQ8 启用内存 <2GB）。
7. **100万 search 延迟**：hybrid topK=10 P99 < 200ms（10万 P99<50ms × 放宽倍数，100万 ~4 倍段数；SPEC §13.1 native 承诺，100万按比例放宽）。
8. **100万 recall**：recall@10 ≥0.95（五档选择率，相对暴力双路+RRF 基线）。
9. **100万 compact**：compact 后段数 ≤10，延迟可接受（M1 MergeTask 调优）。
10. **wasm32 编译**：`cargo check --target wasm32-unknown-unknown -p vane-core` 不启 executor-native（rayon 不进 wasm）；SerialExecutor 走串行。
11. **段合并策略调优**：100万场景段数硬上限 10 触发自动合并，小段优先（SPEC §7.3）。
12. **并发安全**：多线程并发 search + 单写者 flush/compact，I-4 不破（reindex 不并发）。

## 5. 验收标准
- 100万×384 维全流程（add/flush/search/compact）不崩不错。
- search P99 < 200ms（native，SQ8 启用）。
- recall@10 ≥0.95（五档）。
- `cfg(target_arch="wasm32")` 仅 executor/mod.rs；核心算法零 cfg（I-5）。
- rayon 仅 executor impl；wasm32 不启 rayon。
- 既有 10万测试不退步。

## 6. 前置依赖
- M2-09（SQ8 降内存，100万可行）。
- M2-07（懒加载，100万 open 不全加载）。

## 7. 不变量覆盖
- **I-5 核心零平台分支**：`cfg(target_arch)` 仅 executor/mod.rs + vfs impl；算法零 cfg。测试 4 守护。rayon 仅 Executor impl。测试 5 守护。
- **§3.3 100万恢复承诺**：测试 6+7+8 守护。
- **§11 Executor trait**：native=rayon，wasm=串行，cfg 仅 impl。测试 1+2+10 守护。
- **§13.1 延迟**：测试 7 守护（100万放宽档）。
- **I-4 单一分词身份**：测试 12 守护（reindex 不并发）。
