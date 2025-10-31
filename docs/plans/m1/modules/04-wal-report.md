# 04-wal 实装报告

> 模块：薄 WAL 元操作日志 + 崩溃恢复（SPEC §6.4/§6.2/§7.2，不变量 I-6，B-2 修复）。
> 状态：**完成**，自证门禁全绿。

## Task 改动

### Task 1：Wal append + read_all roundtrip
- 新建 `crates/vane-core/src/wal/mod.rs`：`Wal`/`WalRecord`（AddSegment/DeleteSegment/AddTombstone）。
- `Wal::open`：幂等 create `<db>/wal.log`（已存在则保留，追加语义）。
- `append`：serde_json 序列化 + `\n` + `Vfs::append` + `Vfs::sync`。
- `read_all`：循环 read_at 全文件 → 按 `\n` split → serde_json 反序列化（文件不存在返回空）。
- 单测：`wal_append_read_roundtrip` / `wal_open_is_idempotent` / `wal_read_all_empty_for_new_db`。

### Task 2：truncate（仅 compact 调，flush 不调）
- `Wal::truncate`：`Vfs::delete`（忽略不存在）+ `Vfs::create`（空文件）+ `Vfs::sync`。
- B-2 纪律以 doc + 注释强约束（非类型强制）：`truncate` 文档明确「仅 compact/merge 成功 + manifest 切换后调用」。
- 单测：`wal_truncate_clears_records` / `wal_truncate_then_append_works`。

### Task 3：崩溃恢复 tombstone 重放
- `wal::recover(vfs, db_path, manifest)`：read_all WAL → AddTombstone 按 (collection, ulid) 聚合为 `RecoveredTombstones` map（绝对 docid）；AddSegment 孤儿段清理；DeleteSegment 不动作。
- `Db::open`：加载 manifest 后调 `recover`，将聚合的 tombstone 注入各 `CollectionInner.tombstones`（仅对 manifest 中仍存在的 ULID，双重保险）。
- 集成测试：`crash_recovery_replays_tombstone`。

### Task 4：半成品段清理
- recover 中 AddSegment 的 ULID 不在 `manifest.collections[c].segment_ulids` → `merge::delete_segment_dir` 递归删 `db/segments/seg_<ULID>`。
- 集成测试：`crash_recovery_cleans_orphan_segment`。

### Task 5：flush/delete/compact 接入 WAL（B-2：flush 不 truncate）
- **flush**：段文件集全部 sync 后 → `wal.append(AddSegment)` → `manifest_store.add_segment`（manifest rename）。**不调 truncate**（B-2）。
- **delete**：先计算 (ulid, abs_docid) 对 → `wal.append(AddTombstone)`（SPEC §7.2 即时进 WAL）→ 再更新内存位图。crash 在 WAL 后位图前 → reopen 重放注入；crash 在 WAL 前 → 位图也未改，一致。count 仅记 newly inserted（与 02 语义一致）。
- **compact**：`merge_segments` 在 manifest 切换前 `wal.append(DeleteSegment 旧)` + `wal.append(AddSegment 新)`；`run_compact` 在 merge_segments 成功后调 `wal.truncate`（**唯一 truncate 调用点**，B-2）。
- 集成测试：`flush_appends_add_segment_does_not_truncate` / `delete_appends_tombstone_to_wal` / `compact_truncates_wal_after_manifest_switch`。
- **副作用**：02 期测试 `tombstone_not_persisted_without_wal` 原断言「reopen 后 tombstone 丢失（02 预期）」已翻转——04 接入后 reopen 保留 tombstone（d0 仍被排除）。测试注释更新。

### Task 5b：B-2 回归（flush→delete→flush→崩溃 不丢 tombstone）
- 集成测试 `crash_after_flush_delete_flush_keeps_tombstone`：flush1(AddSegment a) → delete(AddTombstone a,d0) → flush2(AddSegment b) → 崩溃。reopen 后 d0 仍被排除，d1 仍可见。**B-2 核心回归绿**。

### Task 6：I-6（manifest 原子性 + WAL 一致）
- 集成测试 `manifest_consistent_after_crash_mid_flush`：WAL 有 AddSegment 但 manifest 未切换 → reopen 后孤儿段清理，segment_count==0，搜索空。

## reindex WAL 接入（06 遗留 #1）
- `run_reindex` 在 manifest 切换前 append WAL：
  - `AddSegment(新段)` × N —— crash 在 manifest 前 → 孤儿清理。
  - `DeleteSegment(旧段)` × N —— 信息记录（recover 不动作）。
  - `AddTombstone(新 ULID, 绝对 docid)` —— **关键**：reindex 保留 tombstone（re-key 到新 ULID），需重新记录到 WAL，否则 crash 后新 ULID 在 manifest 但 tombstone 仅内存 → 丢失。docid 顺序不变 → 位图原值（绝对 docid）对新段同样有效。
- reindex **不 truncate**：tombstone 未物理清除（与 compact 区分），WAL 累积到下次 compact。
- 集成测试 `reindex_crash_keeps_tombstone_and_cleans_old_segments`：reindex + crash 后 tombstone 存活、旧段孤儿清理、仅 1 段。

