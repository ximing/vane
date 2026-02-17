//! FaultVfs：故障注入 VFS 包装（M4 阶段二 a，设计 §3.1）。
//!
//! 在任意 inner Vfs 上做透明包装，按 (path_pattern, op, 调用计数) 三层匹配
//! 注入可控故障，供崩溃恢复测试精确模拟 IO 错误 / 部分写 / ENOSPC / 延迟。
//! `cfg(test)` 或 `feature="fault-injection"` 启用，**绝不进生产/wasm 二进制**。
//! 不引 regex（黑名单），path matcher 自研轻量 glob（`*` 通配）。

use crate::types::{Result, VaneError};
use crate::vfs::memory::MemoryVfs;
use crate::vfs::Vfs;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// 故障规则。按 (path_pattern, op) 匹配，触发后消费（one_shot）或持久（每次命中）。
///
/// - `trigger_on_nth`：0 = 每次匹配都触发（无计数门控）；N>0 = 仅在该规则第 N 次匹配时触发。
/// - `one_shot`：true = 触发后从规则表移除；false = 触发后保留（持久故障）。
// TODO(M4-Could): LostWrite 真实丢写模拟，暂用 sync 失败注入近似（见 phase0-design.md §3.1 取舍）
#[derive(Debug, Clone)]
pub enum Fault {
    /// 指定 op 在匹配 path_pattern 时返 `VaneError::Io(msg)`。
    IoError {
        op: VfsOp,
        path_pattern: String,
        msg: String,
        one_shot: bool,
        trigger_on_nth: u32,
    },
    /// write_at/append 写 `bytes_before_fail` 字节后返 Err（模拟中途失败）。
    /// FaultVfs 先写前 min(N, buf.len()) 字节到 inner，再返错。
    PartialWrite {
        op: VfsOp,
        path_pattern: String,
        bytes_before_fail: usize,
        one_shot: bool,
        trigger_on_nth: u32,
    },
    /// write_at/append 返 `VaneError::Io("ENOSPC...")`，不写任何字节到 inner。
    Enospc {
        op: VfsOp,
        path_pattern: String,
        one_shot: bool,
        trigger_on_nth: u32,
    },
    /// 注入延迟（ms）。仅影响时序，不影响正确性（延迟后正常转发 inner）。
    Delay {
        op: VfsOp,
        path_pattern: String,
        ms: u64,
        one_shot: bool,
        trigger_on_nth: u32,
    },
}

impl Fault {
    fn op(&self) -> VfsOp {
        match self {
            Fault::IoError { op, .. }
            | Fault::PartialWrite { op, .. }
            | Fault::Enospc { op, .. }
            | Fault::Delay { op, .. } => *op,
        }
    }

    fn path_pattern(&self) -> &str {
        match self {
            Fault::IoError { path_pattern, .. }
            | Fault::PartialWrite { path_pattern, .. }
            | Fault::Enospc { path_pattern, .. }
            | Fault::Delay { path_pattern, .. } => path_pattern,
        }
    }

    fn one_shot(&self) -> bool {
        match self {
            Fault::IoError { one_shot, .. }
            | Fault::PartialWrite { one_shot, .. }
            | Fault::Enospc { one_shot, .. }
            | Fault::Delay { one_shot, .. } => *one_shot,
        }
    }

    fn trigger_on_nth(&self) -> u32 {
        match self {
            Fault::IoError { trigger_on_nth, .. }
            | Fault::PartialWrite { trigger_on_nth, .. }
            | Fault::Enospc { trigger_on_nth, .. }
            | Fault::Delay { trigger_on_nth, .. } => *trigger_on_nth,
        }
    }

    fn to_action(&self) -> FaultAction {
        match self {
            Fault::IoError { msg, .. } => FaultAction::ReturnErr(VaneError::Io(msg.clone().into())),
            Fault::PartialWrite {
                bytes_before_fail, ..
            } => FaultAction::PartialWrite(*bytes_before_fail),
            Fault::Enospc { .. } => FaultAction::Enospc,
            Fault::Delay { ms, .. } => FaultAction::DelayMs(*ms),
        }
    }
}

/// Vfs 操作分类（用于故障规则匹配）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VfsOp {
    Create,
    ReadAt,
    WriteAt,
    Append,
    Sync,
    Rename,
    Delete,
    List,
}

