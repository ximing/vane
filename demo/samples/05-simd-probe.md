# SIMD128 探针

WebAssembly SIMD128 提供 128 位 SIMD 指令（v128, f32x4, i32x4），加速向量计算和位图操作。

## 运行时探测

并非所有浏览器支持 SIMD128。通过 WebAssembly.validate(含 v128.const 指令的模块) 探测运行时支持。支持则加载 simd 产物，否则加载 scalar 产物。

## 双产物

构建时用 -Ctarget-feature=+simd128 生成 simd 产物，默认构建生成 scalar 产物。两产物 API 一致，仅 target-feature 不同。
