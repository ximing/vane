# 阶段零-A：M0 段格式冻结清理（派发简报）

> 这是你（cleanup SubAgent）的需求文档。读它 first——它是你的需求来源，含精确值与 SPEC 引用。
> 你是 Rust 实现者，遵循 TDD。完成后自证全量门禁。产出报告写入本文件末尾"实现报告"节。
> 上游契约：`docs/SPEC.md` v1.0（§6.2/§6.3/§13.3）、`docs/plans/m0/README.md` Global Interface Contracts（单一事实源）。

## 背景

Vane M0 已完成（commit 538db51）。M1 的 HNSW 会扩展 segment 格式，必须先把 M0 segment 格式冻结。本任务清理 M0 遗留的格式/CI 项，使格式冻结，并落地 §13.3 corpus 兼容测试骨架作为格式冻结的契约门禁。

本仓库 M0 未发布任何产物（fresh repo），故格式变更无向后兼容约束——corpus 兼容测试冻结的是清理**后**的格式。

## 环境

- 工作目录：`/Users/ximing/project/mygithub/vane`（main 分支，干净，已 commit 538db51）。
- worktree 隔离不可用，直接在 main 上工作。每完成一个 phase 增量 commit。
- 全程中文注释与文档。

## 范围（按 phase 顺序，每 phase 后跑门禁）

### Phase 1 · FF1：vectors.bin 加 magic+version 头（SPEC §6.2 合规）

**问题**：`crates/vane-core/src/segment/mod.rs` `SegmentWriter::finalize`（约 104-112 行）写 vectors.bin 为纯 f32 LE，无 8 字节头；`SegmentReader::open`（约 215-223 行）直接 `chunks_exact(4)` 读全文件。违反 SPEC §6.2"所有文件以 4 字节 magic + 4 字节 format_version 开头"。

**裁决 FA1**：vectors.bin 加 8 字节头 = `MAGIC`(4) + `FORMAT_VERSION`(4, LE，与 FF3 统一)。
- 写：`finalize` 写 vectors.bin 时先写 `MAGIC` + `&FORMAT_VERSION.to_le_bytes()`，再写 f32 LE payload。
- 读：`SegmentReader::open` 读 vectors.bin 时跳过前 8 字节，再 `chunks_exact(4)` → f32。doc_count=0 时 vectors.bin 仍写 8 字节头（空段合规）。
- 不影响 `vectors()` 返回纯 f32（brute_search 不受影响）。
- 更新 `segment/tests.rs` 中 `segment_reader_roundtrip`（`reader.vectors().len()==8` 仍成立，因跳过头）等受影响断言。

### Phase 2 · FF3：format_version 统一全 LE

**问题**：`segment/header.rs:16` encode 用 `FORMAT_VERSION.to_be_bytes()`、`:40` decode 用 `from_be_bytes`；`mod.rs` stored.bin(:120)/idmap.bin(:135)/scalars.col(:151) 写入均用 `to_be_bytes()`，而 payload 字段用 LE——字节序混合。`segment/tests.rs:30` 断言 `&bytes[4..8]==[0,0,0,1]`（BE）。

**裁决 FA2**：统一全 LE。
- header.rs：encode `to_le_bytes()`、decode `from_le_bytes()`；更新文件头注释（去掉"format_version 采用大端"说明，改为"全字段 LE"）。
- mod.rs：stored.bin/idmap.bin/scalars.col 的 format_version 一律 `to_le_bytes()`。
- `decode_kv_map`（mod.rs:295）当前跳过 magic+version 不校验 version——顺手加 version 校验（不匹配返回 `E_VERSION`），属 FF4 严格化的可接受轻量部分。
- 更新 `tests.rs:30` 断言为 `&[1, 0, 0, 0]`（LE）。
- `header_roundtrip` 测试仍须通过。

### Phase 3 · FF2：add_doc 局部 docid 断言

**问题**：代码注释（`mod.rs:66-67`）已正确说明"返回段内局部 docid（从 0 起），全局=docid_base+返回值"。`segment_writer_docid_base_nonzero`（tests.rs:188）测了 base>0 的 meta 读回，但未断言 add_doc **返回值**是局部 docid。

**裁决**：在 `segment_writer_docid_base_nonzero` 测试中（或新增测试），断言：base=2 时 `w2.add_doc("c", ...)` 返回 `0`（局部，非全局 2），并显式验证全局 docid 概念 = base + 返回值。这是文档化的不变量（SPEC §3.2）。

