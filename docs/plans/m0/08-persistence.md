# Persistence 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development 执行本计划。步骤用 checkbox `- [ ]` 标记。

**Goal:** 实现 Manifest 原子读写（SPEC §6.4）+ AutoCommitter（计数+时间双触发）+ Db open 加载流程，为 api-core 提供崩溃安全的持久化原语。
**Architecture:** ManifestStore 封装 manifest.json 的加载与原子保存（临时文件 → sync → rename，不变量 I-6）。AutoCommitter 是无状态计数器+时间戳，由 api-core 在 add 路径查询。manifest.json 用 serde_json 序列化。本模块不直接读写段内容（段由 04-segment-format 的 SegmentWriter 产出），只管 manifest 指针。
**Tech Stack:** serde + serde_json。
**SPEC 引用:** §6.2 manifest.json 结构、§6.4 写入与崩溃恢复、§7.1 auto-commit（intervalMs=1000/maxDocs=1000）、§14 I-6 manifest 原子性。
**前置依赖:** 00-workspace（Schema, TokenizerId, Result, VaneError）、01-vfs（Vfs trait）、02-tokenizer（BuiltinTokenizer, UserDictEntry）。
**验收标准:**
- [ ] manifest 原子切换：save_atomic 期间崩溃（模拟）后 manifest 指向完整状态（不变量 I-6）
- [ ] load 不存在的 manifest 返回 Ok(None)（新库）
- [ ] load 损坏的 manifest 返回 Err(Corrupt)
- [ ] AutoCommitter 默认 On{1000, 1000}；record_docs 累加；should_flush 在 max_docs 或 interval_ms 触发
- [ ] rename 覆盖已有 manifest 不损坏旧数据

## Global Constraints
- manifest 原子切换唯一原语：临时文件 → sync → rename（SPEC §6.4/§3.5）。
- 崩溃恢复：manifest 永远指向最后一个完整状态；孤儿段文件按 ULID 不在 manifest 中即垃圾（§6.4/不变量 I-6）。
- auto-commit 默认开启：intervalMs=1000 或 maxDocs=1000 先到先触发（§7.1）。
- core 禁 std::fs（§13.3）；本模块通过 Vfs trait 读写。
- 段数上限 10（§3.3）；超限强制合并是 M1，M0 仅记录。

## File Structure
- `crates/vane-core/src/persistence/mod.rs` — Manifest + CollectionMeta + ManifestStore + AutoCommitter + re-export
- `crates/vane-core/src/persistence/tests.rs` — 原子性 + AutoCommitter 测试

## 任务清单（bite-sized TDD）

### Task 1: Manifest + CollectionMeta + 序列化
**Files:**
- Create: `crates/vane-core/src/persistence/mod.rs`, `crates/vane-core/src/persistence/tests.rs`
- Modify: 无（00-workspace 已在 lib.rs 一次性预声明全部 9 模块（含 `pub mod persistence;`），本计划不修改 lib.rs（B1 裁决）。）

**Interfaces:**
- Consumes from 00-workspace: Schema, TokenizerId, Result, VaneError
- Consumes from 02-tokenizer: BuiltinTokenizer, UserDictEntry
- Produces: `Manifest`, `CollectionMeta`, `Manifest::empty()`

- [ ] **Step 1: 写失败测试** — 创建 `crates/vane-core/src/persistence/tests.rs`：
```rust
use super::*;
use crate::types::{Schema, FieldDef, Metric, TokenizerId};
use crate::tokenizer::{BuiltinTokenizer, UserDictEntry};

#[test]
fn manifest_empty_serialize_roundtrip() {
    let m = Manifest::empty();
    let json = serde_json::to_string(&m).unwrap();
    let back: Manifest = serde_json::from_str(&json).unwrap();
    assert_eq!(back.version, 1);
    assert!(back.collections.is_empty());
}

#[test]
fn manifest_with_collection_roundtrip() {
    let schema = Schema::new(vec![
        ("body".into(), FieldDef::Text),
        ("v".into(), FieldDef::Vector { dim: 384, metric: Metric::Cosine }),
    ]).unwrap();
    let mut m = Manifest::empty();
    m.collections.insert("docs".into(), CollectionMeta {
        schema,
        tokenizer_kind: BuiltinTokenizer::Standard,
        tokenizer_id: TokenizerId([0xab; 32]),
        user_dict: vec![UserDictEntry::Word("test".into())],
        segment_ulids: vec!["01HZX...".into()],
    });
    let json = serde_json::to_string(&m).unwrap();
    let back: Manifest = serde_json::from_str(&json).unwrap();
    assert_eq!(back.collections.len(), 1);
    let col = &back.collections["docs"];
    assert_eq!(col.tokenizer_kind, BuiltinTokenizer::Standard);
    assert_eq!(col.segment_ulids, vec!["01HZX...".to_string()]);
}
```

