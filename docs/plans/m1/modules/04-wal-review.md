# 04-wal 代码审查

> 审查对象：M1 模块 04-wal（薄 WAL 崩溃恢复，DoD 核心）。
> 基线 BASE=3bc094f → HEAD=0650e6f。diff：`git diff 3bc094f..HEAD -- crates/`（7 文件 +740/-21）。
> 审查方式：只读 diff + 代码审查，未运行 cargo（编排者集成门禁已确认 250 lib + 9 wal_crash 绿 + clippy/wasm32/fmt/no-std-fs/thin/bench 全过）。
> 审查日期：2026-08-09。

## 维度逐条结论

### 1. Wal/WalRecord 三变体 + append/read_all/truncate ✅

- `WalRecord` 三变体齐全（`wal/mod.rs:34-47`）：AddSegment / DeleteSegment / AddTombstone，字段与 README §04 契约一致（collection/ulid/docids）。
- `append`（`wal/mod.rs:65-72`）：serde_json 序列化 + `\n` + `Vfs::append` + `Vfs::sync`，JSON 行格式，每条 sync 落盘。经 Vfs，无 std::fs。
- `read_all`（`wal/mod.rs:75-101`）：循环 `read_at` 8KB 块拼全文件 → 按 `\n` split → 反序列化；文件不存在（`VaneError::Io`）返回空 Vec。正确。
- `truncate`（`wal/mod.rs:110-115`）：`Vfs::delete`（忽略不存在）+ `Vfs::create`（空文件）+ `Vfs::sync`。正确。
- `Wal::open`（`wal/mod.rs:57-62`）：幂等 `vfs.create`（best-effort，已存在忽略），追加语义保留。正确。
- 全程经 Vfs trait，core 禁 std::fs 守住（`check-no-std-fs.sh` 通过，grep 确认 wal/ 无 std::fs）。

### 2. B-2 闭环（核心）✅

**B-2 已闭环。** 证据：

- **flush 不调 truncate**：`api/collection.rs:411-415` flush 路径仅 `wal.append(AddSegment)` 后接 `manifest_store.add_segment`，无 `wal.truncate` 调用。grep 确认全 crate `wal.truncate()` 仅出现在 `run_compact`（`api/collection.rs:1036`）一处。
- **compact 是唯一 truncate 点**：`run_compact`（`api/collection.rs:1011-1038`）调 `merge_segments` 后 `wal.truncate()`。`merge_segments` 本身不 truncate（`api/collection.rs:558` 注释明示）。
- **auto_merge（partial）不 truncate**：`auto_merge_two_smallest` → `merge_segments`，无 truncate。reindex 也不 truncate（`api/collection.rs:1162` 注释）。WAL 累积到下次 compact——与 B-2 一致。
- **Task 5b 回归测试**：`tests/wal_crash.rs:187-211` `crash_after_flush_delete_flush_keeps_tombstone` 覆盖 `flush1(AddSegment a) → delete(AddTombstone a,d0) → flush2(AddSegment b) → 崩溃 → reopen`，断言 d0 仍被排除、d1 仍可见。B-2 核心回归绿。
- **无遗漏 truncate 路径**：全 crate grep `\.truncate()\?` 仅 `run_compact:1036` 一处（其余为 Vec::truncate / 文件 truncate 注释）。

B-2 闭环判定：**通过**。flush 不 truncate → WAL 累积 AddTombstone → reopen 重放注入 → tombstone 不丢。

### 3. recover 崩溃恢复 ✅

`wal/mod.rs:140-185`：

