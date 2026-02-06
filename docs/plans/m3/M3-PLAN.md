# Vane M3 —— Web 端 npm 分发对齐 Node 端体验

> 编排者：主 Agent 只做任务管理与调度，禁止写代码/编辑文件（唯一例外：本目录计划状态文件与看板）。
> 全部工作通过 TaskCreate 建看板 / Agent+SendMessage 派发 SubAgent / 审查产出 / 修复节点跑全量门禁。
> 串行+审查/实现重叠（同时只一个 implementer 写）。全程中文。每步确认可 commit。

## 背景与核心痛点

Vane（github.com/ximing/vane，public）M0+M1+M2+post-M2+post-v0.1.1 全完成。三端：Node（@vane-rs/node napi-rs native）/ Go（cgo .a）/ 浏览器 WASM（GitHub Release 双变体 .wasm）。f32 SIMD 向量化已落地（SPEC v1.4），文档站已上线（website/，ximing.github.io/vane）。

**核心痛点**：Web 端分发体验差——用户需 clone 仓库 `bash demo/build.sh` 自己 build wasm + 字典走 jsdelivr CDN。Node 端 `npm i @vane-rs/node` 零成本，Web 端却要 clone/build/CDN。

**M3 目标**：发 `@vane-rs/web` + `@vane-rs/dict-zh` npm 包，vite/webpack 直接 import，消除 clone/build/CDN，Web 端体验对齐 Node 端。

## 阶段零：v0.1.2 发版闭环（前置）

**状态：✅ 已闭环（2026-08-11 核查）**
- npm `@vane-rs/node@0.1.2` published（latest）；gh release v0.1.2 published，10 assets（4 .a + 4 .node + 2 .wasm）。
- 条件「若 npm 无 0.1.2」不成立 → 跳过闭环工作。

## 阶段一：@vane-rs/web npm 包设计与实现

1. **只读 SubAgent 出设计**：@vane-rs/web 包结构——vane-wasm 用 wasm-bindgen `--target web` 产出 ESM（.wasm + glue.js + .d.ts），含 simd/scalar 双变体 + 运行时探针 + worker 壳 + dict_loader。package.json（name @vane-rs/web, main/module/exports/types, files）。双变体 .wasm 作为包资源。**用户确认设计（AskUserQuestion 检查点）**。
2. **build-web 产物**：扩展 build-wasm-variants.sh 或新增脚本，wasm-bindgen --target web 产出 ESM 双变体（simd/scalar），wasm-opt -Oz，体积 ≤800KB gzip 红线（词典永不进 wasm）。
3. **@vane-rs/web 包实现**：JS glue（VaneWorker ESM export + worker 入口 + dict_loader 集成）+ TS 类型（.d.ts）+ package.json（exports map，vite/webpack 友好）。双变体探针自动选 .wasm。
4. **词典内联入口**：VaneWorker 支持 dictData 内联（M2-04 已有）——@vane-rs/web 暴露 dictData 接口，用户传 @vane-rs/dict-zh 的 dict.bin 字节。CDN 作 fallback（dictUrl 默认 jsdelivr，dictData 优先）。
5. **TDD + 自证**：wasm32 check / cargo test vane-wasm / clippy / wasm 体积双变体 ≤800KB gzip / ESM 导出可 import（node smoke 或 vite build 冒烟）。

## 阶段二：@vane-rs/dict-zh npm 包

1. **包结构**：crates/vane-dict-zh/data/dict.bin 发 npm 包 @vane-rs/dict-zh。package.json（name @vane-rs/dict-zh, files: [dict.bin, sha256_prefix.bin], version 与 dict 版本对齐 2026.8.0 或 semver）。dict.bin 1.48MB（npm 包无 800KB 红线，仅 wasm 产物有）。
2. **Web 端 import**：`import dictBinUrl from '@vane-rs/dict-zh/dict.bin'`（vite/webpack 作 asset url）或 fetch + arrayBuffer。传 VaneWorker dictData。零 CDN。
3. **三渠道→四渠道**：Node（include_bytes）/ Go（embed）/ WASM CDN（fallback）/ **WASM npm dictData（新，优先）**。三渠道哈希一致校验扩展（check-dict-hash.sh + dict_tests.rs）。
4. **SPEC 修订（合并至阶段四）**：§12.3 词典三渠道→四渠道。→ 见阶段四编排决策：v1.5 一次性修订。

## 阶段三：vite/webpack 集成 + 示例

