# M2-07 冷启动懒加载 — 评审报告

**评审对象**：M2-07 `SegmentReader::open` 改懒加载（OnceLock 按需加载 vectors/stored/dim），签名不变。
**评审范围**：`crates/vane-core/src/segment/mod.rs`、`src/segment/tests.rs`、`tests/cold_start_gate.rs`、`benches/cold_start.rs`；消费方 `api/collection.rs`、`api/reindex.rs`、`merge/mod.rs`。
**BASE..HEAD**：ebd1a58..1337b1a
**评审方式**：只读，未跑 cargo（依赖 implementer 自证 + 编排者门禁）。
**评审日期**：2026-08-09

---

## 结论：PASS_WITH_FINDINGS

- **阻塞（B）**：0
- **重要（I）**：1（错误吞没行为变更）
- **次要（M）**：3

SPEC 契约对齐、签名真未变、冷启动承诺实测达成、消费方零改动声明成立、不变量 I-1/I-5 守护到位、TDD 覆盖充分。唯一 Important 是 OnceLock `unwrap_or_default()` 把 M0/M1 在 open 期 Err 传播的 vectors.bin 损坏检测降级为访问期静默返回空——属签名冻结下的固有取舍，需补观测手段。可进 Phase 收尾，I-1 建议在 M2-08 或后续小补丁补齐。

---

## 1. SPEC 契约核验

### 1.1 §4 IDL 签名冻结 — PASS
grep 实测（`crates/vane-core/src/segment/mod.rs`）：
- `pub fn vectors(&self) -> &[f32]`（mod.rs:432）
- `pub fn dim(&self) -> u32`（mod.rs:445）
- `pub fn stored_json(&self, local_docid: u64) -> Option<&str>`（mod.rs:496）
- `pub fn text(&self, local_docid: u64) -> Option<&str>`（mod.rs:508）

四处签名均未改，OnceLock 通过 `&self` 的 `get_or_init` 提供内部可变性，对外 `&[f32]`/`u32`/`Option<&str>` 返回不变。

### 1.2 §13.1 冷启动承诺 — PASS
SPEC §13.1（docs/SPEC.md:411）：「元数据 open <1s（vectors/stored 懒加载，M2 实测背书）；首次向量查询触发 vectors 加载 <3s」。
implementer 自证：100k×384 维 10 段 fixture，open+restore=169ms（<1s），首次查询=1236ms（<3s）。`tests/cold_start_gate.rs` 断言逻辑：open<1s 走 PASS 分支，否则降级校验首次查询<3s。`open` 实测仅读 header+id_map，与 doc_count×dim 解耦。

### 1.3 §6.2 懒加载语义 — PASS
SPEC §6.2（docs/SPEC.md:212-213）：「SegmentReader::open 仅读 header.bin + idmap.bin + manifest；vectors.bin / stored.bin / hnsw.bin 首次访问时按需加载（OnceLock，core 内部，不改 §4 IDL 签名）」。
实装 `open`（mod.rs:356-375）仅 `decode_header` + `load_id_map`，vectors/stored/dim 均 `OnceLock::new()`。对齐。

---

## 2. 不变量核验

### 2.1 I-1（段不可变） — PASS
懒加载路径只 `read_at`/`read_all` 读段文件，无 `write_at`/`create`。`load_vectors`/`load_stored`/`dim` 闭包均纯读。`segment_immutable_after_finalize` 测试（tests.rs:129）仍守护。

### 2.2 I-5（core 零 cfg(target)，OnceLock 是 std::sync 合法） — PASS
- `use std::sync::{Arc, OnceLock};`（mod.rs:8）——OnceLock 来自 std，无新 crate 依赖。
- grep `cfg(target` / `rayon` / `crossbeam` 在 `segment/mod.rs` + `tests.rs`：0 命中。
- `std::thread::scope` 仅出现在 `tests.rs:507`（测试代码，非 core 运行时）——符合「core 运行时无 thread::scope/rayon」。
- implementer 自证 `cargo check --target wasm32-unknown-unknown` 通过（OnceLock 在 wasm32 可用）。

