# M2-08 stored.bin zstd + per-file format_version — 报告

## 1. 逐项改动

### 1.1 per-file format_version 常量
- `crates/vane-core/src/types.rs:14-29`：保留 `FORMAT_VERSION` 作全库 schema 版本；新增 `HEADER_FORMAT_V1`/`VECTORS_FORMAT_V1`/`VECTORS_FORMAT_V2=2`/`STORED_FORMAT_V1`/`STORED_FORMAT_V2=2`/`IDMAP_FORMAT_V1`/`SCALARS_FORMAT_V1`/`HNSW_FORMAT_V1`。
- `types.rs:396-410`：新增 `per_file_format_versions_independent` 测试（测试 1）。

### 1.2 段文件编解码点改用 per-file 常量
- `segment/header.rs:2,21,46`：`FORMAT_VERSION` → `HEADER_FORMAT_V1`。
- `segment/mod.rs` finalize：
  - vectors.bin 写 v2 头（`VECTORS_FORMAT_V2` + `dim` 字段，12 字节头）。
  - stored.bin 经 `encode_stored` 写 v1/v2（feature 分支）。
  - idmap.bin → `IDMAP_FORMAT_V1`。
  - scalars.col → `SCALARS_FORMAT_V1`。
- `segment/mod.rs` `load_vectors`：字面量 `2u32`/`FORMAT_VERSION` → `VECTORS_FORMAT_V1`/`VECTORS_FORMAT_V2`。
- `segment/mod.rs` open 期头校验：vectors.bin 接受 v1/v2；stored.bin 接受 v1/v2。
- `segment/mod.rs` `decode_kv_map`：`FORMAT_VERSION` → `IDMAP_FORMAT_V1`。
- `segment/mod.rs` `decode_scalars`：`FORMAT_VERSION` → `SCALARS_FORMAT_V1`。
- `hnsw/mod.rs:23,461,534`：`FORMAT_VERSION` → `HNSW_FORMAT_V1`。
- inverted.bin 保留 `FORMAT_VERSION`（spec 未要求 per-file 化，属 schema 级版本）。

### 1.3 vectors.bin v2 头（M2-07 回归交接）
- finalize 始终写 v2（`magic|version=2|dim(4 LE)|payload`，12 字节头），无 feature 门。
- `load_vectors`/open 头校验用 `VECTORS_FORMAT_V2` 常量替代字面量 `2u32`。
- M2-07 `build_v2_stub_segment` → `build_v2_segment`：切到真实 finalize v2 产物。
- `build_v1_segment` 改为手工构造 v1 段（finalize 现写 v2，v1 须手工模拟旧 corpus）。

### 1.4 stored.bin v2 zstd
- `segment/mod.rs` `encode_stored(raw_payload)`：
  - `#[cfg(feature = "zstd-encode")]`：写 v2 `magic|version=2|raw_payload_len|zstd_block_len|zstd_block`（zstd level 3）。
  - `#[cfg(not(feature = "zstd-encode"))]`：写 v1 `magic|version=1|raw_payload`（wasm 裸 JSON）。
- `decode_stored`：v1/v2 双模分支；v2 经 `decode_stored_v2` ruzstd 解压 → raw_payload → `parse_stored_entries`（v1/v2 共享 entries 解析）。
- `decode_stored_v2`：`#[cfg(feature = "zstd-decode")]` 用 ruzstd；`#[cfg(not)]` 返 `Err(Unsupported)`。

### 1.5 feature 设计
- `crates/vane-core/Cargo.toml`：
  - `zstd = { version = "0.13", optional = true }`（C 库编码器，native/node）。
  - `ruzstd = { version = "0.5", optional = true }`（纯 Rust 解码器，wasm 也可用）。
  - `zstd-decode = ["dep:ruzstd"]`（读期解码）。
  - `zstd-encode = ["dep:zstd", "zstd-decode"]`（写期编码，隐含 zstd-decode 保证 roundtrip）。
  - `jieba = ["zstd-decode"]`（原 `jieba=["ruzstd"]` 解耦为 zstd-decode）。
- `vane-ffi/Cargo.toml`：启用 `zstd-encode`（native 写 v2）。
- `vane-node/Cargo.toml`：启用 `zstd-encode`（+ dict-zh）。
- `vane-wasm/Cargo.toml`：启用 `zstd-decode`（读 v2），不启 zstd-encode（C 库不进 wasm）。

