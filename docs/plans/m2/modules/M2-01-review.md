# M2-01 vane-wasm cdylib + 体积门禁 — 评审报告

**评审日期**：2026-08-09
**评审者**：task reviewer（只读）
**评审对象**：BASE 096b6b1..HEAD 6818eeb
**状态**：**PASS_WITH_FINDINGS**（0 B / 0 I / 2 M）

---

## 0. 评审范围

| 项 | 文件 |
|----|------|
| API 胶水 | `crates/vane-wasm/src/lib.rs` |
| SIMD 探针 | `crates/vane-wasm/src/simd_probe.rs` |
| Cargo | `crates/vane-wasm/Cargo.toml`、`crates/vane-core/Cargo.toml` |
| core 改动 | `crates/vane-core/src/segment/ulid.rs`、`crates/vane-core/src/persistence/mod.rs` |
| CI/脚本 | `.github/workflows/ci.yml`、`scripts/check-wasm-size.sh`、`scripts/check-no-std-fs.sh` |
| 测试 | `crates/vane-wasm/tests/web.rs` |
| 计划/报告 | `docs/plans/m2/modules/M2-01-wasm-cdylib-size.md`、`-report.md` |

---

## 1. web-time 依赖 soundness（重点）— **PASS**

### 1.1 native 零开销（re-export std::time）— 确认

`web-time 1.1.0`（Cargo.lock:1017）的 js-sys/wasm-bindgen 依赖均为 **target-gated**：

```
js-sys      : target = cfg(all(target_family = "wasm", target_os = "unknown"))
wasm-bindgen: target = cfg(all(target_family = "wasm", target_os = "unknown"))
```

（经 `cargo metadata` 确认，web-time 的非 dev 依赖中 js-sys/wasm-bindgen 仅在 wasm target 拉入。）

→ **native 构建不拉入 js-sys/wasm-bindgen**，web-time 在 native 纯 re-export `std::time::Instant/SystemTime`。vane-core native 行为/性能零变化。报告"native 零开销"成立。

### 1.2 wasm32 走 js-sys — 确认

- `persistence/mod.rs:152,160,189`：`AutoCommitter.last_flush: web_time::Instant`，`Instant::now()` / `elapsed()` 调用形态不变。
- `segment/ulid.rs:13,19`：`use web_time::{SystemTime, UNIX_EPOCH}`，`SystemTime::now().duration_since(UNIX_EPOCH)` 调用形态不变。

wasm32 下 web-time 经 js-sys 调 `performance.now()`（Instant）/`Date.now()`（SystemTime），消解 M0 已知 panic 遗留。✓

### 1.3 I-5 守护（vane-core 生产代码零 cfg(target)）— 确认

grep `cfg(target` 在 vane-core/src 生产代码仅命中：
- `vfs/std_fs.rs`（5 处 `#[cfg(not(target_arch = "wasm32"))]`）— **合法**，StdFsVfs 是 VFS 层 cfg 隔离，check-no-std-fs.sh 排除。
- `vfs/mod.rs:18`（`#[cfg(not(target_arch = "wasm32"))] pub mod std_fs;`）— 模块门控，合法。

**算法/持久化/段/ulid 生产代码零 `cfg(target)`**。web-time 的平台差异封装在 web-time crate 内部，core 代码无平台分支。I-5 守护不变。✓

### 1.4 std::time 残留核查 — 确认无生产残留

grep `std::time::` 在 vane-core/src：
- `vfs/tests.rs:95-96` — 在 `#[cfg(not(target_arch="wasm32"))] mod std_fs_tests` 内（test fixture，cfg 隔离），benign。
- `segment/ulid.rs:7` — 注释文本（"re-export std::time::SystemTime"），非代码，benign。
- `api/reindex_tests.rs:301,308,406,413` — test 文件，`thread::sleep(std::time::Duration)`，benign。
- `vector/mod.rs:601` — 在 `#[test] #[ignore] fn perf_100k_384_cosine_top10` 内（确认 line 592-607），benign。编排者说明正确。

**生产代码零 `std::time::Instant/SystemTime` 残留**。✓

### 1.5 依赖黑名单 / cargo-deny — 确认

`deny.toml` 黑名单：regex/tokio/prost/tonic/openssl/lindera/ndarray/wee_alloc/dashmap/parking_lot。
- `web-time` 不在黑名单。✓
- 传递依赖 `js-sys`/`wasm-bindgen` 不在黑名单。✓
- license：web-time（MIT）、js-sys/wasm-bindgen（MIT/Apache-2.0）均在 allow 列表。报告称 `cargo deny check` advisories/bans/licenses/sources 全 ok。✓

