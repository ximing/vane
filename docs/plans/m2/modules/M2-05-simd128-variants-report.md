# M2-05 SIMD128 双变体 — 实施报告

## 1. 实施概要

落实 vane-wasm SIMD128 双产物构建 + 真实运行时探针。**首选方案**：core 算法零 cfg，依赖 `-Ctarget-feature=+simd128` 启用 LLVM SIMD 代码生成。

### 1.1 simd_probe 实装（`crates/vane-wasm/src/simd_probe.rs`）

- 替换 M2-01 占位 `false`，落实真实探针 `simd128_supported()`。
- 探针：`WebAssembly.validate(SIMD128_TEST_MODULE)` —— 一个最小 simd128 wasm 模块（38 字节，含 `v128.const` 指令 opcode `FD 0C`）。仅 simd128 运行时 validate 通过；不支持则返 false 或抛 CompileError（catch 后置 false）。
- JS 绑定经 `#[wasm_bindgen(js_namespace = WebAssembly, catch)]` 外部声明，`&[u8]` 自动转 `Uint8Array`，**不引入 js-sys 依赖**（js-sys 仍 optional，default 构建不受影响）。
- `#[cfg(target_arch = "wasm32")]` 分派：wasm32 调真实探针；非 wasm32（host 测试）无 `WebAssembly` 对象恒返 false。
- 探针模块字节常量 `SIMD128_TEST_MODULE` 公开，供下游/测试引用。

### 1.2 构建脚本（`scripts/build-wasm-variants.sh`）

- simd 变体：`RUSTFLAGS="-Ctarget-feature=+simd128" cargo build --release --target wasm32-unknown-unknown -p vane-wasm --features worker`
- scalar 变体：默认构建（无 simd128 target-feature）。
- 两产物经 `wasm-opt -Oz` 优化后拷贝到 `target/wasm-variants/`。
- 内置特征校验（wasm-objdump target_features 段 + SIMD 指令计数）+ 体积门禁（gzip ≤ 800KB）。
- 环境变量：`FEATURES`（默认 worker）、`OUT_DIR`、`NO_OPT`。

### 1.3 Cargo.toml

- 新增 `[features] simd128 = []`（marker feature，不门控任何 core 代码；core 算法零 cfg）。

## 2. 自证门禁结果

| # | 门禁 | 结果 |
|---|------|------|
| 1 | `cargo check --target wasm32-unknown-unknown -p vane-wasm --features worker` | ✅ 通过 |
| 2 | `cargo test --workspace --all-features` | ✅ 459 绿（456 基线 + 4 新增 simd_probe 测试 − 1 移除占位测试） |
| 3 | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | ✅ clean |
| 4 | `cargo fmt --all -- --check` | ✅ clean |
| 5 | `bash scripts/check-no-std-fs.sh` | ✅ OK |
| 6 | `cargo deny check` | ✅ ok（advisories/bans/licenses/sources） |
| 7 | `bash scripts/build-wasm-variants.sh` 双产物 | ✅ 产出两 `.wasm` |
| 8 | wasm-objdump 特征 | ✅ simd 含 `[+] simd128`；scalar 不含 |
| 9 | LLVM 自动向量化 grep | ⚠️ 3116 命中（i32x4/v128），但 **f32x4 = 0**（见 §3） |
| 10 | 双产物体积 gzip ≤800KB | ✅ simd 406123 bytes / scalar 409415 bytes |
| 11 | 探针测试 | ⚠️ node v20.20.1 返 **true**（支持 simd128，非 spec 假设的 false）；host 返 false（见 §4） |
| 12 | core 零新增 cfg | ✅ `grep -rn 'cfg(target_feature\|cfg(target_arch' crates/vane-core/src/` 空（0 命中） |

## 3. 自动向量化是否充分（决定方案走向）— 关键发现

### 3.1 字面门禁通过

`wasm-objdump -d vane_wasm_simd.wasm | grep -E 'f32x4|i32x4|v128'` 命中 **3116 行**，scalar 变体 **0 命中**。spec 的字面条件「必须命中 simd128 指令」满足，不触发「grep 无命中 → 停下上报 SPEC 修订」分支。

### 3.2 ⚠️ 但 f32x4 = 0 —— f32 距离循环未被自动向量化

指令分布（simd 变体）：

| 指令类型 | 计数 | 来源 |
|----------|------|------|
| v128.load / v128.store | 1399 / 1350 | roaring 位图（依赖库内部 SIMD 路径） |
| v128.const | 152 | roaring |
| v128.and / v128.or / v128.not | 36 / 18 / 13 | roaring 位图运算 |
| i32x4.add / i32x4.extract_lane / i32x4.lt / ... | ~80 | roaring |
| **f32x4.\*** | **0** | **（无）—— vane 的 cosine/l2/dot 距离循环未被向量化** |

**根因**：`crates/vane-core/src/vector/mod.rs` 的 `cosine_score`/`l2_score`/`dot_score` 是 f32 归约循环（`dot += a[i] * b[i]`）。浮点加法不满足结合律，LLVM loop vectorizer 在无 fast-math 标志时拒绝向量化 f32 归约（会改变数值结果）。Rust stable 无稳定 per-loop fast-math 标注（`-Cllvm-args=-fast-math` 全局未启用）。因此 f32 距离循环保持标量。

