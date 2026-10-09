use crate::types::{Result, VaneError};
use crate::vfs::{DbExclusiveLock, Vfs};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;

/// Native 文件系统 VFS 后端。SPEC §6.1 四后端之一。
/// 这是 core crate 中唯一允许使用 std::fs 的模块（cfg 隔离，§13.3 例外）。
#[cfg(not(target_arch = "wasm32"))]
pub struct StdFsVfs {
    root: PathBuf,
    // P4 生产化缓存：已 create_dir_all 的父目录集合，避免每次 resolve 都 stat。
    // create_dir_all 本身幂等，此缓存仅减少重复系统调用。
    created_dirs: Mutex<HashSet<PathBuf>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl StdFsVfs {
    pub fn new() -> Self {
        Self {
            root: PathBuf::new(),
            created_dirs: Mutex::new(HashSet::new()),
        }
    }

    pub fn with_root(root: &str) -> Self {
        Self {
            root: PathBuf::from(root),
            created_dirs: Mutex::new(HashSet::new()),
        }
    }

    fn resolve(&self, path: &str) -> PathBuf {
        // 简化：路径相对于 root
        let p = self.root.join(path);
        // 确保父目录存在（缓存命中则跳过 create_dir_all）
        if let Some(parent) = p.parent() {
            if !parent.as_os_str().is_empty() {
                let mut cache = self.created_dirs.lock().unwrap();
                if cache.insert(parent.to_path_buf()) {
                    // 新插入（首次见到此目录）：实际建目录
                    drop(cache);
                    let _ = std::fs::create_dir_all(parent);
                }
            }
        }
        p
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Default for StdFsVfs {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Vfs for StdFsVfs {
    fn create(&self, path: &str) -> Result<()> {
        let p = self.resolve(path);
        if p.exists() {
            return Err(VaneError::Io(
                format!("file already exists: {}", path).into(),
            ));
        }
        std::fs::File::create(&p).map_err(io_err)?;
        Ok(())
    }

    fn read_at(&self, path: &str, buf: &mut [u8], offset: u64) -> Result<usize> {
        use std::io::{Read, Seek, SeekFrom};
        let p = self.resolve(path);
        let mut f = std::fs::File::open(&p).map_err(io_err)?;
        f.seek(SeekFrom::Start(offset)).map_err(io_err)?;
        f.read(buf).map_err(io_err)
    }

    fn write_at(&self, path: &str, buf: &[u8], offset: u64) -> Result<()> {
        use std::io::{Seek, SeekFrom, Write};
        let p = self.resolve(path);
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&p)
            .map_err(io_err)?;
        f.seek(SeekFrom::Start(offset)).map_err(io_err)?;
        f.write_all(buf).map_err(io_err)
    }

    fn append(&self, path: &str, buf: &[u8]) -> Result<u64> {
        use std::io::{Seek, SeekFrom, Write};
        let p = self.resolve(path);
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&p)
            .map_err(io_err)?;
        let offset = f.seek(SeekFrom::End(0)).map_err(io_err)?;
        f.write_all(buf).map_err(io_err)?;
        Ok(offset)
    }

    fn sync(&self, path: &str) -> Result<()> {
        let p = self.resolve(path);
        let f = std::fs::File::open(&p).map_err(io_err)?;
        f.sync_all().map_err(io_err)
    }

    fn rename(&self, from: &str, to: &str) -> Result<()> {
        let pf = self.resolve(from);
        let pt = self.resolve(to);
        // 原子覆盖：禁止先删目标。崩溃若落在 delete 与 rename 之间，manifest.json 会消失。
        #[cfg(windows)]
        {
            windows_replace(&pf, &pt)?;
        }
        #[cfg(not(windows))]
        {
            std::fs::rename(&pf, &pt).map_err(io_err)?;
            #[cfg(unix)]
            sync_renamed_parent(&pt);
        }
        Ok(())
    }

    fn delete(&self, path: &str) -> Result<()> {
        let p = self.resolve(path);
        if p.is_dir() {
            std::fs::remove_dir_all(&p).map_err(io_err)
        } else {
            std::fs::remove_file(&p).map_err(io_err)
        }
    }

    fn list(&self, dir: &str) -> Result<Vec<String>> {
        let p = self.resolve(dir);
        let mut out = Vec::new();
        let entries = std::fs::read_dir(&p).map_err(io_err)?;
        for entry in entries {
            let entry = entry.map_err(io_err)?;
            if let Some(name) = entry.file_name().to_str() {
                out.push(name.to_string());
            }
        }
        // 与 MemoryVfs::list 一致：返回有序结果（read_dir 顺序依赖平台/FS，
        // 排序后调用方可预测，conformance 测试用 .contains() 不受影响）。
        out.sort();
        Ok(out)
    }

    fn try_lock_exclusive(&self, db_path: &str) -> Result<Box<dyn DbExclusiveLock>> {
        // 已存在必须能打开，不能走 Vfs::create（create 在文件已存在时失败）。
        let path = self.resolve(&format!("{db_path}/.vane.lock"));
        let file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| VaneError::Io(e.to_string().into()))?;
        match file.try_lock() {
            Ok(()) => Ok(Box::new(StdFsExclusiveLock { file })),
            Err(std::fs::TryLockError::WouldBlock) => Err(crate::vfs::db_already_open()),
            Err(std::fs::TryLockError::Error(e)) => Err(VaneError::Io(e.to_string().into())),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
struct StdFsExclusiveLock {
    file: std::fs::File,
}

#[cfg(not(target_arch = "wasm32"))]
impl DbExclusiveLock for StdFsExclusiveLock {}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for StdFsExclusiveLock {
    fn drop(&mut self) {
        // unlock 失败也没关系：fd 关闭时操作系统会放锁。
        let _ = self.file.unlock();
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn io_err(e: std::io::Error) -> VaneError {
    if e.kind() == std::io::ErrorKind::NotFound {
        // 前缀固定，供 manifest 缺失判断识别（小写后含 "not found"）。
        VaneError::Io(format!("not found: {e}").into())
    } else {
        VaneError::Io(e.to_string().into())
    }
}

/// Unix：`rename` 本身原子覆盖。成功后 fsync 父目录；失败仍返回成功（调用方已看不到）。
#[cfg(all(unix, not(target_arch = "wasm32")))]
fn sync_renamed_parent(target: &std::path::Path) {
    let Some(parent) = target.parent() else {
        return;
    };
    if parent.as_os_str().is_empty() {
        return;
    }
    if let Ok(dir) = std::fs::File::open(parent) {
        let _ = dir.sync_all();
    }
}

/// Windows：`MoveFileExW` 覆盖已存在目标，不先删除。`WRITE_THROUGH` 在返回前落盘。
#[cfg(windows)]
fn windows_replace(from: &std::path::Path, to: &std::path::Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;

    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(
            lp_existing_file_name: *const u16,
            lp_new_file_name: *const u16,
            dw_flags: u32,
        ) -> i32;
    }

    let from_w: Vec<u16> = from
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let to_w: Vec<u16> = to
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: 两个缓冲区以 NUL 结尾，且在调用期间有效。
    let rc = unsafe {
        MoveFileExW(
            from_w.as_ptr(),
            to_w.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if rc == 0 {
        Err(io_err(std::io::Error::last_os_error()))
    } else {
        Ok(())
    }
}
