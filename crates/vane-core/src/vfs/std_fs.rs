use crate::types::{Result, VaneError};
use crate::vfs::Vfs;
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
        // 跨平台一致覆盖：先删目标（忽略不存在），再 rename（S11）
        let _ = std::fs::remove_file(&pt);
        std::fs::rename(&pf, &pt).map_err(io_err)
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
}

#[cfg(not(target_arch = "wasm32"))]
fn io_err(e: std::io::Error) -> VaneError {
    VaneError::Io(e.to_string().into())
}
