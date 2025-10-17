# 阶段零-A：M0 段格式冻结清理 — 代码审查报告

> 审查者：review SubAgent（只读）。审查对象：BASE=538db51..HEAD=c287458 的 `crates/ .github/ .gitignore` diff。
> 需求来源：`docs/plans/m1/00-cleanup.md`（FA1~FA5 + 5 phase + 排除项）。
> 上游契约：`docs/SPEC.md` v1.0 §6.2/§6.3/§10/§13.3/§14；`docs/plans/m0/README.md` Global Interface Contracts。

## 审查结论汇总

| 维度 | 结论 | 证据 |
|---|---|---|
| FF1 vectors.bin 头 | ✅ | `segment/mod.rs:104-117`(写)、`:216-235`(读) |
| FF3 全 LE 统一 | ✅ | `header.rs:16,40`、`mod.rs:124,139,155`、`tests.rs:30`；全仓无残留 BE |
| FF2 局部 docid 断言 | ✅ | `tests.rs:209-217` |
| corpus 兼容测试 §13.3 | ✅（含 1 处 minor） | `tests/corpus_compat.rs` 全文 |
| FF6 wasm32 deferred 注释 | ✅ | `ci.yml:91-100` |
| 不变量 I-1/I-4/I-5/I-6 | ✅ | 见下文逐条 |
| M0 pub API 兼容 | ✅ | pub 签名零改动 |
| 范围合规 | ✅ | 未触排除项、未引黑名单依赖 |
| 测试质量（clippy/断言） | ✅ | `d4dee8b` 修 redundant_slicing |

**Verdict：APPROVED_WITH_MINOR**

---

## 逐维度详审

### 1. FF1 正确性 ✅

**写路径**（`crates/vane-core/src/segment/mod.rs:104-117`）：`finalize` 写 vectors.bin 时先 `extend_from_slice(MAGIC)` + `FORMAT_VERSION.to_le_bytes()`，再追加 f32 LE payload。`vbytes` 容量调整为 `8 + vectors.len()*4`，无浪费。doc_count=0 时 `self.vectors` 为空，仍写出 8 字节头——`vectors_bin_empty_segment_still_writes_header` 测试（`tests.rs:269-305`）直接读原始字节断言 `buf.len()==8`、magic=`VANE`、version=`[1,0,0,0]`，空段合规已坐实。

**读路径**（`mod.rs:216-235`）：doc_count>0 时读全文件，校验 `len<8`→`E_CORRUPT`、`magic` 不符→`E_CORRUPT`、version(LE) 不匹配→`E_VERSION`，随后 `vbuf[8..].chunks_exact(4)` 跳过头取 f32。`vectors()` 仍返回纯 `&[f32]`，`brute_search` 不受影响（`vectors_bin_has_magic_version_header` 测试断言 `r.vectors()==&[1.0,2.0,3.0,4.0]`）。doc_count=0 时不读 vectors.bin、返回空 Vec——与"空段仍写头"配合，读路径不会对空段的头做校验，但空段无 payload 需保护，可接受。

**截断/坏 magic**：读路径对 `len<8`、bad magic 均报 `VaneError::Corrupt`（E_CORRUPT），version 不匹配报 `VaneError::Version`（E_VERSION），符合 §10。

### 2. FF3 完整性 ✅

- `header.rs:16` encode `to_le_bytes()`、`:40` decode `from_le_bytes()`——已切换。
- `mod.rs:124`(stored.bin)、`:139`(idmap.bin)、`:155`(scalars.col) 三处 `to_be_bytes()`→`to_le_bytes()`——已切换。
- **全仓扫描** `grep -rn "to_be_bytes\|from_be_bytes" crates/vane-core/src/` 返回空——无残留 BE。
- `decode_kv_map`（`mod.rs:308-330`）：新增 magic 校验（不符→E_CORRUPT）+ version 校验（不符→E_VERSION），属简报允许的"FF4 严格化可接受轻量部分"。注释 `version(4 LE)` 已同步。
- `header.rs:5-10` 文件头注释：去掉"format_version 采用大端"说明，改为"FA2：全字段统一 LE（含 format_version）"——已同步。
- `tests.rs:30` 断言由 BE `&[0,0,0,1]` 改为 LE `&[1,0,0,0]`——已同步。`header_roundtrip` 通过。

