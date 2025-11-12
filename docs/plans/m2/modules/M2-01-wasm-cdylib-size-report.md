# M2-01 vane-wasm cdylib + 体积门禁 — 报告

## 1. 概述

在 M2-00 vane-wasm 骨架（仅 `vane_version()` 占位）基础上，加入真实检索/管理 API 的 wasm-bindgen 胶水 + SIMD 探针占位，CI wasm32-size job 切到 vane-wasm 真实 deliverable 口径，强制 800KB gzip 门禁。

**关键修复**：vane-core 的 `AutoCommitter`（`std::time::Instant::now()`）和 `gen_ulid`（`std::time::SystemTime::now()`）在 wasm32-unknown-unknown 运行时 panic（无单调时钟）。M0 注释标注"M2 处理"。本模块用 `web-time` crate 消解——native 零开销 re-export `std::time`，wasm32 经 `js-sys` 调 `performance.now()`/`Date.now()`。

## 2. API 胶水（逐函数）

`crates/vane-wasm/src/lib.rs` — 与 vane-ffi 同构的句柄注册表 + JSON 序列化（I-8 薄壳）。

| 函数 | 签名 | 内部调 core | 说明 |
|------|------|------------|------|
| `vane_open(path, opts_json)` | `-> Result<u64, JsValue>` | `Db::open(Arc<MemoryVfs>, path, opts)` | MemoryVfs（M2-02 接 OPFS） |
| `vane_collection(db_h, name, schema_json, opts_json)` | `-> Result<u64, JsValue>` | `Db::collection(name, schema, opts)` | 返回 Collection 句柄 |
| `vane_add(col_h, docs_json)` | `-> Result<u64, JsValue>` | `Collection::add(&docs)` | 返回 accepted 数 |
| `vane_flush(col_h)` | `-> Result<(), JsValue>` | `Collection::flush()` | |
| `vane_search(col_h, query_json)` | `-> Result<String, JsValue>` | `Collection::search(&query)` | 返回 Hit[] JSON |
| `vane_delete(col_h, ids_json)` | `-> Result<u64, JsValue>` | `Collection::delete(&ids)` | 返回已删数 |
| `vane_compact(col_h)` | `-> Result<(), JsValue>` | `Collection::compact()` | |
| `vane_reindex(col_h)` | `-> Result<f32, JsValue>` | `Collection::reindex()` | M1 同步，返 progress |
| `vane_export(db_h, dest)` | `-> Result<(), JsValue>` | `Db::export(dest)` | M2-12 接入 |
| `vane_close(handle)` | `-> Result<(), JsValue>` | `remove_handle(h)` | Db/Col/Reindex 均可 |

JSON convert 辅助（parse_schema/parse_docs/parse_search_query/hits_to_json 等）与 vane-ffi convert.rs 同构，各 binding 层独立维护（I-8）。

## 3. SIMD 探针占位

`crates/vane-wasm/src/simd_probe.rs`：
- `simd128_supported() -> bool` — 占位恒返 `false`。
- M2-05 落实 `WebAssembly.validate(simd_module_bytes)` 真实探针。
- M2-04/M2-05 消费此函数决定走 brute_search 还是 SIMD 路径。

## 4. Cargo features

`crates/vane-wasm/Cargo.toml`：
- `default = []`（红线：不启 dict-zh/jieba）
- vane-core features: `["zstd-decode"]`（M2-08，ruzstd 读 v2 stored.bin）
- `opfs`/`idb`/`worker` 占位 feature（M2-02/03/04 启用，feature-gated `web-sys`/`js-sys`）
- dev-dep: `wasm-bindgen-test`（node 行为测试）

## 5. vane-core 改动（web-time）

**必要修复**（非 pub API 变更，消解 M0 已知 panic 遗留）：

- `crates/vane-core/Cargo.toml`：增 `web-time = "1"`（native 零开销 re-export `std::time`；wasm32 经 js-sys 调 `performance.now()`/`Date.now()`）。
- `crates/vane-core/src/persistence/mod.rs`：`AutoCommitter.last_flush` 从 `std::time::Instant` → `web_time::Instant`。
- `crates/vane-core/src/segment/ulid.rs`：`gen_ulid` 从 `std::time::SystemTime` → `web_time::SystemTime`。

I-5 守护：vane-core 代码零 `cfg(target)` 分支（平台差异封装在 web-time 内部）。

## 6. CI / 脚本改动

### `.github/workflows/ci.yml`
- `wasm32-check` job：增 `cargo check --target wasm32-unknown-unknown -p vane-wasm` + `cargo clippy --target wasm32-unknown-unknown -p vane-wasm -- -D warnings`；check-no-std-fs.sh 覆盖 vane-wasm。
- `wasm32-size` job：`check-wasm-size.sh` 双口径测量（vane-wasm default + vane-core --export-all）。

