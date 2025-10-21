# 04-wal：薄 WAL 元操作日志 + 崩溃恢复

> SPEC 引用：§6.4（写入与崩溃恢复）、§6.2（wal.log 布局）、§7.2（tombstone 即时进 WAL）。
> 前置依赖：M0 `persistence`/`vfs`/`segment`；02-tombstone-merge（WalRecord 语义对齐 delete/merge）。
> M1 README 契约：`vane_core::wal`。

## Goal

薄 WAL：仅段增删/tombstone 元操作日志（SPEC §6.4）。`Wal::append` 在 manifest 原子切换前持久化元操作；`recover` 在 open 时重放未提交的 tombstone/段增删；半成品 segment（ULID 不在 manifest）判定垃圾并清除。

## Architecture

- **WAL 文件**：`<db>/wal.log`，JSON 行格式（每行一条 `WalRecord`，serde_json 序列化）。append 后 sync（保证崩溃前落盘）。
- **记录类型**（`WalRecord`）：AddSegment / DeleteSegment / AddTombstone。
- **写入流程**（对齐 SPEC §6.4）：flush 构建新 segment → sync 段文件 → **WAL append AddSegment** → manifest 临时文件 → sync → rename。delete → **WAL append AddTombstone**（即时）。compact/merge 完成 → **WAL append DeleteSegment(旧)** + AddSegment(新) → manifest 切换。
- **崩溃恢复**：`Db::open` → load manifest → read_all WAL → 重放：
  - AddTombstone：恢复内存 tombstone（段未被合并清除前）。
  - AddSegment：若 ULID 不在 manifest → 半成品，删除（manifest rename 前崩溃，WAL 有记录但 manifest 未切换）。
  - DeleteSegment：若 ULID 仍在 manifest → 合并未完成，旧段保留（恢复到合并前状态）。
- **截断**（B-2 修复）：**flush 路径不调 `Wal::truncate`**——否则 `flush→delete→flush→崩溃` 会丢失未消费的 AddTombstone（tombstone 仅存 WAL，02 不改 header.bin），致已删文档复活（数据损坏）。**仅 compact/merge 成功 + manifest 切换后调 `truncate`**（此时 AddTombstone 随旧段物理清除）。WAL 累积 AddSegment 记录直到 compact（ULID 字符串体积可忽略），compact 后一次性清空。
- **WASM 适配**：WAL 同步 IO（Vfs::append/sync），WASM Worker 内 SyncAccessHandle 同步写入，写间隙小步。

## 涉及文件

- **Create**：
  - `crates/vane-core/src/wal/mod.rs`（Wal / WalRecord / recover）
  - `crates/vane-core/src/wal/tests.rs`
- **Modify**：
  - `crates/vane-core/src/lib.rs`（增 `pub mod wal;`）
  - `crates/vane-core/src/api/db.rs`（open 流程调 recover）
  - `crates/vane-core/src/api/collection.rs`（flush 后 WAL append AddSegment；delete 后 WAL append AddTombstone；compact 后 WAL append DeleteSegment+AddSegment；manifest 切换后 truncate）
- **Test**：
  - `crates/vane-core/src/wal/tests.rs`
  - `crates/vane-core/tests/wal_crash.rs`（集成崩溃恢复）

## Interfaces

### Consumes from M0（已核查 git HEAD）

```rust
// crates/vane-core/src/vfs/mod.rs（M0 冻结）
pub trait Vfs: Send + Sync {
    fn create(&self, path: &str) -> Result<()>;
    fn read_at(&self, path: &str, buf: &mut [u8], offset: u64) -> Result<usize>;
    fn append(&self, path: &str, buf: &[u8]) -> Result<u64>;  // 返回写入起始 offset
    fn sync(&self, path: &str) -> Result<()>;
    fn rename(&self, from: &str, to: &str) -> Result<()>;
    fn delete(&self, path: &str) -> Result<()>;
    fn list(&self, dir: &str) -> Result<Vec<String>>;
}
// crates/vane-core/src/persistence/mod.rs
pub struct Manifest { pub version: u32, pub collections: HashMap<String, CollectionMeta> }
pub struct ManifestStore { ... }
impl ManifestStore {
    pub fn load(&self) -> Result<Option<Manifest>>;
    pub fn save_atomic(&self, manifest: &Manifest) -> Result<()>;
    pub fn add_segment(&self, collection: &str, ulid: &str) -> Result<()>;
}
pub struct CollectionMeta { pub segment_ulids: Vec<String>, ... }
```