### Phase 4 · corpus 兼容测试骨架（SPEC §13.3）

**裁决 FA3**：新建 `crates/vane-core/tests/corpus_compat.rs`：
- 用 `StdFsVfs`（tempdir）建 DB → 声明 collection（含 text+vector+scalar 字段）→ `add` 若干文档（含中文/英文 mixed）→ `flush` → `close`。
- 重新 `open` 同目录 → 验证 manifest 加载、segment 读取、`search`（hybrid/vector/text 三模式）结果与写入前的基线一致、`external_id`/`stored_json` 回填正确。
- 文件头注释文档化：此测试冻结 segment 格式；任何格式变更必须保持此测试通过，或 bump `FORMAT_VERSION` + 提供迁移器/双模读取（SPEC §6.2）。
- uncomment `.github/workflows/ci.yml` 的 `corpus-compat` job（约 82-84 行），接入 `cargo test --test corpus_compat -p vane-core`，放在 `needs: test` 之后。
- 这是"骨架"——M0 口径即可（不需要真实历史版本 golden fixture，因无发布版本）；建立契约门禁是目标。

### Phase 5 · FF6：wasm32 体积门禁 deferred 注释

**裁决 FA4**：`.github/workflows/ci.yml` 加一个注释化的 deferred job（不实跑），说明：SPEC §13.2-3 核心 wasm gzip ≤800KB（含 jieba 代码、不含词典）门禁 M1 jieba 落地起生效，届时补 `wasm-opt` + gzip size check。放在合适位置（如 wasm32-check job 之后或 deny 之后）。

## 自证门禁（全部须绿，clippy 必须含 --all-targets）

完成后在仓库根跑并贴出结果：
```
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo check --target wasm32-unknown-unknown -p vane-core
cargo fmt --all -- --check
bash scripts/check-no-std-fs.sh
bash crates/vane-node/scripts/check-thin.sh
cargo test --test corpus_compat -p vane-core        # 新增
cargo bench --no-run -p vane-core                   # 确认 bench 仍编译
```

## 全局约束（每条须遵守）

- core 禁 `std::fs`/`std::net`/mmap（CI 门禁；`cfg(target_arch="wasm32")` 只允许在 VFS 实现）。新测试用 `StdFsVfs`，不在 core 业务代码用 `std::fs`（测试夹具 `#[cfg(test)]` 或 `tests/` 目录可用 `std::fs`，参照既有 `segment_stdfs_roundtrip` 测试）。
- `cfg` 只允许在 VFS/Executor 实现处（不变量 I-5）。
- 并发原语统一 `std::sync::RwLock`/`Mutex`；不引入 dashmap/parking_lot。
- 依赖黑名单：regex/tokio/prost/tonic/openssl/lindera/ndarray/wee_alloc。
- BM25 k1=1.2/b=0.75、RRF k=60、段数上限 10、dim≤4096、单文档≤16MB、topK≤1000、用户词表≤10万——均冻结，不得改。
- 段文件头 = 4 字节 magic + 4 字节 format_version（SPEC §6.2）。
- MoSCoW 即合同：不得新增需求。本任务只做上述 5 个 phase，不实现 HNSW/jieba/删除等 M1 功能。
- 不变量 I-1（段不可变）/I-4（单一分词身份）/I-5（核心零平台分支）/I-6（manifest 原子性）不得违反。

## 排除项（不要做，属 M1）

- recall 硬编码 1.0 → M1 HNSW 真实回归 job。
- FF4 严格解码加严（除 FA2 顺手的 version 校验外，dim 推导校验/stored 解码截断严格化留 M1）。
- 07-api-core 健壮性 parked 项（restore base/inv_readers zip/checked_sub/vector_field hoist）→ 留阶段零-B 或 M1。
- stored.bin zstd 压缩 → M1。
- 任何 HNSW/jieba/tombstone/WAL/Go 相关代码。

## 提交

每完成一个 phase 增量 commit（commit message 中文，末尾加 ``）。例：`fix(segment): FF1 vectors.bin 加 magic+version 头合规 §6.2`。

---

## 实现报告（SubAgent 填写）

> 完成后在此填写：每个 phase 的实际改动（文件:行）、遇到的偏离与裁决、自证门禁结果（贴关键输出）、提交 hash 列表、遗留/疑问。
> 报告是你的返回值——编排者据它审查。