### 3.3 SIMD 收益来源

simd 变体的 3116 条 SIMD 指令**全部来自 roaring 依赖**（roaring 位图库内部用 `#[cfg(target_feature = "simd128")]` 门控其 SIMD 路径）。这是**真实收益**（位图运算提速），非仅 feature flag 开关——pre-filter 场景（`filter` 位图扫描）受益；但向量距离计算本身无 SIMD 加速。

### 3.4 方案判定

- **首选方案（core 零 cfg）成立**：core 算法零新增 cfg，I-5 不变量保持。
- **字面门禁通过**：spec「grep 无命中 → SPEC 修订」分支未触发。
- **但 spec 测试 4 括号意图（「证明 brute_search/HNSW 距离循环被 LLVM 实际向量化」）未达成**：f32 距离循环保持标量。
- **结论**：不触发 BLOCKED。标 DONE_WITH_CONCERNS，交编排者裁定：
  - 若 SIMD 仅需「feature flag 有真实 codegen 差异」（roaring 受益）→ 接受当前方案。
  - 若 SIMD 必须加速 f32 距离计算 → 需 SPEC 修订：引入 `trait Distance { fn distance(a,b) -> f32 }` 抽象，simd impl 用 `std::arch::wasm32::f32x4::*` intrinsics（cfg 在 impl 处，非算法处），需用户批准 I-5 再澄清。

## 4. 探针测试（门禁 11）说明

- **node v20.20.1**：`WebAssembly.validate(SIMD128_TEST_MODULE)` 返 **true**（node v16+ 默认启用 simd128）。spec 假设「node 无 simd128 → 探针 false」已过时——现代 node 支持 simd128。
- **host（macOS，非 wasm32）**：`simd128_supported()` 返 **false**（无 `WebAssembly` 对象，`#[cfg(not(target_arch="wasm32"))]` 路径）——探针 false 路径在 host 单元测试覆盖。
- **浏览器**：预期 true（Chrome/Edge/Firefox/Safari 普遍支持 simd128）；旧 Safari 可能 false。浏览器路径由 M2-06 召回回归覆盖。
- 探针逻辑正确：`WebAssembly.validate` 是 stdlib，无论运行时是否支持 simd128 都可调用；不支持时返 false（不抛错），支持时返 true。

## 5. 产物路径与体积

| 产物 | 路径 | gzip 体积 |
|------|------|-----------|
| simd | `target/wasm-variants/vane_wasm_simd.wasm` | 406123 bytes (397KB) |
| scalar | `target/wasm-variants/vane_wasm_scalar.wasm` | 409415 bytes (400KB) |

两产物均 ≤ 800KB（SPEC §13.2-3）。simd 比 scalar 小 3292 bytes（wasm-opt 对 SIMD 指令序列优化更紧凑）。

## 6. core cfg grep（门禁 12）

```
$ grep -rn 'cfg(target_feature\|cfg(target_arch' crates/vane-core/src/
（空，0 命中）
```

既有 `cfg(not(target_arch = "wasm32"))`（vfs/mod.rs:18、std_fs.rs、tests.rs）使用 `cfg(not(...))` 形式，不含子串 `cfg(target_arch`，不被本 grep 匹配——属 M0 既有 vfs 平台门，本模块未引入新 core cfg。I-5 不变量保持。

## 7. wasm-objdump 特征输出（门禁 8）

simd 变体 `target_features` 自定义段：
```
 - name: "target_features"
  - [+] mutable-globals
  - [+] nontrapping-fptoint
  - [+] simd128
  - [+] bulk-memory
  - [+] sign-ext
  - [+] reference-types
  - [+] multivalue
  - [+] bulk-memory-opt
  - [+] call-indirect-overlong
```

scalar 变体 `target_features` 段**不含** `simd128`（其余特征相同）。

## 8. 涉及文件

- **Modify** `crates/vane-wasm/src/simd_probe.rs` —— 真实探针实装（替换占位 false）。
- **Modify** `crates/vane-wasm/Cargo.toml` —— 新增 `simd128 = []` marker feature。
- **Create** `scripts/build-wasm-variants.sh` —— 双产物构建脚本。
- **Modify** `.gitignore` —— 排除 `target/wasm-variants/`（wasm 产物不入库）。

## 9. 遗留

1. **f32 距离循环未 SIMD 化**（§3.2）—— 标 DONE_WITH_CONCERNS，交编排者裁定是否需 SPEC 修订（trait Distance 抽象）。
2. **浏览器探针 true 路径**未在本模块自动测试（需 wasm-bindgen-test 浏览器矩阵），由 M2-06 召回回归间接覆盖。
3. **双变体召回回归**（SPEC §8.4）由 M2-06 落实，本模块仅产出双产物。
4. node v20 实际支持 simd128（返 true），与 spec 假设不符——探针逻辑正确，spec 假设过时。