/// 故障命中后 FaultVfs 内部执行的动作（私有，仅 Vfs impl 消费）。
enum FaultAction {
    /// 直接返回该错误（不调 inner，inner 状态不变）。
    ReturnErr(VaneError),
    /// 先写前 N 字节到 inner，再返 Err（仅 write_at/append 适用）。
    PartialWrite(usize),
    /// 返 ENOSPC 错误，不写任何字节（仅 write_at/append 适用）。
    Enospc,
    /// 延迟 ms 毫秒后正常转发 inner。
    DelayMs(u64),
}

/// 故障注入 VFS。包装 inner Vfs，按规则表注入故障。
///
/// 匹配机制（设计 §3.1 层 1+2）：
/// - 层 1：path_pattern（glob `*` 通配，匹配整段 path）+ op 匹配。
/// - 层 2：每条规则维护调用计数器；`trigger_on_nth` 控制第几次匹配触发。
///
/// one_shot 规则触发后从表移除；持久规则触发后保留。
/// `check_fault` 在调 inner 前执行，返错则不调 inner，保证 inner 状态不变。
pub struct FaultVfs {
    inner: Arc<dyn Vfs>,
    faults: Mutex<Vec<Fault>>,
    /// 调用计数器（层 2）：key = (op, path_pattern)，value = 累计匹配次数。
    /// 与 faults 分离以保持 `faults: Mutex<Vec<Fault>>` 签名（设计 §3.1）。
    call_counts: Mutex<HashMap<(VfsOp, String), u32>>,
}

impl FaultVfs {
    pub fn new(inner: Arc<dyn Vfs>) -> Self {
        Self {
            inner,
            faults: Mutex::new(Vec::new()),
            call_counts: Mutex::new(HashMap::new()),
        }
    }

    /// 便捷构造：包装 MemoryVfs（崩溃恢复测试主力）。
    pub fn wrap_memory() -> Self {
        Self::new(Arc::new(MemoryVfs::new()))
    }

    /// 注册故障规则（链式）。测试用：`vfs.inject(Fault::IoError{...}).inject(...)`。
    pub fn inject(&self, fault: Fault) -> &Self {
        self.faults.lock().unwrap().push(fault);
        self
    }

    /// 检查是否命中故障，命中返 `Some(FaultAction)`。
    ///
    /// 遍历规则表（注册顺序），对每条 (path_pattern, op) 匹配的规则：
    /// 1. 递增其计数器。
    /// 2. 若 `trigger_on_nth == 0` 或 `计数器 == trigger_on_nth` → 触发，返动作。
    /// 3. first-fire-wins：首条触发的规则返回后停止（之前的非触发规则仍递增计数器）。
    ///
    /// one_shot 规则触发后从表移除。
    fn check_fault(&self, op: VfsOp, path: &str) -> Option<FaultAction> {
        let mut faults = self.faults.lock().unwrap();
        let mut counts = self.call_counts.lock().unwrap();
        let mut fire_idx: Option<usize> = None;
        let mut action: Option<FaultAction> = None;
        for (i, fault) in faults.iter().enumerate() {
            if fault.op() != op || !glob_match(fault.path_pattern(), path) {
                continue;
            }
            // 层 2：递增计数器。
            let key = (op, fault.path_pattern().to_string());
            let count = counts.entry(key).or_insert(0);
            *count += 1;
            let should_fire = fault.trigger_on_nth() == 0 || *count == fault.trigger_on_nth();
            if should_fire {
                action = Some(fault.to_action());
                fire_idx = Some(i);
                break;
            }
        }
        if let Some(idx) = fire_idx {
            let is_one_shot = faults[idx].one_shot();
            if is_one_shot {
                faults.remove(idx);
            }
        }
        action
    }
}

impl Vfs for FaultVfs {
    fn create(&self, path: &str) -> Result<()> {
        match self.check_fault(VfsOp::Create, path) {
            Some(FaultAction::ReturnErr(e)) => return Err(e),
            Some(FaultAction::DelayMs(ms)) => sleep_ms(ms),
            // PartialWrite/Enospc 对 create 无写入语义，忽略并转发 inner。
            _ => {}
        }
        self.inner.create(path)
    }

    fn read_at(&self, path: &str, buf: &mut [u8], offset: u64) -> Result<usize> {
        match self.check_fault(VfsOp::ReadAt, path) {
            Some(FaultAction::ReturnErr(e)) => return Err(e),
            Some(FaultAction::DelayMs(ms)) => sleep_ms(ms),
            _ => {}
        }
        self.inner.read_at(path, buf, offset)
    }

