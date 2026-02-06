# M3 Task 4 Report —— 阶段一门禁自证归档

> 编排者归档节点（非 implementer 任务）。门禁在 Task 3 review 节点已全绿跑过，本 task 正式归档阶段一交付证据。
> 日期：2026-08-11
> 分支：feat/m3-web-npm

## 状态：✅ 全绿

阶段一（@vane-rs/web npm 包）交付完成，全量门禁不回退。

## 门禁清单（2026-08-11 最终归档）

| 门禁 | 命令 | 结果 |
|------|------|------|
| 格式 | `cargo fmt --all -- --check` | ✅ fmt OK |
| 静态检查 | `cargo clippy --all-targets --all-features -- -D warnings` | ✅ Finished，无 warning |
| wasm32 core | `cargo check --target wasm32-unknown-unknown -p vane-core` | ✅ Finished |
| wasm32 wasm | `cargo check --target wasm32-unknown-unknown -p vane-wasm` | ✅ Finished |
| 工作区测试 | `cargo test --workspace --all-features` | ✅ exit 0 |
| build-web 全流程 | `bash bindings/web/scripts/build-web.sh` | ✅ 13 文件产出 |
| wasm 体积 simd | gzip | ✅ 318424 bytes（≤819200，38.9%） |
| wasm 体积 scalar | gzip | ✅ 320589 bytes（≤819200，39.1%） |
| W8 校验 | vane_wasm.js 含 `__wbg_init` + `new URL(..., import.meta.url)` | ✅ |
| tsc 编译 | `tsc -p tsconfig.json` | ✅ 零错误 |
| ESM 导出冒烟 | `node --input-type=module -e "import('./dist/index.js')..."` | ✅ createVane=function / simd128Supported=function / SIMD128_TEST_MODULE.length=50 |
| probe 字节对齐 | SIMD128_TEST_MODULE 与 simd_probe.rs 逐字节一致 | ✅ 50 bytes，magic + FD 0C opcode |
| 冻结契约 | crates/vane-wasm/ 零改动 | ✅ git diff 确认 |

## dist 产出清单（13 文件）

```
bindings/web/dist/
├── index.js            7631    tsc 编译 src/index.ts → createVane 工厂
├── index.d.ts          1348    TS 类型
├── worker.js           4203    tsc 编译 src/worker.ts → Worker 入口
├── worker.d.ts           11    export {}（worker 无导出）
├── probe.js            2261    tsc 编译 src/probe.ts → 探针
├── probe.d.ts           954    TS 类型
├── types.js             421    export {}（类型仅 .d.ts）
├── types.d.ts          6990    TS 类型定义
├── vane_wasm.js       34079    wasm-bindgen --target web 生成 ESM 胶水
├── vane_wasm.d.ts      8347    wasm-bindgen 生成 TS 类型
├── vane_wasm_simd.wasm 803906  SIMD128 加速（318KB gzip）
├── vane_wasm_scalar.wasm 814624 scalar 兜底（320KB gzip）
└── vane_wasm_bg.wasm   814624  cp scalar 别名（默认 URL 兼容）
```

## 阶段一交付总结

@vane-rs/web npm 包实现完成（Task 2 骨架 + Task 3 JS/TS 源码）：
- wasm-bindgen --target web ESM 双变体（simd/scalar）+ 运行时探针（JS 侧 WebAssembly.validate）
- VaneWorker ESM export + worker 入口（new Worker(new URL(...), {type:'module'})）
- dict_loader 集成（dictData 内联优先 + CDN fallback + transferable 零拷贝）
- TS 类型（.d.ts，与 worker.rs 冻结契约字段名对齐）
- package.json（exports map vite/webpack 友好 + sideEffects + optionalDep @vane-rs/dict-zh）
- vite/webpack 零配置目标（new URL asset + import.meta.url）

冻结契约遵守：crates/vane-wasm/ 零改动（@vane-rs/web 是 vane-wasm 的 npm 包装，只新增 bindings/web/）。

## 阶段一未验证项（跨任务，阶段三/四覆盖）

- vite/webpack 实际打包行为 → Task 7/8 examples 实测
- npm publish 行为 → Task 11 release.yml
- @vane-rs/dict-zh 包 exports → Task 5
- 浏览器运行时（SIMD 探针/OPFS/IDB）→ Task 7/8 端到端

## Task 5 衔接

Task 5 发 @vane-rs/dict-zh 独立 npm 包（dict.bin + sha256_prefix.bin + package.json exports ./dict.bin）。@vane-rs/web 的 optionalDep 引用 `@vane-rs/dict-zh: 2026.8.0` 依赖此包就绪。
