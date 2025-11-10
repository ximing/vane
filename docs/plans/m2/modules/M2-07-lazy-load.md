# M2-07 冷启动懒加载

## 1. 目标
`SegmentReader` 改为按需加载 vectors/stored（`OnceLock`），`open` 仅读 header+idmap+manifest，元数据 open <1s（SPEC v1.2 修订 A 已批准，§13.1）；首次向量查询触发 vectors 加载 <3s。**不改 §4 IDL 签名**（`vectors(&self)->&[f32]` 等保 `&self` 返回 `&[f32]`，用 `OnceLock` 内部可变）。

SPEC 节号：§13.1（元数据 open<1s + 首次查询<3s，v1.2 修订 A）、§6.2（懒加载语义注释，v1.2 已加 lines 212-213）。

## 2. 涉及文件
- **Modify** `crates/vane-core/src/segment/mod.rs:315-325`（`SegmentReader` struct）：`vectors: Vec<f32>` → `vectors: OnceLock<Vec<f32>>`；`stored: HashMap<u64, StoredReadEntry>` → `stored: OnceLock<HashMap<u64, StoredReadEntry>>`。
- **Modify** `crates/vane-core/src/segment/mod.rs:344-394`（`SegmentReader::open`）：移除 vectors/stored 全量加载；仅读 header（`decode_header`）+ id_map（`load_id_map`）+ 从 vectors.bin 头读 dim（v2 头含 dim；v1 回退 `payload_len/doc_count`，协同 M2-08）。
- **Modify** `crates/vane-core/src/segment/mod.rs:417-419`（`vectors()`）：改为 `self.vectors.get_or_init(|| Self::load_vectors(&self.vfs, &self.segment_dir, self.dim, self.meta.doc_count).unwrap_or_default()).as_slice()`。
- **Modify** `crates/vane-core/src/segment/mod.rs:442-456`（`stored_json`/`text`）：首次调用触发 `stored.get_or_init(|| Self::load_stored(..))`。
- **Create** `fn load_vectors(vfs, segment_dir, dim, doc_count) -> Result<Vec<f32>>`：从 vectors.bin 读 + decode（原 open 内逻辑提取）。
- **Modify** `crates/vane-core/src/segment/header.rs` 或 `mod.rs`：dim 来源逻辑——vectors.bin v2 头 `magic|version|dim(4 LE)`（M2-08 写入）；v1 回退 `(vbuf.len()-8) / doc_count / 4`（原 `mod.rs:373-377`）。
- **Modify** `crates/vane-core/benches/cold_start.rs`（M1 11-cold-start-bench）：更新断言——元数据 open<1s（M2 实测背书），首次查询<3s（降级分级保留 fallback）。
- **Modify** `crates/vane-core/tests/cold_start_gate.rs`：gate 断言改 open<1s + 首次查询<3s。

## 3. 接口契约
### Consumes from
- M0/M1 `SegmentReader::open`/`vectors`/`dim`/`stored_json`/`text`（**签名不变**，`mod.rs:344/417/420/442/450`）。
- M0 `header::decode_header`（`header.rs`）。
- M2-08 `vectors.bin` v2 头含 dim（per-file `VECTORS_FORMAT_V2`，M2-08 落实写入；M2-07 实现读取，v1 回退）。

### Produces for
- `SegmentReader` 字段改 `OnceLock`（内部行为，对外签名不变）。下游消费方零改动：
  - M1 `HnswReader::search`（`hnsw/mod.rs:624`）借 `reader.vectors()`——首次 search 触发 load。
  - M1 `api/collection.rs:766,773,781` `brute_search(reader.vectors(), ..)`——同上。
  - M1 `api/reindex.rs:142`、`merge/mod.rs:199` `reader.vectors()[..]`——reindex/merge 本需全量，首次访问触发 load。
- 下游 M2-09（SQ8 挂在 `vectors()` 访问点）、M2-10（Executor 并行搜索消费 `vectors()`）、M2-12（export 读段 vectors）。