    fn write_at(&self, path: &str, buf: &[u8], offset: u64) -> Result<()> {
        match self.check_fault(VfsOp::WriteAt, path) {
            Some(FaultAction::ReturnErr(e)) => return Err(e),
            Some(FaultAction::PartialWrite(n)) => {
                let n = n.min(buf.len());
                if n > 0 {
                    self.inner.write_at(path, &buf[..n], offset)?;
                }
                return Err(VaneError::Io(
                    format!(
                        "partial write at {} ({} bytes written before failure)",
                        path, n
                    )
                    .into(),
                ));
            }
            Some(FaultAction::Enospc) => {
                return Err(VaneError::Io(
                    format!("ENOSPC: write_at {} (simulated, no bytes written)", path).into(),
                ));
            }
            Some(FaultAction::DelayMs(ms)) => sleep_ms(ms),
            None => {}
        }
        self.inner.write_at(path, buf, offset)
    }

    fn append(&self, path: &str, buf: &[u8]) -> Result<u64> {
        match self.check_fault(VfsOp::Append, path) {
            Some(FaultAction::ReturnErr(e)) => return Err(e),
            Some(FaultAction::PartialWrite(n)) => {
                let n = n.min(buf.len());
                if n > 0 {
                    self.inner.append(path, &buf[..n])?;
                }
                return Err(VaneError::Io(
                    format!(
                        "partial append at {} ({} bytes written before failure)",
                        path, n
                    )
                    .into(),
                ));
            }
            Some(FaultAction::Enospc) => {
                return Err(VaneError::Io(
                    format!("ENOSPC: append {} (simulated, no bytes written)", path).into(),
                ));
            }
            Some(FaultAction::DelayMs(ms)) => sleep_ms(ms),
            None => {}
        }
        self.inner.append(path, buf)
    }

    fn sync(&self, path: &str) -> Result<()> {
        match self.check_fault(VfsOp::Sync, path) {
            Some(FaultAction::ReturnErr(e)) => return Err(e),
            Some(FaultAction::DelayMs(ms)) => sleep_ms(ms),
            _ => {}
        }
        self.inner.sync(path)
    }

    fn rename(&self, from: &str, to: &str) -> Result<()> {
        // manifest 原子切换关键：check_fault(Rename, from) 匹配 tmp 路径。
        match self.check_fault(VfsOp::Rename, from) {
            Some(FaultAction::ReturnErr(e)) => return Err(e),
            Some(FaultAction::DelayMs(ms)) => sleep_ms(ms),
            _ => {}
        }
        self.inner.rename(from, to)
    }

    fn delete(&self, path: &str) -> Result<()> {
        match self.check_fault(VfsOp::Delete, path) {
            Some(FaultAction::ReturnErr(e)) => return Err(e),
            Some(FaultAction::DelayMs(ms)) => sleep_ms(ms),
            _ => {}
        }
        self.inner.delete(path)
    }

    fn list(&self, dir: &str) -> Result<Vec<String>> {
        match self.check_fault(VfsOp::List, dir) {
            Some(FaultAction::ReturnErr(e)) => return Err(e),
            Some(FaultAction::DelayMs(ms)) => sleep_ms(ms),
            _ => {}
        }
        self.inner.list(dir)
    }
}

/// 轻量 glob 匹配：`*` 匹配任意序列（含 `/`，零或多个字符），无其他特殊字符。
/// 自研实现，不引 regex（黑名单）。
fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let m = p.len();
    let n = t.len();
    // dp[i][j] = true 表示 pattern[0..i] 匹配 text[0..j]。
    let mut dp = vec![vec![false; n + 1]; m + 1];
    dp[0][0] = true;
    for i in 1..=m {
        if p[i - 1] == '*' {
            // '*' 匹配空（dp[i-1][j]）或延续前一字符（dp[i][j-1]）。
            dp[i][0] = dp[i - 1][0];
            for j in 1..=n {
                dp[i][j] = dp[i - 1][j] || dp[i][j - 1];
            }
        } else {
            for j in 1..=n {
                dp[i][j] = p[i - 1] == t[j - 1] && dp[i - 1][j - 1];
            }
        }
    }
    dp[m][n]
}

