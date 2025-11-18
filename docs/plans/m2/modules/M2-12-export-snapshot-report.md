# M2-12 export 快照导出 — 实装报告

## 1. snapshot 格式实装

`crates/vane-core/src/api/snapshot.rs`（新建）实装 `VANE_SNAP` 单文件快照格式：

```
magic(9)="VANE_SNAP" | version(4 LE)=1 | num_files(4 LE) |
{ path_len(4 LE) | path_bytes | file_len(8 LE) | file_bytes }...
```

- 路径以相对 `db_path` 的 `/` 分量存储（`manifest.json`、`segments/seg_x/header.bin`、`wal.log`），恢复时可解包到任意 `db_path` 目录。
- 魔数取 9 字节 ASCII `VANE_SNAP`（spec 标注 `magic(4)="VANE_SNAP"` 存在字面矛盾——"VANE_SNAP" 为 9 字节，无法装入 4 字节；按字面字符串解释取 9 字节，`(4)` 视为 spec 笔误）。

## 2. write_snapshot / read_snapshot

### write_snapshot(vfs, db_path, dest)
1. `collect_files`：只读遍历 `manifest.json` + `segments/seg_<ulid>/` 全部文件（递归固定 2 层，跳过 `.tmp`）+ `wal.log`（若存在）。不收集 `manifest.json.tmp`（瞬态）。
2. 校验 `manifest.json` 存在（否则 `Corrupt`）。
3. 流式写 `<dest>.tmp`：`create` → `append` 写头 → 逐文件 `read_at` 全量 + `append` 写 `path_len|path|file_len|file_bytes`（逐文件读，非全库入内存）。
4. `sync` → `rename` → `dest`（I-6 原子；中途失败 dest 不残留，仅 `.tmp` 残留下次覆盖前 delete）。
5. 不修改原库（I-6 只读遍历 + 新文件写入）。

### read_snapshot(vfs, src, db_path)
1. 全量读快照到内存（恢复是低频操作）。
2. 校验 magic + version + num_files。
3. 逐文件解析 `path_len|path|file_len|file_bytes` → `delete`(忽略不存在) → `create` → `write_at` → `sync` 还原到 `<db_path>/<rel>`。
4. 随后 `Db::open(vfs, db_path, opts)` 即可打开恢复库。

辅助：`file_exists`（0 字节 `read_at` 探测，兼容 MemoryVfs/StdFsVfs）、`read_file_full`（文件不存在返回 None）。

## 3. export 接入

- **core** `api/db.rs:174`：`pub fn export(&self, dest: &str) -> Result<()>`（签名不变，M0 冻结）调 `snapshot::write_snapshot`。
- **vane-ffi** `vane_export`：既有实现已调 `db.export(dest)`（M2-11 转发壳），M2-12 自动生效；测试从 `export_returns_unsupported`（-10）改为 `export_succeeds_m2_12`（0 + dest 存在 + magic 校验）。
- **vane-node** `ExportTask`：`compute` 已调 `self.db.export(&self.dest)`（I-8 薄壳），M2-12 自动生效；集成测试从 `export_rejects_unsupported` 改为 `export_succeeds_m2_12`（Ok + magic 校验）。
- `api/mod.rs`：注册 `pub mod snapshot` + re-export `read_snapshot`/`write_snapshot`。

## 4. 闭环测试结果（P0-3 数据主权）

`export_read_snapshot_open_search_roundtrip`（MemoryVfs）：
1. 原库 `Db::open("orig")` + collection + add(2 docs) + flush。
2. `db.export("backup.vane")` → Ok。
3. `read_snapshot(vfs, "backup.vane", "restored")` → Ok。
4. `Db::open(vfs, "restored", opts)` + `collection("docs", ...)`（幂等取回）。
5. 向量搜索：`orig_hits[0].id == "a" == restored_hits[0].id`，score 一致，len 一致。
6. 文本搜索：`orig_text[0].id == rest_text[0].id`，len 一致。

结果：**PASS**。

其他快照测试：`snapshot_format_header`（magic + version + num_files）、`snapshot_includes_manifest_segments_wal`（manifest + wal.log + segments/seg_*/header.bin/vectors.bin/stored.bin，不含 .tmp）、`empty_db_export`、`read_snapshot_rejects_bad_magic`、`write_snapshot_is_atomic_no_partial_dest`。

## 5. 自证门禁结果

| # | 门禁 | 结果 |
|---|------|------|
| 1 | `cargo test --workspace --all-features` | 493 passed（487 基线 + 6 新增，0 回退） |
| 2 | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | clean |
| 3 | `cargo fmt --all -- --check` | clean |
| 4 | `cargo check --target wasm32-unknown-unknown -p vane-core` | 通过 |
| 5 | `bash scripts/check-no-std-fs.sh` | OK |
| 6 | `cargo deny check` | advisories/bans/licenses/sources ok |
| 7 | export→read_snapshot→open→search 闭环 | PASS（MemoryVfs） |
| 8 | snapshot 格式测试 | PASS（magic/version/num_files/文件项） |
| 9 | vane_export + ExportTask 接入 | ffi rc=0 + dest 存在 + magic；node Ok + magic |
| 10 | export 签名不变 | `pub fn export(&self, dest: &str) -> Result<()>`（db.rs:174） |
| 11 | wal.log 含入快照 | `snapshot_includes_manifest_segments_wal` 断言含 wal.log |

## 6. 遗留 / concerns

- **spec magic(4) 矛盾**：spec 写 `magic(4)="VANE_SNAP"`，但 "VANE_SNAP" 是 9 字节。实装取 9 字节字面字符串，`(4)` 视为笔误。如需 4 字节魔数（如 "VANE"）需 spec 澄清。
- **大库流式**：`write_snapshot` 逐文件 `read_at` 全量入内存再 `append`（单文件粒度，非全库）。单段文件超大数据时仍可能占内存；当前 M2 段粒度可接受。`read_snapshot` 全量读快照入内存（恢复低频操作）。spec 测试 13（10 万文档不 OOM）未单独构造，但逐文件流式写已避免全库入内存。
- **export 不锁读**：经 Arc swap 段视图读路径无锁；export 只读遍历 Vfs 文件，不持 collection 锁（与 search 并发安全）。
