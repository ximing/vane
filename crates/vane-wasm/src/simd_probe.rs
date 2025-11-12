//! SIMD 探针占位（SPEC §12.1，M2-05 落实）。
//!
//! WASM SIMD128 可显著加速向量距离计算（cosine/L2/dot）。是否启用取决于
//! 运行时 `WebAssembly.validate` 探测——M2-05 会在 Worker init 阶段注入探针结果。
//!
//! 本模块为占位：恒返回 `false`，确保 M2-01 体积门禁测量的是非 SIMD 路径
//! （保守上界）。M2-04/M2-05 消费此函数决定走 brute_search 还是 SIMD 路径。

/// 探测运行时是否支持 WebAssembly SIMD128。
///
/// **占位实现**（M2-01）：恒返回 `false`。
/// M2-05 落实真实探针——通过 `WebAssembly.validate(simd_module_bytes)` 判定，
/// 或经 Worker init postMessage 注入 `self.WebAssembly.validate(...)` 结果。
pub fn simd128_supported() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_returns_false() {
        // M2-01 占位：恒 false。M2-05 落实真实探针后此测试需更新。
        assert!(!simd128_supported());
    }
}
