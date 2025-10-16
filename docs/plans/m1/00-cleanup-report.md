# 阶段零-A：M0 段格式冻结清理 — 实现报告

> 实现者：cleanup SubAgent（opus）。需求来源：`docs/plans/m1/00-cleanup.md`。
> 上游契约：`docs/SPEC.md` v1.0 §6.2/§6.3/§13.3、`docs/plans/m0/README.md` Global Interface Contracts。
> 基线：HEAD 538db51（M0 完成，204 测试绿）。完成基线：5 个 phase 全部落地，自证门禁全绿。

## 提交 hash 列表

| 顺序 | hash | 说明 |
|---|---|---|
| Phase 1 | `5236257` | fix(segment): FF1 vectors.bin 加 magic+version 头合规 §6.2 |
| Phase 2 | `e329c53` | fix(segment): FF3 format_version 统一全 LE |
| Phase 3 | `348f946` | test(segment): FF2 断言 add_doc 返回局部 docid（SPEC §3.2） |
| Phase 4+5 | `37a895d` | test(corpus): §13.3 corpus 兼容测试骨架 + FF6 wasm32 体积门禁 deferred 注释 |
| 修复 | `d4dee8b` | fix(segment): clippy redundant_slicing 修复 |

> 注：Phase 1 的提交中误纳入了编排者预生成的未跟踪文件（`docs/plans/m1/*.md`、`scripts/__pycache__/*.pyc`）。Phase 2 提交中已将 `scripts/__pycache__` 取消跟踪并加入 `.gitignore`；计划文档保留（属 M1 工作产物）。

## 每 phase 实际改动

### Phase 1 · FF1：vectors.bin 加 magic+version 头（SPEC §6.2 合规）

**改动文件：**
- `crates/vane-core/src/segment/mod.rs`
  - `SegmentWriter::finalize`（约 104-117 行）：写 vectors.bin 时先写 `MAGIC` + `FORMAT_VERSION.to_le_bytes()`，再写 f32 LE payload。`vbytes` 容量从 `vectors.len()*4` 调整为 `8 + vectors.len()*4`。doc_count=0 时仍写 8 字节头（空段合规）。
  - `SegmentReader::open`（约 215-235 行）：读 vectors.bin 时校验 `len>=8` + `magic`，读出 `format_version`(LE) 并校验，跳过前 8 字节后再 `chunks_exact(4)` → f32。doc_count=0 时不读 vectors.bin（保持空 Vec）。
- `crates/vane-core/src/segment/tests.rs`：新增两个测试
  - `vectors_bin_has_magic_version_header`：读原始 vectors.bin 字节，断言 `[0..4]==b"VANE"`、`[4..8]==[1,0,0,0]`(LE)、`len==8+4*4`；reader 跳过头后 `vectors().len()==4`。
  - `vectors_bin_empty_segment_still_writes_header`：空段 vectors.bin 仍为 8 字节头，reader 读回 doc_count=0、vectors 空。

**裁决遵循：** FA1（magic LE + format_version LE，与 FF3 统一）。`vectors()` 仍返回纯 f32，brute_search 不受影响。既有 `segment_reader_roundtrip` 中 `reader.vectors().len()==8`（2 docs × 4 dim）断言无需改动（跳过头后仍成立）。

### Phase 2 · FF3：format_version 统一全 LE

**改动文件：**
- `crates/vane-core/src/segment/header.rs`
  - 文件头注释（4-10 行）：去掉"format_version 采用大端"说明，改为"FA2：全字段统一 LE（含 format_version）"。
  - `encode_header`（16 行）：`FORMAT_VERSION.to_be_bytes()` → `to_le_bytes()`。
  - `decode_header`（40 行）：`u32::from_be_bytes` → `from_le_bytes`。
- `crates/vane-core/src/segment/mod.rs`
  - `finalize` stored.bin（120 行）：`to_be_bytes()` → `to_le_bytes()`。
  - `finalize` idmap.bin（135 行）：`to_be_bytes()` → `to_le_bytes()`。
  - `finalize` scalars.col（151 行）：`to_be_bytes()` → `to_le_bytes()`。
  - `decode_kv_map`（294-310 行）：注释 `version(4 BE)` 改为 `version(4 LE)`；新增 magic 校验 + version 校验（不匹配返回 `E_VERSION`）。属简报允许的"FF4 严格化可接受轻量部分"。
- `crates/vane-core/src/segment/tests.rs:30`：断言由 BE `&[0, 0, 0, 1]` 改为 LE `&[1, 0, 0, 0]`。

