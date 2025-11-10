# M2-08 stored.bin zstd + per-file format_version

## 1. 目标
消解 SPEC §6.2 stored.bin zstd 承诺与 M1 裸 JSON 实现的张力：引入 per-file format_version 常量（替代全局 `FORMAT_VERSION` 单常量），stored.bin v2 zstd 块压缩 + v1/v2 双模读取（不做原地迁移），zstd-encode feature（native/node 启用，wasm 不启）+ ruzstd 解码（wasm 也启，支持跨平台读 v2）（SPEC v1.2 修订 B + I-5 释义澄清已批准）。

SPEC 节号：§6.2（per-file format_version + stored v1/v2 双模 + 懒加载注释，v1.2 已修订 lines 211-216）、§14 I-5 注（v1.2 已加，cfg(feature) 允许 segment 编解码）。

## 2. 涉及文件
- **Modify** `crates/vane-core/src/types.rs:15`（`pub const FORMAT_VERSION: u32 = 1;`）：保留作全库 schema 版本；新增 per-file 常量：
  ```rust
  pub const HEADER_FORMAT_V1: u32 = 1;
  pub const VECTORS_FORMAT_V1: u32 = 1;
  pub const VECTORS_FORMAT_V2: u32 = 2;   // +dim(4 LE) 头字段（M2-07 dim 来源）
  pub const STORED_FORMAT_V1: u32 = 1;
  pub const STORED_FORMAT_V2: u32 = 2;
  pub const IDMAP_FORMAT_V1: u32 = 1;
  pub const SCALARS_FORMAT_V1: u32 = 1;
  pub const HNSW_FORMAT_V1: u32 = 1;
  ```
- **Modify** `crates/vane-core/Cargo.toml`：
  ```toml
  zstd = { version = "0.13", optional = true }       # 写期编码（native/node）
  ruzstd = { version = "0.5", optional = true }       # 读期解码（wasm 也启）
  [features]
  zstd-encode = ["dep:zstd"]
  zstd-decode = ["dep:ruzstd"]
  jieba = ["zstd-decode"]    # jieba 复用 ruzstd 解码 dict.bin（原 jieba=["ruzstd"] 调整）
  ```
  **wasm32 check 不启 zstd-encode**（zstd-sys C 库不进 wasm）；**wasm32 check 启 zstd-decode**（ruzstd 纯 Rust）。
- **Modify** `crates/vane-core/src/segment/mod.rs:212-228`（finalize 写 stored.bin）：`#[cfg(feature="zstd-encode")]` 走 v2 zstd 块；`#[cfg(not(feature="zstd-encode"))]` 落 v1 裸 JSON。
- **Modify** `crates/vane-core/src/segment/mod.rs:509-562`（`decode_stored`）：按 version 分支——v1 原路径；v2 读 `raw_payload_len` + `zstd_block_len` + `zstd_block` → ruzstd 解压 → 对 raw_payload 走 v1 解码逻辑。
- **Modify** `crates/vane-core/src/segment/header.rs:21,46`：`FORMAT_VERSION` → `HEADER_FORMAT_V1`（行 21 encode `to_le_bytes`；行 46 decode `version != FORMAT_VERSION` 校验，行 40 是 `buf.len() < 8` 长度检查）。
- **Modify** `crates/vane-core/src/segment/mod.rs`：所有段文件编解码点改用 per-file 常量（vectors.bin 用 `VECTORS_FORMAT_V1`/`V2`、idmap 用 `IDMAP_FORMAT_V1`、scalars 用 `SCALARS_FORMAT_V1`、hnsw 用 `HNSW_FORMAT_V1`）。
- **Modify** `crates/vane-core/src/hnsw/mod.rs:534`：`FORMAT_VERSION` → `HNSW_FORMAT_V1`（行 461 encode、行 534 decode 校验）。
- **Modify** `crates/vane-core/src/segment/mod.rs:652`（`fn decode_scalars`，定义在 `segment/mod.rs` 而非 `scalars.rs`——`segment/` 目录无 `scalars.rs`，`ScalarReader` 在 `mod.rs:583`、`decode_scalars` 在 `mod.rs:652`）：`FORMAT_VERSION` → `SCALARS_FORMAT_V1`。
- **Modify** `crates/vane-core/tests/corpus_compat.rs:221-280`：既有 v1 roundtrip 保留；新增 v2 用例。
- **Modify** `crates/vane-ffi/Cargo.toml` + `crates/vane-node/Cargo.toml`：启用 `vane-core` 的 `zstd-encode` feature（native/node 写 v2）。
- **Modify** `crates/vane-wasm/Cargo.toml`：vane-wasm 启用 `vane-core` 的 `zstd-decode` feature（wasm 读 v2）；**不启 zstd-encode**。