/// 延迟注入。wasm32 无线程模型，FaultVfs 不进 wasm 生产，此处 no-op 守护。
fn sleep_ms(ms: u64) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        if ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(ms));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vfs::Vfs;

    #[test]
    fn io_error_one_shot_consumed() {
        // IoError one_shot=true：第一次调用返 Err，第二次（规则已消费）返 Ok。
        let vfs = FaultVfs::wrap_memory();
        vfs.create("db/wal.log").unwrap();
        vfs.inject(Fault::IoError {
            op: VfsOp::Sync,
            path_pattern: "*/wal.log".to_string(),
            msg: "sync failed".to_string(),
            one_shot: true,
            trigger_on_nth: 0,
        });
        // 第一次 sync → Err
        assert!(
            vfs.sync("db/wal.log").is_err(),
            "first sync should fail (fault fires)"
        );
        // 第二次 sync → Ok（one_shot 规则已消费）
        assert!(
            vfs.sync("db/wal.log").is_ok(),
            "second sync should succeed (one_shot consumed)"
        );
        // 规则表已空
        assert!(
            vfs.faults.lock().unwrap().is_empty(),
            "fault table should be empty after one_shot consumed"
        );
    }

    #[test]
    fn trigger_on_nth_fires_on_nth() {
        // trigger_on_nth=3：前 2 次返 Ok，第 3 次返 Err，第 4 次返 Ok（已消费）。
        let vfs = FaultVfs::wrap_memory();
        vfs.create("db/wal.log").unwrap();
        vfs.inject(Fault::IoError {
            op: VfsOp::Sync,
            path_pattern: "db/wal.log".to_string(),
            msg: "third sync fails".to_string(),
            one_shot: true,
            trigger_on_nth: 3,
        });
        assert!(vfs.sync("db/wal.log").is_ok(), "1st sync: Ok (count=1)");
        assert!(vfs.sync("db/wal.log").is_ok(), "2nd sync: Ok (count=2)");
        assert!(
            vfs.sync("db/wal.log").is_err(),
            "3rd sync: Err (count=3, trigger_on_nth reached)"
        );
        assert!(
            vfs.sync("db/wal.log").is_ok(),
            "4th sync: Ok (one_shot consumed after firing)"
        );
    }

    #[test]
    fn partial_write_writes_n_bytes_then_err() {
        // PartialWrite：写前 N 字节到 inner，再返 Err。inner 恰好 N 字节。
        let vfs = FaultVfs::wrap_memory();
        vfs.create("db/header.bin").unwrap();
        vfs.inject(Fault::PartialWrite {
            op: VfsOp::WriteAt,
            path_pattern: "db/header.bin".to_string(),
            bytes_before_fail: 8,
            one_shot: true,
            trigger_on_nth: 0,
        });
        let buf = [0xAA_u8; 32];
        assert!(
            vfs.write_at("db/header.bin", &buf, 0).is_err(),
            "write_at should fail after partial write"
        );
        // 故障已消费（one_shot），read_at 透传 inner → 恰好 8 字节
        let mut read_buf = [0u8; 32];
        let n = vfs.read_at("db/header.bin", &mut read_buf, 0).unwrap();
        assert_eq!(
            n, 8,
            "inner should have exactly 8 bytes written before failure"
        );
        assert!(read_buf[..8].iter().all(|&b| b == 0xAA));
        assert!(
            read_buf[8..].iter().all(|&b| b == 0),
            "bytes beyond partial write should be zero"
        );
    }

    #[test]
    fn enospc_returns_err_inner_unchanged() {
        // Enospc：返 Err，不写任何字节，inner 保持基线不变。
        let vfs = FaultVfs::wrap_memory();
        vfs.create("db/data.bin").unwrap();
        vfs.write_at("db/data.bin", &[1, 2, 3, 4], 0).unwrap();
        vfs.inject(Fault::Enospc {
            op: VfsOp::WriteAt,
            path_pattern: "db/data.bin".to_string(),
            one_shot: true,
            trigger_on_nth: 0,
        });
        assert!(
            vfs.write_at("db/data.bin", &[0xFF; 16], 0).is_err(),
            "write_at should fail with ENOSPC"
        );
        // 故障已消费，read_at 透传 → inner 不变（4 字节基线）
        let mut read_buf = [0u8; 16];
        let n = vfs.read_at("db/data.bin", &mut read_buf, 0).unwrap();
        assert_eq!(n, 4, "inner unchanged: still 4 bytes baseline");
        assert_eq!(&read_buf[..4], &[1, 2, 3, 4]);
    }

    #[test]
    fn path_matcher_star_and_prefix() {
        // `*` 通配：匹配任意序列（含 /，零或多个字符）。
        assert!(glob_match("*", "anything"));
        assert!(glob_match("*", ""));
        assert!(glob_match("*/wal.log", "db/wal.log"));
        assert!(glob_match("*/wal.log", "db/nested/wal.log"));
        assert!(glob_match("db/*", "db/wal.log"));
        assert!(glob_match("db/*", "db/segments/seg_1/header.bin"));
        // 精确匹配（无 *）：需整段一致
        assert!(glob_match("db/wal.log", "db/wal.log"));
        assert!(!glob_match("db/wal", "db/wal.log"));
        // 通配中间段：
        assert!(glob_match(
            "*/segments/*/inverted.bin",
            "db/segments/seg_1/inverted.bin"
        ));
        // 多段 *：
        assert!(glob_match("*/*", "a/b"));
        assert!(
            !glob_match("*/*", "a"),
            "single segment should not match */*"
        );
    }

    #[test]
    fn non_matching_path_passes_through_inner() {
        // 注入的 pattern 不匹配实际 path → inner 正常执行，规则不被消费。
        let vfs = FaultVfs::wrap_memory();
        vfs.inject(Fault::IoError {
            op: VfsOp::WriteAt,
            path_pattern: "*/foo.bin".to_string(),
            msg: "should not fire".to_string(),
            one_shot: true,
            trigger_on_nth: 0,
        });
        // 对非匹配 path 写入 → Ok（inner 正常）
        vfs.create("db/bar.bin").unwrap();
        assert!(vfs.write_at("db/bar.bin", &[1, 2, 3], 0).is_ok());
        // 验证 inner 确实写了
        let mut buf = [0u8; 3];
        vfs.read_at("db/bar.bin", &mut buf, 0).unwrap();
        assert_eq!(&buf, &[1, 2, 3]);
        // 故障规则仍在（未命中未消费）
        assert_eq!(
            vfs.faults.lock().unwrap().len(),
            1,
            "non-matching fault rule should remain registered"
        );
    }

    #[test]
    fn persistent_io_error_fires_every_call() {
        // one_shot=false + trigger_on_nth=0：每次匹配都触发，规则不被移除。
        let vfs = FaultVfs::wrap_memory();
        vfs.create("db/wal.log").unwrap();
        vfs.inject(Fault::IoError {
            op: VfsOp::Sync,
            path_pattern: "*/wal.log".to_string(),
            msg: "persistent sync fail".to_string(),
            one_shot: false,
            trigger_on_nth: 0,
        });
        for i in 0..5 {
            assert!(
                vfs.sync("db/wal.log").is_err(),
                "call #{} should fail (persistent fault)",
                i
            );
        }
        // 规则仍在（持久）
        assert!(
            !vfs.faults.lock().unwrap().is_empty(),
            "persistent fault should not be removed"
        );
    }

    #[test]
    fn rename_fault_blocks_and_inner_unchanged() {
        // rename 失败注入（manifest 原子切换前）：inner 不被 rename，状态不变。
        let vfs = FaultVfs::wrap_memory();
        vfs.create("db/manifest.json").unwrap();
        vfs.write_at("db/manifest.json", b"OLD", 0).unwrap();
        vfs.create("db/manifest.json.tmp").unwrap();
        vfs.write_at("db/manifest.json.tmp", b"NEW", 0).unwrap();
        vfs.inject(Fault::IoError {
            op: VfsOp::Rename,
            path_pattern: "*.tmp".to_string(),
            msg: "rename failed".to_string(),
            one_shot: true,
            trigger_on_nth: 0,
        });
        // rename 应失败（check_fault 返错前不调 inner）
        assert!(
            vfs.rename("db/manifest.json.tmp", "db/manifest.json")
                .is_err(),
            "rename should fail (fault injected)"
        );
        // inner 不变：target 仍是旧内容，tmp 仍存在
        let mut buf = [0u8; 3];
        vfs.read_at("db/manifest.json", &mut buf, 0).unwrap();
        assert_eq!(&buf, b"OLD", "target content unchanged (rename blocked)");
        let mut buf2 = [0u8; 3];
        vfs.read_at("db/manifest.json.tmp", &mut buf2, 0).unwrap();
        assert_eq!(&buf2, b"NEW", "tmp still exists with new content");
    }
}