**裁决遵循：** FA2。`header_roundtrip` 测试通过。decode_kv_map 此前只跳过 magic+version 不校验 version，现已加校验（轻量，未触及 FF4 的 dim 推导/stored 截断严格化，属排除项）。

### Phase 3 · FF2：add_doc 局部 docid 断言

**改动文件：**
- `crates/vane-core/src/segment/tests.rs` `segment_writer_docid_base_nonzero`（208-217 行）：
  - 第二段 base=2 时，断言 `w2.add_doc("c", ...)` 返回值 `local_id == 0`（局部 docid，非全局 2）。
  - 显式验证全局 docid = `m1.docid_base + m1.doc_count as u64 + local_id == 2`。

**裁决遵循：** 简报要求"在 `segment_writer_docid_base_nonzero` 测试中断言 add_doc 返回 0（局部），并显式验证全局 docid 概念 = base + 返回值"。已落地，文档化 SPEC §3.2 局部 docid 语义。

### Phase 4 · corpus 兼容测试骨架（SPEC §13.3）

**新建文件：** `crates/vane-core/tests/corpus_compat.rs`（含文件头文档化：此测试冻结 segment 格式；任何格式变更必须保持此测试通过，或 bump FORMAT_VERSION + 提供迁移器/双模读取）。

**两个测试：**
1. `corpus_format_compat_roundtrip`：
   - 用 `StdFsVfs`（唯一临时目录，AtomicU64 + pid + nanos 防并行冲突）建 DB。
   - 声明 collection（text `body` + vector `v` dim=4 Cosine + scalar `tag` Keyword）。
   - 灌入 5 篇中英混排文档（含中文"向量检索/机器学习/全文检索"等 + 英文"hybrid search/BM25 ranking"等）。
   - `add` → `flush` → 捕获三模式（Hybrid/Vector/Text）搜索基线 `(id, score, tag)` → `close`。
   - 重新 `open` 同目录：断言 manifest restore（`db.collections()` 含 "docs"）；三模式搜索结果与基线逐条比对（id 相等、score 绝对差 <1e-6、tag 相等）；验证 hybrid 命中非空、stored `tag` 回填正确（Keyword 经 JSON 序列化后回填为 `"\"a\""` 形态，已校验）。
2. `corpus_segment_files_have_magic_version_headers`：
   - 校验 `header.bin`/`vectors.bin`/`stored.bin`/`idmap.bin`/`scalars.col` 均含 `b"VANE"` magic + `[1,0,0,0]` format_version(LE) 头。

**CI 接入：** `.github/workflows/ci.yml` uncomment `corpus-compat` job（`needs: test`，`cargo test --test corpus_compat -p vane-core`）。

**裁决遵循：** FA3。M0 口径骨架（无历史 golden fixture，因 fresh repo 无发布产物）；建立契约门禁是目标。`inverted.bin` 由 `write_inverted` 写入，本测试未单独校验其头（inverted 格式属 BM25 模块，且 `corpus_format_compat_roundtrip` 的 text/hybrid 搜索隐式验证了其可读性）。

### Phase 5 · FF6：wasm32 体积门禁 deferred 注释

**改动文件：** `.github/workflows/ci.yml`（corpus-compat job 之后）。
- 新增注释化 deferred job `wasm32-size`（不实跑）：说明 SPEC §13.2-3 核心 wasm gzip ≤800KB（含 jieba 代码、不含词典）/ 全功能 ≤1.2MB / dict-zh ≤1.5MB / Go embed 增量 <2MB / 500KB 为 M2 优化目标非门禁；M1 jieba 落地起生效，届时补 `wasm-opt` + gzip size check（注释中给出命令骨架）。

**裁决遵循：** FA4。不实跑，纯文档化 deferred。

## 偏离与裁决

1. **`decode_kv_map` version 校验扩展**：简报 Phase 2 允许"顺手加 version 校验（不匹配返回 E_VERSION），属 FF4 严格化的可接受轻量部分"。同时加了 magic 校验（此前只跳过不校验）。未触及 FF4 的 dim 推导校验/stored 解码截断严格化（属排除项，留 M1）。裁决：在允许范围内，未越界。
2. **`vectors.bin` 读取增加 magic+version 校验**：FA1 只明确"跳过 8 字节头"，实现中顺手加了 magic/version 校验（与 header.bin/stored.bin/idmap.bin 的校验口径一致）。裁决：与 FF4 严格化方向一致且属同一文件头的合规校验，未引入新需求。
3. **stored `tag` 回填值为带引号 JSON 串**：`collection.rs::search` 中 `v.to_string()` 对 `serde_json::Value::String("a")` 产出 `"\"a\""`。测试断言按实际形态校验（`Some("\"a\"")`），未改业务代码。这是 M0 既有行为，不在本任务范围。
4. **Phase 1 提交误纳入预生成文件**：`docs/plans/m1/*.md` 与 `scripts/__pycache__/*.pyc` 在 Phase 1 提交时被一并纳入。`.pyc` 已在 Phase 2 取消跟踪并加入 `.gitignore`；计划文档保留（属 M1 工作产物，编排者已生成）。
5. **clippy 修复**：新增测试 `&r.vectors()[..]` 触发 `clippy::redundant_slicing`，已改为 `r.vectors()`，独立提交 `d4dee8b`。

