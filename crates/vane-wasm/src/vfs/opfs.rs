//! OpfsVfs——OPFS 同步 VFS 后端（M2-02，feature = "opfs"）。
//!
//! Worker init 异步获取 `FileSystemSyncAccessHandle` 后传入 `OpfsVfs::from_handle`，
//! 此后全部 Vfs 方法基于该同步句柄操作单容器 `vane.db` 内字节区间。
//! 唯一 await 点在 Worker init（M2-04 §4.7 异步序列），core 内部全同步。
//!
//! 薄层：`OpfsBackend` impl `OverlayBackend`（read/write/flush/size/truncate），
//! 逻辑全部委托 `MemOverlay`（I-8 binding 薄壳）。

use std::sync::Arc;

use vane_core::types::{Result, VaneError};
use vane_core::vfs::Vfs;
use wasm_bindgen::JsValue;

use super::overlay::{MemOverlay, OverlayBackend};

// ── OpfsBackend ────────────────────────────────────────────────────────────

/// OPFS 后端：`FileSystemSyncAccessHandle` 的薄封装。
///
/// `FileSystemSyncAccessHandle` 包含 `JsValue`（!Send + !Sync），但 wasm32
/// 单线程环境下无并发风险，通过 `unsafe impl Send/Sync` 满足 trait 约束。
/// 这与 wasm-bindgen 生态的通行做法一致（单线程 Worker 内 JS 对象安全共享）。
pub struct OpfsBackend {
    sah: web_sys::FileSystemSyncAccessHandle,
}

// SAFETY: wasm32 单线程环境下（wasm32-unknown-unknown，无 atomics），
// FileSystemSyncAccessHandle 仅在单个 Worker 线程内使用，无跨线程访问风险。
// Send/Sync impl 仅为满足 `Arc<dyn OverlayBackend>` 的 trait 约束。
unsafe impl Send for OpfsBackend {}
unsafe impl Sync for OpfsBackend {}

impl OpfsBackend {
    pub fn new(sah: web_sys::FileSystemSyncAccessHandle) -> Self {
        Self { sah }
    }
}

fn js_err(e: JsValue) -> VaneError {
    VaneError::Io(format!("OPFS error: {:?}", e).into())
}

impl OverlayBackend for OpfsBackend {
    fn read(&self, off: u64, buf: &mut [u8]) -> Result<usize> {
        let opts = web_sys::FileSystemReadWriteOptions::new();
        opts.set_at(off as f64);
        let n = self
            .sah
            .read_with_u8_array_and_options(buf, &opts)
            .map_err(js_err)?;
        Ok(n as usize)
    }

    fn write(&self, off: u64, buf: &[u8]) -> Result<()> {
        let opts = web_sys::FileSystemReadWriteOptions::new();
        opts.set_at(off as f64);
        self.sah
            .write_with_u8_array_and_options(buf, &opts)
            .map_err(js_err)?;
        Ok(())
    }

    fn flush(&self) -> Result<()> {
        self.sah.flush().map_err(js_err)
    }

    fn size(&self) -> Result<u64> {
        let s = self.sah.get_size().map_err(js_err)?;
        Ok(s as u64)
    }

    fn truncate(&self, sz: u64) -> Result<()> {
        // SyncAccessHandle.truncate 接受 f64（JS Number），无 4GB u32 限制。
        self.sah.truncate_with_f64(sz as f64).map_err(js_err)
    }
}

// ── OpfsVfs ────────────────────────────────────────────────────────────────

/// OPFS VFS 后端：单容器 + 内存 overlay。
///
/// 持有 `MemOverlay`（包含 `OpfsBackend`），实现 `Vfs` trait 8 方法（全同步）。
/// Worker init 异步获取 `FileSystemSyncAccessHandle` 后通过 `from_handle` 构造。
pub struct OpfsVfs {
    overlay: MemOverlay,
}

impl OpfsVfs {
    /// 从 `FileSystemSyncAccessHandle` 构造（唯一构造路径）。
    /// Worker init 异步获取句柄后调用，此后进入同步 Vfs 世界。
    pub fn from_handle(sah: web_sys::FileSystemSyncAccessHandle) -> Result<Self> {
        let backend: Arc<dyn OverlayBackend> = Arc::new(OpfsBackend::new(sah));
        let overlay = MemOverlay::open(backend)?;
        Ok(Self { overlay })
    }
}

impl Vfs for OpfsVfs {
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
