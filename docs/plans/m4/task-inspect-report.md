# M4 阶段五 b — inspect API 实现报告

## 概要

M4 阶段五 b inspect API 已实现：新增 `Db::stats()` / `Db::segment_info()` /
`Db::collection_segment_info()` pub 方法和 7 个返回结构体 + 健康检查 + 20 个单测。
纯新增，不改 M0-M3 冻结 pub API。

## struct / 方法签名实现摘要

### 新增结构体（`crates/vane-core/src/api/inspect.rs`）

| struct | 字段 | derive |
|---|---|---|
| `DbStats` | db_path / collections / dict_available / executor_kind | Debug, Clone |
| `CollectionStats` | name / segment_count / total_docs / live_docs / tombstoned_docs / index_bytes / dict_state / tokenizer_id / health | Debug, Clone |
| `SegmentInfo` | ulid / doc_count / docid_base / tombstoned_count / format_versions / file_sizes / health | Debug, Clone |
| `FormatVersions` | header / vectors / stored / idmap / scalars / inverted / hnsw | Debug, Clone |
| `SegmentFileSizes` | header / vectors / stored / idmap / scalars / inverted / hnsw(Option) | Debug, Clone |
| `Health` | Healthy / Degraded / Corrupt | Debug, Clone, Copy, PartialEq, Eq |
| `ExecutorKind` | Serial / Rayon | Debug, Clone, Copy, PartialEq, Eq |

所有 struct 加 `#[derive(Debug, Clone)]`（Health/ExecutorKind 额外加 Copy/PartialEq/Eq），
避免 2b 的 SegmentMeta 无 Debug 触 E0277 教训。

### 新增方法（`crates/vane-core/src/api/db.rs` impl Db）

```rust
pub fn stats(&self) -> DbStats
pub fn segment_info(&self) -> Vec<SegmentInfo>
pub fn collection_segment_info(&self, name: &str) -> Option<Vec<SegmentInfo>>
```

签名按 §3.6 字面采用。纯新增，不改现有 pub fn/struct。

### 模块声明（`crates/vane-core/src/api/mod.rs`）

```rust
pub mod inspect;
pub use inspect::*;
```

### CollectionInner 字段可见性（`crates/vane-core/src/api/collection.rs`）

三个私有字段改为 `pub(crate)`（crate 内部可见，非 pub API 变更）：
- `snapshot: RwLock<Vec<Arc<SegmentReader>>>` → `pub(crate)`
- `hnsw_readers: RwLock<Vec<Option<Arc<HnswReader>>>>` → `pub(crate)`
- `dict_state: RwLock<DictState>` → `pub(crate)`

## 健康检查实现（读哪些内部状态）

| 健康标志 | 判定来源 | 实现位置 |
|---|---|---|
| 词典降级 | jieba feature on 时，`col_inner.tokenizer_kind == Jieba` 且 `DbInner.jieba_dict` None → Degraded | `build_collection_stats` 读 `dict_available`（来自 `DbInner.jieba_dict`） |
| 段损坏 | `SegmentReader::open(vfs, seg_dir)` 失败 → Corrupt | `segment_health` 调 `SegmentReader::open` |
| hnsw 缺失 fallback | `CollectionInner.hnsw_readers[i]` 为 None → Degraded | `segment_health` 读 `hnsw_readers.get(i)` |
| dict_state | `CollectionInner.dict_state` | `build_collection_stats` 读 `col_inner.dict_state` |
| executor_kind | `cfg!(all(not(target_arch="wasm32"), feature="executor-native"))` → Rayon / Serial | `executor_kind()` 函数 |

collection 级 health = 各段 health 的 worst（Corrupt > Degraded > Healthy），
再与词典降级取 worst。

### 关于"重新 open"的取舍

§3.6 取舍建议"不主动重新 open 校验（性能）"，但表 spec 要求"SegmentReader::open 失败 → Corrupt"。
本实现选择**重新 open**（`segment_health` 调 `SegmentReader::open`）：
- inspect 非热路径，性能可接受
- 能真实检测段损坏（文件被外部篡改后）
- 与表 spec 一致
- 与"index_bytes 用 read_at 探测"同属非热路径可接受范围

## index_bytes 方案：read_at 探测 EOF

Vfs trait 无 `size()` 方法（M0 冻结签名，不改）。采用**方案 A：read_at 探测 EOF**。

实现：`probe_file_size(vfs, path)` 从 offset=0 循环读 8KB buffer，n=0 即 EOF，
累计推算 size。文件不存在（read_at 返 Err）→ 返回 0。

`SegmentFileSizes` 各字段用 `u64`（文件缺失 → 0），`hnsw` 用 `Option<u64>`
（None = 无 hnsw.bin，fallback brute）。`index_bytes` = 各段 `file_sizes.total()` 之和。

未用方案 B（段文件格式已知字段推算——复杂）或方案 C（Vfs trait 加 size()——破坏 M0 冻结）。