### 1.6 是否超范围 — 判断：**合理**

vane-core 用 time 是既有事实（AutoCommitter `Instant` + gen_ulid `SystemTime`），wasm32 panic 是 M0 标注"M2 处理"的真 bug。web-time 是最干净修法（cfg 在 web-time 内部，core 零 cfg，native 零开销）。非 pub API 变更，属 bugfix 范畴。不超范围。✓

---

## 2. I-8 binding 薄壳 — **PASS**

`crates/vane-wasm/src/lib.rs`：
- 全部 10 个导出函数（vane_open/collection/add/flush/search/delete/compact/reindex/export/close）纯转发 `vane_core::api`，无检索逻辑。✓
- 错误映射：`err_to_js(e: VaneError) -> JsValue`（lib.rs:111-113），`VaneError::name()` + Display 拼字符串 throw。所有 `?` 经 `.map_err(err_to_js)`。✓
- 返回类型：句柄 `u64`、`()`、`String`（hits JSON）、`f32`（progress），均 `JsResult<T>`。✓
- JSON 解析辅助（parse_schema/parse_docs/parse_search_query/hits_to_json 等）与 vane-ffi convert 同构，各 binding 层独立维护。✓
- `vane_export`（lib.rs:484-491）调 `db.export(dest)`，core stub（`api/db.rs:167-169`）返 `Err(VaneError::Unsupported)`，wasm 转发 throw。与计划"M2-12 接入"一致。✓
- `vane_reindex`（lib.rs:471-480）同步调 `col.reindex()` + `rh.progress()`，M1 同步语义，返 1.0。ReindexHandle 存入 registry 保持存活（`#[allow(dead_code)]` on reindex field，lib.rs:37，合理）。✓

---

## 3. 800KB 体积门禁 — **PASS**

`scripts/check-wasm-size.sh` 双口径：
- **口径 1**（真实 deliverable）：`cargo build --release --target wasm32-unknown-unknown -p vane-wasm`（default features）→ `wasm-opt -Oz` → `gzip | wc -c`。报告 348,473 bytes（340KB）≤ 800KB。✓
- **口径 2**（保守上界）：`RUSTFLAGS=-C link-arg=--export-all cargo build --release --target wasm32-unknown-unknown -p vane-core` → wasm-opt -Oz → gzip。报告 636,722 bytes（622KB）≤ 800KB。✓

脚本 wasm-opt 可用性有 fallback（line 26-32），测量逻辑正确。

**M2-08 carry-forward 消解**：M2-00 骨架 11.4KB 是占位 LTO 剥离 ruzstd decode 路径。本模块接入真实 API 后 ruzstd decode 被引用，体积上升至 340KB（default），carry-forward 已消解。✓

**vane-core --export-all 622KB 增量**（491KB→622KB）：来自 web-time 在 wasm32 拉入 js-sys（performance.now()/Date.now() 绑定）。vane-wasm default（340KB）因 js-sys/wasm-bindgen 本就在 vane-wasm 依赖树，无额外增量。增量可接受。✓

⚠️ 体积数值为报告自证，未独立复测（只读评审不跑 cargo）。脚本逻辑正确，CI wasm32-size job 会复现。

---

## 4. SIMD 探针占位 — **PASS**

`crates/vane-wasm/src/simd_probe.rs`：
- `simd128_supported() -> bool` 恒返 `false`（line 14-16）。✓
- 占位注释说明 M2-05 落实 `WebAssembly.validate(simd_module_bytes)`。✓
- 单元测试 `placeholder_returns_false`（line 23-26）。✓

---

## 5. CI / 脚本 — **PASS**

`.github/workflows/ci.yml`：
- `wasm32-check` job（line 56-73）：增 `cargo check --target wasm32-unknown-unknown -p vane-wasm`（line 68-69）+ `cargo clippy --target wasm32-unknown-unknown -p vane-wasm -- -D warnings`（line 70-71）+ check-no-std-fs.sh 覆盖 vane-wasm（line 72-73）。✓
- `wasm32-size` job（line 119-132）：跑 `check-wasm-size.sh` 双口径。✓

