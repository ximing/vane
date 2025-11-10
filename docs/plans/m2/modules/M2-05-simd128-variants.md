# M2-05 SIMD128 双变体

## 1. 目标
产出 wasm simd128 默认 / scalar fallback 两个 wasm 产物，init 时 `WebAssembly.validate` 探针选择（用户只下载其一），保证 SIMD 数值路径不分歧（SPEC §8.4/§12.2/§3.6，REQUIREMENTS §4.1）。

SPEC 节号：§8.4（SIMD128/scalar 双变体各跑召回回归）、§12.2（wasm 双变体目标矩阵）、§3.6（Executor wasm=串行，但 SIMD 是数值路径）。

## 2. 涉及文件
- **Modify** `crates/vane-wasm/src/simd_probe.rs`（M2-01 占位）：落实 `simd128_supported()` 用 `WebAssembly.validate` 探测 simd128 模块。
- **Create** `crates/vane-wasm/build.rs`（或 `scripts/build-wasm-variants.sh`）：双产物构建脚本——`RUSTFLAGS="-Ctarget-feature=+simd128"` 构建 `vane_wasm_simd.wasm`；默认构建 `vane_wasm_scalar.wasm`。
- **Modify** `crates/vane-wasm/Cargo.toml`：增 `[features] simd128 = []`（feature-gated，启用时编译 simd128 路径；core 算法零 cfg，simd128 feature 仅在 vane-wasm 胶水层标注，不进 core）。
- **Modify** `crates/vane-core/src/vector/mod.rs`（**评估**）：core 的暴力距离/HNSW 距离计算是否用 `std::arch::wasm32::*` SIMD intrinsics。**若 core 引入 SIMD intrinsics，必须 `#[cfg(target_feature="simd128")]` 包裹且不出现在非 wasm32 路径**——评估后若需引入，走 cfg(target_feature) 在 core 算法处，**这违反 I-5 严格读法**，需停下标注「⚠️ 需 SPEC 修订」。
  - **首选方案**：core 不引入手写 SIMD intrinsics，依赖 LLVM 自动向量化（`-Ctarget-feature=+simd128` 时编译器自动向量化 f32 距离循环）。两产物由构建 flag 区分，core 代码零 cfg。本模块按首选方案推进。

## 3. 接口契约
### Consumes from
- M2-01 vane-wasm cdylib + `simd_probe` 占位。
- M0 `vane_core::vector::brute_search`、M1 `HnswReader::search`（距离计算路径，依赖 LLVM 向量化）。

### Produces for
```rust
// crates/vane-wasm/src/simd_probe.rs
pub fn simd128_supported() -> bool;  // WebAssembly.validate(simd128 test module) → true/false

// 构建产物：
//   vane_wasm_simd.wasm   (RUSTFLAGS="-Ctarget-feature=+simd128")
//   vane_wasm_scalar.wasm (默认)
```
下游：M2-04（Worker init 探针选产物）、M2-06（双变体召回回归）。

## 4. TDD 测试清单
1. **探针正确性**：`simd128_supported()` 在支持 simd128 的浏览器返 `true`，不支持返 `false`（wasm-bindgen-test，跨浏览器矩阵）。
2. **simd 产物构建**：`RUSTFLAGS="-Ctarget-feature=+simd128" cargo build --release --target wasm32-unknown-unknown -p vane-wasm` 产出 `vane_wasm_simd.wasm`，`wasm-objdump -x` 显示 `features: simd128`。
3. **scalar 产物构建**：默认构建产出 `vane_wasm_scalar.wasm`，`wasm-objdump -x` 不含 simd128 feature。
4. **LLVM 自动向量化验证**（reviewer B-I6）：`wasm-objdump -d vane_wasm_simd.wasm | grep -E 'f32x4|i32x4|v128'` 命中 simd128 指令（证明 brute_search/HNSW 距离循环被 LLVM 实际向量化，而非仅 feature flag 开启但无数值收益）。**若 grep 无命中**：自动向量化不足，回退 trait Distance 方案——`trait Distance { fn distance(a,b) -> f32 }`，simd/scalar 两 impl，`cfg(target_feature="simd128")` 在 impl 处（非算法处），停下标注「⚠️ 需 SPEC 修订：core 算法引入 trait Distance 抽象，需用户批准 I-5 再澄清」。
5. **召回回归 gate**（与 M2-06 协同）：simd 产物 vs scalar 产物 recall@10 Jaccard ≥0.99（M2-06 测试 3）；若自动向量化导致 recall 退步（<0.95），回退 trait Distance 方案。
6. **两产物体积**：simd 产物 gzip ≤800KB，scalar 产物 gzip ≤800KB（SPEC §13.2-3，双门禁）。
7. **两产物功能等价**（smoke）：两产物各跑 open→collection→add→flush→search 端到端，结果一致（数值分歧由 M2-06 召回回归覆盖）。
8. **core 零 cfg**：`grep -rn 'cfg(target_feature\|cfg(target_arch' crates/vane-core/src/` 仅命中 executor（M2-10 引入）/vfs（`vfs/mod.rs:18` `cfg(not(target_arch="wasm32"))` std_fs 模块）——本模块不引入新 core cfg。**首选方案下 core 零新增 cfg**。
9. **Worker init 选产物**：`simd128_supported()==true` → import simd 产物；`false` → import scalar 产物（M2-04 协同，本测试 mock）。

## 5. 验收标准
- 双产物构建脚本可重复执行，产出两 `.wasm`。
- 两产物 gzip 均 ≤800KB。
- 探针在主流浏览器（Chrome/Edge/Firefox/Safari）正确返 true/false（simd128 普遍支持，预期 true；旧 Safari 可能 false）。
- core 零新增 `cfg(target_feature)`/`cfg(target_arch)`（首选方案）。
- clippy clean（双构建配置）。

## 6. 前置依赖
- M2-01（vane-wasm cdylib）。

## 7. 不变量覆盖
- **I-5 核心零平台分支**：首选方案 core 零新增 cfg，依赖 LLVM 自动向量化。测试 6 守护。**若评估发现必须手写 SIMD intrinsics 进 core**，停下标注「⚠️ 需 SPEC 修订：core 算法引入 `cfg(target_feature="simd128")` 违反 I-5 严格读法，需用户批准 I-5 再澄清或改用 trait 抽象（距离计算抽象为 `trait Distance { fn distance(a,b) -> f32 }`，simd/scalar 两 impl，cfg 在 impl 处而非算法处）」。
- **§8.4 双变体召回回归**：本模块产出双产物，回归由 M2-06 落实。
- **体积门禁**：测试 4 双门禁。