- [ ] **Step 2: 跑测试确认失败** — `cargo test -p vane-core -- persistence`，编译失败。
- [ ] **Step 3: 最小实现** — `crates/vane-core/src/persistence/mod.rs`：
```rust
use crate::types::{Schema, TokenizerId};
use crate::tokenizer::{BuiltinTokenizer, UserDictEntry};
use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests;

/// SPEC §6.2 manifest.json 结构。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub collections: std::collections::HashMap<String, CollectionMeta>,
}

impl Manifest {
    pub fn empty() -> Self {
        Self { version: 1, collections: std::collections::HashMap::new() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionMeta {
    pub schema: Schema,
    pub tokenizer_kind: BuiltinTokenizer,
    pub tokenizer_id: TokenizerId,
    pub user_dict: Vec<UserDictEntry>,
    pub segment_ulids: Vec<String>,
}
```

确认 00-workspace 已为 Schema/FieldDef/ScalarKind/Metric/TokenizerId 派生 serde（`#[derive(serde::Serialize, serde::Deserialize)]`，TokenizerId 用 `#[serde(transparent)]`），02-tokenizer 已为 BuiltinTokenizer/UserDictEntry 派生 serde。无需重复添加（B8 裁决：重复 derive 会导致编译错误）。

- [ ] **Step 4: 跑测试确认通过** — `cargo test -p vane-core -- persistence`，2 测试绿。
- [ ] **Step 5: Commit**
```bash
git add -A
git commit -m "feat(persistence): Manifest + CollectionMeta serde roundtrip (§6.2)

"
```

### Task 2: ManifestStore 原子读写
**Files:**
- Modify: `crates/vane-core/src/persistence/mod.rs`（追加 ManifestStore）

**Interfaces:**
- Consumes from 00-workspace: Result, VaneError
- Consumes from 01-vfs: Vfs
- Consumes from Task 1: Manifest
- Produces: `ManifestStore::new()`, `load()`, `save_atomic()`, `add_segment()`

- [ ] **Step 1: 写失败测试** — 追加到 tests.rs：
```rust
use crate::vfs::memory::MemoryVfs;
use crate::vfs::Vfs;

#[test]
fn manifest_store_save_and_load() {
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
    let store = ManifestStore::new(vfs.clone(), "mydb");
    // 新库：load 返回 None
    assert!(matches!(store.load(), Ok(None)));

    let mut m = Manifest::empty();
    m.collections.insert("c1".into(), CollectionMeta {
        schema: Schema::new(vec![("v".into(), FieldDef::Vector{dim:8,metric:Metric::Cosine})]).unwrap(),
        tokenizer_kind: BuiltinTokenizer::Standard,
        tokenizer_id: TokenizerId([0;32]),
        user_dict: vec![],
        segment_ulids: vec![],
    });
    store.save_atomic(&m).unwrap();

    let loaded = store.load().unwrap().unwrap();
    assert_eq!(loaded.collections.len(), 1);
    assert!(loaded.collections.contains_key("c1"));
}

#[test]
fn manifest_store_save_atomic_overwrites() {
    // 不变量 I-6：rename 覆盖旧 manifest，旧数据不损坏
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
    let store = ManifestStore::new(vfs.clone(), "db");
    let mut m1 = Manifest::empty();
    m1.collections.insert("old".into(), CollectionMeta {
        schema: Schema::new(vec![("v".into(), FieldDef::Vector{dim:4,metric:Metric::Dot})]).unwrap(),
        tokenizer_kind: BuiltinTokenizer::Standard,
        tokenizer_id: TokenizerId([1;32]),
        user_dict: vec![],
        segment_ulids: vec![],
    });
    store.save_atomic(&m1).unwrap();

    let mut m2 = Manifest::empty();
    m2.collections.insert("new".into(), CollectionMeta {
        schema: Schema::new(vec![("v".into(), FieldDef::Vector{dim:4,metric:Metric::Dot})]).unwrap(),
        tokenizer_kind: BuiltinTokenizer::Standard,
        tokenizer_id: TokenizerId([2;32]),
        user_dict: vec![],
        segment_ulids: vec![],
    });
    store.save_atomic(&m2).unwrap();

    let loaded = store.load().unwrap().unwrap();
    assert!(!loaded.collections.contains_key("old"));
    assert!(loaded.collections.contains_key("new"));
}

#[test]
fn manifest_store_corrupt_returns_error() {
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
    // 写损坏的 manifest
    vfs.create("db/manifest.json").unwrap();
    vfs.write_at("db/manifest.json", b"not json {{{", 0).unwrap();
    let store = ManifestStore::new(vfs, "db");
    assert!(store.load().is_err());
}
```