### 1.6 测试
- `segment/tests.rs`：新增 `m2_08_stored_v2_zstd_roundtrip`、`m2_08_stored_v1_read_compat`、`m2_08_stored_v1_written_without_zstd_encode`、`m2_08_vectors_v2_header_contains_dim`、`m2_08_vectors_v1_read_compat_dim_fallback`、`m2_08_zstd_encode_feature_writes_v2_stored`。
- `tests/corpus_compat.rs`：`corpus_segment_files_have_magic_version_headers` 改为 per-file version 集合校验 + stored v1/v2 body 分支；新增 `corpus_format_compat_v2_roundtrip`（zstd-encode 写 v2 → close → open → search 基线一致）。

## 2. M2-07 回归交接落实
- `load_vectors`/`dim`/open 头校验中字面量 `2u32` 全部替换为 `VECTORS_FORMAT_V2`。
- `build_v2_stub_segment`（手工 stub）→ `build_v2_segment`（真实 finalize v2 产物）。
- 截断 v2 头测试（`m2_07_open_rejects_truncated_v2_header`）保留手工构造（无法经 finalize 产截断头），字面量替换为常量。
- M2-07 全部懒加载测试（v1 回退 + v2 dim 预存）回归全绿。

## 3. 自证门禁结果

| # | 门禁 | 结果 |
|---|------|------|
| 1 | `cargo test --workspace --all-features` | ok（277 lib + 全套绿，0 failed） |
| 2 | `cargo test -p vane-core --features jieba` | ok（276 passed） |
| 3 | `cargo test -p vane-core --features zstd-encode` | ok（261 passed） |
| 4 | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | clean |
| 5 | `cargo fmt --all -- --check` | clean |
| 6 | `cargo check --target wasm32-unknown-unknown -p vane-core` | ok（不启 zstd-encode） |
| 7 | `cargo check --target wasm32-unknown-unknown -p vane-wasm` | ok（启 zstd-decode） |
| 7b | `cargo check --target wasm32-unknown-unknown -p vane-core --features zstd-decode` | ok（ruzstd 进 wasm） |
| 8 | `bash scripts/check-no-std-fs.sh` | OK |
| 9 | `cargo deny check` | advisories ok, bans ok, licenses ok, sources ok |
| 10 | wasm 体积 | vane_wasm.wasm gzip = 11,698 bytes (11.4 KB) ≤ 800KB |
| 11 | corpus 兼容 | v1 roundtrip 绿 + v2 roundtrip（zstd-encode）绿 |
| 12 | M2-07 回归 | 全绿（字面量→常量，stub→真实 finalize） |
| I-5 | `grep cfg(target)` segment/types | 无命中；`cfg(feature)` zstd-encode/zstd-decode 允许 |
| 隔离 | `--no-default-features`（v1 写）/ `--no-default-features --features zstd-encode`（v2 写） | 两配置全绿 |

## 4. 体积实测
- `target/wasm32-unknown-unknown/release/vane_wasm.wasm`：35,928 bytes 原始，gzip 11,698 bytes (11.4 KB)。
- ruzstd 已编译进 wasm（zstd-decode feature），但 vane-wasm 当前为 Phase Zero 占位（仅 `vane_version()` 导出），LTO + 死代码消除剥离未引用的 decode 路径，故体积远低于预估 +30~60KB。
- 后续 M2 浏览器模块接入真实检索 API 后，decode 路径被引用，体积会上升——届时重测，预期仍远低于 800KB。

## 5. corpus 兼容结果
- v1 既有 corpus（`build_v1_segment` 手工构造 v1 段）→ 新版本 open + search 正常（`m2_08_stored_v1_read_compat`、`m2_08_vectors_v1_read_compat_dim_fallback`、`corpus_format_compat_roundtrip` 绿）。
- v2 roundtrip（`corpus_format_compat_v2_roundtrip`，zstd-encode）：写 v2 → close → open → 三模式 search 基线一致；段文件头校验 vectors v2 + stored v2。

## 6. 遗留/疑问
- **zstd-encode 隐含 zstd-decode**：spec Cargo.toml 未显式声明此依赖关系，本模块加（`zstd-encode = ["dep:zstd", "zstd-decode"]`）以保证写 v2 的配置也能读 v2（roundtrip 自洽）。若编排者认为应拆分（写不隐含读），可调整——但 vane-ffi/vane-node 写 v2 后必须读 v2，实务上必须隐含。
- **inverted.bin 未 per-file 化**：spec 仅列 6 个段文件（header/vectors/stored/idmap/scalars/hnsw），inverted.bin 保留 `FORMAT_VERSION`。若后续需 per-file 化可补 `INVERTED_FORMAT_V1`。
- **wasm 体积门禁**：当前 11.4KB 远低于 800KB，但因 vane-wasm 占位未引用 decode 路径。M2 浏览器模块接入后须重测。
