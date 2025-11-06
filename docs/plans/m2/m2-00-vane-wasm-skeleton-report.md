# M2-00 vane-wasm 骨架报告

> 产出：M2 Phase Zero developer SubAgent（2026-08-09）
> 起点：HEAD `c2bd0bb`（main，SPEC v1.2 已批准，340 测试绿）
> 任务：新建 `crates/vane-wasm` cdylib 骨架（wasm-bindgen 占位），测真实 wasm 体积基线。

---

## 1. 逐项改动

### 1.1 新建 `crates/vane-wasm/Cargo.toml`

```toml
[package]
name = "vane-wasm"
version.workspace = true
edition.workspace = true
license.workspace = true

[lib]
crate-type = ["cdylib"]

[dependencies]
vane-core = { workspace = true }
wasm-bindgen = "0.2"
```

- `crate-type = ["cdylib"]`：仅 wasm 产物（无 rlib，本 crate 不被其他 rust crate 依赖）。
- `vane-core = { workspace = true }`：**default features**，不启用 jieba/dict-zh——词典永不进 wasm（红线）。workspace 注册的 `vane-core` 默认无 jieba/ruzstd/dict-zh。
- `wasm-bindgen = "0.2"`：workspace 未注册，本 crate 直接指定（解析到 0.2.127，当前稳定）。不在依赖黑名单。
- 不引 web-sys/js-sys/任何浏览器 API。

### 1.2 新建 `crates/vane-wasm/src/lib.rs`

```rust
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn vane_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}
```

占位导出，仅 `vane_version()` 返回 CARGO_PKG_VERSION。无 std::fs/std::net/mmap。

### 1.3 根 `Cargo.toml` workspace members 追加

```diff
-members = ["crates/vane-core", "crates/vane-ffi", "crates/vane-node", "crates/vane-dict-zh"]
+members = ["crates/vane-core", "crates/vane-ffi", "crates/vane-node", "crates/vane-dict-zh", "crates/vane-wasm"]
```

末尾追加，保持原 4 个顺序。

### 1.4 vane-core 未改

零改动（回归未破）。

---

## 2. 自证门禁结果

| # | 门禁 | 命令 | 结果 |
|---|---|---|---|
| 1 | wasm32 check vane-wasm | `cargo check --target wasm32-unknown-unknown -p vane-wasm` | `Finished dev profile in 8.94s` ✅ |
| 2 | wasm32 check vane-core 回归 | `cargo check --target wasm32-unknown-unknown -p vane-core` | `Finished in 0.07s`（缓存命中）✅ |
| 3 | workspace 全测试 | `cargo test --workspace --all-features` | 340 passed, 0 failed（250 unit + 90 integration/doc），未回退基线 ✅ |
| 4 | clippy | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | `Finished dev profile in 9.58s`，clean ✅ |
| 5 | fmt | `cargo fmt --all -- --check` | exit 0，clean ✅ |
| 6a | no-std-fs 脚本 | `bash scripts/check-no-std-fs.sh` | `OK` ✅ |
| 6b | vane-wasm grep | `grep -rn 'std::fs::\|std::net::\|mmap' crates/vane-wasm/src/` | 无输出（exit 1 = no match）✅ |
| 7 | 体积基线 | 见下节 | gzip ≤ 800KB ✅ |
| 8 | cargo deny | `cargo deny check` | `advisories ok, bans ok, licenses ok, sources ok` ✅ |

### 门禁 6b grep 原始输出

```
$ grep -rn 'std::fs::\|std::net::\|mmap' crates/vane-wasm/src/
$ echo $?
1
```

无输出（exit 1 = grep 无匹配）。

---

## 3. 体积基线数值

构建命令：`cargo build --target wasm32-unknown-unknown -p vane-wasm --release`
wasm-opt：`/opt/homebrew/bin/wasm-opt`（binaryen 131，brew install）
gzip：`gzip -c file | wc -c`

| 构建模式 | raw (bytes) | wasm-opt -Oz (bytes) | gzip (bytes) | gzip (KB) |
|---|---|---|---|---|
| default（仅 vane_version 导出，vane-core 被 dead-code 消除） | 35,928 | 26,554 | 9,697 | 9.46 |
| `--export-all`（保守上界，强制导出所有符号，对照 vane-core 脚本方法论） | 609,646 | 435,317 | 154,777 | 151.14 |

**断言**：gzip ≤ 800KB（SPEC §13.2-3）。
- default gzip 9.46 KB << 800 KB ✅
- --export-all gzip 151.14 KB << 800 KB ✅（最保守口径）

### 与 vane-core cdylib 基线对比

- M1 vane-core cdylib `--export-all` gzip：557 KB（M1-SUMMARY §2，check-wasm-size.sh 方法论）。
- vane-wasm `--export-all` gzip：151 KB。**远低于** vane-core 单独 cdylib，原因：vane-wasm 作为 cdylib 顶层，vane-core 以 rlib 依赖链接，linker 仅保留 `#[wasm_bindgen]` 导出可达的代码路径（vane_version 仅用 `env!` 宏，不触达 vane-core 任何检索/HNSW/BM25 代码）；`--export-all` 虽强制导出，但 wasm-ld 对 rlib 内部私有符号仍受可达性裁剪。
- scoping 报告 §2.3 预估 "wasm-bindgen 胶水 +10~20KB gzip"：实测 default opt gzip 9.46 KB（含胶水），符合预估量级。
- **后续 M2-01+ 加入真实检索 API 胶水后体积将显著上升**（vane-core 检索路径被拉入），届时以 `--export-all` 口径跟踪。当前骨架基线已建立。

---

## 4. 遗留 / 疑问

1. **CI wasm32-size job 未更新**：scoping 报告 §2.3 任务 3 要求 CI 增加 `cargo build -p vane-wasm` + 体积测量。本任务仅建骨架+本地测基线，未改 CI workflow（`.github/workflows/`）。建议 M2-01 在 CI job 中补 vane-wasm 体积测量，复用 `scripts/check-wasm-size.sh` 模式。
2. **体积测量口径选择**：default 构建（9.46 KB）反映真实部署体积（只导出 vane_version），--export-all（151 KB）是保守上界。后续 M2-01 增加真实 API 后，建议以 default 构建为准（真实 deliverable 体积），--export-all 仅作回归膨胀告警。
3. **wasm-bindgen 版本未入 workspace.dependencies**：本 crate 直接指定 `wasm-bindgen = "0.2"`。若后续 M2-01+ 需在多 crate 共享（如独立 Worker 胶水 crate），可考虑注册到 workspace.dependencies。当前单 crate 使用，无即时需求。
4. 无 deny.toml 冲突（wasm-bindgen 不在 ban 列表，licenses MIT/AGPL-3.0 均 allow 或不在 wasm32 依赖链）。

---

## 5. 结论

vane-wasm 骨架建立完成，全 8 项自证门禁绿，体积基线 gzip 9.46 KB（default）/ 151 KB（--export-all），远在 800KB 门禁内。未引任何浏览器 API。可进入 M2-01 真实胶水交付。