### `scripts/check-wasm-size.sh`
- 口径 1（真实 deliverable）：`cargo build --release --target wasm32-unknown-unknown -p vane-wasm` → wasm-opt -Oz → gzip。
- 口径 2（保守上界对照）：`RUSTFLAGS=-C link-arg=--export-all cargo build --release --target wasm32-unknown-unknown -p vane-core` → wasm-opt -Oz → gzip。
- 两口径均 ≤800KB 才 pass。

### `scripts/check-no-std-fs.sh`
- 增 vane-wasm 覆盖：`grep -rn 'std::fs::\|std::net::\|mmap' crates/vane-wasm/src/` 无输出。

## 7. 体积实测（消解 M2-08 carry-forward）

M2-08 carry-forward：wasm 11.4KB 是占位 LTO 剥离 decode 路径。本模块接入真实检索 API 后 ruzstd decode 路径被引用，体积上升。

| 口径 | 体积（gzip） | 上限 | 状态 |
|------|-------------|------|------|
| vane-wasm default（真实 deliverable） | 348,473 bytes (340KB) | 800KB | PASS |
| vane-core --export-all（保守上界） | 636,722 bytes (622KB) | 800KB | PASS |

相比 M2-00 骨架（9.46KB default / 151KB --export-all），接入真实 API + ruzstd decode + web-time/js-sys 后体积上升，但远在 800KB 以内。

vane-core --export-all 从 491KB → 622KB：增量来自 web-time 在 wasm32 拉入 js-sys（`performance.now()`/`Date.now()` 绑定）。vane-wasm default（真实 deliverable）仅 340KB——js-sys/wasm-bindgen 本就在 vane-wasm 依赖树中，无额外增量。

命令：
```bash
bash scripts/check-wasm-size.sh
# vane-wasm default gzip size: 348473 bytes (max 819200)
# vane-core --export-all gzip size: 636722 bytes (max 819200)
```

## 8. JS 行为测试输出（wasm-bindgen-test, node）

`crates/vane-wasm/tests/web.rs` — 4 个测试：

```
running 4 tests
test version_nonempty ... ok
test simd_probe_placeholder_false ... ok
test open_collection_add_flush_search_roundtrip ... ok
test delete_and_compact ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 filtered out; finished in 0.04s
```

测试覆盖：
1. `open_collection_add_flush_search_roundtrip`：open(MemoryVfs) → collection → add(3 docs) → flush → search(vector mode, d1 top) → search(text mode, d1+d3 命中) → close。I-8 薄壳行为与 vane-core 等价。
2. `delete_and_compact`：add → flush → delete("a") → compact → search 验证 "a" 不出现。
3. `simd_probe_placeholder_false`：占位返 false。
4. `version_nonempty`：版本非空。

运行方式：
```bash
cargo test --target wasm32-unknown-unknown -p vane-wasm --test web
WASM=$(ls target/wasm32-unknown-unknown/debug/deps/web-*.wasm | head -1)
wasm-bindgen-test-runner "$WASM"
```

## 9. 自证门禁结果表

| # | 门禁 | 结果 |
|---|------|------|
| 1 | `cargo check --target wasm32-unknown-unknown -p vane-wasm` | PASS |
| 2 | `cargo check --target wasm32-unknown-unknown -p vane-core` | PASS |
| 3 | `cargo test --workspace --all-features` | 380 passed, 0 failed (379 baseline + 1 simd_probe) |
| 4 | `cargo clippy --workspace --all-targets --all-features -- -D warnings` + `--target wasm32-unknown-unknown -p vane-wasm` | clean |
| 5 | `cargo fmt --all -- --check` | clean |
| 6 | `bash scripts/check-no-std-fs.sh`（覆盖 vane-wasm） | OK |
| 7 | `cargo deny check` | advisories ok, bans ok, licenses ok, sources ok |
| 8 | 体积门禁：vane-wasm default 340KB / vane-core --export-all 622KB | ≤800KB PASS |
| 9 | JS 行为测试（wasm-bindgen-test, node） | 4 passed, 0 failed |
| 10 | `simd_probe::simd128_supported()` 占位返 false | PASS |

## 10. 遗留 / 疑问

- **vane-core 依赖 web-time**：为消解 M0 已知 panic 遗留（`Instant::now()`/`SystemTime::now()` 在 wasm32 panic），vane-core 增 `web-time = "1"` 依赖。native 零开销（re-export std::time），wasm32 经 js-sys 调 `performance.now()`/`Date.now()`。vane-core 代码零 `cfg(target)` 分支（I-5 守护不变）。vane-core --export-all 从 491KB → 622KB（js-sys 增量），但真实 deliverable（vane-wasm default）仅 340KB。
- **wasm-bindgen-test runner**：本地需 `cargo install wasm-bindgen-cli --version 0.2.127` 安装 `wasm-bindgen-test-runner`。CI 需在 wasm32-size 或独立 job 增安装步骤（本模块未加 CI wasm-bindgen-test job——可后续模块加）。
- **filter 支持**：与 vane-ffi 一致，wasm 绑定层暂不支持 filter（返 InvalidArg）。M2 后续模块接入。