## 3. 接口契约
### Consumes from
- M0 `types::{MAGIC, FORMAT_VERSION}`（保留）、`segment::decode_stored`/`finalize`、`header::encode_header`/`decode_header`。
- M1 `corpus_compat` 测试套件。
- M2-07 协同（vectors.bin v2 头含 dim）：**stub-then-regress 策略**——M2-07 与 M2-08 同批 L0 推进时，M2-07 测试用 stub v2 header（手工构造 `magic|version=2|dim(4 LE)|payload` 12 字节头）验证读 dim 逻辑；M2-08 落实 `finalize` 写 v2 头 + `VECTORS_FORMAT_V2` 常量后，M2-07 回归测试切到真实 v2 产物。两计划 dim 读/写版本对齐：v2 头固定 12 字节（magic4+version4+dim4 LE），v1 头 8 字节（magic4+version4），version 字段是判别位。M2-08 写 v2 时 dim 字段必填 `schema.dim`；M2-07 读 v2 时直接取 dim 字段，读 v1 回退 `payload_len/doc_count/4`。

### Produces for
- per-file format_version 常量（`types.rs`）。
- stored.bin v2 zstd 块格式（README M2-08 节布局）。
- `zstd-encode`/`zstd-decode` feature。
- 下游 M2-12（export 快照含 v2 段）、M2-13（维基 corpus v2 兼容）。

## 4. TDD 测试清单
1. **per-file 常量独立性**：`HEADER_FORMAT_V1`/`STORED_FORMAT_V2` 等各自独立值（unit test 断言常量值）。
2. **stored v2 写（zstd-encode）**：`finalize`（feature=zstd-encode）写 stored.bin，头 `version=2`，`raw_payload_len`+`zstd_block_len`+`zstd_block` 布局正确。
3. **stored v2 读（zstd-decode）**：v2 stored.bin → `decode_stored` ruzstd 解压 → raw_payload 走 v1 解码 → HashMap 与原数据一致。
4. **stored v1 读兼容**：M0/M1 产物 v1 stored.bin（裸 JSON）→ 新 `decode_stored` 按 version=1 走原路径 → 一致。
5. **stored v1 写（无 zstd-encode）**：`finalize`（feature 无 zstd-encode，如 wasm）写 v1 裸 JSON，version=1。
6. **corpus 兼容 roundtrip v2**：写 v2（zstd-encode）→ close → open → search 基线一致（SPEC §13.3 冻结兼容）。
7. **corpus 兼容 v1 读**：用 M1 产物 v1 corpus → 新版本 open → 读回 stored 一致。
8. **wasm32 v2 解码编译**：`cargo check --target wasm32-unknown-unknown -p vane-core --features zstd-decode` 通过（ruzstd 进 wasm）。
9. **wasm32 不启 zstd-encode**：`cargo check --target wasm32-unknown-unknown -p vane-core` 不启 zstd-encode（zstd-sys C 库不进 wasm）。
10. **wasm32 体积**：ruzstd 进 wasm 后核心 wasm gzip ≤800KB（实测登记，预估 +30~60KB）。
11. **vectors.bin v2 头含 dim**：`finalize` 写 vectors.bin v2（`magic|version=2|dim(4 LE)|payload`），M2-07 open 读 dim 正确。
12. **vectors.bin v1 读兼容**：M0/M1 产物 v1 vectors.bin → open 读 dim 回退 `payload_len/doc_count/4`（M2-07 协同）。
13. **I-5 守护**：`grep -rn 'cfg(target' crates/vane-core/src/segment/ crates/vane-core/src/types.rs` 无 `cfg(target)`；`cfg(feature="zstd-encode")` 允许（SPEC v1.2 I-5 注）。
14. **header/idmap/scalars/hnsw per-file 化回归**：所有段文件编解码改用 per-file 常量后，既有 corpus 兼容测试全绿。
15. **zstd-encode feature 隔离**：`cargo test --no-default-features --features zstd-encode` 写 v2；`cargo test --no-default-features`（无 zstd-encode）写 v1。

## 5. 验收标准
- stored v2 zstd 写 + 读 roundtrip 一致。
- stored v1 双模读取兼容（M0/M1 corpus 不破）。
- per-file 常量替换全局 `FORMAT_VERSION`，所有段文件编解码点改用 per-file，既有测试全绿。
- wasm32 check 启 zstd-decode 通过，不启 zstd-encode。
- 核心 wasm gzip ≤800KB（ruzstd 进 wasm 后实测）。
- I-5：segment 编解码处仅 `cfg(feature)`，无 `cfg(target)`。
- corpus_compat v1+v2 双用例绿。

## 6. 前置依赖
- SPEC v1.2 修订 B + I-5 释义澄清（已批准）。
- M2-07 协同（vectors.bin v2 dim 字段；可分别落地，但 v2 头写入与读取需一致）。

## 7. 不变量覆盖
- **I-1 段不可变**：stored v2 仍 finalize 一次性写入（懒加载/双模不写回段文件）。测试 6 守护。
- **I-5 核心零平台分支**：`cfg(feature="zstd-encode")` 在 segment 编解码（SPEC v1.2 I-5 注允许）；`cfg(target)` 仍仅限 VFS/Executor。测试 13 守护。
- **§6.2 per-file format_version**：每文件独立递增，替代全局常量。测试 1+14 守护。
- **§13.3 corpus 兼容**：v1/v2 双模读取 + 冻结兼容测试。测试 6+7 守护。
- **体积门禁**：ruzstd 进 wasm ≤800KB。测试 10 守护。
