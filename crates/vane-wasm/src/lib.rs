//! vane-wasm：浏览器 wasm deliverable 胶水 crate（SPEC §12.1）。
//!
//! Phase Zero 骨架：仅占位导出，不引任何浏览器 API（web-sys/js-sys/OPFS/IDB 均不引）。
//! 真实 Worker 胶水在后续模块（M2-01+）交付。
//!
//! 依赖 vane-core default features（不含 jieba/dict-zh——词典永不进 wasm，红线）。

use wasm_bindgen::prelude::*;

/// 返回 vane 包版本（CARGO_PKG_VERSION）。
///
/// Phase Zero 占位导出，证明 vane-core 可编 wasm deliverable；
/// 后续模块将在此 crate 增加真实检索/管理 API 胶水。
#[wasm_bindgen]
pub fn vane_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}