## M-minor-2：tombstone abs/local 语义（02 遗留）
- `WalRecord::AddTombstone.docids` 存**绝对 docid**（与运行期 `CollectionInner.tombstones` 位图一致——delete 期写入的也是绝对 docid；filter/tombstone 运行期统一在绝对空间）。
- `recover` 注入时直接用绝对 docid（roaring 存 u32，故 u64 截断到 u32，与 delete 期 `abs as u32` 一致）。
- 段内 local docid 仅在 SegmentReader 边界处由 `docid_base` 转换，WAL 不涉及 local 语义。
- 文档化于 `wal/mod.rs` 模块 doc + `WalRecord::AddTombstone` 字段 doc。

## M-minor-1：compacting 标志 panic-safe Drop guard（02 遗留，可选 → 已实装）
- `CompactingGuard` 在 drop 时复位 `compacting` 标志（含 panic 路径），避免一次 panic 致永久 E_BUSY。
- `compact`/`reindex` 改用 guard 替代显式 finally 赋值。guard 不持有锁——仅在 drop 时重新获取锁复位（与原显式 finally 模式等价，但 panic-safe）。

## 偏离与裁决

### R-1：`recover` 返回类型偏离 README 契约
- README § 04-wal 契约标注 `recover(...) -> Result<()>`，实装返回 `Result<RecoveredTombstones>`（`HashMap<collection, HashMap<ulid, RoaringBitmap>>`）。
- **原因**：wal 模块不可依赖 api 模块（`CollectionInner` 在 api 内，反向依赖形成环）。故 recover 返回聚合的 tombstone map，由 `Db::open` 注入。这是 layering 必要偏离。
- **影响**：仅签名扩展（返回值携带 tombstone），语义不变；recover 仍执行孤儿段清理副作用。

### R-2：delete 的 WAL append 顺序
- 计划 Task 5 描述「追加内存 tombstone 后 → wal.append(AddTombstone)」。实装改为「先 wal.append → 后更新内存位图」。
- **原因**：SPEC §7.2「即时进 WAL」+ crash 安全——crash 在 WAL 后位图前 → reopen 重放注入；crash 在 WAL 前 → 位图也未改，一致。原顺序（位图先 → WAL 后）crash 在中间会丢 tombstone。
- count 仍仅记 newly inserted（位图 mutation 阶段计算）。

### R-3：reindex tombstone re-key 写 WAL（超出计划字面，属「reindex 接入」必要组成）
- 计划编排者补充仅字面要求「AddSegment 新段 + DeleteSegment 旧段」。实装额外 append `AddTombstone(新 ULID, 绝对 docid)`。
- **原因**：reindex 保留 tombstone（re-key 到新 ULID），若不重写 WAL，crash 后新 ULID 在 manifest 但 tombstone 仅内存 → 丢失（违反 I-6 精神）。这是「reindex 接入 WAL」即「使 reindex crash-safe」的必要组成，非新功能。
- **若编排者认为越界**：可移除 AddTombstone re-key 写入，但 reindex+crash 会丢 tombstone（需文档化为已知限制）。

## 自证门禁（全绿）

```
cargo test --workspace --all-features       # 250 lib + 集成全绿（含 wal::tests 5 + wal_crash 9）
cargo clippy --workspace --all-targets --all-features -- -D warnings   # 零告警
cargo check --target wasm32-unknown-unknown -p vane-core                # 零 cfg
cargo fmt --all -- --check                                              # 绿
bash scripts/check-no-std-fs.sh                                         # OK（WAL 经 Vfs）
bash crates/vane-node/scripts/check-thin.sh                             # OK
cargo bench --no-run -p vane-core                                       # 编译绿
```

## 提交 hash
- `5e34ab4` wal: implement append/read_all/truncate (Task 1+2)
- `20cf257` wal: 接入 flush/delete/compact + Db::open recover (Task 3/4/5/6)
- `59d335f` wal: 集成崩溃恢复测试 + B-2 回归 (Task 5b)
- `f722598` wal: reindex 接入 WAL (06 遗留 #1) + reindex crash 测试
- `639fb8c` wal: 修正 Task 6 测试 ULID 命名 + fmt
- `cbd44eb` api: M-minor-1 compacting 标志 panic-safe Drop guard

## 遗留 / 疑问

1. **R-1（recover 返回类型）**：返回 `RecoveredTombstones` 而非 `Result<()>`，layering 必要偏离。请编排者确认是否需更新 README § 04-wal 契约签名。
2. **R-3（reindex AddTombstone re-key 写 WAL）**：超出计划字面，但为 reindex crash-safe 必要组成。请编排者裁决是否保留。
3. **partial auto-merge 不 truncate**：B-2 要求「仅 compact truncate」。partial auto-merge（`auto_merge_two_smallest`）经 `merge_segments` append WAL 但**不 truncate**（tombstone for 保留段必须存活）。WAL 累积到下次 compact。这与 B-2 一致，但若长期不 compact，WAL 会累积——可观测指标留待后续。
4. **WAL 无尺寸上限/滚动**：薄 WAL 设计，依赖 compact 清空。极端场景（海量 flush 无 compact）WAL 可能增长。M1 不引入滚动（SPEC 未要求），留 M2。
