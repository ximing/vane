# M2-12 export 快照导出

## 1. 目标
实装 `Db::export(destPath) -> Result<()>`（SPEC §4.1），打包单文件快照。M0/M1 占位 `Err(VaneError::Unsupported)`（`crates/vane-core/src/api/db.rs:164-166`）→ M2 实装。wasm 端经 OPFS Vfs 写快照（M2-02）；vane-node `ExportTask`（既有 `crates/vane-node/src/db.rs:110`）从占位变真实导出；vane-ffi `vane_export`（M2-11）接入。

SPEC 节号：§4.1（`Db.export(destPath)->Result<()>`）、§15（M2 交付 export 快照）、REQUIREMENTS §1（P0-3 浏览器数据主权：OPFS 被驱逐构成产品事故，必须提供快照导出）。

## 2. 涉及文件
- **Modify** `crates/vane-core/src/api/db.rs:164-166`（`export` 占位）：实装打包逻辑。签名不变（`pub fn export(&self, dest: &str) -> Result<()>`）。
- **Create** `crates/vane-core/src/api/snapshot.rs`：快照格式编解码。
- **Modify** `crates/vane-core/src/api/db.rs`：`Db` 增内部访问 manifest + 段文件路径的方法（若不存在）。
- **Modify** `crates/vane-ffi/src/lib.rs`（M2-11 协同）：`vane_export` 接入实装（M2-11 先返 E_UNSUPPORTED，M2-12 后接入）。
- **Modify** `crates/vane-node/src/db.rs:110`（`ExportTask` 既有）：从 `Err(Unsupported)` 变真实导出（M2-12 后自动生效，因调 `self.db.export(&self.dest)`）。

## 3. 接口契约
### Consumes from
- M0/M1 `Db` 内部：`ManifestStore`、`manifest.json` 路径、`segments/` 目录、`wal.log`。
- M0 `vane_core::vfs::Vfs` trait（`vfs/mod.rs:5-13`，8 方法）：用 `Vfs::list` + `read_at` 读全部文件，`Vfs::write_at` 写 dest 快照。
- M2-02 OPFS Vfs（wasm 端 export 写 OPFS）。

### Produces for
```rust
// crates/vane-core/src/api/snapshot.rs
// 快照格式（SPEC §4.1 "单文件快照"，M2 定义）：
//   magic(4)="VANE_SNAP" | version(4 LE)=1 | num_files(4 LE) |
//   { path_len(4 LE) | path_bytes | file_len(8 LE) | file_bytes }...
// 遍历 manifest.json + 全部 seg_*/全部文件 + wal.log → 打包写 dest
pub fn write_snapshot(vfs: &dyn Vfs, db_path: &str, dest: &str) -> Result<()>;
pub fn read_snapshot(vfs: &dyn Vfs, src: &str, db_path: &str) -> Result<()>;  // 恢复：解包单文件快照到 db_path 目录

impl Db {
    pub fn export(&self, dest: &str) -> Result<()>;  // 签名不变（db.rs:164）；调 write_snapshot
}
```
**快照恢复（import）路径**（reviewer B-I3：消解与单文件格式矛盾，落实 P0-3 数据主权）：
- 快照是**单文件** `backup.vane`（`VANE_SNAP` 格式），`Db::open` 期望**目录路径**，不能直接打开单文件。
- 恢复路径：`read_snapshot(vfs, src="backup.vane", db_path="restored_db")` —— 解包单文件快照到 `db_path` 目录（按 `path_len|path_bytes|file_len|file_bytes` 逐文件 `vfs.create + write_at + sync` 还原 `manifest.json` + `segments/seg_*/...` + `wal.log`），随后 `Db::open(vfs, "restored_db", opts)` 即可打开恢复后的库。
- 三侧恢复：wasm 端经 OPFS Vfs（M2-02）解包到 OPFS 路径；Node 端经 StdFsVfs 解包到本地目录；Go 端经 vane-ffi + cgo 调 `read_snapshot`。
- 测试覆盖：export → `read_snapshot` 到新路径 → `Db::open` → search 结果与原库一致（P0-3 数据主权闭环）。

