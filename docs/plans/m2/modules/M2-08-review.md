# M2-08 stored.bin zstd + per-file format_version — 评审报告

**评审对象**：M2-08（per-file format_version 常量 + vectors.bin v2 头 + stored.bin v2 zstd + v1/v2 双模读取 + zstd-encode/zstd-decode feature + M2-07 回归交接）
**评审人**：task reviewer（只读）
**基线**：BASE 6db2d45..HEAD b91f28f（仅 crates/ + docs/）
**日期**：2026-08-09
**结论**：**PASS_WITH_FINDINGS**（0 B / 1 I / 3 M）

---

## 1. 评审重点逐项核查

### 1.1 per-file 常量重构完整性 — PASS
6 个 spec 列段文件全部改用 per-file 常量，grep 核查无残留误用：

| 段文件 | 编码点 | 解码点 | 常量 |
|--------|--------|--------|------|
| header.bin | `header.rs:21` | `header.rs:46` | `HEADER_FORMAT_V1` |
| vectors.bin | `segment/mod.rs:205` | `segment/mod.rs:388-403, 456-463` | `VECTORS_FORMAT_V1`/`V2` |
| stored.bin | `segment/mod.rs:640,651` | `segment/mod.rs:672,679` | `STORED_FORMAT_V1`/`V2` |
| idmap.bin | `segment/mod.rs:242` | `segment/mod.rs:597` | `IDMAP_FORMAT_V1` |
| scalars.col | `segment/mod.rs:261` | `segment/mod.rs:876` | `SCALARS_FORMAT_V1` |
| hnsw.bin | `hnsw/mod.rs:461` | `hnsw/mod.rs:534` | `HNSW_FORMAT_V1` |

残留 `FORMAT_VERSION` 使用点（均合规）：
- `types.rs:18` 定义 + `:392` 既有常量测试（保留作全库 schema 版本，spec 明确）。
- `bm25.rs:6,236,354,357` —— inverted.bin 版本字段（见 §1.9）。
- `tokenizer/jieba/dict.rs:27` —— 模块内**私有** `const FORMAT_VERSION: u32 = 1`，与全局常量同名但独立，属 dict.bin 自身版本（见 M-2）。

### 1.2 vectors.bin v2 头 — PASS
- `segment/mod.rs:204-211` finalize 写 `MAGIC | VECTORS_FORMAT_V2 | self.dim(4 LE) | payload`，12 字节头，无 feature 门。
- `self.dim` 取自 `schema.vector_field().map(|(_, d, _)| d).unwrap_or(0)`（`segment/mod.rs:64`），即 schema.dim。✓
- `load_vectors`（`segment/mod.rs:454-463`）字面量 `2u32`/`FORMAT_VERSION` 全部替换为 `VECTORS_FORMAT_V1`/`VECTORS_FORMAT_V2`，payload_off v1=8/v2=12。✓
- M2-07 回归交接落实：`build_v2_stub_segment` → `build_v2_segment`（`tests.rs:786-808`，切真实 finalize 产物）；`build_v1_segment` 改手工构造 v1 段（`tests.rs:693-784`）；截断 v2 头测试（`tests.rs:843-878`）保留手工构造并替换字面量为 `VECTORS_FORMAT_V2`。✓

### 1.3 stored.bin v2 zstd — PASS（附 I-1）
- `encode_stored`（`segment/mod.rs:633-653`）：`#[cfg(feature="zstd-encode")]` 写 v2 `magic|version=2|raw_payload_len(4 LE)|zstd_block_len(4 LE)|zstd_block`；`#[cfg(not)]` 写 v1 `magic|version=1|raw_payload`。raw_payload 为 v1 body（count+entries），v2 仅外包 zstd 压缩 + 长度前缀。✓
- `decode_stored`（`segment/mod.rs:663-686`）：按 version 分支，v1 走 `parse_stored_entries`，v2 走 `decode_stored_v2`。
- `decode_stored_v2`（`segment/mod.rs:694-727`）：ruzstd `StreamingDecoder` 解压 → 校验 `owned.len() == raw_len` → 复用 `parse_stored_entries`。✓
- `parse_stored_entries`（`segment/mod.rs:735-...`）v1/v2 共享 body 解析。✓
- 不做原地迁移（旧 v1 段只读服务）。✓
- native/node 写 v2（vane-ffi/vane-node 启 zstd-encode）、wasm 写 v1 但读 v2（vane-wasm 启 zstd-decode）。✓

**I-1 见 §2**。

### 1.4 feature 设计 — PASS
- `Cargo.toml`：`zstd = {0.13, optional}`、`ruzstd = {0.5, optional}`。
- `zstd-decode = ["dep:ruzstd"]`；`zstd-encode = ["dep:zstd", "zstd-decode"]`（隐含 zstd-decode 保证 roundtrip，合理）；`jieba = ["zstd-decode"]`（原 `jieba=["ruzstd"]` 解耦）。
- ruzstd 从 jieba 解耦：`dict.rs:53` 确实用 ruzstd 解压 dict.bin，但该路径在 `jieba` feature 下；vane-wasm 启 `zstd-decode` 不启 `jieba`，ruzstd 通过 `zstd-decode` feature 引入，路径清晰（`vane-wasm/Cargo.toml` → `vane-core` features=["zstd-decode"] → `dep:ruzstd`）。✓
- zstd-encode 隐含 zstd-decode：写 v2 的配置也必须能读 v2，实务必须。✓