- **AddTombstone**（:150-170）：仅当 ULID 仍在 manifest 时聚合到 `RecoveredTombstones`（`ulid_in_manifest` 校验，:156）；roaring 存 u32，`d <= u32::MAX` 截断（与 delete 期 `abs as u32` 一致）。正确——段已被 compact/reindex 清除时 tombstone 无意义。
- **AddSegment 孤儿**（:171-177）：ULID 不在 manifest → `delete_segment_dir` 递归删 `db/segments/seg_<ULID>`。正确（manifest 切换前崩溃 → 半成品段清理）。错误被 `let _ =` 忽略（段目录不存在时 `list` 报错，尽力清理）。
- **DeleteSegment**（:178-181）：不动作。ULID 仍在 manifest → 合并未完成 → 旧段保留（恢复到合并前）；ULID 已不在 manifest → 已清除。两种情况均无需操作。正确。
- **Db::open 调 recover**：`api/db.rs:58` `recover(&vfs, path, &manifest)?`，在 `restore_from_manifest` 之前调用（先清理孤儿段目录，再 open manifest 段 reader）。tombstone 注入在 `restore_from_manifest` 之后、注册到 collections 之前（`api/db.rs:69-79`），无并发 search 可见窗口。正确。

**recover 正确性判定：通过。**

注意：DeleteSegment + AddSegment 在 `merge_segments` 中先于 manifest save_atomic append（`api/collection.rs:559-570`）。crash 在 manifest 切换前：DeleteSegment(old) 不动作（old 仍在 manifest，保留）、AddSegment(new) 孤儿清理（new 不在 manifest）。恢复到合并前状态。✅ 加上旧段原始 flush 期的 AddSegment(old) 记录仍在 WAL（未 truncate），crash 后 old 仍在 manifest → AddSegment(old) 不触发清理。一致。

### 4. WAL 接入 ✅

- **flush → AddSegment**：`api/collection.rs:411-415`，段文件 sync 后、manifest rename 前。✅
- **delete → AddTombstone**：`api/collection.rs:967-974`，先 append WAL 再更新内存位图（R-2 顺序偏离，crash 安全更优）。✅
- **compact → DeleteSegment(旧) + AddSegment(新) + truncate**：`merge_segments` 内 append 段增删（:559-569），`run_compact` 末尾 truncate（:1035-1036）。✅
- **reindex → AddSegment(新) + DeleteSegment(旧) + AddTombstone(新, re-key)**：`api/collection.rs:1163-1189`，manifest 切换前 append。reindex 不 truncate（tombstone 未物理清除）。✅

### 5. R-1 recover 签名偏离 ✅（layering 合理，文档待裁决）

- `recover` 返回 `Result<RecoveredTombstones>`（`wal/mod.rs:121,140`），非 README 契约的 `Result<()>`。
- **layering 合理**：wal 模块依赖 `persistence`（Manifest）、`types`、`vfs`、`roaring`（外部 crate），不依赖 `api` 模块。`CollectionInner` 在 api 内，若 wal 依赖 api 则成环。recover 返回聚合 tombstone map 由 `Db::open` 注入，是必要的 layering 偏离。
- `Db::open` 注入逻辑（`api/db.rs:69-79`）：双重保险再校验 `meta.segment_ulids.contains(ulid)` 且 `!bm.is_empty()`，merge 到 `CollectionInner.tombstones`。正确。
- ⚠️ **README §04 契约未更新**：`docs/plans/m1/README.md:333` 仍标注 `recover(...) -> Result<()>`。需编排者裁决是否更新契约签名（实装已文档化于 `wal/mod.rs:135-139` + `04-wal-report.md` R-1）。

### 6. R-3 reindex AddTombstone re-key ✅

- `api/collection.rs:1176-1189`：reindex 对每个有非空 tombstone 的旧段，append `AddTombstone(新 ULID, 绝对 docid)` 到 WAL。
- **合理性**：reindex 保留 tombstone（re-key 到新 ULID），若不重写 WAL，crash 后新 ULID 在 manifest 但 tombstone 仅内存 → 丢失（违反 I-6 精神）。这是「reindex 接入 WAL = reindex crash-safe」的必要组成。
- recover 验证：crash 在 manifest 切换前 → AddSegment(new) 孤儿清理 → AddTombstone(new) 因 new 不在 manifest 被跳过（一致）。crash 在 manifest 切换后 → new 在 manifest → AddTombstone 注入。✅
- 集成测试 `reindex_crash_keeps_tombstone_and_cleans_old_segments`（`tests/wal_crash.rs:267-309`）覆盖。✅