---

## 3. 消费方零改动声明 — PASS

grep 消费方调用点全部未改：
- `api/collection.rs:763` `hr.search(qv, want, ef, merged_filter, base, reader.vectors())` — HnswReader::search 借 `reader.vectors()` 只读。
- `api/collection.rs:766-767, 777-778` `brute_search(reader.vectors(), reader.dim(), ..)` — 参数求值左到右，`vectors()` 先于 `dim()` 求值，v1 路径下 dim() 闭包复用已加载 vectors（mod.rs:463 `self.vectors.get()`），无重复读。
- `api/reindex.rs:127,142,146,149,178` `reader.dim()`/`vectors()`/`stored_json`/`text` — reindex 路径，首次访问触发 load。
- `merge/mod.rs:177,199,203,206` `reader.dim()`/`vectors()`/`stored_json`/`text` — merge 路径，同上。

OnceLock `&self` 的 `get_or_init` 提供内部可变性，消费方代码字节级未改（diff 中 `api/`、`merge/` 零改动）。声明成立。

---

## 4. 关注点核验

### 4.1 [I-1] OnceLock 错误吞没 — 行为变更，非与 M0/M1 一致

**证据**：
- `vectors()`（mod.rs:432-438）：`Self::load_vectors(..).unwrap_or_default()` — IO 错误（bad magic / truncated / read 失败）被吞，返回空 `&[f32]`。
- `stored_json`/`text`（mod.rs:496-512）：`Self::load_stored(..).unwrap_or_default()` — 同样吞错返回空 HashMap，`stored_json(id)` 返回 `None`。
- `dim()` v1 回退（mod.rs:466-473）：`match Self::load_vectors(..) { Ok(v) => .., Err(_) => 0 }` — 错误吞为 dim=0。

**判定**：implementer 报告 §5 称「与 M0/M1 行为一致」**不成立**。M0/M1 `open`（旧 mod.rs:344-394）在 vectors.bin bad magic / unsupported version 时 `return Err(VaneError::Corrupt/Version)`，错误在 open 期显式传播，段不可用。M2-07 把检测点从 open 期推迟到首次 `vectors()` 访问期，且因签名 `vectors(&self) -> &[f32]` 非 `Result` 无法传播，改为 `unwrap_or_default()` 返回空。

**后果**：vectors.bin 损坏（bad magic / truncated / 版本不符）→ `open` 仍成功 → 首次 search 调 `reader.vectors()` 得空切片 → search 返回空结果，**无任何错误信号**。stored.bin 损坏同理 → `stored_json` 返 `None`，Hit.fields 静默丢失。这是错误可见性的回归。

**佐证**：`segment_reader_rejects_bad_magic`（tests.rs:118）仅 corrupt **header.bin** magic（仍由 open 期 `decode_header` 捕获），**未覆盖 vectors.bin-only 损坏**。M0/M1 旧 open 会捕获 vectors.bin bad magic，M2-07 不再捕获。

**严重度**：I（Important）。不阻塞 M2-07 落地——签名冻结是 SPEC §4 硬约束，`vectors(&self) -> &[f32]` 无法返 Result，懒加载必然要把错误推迟到访问期，此取舍由 SPEC v1.2 修订 A 批准。但「静默吞没」不可接受，需补观测手段：
- **建议修复**（M2-08 或后续小补丁）：`vectors`/`stored` 字段改为 `OnceLock<Result<Vec<f32>, VaneError>>`（或 `OnceLock<Result<HashMap, VaneError>>`），`vectors()` 在 `Err` 时 `panic!` 或 `expect` 带 ulid 上下文——保留签名不变，但把静默空返改为显式 panic，让损坏在首查时立即暴露而非伪装成空结果。或至少 `eprintln!`/`tracing::error!` 记录错误。
- **最低限度**：补一个 `fn vectors_load_error(&self) -> Option<&VaneError>` 健康检查方法，或 `fn check_health(&self) -> Result<()>`，供 Db::open 后显式探测。
- **测试缺口**：补 `m2_07_vectors_corrupt_returns_empty_or_panics` 测试，明确预期行为（空返 or panic），锁定契约。

