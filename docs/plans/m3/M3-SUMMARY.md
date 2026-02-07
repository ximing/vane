# M3 总结报告 —— Web 端 npm 分发对齐 Node 端体验

> 里程碑：M3（post-M2 之后）
> 完成日期：2026-08-11
> 发版：v0.2.0
> 编排者：纯编排者模式（主 Agent 只做任务管理/调度/审查/门禁，全部代码通过 SubAgent 实现）

## 目标与达成

**核心痛点**：Web 端分发体验差——用户需 clone 仓库 `bash demo/build.sh` 自己 build wasm + 字典走 jsdelivr CDN。Node 端 `npm i @vane-rs/node` 零成本，Web 端却要 clone/build/CDN。

**M3 目标**：发 `@vane-rs/web` + `@vane-rs/dict-zh` npm 包，vite/webpack 直接 import，消除 clone/build/CDN，Web 端体验对齐 Node 端。

**达成**：v0.2.0 四端全部 published，Web 端 `npm i @vane-rs/web @vane-rs/dict-zh` 零配置 import + 检索。

## 交付物

### 1. @vane-rs/web npm 包（@vane-rs/web@0.2.0）
- wasm-bindgen `--target web` ESM 双变体（simd128/scalar）+ 运行时探针（JS 侧 `WebAssembly.validate(SIMD128_TEST_MODULE)`）
- VaneWorker ESM export + worker 入口（`new Worker(new URL('@vane-rs/web/worker', import.meta.url), {type:'module'})`）
- dict_loader 集成（dictData 内联优先 + CDN fallback + transferable 零拷贝）
- TS 类型（.d.ts，与 worker.rs 冻结契约字段名对齐 camelCase）
- package.json（exports map vite/webpack 友好 + sideEffects + optionalDep @vane-rs/dict-zh）
- wasm 体积：simd 318KB / scalar 320KB gzip（≤800KB 红线，余量 61%）
- 目录：`bindings/web/`（与 bindings/go/ 平级，纯 JS 包非 Rust crate）

### 2. @vane-rs/dict-zh npm 包（@vane-rs/dict-zh@2026.8.0）
- 纯数据包：dict.bin（1.41MB zstd）+ sha256_prefix.bin（8 字节）+ LICENSE
- exports `./dict.bin` + `./sha256_prefix.bin`（vite/webpack asset url）
- Web 端 `import dictBinUrl from '@vane-rs/dict-zh/dict.bin'` → fetch + arrayBuffer → VaneWorker dictData（零强制 CDN）
- 目录：`crates/vane-dict-zh/`（兼 Rust crate + npm 数据包源，package.json files 直接引用源 data/dict.bin 非拷贝）

### 3. 词典四渠道（SPEC §12.3 v1.5）
| 渠道 | 来源 | 优先级 |
|------|------|--------|
| Node | vane-dict-zh cargo path include_bytes 编译期内嵌 | 唯一 |
| Go | go:embed dict.bin.gz | 唯一 |
| WASM npm dictData [M3 新] | @vane-rs/dict-zh npm 包 data/dict.bin → import → fetch → dictData | 优先 |
| WASM CDN [M2] | jsdelivr npm fallback | 降级 |
- 四渠道哈希一致校验（check-dict-hash.sh + dict_test.rs 扩展，npm pack 产物字节比对）

### 4. release.yml 四端发布（三端→四端）
- 新增 build-web job（cargo build 双变体 + wasm-bindgen + wasm-opt + tsc + 体积门禁）
- release job 扩展：Stage web dist/ + npm publish @vane-rs/dict-zh（先）+ @vane-rs/web（后，C4 顺序修复）
- publish 顺序：dict-zh 先 web 后（optionalDep 依赖先发，失败阻塞 web 避免断裂 optionalDep 上线）

### 5. vite + webpack 示例（examples/）
- examples/vite/：零配置（仅 `assetsInclude: ['**/*.bin']`，无 wasm/worker 插件），vite build 成功
- examples/webpack/：`experiments.outputModule` + `asset/resource` + `scriptLoading:'module'`（无 asyncWebAssembly，init(wasmUrl) 绕过），webpack build 成功
- file: 本地路径引用（@vane-rs/web + @vane-rs/dict-zh 未发 npm 前的测试方案）

