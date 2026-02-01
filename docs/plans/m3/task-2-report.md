# M3 Task 2 Report —— @vane-rs/web 包骨架 + build-web.sh

> implementer：Task 2 implementer（Claude）
> 日期：2026-08-11
> 分支：feat/m3-web-npm
> 任务：创建 bindings/web/ 包目录骨架 + scripts/build-web.sh 构建脚本，产出 wasm-bindgen --target web 的 ESM 双变体产物。

## 状态

**DONE_WITH_CONCERNS**

核心交付完成：bindings/web/ 骨架 + build-web.sh 产出双变体 wasm + glue + .d.ts，体积门禁通过，冻结契约零改动。3 处 concern 需编排者确认（见下）。

## Commits

```
520a00e feat(web): @vane-rs/web 包骨架 + build-web.sh wasm 双变体构建脚本——M3 Task 2
```

（单 commit，仅含 bindings/web/ 5 个源文件；dist/ 被 .gitignore 忽略不入库）

## 测试摘要

`bash bindings/web/scripts/build-web.sh` 成功产出 dist/ 全部 5 文件；双变体 gzip 体积门禁通过（simd 318424 / scalar 320589 ≤ 819200）；W8 校验通过（vane_wasm.js 含 `__wbg_init` + `new URL('vane_wasm_bg.wasm', import.meta.url)`）；simd 变体 3192 个 simd 指令 / scalar 0；crates/vane-wasm/ 零改动（冻结契约遵守）。

## Concerns

### C1：LICENSE 矛盾（需编排者定夺）

- task-1-design.md §1 package.json 草案 + task brief 都明确写 `"license": "MIT"`。
- 但仓库根 LICENSE 是 Apache-2.0（完整 Apache 协议文本），Cargo.toml `license = "Apache-2.0"`，@vane-rs/node package.json 无 license 字段（继承仓库 Apache-2.0）。
- 我按设计文档 MIT 实现（bindings/web/LICENSE = MIT 文本，package.json license = "MIT"）。
- **建议**：若 web 包应与仓库整体 Apache-2.0 一致，改 LICENSE 文件 + package.json license 字段（一处文件 + 一个字段）；若有意 MIT（纯 JS 包更宽松鼓励采纳），确认即可。
- 编排者决策列表（task-1-design.md 开头 3 处）未提及 license 变更，可能是设计文档未捕获的矛盾。

### C2：scalar 变体也跑 wasm-bindgen（偏离 task brief 字面，技术必需）

- task brief 第 2 步："scalar 变体不需要再跑 wasm-bindgen"。
- 但 raw .wasm（cargo build 产出）的 `__wbindgen_*` 导入需经 wasm-bindgen 重写为 `__wbg_*`，才能匹配 vane_wasm.js glue 的 import object（键名 `__wbg_*`）。未经后处理的 raw .wasm 与 glue 不匹配，`WebAssembly.instantiate` 报 TypeError。
- **实现**：双变体都跑 wasm-bindgen（与 demo/build.sh 同模式，已验证可用），glue 只拷一份（simd 与 scalar 的 glue 相同，导出一致）。
- 影响极小：多一次 wasm-bindgen 调用（~2 秒），无产物差异（glue 相同）。
- build-web.sh 脚本头部注释已记录此偏离 + 技术原因。

### C3：双变体 gzip 体积略高于 task brief 预期

- task brief："实测 simd ~312KB / scalar ~315KB"。
- 本次产出：simd 318424 bytes（~311KB）/ scalar 320589 bytes（~313KB）。gzip 口径一致，差 ~6KB。
- 可能原因：代码变化（M2 后续提交）或 wasm-bindgen/wasm-opt 版本差异。
- 仍远 <800KB 门禁（占 38.9% / 39.1%），无风险。

### C4（minor）：target_features 段未声明 simd128

- wasm-objdump -j target_features 对 simd 变体只显示 `[+] mutable-globals`，无 simd128。
- 但 simd 变体含 3192 个 simd 指令（f32x4/i32x4/v128），运行时仍需 simd128 支持。
- wasm-bindgen 后处理可能 strip 了 target_features 段（demo/pkg 的 simd 变体同现象）。
- 不影响功能：worker.js 用 JS 探针（`WebAssembly.validate(SIMD128_TEST_MODULE)`）选择加载 simd 还是 scalar，不依赖 target_features 段。

## 产出文件清单

### bindings/web/ 树（入库 5 文件）

```
bindings/web/
├── .gitignore                  # 忽略 dist/ + .build-tmp/（build 产物不入库）
├── package.json                # §1 草案逐字段（@vane-rs/web@0.2.0, type=module, exports, sideEffects, optionalDep）
├── README.md                   # 安装 + vite/webpack 集成 + API 占位（Task 3 补全）
├── LICENSE                     # MIT（见 C1 concern）
└── scripts/
    └── build-web.sh            # 构建脚本（可执行，7 步流程）
```

### bindings/web/dist/（build 产物，不入库，npm 发布内容）

```
bindings/web/dist/
├── vane_wasm.js                # 34079 bytes，wasm-bindgen 生成 ESM 胶水
├── vane_wasm.d.ts              # 8347 bytes，wasm-bindgen 生成 TS 类型（VaneWorker）
├── vane_wasm_simd.wasm         # 803906 bytes raw / 318424 bytes gzip，SIMD128 加速
├── vane_wasm_scalar.wasm       # 814624 bytes raw / 320589 bytes gzip，scalar 兜底
└── vane_wasm_bg.wasm           # 814624 bytes（cp scalar 别名，§7.3 默认 URL 兼容）
```

### 体积数据

| 变体 | raw | gzip | 门禁 (≤819200) | simd 指令数 |
|------|-----|------|----------------|-------------|
| simd | 803906 | 318424 | ✅ (38.9%) | 3192 |
| scalar | 814624 | 320589 | ✅ (39.1%) | 0 |
| bg (alias) | 814624 | 320585 | 不入门禁 | 0 |

### 工具链版本

- wasm-bindgen 0.2.127
- wasm-opt version 131
- wasm-objdump 1.0.41

## 验证清单

- [x] `bash bindings/web/scripts/build-web.sh` 成功产出 dist/ 全部 5 文件
- [x] 体积门禁：双变体 gzip ≤800KB（simd 318424 / scalar 320589）
- [x] W8 校验：vane_wasm.js 含 `__wbg_init`（第 924 行）+ `new URL('vane_wasm_bg.wasm', import.meta.url)`（第 937 行）
- [x] W1 缓解：cp vane_wasm_scalar.wasm vane_wasm_bg.wasm 别名存在，默认 URL 可解析
- [x] simd 变体含 simd128 指令（3192 个 f32x4/i32x4/v128），scalar 0 个
- [x] git diff 确认未改 crates/vane-wasm/ 任何 .rs（冻结契约遵守）
- [x] dist/ 不入库（.gitignore），与 demo/pkg/ / node *.node 模式一致

## Task 3 衔接

Task 3 将在此骨架上加：
- `src/index.ts` / `worker.ts` / `probe.ts` / `types.ts`（手写 TS 源）
- `dist/index.js` / `worker.js` / `probe.js` + 对应 .d.ts（tsc 编译产出）
- build-web.sh 扩展 tsc 编译步骤（第 6 步，当前脚本注释 "Task 3 扩展"）
- probe.js 的 SIMD128_TEST_MODULE 字节常量需与 crates/vane-wasm/src/simd_probe.rs 逐字节对齐（§3 维护红线）

package.json 的 exports map / sideEffects / main / module / types 字段已预置指向 dist/index.js 等 Task 3 产出文件，Task 3 无需改 package.json。
