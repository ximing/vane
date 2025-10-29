# 07 SHA-256 严格性修复报告

> 基线：07-review 发现 `gen_dict.rs::compute_sha256_prefix` 用 SipHash（`DefaultHasher`）而非真 SHA-256，违反 SPEC §5.2 字面契约 + §12.3 三渠道一致性。
> 修复范围：仅 SHA-256 算法 + 重生成 dict.bin + 一致性测试，不扩展功能。

## 修复改动

### 1. `crates/vane-dict-zh/examples/gen_dict.rs`

- **删除**旧 `compute_sha256_prefix(words, hmm_blob)`：用 `std::collections::hash_map::DefaultHasher`（SipHash 1-3）对 words+hmm 做 Hash，取 `finish().to_le_bytes()`——非真 SHA-256。
- **新增** `compute_sha256_prefix(dict_bin_uncompressed: &[u8]) -> [u8; 8]`：用 `sha2::Sha256` 对解压后 dict.bin 的 `[16..]`（即去掉头部 16 字节 `magic(4)+format_version(4)+sha256_prefix(8)` 后的 payload）做 SHA-256，取前 8 字节。
- **SHA-256 输入范围定义**（SPEC §5.2 明确化）：payload = 解压后 dict.bin `[16..]`。三渠道（Node/Go/WASM）独立校验路径：解压 dict.bin → 跳过前 16 字节 → `crypto/sha256` / `SubtleCrypto.digest("SHA-256")` → 取前 8 字节比对。
- **main 流程调整**：先用占位 `[0u8;8]` 序列化 uncompressed dict.bin → 算 payload SHA-256 → 写回 `uncompressed[8..16]` → zstd 压缩。`build_full` / `build_small` 返回类型去掉 sha256_prefix 元组项。
- core `dict.rs` **未改**：`JiebaDict::sha256_prefix()` 仅返回存储值，不计算——只需 gen_dict 改算法 + 重生成产物。

### 2. `crates/vane-dict-zh/Cargo.toml`

- `[dev-dependencies]` 新增 `sha2 = { workspace = true }` + `ruzstd = "0.5"`（与 vane-core 同版本）。
- sha2 仅 dev-dep（gen_dict example + 测试用），**不进 core 运行时/wasm**——core 禁 std::fs、零 cfg 红线保持。

### 3. `crates/vane-dict-zh/tests/dict_test.rs`

- 新增测试 `sha256_prefix_is_real_sha256_of_payload`：
  - 用 `ruzstd` 解压 `DICT_BIN` 得原始字节。
  - 取 `[16..]` payload，`sha2::Sha256::digest` 取前 8 字节。
  - 断言 == `sha256_prefix()`（include_bytes 暴露值）。
  - 同时断言 dict.bin 头部 `[8..16]` 内嵌值 == `sha256_prefix()`。
- 覆盖 SPEC §5.2 字面契约 + §12.3 三渠道一致性（Go/WASM 独立计算须匹配）。

### 4. 重新生成产物

- 重跑 `cargo run --release -p vane-dict-zh --example gen_dict -- --full`。
- `data/dict.bin` + `data/sha256_prefix.bin` 已更新（新 sha256_prefix 写入头 + 独立文件）。

## 新 sha256_prefix 值

```
ae 2d 12 30 49 c4 bc b4
```

（旧值 `98 72 36 6d 4b f5 b5 c1` 为 SipHash 产物，已废弃。）

## 体积实测

| 指标 | 旧值 | 新值 | 门禁 |
|------|------|------|------|
| dict.bin (zstd) | 1,479,454 B | 1,479,454 B | — |
| dict.bin gzip -9 | 1,477,877 B | 1,477,876 B | ≤ 1,500,000 ✅ |
| 余量 | 22,123 B (1.48%) | 22,124 B (1.48%) | — |

体积无变化（SHA-256 改动不影响 payload 字节，仅替换头部 8 字节指纹）。

## 冷加载实测

```
dict_load  time:   [29.491 ms 30.006 ms 30.644 ms]
```

30.0ms < 150ms ✅（SPEC §13.1）。与修复前 29.7ms 相比无回归（p=0.33 > 0.05，criterion 判定 No change in performance detected）。

## 自证门禁（全绿）

| 门禁 | 结果 |
|------|------|
| `cargo test --workspace --all-features` | 245 passed, 0 failed, 1 ignored ✅ |
| `cargo test -p vane-dict-zh` | 9 passed (含新 SHA-256 一致性测试) ✅ |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 无 warning ✅ |
| `cargo fmt --all -- --check` | 干净 ✅ |
| dict.bin gzip ≤ 1.5MB | 1,477,876 B ✅ |
| `cargo bench -p vane-dict-zh --bench dict_load --no-run` | 编译通过 ✅ |
| 冷加载 bench 实测 | 30.0ms < 150ms ✅ |

## 提交

- commit: `fix(dict-zh): use real SHA-256 for dict.bin sha256_prefix (SPEC §5.2)`
- hash: `5b28dcc`
- 文件：`crates/vane-dict-zh/Cargo.toml`、`crates/vane-dict-zh/examples/gen_dict.rs`、`crates/vane-dict-zh/tests/dict_test.rs`、`crates/vane-dict-zh/data/dict.bin`、`crates/vane-dict-zh/data/sha256_prefix.bin`、`Cargo.lock`。
