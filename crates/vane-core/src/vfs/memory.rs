use crate::types::{Result, VaneError};
use crate::vfs::Vfs;
use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::RwLock;

/// 纯内存 VFS 后端（测试/纯内存场景）。SPEC §6.1 四后端之一。
pub struct MemoryVfs {
    files: RwLock<HashMap<String, Vec<u8>>>,
    #[allow(dead_code)]
    seq: AtomicU64,
}

impl MemoryVfs {
    pub fn new() -> Self {
        Self {
            files: RwLock::new(HashMap::new()),
            seq: AtomicU64::new(0),
        }
    }
}

impl Default for MemoryVfs {
    fn default() -> Self {
        Self::new()
    }
}

impl Vfs for MemoryVfs {
    fn create(&self, path: &str) -> Result<()> {
        let mut files = self.files.write().unwrap();
        if files.contains_key(path) {
            return Err(VaneError::Io(
                format!("file already exists: {}", path).into(),
            ));
        }
        files.insert(path.to_string(), Vec::new());
        Ok(())
    }

    fn read_at(&self, path: &str, buf: &mut [u8], offset: u64) -> Result<usize> {
        let files = self.files.read().unwrap();
        let file = files
            .get(path)
            .ok_or_else(|| VaneError::Io(format!("file not found: {}", path).into()))?;
        let off = offset as usize;
        if off >= file.len() {
            return Ok(0);
        }
        let avail = file.len() - off;
        let n = buf.len().min(avail);
        buf[..n].copy_from_slice(&file[off..off + n]);
        Ok(n)
    }

    fn write_at(&self, path: &str, buf: &[u8], offset: u64) -> Result<()> {
        let mut files = self.files.write().unwrap();
        let file = files
            .get_mut(path)
            .ok_or_else(|| VaneError::Io(format!("file not found: {}", path).into()))?;
        let off = offset as usize;
        let end = off + buf.len();
        if file.len() < end {
            file.resize(end, 0);
        }
        file[off..end].copy_from_slice(buf);
        Ok(())
    }

    fn append(&self, path: &str, buf: &[u8]) -> Result<u64> {
        let mut files = self.files.write().unwrap();
        let file = files
            .get_mut(path)
            .ok_or_else(|| VaneError::Io(format!("file not found: {}", path).into()))?;
        let offset = file.len() as u64;
        file.extend_from_slice(buf);
        Ok(offset)
    }

    fn sync(&self, _path: &str) -> Result<()> {
        Ok(()) // 内存无需 sync
    }

    fn rename(&self, from: &str, to: &str) -> Result<()> {
        let mut files = self.files.write().unwrap();
        let data = files
            .remove(from)
            .ok_or_else(|| VaneError::Io(format!("file not found: {}", from).into()))?;
        files.insert(to.to_string(), data);
        Ok(())
    }

    fn delete(&self, path: &str) -> Result<()> {
        let mut files = self.files.write().unwrap();
        files
            .remove(path)
            .ok_or_else(|| VaneError::Io(format!("file not found: {}", path).into()))?;
        Ok(())
    }

    fn list(&self, dir: &str) -> Result<Vec<String>> {
        // 按 dir 前缀过滤，仅返回下一层路径分量（与 StdFsVfs::list 的 read_dir 语义一致）。
        // 即：对 dir="."，文件 "sub/x.bin" 贡献条目 "sub"（目录名），而非完整路径。
        let prefix = if dir == "." || dir.is_empty() {
            String::new()
        } else {
            format!("{}/", dir.trim_end_matches('/'))
        };
        let files = self.files.read().unwrap();
        let mut out: Vec<String> = files
            .keys()
            .filter(|path| path.starts_with(&prefix))
            .filter_map(|path| {
                path[prefix.len()..]
                    .split('/')
                    .next()
                    .map(|s| s.to_string())
            })
            .collect();
        out.sort();
        out.dedup();
        Ok(out)
    }
}

// seq 字段为未来生成唯一路径预留（M0 未消费），用 #[allow(dead_code)] 避免告警。
