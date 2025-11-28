//! SIMD128 探针（SPEC §12.1/§3.6，M2-05 落实）。
//!
//! 通过 `WebAssembly.validate(simd128_test_module)` 探测运行时是否支持
//! WebAssembly SIMD128。Worker init 调用此函数决定加载 simd 还是 scalar 产物
//! （M2-04 协同）。两产物由构建脚本区分（M2-05），core 算法无平台分支
//! （`cfg(target_arch)`/`cfg(target_os)` 仅限 VFS/Executor，SPEC v1.4 I-5），依赖
//! `-Ctarget-feature=+simd128` 启用 LLVM SIMD 代码生成（roaring 位图等依赖
//! 的 SIMD 路径被激活；f32 距离三核 cosine/L2/dot 已由 post-v0.1.1 Task 1
//! 显式向量化——simd128/标量双路径 `cfg(target_feature="simd128")`，归约顺序
//! 逐位对齐保证双变体 top-10 一致，见 `vane-core/src/vector/mod.rs`）。
//!
//! 非 wasm32 环境（host 测试）无 `WebAssembly` 对象，恒返 false。
//! wasm32 环境调用 JS `WebAssembly.validate`；测试模块含 `v128.const` 指令，
//! 不支持 simd128 的运行时 validate 失败返 false（或抛 CompileError→false）。

use wasm_bindgen::JsValue;

/// 最小 simd128 测试模块（wat2wasm 生成，M2-05 固定字节）。
///
/// 等价 WAT：
/// ```wat
/// (module
///   (func (export "t")
///     v128.const i32x4 0 0 0 0
///     drop
///   )
/// )
/// ```
///
/// 含 `v128.const` 指令（opcode `FD 0C` + 16 字节立即数），仅 simd128 运行时
/// 可 validate 通过。模块无 import、无自定义 section、无内存——最小探测开销。
pub const SIMD128_TEST_MODULE: &[u8] = &[
    // [magic + version] 8 bytes
    0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00,
    // [type section (id=1)] 1 type: () -> ()
    0x01, 0x04, 0x01, 0x60, 0x00, 0x00,
    // [function section (id=3)] 1 function, type idx 0
    0x03, 0x02, 0x01, 0x00, // [export section (id=7)] "t" -> function 0
    0x07, 0x05, 0x01, 0x01, 0x74, 0x00, 0x00,
    // [code section (id=10)] 1 body, body_size=0x15, 0 locals
    0x0a, 0x17, 0x01, 0x15, 0x00,
    // v128.const (opcode FD 0C) + 16-byte immediate (all zeros)
    0xfd, 0x0c, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, // drop (0x1A) + end (0x0B)
    0x1a, 0x0b,
];

/// JS `WebAssembly.validate(bufferSource)` 绑定（无需 js-sys dep）。
///
/// wasm_bindgen 将 `&[u8]` 自动转为 `Uint8Array` 传入。返回模块是否合法
/// （含 simd128 支持判定——不支持 simd128 的运行时对含 v128 指令的模块
/// 返 false 或抛 CompileError，catch 后置 false）。
#[wasm_bindgen::prelude::wasm_bindgen]
extern "C" {
    #[wasm_bindgen::prelude::wasm_bindgen(js_namespace = WebAssembly, catch)]
    fn webassembly_validate(buffer: &[u8]) -> Result<bool, JsValue>;
}

/// 探测运行时是否支持 WebAssembly SIMD128。
///
/// 实现：`WebAssembly.validate(SIMD128_TEST_MODULE)`。该模块含 `v128.const`
/// 指令，仅 simd128 运行时 validate 通过返 true；不支持则返 false（或抛错→false）。
///
/// 非 wasm32 环境（host）无 `WebAssembly` 对象，恒返 false。
///
/// # 线程安全
/// 无状态、纯探测。可安全并发调用（wasm32 单线程，实际由 Worker init 调一次）。
pub fn simd128_supported() -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        probe_simd128()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        // host 测试环境无 WebAssembly 对象——探针 false（保守，走 scalar）。
        false
    }
}

#[cfg(target_arch = "wasm32")]
fn probe_simd128() -> bool {
    match webassembly_validate(SIMD128_TEST_MODULE) {
        Ok(true) => true,
        // validate 返 false（不支持 simd128）或抛 CompileError → 视为不支持。
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_returns_false() {
        // 非 wasm32 host 无 WebAssembly 对象——探针恒 false（保守走 scalar）。
        // wasm32 路径由浏览器/wasm-bindgen-test 覆盖（M2-04/M2-06）。
        assert!(!simd128_supported());
    }

    #[test]
    fn test_module_has_wasm_magic() {
        // 魔数校验——证明字节序列是合法 wasm 头。
        assert_eq!(&SIMD128_TEST_MODULE[0..4], b"\x00asm");
        assert_eq!(&SIMD128_TEST_MODULE[4..8], &[0x01, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn test_module_contains_v128_const_opcode() {
        // v128.const opcode = FD 0C——simd128 探测的关键指令。
        // 不含此指令的模块在非 simd128 运行时也会 validate 通过，探测无意义。
        let needle = [0xfd, 0x0c];
        let found = SIMD128_TEST_MODULE.windows(2).any(|w| w == needle);
        assert!(found, "SIMD128_TEST_MODULE must contain v128.const (FD 0C)");
    }

    #[test]
    fn test_module_is_well_formed_section_structure() {
        // 段结构健全性：type(1) + function(3) + export(7) + code(10) 四段齐备。
        let bytes = SIMD128_TEST_MODULE;
        // 跳过 8 字节 magic+version。
        let mut idx = 8;
        let mut section_ids = Vec::new();
        while idx < bytes.len() {
            let id = bytes[idx];
            let size = bytes[idx + 1] as usize;
            section_ids.push(id);
            idx += 2 + size;
        }
        assert!(section_ids.contains(&1), "missing type section");
        assert!(section_ids.contains(&3), "missing function section");
        assert!(section_ids.contains(&7), "missing export section");
        assert!(section_ids.contains(&10), "missing code section");
        assert_eq!(idx, bytes.len(), "section sizes must sum to module length");
    }
}