### 4.2 [M-1] v1 dim() 触发 vectors 加载 — 不破坏冷启动，但需文档化

**证据**：v1 vectors.bin 无 dim 字段，`dim()`（mod.rs:461-473）v1 回退路径必须 `load_vectors` 反推 dim。

**核验消费方是否在 cheap-path 调 dim()**：grep `\.dim()` 全仓（src + benches + tests，排除测试）仅 4 处：
- `merge/mod.rs:177`（merge 路径，本需全量）
- `api/collection.rs:767,778`（search 路径，brute_search 参数，vectors() 已先求值）
- `api/reindex.rs:127`（reindex 路径，本需全量）

**无消费方在 open/restore 期 cheap-path 调 dim()**。`Db::open` + collection restore 不调 dim()。故 v1 corpus 冷启动 open 仍 <1s，冷启动语义不退化。首次 search 时 dim() 复用已加载 vectors（左到右求值），无重复读。

**判定**：不破坏冷启动。implementer 报告 §5 已文档化此 v1 固有限制，M2-08 迁移 v2 后消除。**次要**。

### 4.3 [M-2] M2-08 回归交接 — 清晰，无遗漏

**证据**：
- `load_vectors`（mod.rs:387-398）：`match version { v if v == FORMAT_VERSION => 8, 2 => 12, _ => Err }`，字面量 `2` 判别 v2，注释「VECTORS_FORMAT_V2 常量由 M2-08 落实，此处用字面量判别」。
- `dim()` v2 路径（mod.rs:456）：`if version == 2`，同样字面量。
- 报告 §2 明确交接：M2-08 落实 `VECTORS_FORMAT_V2` 常量 + finalize 写 v2 头后，替换字面量 + 回归 `build_v2_stub_segment` 切到真实 finalize 产物。

交接点清晰，mod.rs 注释 + 报告双标注。**次要**：建议 M2-08 落实时把字面量 `2` 替换为常量后跑一次本模块测试回归。

### 4.4 [M-3] dim() 读 v2 头的 read_at 错误吞没

**证据**：`dim()`（mod.rs:453）`self.vfs.read_at(&vpath, &mut hdr, 0).unwrap_or(0)` — read_at IO 错误被吞为 n=0，走 v1 回退。

**判定**：v1 回退路径会调 `load_vectors`，对 v2 文件也能正确算 dim（payload=file_len-12，dim=payload/4/doc_count），故功能正确。但 IO 错误再次被静默吞没，与 I-1 同类问题。**次要**，随 I-1 修复一并处理。

---

## 5. TDD 覆盖核验

| # | 计划测试 | 实装测试 | 状态 |
|---|---------|---------|------|
| 1 | open 不加载 vectors | `m2_07_open_does_not_load_vectors`（v2 stub，断言 `vectors.get().is_none()`） | ✅ |
| 2 | open <1s | `cold_start_gate`（ignored，手动/CI） | ✅ 自证 169ms |
| 3 | 首次 vectors() 触发加载 | `m2_07_vectors_lazy_load_and_idempotent` | ✅ |
| 4 | 首次查询 <3s | `cold_start_gate` 阶段 2 | ✅ 自证 1236ms |
| 5 | vectors() 幂等 | 同测试 3，指针相等 | ✅ |
| 6 | vectors() 并发安全 | `m2_07_vectors_concurrent_load_once`（`std::thread::scope` 双线程，指针相等） | ✅ |
| 7 | stored 懒加载 | `m2_07_stored_lazy_load` | ✅ |
| 8 | dim 正确性 v2 | `m2_07_dim_from_v2_header` | ✅ |
| 9 | dim 回退 v1 | `m2_07_dim_v1_fallback` + `m2_07_dim_before_vectors_v1` + `m2_07_vectors_before_dim_v1` | ✅ 覆盖两种调用顺序 |
| 10 | corpus 兼容 | `corpus_compat`（tests/corpus_compat.rs） | ✅ 自证绿 ⚠️ 无法从 diff 确认细节，信任自证 |
| 11 | reindex/merge 不破 | 无新增专用测试，依赖既有 reindex/merge 回归 | ⚠️ 自证全绿，信任 |
| 12 | HNSW search recall | 无新增专用测试，依赖既有 HNSW 回归 | ⚠️ 同上 |
| 13 | OnceLock 无新依赖 | import `std::sync::OnceLock` | ✅ |