### 1.5 不变量 — PASS
- **I-1 段不可变**：stored v2 仍 `finalize` 一次性写入（`segment/mod.rs:195-252`），懒加载/双模不写回段文件。`corpus_format_compat_v2_roundtrip` 测试守护。✓
- **I-5 核心零平台分支**：`grep -rn 'cfg(target' crates/vane-core/src/segment/ crates/vane-core/src/types.rs` 无命中。`cfg(feature="zstd-encode")`/`cfg(feature="zstd-decode")` 在 segment 编解码处（SPEC v1.2 I-5 注允许）。✓
- **I-6 corpus 兼容**：v1 旧段双模读（`m2_08_stored_v1_read_compat`、`m2_08_vectors_v1_read_compat_dim_fallback`）+ v2 roundtrip（`corpus_format_compat_v2_roundtrip`）。✓

### 1.6 800KB 门禁 caveat — PASS（carry-forward）
报告 §4 + §6 遗留已标注：wasm 11.4KB 因 vane-wasm Phase Zero 占位（仅 `vane_version()` 导出），LTO + 死代码消除剥离未引用的 decode 路径，故体积远低于预估。后续 M2 浏览器模块接入真实检索 API 后 decode 路径被引用，体积会上升——届时重测。M2-08 本身不引浏览器 API，不阻塞。✓

### 1.7 签名冻结 — PASS
per-file 常量是新增 `pub const`（`types.rs:24-35`），不改既有 pub API 签名。`finalize`/`open`/`SegmentReader` pub 签名不变，内部行为变（v2 写入、双模读）。✓

### 1.8 TDD 覆盖 — PASS
15 项测试覆盖（见计划 §4）。逐项核查 diff 中测试落地：
- 测试 1 per-file 独立性：`per_file_format_versions_independent`（`types.rs:396-410`）✓
- 测试 2+3 v2 zstd roundtrip：`m2_08_stored_v2_zstd_roundtrip`（`tests.rs:907-966`）✓
- 测试 4 v1 读兼容：`m2_08_stored_v1_read_compat`（`tests.rs:971-1026`）✓
- 测试 5 v1 写无 zstd-encode：`m2_08_stored_v1_written_without_zstd_encode`（`tests.rs:1031-1067`）✓
- 测试 6 corpus v2 roundtrip：`corpus_format_compat_v2_roundtrip`（`corpus_compat.rs:1292-1383`）✓
- 测试 7 corpus v1 读：既有 `corpus_format_compat_roundtrip` + `m2_08_*_v1_read_compat` ✓
- 测试 11 vectors v2 头含 dim：`m2_08_vectors_v2_header_contains_dim`（`tests.rs:1071-1078`）✓
- 测试 12 vectors v1 读兼容：`m2_08_vectors_v1_read_compat_dim_fallback`（`tests.rs:1082-1088`）✓
- 测试 14 per-file 化回归：`corpus_segment_files_have_magic_version_headers` 改为 per-file version 集合校验（`corpus_compat.rs:1208-1214`）✓
- 测试 15 feature 隔离：`m2_08_zstd_encode_feature_writes_v2_stored`（zstd-encode）+ `m2_08_stored_v1_written_without_zstd_encode`（无 zstd-encode）✓

⚠️ 测试 8/9（wasm32 cargo check）与测试 10（wasm 体积）属编译/构建门禁，无法从 diff 确认实际运行结果，报告 §3 门禁表自证通过（门禁 6/7/7b/10）。测试 13 I-5 守护为 grep 检查（非自动化测试），reviewer 独立 grep 复核无命中。可接受。

### 1.9 inverted.bin 保留 FORMAT_VERSION — PASS（可接受）
`bm25.rs:236,354` 用全局 `FORMAT_VERSION` 作 inverted.bin 版本字段。spec §6.2 仅列 6 段文件 per-file（header/vectors/stored/idmap/scalars/hnsw），inverted 未涉。inverted.bin 由 bm25 模块自管，保留全局 schema 版本作其版本字段，语义自洽（inverted 属 schema 级格式，随全库版本演进）。**判断：可接受**，不阻塞 M2-08。后续若需 per-file 化可补 `INVERTED_FORMAT_V1`（见 M-1）。

### 1.10 依赖 — PASS
- `deny.toml` 黑名单：regex/tokio/prost/tonic/openssl/lindera/ndarray/wee_alloc/dashmap/parking_lot。`zstd`/`ruzstd`/`zstd-sys` 均不在黑名单。✓
- zstd crate 0.13 → zstd-sys（C 库），仅 zstd-encode feature 引入；vane-wasm 不启 zstd-encode，zstd-sys 不进 wasm。✓
- ruzstd 0.5 纯 Rust，wasm32 可用。✓
- ⚠️ cargo-deny 实际输出无法从 diff 确认，报告 §3 门禁 9 自证 advisories/bans/licenses/sources ok。

