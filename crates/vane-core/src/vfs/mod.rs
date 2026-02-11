use crate::types::Result;

/// SPEC §6.1 VFS trait（M0 冻结签名）。
/// core 对 IO 的全部认知仅此接口。
pub trait Vfs: Send + Sync {
    fn create(&self, path: &str) -> Result<()>;
    fn read_at(&self, path: &str, buf: &mut [u8], offset: u64) -> Result<usize>;
    fn write_at(&self, path: &str, buf: &[u8], offset: u64) -> Result<()>;
    fn append(&self, path: &str, buf: &[u8]) -> Result<u64>;
    fn sync(&self, path: &str) -> Result<()>;
    fn rename(&self, from: &str, to: &str) -> Result<()>;
    fn delete(&self, path: &str) -> Result<()>;
    fn list(&self, dir: &str) -> Result<Vec<String>>;
}

pub mod memory;
pub mod page_cache;
#[cfg(not(target_arch = "wasm32"))]
pub mod std_fs;
// M4 §3.1：FaultVfs 故障注入 VFS，仅 cfg(test) 或 feature="fault-injection" 编译。
// 绝不进生产/wasm 二进制（wasm32 check 不启此 feature，不设 test cfg）。
#[cfg(any(test, feature = "fault-injection"))]
pub mod fault;

#[cfg(test)]
mod tests;
