//! IdbVfs——IndexedDB 降级 VFS 后端（M2-03，feature = "idb"）。
//!
//! OPFS 不可用时（Safari 旧版 / 无 OPFS 支持）的浏览器降级路径。
//! 复用 M2-02 `MemOverlay` 内核（文件表/区间/free list/双 meta_slot + CRC），
//! 差异仅在 `OverlayBackend` impl：底层为内存 `Vec<u8>` 容器映像 + 异步 checkpoint。
//!
//! ## sync 语义（best-effort，I-6 语义降级）
//! `sync(path)` 经 `MemOverlay::sync` → `persist_meta`（写 meta 到内存 Vec）+
//! `IdbBackend::flush`（标 dirty=true，**不真正落盘**）。JS 壳层（M2-04）异步 tick
//! 触发 `snapshot()` → IDB `put`。崩溃可能丢最近未 checkpoint 的写入——降级场景
//! 可接受，关键数据走 `export()` 快照（M2-12）。
//!
//! 与 OPFS 主路径的区别：OPFS `flush` = `SyncAccessHandle.flush`（真落盘，I-6 等价原子）；
//! IDB `flush` = 标 dirty（尽力持久化）。文档明示此降级折损。
//!
//! ## 测试策略
//! IDB 实际 put/get 是 JS 异步（浏览器验证，node 无 IDB）——薄层，wasm32 编译通过 +
//! 浏览器验证标注待 M2-04。本模块的 Vfs 语义（8 方法）用内存 Vec backend 测，node 可跑
//! （与 `MemoryBackend` 等价）。重点测：`from_blob` 恢复、`schedule_checkpoint` 标 dirty、
//! `sync` best-effort 不抛错、降级路径不返 `E_UNSUPPORTED`。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use vane_core::types::{Result, VaneError};
use vane_core::vfs::Vfs;

use super::overlay::{MemOverlay, OverlayBackend};

// ── IdbBackend ─────────────────────────────────────────────────────────────

/// IDB 降级后端：内存 `Vec<u8>` 容器映像 + dirty 标志。
///
/// `flush` 标 dirty=true（best-effort，不真正落盘）。JS 壳层异步 tick 读取
/// `snapshot()` put 回 IDB。read/write/size/truncate 操作内存 Vec（同步）。
///
/// 与 `MemoryBackend`（M2-02 测试用）的区别：`flush` 标 dirty（M2-03 best-effort
/// 语义），`MemoryBackend::flush` 为 no-op。文件表/区间/CRC 逻辑全部复用 `MemOverlay`。
pub struct IdbBackend {
    data: RwLock<Vec<u8>>,
    dirty: AtomicBool,
}

impl IdbBackend {
    /// 从容器映像构造（Worker init 异步从 IDB 读取 blob 后传入）。
    /// 新库传空 `Vec::new()`——`MemOverlay::open` 初始化空容器。
    pub fn new(blob: Vec<u8>) -> Self {
        Self {
            data: RwLock::new(blob),
            dirty: AtomicBool::new(false),
        }
    }

    /// 返回容器映像完整快照（JS 壳层 checkpoint tick 调用，put 回 IDB）。
    pub fn snapshot(&self) -> Vec<u8> {
        self.data.read().unwrap().clone()
    }

    /// 是否有未 checkpoint 的变更。
    pub fn is_dirty(&self) -> bool {
        self.dirty.load(Ordering::Acquire)
    }

    /// checkpoint 完成（IDB put 成功）后清 dirty。
    pub fn clear_dirty(&self) {
        self.dirty.store(false, Ordering::Release);
    }
}

fn poison_err<T>(_: std::sync::PoisonError<T>) -> VaneError {
    VaneError::Io("idb backend lock poisoned".into())
}

impl OverlayBackend for IdbBackend {
    fn read(&self, off: u64, buf: &mut [u8]) -> Result<usize> {
        let d = self.data.read().map_err(poison_err)?;
        let off = off as usize;
        if off >= d.len() {
            return Ok(0);
        }
        let n = buf.len().min(d.len() - off);
        buf[..n].copy_from_slice(&d[off..off + n]);
        Ok(n)
    }