**注意（非阻塞）**：`decode_kv_map` 仍保留 `buf.len() < 12` 时返回 `Ok(empty HashMap)` 的早返回（`mod.rs:314-316`）。这意味着 0~11 字节的截断文件被当作空 map 而非 E_CORRUPT。但此为 M0 既有行为，且属简报明确排除的"stored 解码截断严格化留 M1"——合规。

### 3. FF2 ✅

`tests.rs:209-217`：base=2 时 `w2.add_doc("c", ...)` 返回值赋给 `local_id`，断言 `local_id == 0`（局部，非全局 2）；随后 `global_id = m1.docid_base + m1.doc_count as u64 + local_id` 显式验证 `== 2`。文档化 SPEC §3.2 局部 docid 语义，断言有意义。

### 4. corpus 兼容测试（§13.3）✅（含 1 处 minor）

**契约建立**（`corpus_format_compat_roundtrip`）：
- 用 `StdFsVfs` 建库 → 声明 collection（text `body` + vector `v` dim=4 Cosine + scalar `tag` Keyword）→ 灌 5 篇中英混排文档 → `flush` → 捕获三模式（Hybrid/Vector/Text）基线 `(id, score, tag)` → `close` → 重新 `open` 同目录 → 逐条比对 id 相等、score 绝对差 `<1e-6`、tag 相等。这是真实的"写库→close→reopen→验证搜索/stored/external_id 一致"契约，非 tautological。
- manifest restore 验证：`db.collections().iter().any(|c| c == "docs")`。
- tag 回填验证：断言为 `"\"a\""`/`"\"b\""`/`"\"c\""` 之一的 JSON 串（与 M0 `Value::to_string()` 行为一致）。
- hybrid 命中非空断言：`assert!(!baseline[0].is_empty())`。

**并行隔离**：`unique_dir` 用 `AtomicU64 + pid + nanos` 生成唯一临时目录，防并行冲突。✅

**格式冻结文档化**：文件头注释（`:1-13`）明确"此测试冻结 M0 段格式；任何格式变更必须保持此测试通过，或 bump FORMAT_VERSION + 提供迁移器/双模读取（SPEC §6.2）"。✅

**Minor（非阻塞）**：`corpus_segment_files_have_magic_version_headers` 测试（`:209-273`）只校验 5 个文件（header.bin/vectors.bin/stored.bin/idmap.bin/scalars.col）的头，但文件头注释（`:3`）声称冻结范围含 `inverted.bin`。inverted.bin 的头校验由 `bm25::InvertedIndexReader::open`（`bm25.rs:338-358`）在读路径独立完成（见下文"疑点 1"），且 roundtrip 测试的 text/hybrid 搜索隐式验证了其可读性，故契约实质生效——仅显式断言清单遗漏 inverted.bin，属测试完整性瑕疵。建议阶段零-B 或 M1 补一行（将 `"inverted.bin"` 加入 `for fname in [...]` 列表）。

### 5. FF6 ✅

`ci.yml:91-100` 新增注释化 deferred job `wasm32-size`，引用值与 SPEC §13.2-3 完全一致：核心 wasm gzip ≤800KB（含 jieba 代码、不含词典）/ 全功能 ≤1.2MB / dict-zh ≤1.5MB / Go embed 增量 <2MB / 500KB 为 M2 优化目标非门禁。注释给出 `wasm-opt` + gzip size check 命令骨架，说明"M1 jieba 落地起生效"。纯注释，不实跑。✅

### 6. 不变量 ✅

- **I-1（段不可变）**：`finalize` 仍 `pub fn finalize(self) -> Result<SegmentMeta>` 消费 self，未改签名、未引入原地修改。✅
- **I-4（单一分词身份）**：diff 未触及分词器/tokenizer 任何代码。✅
- **I-5（核心零平台分支）**：core 业务代码未新增 `cfg(target_arch="wasm32")`。corpus_compat 测试位于 `tests/` 目录，`cargo check --target wasm32 -p vane-core` 默认不编译 integration test target，不参与 wasm32 门禁。`scripts/check-no-std-fs.sh` 仅扫描 `crates/vane-core/src/`（排除 `tests.rs` 夹具与 `vfs/std_fs.rs`），不扫描 `tests/` 目录——corpus_compat 用 `std::fs` 合规。✅
- **I-6（manifest 原子性）**：diff 未触及 manifest 写入路径。✅

