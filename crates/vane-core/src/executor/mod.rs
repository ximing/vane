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
