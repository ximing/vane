// SIMD128 探针（src/probe.ts）。
//
// §3 双变体探针策略：Worker init 之前用 WebAssembly.validate 探测运行时是否支持
// SIMD128，据结果选择加载 vane_wasm_simd.wasm 或 vane_wasm_scalar.wasm。
//
// ⚠️ §3.5 维护红线：SIMD128_TEST_MODULE 必须与 crates/vane-wasm/src/simd_probe.rs
// 的 SIMD128_TEST_MODULE 常量逐字节一致。单测校验 magic + FD 0C opcode + 段结构。
// 若 simd_probe.rs 常量变更，本文件必须同步。

/**
 * 最小 SIMD128 测试模块（wat2wasm 生成，固定字节）。
 *
 * 等价 WAT：
 * ```wat
 * (module
 *   (func (export "t")
 *     v128.const i32x4 0 0 0 0
 *     drop
 *   )
 * )
 * ```
 *
 * 含 `v128.const` 指令（opcode `FD 0C` + 16 字节立即数），仅 simd128 运行时
 * 可 validate 通过。模块无 import、无自定义 section、无内存——最小探测开销。
 *
 * 逐字节复制自 crates/vane-wasm/src/simd_probe.rs SIMD128_TEST_MODULE（50 bytes）。
 */
export const SIMD128_TEST_MODULE = new Uint8Array([
  // [magic + version] 8 bytes
  0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00,
  // [type section (id=1)] 1 type: () -> ()
  0x01, 0x04, 0x01, 0x60, 0x00, 0x00,
  // [function section (id=3)] 1 function, type idx 0
  0x03, 0x02, 0x01, 0x00,
  // [export section (id=7)] "t" -> function 0
  0x07, 0x05, 0x01, 0x01, 0x74, 0x00, 0x00,
  // [code section (id=10)] 1 body, body_size=0x15, 0 locals
  0x0a, 0x17, 0x01, 0x15, 0x00,
  // v128.const (opcode FD 0C) + 16-byte immediate (all zeros)
  0xfd, 0x0c, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
  0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
  0x00, 0x00,
  // drop (0x1A) + end (0x0B)
  0x1a, 0x0b,
]);

/**
 * 探测运行时是否支持 WebAssembly SIMD128。
 *
 * 实现：`WebAssembly.validate(SIMD128_TEST_MODULE)`。该模块含 `v128.const`
 * 指令，仅 simd128 运行时 validate 通过返 true；不支持则返 false（或抛错→false）。
 *
 * Worker init 调用此函数决定加载 simd 还是 scalar 产物。
 */
export function simd128Supported(): boolean {
  try {
    return WebAssembly.validate(SIMD128_TEST_MODULE);
  } catch {
    return false; // 不支持或 CompileError → 保守走 scalar
  }
}