## 自证门禁结果

全部在仓库根执行，结果如下：

| 门禁 | 结果 |
|---|---|
| `cargo test --workspace --all-features` | ✅ vane-core lib 182 passed / 1 ignored；corpus_compat 2 passed；recall 1 passed；vane-node 19 passed；vane-ffi 4 passed；其余 0 |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | ✅ Finished，无 warning |
| `cargo check --target wasm32-unknown-unknown -p vane-core` | ✅ Finished |
| `cargo fmt --all -- --check` | ✅ FMT OK |
| `bash scripts/check-no-std-fs.sh` | ✅ OK（core 业务代码无 std::fs/std::net/mmap，例外仅 vfs/std_fs.rs） |
| `bash crates/vane-node/scripts/check-thin.sh` | ✅ OK: vane-node is a thin binding (I-8 clean) |
| `cargo test --test corpus_compat -p vane-core` | ✅ 2 passed |
| `cargo bench --no-run -p vane-core` | ✅ Finished（benches 编译通过） |

关键输出摘要：
```
cargo test --workspace --all-features:
  test result: ok. 182 passed; 0 failed; 1 ignored  (vane-core lib)
  test result: ok. 2 passed; 0 failed                (corpus_compat)
  test result: ok. 1 passed; 0 failed                (recall)
  test result: ok. 19 passed; 0 failed               (vane-node)
  test result: ok. 4 passed; 0 failed                (vane-ffi)

cargo clippy --workspace --all-targets --all-features -- -D warnings:
  Finished `dev` profile [unoptimized + debuginfo] target(s)

cargo check --target wasm32-unknown-unknown -p vane-core:
  Finished `dev` profile

scripts/check-no-std-fs.sh: OK
crates/vane-node/scripts/check-thin.sh: OK: vane-node is a thin binding (I-8 clean)
cargo bench --no-run -p vane-core: Finished `bench` profile [optimized]
```

## 不变量遵守

- I-1（段不可变）：finalize 消费 self，未改。✅
- I-4（单一分词身份）：未触及分词器。✅
- I-5（核心零平台分支）：未新增 `cfg(target_arch="wasm32")`；corpus_compat 测试用 `StdFsVfs`（其 cfg 在 vfs 实现处，合规），测试位于 `tests/` 目录不参与 wasm32 check。✅
- I-6（manifest 原子性）：未触及 manifest 写入路径。✅
- core 业务代码无 `std::fs`/`std::net`/mmap；测试夹具用 std::fs 在 `tests/` 目录（合规）。✅
- 未引入 dashmap/parking_lot/黑名单依赖。✅
- 冻结常量（BM25 k1/b、RRF k、段数上限、dim、DOC_MAX、topK）未改。✅

## 遗留 / 疑问

1. **`inverted.bin` 头校验缺失**：`corpus_segment_files_have_magic_version_headers` 校验了 5 个段文件，未含 `inverted.bin`（由 `bm25::write_inverted` 写入）。若编排者希望 inverted.bin 也纳入显式头校验，可在阶段零-B 或 M1 补。当前 `corpus_format_compat_roundtrip` 的 text/hybrid 搜索已隐式验证 inverted.bin 可读。
2. **stored `tag` 回填带引号**：M0 既有行为（`serde_json::Value::to_string()` 保留 JSON 引号）。非本任务范围，但若编排者认为应回填裸值，需在 07-api-core 健壮性阶段调整 `collection.rs::search` 的 fields 回填逻辑。
3. **排除项确认**：FF4 严格化（dim 推导/stored 截断）、stored.bin zstd、HNSW/jieba/tombstone/WAL/Go、07-api-core parked 项——均未触碰，符合简报排除项。

## 状态

DONE — 5 个 phase 全部落地，自证门禁全绿，无阻塞。