    fn write(&self, off: u64, buf: &[u8]) -> Result<()> {
        let mut d = self.data.write().map_err(poison_err)?;
        let off = off as usize;
        let end = off + buf.len();
        if d.len() < end {
            d.resize(end, 0);
        }
        d[off..end].copy_from_slice(buf);
        Ok(())
    }

    fn flush(&self) -> Result<()> {
        // best-effort（I-6 语义降级）：标 dirty，不真正落盘。
        // JS 壳层（M2-04）异步 tick 把 snapshot() put 回 IDB。
        // 与 OPFS 的 SyncAccessHandle.flush（真落盘）有意识地区分——文档明示。
        self.dirty.store(true, Ordering::Release);
        Ok(())
    }

    fn size(&self) -> Result<u64> {
        Ok(self.data.read().map_err(poison_err)?.len() as u64)
    }

    fn truncate(&self, sz: u64) -> Result<()> {
        let mut d = self.data.write().map_err(poison_err)?;
        d.resize(sz as usize, 0);
        Ok(())
    }
}

// ── IdbVfs ─────────────────────────────────────────────────────────────────

/// IndexedDB 降级 VFS：内存容器映像 + `MemOverlay`（文件表/区间/free list/双 meta_slot）。
///
/// 实现 `Vfs` trait 8 方法（全同步，委托 `MemOverlay`）。Worker init 异步从 IDB
/// 读取容器 blob 后通过 `from_blob` 构造；此后进入同步 Vfs 世界。
///
/// sync 语义 best-effort：`sync` 标 dirty，JS 壳层异步 checkpoint tick 落盘 IDB。
/// 崩溃可能丢最近未 checkpoint 的写入——降级场景可接受，关键数据走 `export()`。
///
/// I-8 binding 薄壳：无检索逻辑，行为测试在 core Vfs 套件。
pub struct IdbVfs {
    overlay: MemOverlay,
    backend: Arc<IdbBackend>,
}

impl IdbVfs {
    /// 从 IDB 读取的容器 blob 构造（Worker init 异步读取后传入）。
    /// 新库传空 `Vec::new()`——`MemOverlay::open` 初始化空容器。
    ///
    /// 构造后 dirty=false：`from_blob` 是「从 IDB 加载」边界，加载后内存状态
    /// 反映 blob 内容（既有库）或初始空容器（新库 init 写入视为加载过程的一部分）。
    /// 后续 `sync` / `schedule_checkpoint` 标 dirty 触发 JS 壳层 checkpoint。
    pub fn from_blob(blob: Vec<u8>) -> Result<Self> {
        let backend = Arc::new(IdbBackend::new(blob));
        let overlay = MemOverlay::open(backend.clone())?;
        // init_new（空容器）会调 flush 标 dirty；from_blob 是加载边界，清 dirty。
        backend.clear_dirty();
        Ok(Self { overlay, backend })
    }

    /// 标记需要 checkpoint（JS 壳层异步 tick 触发 IDB put）。
    pub fn schedule_checkpoint(&self) {
        self.backend.dirty.store(true, Ordering::Release);
    }

    /// 是否有未 checkpoint 的变更（JS 壳层 tick 轮询）。
    pub fn is_dirty(&self) -> bool {
        self.backend.is_dirty()
    }

    /// 返回容器映像快照（JS 壳层 checkpoint tick put 回 IDB）。
    pub fn snapshot(&self) -> Vec<u8> {
        self.backend.snapshot()
    }

    /// checkpoint 完成（IDB put 成功）后清 dirty。
    pub fn clear_dirty(&self) {
        self.backend.clear_dirty()
    }

    /// 活跃 meta slot（测试 / 调试用）。
    pub fn active_meta_slot(&self) -> u8 {
        self.overlay.active_meta_slot()
    }

    /// 当前 generation（测试用）。
    pub fn generation(&self) -> u64 {
        self.overlay.generation()
    }
}