### 6. 文档站 Web Integration 指南页
- `website/src/pages/guides/WebIntegration.tsx`（第 5 个 guide）
- 7 节：安装 / vite 配置 / webpack 配置 / 用法 / dictData / worker / 常见坑
- 路由 `/vane/guides/web-integration`（routes.ts + nav.ts 注册）

### 7. SPEC v1.4→v1.5（用户批准）
- §12.1 Workspace 补全（crates/vane-dict-zh M1 起漏列 + bindings/web M3 新增）
- §12.2 三端→四端（加 Web npm @vane-rs/web 行）+ Node prebuilt 追加标注"未实现，顺延"
- §12.3 三渠道→四渠道（scope @vane-rs 修正 + 删 @vane/slim + WASM npm dictData 第四渠道）
- §12.4 三端→四端 + @vane-rs/dict-zh 日历版例外
- §13.2 加第 5 项 Web npm 安装门禁 + scope 修正 + @vane-rs/web 双变体体积门禁
- changelog v1.5 条目

## 冻结契约全程遵守

- **crates/vane-wasm/ 零 .rs 改动**：VaneWorker JS 契约冻结，@vane-rs/web 是其 npm 包装（只新增 bindings/web/，不改 worker.rs/lib.rs/dict_loader.rs/simd_probe.rs）
- **crates/vane-dict-zh/ src/ + data/ + Cargo.toml 零改动**：只加 package.json/.npmignore/README/LICENSE
- core 禁 std::fs/std::net/mmap + 依赖黑名单 + MoSCoW Won't-have（不内置 embedding/GPU/SQL/分布式）全程不触碰

## 编排流程

纯编排者模式：14 任务 TaskCreate 看板 + 串行+审查/实现重叠流水线。每任务：implementer (SubAgent) → 编排者跑全量门禁 → task reviewer (SubAgent) → fix loop（如需）→ re-review → complete。失败重试有上限（同一 SubAgent 失败 2 次换策略）。

- 22 commits（feat/m3-web-npm 分支，merge 38573e3 到 main）
- 5 个 fix loop（Task 2 C1 license / Task 2 I1 README / Task 5 I1 LICENSE / Task 11 C4 publish 顺序 / Task 13 C1 typo）
- 2 个 AskUserQuestion 检查点（Task 1 设计 3 决策 + Task 13 SPEC v1.5 批准 + Task 14 打 tag 确认）
- 全程中文沟通与文档

## 验证

### v0.2.0 发版验证（四端全绿）
- release.yml tag 触发 run 31467745900 conclusion: success
- npm 四端：@vane-rs/node@0.2.0 + @vane-rs/web@0.2.0 + @vane-rs/dict-zh@2026.8.0 全 published
- GitHub Release v0.2.0：10 assets（4 .a + 4 .node + 2 .wasm）
- ci.yml 19 jobs 全绿

### 全量门禁
- cargo fmt --all -- --check ✅
- cargo clippy --all-targets --all-features -- -D warnings ✅
- cargo test --workspace --all-features ✅
- cargo check --target wasm32-unknown-unknown -p vane-core + -p vane-wasm ✅
- check-dict-hash.sh 四渠道 ✅
- build-web.sh（wasm 体积双变体 ≤800KB gzip）✅
- vane-wasm .rs 零改动 ✅

## DoD 达成

- ✅ v0.1.2 发版闭环（阶段零前置）
- ✅ @vane-rs/web npm published
- ✅ @vane-rs/dict-zh npm published
- ✅ vite + webpack 示例可用
- ✅ 文档站集成指南页
- ✅ release.yml 四端发布，v0.2.0 发版
- ✅ SPEC v1.5（用户批准）+ changelog
- ✅ cargo test --workspace 全绿 + clippy/fmt/wasm32/check-dict-hash/deny 不回退
- ✅ docs/plans/m3/ 计划 + 总结报告
- ⏸ install-matrix 扩展 @vane-rs/web 冒烟：Task 10 deferred（Could 非 Must），发版后 @vane-rs/web + @vane-rs/dict-zh 已发 npm，可用 install-matrix 扩展真正 `npm i` 测试

## 后续可选

- install-matrix 扩展 @vane-rs/web vite/webpack build 冒烟（Task 10 deferred）
- Node prebuilt 追加 musl/arm64-win（SPEC 标注"未实现，顺延"，非 M3 范围）
- simd vs scalar 性能基准
- HNSW 路径向量化