- [ ] **Step 2: 跑测试确认失败** — `cargo test -p vane-core -- manifest_store`，编译失败。
- [ ] **Step 3: 最小实现** — 追加到 persistence/mod.rs：
```rust
use crate::vfs::Vfs;
use std::sync::Arc;

const MANIFEST_TMP: &str = "manifest.json.tmp";

pub struct ManifestStore {
    vfs: Arc<dyn Vfs>,
    db_path: String,
}

impl ManifestStore {
    pub fn new(vfs: Arc<dyn Vfs>, db_path: &str) -> Self {
        Self { vfs, db_path: db_path.to_string() }
    }

    fn manifest_path(&self) -> String {
        format!("{}/manifest.json", self.db_path)
    }
    fn tmp_path(&self) -> String {
        format!("{}/{}", self.db_path, MANIFEST_TMP)
    }

    pub fn load(&self) -> Result<Option<Manifest>> {
        let path = self.manifest_path();
        let mut buf = Vec::new();
        let mut tmp = [0u8; 8192];
        let mut off = 0u64;
        loop {
            let n = match self.vfs.read_at(&path, &mut tmp, off) {
                Ok(n) => n,
                Err(crate::types::VaneError::Io(_)) => return Ok(None), // 不存在
                Err(e) => return Err(e),
            };
            if n == 0 { break; }
            buf.extend_from_slice(&tmp[..n]);
            off += n as u64;
        }
        if buf.is_empty() { return Ok(None); }
        let m: Manifest = serde_json::from_slice(&buf)
            .map_err(|e| crate::types::VaneError::Corrupt(format!("manifest parse: {}", e)))?;
        Ok(Some(m))
    }

    /// SPEC §6.4 原子切换：写临时文件 → sync → rename。
    /// 不变量 I-6：任何崩溃后 manifest 指向完整状态。
    pub fn save_atomic(&self, manifest: &Manifest) -> Result<()> {
        let json = serde_json::to_vec(manifest)
            .map_err(|e| crate::types::VaneError::Corrupt(format!("manifest serialize: {}", e)))?;
        let tmp = self.tmp_path();
        // I16 裁决：先清理残留 tmp（忽略错误，tmp 可能不存在），再 create/write
        let _ = self.vfs.delete(&tmp);
        self.vfs.create(&tmp)?;
        self.vfs.write_at(&tmp, &json, 0)?;
        self.vfs.sync(&tmp)?;
        // 原子 rename 覆盖
        self.vfs.rename(&tmp, &self.manifest_path())?;
        // rename 后 sync 目录（MemoryVfs 无操作；StdFsVfs 的 rename 已落盘）
        Ok(())
    }

    pub fn add_segment(&self, collection: &str, ulid: &str) -> Result<()> {
        let mut m = self.load()?.unwrap_or_else(Manifest::empty);
        let col = m.collections.get_mut(collection)
            .ok_or_else(|| crate::types::VaneError::NotFound(
                format!("collection not found: {}", collection)
            ))?;
        if !col.segment_ulids.contains(&ulid.to_string()) {
            col.segment_ulids.push(ulid.to_string());
        }
        self.save_atomic(&m)
    }
}
```
- [ ] **Step 4: 跑测试确认通过** — `cargo test -p vane-core -- manifest_store`，3 测试绿。
- [ ] **Step 5: Commit**
```bash
git add -A
git commit -m "feat(persistence): ManifestStore atomic save/load via rename (§6.4, I-6)

"
```