impl Vfs for IdbVfs {
    fn create(&self, path: &str) -> Result<()> {
        self.overlay.create(path)
    }
    fn read_at(&self, path: &str, buf: &mut [u8], offset: u64) -> Result<usize> {
        self.overlay.read_at(path, buf, offset)
    }
    fn write_at(&self, path: &str, buf: &[u8], offset: u64) -> Result<()> {
        self.overlay.write_at(path, buf, offset)
    }
    fn append(&self, path: &str, buf: &[u8]) -> Result<u64> {
        self.overlay.append(path, buf)
    }
    fn sync(&self, path: &str) -> Result<()> {
        // best-effort：MemOverlay::sync 写 meta 到内存 Vec + IdbBackend::flush 标 dirty。
        // 不真正落盘——JS 壳层异步 tick 把 snapshot put 回 IDB。
        self.overlay.sync(path)
    }
    fn rename(&self, from: &str, to: &str) -> Result<()> {
        self.overlay.rename(from, to)
    }
    fn delete(&self, path: &str) -> Result<()> {
        self.overlay.delete(path)
    }
    fn list(&self, dir: &str) -> Result<Vec<String>> {
        self.overlay.list(dir)
    }
}

// ── OPFS 能力探针（M2-04 落实真实探针）─────────────────────────────────────

/// OPFS 能力探针占位（M2-04 落实真实 `navigator.storage.getDirectory` 探测）。
///
/// 当前 stub 返 `true`（假设 OPFS 可用）。M2-04 Worker init 调用本函数：
/// - `true` → 走 `OpfsVfs` 主路径
/// - `false` → 降级 `IdbVfs` + `console.warn`（不抛错，SPEC §10 E_UNSUPPORTED 消解）
///
/// 返 `true` 不代表 OPFS 一定可用——真实探测在 M2-04。本占位仅保证接口存在，
/// 让 M2-04 Worker 能引用本模块的 `IdbVfs` 作为降级候选。
pub fn opfs_available() -> bool {
    // TODO(M2-04): 真实探针——navigator.storage.getDirectory() + feature 检测 +
    // try/catch 探测写入能力（Safari 历史 OPFS bug 缓解）。
    true
}

