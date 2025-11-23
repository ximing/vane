# `vane-wasm` 指令

## 职责

- 此 crate 是浏览器交付物：将 `vane-core` API 适配为 wasm-bindgen 导出，并提供 OPFS、IndexedDB、Worker、SIMD 探针和词典加载胶水。
- 检索、排序和存储业务规则应留在 `vane-core`；这里只保留边界转换、浏览器能力适配与运行时编排。

## 约束

- 目标固定为 `wasm32-unknown-unknown`。不得引入 `std::fs`、`std::net`、mmap、原生线程假设或 WASI 依赖。
- 异步仅存在于页面与 Worker、浏览器 API 的边界；核心访问维持同步 VFS 语义。
- 词典数据不得打包进 WASM；保持 `dict-zh` feature 禁用，并沿用现有的下载、校验、缓存与降级路径。
- 控制 `web-sys` feature 和新增依赖，避免破坏 WASM 体积门禁；不要把平台分支扩散进 `vane-core`。

## 验证

- 运行 `cargo check --target wasm32-unknown-unknown -p vane-wasm` 与对应的 wasm Clippy 检查。
- 修改 Worker、双变体、VFS 或词典加载时，运行相关 WASM 测试/脚本，并确认 scalar 与 SIMD 行为一致。