### Task 3: AutoCommitter
**Files:**
- Modify: `crates/vane-core/src/persistence/mod.rs`（追加 AutoCommitConfig + AutoCommitter）

**Interfaces:**
- Consumes from: 无新依赖
- Produces: `AutoCommitConfig`（含 Default）、`AutoCommitter::new/record_docs/should_flush/reset`

- [ ] **Step 1: 写失败测试** — 追加到 tests.rs：
```rust
#[test]
fn auto_committer_default_is_on_1000_1000() {
    let c = AutoCommitConfig::default();
    match c {
        AutoCommitConfig::On { interval_ms, max_docs } => {
            assert_eq!(interval_ms, 1000);
            assert_eq!(max_docs, 1000);
        }
        AutoCommitConfig::Off => panic!("default should be On"),
    }
}

#[test]
fn auto_committer_triggers_on_max_docs() {
    let mut ac = AutoCommitter::new(AutoCommitConfig::On { interval_ms: 60_000, max_docs: 100 });
    assert!(!ac.should_flush());
    ac.record_docs(50);
    assert!(!ac.should_flush());
    ac.record_docs(50);
    assert!(ac.should_flush());
    ac.reset();
    assert!(!ac.should_flush());
}

#[test]
fn auto_committer_triggers_on_interval() {
    let mut ac = AutoCommitter::new(AutoCommitConfig::On { interval_ms: 0, max_docs: 1_000_000 });
    // interval_ms=0 → 任何时间差都触发（只要有未 flush 文档）
    ac.record_docs(1);
    assert!(ac.should_flush());
}

#[test]
fn auto_committer_off_never_flushes() {
    let mut ac = AutoCommitter::new(AutoCommitConfig::Off);
    ac.record_docs(9999);
    assert!(!ac.should_flush());
}
```

- [ ] **Step 2: 跑测试确认失败** — `cargo test -p vane-core -- auto_committer`，编译失败。
- [ ] **Step 3: 最小实现** — 追加到 persistence/mod.rs：
```rust
use std::time::Instant;

/// SPEC §7.1 auto-commit 配置。默认 On { interval_ms=1000, max_docs=1000 }。
#[derive(Debug, Clone)]
pub enum AutoCommitConfig {
    Off,
    On { interval_ms: u32, max_docs: u32 },
}

impl Default for AutoCommitConfig {
    fn default() -> Self {
        AutoCommitConfig::On { interval_ms: 1000, max_docs: 1000 }
    }
}

pub struct AutoCommitter {
    config: AutoCommitConfig,
    docs_since_flush: u32,
    last_flush: Instant,
}

impl AutoCommitter {
    pub fn new(config: AutoCommitConfig) -> Self {
        Self {
            config,
            docs_since_flush: 0,
            last_flush: Instant::now(),
        }
    }

    pub fn record_docs(&mut self, n: u32) {
        self.docs_since_flush = self.docs_since_flush.saturating_add(n);
    }

    pub fn should_flush(&self) -> bool {
        match &self.config {
            AutoCommitConfig::Off => false,
            AutoCommitConfig::On { interval_ms, max_docs } => {
                if self.docs_since_flush == 0 { return false; }
                if self.docs_since_flush >= *max_docs { return true; }
                let elapsed = self.last_flush.elapsed().as_millis() as u32;
                elapsed >= *interval_ms
            }
        }
    }

    pub fn reset(&mut self) {
        self.docs_since_flush = 0;
        self.last_flush = Instant::now();
    }
}
```
- [ ] **Step 4: 跑测试确认通过** — `cargo test -p vane-core -- auto_committer`，4 测试绿。
- [ ] **Step 5: Commit**
```bash
git add -A
git commit -m "feat(persistence): AutoCommitter with max_docs + interval triggers (§7.1)

"
```

