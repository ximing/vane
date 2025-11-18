//! Executor trait 测试（SPEC §11，测试清单 1/2/10/11）。
//!
//! - RayonExecutor: join_all 并行执行多任务，结果归并正确（native + executor-native）。
//! - SerialExecutor: join_all 串行执行，等价单线程。
//! - 多段并行搜索归并：与串行一致（在 collection 集成测试覆盖；此处 unit 验 Executor 语义）。

use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

#[test]
#[cfg(all(not(target_arch = "wasm32"), feature = "executor-native"))]
fn rayon_join_all_parallel() {
    // 测试清单 1：RayonExecutor::join_all 并行执行多任务，结果归并正确。
    let exec = RayonExecutor;
    let counter = Arc::new(AtomicUsize::new(0));
    let results: Vec<Arc<Mutex<Vec<u32>>>> =
        (0..8).map(|_| Arc::new(Mutex::new(Vec::new()))).collect();

    let tasks: Vec<Box<dyn FnOnce() + Send>> = (0..8u32)
        .map(|i| {
            let results_clone = results[i as usize].clone();
            let counter = counter.clone();
            Box::new(move || {
                let mut v = Vec::new();
                for j in 0..100u32 {
                    v.push(i * 1000 + j);
                }
                *results_clone.lock().unwrap() = v;
                counter.fetch_add(1, Ordering::SeqCst);
            }) as Box<dyn FnOnce() + Send>
        })
        .collect();

    exec.join_all(tasks);

    // join_all 返回后全部任务完成
    assert_eq!(counter.load(Ordering::SeqCst), 8);
    // 归并结果
    let mut merged: Vec<u32> = Vec::new();
    for r in &results {
        merged.extend_from_slice(&r.lock().unwrap());
    }
    assert_eq!(merged.len(), 800);
    // 验证各段数据正确
    for i in 0..8u32 {
        for j in 0..100u32 {
            assert!(merged.contains(&(i * 1000 + j)));
        }
    }
}

#[test]
#[cfg(all(not(target_arch = "wasm32"), feature = "executor-native"))]
fn rayon_join_all_completes_all() {
    let exec = RayonExecutor;
    let acc = Arc::new(AtomicUsize::new(0));
    let tasks: Vec<Box<dyn FnOnce() + Send>> = (0..4)
        .map(|_| {
            let acc = acc.clone();
            Box::new(move || {
                acc.fetch_add(1, Ordering::SeqCst);
            }) as Box<dyn FnOnce() + Send>
        })
        .collect();
    exec.join_all(tasks);
    assert_eq!(acc.load(Ordering::SeqCst), 4);
}

#[test]
#[cfg(not(all(not(target_arch = "wasm32"), feature = "executor-native")))]
fn serial_join_all_executes_all() {
    // 测试清单 2：SerialExecutor::join_all 串行执行全部任务。
    let exec = SerialExecutor;
    let order = Arc::new(Mutex::new(Vec::new()));
    let tasks: Vec<Box<dyn FnOnce() + Send>> = {
        let order = order.clone();
        let order2 = order.clone();
        vec![
            Box::new(move || {
                order.lock().unwrap().push("task1");
            }),
            Box::new(move || {
                order2.lock().unwrap().push("task2");
            }),
        ]
    };
    exec.join_all(tasks);
    let order = order.lock().unwrap();
    assert_eq!(order.len(), 2);
    assert!(order.contains(&"task1"));
    assert!(order.contains(&"task2"));
}

#[test]
#[cfg(not(all(not(target_arch = "wasm32"), feature = "executor-native")))]
fn serial_join_all_empty() {
    let exec = SerialExecutor;
    exec.join_all(Vec::new());
}

#[test]
fn default_executor_join_all_runs() {
    // 测试清单 10/11：工厂构造的 Executor 可正常 join_all。
    let exec = default_executor();
    let counter = Arc::new(AtomicUsize::new(0));
    let tasks: Vec<Box<dyn FnOnce() + Send>> = (0..10)
        .map(|_| {
            let counter = counter.clone();
            Box::new(move || {
                counter.fetch_add(1, Ordering::SeqCst);
            }) as Box<dyn FnOnce() + Send>
        })
        .collect();
    exec.join_all(tasks);
    assert_eq!(counter.load(Ordering::SeqCst), 10);
}

#[test]
fn default_executor_returns_send_sync() {
    // Executor trait object 必须 Send + Sync（Db 持有 Arc<dyn Executor>）。
    let exec: Arc<dyn Executor> = default_executor();
    // 编译期验证 Send + Sync
    fn assert_send_sync<T: ?Sized + Send + Sync>() {}
    assert_send_sync::<dyn Executor>();
    // 使用 exec 避免未使用警告
    let _ = &exec;
}