---

## 2. 发现分级

### I-1（Intermediate）：`encode_stored` zstd 压缩失败回退路径产生损坏的 v2 文件

**证据**：`crates/vane-core/src/segment/mod.rs:636`
```rust
let zstd_block = zstd::encode_all(raw_payload, 3).unwrap_or_else(|_| raw_payload.to_vec());
```

**问题**：`zstd::encode_all` 失败时，回退将 `zstd_block` 设为**未压缩的** `raw_payload` 字节，但后续仍写 `version=2` + `zstd_block_len = raw_payload.len()`（`segment/mod.rs:640-644`）。`decode_stored_v2`（`segment/mod.rs:694-727`）用 ruzstd `StreamingDecoder` 解压该"zstd_block"——因 raw_payload 非合法 zstd frame，解压失败 → `VaneError::Corrupt("stored v2 zstd decompress failed")`。回退路径产生**不可读的 v2 文件**，违反 I-6 corpus 兼容精神。

**影响**：实务上 `zstd::encode_all` 内存操作极少失败（除非 OOM，那时整个进程已炸），故实际命中概率极低。但代码逻辑上回退路径是错的——回退到不可读格式比直接报错更糟。

**建议**：二选一——
1. 失败时回退写 **v1**（`version=1` + raw_payload，可读）；
2. 失败时返回 `Err(VaneError::Io(...))`，让 finalize 失败而非落损坏段。

不阻塞 M2-08（命中概率极低 + 仅压缩失败时触发），但应在 M2 后续阶段修复。

### M-1（Minor 建议）：inverted.bin 后续可 per-file 化
**证据**：`crates/vane-core/src/bm25.rs:236,354` 用全局 `FORMAT_VERSION`。
spec 仅列 6 段文件 per-file，inverted 由 bm25 模块自管，当前可接受。后续若 inverted 格式演进，建议补 `INVERTED_FORMAT_V1` 常量与 per-file 化，与全库 schema 版本解耦。非本模块职责。

### M-2（Minor 建议）：dict.rs 私有 `FORMAT_VERSION` 同名易混
**证据**：`crates/vane-core/src/tokenizer/jieba/dict.rs:27` `const FORMAT_VERSION: u32 = 1;`（模块内私有，dict.bin 自身版本）。
与全局 `crate::types::FORMAT_VERSION` 同名但语义不同。虽语法合规（私有不污染外部），但易在维护时混淆。建议改名 `DICT_FORMAT_V1`。非本模块引入（既有代码），不阻塞。

### M-3（Minor 建议）：`decode_stored` v2 头长度阈值与 `decode_stored_v2` 一致性
**证据**：`decode_stored`（`segment/mod.rs:665`）`buf.len() < 8` 早返空 HashMap；`decode_stored_v2`（`:696`）`buf.len() < 16` 返 Corrupt。
v2 段至少 16 字节头。若 v2 buf 恰为 8-15 字节（截断），`decode_stored` 不早返（≥8），进入 `decode_stored_v2` → 返回 Corrupt（正确行为）。逻辑正确，仅建议 `decode_stored` 注释说明 v2 最小 16 字节以提升可读性。非阻塞。

---

## 3. 无法从 diff 确认项（交编排者）

- **cargo check wasm32**（测试 8/9）：报告 §3 门禁 6/7/7b 自证通过。reviewer 未跑 cargo，无法独立复核。属信任范围。
- **cargo-deny check**（测试依赖门禁 9）：报告 §3 门禁 9 自证 advisories/bans/licenses/sources ok。reviewer 已核查 `deny.toml` 黑名单不含 zstd/ruzstd，配置层面无阻断；实际输出未独立复核。
- **wasm 体积 11.4KB**：报告 §4 自证，caveat 已标注（占位 LTO 剥离 decode 路径，M2 浏览器模块接入后须重测）。carry-forward，不阻塞 M2-08。
- **`cargo test` 全绿**：报告 §3 门禁 1-5 自证 277 lib + 全套绿、clippy clean、fmt clean。reviewer 未跑 cargo，无法独立复核。

---

## 4. 总结

M2-08 实装与计划高度一致：per-file 常量重构完整（6 段文件全覆盖、无残留误用）、vectors.bin v2 头落实（dim 取自 schema.dim、M2-07 回归交接到位）、stored.bin v2 zstd 双模读设计清晰、feature 解耦合理（ruzstd 从 jieba 解耦、zstd-encode 隐含 zstd-decode）、不变量 I-1/I-5/I-6 守护到位、TDD 覆盖 15 项全面。唯一实质问题是 `encode_stored` 压缩失败回退路径产生不可读 v2 文件（I-1），命中概率极低但逻辑错误，建议后续修复。inverted.bin 保留 `FORMAT_VERSION` 可接受（spec 未涉，bm25 自管）。wasm 体积 caveat 已标注，carry-forward 不阻塞。

**状态**：PASS_WITH_FINDINGS
**B/I/M**：0 B / 1 I / 3 M