## 单测清单（20 项，全部通过）

| # | 测试名 | 验证点 |
|---|---|---|
| 1 | stats_returns_db_path_and_executor | db_path 正确 + executor_kind 变体匹配 |
| 2 | stats_returns_one_collection_with_correct_counts | 1 collection / 1 segment / 2 docs / 0 tombstones |
| 3 | stats_index_bytes_nonzero | flush 后 index_bytes > 0 |
| 4 | stats_health_healthy_for_fresh_segment | 新段 + hnsw 存在 → Healthy |
| 5 | segment_info_returns_one_segment_with_correct_fields | ULID 非空 / doc_count=2 / docid_base=0 / tombstoned=0 |
| 6 | segment_info_format_versions_correct | 各文件 format_version 匹配写入常量 |
| 7 | segment_info_file_sizes_nonzero | header/vectors/stored/idmap/inverted 非零 + hnsw 存在 |
| 8 | segment_info_health_healthy | 段级 health = Healthy |
| 9 | collection_segment_info_returns_some_for_existing | 存在的 collection → Some |
| 10 | collection_segment_info_returns_none_for_missing | 不存在的 collection → None |
| 11 | stats_multiple_collections | 多 collection 排序 + 未 flush 0 segments |
| 12 | stats_empty_db | 空 DB / 0 collections / dict_available 与 jieba_dict_available() 一致 |
| 13 | segment_info_empty_collection | 未 flush → 0 segments |
| 14 | probe_file_size_empty_file_returns_zero | 不存在文件 → 0 |
| 15 | probe_file_size_correct_for_known_content | 100 字节文件 → 100 |
| 16 | read_version_field_missing_file_returns_zero | 不存在 → 0 |
| 17 | read_version_field_correct_for_vane_magic | MAGIC + V2 → VECTORS_FORMAT_V2 |
| 18 | worst_function_ordering | Corrupt > Degraded > Healthy |
| 19 | executor_kind_consistent_with_cfg | 与 cfg 推断一致 |
| 20 | stats_after_tombstone_delete | delete+flush 后 total/live/tombstoned 计数正确 |

断言非 vacuous：检查具体字段值（段数、文档数、format_version 常量、file_size > 0 等），
非仅 is_some()。

## 各门禁真实输出

### cargo fmt --all -- --check
```
（无输出 — 通过）
```

### cargo clippy --all-targets --all-features --workspace --exclude vane-fuzz -- -D warnings
```
    Checking vane-core v0.2.0
    Checking vane-wasm v0.2.0
    Checking vane-node v0.2.0
    Checking vane-dict-zh v2026.8.0
    Checking vane-ffi v0.2.0
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 6.13s
```

### cargo test -p vane-core --all-features --lib inspect
```
running 20 tests
...
test result: ok. 20 passed; 0 failed; 0 ignored; 0 measured; 323 filtered out; finished in 0.51s
```

### cargo test --workspace --all-features --exclude vane-fuzz
```
test result: ok. 342 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 3.41s
（全部 test result 行均 ok，0 failed）
```

### cargo deny check
```
advisories ok, bans ok, licenses ok, sources ok
```

### cargo check --target wasm32-unknown-unknown -p vane-core
```
    Checking vane-core v0.2.0
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.92s
（无 warning）
```

## commit

```
feat(core): inspect API（Db::stats/segment_info + 健康检查）（M4 阶段五 b）
```

文件：
- `crates/vane-core/src/api/inspect.rs`（新模块，~470 行）
- `crates/vane-core/src/api/db.rs`（+3 pub 方法 + dict_available_internal helper）
- `crates/vane-core/src/api/mod.rs`（+2 行 mod 声明）
- `crates/vane-core/src/api/collection.rs`（3 字段 pub(crate) 可见性）

## 自审

1. **Vfs 无 size 方案取舍**：选 read_at 探测 EOF（方案 A），不改 Vfs trait（M0 冻结）。
   inspect 非热路径，性能可接受。SegmentFileSizes.hnsw 用 Option<u64> 区分"无 hnsw.bin"
   vs "文件存在但 0 字节"。

2. **Debug derive 加了**：所有 7 个 struct/enum 加 `#[derive(Debug, Clone)]`，
   Health/ExecutorKind 额外加 Copy/PartialEq/Eq。避免 E0277。

3. **不改冻结 API 确认**：
   - 新增 `pub fn stats/segment_info/collection_segment_info`（纯新增，不改现有 pub fn）
   - 新增 7 个 pub struct/enum（纯新增）
   - CollectionInner 3 字段 private → pub(crate)（crate 内部可见性，非 pub API 变更）
   - DbInner 无改动（字段已 pub(crate)）
   - Vfs trait 不改（无 size()）
   - 不碰 SPEC.md / CI yml / fault.rs / crash_recovery / vane-fuzz / proptest / cross_version / tracing 埋点