### Consumes from 02-tombstone-merge

`CollectionInner.tombstones`（delete 产出，WAL 记录 AddTombstone 对应）；`MergeTask`（合并完成时 WAL 记录段增删）。

### Produces（见 README § 04-wal 契约）

## TDD 任务清单

### Task 1：Wal append + read_all roundtrip

**测试**（`crates/vane-core/src/wal/tests.rs`）：
```rust
use super::*;
use crate::vfs::memory::MemoryVfs;
use std::sync::Arc;

#[test]
fn wal_append_read_roundtrip() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn crate::vfs::Vfs>;
    let wal = Wal::open(vfs, "db").unwrap();
    wal.append(&WalRecord::AddSegment { collection: "c".into(), ulid: "seg_001".into() }).unwrap();
    wal.append(&WalRecord::AddTombstone { collection: "c".into(), ulid: "seg_001".into(), docids: vec![1, 3] }).unwrap();
    let records = wal.read_all().unwrap();
    assert_eq!(records.len(), 2);
    assert!(matches!(records[0], WalRecord::AddSegment { ref ulid, .. } if ulid == "seg_001"));
    assert!(matches!(records[1], WalRecord::AddTombstone { ref docids, .. } if docids == &vec![1, 3]));
}
```
验证失败：Wal 类型不存在。
最小实现：`Wal::open` 记录 vfs + path（`db/wal.log`）；`append` = serde_json::to_vec + `Vfs::append` + `Vfs::sync`（每行尾加 `\n`）；`read_all` = 循环 read_at 全文件 + 按行 split + serde_json 反序列化。
commit：`wal: implement append/read_all roundtrip`。

### Task 2：truncate 清空（仅 compact 调用，flush 不调）

**测试**：
```rust
#[test]
fn wal_truncate_clears_records() {
    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn crate::vfs::Vfs>;
    let wal = Wal::open(vfs, "db").unwrap();
    wal.append(&WalRecord::AddSegment { collection: "c".into(), ulid: "seg_001".into() }).unwrap();
    wal.truncate().unwrap();
    let records = wal.read_all().unwrap();
    assert!(records.is_empty());
}
```
最小实现：`truncate` = `Vfs::create`（重置空文件）+ `Vfs::sync`。或 `Vfs::delete` + `Vfs::create`。
**B-2 纪律**：`truncate` **仅由 compact/merge 成功 + manifest 切换后调用**；flush 路径绝不调 truncate（见 Task 5）。调用方约束以注释 + 文档强约束，非类型强制。
commit：`wal: implement truncate (compact-only, never on flush — B-2)`。

### Task 3：崩溃恢复 — tombstone 重放

**测试**（`crates/vane-core/tests/wal_crash.rs`）：
```rust
use vane_core::api::*;
use vane_core::types::*;
use vane_core::vfs::memory::MemoryVfs;
use std::sync::Arc;

#[test]
fn crash_recovery_replays_tombstone() {
    let vfs = Arc::new(MemoryVfs::new());
    // 会话 1：灌库 + flush + delete（WAL 记录 tombstone），不 truncate（模拟崩溃前）
    {
        let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
        let col = setup_col(&db);
        col.add(&docs()).unwrap(); col.flush().unwrap();
        col.delete(&["d1".into()]).unwrap();
        // 不调 col.close()（模拟崩溃），WAL 未 truncate
    }
    // 会话 2：reopen，WAL 重放恢复 tombstone
    let db2 = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col2 = db2.collection("c", schema(), CollectionOptions::default()).unwrap();
    let hits = col2.search(&SearchQuery { text: Some("hello".into()), top_k: 10, mode: SearchMode::Text, ..Default::default() }).unwrap();
    assert!(!hits.iter().any(|h| h.id == "d1"), "tombstone must be replayed");
}
```
最小实现：`Db::open` 加载 manifest 后调 `wal::recover(vfs, db_path, &manifest)`。recover：read_all WAL → AddTombstone 记录按 (collection, ulid) 聚合 → 注入对应 CollectionInner.tombstones（reopen 时 restore_from_manifest 后注入）。AddSegment 若 ULID 不在 manifest → Vfs::delete 段目录（半成品）。DeleteSegment 若 ULID 仍在 manifest → 忽略（合并未完成，保留旧段）。
commit：`wal: implement crash recovery with tombstone replay`。