`scripts/check-no-std-fs.sh`（line 18-22）：增 vane-wasm 覆盖，`grep -rn 'std::fs::\|std::net::\|mmap' crates/vane-wasm/src/` 无输出（实测确认无命中）。✓

vane-wasm/src 零 `cfg(target)`、零 `std::fs/std::net/mmap`。✓

---

## 6. 词典永不进 wasm — **PASS**

`crates/vane-wasm/Cargo.toml`：
- `default = []`（line 26）。✓
- `vane-core = { workspace = true, features = ["zstd-decode"] }`（line 14）— 仅启 zstd-decode（ruzstd 纯 Rust），不启 jieba/dict-zh。✓
- jieba/dict-zh 非 default，词典数据不进 wasm。✓

check-wasm-size.sh 口径 1 用 default features 构建测量，守护红线。✓

---

## 7. TDD 覆盖 — **PASS（含 1 Minor）**

`crates/vane-wasm/tests/web.rs` — 4 个 wasm-bindgen-test（node）：
1. `open_collection_add_flush_search_roundtrip`：open(MemoryVfs) → collection → add(3) → flush → search(vector, d1 top) → search(text, d1+d3) → close。I-8 端到端。✓
2. `delete_and_compact`：add → flush → delete("a") → compact → search 验证 "a" 不出现。✓
3. `simd_probe_placeholder_false`：占位返 false。✓
4. `version_nonempty`：版本非空。✓

覆盖 open→search 端到端 + delete/compact + simd 占位。

---

## 8. 发现清单

### M-1：CI 未跑 wasm-bindgen-test（信息性）

**证据**：`.github/workflows/ci.yml` wasm32-check job（line 56-73）仅跑 `cargo check` + `cargo clippy`，未跑 `cargo test --target wasm32-unknown-unknown -p vane-wasm --test web` + `wasm-bindgen-test-runner`。报告第132行承认"本模块未加 CI wasm-bindgen-test job——可后续模块加"。

**影响**：4 个 JS 行为测试仅在本地验证，CI 不守护回归。计划验收标准未要求 CI 跑 wasm-bindgen-test，故非阻断；但 I-8 薄壳行为测试是核心守护，建议后续模块（M2-02+）在 CI 增 wasm-bindgen-test step（需装 wasm-bindgen-cli 0.2.127）。

**建议**：M2 后续模块加 CI wasm-bindgen-test job。

### M-2：vane_export 注释措辞（信息性）

**证据**：`crates/vane-wasm/src/lib.rs:482` 注释"M2-12 接入；当前返 E_UNSUPPORTED"，但代码（line 489）直接调 `db.export(dest)`，Unsupported 由 core stub（`api/db.rs:167-169`）返回，非 wasm 层 stub。

**影响**：行为正确（throw Unsupported），注释暗示 wasm 层有 stub，实际是 core 层 stub 透传。非问题，注释可更精确（如"转发 Db::export，core 当前返 Unsupported，M2-12 落实"）。

**建议**：注释微调（可选）。

---

## 9. 总结

| 重点 | 结论 |
|------|------|
| web-time soundness | PASS — native 零开销（js-sys target-gated 不拉入），wasm32 经 js-sys 正确，core 零 cfg（I-5），不在黑名单，license ok |
| I-5（core 零 cfg(target)） | PASS — 生产代码零 cfg(target)，vfs 层 cfg 合法，web-time 平台差异封装在 crate 内部 |
| 800KB 体积门禁 | PASS — default 340KB / --export-all 622KB 双口径 ≤800KB，M2-08 carry-forward 消解 |
| I-8 薄壳 | PASS — 纯转发 vane_core::api，错误映射/返回类型正确 |
| SIMD 占位 | PASS — 恒 false |
| CI/脚本 | PASS — wasm32-check + wasm32-size 覆盖 vane-wasm，check-no-std-fs 扩展 |
| 词典不进 wasm | PASS — default=[]，仅 zstd-decode |
| TDD | PASS — 4 测试覆盖端到端（CI 未跑，M-1） |

**状态**：PASS_WITH_FINDINGS（0 B / 0 I / 2 M）

web-time 是消解 M0 wasm32 panic 遗留的干净修法，core 零 cfg、native 零开销、wasm32 经 js-sys 正确，I-5 守护不变。800KB 双口径远在门禁内，M2-08 carry-forward 消解。2 个 Minor 均为信息性（CI wasm-bindgen-test 缺失 + 注释措辞），非阻断。
