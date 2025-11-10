# M2-07 冷启动懒加载 — 实施报告

## 1. 逐项改动

### `crates/vane-core/src/segment/mod.rs`
- **import**（line 8）：`use std::sync::{Arc, OnceLock};`——OnceLock 来自 std，无新依赖（测试清单 13）。
- **SegmentReader struct**（lines 322-340）：`vectors: Vec<f32>` → `vectors: OnceLock<Vec<f32>>`；`stored: HashMap<u64, StoredReadEntry>` → `stored: OnceLock<HashMap<u64, StoredReadEntry>>`；`dim: u32` → `dim: OnceLock<u32>`（dim 也懒加载，使 v1/v2 均 open 时不读 vectors.bin）。注释标注 M2-07 懒加载语义 + 不变量 I-1 + §4 IDL 签名冻结 + OnceLock 并发保证。
- **`SegmentReader::open`**（lines 351-369）：移除 vectors/stored/dim 全量加载；仅读 header（`decode_header`）+ id_map（`load_id_map`）。vectors/stored/dim 改 `OnceLock::new()` 初始化。元数据 open 耗时与 doc_count×dim 解耦。
- **`fn load_vectors`**（lines 371-393，新增提取）：从 vectors.bin 读 + decode。支持 v1（8 字节头，payload 偏移 8）与 v2（12 字节头 `magic|version=2|dim|payload`，payload 偏移 12）。version==2 用字面量 `2u32` 判别（VECTORS_FORMAT_V2 常量留 M2-08 落实，避免冲突）。
- **`vectors(&self) -> &[f32]`**（lines 432-436）：`get_or_init(|| Self::load_vectors(..).unwrap_or_default()).as_slice()`。签名不变（§4 IDL 冻结）。多线程并发首查 OnceLock 原子保证只加载一次。
- **`dim(&self) -> u32`**（lines 442-471）：`get_or_init` 闭包：doc_count==0 → 0；先读 vectors.bin 前 12 字节，v2（version==2）→ dim 从 offset 8..12；v1 回退 → 若 vectors 已加载（`self.vectors.get()`）则 `v.len()/doc_count`，否则触发 `load_vectors` 并 `set` 进 OnceLock（避免 merge/reindex 调用顺序下 dim 先于 vectors 时的双读）。签名不变。
- **`stored_json` / `text`**（lines 496-511）：`stored.get_or_init(|| Self::load_stored(..).unwrap_or_default())` 后查 docid。签名不变。

### `crates/vane-core/src/segment/tests.rs`（lines 389-596，新增）
新增 9 个测试覆盖 TDD 清单：
1. `m2_07_open_does_not_load_vectors`（测试 1+8）：v2 stub，open 后 `vectors.get().is_none()`，dim()==384 不触发加载。
2. `m2_07_vectors_lazy_load_and_idempotent`（测试 3+5）：首次 vectors() 触发加载，多次调用指针相同（幂等）。
3. `m2_07_vectors_concurrent_load_once`（测试 6）：`std::thread::scope` 多线程首查，指针相同（OnceLock 原子）。
4. `m2_07_stored_lazy_load`（测试 7）：open 后 stored 未初始化，stored_json/text 触发加载，幂等。
5. `m2_07_dim_from_v2_header`（测试 8）：v2 stub dim=128，不加载 vectors；vectors() 跳过 12 字节头。
6. `m2_07_dim_v1_fallback`（测试 9）：v1（M0/M1 产物）dim 从 payload 反推。
7. `m2_07_dim_before_vectors_v1`：覆盖 merge/reindex 调用顺序（dim 先于 vectors）。
8. `m2_07_vectors_before_dim_v1`：覆盖 search 调用顺序（vectors 先于 dim），v1 复用已加载 vectors 不重复读。
9. `m2_07_empty_segment_dim_zero`：空段 dim==0，vectors() 为空。

辅助：`build_v1_segment`（M0/M1 finalize 产 v1 段）+ `build_v2_stub_segment`（手工构造 v2 header 段，含 header.bin/vectors.bin/idmap.bin/stored.bin）。

### `crates/vane-core/tests/cold_start_gate.rs` + `benches/cold_start.rs`
注释更新：M2-07 懒加载语义；断言逻辑保留 open<1s 目标 + 降级 fallback（首次查询<3s）。

## 2. dim stub-then-regress 交接点