### 7. M-minor-1 Drop guard ✅

- `CompactingGuard`（`api/collection.rs:89-103`）：drop 时 `lock()` 复位 `compacting=false`（含 panic 路径）。
- guard 不持有锁——仅在 drop 时重新获取。与原显式 finally 等价但 panic-safe。
- compact（:1005）、reindex（:1089）均改用 guard。guard 在 acquire-and-set 作用域结束后创建（:998-1004 / :1080-1086），无 panic-during-lock-hold 问题。✅

### 8. M-minor-2 tombstone 语义 ✅

- `WalRecord::AddTombstone.docids` 存绝对 docid（字段 doc `wal/mod.rs:41`）。
- delete 期写入的是绝对 docid（`api/collection.rs:952-960` `abs = base + l`）；recover 注入直接用绝对 docid（`wal/mod.rs:166-168` `d as u32`）。
- 运行期 `CollectionInner.tombstones` 位图也是绝对 docid；filter/tombstone 统一在绝对空间。语义一致。✅
- 段内 local docid 仅在 SegmentReader 边界转换，WAL 不涉及。✅

### 9. 不变量 ✅

- **I-6（manifest 原子性 + WAL 一致）**：manifest 经 `save_atomic`（rename）切换；WAL 在切换前 append；recover 重放 tombstone + 清理孤儿段。`manifest_consistent_after_crash_mid_flush` 测试覆盖。✅
- **I-1（段不可变）**：delete 仅更新内存位图 + WAL，不改段文件。compact/reindex 重建新段而非原地改。✅
- **I-5（零 cfg）**：wal 模块无任何 `cfg`。wasm32 构建通过。✅

### 10. M0 签名零破坏 ✅

- `Vfs` trait、`ManifestStore`、`Collection` pub API（flush/delete/compact/reindex/search/add 等）签名均未变。
- 新增 `pub mod wal`（`lib.rs:14`）+ `Wal`/`WalRecord`/`recover`/`RecoveredTombstones` pub 类型。
- api 内部接入：`Db::open` 内部调 recover + 注入（非 pub API 变更）；`CollectionInner.tombstones` 字段从 `RwLock<HashMap<...>>` 改为 `pub(crate)`（`api/collection.rs:62`）供 `Db::open` 注入——这是内部结构，非 pub API 破坏。✅
- `tombstone_merge.rs` 测试 `tombstone_not_persisted_without_wal` 断言翻转（02 期「reopen 丢失」→ 04 期「reopen 保留」），注释更新。合理。✅

### 11. 范围合规 ✅

- 只做 04-wal + reindex 接入 + M-minor-1/M-minor-2。无越界功能。
- 无黑名单依赖（roaring 已是 workspace dep，serde_json 已有）。core 禁 std::fs 守住。
- wal 模块依赖图：persistence + types + vfs + roaring（外部），不依赖 api。无环。✅

### 12. 测试质量 ✅

9 个 wal_crash 集成测试（`tests/wal_crash.rs`）+ 5 个 wal 单测（`wal/tests.rs`）：