### Task 4：崩溃恢复 — 半成品段清理

**测试**：
```rust
#[test]
fn crash_recovery_cleans_orphan_segment() {
    let vfs = Arc::new(MemoryVfs::new());
    {
        let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
        let col = setup_col(&db);
        col.add(&docs()).unwrap(); col.flush().unwrap();
        // 模拟：写一个半成品段目录（不在 manifest）+ WAL 有 AddSegment
        let wal = vane_core::wal::Wal::open(vfs.clone(), "db").unwrap();
        wal.append(&vane_core::wal::WalRecord::AddSegment { collection: "c".into(), ulid: "seg_ORPHAN".into() }).unwrap();
        vfs.create("db/segments/seg_ORPHAN").unwrap();
        vfs.write_at("db/segments/seg_ORPHAN/header.bin", b"partial", 0).unwrap();
    }
    let db2 = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
    // 孤儿段已清理
    let files = vfs.list("db/segments").unwrap();
    assert!(!files.iter().any(|f| f.contains("ORPHAN")));
}
```
最小实现：recover 中 AddSegment 记录的 ULID 若不在 manifest.collections[c].segment_ulids → `Vfs::delete("db/segments/seg_<ULID>")`（递归删段目录——Vfs::list 列文件后逐个 delete，或约定段目录删除原语）。
commit：`wal: clean orphan segments on recovery`。

### Task 5：flush/delete/compact 接入 WAL append（B-2：flush 不 truncate）

**测试**：
```rust
#[test]
fn flush_appends_add_segment_does_not_truncate() {
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
    let col = setup_col(&db);
    col.add(&docs()).unwrap(); col.flush().unwrap();
    let wal = vane_core::wal::Wal::open(vfs.clone(), "db").unwrap();
    let records = wal.read_all().unwrap();
    // B-2：flush 后 WAL **不** truncate，AddSegment 保留直到 compact。
    // （否则 flush→delete→flush→崩溃 会丢失 AddTombstone。）
    assert!(records.iter().any(|r| matches!(r, vane_core::wal::WalRecord::AddSegment { .. })),
            "flush must NOT truncate WAL (B-2)");
}

#[test]
fn delete_appends_tombstone_to_wal() {
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
    let col = setup_col(&db);
    col.add(&docs()).unwrap(); col.flush().unwrap();
    col.delete(&["d1".into()]).unwrap();
    let wal = vane_core::wal::Wal::open(vfs.clone(), "db").unwrap();
    let records = wal.read_all().unwrap();
    assert!(records.iter().any(|r| matches!(r, vane_core::wal::WalRecord::AddTombstone { .. })));
}

#[test]
fn compact_truncates_wal_after_manifest_switch() {
    let vfs = Arc::new(MemoryVfs::new());
    let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
    let col = setup_col(&db);
    col.add(&docs()).unwrap(); col.flush().unwrap();
    col.delete(&["d1".into()]).unwrap();
    col.compact().unwrap();  // compact 成功 + manifest 切换 → truncate
    let wal = vane_core::wal::Wal::open(vfs.clone(), "db").unwrap();
    let records = wal.read_all().unwrap();
    assert!(records.is_empty(), "WAL must be truncated after compact (B-2)");
}
```
最小实现：
- `Collection::flush`：sync 段 → `wal.append(AddSegment)` → `manifest_store.add_segment`（save_atomic）→ **不调 `wal.truncate`**（B-2）。
- `Collection::delete`：追加内存 tombstone 后 → `wal.append(AddTombstone)`（不 truncate）。
- `Collection::compact`：MergeTask 完成后 → `wal.append(DeleteSegment 旧)` + `wal.append(AddSegment 新)` → manifest 切换 → **`wal.truncate`**（compact 是唯一 truncate 调用点）。
commit：`api: wire WAL append (flush no-truncate, compact truncates — B-2)`。