**覆盖缺口**：
- **无 OnceLock 错误吞没行为测试**（vectors.bin corrupt 后 `vectors()` 返空 or panic？契约未锁定）——见 I-1。
- 测试 11/12 依赖既有回归，无新增懒加载专用断言——可接受（既有测试已覆盖正确性），但建议补一个 `m2_07_reindex_merge_lazy_load_path` 显式断言 reindex/merge 路径在懒加载下首次访问触发 load（`vectors.get().is_none()` before / `.is_some()` after）。

---

## 6. 代码质量

- **OnceLock 用法**：`get_or_init` 闭包内 `load_vectors`/`load_stored` 提取干净；`dim()` 闭包内 `let _ = self.vectors.set(v);` 正确处理并发下已被 set 的 Err 返回（忽略即可，另一线程已写入）。两 OnceLock（vectors/dim）独立，dim() 闭包内只 `get()`/`set()` vectors OnceLock 不调 `vectors()` 方法，无循环依赖/死锁。
- **load_vectors 提取**：v1/v2 分支清晰，payload_off 随 version 变化，truncated 校验到位（mod.rs:399）。
- **无残留全加载路径**：`open` 已移除 vectors/stored/dim 全加载，仅留 header+id_map。
- **dim() 闭包较长**（mod.rs:445-475，30 行），但逻辑分层清晰（doc_count==0 / v2 头 / v1 回退），可读性可接受。
- **cold_start_gate/bench 注释更新到位**，断言逻辑保留降级 fallback。

---

## 7. 发现汇总

| 级别 | ID | 发现 | 证据 |
|------|----|------|------|
| I | I-1 | OnceLock `unwrap_or_default()` 吞 IO 错误，vectors.bin/stored.bin 损坏从 open 期 Err 传播降级为访问期静默空返，错误可见性回归；implementer「与 M0/M1 一致」判定不准确。需补观测手段（panic/日志/health check）+ 契约测试。 | mod.rs:432-438, 496-512, 466-473；旧 open mod.rs:344-394（diff 删除段） |
| M | M-1 | v1 dim() 触发 vectors 加载——不破坏冷启动（无 cheap-path 调用），已文档化，M2-08 迁移 v2 后消除。 | mod.rs:461-473；grep dim() 调用点 4 处均在 search/reindex/merge |
| M | M-2 | v2 头判别用字面量 `2u32` 非 `VECTORS_FORMAT_V2` 常量——交接点清晰，M2-08 落实后替换+回归。 | mod.rs:387-398, 456 |
| M | M-3 | `dim()` 读 v2 头 `read_at(..).unwrap_or(0)` 吞 IO 错误——功能正确（v1 回退能算对 v2 dim），但同类静默吞没，随 I-1 一并处理。 | mod.rs:453 |

---

## 8. 建议

1. **I-1 修复**（M2-08 或后续小补丁）：`vectors`/`stored` 字段改 `OnceLock<Result<T, VaneError>>`，`vectors()` 在 Err 时 panic 带段 ulid 上下文；或补 `check_health()` 方法 + 日志。补契约测试锁定行为。
2. **M-1/M-2**：M2-08 落实 `VECTORS_FORMAT_V2` 常量 + finalize 写 v2 头后，跑本模块 v2 读路径回归，替换字面量。
3. **测试补强**：补 reindex/merge 懒加载路径专用断言 + vectors.bin corrupt 契约测试。

---

**状态**：PASS_WITH_FINDINGS