1. **vite 示例**：examples/vite/（最小 vite 项目，npm i @vane-rs/web @vane-rs/dict-zh，import VaneWorker + dictData，wasm asset + worker 配置）。
2. **webpack 示例**：examples/webpack/（webpack 5 wasm async module + worker）。
3. **文档站集成指南**：website/ 补 vite/webpack 集成页（安装 + import + dictData + worker 配置 + 常见坑）。
4. **CI 集成冒烟（可选）**：install-matrix 或新 job，vite build + webpack build @vane-rs/web 冒烟。

## 阶段四：release.yml 扩展 + 发版 v0.2.0

1. **build-web job**：release.yml 加 build-web（wasm-bindgen --target web 双变体 + wasm-opt + upload .wasm + ESM glue）。
2. **publish 扩展**：release job 加 npm publish @vane-rs/web + @vane-rs/dict-zh（if tag，NPM_TOKEN，publishConfig.access=public）。三端→四端。
3. **bump version 0.1.2→0.2.0**（minor）三端同步 + @vane-rs/web + @vane-rs/dict-zh version。
4. **SPEC 修订（v1.4→v1.5，一次性，用户批准）**：§12.2 三端→四端 prebuilt（加 Web npm @vane-rs/web）+ §12.3 四渠道 + §13.2 DoD 加 Web npm 安装门禁。
5. **发版 v0.2.0**：workflow_dispatch 验证 release.yml 四端 → 打 tag v0.2.0（**用户确认**）→ npm publish @vane-rs/node + @vane-rs/web + @vane-rs/dict-zh + GitHub Release → install-matrix 扩展验证。

## 完成定义（DoD）

- v0.1.2 发版闭环（✅ 阶段零）；
- @vane-rs/web npm 包 published（wasm-bindgen --target web ESM 双变体 + worker + dict_loader + TS 类型，vite/webpack 可 import，wasm ≤800KB gzip）；
- @vane-rs/dict-zh npm 包 published（dict.bin，Web 端 dictData 内联，零强制 CDN）；
- vite + webpack 示例可用（examples/，npm i → import → 检索，零 clone/build/CDN）；
- 文档站集成指南（vite/webpack 页）；
- release.yml 四端发布（Node + Go .a + WASM Release + Web npm + dict-zh npm），v0.2.0 发版；
- SPEC v1.5（§12.2 四端 + §12.3 四渠道 + §13.2 Web npm 门禁，用户批准）+ changelog；
- cargo test --workspace 全绿 + clippy/fmt/wasm32 check/check-no-std-fs/deny 不回退；
- install-matrix 扩展（@vane-rs/web vite/webpack build 冒烟）；
- docs/plans/m3/ 计划 + 总结报告。

## 约束（MoSCoW 即合同）

- SubAgent 超范围需求一律拒绝并记录；Won't-have（内置 embedding/GPU/SQL/分布式）不得触碰。
- 词典永不进 wasm 产物（800KB gzip 红线，含 jieba 算法代码不含词典数据）。@vane-rs/dict-zh 是独立 npm 包，dict.bin 不进 wasm。
- core 禁 std::fs/std::net/mmap；cfg 仅 VFS/Executor + cfg(target_feature) 算法向量化（SPEC v1.4 I-5）。
- 依赖黑名单（regex/tokio/prost/tonic/openssl/lindera/ndarray/wee_alloc/dashmap/parking_lot）；rayon 仅 Executor impl。
- 不改 M0/M1/M2/post-M2/post-v0.1.1 冻结 pub API（vane-wasm VaneWorker JS 契约不变，@vane-rs/web 是其 npm 包装）；SPEC 矛盾上报不绕行。
- SPEC 修订需用户批准（AskUserQuestion 检查点）。
- npm scope @vane-rs（用户已拥有）+ NPM_TOKEN 已配置。
- 发版（打 tag v0.2.0）需用户确认。

## 编排决策记录

- **SPEC v1.5 合并修订**：原计划阶段 2.4 与阶段 4.4 均提 v1.5。合并为阶段四一次性提议（§12.2+§12.3+§13.2），阶段二仅在 ledger 记 §12.3 待修订，不单独打扰用户。
- **分支**：首个 implementer 写代码前创建 `feat/m3-web-npm`（post-M2 模式，main 上不开实施提交）。
- **审查/实现重叠**：implementer(写) || reviewer(只读不运行 cargo) 可重叠；同时只一个 implementer 写。