### dim 来源设计（与 M2-08 协同，stub-then-regress 策略）
- vectors.bin v2 头：`magic(4)|version(4 LE)=2|dim(4 LE)|payload`（12 字节头）。
- vectors.bin v1 头：`magic(4)|version(4 LE)=1|payload`（8 字节头，M0/M1 产物）。
- open 读 version：v2 → 读 dim 字段；v1 → dim = `(file_len - 8) / doc_count / 4`（回退原逻辑）。
- **stub-then-regress 策略**（reviewer A 确认无死锁，需显式标注）：M2-07 与 M2-08 同批 L0 推进时，M2-07 的 dim 读 v2 测试用 **stub v2 header**（手工构造 `magic|version=2|dim(4 LE)|payload` 12 字节头，不依赖 M2-08 finalize 落实）；M2-08 落实 `finalize` 写 v2 头 + `VECTORS_FORMAT_V2` 常量后，M2-07 回归测试切到真实 v2 产物（由 M2-08 finalize 生成）。两计划 dim 读/写版本对齐：v2 头固定 12 字节，v1 头 8 字节，`version` 字段是判别位（M2-07 读、M2-08 写同一字段）。两计划可独立落地 + 回归，无循环依赖。

## 4. TDD 测试清单
1. **open 不加载 vectors**：open 后构造 `SegmentReader`，断言 `vectors` OnceLock 未初始化（`get()` 返回 None）。需暴露测试 hook 或通过行为间接验证（open 时间 <加载 vectors 时间）。
2. **open <1s**（10万×384 维 fixture，M1 cold_start fixture 复用）：`cold_start_gate` 断言 open 耗时 <1000ms（M1 实测 1573ms → M2 懒加载后 <1s）。
3. **首次 vectors() 触发加载**：open 后调 `vectors()`，返回非空 `&[f32]`（doc_count>0 时），且 OnceLock 已初始化。
4. **首次查询 <3s**：open 后首次 `search({vector, topK:10})`，耗时 <3000ms（含 vectors 加载 + HNSW 搜索）。
5. **vectors() 幂等**：多次调 `vectors()` 返回同一 `&[f32]`（OnceLock 只加载一次）。
6. **vectors() 并发安全**：多线程同时首次调 `vectors()`，只加载一次（`OnceLock::get_or_init` 原子保证，无数据竞争）。用 `std::thread::scope` 测试。
7. **stored 懒加载**：open 后 `stored_json(0)` / `text(0)` 触发 stored 加载；多次调用幂等。
8. **dim 正确性 v2**：vectors.bin v2 头含 dim=384 → open 后 `reader.dim()==384`（不触发 vectors 加载）。
9. **dim 回退 v1**：M0/M1 产物 vectors.bin v1（无 dim 字段）→ open 后 `reader.dim()==384`（从 payload 长度反推）。
10. **corpus 兼容**：M0/M1 既有 corpus（v1 vectors.bin）被新版本 open + search 正常（冻结 corpus 兼容测试，SPEC §13.3）。
11. **reindex/merge 不破**：reindex（`api/reindex.rs`）+ merge（`merge/mod.rs`）路径在懒加载下正常（首次访问触发 load，结果与 M1 一致）。
12. **HNSW search 正确**：open 后 search，recall 与 M1 基线一致（HNSW nodes 仍 open 时加载，vectors 懒加载，导航正确）。
13. **OnceLock 无新依赖**：`std::sync::OnceLock`（Rust 1.70+，无新 crate，符合黑名单）。

## 5. 验收标准
- open 10万库 <1s（M2 实测背书，SPEC §13.1）。
- 首次向量查询 <3s（含 vectors 加载）。
- 全部 M0/M1 既有测试绿（340+，懒加载不破回归）。
- corpus 兼容测试绿（v1 vectors.bin 双模读取）。
- `vectors(&self)->&[f32]` 签名不变（grep 验证 `mod.rs:417` 签名未改）。
- clippy clean。

## 6. 前置依赖
- SPEC v1.2 修订 A（已批准）。
- M2-08 协同（vectors.bin v2 头含 dim；可先 stub，M2-08 落实后回归）。

## 7. 不变量覆盖
- **I-1 段不可变**：懒加载不写回段文件，只读缓存。测试 10+11 守护。
- **§4 IDL 签名冻结**：`vectors`/`dim`/`stored_json`/`text` 签名不变（OnceLock 内部可变）。验收「签名不变」守护。
- **§13.1 冷启动承诺**：测试 2+4 落实 open<1s + 首次查询<3s。
- **并发安全**：测试 6 `OnceLock::get_or_init` 原子性。
- **corpus 兼容**：测试 10 SPEC §13.3 冻结兼容。