| 测试 | 覆盖 | 评价 |
|---|---|---|
| `crash_recovery_replays_tombstone` | Task 3 tombstone 重放 | ✅ 真实 |
| `crash_recovery_cleans_orphan_segment` | Task 4 孤儿段清理 | ✅ |
| `flush_appends_add_segment_does_not_truncate` | B-2 flush 不 truncate | ✅ |
| `delete_appends_tombstone_to_wal` | §7.2 即时进 WAL | ✅ |
| `compact_truncates_wal_after_manifest_switch` | B-2 compact truncate | ✅ |
| `crash_after_flush_delete_flush_keeps_tombstone` | B-2 核心回归 | ✅ 关键 |
| `manifest_consistent_after_crash_mid_flush` | I-6 | ✅ |
| `compact_then_reopen_no_tombstone_needed` | compact 后无需 tombstone | ✅ 补充 |
| `reindex_crash_keeps_tombstone_and_cleans_old_segments` | reindex crash + 旧段孤儿 | ✅ |

覆盖 B-2 / reindex crash / orphan / I-6 端到端。真实断言（搜索结果 + 段数 + 文件列表）。

## 疑点与建议（非阻塞）

### S1（minor）：孤儿段清理仅限 WAL 记录的 ULID，未扫描 segments/ 目录

SPEC §6.4.2「半成品 segment 文件按 ULID 不在 manifest 中即判定垃圾，启动时清理」。实装 recover 仅对 WAL 中 AddSegment 记录的 ULID 做孤儿判定（`wal/mod.rs:171-177`），不扫描 `db/segments/` 目录。

**缺口场景**：flush 写完段文件 → `wal.append(AddSegment)` 失败（IO 错误）→ flush 返回 Err，段文件留在磁盘但无 WAL 记录、无 manifest 引用。reopen 时 recover 看不到该 ULID → 段目录泄漏（非数据损坏，仅磁盘空间）。

**影响**：低。仅在 WAL append IO 失败时触发，M1 不承诺滚动/上限（report 遗留 #4）。但严格按 SPEC 字面，recover 应扫描 `segments/` 目录清理任何 `seg_*` 不在 manifest 的目录。

**建议**：编排者裁决是否在 M1 补「扫描 segments/ 目录」清理路径，或文档化为已知限制留 M2。

### S2（minor）：缺 mid-merge crash 恢复测试

现有测试覆盖 mid-flush crash（`manifest_consistent_after_crash_mid_flush`）和 reindex crash，但无 mid-merge / mid-compact crash 测试（WAL 有 DeleteSegment + AddSegment 但 manifest 未切换 → 旧段保留、新段孤儿清理）。

recover 逻辑对该场景处理正确（分析见维度 3），但缺直接集成测试。建议后续补充。

### S3（minor）：R-1 README 契约签名未更新

`docs/plans/m1/README.md:333` 仍标注 `recover(...) -> Result<()>`，实装为 `Result<RecoveredTombstones>`。需编排者裁决是否更新 README 契约（实装偏离已文档化于 `wal/mod.rs:135-139` + report R-1）。

### S4（trivial）：wal 模块直接依赖 roaring

`RecoveredTombstones = HashMap<String, HashMap<String, roaring::RoaringBitmap>>`（`wal/mod.rs:121`）使 wal 模块 public 类型绑定 roaring。替代方案是返回 `Vec<u64>` 由 api 层转换。当前做法务实（roaring 已是 core 依赖，非新增），不构成环。仅记录，无需改。

## 结论

**Verdict：APPROVED_WITH_MINOR**

B-2 闭环通过（核心 DoD 满足），recover 正确性通过，无环依赖，M0 签名零破坏，范围合规。3 个 minor 疑点（S1 孤儿扫描缺口 / S2 mid-merge 测试缺失 / S3 README 契约签名）均非阻塞，留编排者裁决。

## 需编排者裁决疑点

1. **S1**：recover 是否需扫描 `segments/` 目录清理无 WAL 记录的孤儿段（SPEC §6.4.2 字面要求）？还是文档化为已知限制留 M2？
2. **S3/R-1**：README §04 契约 `recover` 签名是否更新为 `Result<RecoveredTombstones>`？
3. **R-3**：reindex AddTombstone re-key 写 WAL（超出计划字面但为 crash-safe 必要）——确认保留？（report 已提请裁决）