### Task 4: 崩溃恢复语义验证（不变量 I-6）
**Files:**
- Modify: `crates/vane-core/src/persistence/tests.rs`

**Interfaces:**
- Consumes from Task 1-3
- Produces: I-6 测试覆盖

- [ ] **Step 1: 写测试** — 追加：
```rust
#[test]
fn manifest_atomicity_crash_before_rename() {
    // 模拟：临时文件写了但未 rename → load 应返回旧 manifest 或 None
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
    let store = ManifestStore::new(vfs.clone(), "db");
    // 先保存一个有效 manifest
    let m1 = Manifest::empty();
    store.save_atomic(&m1).unwrap();
    // 模拟崩溃：写一个临时文件但不 rename
    vfs.create("db/manifest.json.tmp").unwrap();
    vfs.write_at("db/manifest.json.tmp", b"partial garbage", 0).unwrap();
    // load 应返回上一次完整的 manifest（m1），不受 tmp 影响
    let loaded = store.load().unwrap().unwrap();
    assert_eq!(loaded.version, 1);
    assert!(loaded.collections.is_empty());
}

#[test]
fn orphan_segment_cleanup_on_open() {
    // 不变量 I-6：孤儿段文件（ULID 不在 manifest 中）可安全清理
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
    // 正常段
    vfs.create("db/segments/seg_01AAA").unwrap();
    // 孤儿段
    vfs.create("db/segments/seg_01BBB").unwrap();
    let store = ManifestStore::new(vfs.clone(), "db");
    let mut m = Manifest::empty();
    m.collections.insert("c".into(), CollectionMeta {
        schema: Schema::new(vec![("v".into(), FieldDef::Vector{dim:4,metric:Metric::Cosine})]).unwrap(),
        tokenizer_kind: BuiltinTokenizer::Standard,
        tokenizer_id: TokenizerId([0;32]),
        user_dict: vec![],
        segment_ulids: vec!["01AAA".into()],
    });
    store.save_atomic(&m).unwrap();
    // 验证 manifest 只记 01AAA；01BBB 是孤儿
    let loaded = store.load().unwrap().unwrap();
    assert_eq!(loaded.collections["c"].segment_ulids, vec!["01AAA".to_string()]);
    // 清理孤儿段（api-core 在 open 时调用；此处验证逻辑）
    let known: std::collections::HashSet<_> = loaded.collections.values()
        .flat_map(|c| c.segment_ulids.iter().cloned())
        .collect();
    let all_segs = vfs.list("db/segments").unwrap();
    let orphans: Vec<_> = all_segs.iter()
        .filter(|s| !known.contains(s.trim_start_matches("seg_")))
        .collect();
    assert!(orphans.contains(&"seg_01BBB".to_string()));
}

#[test]
fn save_atomic_with_stale_tmp_succeeds() {
    // I16: 残留 tmp 场景下 save_atomic 仍成功
    let vfs = std::sync::Arc::new(MemoryVfs::new()) as std::sync::Arc<dyn Vfs>;
    let store = ManifestStore::new(vfs.clone(), "db");
    // 模拟残留 tmp
    vfs.create("db/manifest.json.tmp").unwrap();
    vfs.write_at("db/manifest.json.tmp", b"stale garbage", 0).unwrap();
    // save_atomic 应先删 tmp 再写
    let m = Manifest::empty();
    store.save_atomic(&m).unwrap();
    // manifest.json 正确写入
    let loaded = store.load().unwrap().unwrap();
    assert_eq!(loaded.version, 1);
    assert!(loaded.collections.is_empty());
}
```
- [ ] **Step 2: 跑测试确认通过** — `cargo test -p vane-core -- persistence`，全绿。
- [ ] **Step 3: clippy + wasm32 check** —
```bash
cargo clippy -p vane-core -- -D warnings
cargo check --target wasm32-unknown-unknown -p vane-core 2>&1 | tail -5
```
- [ ] **Step 4: 确认全绿**
- [ ] **Step 5: Commit**
```bash
git add -A
git commit -m "test(persistence): crash recovery + orphan cleanup (I-6)

"
```
