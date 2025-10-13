// vane-node: Node.js napi-rs 绑定（薄壳，SPEC §9.3）。
// Task 1：仅 hello() 验证 napi 构建链路通。Task 2 起逐个加 mod（S13 裁决）。
#![deny(warnings)]

use napi_derive::napi;

/// 构建链路自检导出。Task 2 移除。
#[napi]
pub fn hello() -> String {
    "vane-node".to_string()
}