// ══════════════════════════════════════════════════════════════════════════
// 测试（node 可跑——Vfs 语义用内存 Vec backend，IDB put/get 薄层浏览器验证待 M2-04）
// ══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use vane_core::types::VaneError;
    use vane_core::vfs::Vfs;

    // ── 辅助 ────────────────────────────────────────────────────────────

    fn new_idb() -> IdbVfs {
        IdbVfs::from_blob(Vec::new()).unwrap()
    }

    /// 通用 Vfs 契约测试（与 overlay.rs::run_conformance + M0 Vfs 套件同构）。
    fn run_conformance<V: Vfs>(vfs: &V) {
        // create + write_at + read_at
        vfs.create("a.bin").unwrap();
        vfs.write_at("a.bin", b"hello", 0).unwrap();
        let mut buf = [0u8; 5];
        let n = vfs.read_at("a.bin", &mut buf, 0).unwrap();
        assert_eq!(n, 5);
        assert_eq!(&buf, b"hello");

        // append 返回起始 offset
        let off = vfs.append("a.bin", b" world").unwrap();
        assert_eq!(off, 5);
        let mut buf2 = [0u8; 11];
        vfs.read_at("a.bin", &mut buf2, 0).unwrap();
        assert_eq!(&buf2, b"hello world");

        // write_at 覆盖
        vfs.write_at("a.bin", b"HELLO", 0).unwrap();
        let mut buf3 = [0u8; 5];
        vfs.read_at("a.bin", &mut buf3, 0).unwrap();
        assert_eq!(&buf3, b"HELLO");

        // list
        vfs.create("b.bin").unwrap();
        let files = vfs.list(".").unwrap();
        assert!(files.contains(&"a.bin".to_string()));
        assert!(files.contains(&"b.bin".to_string()));

        // rename 原子覆盖
        vfs.create("c.bin").unwrap();
        vfs.write_at("c.bin", b"replaced", 0).unwrap();
        vfs.rename("a.bin", "c.bin").unwrap();
        let mut buf4 = [0u8; 8];
        vfs.read_at("c.bin", &mut buf4, 0).unwrap();
        assert_eq!(&buf4, b"HELLO wo"); // a.bin 的前 8 字节覆盖 c.bin

        // delete
        vfs.delete("c.bin").unwrap();
        assert!(vfs.read_at("c.bin", &mut [0u8; 1], 0).is_err());

        // read 不存在文件报错
        assert!(vfs.read_at("nonexistent", &mut [0u8; 1], 0).is_err());

        // list 按 dir 过滤
        vfs.create("sub/x.bin").unwrap();
        vfs.create("sub/y.bin").unwrap();
        let sub_files = vfs.list("sub").unwrap();
        assert!(sub_files.contains(&"x.bin".to_string()));
        assert!(sub_files.contains(&"y.bin".to_string()));
        let root_files = vfs.list(".").unwrap();
        assert!(!root_files
            .iter()
            .any(|f| f.contains("x.bin") && f.contains("sub")));
    }

    // ── 门禁 8: IdbVfs Vfs 语义测试（与 OpfsVfs/MemoryVfs 等价）─────────

    #[test]
    fn idb_vfs_conformance() {
        let vfs = new_idb();
        run_conformance(&vfs);
    }

    #[test]
    fn idb_create_empty_file_read_returns_zero() {
        let vfs = new_idb();
        vfs.create("empty.bin").unwrap();
        let mut buf = [0u8; 10];
        let n = vfs.read_at("empty.bin", &mut buf, 0).unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn idb_write_at_and_read_at_roundtrip() {
        let vfs = new_idb();
        vfs.create("f.bin").unwrap();
        vfs.write_at("f.bin", b"hello world", 0).unwrap();
        let mut buf = [0u8; 11];
        let n = vfs.read_at("f.bin", &mut buf, 0).unwrap();
        assert_eq!(n, 11);
        assert_eq!(&buf, b"hello world");
    }

    #[test]
    fn idb_append_twice_offsets_and_size() {
        let vfs = new_idb();
        vfs.create("log.bin").unwrap();
        let off1 = vfs.append("log.bin", b"AAAA").unwrap();
        assert_eq!(off1, 0);
        let off2 = vfs.append("log.bin", b"BBBB").unwrap();
        assert_eq!(off2, 4);
        let mut buf = [0u8; 8];
        vfs.read_at("log.bin", &mut buf, 0).unwrap();
        assert_eq!(&buf, b"AAAABBBB");
    }

    #[test]
    fn idb_rename_content_moves_source_gone() {
        let vfs = new_idb();
        vfs.create("from.bin").unwrap();
        vfs.write_at("from.bin", b"content", 0).unwrap();
        vfs.rename("from.bin", "to.bin").unwrap();
        let mut buf = [0u8; 7];
        vfs.read_at("to.bin", &mut buf, 0).unwrap();
        assert_eq!(&buf, b"content");
        assert!(vfs.read_at("from.bin", &mut [0u8; 1], 0).is_err());
    }

    #[test]
    fn idb_delete_then_list_excludes() {
        let vfs = new_idb();
        vfs.create("del.bin").unwrap();
        vfs.write_at("del.bin", b"x", 0).unwrap();
        vfs.delete("del.bin").unwrap();
        let files = vfs.list(".").unwrap();
        assert!(!files.contains(&"del.bin".to_string()));
    }

    #[test]
    fn idb_list_segments_returns_ulid_entries() {
        let vfs = new_idb();
        vfs.create("segments/seg_01abc/header.bin").unwrap();
        vfs.create("segments/seg_02def/header.bin").unwrap();
        vfs.create("manifest.json").unwrap();
        let segs = vfs.list("segments").unwrap();
        assert_eq!(segs, vec!["seg_01abc", "seg_02def"]);
    }

    #[test]
    fn idb_read_nonexistent_returns_io_err() {
        let vfs = new_idb();
        let result = vfs.read_at("nope", &mut [0u8; 1], 0);
        assert!(result.is_err());
        match result {
            Err(VaneError::Io(_)) => {}
            other => panic!("expected VaneError::Io, got {:?}", other.map(|_| ())),
        }
    }

    // ── 门禁 9: from_blob 恢复测试 ─────────────────────────────────────

    #[test]
    fn from_blob_empty_vec_initializes_new_library() {
        // 新库：空 Vec → 初始化空容器
        let vfs = IdbVfs::from_blob(Vec::new()).unwrap();
        assert!(vfs.list(".").unwrap().is_empty());
        assert_eq!(vfs.generation(), 0);
        // 初始 dirty=false（from_blob 不标 dirty）
        assert!(!vfs.is_dirty());
    }

    #[test]
    fn from_blob_recovers_existing_file_table() {
        // 写若干文件 → snapshot → from_blob(snapshot) → 文件表与原一致
        let blob = {
            let vfs = new_idb();
            vfs.create("manifest.json").unwrap();
            vfs.write_at("manifest.json", b"manifest_data", 0).unwrap();
            vfs.create("wal.log").unwrap();
            vfs.append("wal.log", b"entry1").unwrap();
            vfs.append("wal.log", b"entry2").unwrap();
            vfs.create("segments/seg_01/header.bin").unwrap();
            vfs.write_at("segments/seg_01/header.bin", b"hdr", 0)
                .unwrap();
            vfs.sync("manifest.json").unwrap(); // persist meta 到内存 Vec
            vfs.snapshot()
        };
        // 从 blob 恢复
        let vfs2 = IdbVfs::from_blob(blob).unwrap();
        let mut buf = [0u8; 13];
        let n = vfs2.read_at("manifest.json", &mut buf, 0).unwrap();
        assert_eq!(n, 13);
        assert_eq!(&buf, b"manifest_data");

        let mut wal_buf = [0u8; 12];
        let n = vfs2.read_at("wal.log", &mut wal_buf, 0).unwrap();
        assert_eq!(n, 12);
        assert_eq!(&wal_buf, b"entry1entry2");

        let mut hdr_buf = [0u8; 3];
        let n = vfs2
            .read_at("segments/seg_01/header.bin", &mut hdr_buf, 0)
            .unwrap();
        assert_eq!(n, 3);
        assert_eq!(&hdr_buf, b"hdr");

        let segs = vfs2.list("segments").unwrap();
        assert_eq!(segs, vec!["seg_01"]);
    }

    #[test]
    fn snapshot_round_trip_data_intact() {
        // checkpoint 后重新 from_blob 读回 → 数据一致（模拟 IDB put/get round-trip）
        let vfs = new_idb();
        vfs.create("a.bin").unwrap();
        vfs.write_at("a.bin", b"AAAA", 0).unwrap();
        vfs.create("b.bin").unwrap();
        vfs.write_at("b.bin", b"BBBBBBBB", 0).unwrap();
        vfs.sync("a.bin").unwrap(); // persist meta
        assert!(vfs.is_dirty()); // sync 标 dirty

        // 模拟 JS 壳层 checkpoint：snapshot → put IDB → (新 session) get IDB → from_blob
        let blob = vfs.snapshot();
        vfs.clear_dirty();
        assert!(!vfs.is_dirty());

        // 新 session 从 blob 恢复
        let vfs2 = IdbVfs::from_blob(blob).unwrap();
        let mut buf = [0u8; 4];
        vfs2.read_at("a.bin", &mut buf, 0).unwrap();
        assert_eq!(&buf, b"AAAA");
        let mut buf2 = [0u8; 8];
        vfs2.read_at("b.bin", &mut buf2, 0).unwrap();
        assert_eq!(&buf2, b"BBBBBBBB");
    }

    // ── 门禁 10: sync best-effort + schedule_checkpoint + 降级不抛错 ───

    #[test]
    fn sync_marks_dirty_no_panic() {
        // sync best-effort：不 panic，标 dirty
        let vfs = new_idb();
        vfs.create("s.bin").unwrap();
        vfs.write_at("s.bin", b"data", 0).unwrap();
        assert!(!vfs.is_dirty()); // write_at 不经 flush，不标 dirty
        vfs.sync("s.bin").unwrap(); // persist_meta + flush 标 dirty
        assert!(vfs.is_dirty());
    }

    #[test]
    fn schedule_checkpoint_marks_dirty() {
        let vfs = new_idb();
        assert!(!vfs.is_dirty());
        vfs.schedule_checkpoint();
        assert!(vfs.is_dirty());
    }

    #[test]
    fn clear_dirty_after_checkpoint() {
        let vfs = new_idb();
        vfs.schedule_checkpoint();
        assert!(vfs.is_dirty());
        vfs.clear_dirty();
        assert!(!vfs.is_dirty());
    }

    #[test]
    fn sync_on_empty_no_panic() {
        // 无变更时 sync 不 panic，不标 dirty（MemOverlay::sync 在 dirty=false 时不 persist）
        let vfs = new_idb();
        vfs.create("empty.bin").unwrap();
        // create 只置 state.dirty（overlay 内部），不调 flush——is_dirty() 仍 false
        vfs.sync("empty.bin").unwrap();
        // sync 后 backend flush 被调用 → dirty=true
        assert!(vfs.is_dirty());
    }

    #[test]
    fn downgrade_path_never_returns_e_unsupported() {
        // 降级路径不返 E_UNSUPPORTED（SPEC §10 消解）。
        // IdbVfs 所有操作返回 Ok 或 Io/NotFound——无 Unsupported。
        let vfs = new_idb();
        // 正常操作
        assert!(vfs.create("a.bin").is_ok());
        assert!(vfs.write_at("a.bin", b"x", 0).is_ok());
        assert!(vfs.sync("a.bin").is_ok());
        assert!(vfs.rename("a.bin", "b.bin").is_ok());
        assert!(vfs.delete("b.bin").is_ok());
        assert!(vfs.list(".").is_ok());
        // 错误路径：均为 Io，非 Unsupported
        let err = vfs.read_at("nope", &mut [0u8; 1], 0).unwrap_err();
        assert!(matches!(err, VaneError::Io(_)));
        assert!(!matches!(err, VaneError::Unsupported(_)));
        let err = vfs.delete("nope").unwrap_err();
        assert!(matches!(err, VaneError::Io(_)));
        let err = vfs.rename("nope", "dst").unwrap_err();
        assert!(matches!(err, VaneError::Io(_)));
    }

    #[test]
    fn opfs_available_stub_returns_true() {
        // 占位探针返 true（M2-04 落实真实探针）
        assert!(opfs_available());
    }

    // ── 大文件 / 边界 ──────────────────────────────────────────────────

    #[test]
    fn idb_large_append_across_pages() {
        let vfs = new_idb();
        vfs.create("big.bin").unwrap();
        let chunk = vec![42u8; 100_000];
        let off1 = vfs.append("big.bin", &chunk).unwrap();
        assert_eq!(off1, 0);
        let off2 = vfs.append("big.bin", &chunk).unwrap();
        assert_eq!(off2, 100_000);
        let mut buf = vec![0u8; 50];
        vfs.read_at("big.bin", &mut buf, 99_990).unwrap();
        assert!(buf.iter().all(|&b| b == 42));
    }

    #[test]
    fn idb_write_at_beyond_size_grows_with_zero_fill() {
        let vfs = new_idb();
        vfs.create("f.bin").unwrap();
        vfs.write_at("f.bin", b"hello", 0).unwrap();
        vfs.write_at("f.bin", b"X", 10).unwrap();
        let mut buf = [0u8; 11];
        vfs.read_at("f.bin", &mut buf, 0).unwrap();
        assert_eq!(&buf[0..5], b"hello");
        assert_eq!(&buf[5..10], &[0, 0, 0, 0, 0]);
        assert_eq!(buf[10], b'X');
    }

    #[test]
    fn idb_double_meta_slot_alternation() {
        // 连续 rename → active_meta_slot 翻转 0↔1，generation 递增
        let vfs = new_idb();
        let gen0 = vfs.generation();
        let slot0 = vfs.active_meta_slot();

        vfs.create("a.bin").unwrap();
        vfs.write_at("a.bin", b"A", 0).unwrap();
        vfs.rename("a.bin", "b.bin").unwrap();
        let gen1 = vfs.generation();
        let slot1 = vfs.active_meta_slot();
        assert_eq!(gen1, gen0 + 1);
        assert_ne!(slot0, slot1);

        vfs.rename("b.bin", "c.bin").unwrap();
        let gen2 = vfs.generation();
        assert_eq!(gen2, gen1 + 1);
        assert_eq!(vfs.active_meta_slot(), slot0); // 翻回
    }
}