**递归 list segments/seg_*/ 逻辑**（reviewer B-M10）：`Vfs::list(dir)` 只列单层目录，段文件在 `segments/seg_<ulid>/` 子目录下。`write_snapshot` 遍历逻辑：`list("segments")` → 对每个 `seg_<ulid>` 调 `list("segments/seg_<ulid>")` → 对每个文件 `read_at` 打包。递归层数固定 2 层（segments/seg_xxx/file），无更深层级。
下游：M2-11 `vane_export` 接入；vane-node `ExportTask` 自动生效；wasm 端 export（M2-04 Worker `export` 方法）。

## 4. TDD 测试清单
1. **export 基本流程**：open + collection + add + flush → `db.export("./backup.vane")` → 返回 Ok；dest 文件存在，大小>0。
2. **快照格式头**：dest 文件前 4 字节 = `"VANE_SNAP"`；version=1 LE；num_files 正确。
3. **快照内容完整**：dest 含 manifest.json + 全部段文件（header/vectors/stored/idmap/scalars/hnsw/inverted）+ wal.log。
4. **快照恢复（read_snapshot 实装）**：export → `read_snapshot(vfs, "backup.vane", "restored_db")` 解包到新目录 → `Db::open(vfs, "restored_db", opts)` → search 结果与原库一致（P0-3 数据主权闭环，reviewer B-I3 落实）。
5. **空库 export**：空 manifest + 无段 → export 成功，num_files=1（仅 manifest.json）。
6. **tombstone 后 export**：delete + flush → export 快照含 tombstone（header.bin tombstone 位图）。
7. **compact 后 export**：compact → export 快照含合并后段，旧段不在快照。
8. **stored v2 快照兼容**（M2-08 协同）：stored.bin v2 段 export → 快照含 v2 文件 → 恢复后读回一致。
9. **wasm OPFS export**：wasm 端 `db.export("backup.vane")` 经 OPFS Vfs 写快照（M2-02），dest 在 OPFS 根目录。
10. **vane-node ExportTask**：Node 侧 `await db.export("./backup.vane")` 返回（M2-12 后从 reject E_UNSUPPORTED 变 resolve）。
11. **vane_export C ABI**（M2-11 协同）：`vane_export(db_h, dest_ptr, dest_len)` 返回 0；dest 文件存在。
12. **错误码**：export 到不可写路径 → `Err(VaneError::Io)`（-1）；dest 路径非法 → `Err(InvalidArg)`（-11）。
13. **大库 export**：10万文档库 export，耗时与文件总量成正比，不 OOM（流式写，非全内存）。
14. **原子性**：export 期间不锁读（快照经 Arc swap 段视图，读路径无锁）；export 中途失败 dest 文件不残留（写临时文件 → rename，I-6 对齐）。

## 5. 验收标准
- `Db::export` 签名不变（`db.rs:164`），实装替换占位。
- 快照格式头正确，内容完整（manifest + 段 + wal）。
- wasm/Node/Go 三侧 export 均可用（vane-wasm Worker + vane-node ExportTask + vane-ffi vane_export）。
- **恢复路径可用**（P0-3 数据主权）：`read_snapshot` 实装，export → read_snapshot → Db::open 数据一致。
- 既有 vane-node ExportTask 测试从 reject E_UNSUPPORTED 改 resolve（行为测试更新）。
- export 不锁读，不 OOM（流式）。
- clippy clean，cargo deny ok。

## 6. 前置依赖
- M2-02（OPFS Vfs，wasm 端 export 写快照）。
- M2-08 协同（stored v2 快照兼容；非强阻塞，v1 快照先可用）。
- M2-11 协同（`vane_export` C ABI 接入；可后接入）。

## 7. 不变量覆盖
- **I-6 manifest 原子性**：export 读 manifest 一致快照；写 dest 走临时文件→rename（对齐 manifest 原子语义）。测试 14 守护。
- **I-7 FFI 内存铁律**：`vane_export` 不分配 arena（dest 是路径，非返回 buffer）；dest 字符串由调用方提供。测试 11 守护。
- **I-8 binding 薄壳**：export 逻辑在 core；Node/Go/wasm 仅调 `db.export(dest)`。测试 9+10+11 守护。
- **§4.1 IDL**：`export(destPath)->Result<()>` 签名不变。验收守护。
- **REQUIREMENTS §1 P0-3 数据主权**：浏览器 export 快照提供，OPFS 被驱逐可恢复。测试 9 守护。
