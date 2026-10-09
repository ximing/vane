//! SPEC §11 Executor trait：并行搜索调度抽象。
//!
//! `cfg(target_arch)` **仅在本文件**（I-5 不变量核心）。`api/db.rs` / `api/collection.rs`
//! / `merge` 仅调 `default_executor()` 工厂与 `Executor` trait 方法，零 `cfg(target)`。
//!
//! - **native + `executor-native` feature**：`RayonExecutor` 包装 `rayon::scope`，
//!   多段搜索并行执行（SPEC §8.1 多段归并）。
//! - **wasm32 / 无 `executor-native` feature**：`SerialExecutor` 串行执行（立即调用），
//!   wasm32 无线程模型，单线程安全。
//!
//! 工厂 `default_executor()` 集中平台分支：
//! - `cfg(all(not(target_arch = "wasm32"), feature = "executor-native"))` → RayonExecutor
//! - 否则（wasm32 或 native 无 feature）→ SerialExecutor
//!
//! 空闲 auto-commit 的后台线程也只在本文件创建（I-5）。wasm32 不启动线程。
//!
//! 设计：`join_all` 接收 `Vec<Box<dyn FnOnce() + Send>>`（owned tasks，dyn-compatible，
//! 无生命周期约束——任务经 Arc clone 持有数据）。归并经调用方预分配 per-segment 结果槽，
//! join_all 后串行归并（I-2 双索引原子不破：段内 vector/text 结果同任务产出，跨段
//! 归并在 join_all 结束后串行合并）。

#[cfg(test)]
mod tests;

use std::sync::Arc;

/// 并行任务执行抽象（SPEC §11）。
///
/// `join_all` 阻塞至全部任务完成（rayon 并行 / 串行立即调用）。调用方预分配
/// per-segment 结果槽，任务写共享槽，join_all 返回后串行归并。
///
/// dyn-compatible：`join_all` 接收 boxed owned tasks（`Vec<Box<dyn FnOnce() + Send>>`），
/// 无 generic 参数，可经 `Arc<dyn Executor>` 持有（SPEC §11 `Db.executor` 字段）。
pub trait Executor: Send + Sync {
    /// 并行执行所有任务，阻塞至全部完成。
    fn join_all(&self, tasks: Vec<Box<dyn FnOnce() + Send>>);
}

// ---- native impl（cfg(not(target_arch="wasm32")) + feature executor-native）----

/// Rayon 并行执行器（native + `executor-native` feature）。
///
/// 包装 `rayon::scope`，多段搜索并行归并。无状态（rayon 全局线程池），
/// `Send + Sync` trivially。
#[cfg(all(not(target_arch = "wasm32"), feature = "executor-native"))]
pub struct RayonExecutor;

#[cfg(all(not(target_arch = "wasm32"), feature = "executor-native"))]
impl Executor for RayonExecutor {
    fn join_all(&self, tasks: Vec<Box<dyn FnOnce() + Send>>) {
        rayon::scope(|s| {
            for task in tasks {
                s.spawn(move |_| task());
            }
        });
    }
}

// ---- serial impl（wasm32 / native 无 executor-native）----

/// 串行执行器（wasm32 / 无 `executor-native` feature）。
///
/// `join_all` 顺序调用各 task。wasm32 无线程模型，单线程安全。
#[cfg(not(all(not(target_arch = "wasm32"), feature = "executor-native")))]
pub struct SerialExecutor;

#[cfg(not(all(not(target_arch = "wasm32"), feature = "executor-native")))]
impl Executor for SerialExecutor {
    fn join_all(&self, tasks: Vec<Box<dyn FnOnce() + Send>>) {
        for task in tasks {
            task();
        }
    }
}

// ---- 工厂：平台分支集中于此（I-5）----

/// 构造默认 Executor（SPEC §11，平台分支集中于此）。
///
/// - native + `executor-native` feature → `RayonExecutor`（rayon 并行）
/// - wasm32 / 无 feature → `SerialExecutor`（串行）
///
/// `api/db.rs` 仅调此工厂，不出现 `cfg(target)`。
pub fn default_executor() -> Arc<dyn Executor> {
    #[cfg(all(not(target_arch = "wasm32"), feature = "executor-native"))]
    {
        Arc::new(RayonExecutor)
    }
    #[cfg(not(all(not(target_arch = "wasm32"), feature = "executor-native")))]
    {
        Arc::new(SerialExecutor)
    }
}

// ---- 空闲提交线程（I-5：唯一允许 spawn 的地方）----

mod idle {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::Duration;

    #[cfg(not(target_arch = "wasm32"))]
    use std::thread::JoinHandle;