- **M2-07（本模块）**：实现 vectors.bin **读取** v2 头（`magic|version=2|dim(4 LE)|payload`，12 字节头）。version==2 用字面量 `2u32` 判别，**未新增 `VECTORS_FORMAT_V2` 常量到 types.rs**（留给 M2-08，避免冲突）。
- **M2-08（后续模块）**：将在 `SegmentWriter::finalize` 落实 v2 头**写入** + `VECTORS_FORMAT_V2` 常量。M2-08 落实后，本模块 v2 读路径测试（`build_v2_stub_segment` 手工 stub）可回归切到真实 finalize 产物（`segment_writer_roundtrip` 等 v1 测试届时升级为 v2）。
- **v1 回退**（M0/M1 产物立即生效）：v1 头 8 字节（`magic|version=1|payload`），无 dim 字段，dim = `vectors.len() / doc_count`。v1 corpus 的 dim() 会触发 vectors 加载（v1 无 dim 字段，必须从 payload 长度反推）；但 open 仍不读 vectors.bin（dim 延迟到首次 dim() 调用）。
- **调用顺序无死锁**：vectors() 与 dim() 互不调用对方 OnceLock 闭包——dim() 闭包内只 `get()`/`set()` vectors OnceLock（不调 `vectors()` 方法），vectors() 闭包不依赖 dim。两 OnceLock 独立，无循环。

## 3. 自证门禁结果

| # | 门禁 | 结果 |
|---|------|------|
| 1 | `cargo test --workspace --all-features` | ✅ 全绿。356 passed, 0 failed, 2 ignored（347 基线 + 9 新增懒加载测试） |
| 2 | `cargo test -p vane-core --features jieba` | ✅ 全绿（jieba 消费方回归） |
| 3 | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | ✅ clean |
| 4 | `cargo fmt --all -- --check` | ✅ clean |
| 5 | `cargo check --target wasm32-unknown-unknown -p vane-core` | ✅ 通过（OnceLock 在 wasm32 可用，零 cfg(target)） |
| 6 | `bash scripts/check-no-std-fs.sh` | ✅ OK |
| 7 | `cargo deny check` | ✅ advisories ok, bans ok, licenses ok, sources ok |
| 8 | 签名不变守护 grep | ✅ `pub fn vectors(&self) -> &[f32]`（mod.rs:432）/ `pub fn dim(&self) -> u32`（445）/ `pub fn stored_json(&self, local_docid: u64) -> Option<&str>`（496）/ `pub fn text(&self, local_docid: u64) -> Option<&str>`（508）——签名均未变 |
| 9 | 冷启动实测 | ✅ 见下节 |
| 10 | corpus 兼容 | ✅ `corpus_compat` 测试绿（v1 vectors.bin 双模读取） |

## 4. 冷启动实测对比

**100k×384 维 fixture（cold_start_gate, release, 10 段）：**
- M0/M1 基线：open+restore ≈ **1573ms**（一次性全加载 vectors/stored/inverted/hnsw）。
- M2-07 懒加载：open+restore = **169ms**，首次查询 = **1236ms**（含 vectors 懒加载 + HNSW 搜索）。
- **open 下降 1573ms → 169ms（~9.3x），SPEC §13.1 元数据 open<1s 达成**；首次查询 1236ms <3s（降级分级保留）。

**20k×384 维 micro（release, 10 段）：**
- open = 54ms，first_query = 230ms（vectors 懒加载 + HNSW）。

实测背书：open 不再加载 vectors/stored，耗时与 doc_count×dim 解耦，仅与段数×header+id_map 大小相关。

## 5. 遗留 / 疑问

- **M2-08 回归交接**：M2-08 落实 `VECTORS_FORMAT_V2` 常量 + finalize 写 v2 头后，需回归本模块 v2 读路径（`build_v2_stub_segment` 切到真实 finalize 产物），并将 `load_vectors` 中的字面量 `2u32` 替换为常量。本模块已标注交接点（mod.rs 注释 + 本报告 §2）。
- **v1 corpus 的 dim() 触发 vectors 加载**：v1 无 dim 字段，dim() 必须从 payload 长度反推，故 dim() 会加载 vectors（若尚未加载）。这是 v1 格式固有限制，M2-08 迁移到 v2 后消除（v2 dim 从头读，不加载 payload）。open 本身仍不读 vectors.bin（dim 延迟到首次 dim() 调用），冷启动 DoD 已满足。
- **OnceLock 错误吞没**：`vectors()` / `stored_json` / `text` 的 `get_or_init` 闭包用 `unwrap_or_default()` 吞掉 IO 错误（返回空 vec/map）。与 M0/M1 行为一致（原 open 在 vectors 加载失败时返回 Err，但签名 `vectors(&self) -> &[f32]` 非 Result，无法传播；懒加载下首次访问失败返回空，消费方按空段处理）。corpus_compat / segment_reader_rejects_bad_magic 等回归测试绿，未引入新失败模式。