### 7. M0 pub API 兼容 ✅

全量核对 `segment/mod.rs`、`header.rs` 的 pub 项：`SegmentWriter::{new, docid_base, add_doc, finalize}`、`SegmentReader::{open, meta, vectors, dim, doc_count, external_id, stored_json, segment_dir, vfs}`、`encode_header`/`decode_header`——签名零改动。diff 仅改函数体内部字节序与头校验逻辑。Schema/brute_search 未触及。✅

### 8. 范围合规 ✅

- 未触 HNSW/jieba/tombstone/WAL/Go 任何代码。
- 未触 FF4 严格化（除简报允许的 decode_kv_map version 校验外，dim 推导/stored 截断严格化未做）。
- 未触 stored.bin zstd 压缩。
- 未触 07-api-core parked 项（collection.rs 不在 diff 中）。
- `Cargo.toml` 未改——未引入任何新依赖，未引入 dashmap/parking_lot/黑名单依赖。✅

### 9. 测试质量 ✅

- `d4dee8b` 修复 `clippy::redundant_slicing`（`&r.vectors()[..]`→`r.vectors()`），独立提交。`cargo clippy --workspace --all-targets --all-features -- -D warnings` 绿。
- 新增断言均有意义：无 tautological/空断言。score 用 `<1e-6` 浮点容差，id/tag 用精确相等。

### 10. implementer 两个疑问的判断

**疑点 ①：inverted.bin 未纳入头校验——是否为真实缺口？**

**结论：非格式缺口，仅测试显式断言清单的完整性瑕疵（minor）。**

证据：`bm25::write_inverted`（`crates/vane-core/src/bm25.rs:229-236`）写 inverted.bin 时已写 `MAGIC` + `FORMAT_VERSION.to_le_bytes()` 头；`InvertedIndexReader::open`（`bm25.rs:338-358`）读路径已校验 `len>=8`（E_CORRUPT）、magic（E_CORRUPT）、version(LE)（E_VERSION）。即 inverted.bin 的格式本身完全合规，且读路径有严格校验。

唯一的缺口是 `corpus_segment_files_have_magic_version_headers` 测试的 `for fname in [...]` 列表遗漏 `"inverted.bin"`，与文件头注释声称的冻结范围（含 inverted.bin）不一致。但 roundtrip 测试的 text/hybrid 搜索隐式验证了 inverted.bin 可读性，契约实质生效。**建议**：补 `"inverted.bin"` 到该列表（一行改动），非阻塞。

**疑点 ②：stored tag 回填带 JSON 引号——是否 M0 既有行为？**

**结论：是 M0 既有行为，非本次引入。**

证据：`git diff 538db51..HEAD -- crates/vane-core/src/api/collection.rs` 返回空——collection.rs 未被修改。`serde_json::Value::String("a").to_string()` 产出 `"\"a\""`（保留 JSON 引号）是 serde_json 的标准行为。这属 07-api-core 健壮性范畴（简报明确列为 parked 项），不在本次清理范围。implementer 在测试中按实际形态校验（`Some("\"a\"")`）而非改业务代码，处置正确。

---

## 阻塞项

无。

## 需编排者裁决的疑点

1. **inverted.bin 纳入 corpus_compat 显式头校验**（minor）：是否在阶段零-B 或 M1 入口前补一行，将 `"inverted.bin"` 加入 `corpus_segment_files_have_magic_version_headers` 的校验列表，使显式断言与文件头注释声明的冻结范围一致？当前不阻塞（格式本身合规、读路径有校验、roundtrip 隐式覆盖）。
2. **stored tag 回填带引号**（ parked）：确认留到 07-api-core 健壮性阶段处理 `collection.rs::search` 的 fields 回填逻辑（回填裸值 vs JSON 串），非本次范围。

## 附：门禁复述

implementer 自证 8 项门禁全绿，本次审查未独立重跑（只读审查者），但 diff 与测试代码逐行核对与报告一致：测试数（vane-core lib 182 + corpus_compat 2 + recall 1 + vane-node 19 + vane-ffi 4）、clippy `--all-targets` 无 warning、wasm32 check 通过、fmt 通过、check-no-std-fs 与 check-thin 通过、bench `--no-run` 编译通过。