### Task 5b：崩溃恢复 — flush→delete→flush→崩溃 不丢 tombstone（B-2 回归）

**测试**（`crates/vane-core/tests/wal_crash.rs` 扩展）：
```rust
#[test]
fn crash_after_flush_delete_flush_keeps_tombstone() {
    // B-2 核心回归：flush 不 truncate，否则此序列丢 tombstone。
    let vfs = Arc::new(MemoryVfs::new());
    {
        let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
        let col = setup_col(&db);
        col.add(&docs_batch(0)).unwrap(); col.flush().unwrap();       // flush1: AddSegment(seg_a)
        col.delete(&["d0".into()]).unwrap();                          // AddTombstone(seg_a, d0)
        col.add(&docs_batch(1)).unwrap(); col.flush().unwrap();       // flush2: AddSegment(seg_b)
        // 不 close（模拟崩溃）。flush 不 truncate → WAL 含 [AddSegment(a), AddTombstone(a,d0), AddSegment(b)]。
    }
    let db2 = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col2 = db2.collection("c", schema(), CollectionOptions::default()).unwrap();
    let hits = col2.search(&SearchQuery { text: Some("hello".into()), top_k: 100, mode: SearchMode::Text, ..Default::default() }).unwrap();
    assert!(!hits.iter().any(|h| h.id == "d0"), "tombstone must survive (B-2: flush no-truncate)");
    // d1（batch1）仍可见
    assert!(hits.iter().any(|h| h.id == "d1"));
}
```
commit：`wal: regression test for B-2 (flush-delete-flush-crash keeps tombstone)`。

### Task 6：不变量 I-6（manifest 原子性 + WAL 一致）

**测试**：
```rust
#[test]
fn manifest_consistent_after_crash_mid_flush() {
    // 模拟 manifest rename 前崩溃：WAL 有 AddSegment，manifest 旧（无新段）
    // reopen 后：新段为孤儿被清理，manifest 旧状态完整
    let vfs = Arc::new(MemoryVfs::new());
    {
        let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
        let col = setup_col(&db);
        col.add(&docs()).unwrap();
        // 手动写半成品段 + WAL，不切 manifest
        let wal = vane_core::wal::Wal::open(vfs.clone(), "db").unwrap();
        wal.append(&vane_core::wal::WalRecord::AddSegment { collection: "c".into(), ulid: "seg_HALF".into() }).unwrap();
        vfs.create("db/segments/seg_HALF/header.bin").unwrap();
    }
    let db2 = Db::open(vfs, "db", OpenOptions::default()).unwrap();
    let col2 = db2.collection("c", schema(), CollectionOptions::default()).unwrap();
    // 旧状态：无文档可见（flush 未完成）
    assert_eq!(col2.segment_count(), 0);
}
```
commit：`wal: assert manifest atomicity with WAL (I-6)`。

## 验收标准

- **SPEC §6.4**：flush 流程 = sync 段 → WAL append AddSegment → manifest rename（**B-2：flush 不 truncate**）；compact 流程 = WAL append 段增删 → manifest 切换 → truncate；崩溃恢复重放未提交元操作；半成品段清理。
- **SPEC §7.2**：delete tombstone 即时进 WAL。
- **不变量 I-6**：任何崩溃后 manifest 指向完整状态；孤儿段安全清理（Task 4/6）；**flush 不 truncate 保证 tombstone 不丢（Task 5b B-2 回归）**。
- **WASM 适配**：WAL 同步 IO 经 Vfs，WASM Worker 内同步（无异步化）。

## 前置依赖

- M0 persistence/vfs/segment（已合并）。
- 02-tombstone-merge（delete/compact 产出 WAL 记录，Task 5 依赖 delete/compact 实装——若 02 未完成，Task 5 可拆为 flush 部分先行，delete/compact 部分后置）。

## Global Constraints

core 禁 std::fs（WAL 经 Vfs trait）；WAL 文件 wal.log 在 `<db>/` 根（SPEC §6.2）；JSON 行格式（简单可读，无新依赖）；manifest 原子切换不变（I-6）；WAL 同步 append+sync（崩溃前落盘）。
