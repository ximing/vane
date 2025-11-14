//! VFS overlay + OPFS 后端（M2-02）。
//!
//! 路径 A（单 OPFS 容器 + 内存虚拟 FS overlay）：
//! - `overlay`：`MemOverlay` 内核（后端无关，M2-03 IDB 复用）+ `OverlayBackend` trait。
//! - `container`：容器格式（superblock / 双 meta_slot / CRC / data area）。
//! - `opfs`：`OpfsBackend`（SyncAccessHandle）+ `OpfsVfs`（feature = "opfs"）。
//!
//! `overlay` + `container` 纯 Rust（无 web-sys），可原生测试。
//! `opfs` feature-gated，仅 wasm32 编译通过（浏览器手动验证 M2-04）。

pub mod container;
#[cfg(feature = "opfs")]
pub mod opfs;
pub mod overlay;

#[cfg(feature = "opfs")]
pub use opfs::{OpfsBackend, OpfsVfs};
pub use overlay::{MemOverlay, MemoryBackend, OverlayBackend};