    struct Shared {
        stop: AtomicBool,
        epoch: AtomicU64,
        mu: Mutex<()>,
        cv: Condvar,
        /// 循环线程的 id。Drop 若发生在这条线程上，不能 join 自己。
        #[cfg(not(target_arch = "wasm32"))]
        owner: Mutex<Option<std::thread::ThreadId>>,
    }

    /// 克隆进每个 collection。`add()` 用它叫醒 Db 的空闲提交线程。
    #[derive(Clone)]
    pub(crate) struct IdleTicket {
        shared: Arc<Shared>,
    }

    impl IdleTicket {
        pub(crate) fn wake(&self) {
            // 与 wait 持同一把锁，避免「先改 epoch、后进 wait」丢掉这次唤醒。
            let _guard = self.shared.mu.lock().unwrap_or_else(|err| err.into_inner());
            self.shared.epoch.fetch_add(1, Ordering::SeqCst);
            self.shared.cv.notify_one();
        }

        pub(crate) fn stopped(&self) -> bool {
            self.shared.stop.load(Ordering::SeqCst)
        }

        pub(crate) fn epoch(&self) -> u64 {
            self.shared.epoch.load(Ordering::SeqCst)
        }

        /// `seen_epoch` 已变化、收到 stop，或等到 `timeout` 才返回。
        pub(crate) fn wait_until_changed(&self, seen_epoch: u64, timeout: Duration) {
            let guard = self.shared.mu.lock().unwrap_or_else(|err| err.into_inner());
            if self.shared.stop.load(Ordering::SeqCst)
                || self.shared.epoch.load(Ordering::SeqCst) != seen_epoch
            {
                return;
            }
            let _ = self.shared.cv.wait_timeout(guard, timeout);
        }
    }

    /// 每个 Db 一个。Drop 时停线程并 join。wasm 上 `start` 不跑闭包。
    pub(crate) struct IdleScheduler {
        shared: Arc<Shared>,
        #[cfg(not(target_arch = "wasm32"))]
        join: Mutex<Option<JoinHandle<()>>>,
    }

    impl IdleScheduler {
        pub(crate) fn new() -> Self {
            Self {
                shared: Arc::new(Shared {
                    stop: AtomicBool::new(false),
                    epoch: AtomicU64::new(0),
                    mu: Mutex::new(()),
                    cv: Condvar::new(),
                    #[cfg(not(target_arch = "wasm32"))]
                    owner: Mutex::new(None),
                }),
                #[cfg(not(target_arch = "wasm32"))]
                join: Mutex::new(None),
            }
        }

        pub(crate) fn ticket(&self) -> IdleTicket {
            IdleTicket {
                shared: Arc::clone(&self.shared),
            }
        }

        pub(crate) fn start<F>(&self, body: F)
        where
            F: FnOnce(IdleTicket) + Send + 'static,
        {
            #[cfg(not(target_arch = "wasm32"))]
            {
                let mut slot = self.join.lock().unwrap_or_else(|err| err.into_inner());
                if slot.is_some() {
                    return;
                }
                let ticket = self.ticket();
                let shared = Arc::clone(&self.shared);
                let handle = std::thread::Builder::new()
                    .name("vane-idle-commit".into())
                    .spawn(move || {
                        {
                            let mut owner =
                                shared.owner.lock().unwrap_or_else(|err| err.into_inner());
                            *owner = Some(std::thread::current().id());
                        }
                        body(ticket);
                    })
                    .expect("spawn vane-idle-commit");
                *slot = Some(handle);
            }
            #[cfg(target_arch = "wasm32")]
            {
                let _ = body;
            }
        }

        pub(crate) fn shutdown(&self) {
            {
                let _guard = self.shared.mu.lock().unwrap_or_else(|err| err.into_inner());
                self.shared.stop.store(true, Ordering::SeqCst);
                self.shared.cv.notify_one();
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                // 线程自己丢掉最后一个 Db 时，Drop 跑在这条线程上。join 自己会卡死。
                let is_self = self
                    .shared
                    .owner
                    .lock()
                    .unwrap_or_else(|err| err.into_inner())
                    .as_ref()
                    == Some(&std::thread::current().id());
                if !is_self {
                    if let Some(handle) = self
                        .join
                        .lock()
                        .unwrap_or_else(|err| err.into_inner())
                        .take()
                    {
                        let _ = handle.join();
                    }
                }
            }
        }
    }

    impl Drop for IdleScheduler {
        fn drop(&mut self) {
            self.shutdown();
        }
    }
}

pub(crate) use idle::{IdleScheduler, IdleTicket};
