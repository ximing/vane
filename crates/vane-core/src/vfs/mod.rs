use crate::types::{ErrorContext, Result, VaneError};

/// 只有「文件不存在」类 Io 才当成缺失。
/// `os error 2` 不能用裸子串，否则 `os error 20` / `os error 21` 会误判。
pub(crate) fn io_error_is_not_found(err: &VaneError) -> bool {
    let VaneError::Io(ctx) = err else {
        return false;
    };
    let msg = ctx.message.to_lowercase();
    msg.contains("not found")
        || msg.contains("no such file")
        || msg.contains("os error 2)")
        || msg.ends_with("os error 2")
}

/// 独占打开锁。`Sync` 使 `Arc<dyn DbExclusiveLock>` 能放进 `Send + Sync` 的 `Db`。
/// 具体锁在最后一个 `Arc` drop 时释放；本 trait 没有方法。
pub trait DbExclusiveLock: Send + Sync {}

/// WASM 等未覆盖 `try_lock_exclusive` 的后端使用的空锁。
pub struct NoopDbLock;

impl DbExclusiveLock for NoopDbLock {}

impl Drop for NoopDbLock {
    fn drop(&mut self) {}
}

pub(crate) fn db_already_open() -> VaneError {
    VaneError::Busy(
        ErrorContext::new("database is already open")
            .op("open")
            .hint("关掉另一个占用该目录的进程"),
    )
}

/// SPEC §6.1 VFS trait（M0 冻结签名）。
/// core 对 IO 的全部认知仅此接口。
/// `try_lock_exclusive` 有默认实现，已有 `impl Vfs` 不改也能编译。
pub trait Vfs: Send + Sync {
    fn create(&self, path: &str) -> Result<()>;
    fn read_at(&self, path: &str, buf: &mut [u8], offset: u64) -> Result<usize>;
    fn write_at(&self, path: &str, buf: &[u8], offset: u64) -> Result<()>;
    fn append(&self, path: &str, buf: &[u8]) -> Result<u64>;
    fn sync(&self, path: &str) -> Result<()>;
    fn rename(&self, from: &str, to: &str) -> Result<()>;
    fn delete(&self, path: &str) -> Result<()>;
    fn list(&self, dir: &str) -> Result<Vec<String>>;

    /// 独占打开 `db_path`。默认不锁（WASM OPFS/IDB）。
    fn try_lock_exclusive(&self, db_path: &str) -> Result<Box<dyn DbExclusiveLock>> {
        let _ = db_path;
        Ok(Box::new(NoopDbLock))
    }
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
