# M3 PROGRESS —— 编排 ledger（compaction 恢复地图）

> plan: docs/plans/m3/M3-PLAN.md
> 角色：纯编排者（不写代码，只调度/审查/跑门禁）
> 失败重试上限：同一 SubAgent 任务失败 2 次换策略或上报。
> 任务看板同步在 TaskCreate/TaskList；本文件是跨 compaction 的权威状态。

## 阶段零：v0.1.2 发版闭环
- 状态：✅ 已闭环（2026-08-11 核查，无需工作）
- 证据：npm `@vane-rs/node@0.1.2` published（versions 0.1.0/0.1.1/0.1.2，latest=0.1.2）；gh release v0.1.2 published 2026-08-11T01:33:53Z，10 assets（4 .a + 4 .node + 2 .wasm：vane_wasm_scalar.wasm / vane_wasm_simd.wasm）。
- install-matrix：✅ 3 个最新 run 全绿（v0.1.2 release 触发 workflow_run，2026-08-11T01:34:15Z success）
- ci.yml：✅ 绿（2026-08-11T01:30:56Z，7m40s）
- release.yml：✅ v0.1.2 run 绿（3m15s）
- 结论：阶段零五项（npm + gh release + install-matrix + ci + release.yml）全绿，闭环确认。

## 阶段一：@vane-rs/web npm 包
- Task 1.1 只读设计 → 用户确认：✅ 设计产出完成（docs/plans/m3/task-1-design.md），编排者审查通过（无 SPEC 矛盾、无冻结契约冲突）。3 处约束冲突已决策：①包目录 bindings/web/ ②dict-zh 钉死 2026.8.0 ③不主线程预校验。跨任务依赖记 Task 5（@vane-rs/dict-zh exports ./dict.bin）。deferred 记 Task 3（TS 类型对齐 worker.rs 字段名）。**等用户确认设计后开工 Task 2。**
- Task 1.2 build-web 产物脚本：pending
- Task 1.3 @vane-rs/web 包实现：pending
- Task 1.4 dictData 内联入口暴露：pending（并入 1.3）
- Task 1.5 TDD + 自证门禁：pending

## 阶段二：@vane-rs/dict-zh npm 包
- Task 2.1 包结构 + package.json：pending
- Task 2.2 Web 端 import 路径：pending（并入 2.1）
- Task 2.3 四渠道哈希校验扩展：pending
- Task 2.4 SPEC §12.3：合并至阶段四 v1.5

## 阶段三：vite/webpack 集成 + 示例
- Task 3.1 examples/vite/：pending
- Task 3.2 examples/webpack/：pending
- Task 3.3 文档站集成指南页：pending
- Task 3.4 CI 集成冒烟（可选）：pending

## 阶段四：release.yml 扩展 + v0.2.0
- Task 4.1 release.yml build-web job：pending
- Task 4.2 publish 扩展（@vane-rs/web + dict-zh）：pending
- Task 4.3 bump 0.1.2→0.2.0：pending
- Task 4.4 SPEC v1.4→v1.5 一次性修订（用户批准）：pending
- Task 4.5 发版 v0.2.0（用户确认 tag）：pending

## 修复节点门禁清单（每节点全量跑）
- `cargo fmt --all -- --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-features`
- `cargo check --target wasm32-unknown-unknown -p vane-core` + `-p vane-wasm`
- Node 绑定：`cd crates/vane-node && npm test`
- Go 绑定：构建 vane-ffi 后 `cd bindings/go && go test ./...`
- check-no-std-fs.sh / cargo deny check / wasm 体积 / recall / nDCG

## 编排决策记录
- SPEC v1.5 合并修订：阶段二 §12.3 不单独提，合并至阶段四一次性 v1.5（§12.2+§12.3+§13.2）。
- 分支：首个 implementer 写代码前创建 `feat/m3-web-npm`。
- 审查/实现重叠：implementer(写) || reviewer(只读)；同时只一个 implementer 写。

## 侦察结论（2026-08-11，Explore agent 回收）

### vane-wasm 现状
- Cargo.toml：crate-type=[cdylib,rlib]；features simd128（marker，不门控 core，双产物由 RUSTFLAGS=+simd128 区分）/ worker（聚合 web-sys/js-sys/wasm-bindgen-futures/sha2/opfs/idb/jieba）/ jieba（仅算法代码，dict-zh 红线永不启进 wasm）；无 build.rs，wasm-bindgen --target 由脚本指定。
- src/lib.rs（508 行）：wasm-bindgen 胶水导出，句柄 u64 + RwLock。
- src/worker.rs（1228 行）：VaneWorker Dedicated Worker 壳，WorkerOpts 解析 dictUrl/dictSha256/dictData/vfs/dbPath。**JS 契约冻结——@vane-rs/web 是其 npm 包装，不改 worker.rs**。
- src/dict_loader.rs（374 行）：三渠道已实现——①dictData 内联（M2-04）sha256 校验通过直接返回 ②CDN fetch+VFS缓存 ③降级 bigram+warn 永不抛错。
- src/simd_probe.rs（136 行）：运行时探针 WebAssembly.validate(SIMD128_TEST_MODULE)。
- src/worker.js（101 行）：wasm-bindgen 生成 Worker 入口 JS 胶水。

### 构建现状
- scripts/build-wasm-variants.sh：cargo build + wasm-opt -Oz，产 target/wasm-variants/vane_wasm_{simd,scalar}.wasm。**不跑 wasm-bindgen**。800KB gzip 门禁已实现（实测 simd 312KB / scalar 315KB，远低于红线）。
- demo/build.sh：才跑 wasm-bindgen --target web + wasm-opt -Oz，产 demo/pkg/（vane_wasm.js 33KB + 双变体 .wasm + dict.bin + sha256_prefix.bin）。还额外 --target nodejs 产 pkg-node/ 供 e2e。
- release.yml build-wasm job：只跑 build-wasm-variants.sh，upload 裸 .wasm 到 Release，**无 JS glue**。

### vane-dict-zh 现状
- Cargo.toml version=2026.8.0, publish=false；src/lib.rs DICT_BIN=include_bytes(data/dict.bin 1.41MB)，DICT_VERSION="2026.08"。
- Node 作 cargo path 依赖内嵌；Go go:embed gzip；WASM 永不进（红线）。
- check-dict-hash.sh + dict_test.rs：三渠道哈希一致校验已实现。

### release.yml napi-rs 3.x 正解（以当前代码为准，不参考 post-m2 文档）
- create-npm-dirs（复数，3.x）；napi artifacts --output-dir . --npm-dir npm；napi pre-publish -t npm；package.json napi.targets（非 triples）；napi.config.json 已删除（合并进 package.json napi 字段）；publishConfig.access=public；napi/napi-derive=3，napi-build=2。

## 风险登记
- R1（高，已化解）：post-m2 文档记载"napi-rs 2.x 锁定"但代码已 3.x。M3 正解以当前 release.yml/package.json 为准，**不参考 post-m2 命令结论**。
- R2（高，M3 范围）：release.yml build-wasm 不跑 wasm-bindgen，Release 是裸 .wasm 无 JS glue。M3 阶段四需新增 build-web job 产 ESM glue + 双变体 .wasm + publish @vane-rs/web。
- R3（低，已达标）：wasm 体积 simd 312KB / scalar 315KB gzip，远低于 800KB 红线。
- R4（中，M3 SPEC 修订范围）：SPEC §12.3 文本写 @vane/dict-zh / @vane/slim 但实际 scope @vane-rs，无 slim 变体，vane-dict-zh publish=false 内嵌。v1.5 修订时同步修正 scope + 新增 @vane-rs/dict-zh 独立 npm 通道。
- R5（中，非 M3 范围）：SPEC §12.2 Node prebuilt 追加目标（musl/arm64-win）未实现。M3 不扩平台。
- R6（低，Task 1 设计定）：wasm-bindgen --target 双口径（demo 产 web + nodejs）。@vane-rs/web target 策略待定。
- R7（低，设计注意）：dict_loader sha256 校验在 not(jieba) 下恒返 false。worker 启 jieba 所以正常，@vane-rs/web 需确保 worker feature 链启 jieba。
- R8（低，Task 5 定）：vane-dict-zh version 双轨（Cargo 2026.8.0 semver vs DICT_VERSION 2026.08 日历）。@vane-rs/dict-zh npm 包 version 策略待定。

## parked / deferred minors（终局审查 triage）
- Task 2 M1：build-web.sh stat 跨平台 fallback 仅影响日志（macOS -f%z / Linux -c%s），不影响体积门禁。与 demo/build.sh 同模式。
- Task 2 M2：README.md Vane 接口单行内联文本（非结构化 TS 块），Task 2 占位，Task 3 已补全。
- Task 3 M1：index.ts L56-59 worker.onerror 后未设 closed=true（防御性增强选项，worker 自身 catch 透传错误，当前行为可接受）。
- Task 3 M2：index.ts L99-105 dictData.buffer as ArrayBuffer 对 Uint8Array 部分视图的静默错误（文档 L121 + 代码注释已说明 .slice() 解决方案，典型用法不受影响）。
- Task 3 M3：worker.ts e.data/msg 无类型标注（MessageEvent.data 固有 any，主线程侧类型安全由 types.ts 保证）。

## 任务完成记录
- Task 1：complete — 设计产出 + 用户确认（2026-08-11）。3 决策全选推荐：bindings/web/ + optionalDep + --target web 单一。设计落 docs/plans/m3/task-1-design.md。
- Task 2：in_progress — build-web.sh + bindings/web/ 骨架。分支 feat/m3-web-npm 已创建。编排者边界：Task 2 只管 wasm 产物 + package.json + 目录骨架，不含 src/*.ts/tsc（Task 3）。
  - 实现完成（commits 520a00e + 9659a9e）：DONE_WITH_CONCERNS（C1 license 矛盾 / C2 scalar 跑 wasm-bindgen 技术必需 / C3 体积略高 / C4 target_features strip）。
  - 门禁全绿：fmt ✅ / clippy ✅（-D warnings）/ wasm32 core+wasm ✅ / cargo test --workspace ✅（exit 0）/ 体积 simd 318KB+scalar 320KB gzip ≤800KB ✅ / vane-wasm 零改动 ✅。
  - C1 修复（commit 1d442d7）：LICENSE + package.json → Apache-2.0。
  - Review（task reviewer）：Spec ✅ 8/8，quality Approved with 1 Important（I1 README.md:102 License 仍写 MIT 未随 C1 同步）。C2/C3/C4 全确认合理。Minor M1(stat fallback 仅日志)/M2(README 接口占位) deferred。
  - Fix round 1/5：I1 README.md:102 MIT→Apache-2.0（唤醒 C1 implementer 补 README 遗漏）。
  - Fix round 1 re-review：I1 ADDRESSED + 无新 breakage（commit ef8cc04）。all findings addressed。
  - **Task 2：complete（commits bcdd6c3..ef8cc04，review clean）**。reviewer deferred minors：M1(stat fallback 仅日志)/M2(README 接口占位) → 终局审查 triage。
- Task 3：in_progress — @vane-rs/web JS/TS 源码 + tsc 编译。
  - 实现完成（commit b480be8，11 files +1124/-24）：DONE。src/types.ts/probe.ts/worker.ts/index.ts/vane_wasm.d.ts + tsconfig.json + build-web.sh tsc 步骤 + package.json devDep/scripts + README 补全。
  - 门禁全绿：fmt ✅ / clippy ✅ / wasm32 ✅ / cargo test --workspace ✅ / vane-wasm 零改动 ✅ / 体积 simd 318KB+scalar 320KB gzip ≤800KB ✅ / tsc 零错误 ✅ / ESM 冒烟 createVane=function ✅ / probe.ts SIMD128_TEST_MODULE 50 bytes 与 simd_probe.rs 逐字节一致 ✅。
  - Review（task reviewer）：Spec ✅ 13/13，quality Approved（无 Critical/Important）。C1-C5 全确认合理。TS 类型与 worker.rs 字段名核对表全覆盖（VaneWorkerOpts/FieldSchema/Doc/SearchQuery/OpenOptions/CollectionOptions/Hit/Vane 接口）。implementer 正确发现设计 §6 草案 FieldSchema type 与 worker.rs 不一致，以 worker.rs 为准（冻结契约遵守）。
  - **Task 3：complete（commits ef8cc04..b480be8，review clean）**。deferred minors：M1(onerror 未设 closed 防御性)/M2(dictData 部分视图 guard 文档已说明)/M3(worker msg 无类型固有) → 终局审查 triage。
- Task 4：complete — 阶段一门禁自证归档（编排者节点，非 implementer）。全量门禁全绿：fmt/clippy/wasm32 core+wasm/cargo test/build-web.sh 全流程/体积双变体≤800KB/tsc 零错误/ESM 冒烟 createVane=function/probe 50 bytes 对齐/vane-wasm 零改动。report 归档 docs/plans/m3/task-4-report.md。**阶段一闭环**。
- Task 5：in_progress — @vane-rs/dict-zh npm 包（package.json + .npmignore + README + exports ./dict.bin + ./sha256_prefix.bin）。跨任务依赖：@vane-rs/web optionalDep + CDN URL 依赖此包 version=2026.8.0 + exports。
  - 实现完成（commit 2aadd28）：DONE。package.json + .npmignore + README.md。npm pack 产物 4 文件 1.5MB。冻结文件零改动。
  - 门禁全绿：clippy ✅ / fmt ✅ / npm pack dry-run 产物验证 ✅。
  - Review（task reviewer）：Spec ✅ 8/8，quality Issues with 1 Important（I1 LICENSE 文件缺失——package.json 声明 Apache-2.0 但无 LICENSE 文件 + files 未列 LICENSE，与 @vane-rs/web 模式不一致，Apache-2.0 §4.1 文本保留义务）。C2(无根导出)/C3(npmignore 冗余) 确认无问题。
  - Fix round 1/5：I1 cp LICENSE + files 加 "LICENSE"（唤醒 Task 5 implementer，与 Task 2 C1 同模式）。
  - Fix round 1 re-review：I1 ADDRESSED + 无新 breakage（commit 6857c70）。all findings addressed。无 deferred minors。
  - **Task 5：complete（commits b480be8..6857c70，review clean）**。@vane-rs/dict-zh npm 包就绪（package.json exports ./dict.bin + ./sha256_prefix.bin，version 2026.8.0，LICENSE 对齐，npm pack 5 文件 1.5MB）。
- Task 6：in_progress — 四渠道哈希校验扩展（check-dict-hash.sh + dict_test.rs 加 Web npm dictData 第四渠道）。
  - 实现完成（commit d750558，2 files +125/-16）：DONE。第四渠道两层校验（元数据 grep + npm pack 字节比对）+ Rust 测试 npm_package_json_references_source_dict_bin。
  - 门禁全绿：check-dict-hash.sh 四渠道 exit 0（npm pack sha256 = Node sha256 = efa4eee3...）/ cargo test vane-dict-zh 8 passed / clippy ✅ / fmt ✅ / 冻结文件零改动。
  - Review（task reviewer）：Spec ✅ 6/6，quality Approved（2 Minor，无 Critical/Important）。⚠️ Cannot verify 项编排者已自跑全绿确认。
  - **Task 6：complete（commits 6857c70..d750558，review clean）**。deferred minors：M1(TMPDIR_PACK 失败路径未清理，系统清理 /tmp)/M2(grep -F 依赖 JSON 单行格式，fail-closed) → 终局审查 triage。**阶段二闭环**。
- Task 7：in_progress — examples/vite/ 示例。关键点：@vane-rs/web + @vane-rs/dict-zh 未发 npm（Task 11 才发），用 file: 本地路径引用。
  - 实现完成（commit 8dadd7b，9 files +1511）：DONE。vite build 成功——7 模块，worker chunk 12.76KB + wasm asset（simd 804KB + bg 815KB）+ dict.bin 1.48MB，141ms。@vane-rs/web 设计 §9 vite 零配置目标验证通过（无需 wasm/worker 插件，仅 assetsInclude ['**/*.bin'] 一行）。
  - 门禁：vite build 复跑确认 ✅ / 冻结文件零改动 ✅。
  - Review（task reviewer）：Spec ✅ 5/5，quality Approved（1 Minor）。C1-C4 全确认。reviewer 深度验证 API 一致性（main.ts 与 types.ts + worker.rs 逐一核对）+ 实际 dist/ 产出结构。
  - **Task 7：complete（commits d750558..8dadd7b，review clean）**。deferred minors：M1(README 产出列表与实际 dist/ 不符：scalar→bg + sha256 内联) → Task 9 文档打磨修正。文档建议：@vane-rs/web README vite 集成节补 assetsInclude + worker IIFE 约束 → Task 9/11。
- Task 8：in_progress — examples/webpack/ 示例（webpack 5 wasm + worker）。file: 本地路径引用，同 Task 7。
  - 实现完成（commits 55073cb + 3f4928f，9 files +5470）：DONE。webpack build 成功——wasm asset×3 + dict asset×2 + worker ESM chunk 13.1KB + 主线程 4.3KB，1 warning（html-webpack-plugin with 语句，不影响功能），1068ms。@vane-rs/web 设计 §9.3 验证通过（outputModule 足够，不需 asyncWebAssembly，init(wasmUrl) 绕过生效）。
  - 门禁：webpack build 复跑确认 ✅ / 冻结文件零改动 ✅。
  - Review（task reviewer）：Spec ✅ 5/5，quality Approved（无 Critical/Important/Minor）。C1-C6 全确认。reviewer 深度验证 main.ts 一致性 + dist/ 实际产物 + webpack.config 每项。
  - **Task 8：complete（commits 8dadd7b..3f4928f，review clean）**。无 deferred minors。文档建议（Task 9/11）：@vane-rs/web README webpack 集成节补 scriptLoading:'module' + tsconfig 不用 noEmit。
- Task 9：in_progress — 文档站 vite/webpack 集成指南页。汇总 Task 7/8 集成经验 + 修正 Task 7 README M1（产出列表 dist/ 不符）。
  - 实现完成（commits 4a349fd + 16f0400，5 files +438/-2）：DONE。WebIntegration.tsx+.css（7 节 h2：install/vite/webpack/usage/dictdata/worker/gotchas）+ routes.ts/nav.ts 注册 + examples/vite/README.md M1 修正（bg 替代 scalar + sha256 内联）。
  - 门禁全绿：tsc --noEmit exit 0 ✅ / vite build 成功（176 modules, 15 routes, sitemap 含 /vane/guides/web-integration, 832ms）✅ / 注册 grep 确认 ✅ / 冻结文件零改动 ✅。
  - Review（task reviewer）：Spec ✅ 9/9，quality Approved（无 Critical/Important/Minor）。reviewer 深度验证 vite/webpack 配置与 examples 逐项一致 + Callout 覆盖 5 项 + BEM/lang 约定 + M1 修正。清掉 Task 7 M1 deferred minor。
  - **Task 9：complete（commits 3f4928f..16f0400，review clean）**。无 deferred minors。**阶段三闭环**（vite+webpack 示例 + 文档站集成页全完成）。
- Task 10：deferred（编排者决策）—— CI 冒烟 job 是 Could 非 Must。现在 @vane-rs/web 未发 npm，CI 冒烟只能测 file: 本地引用（与 Task 7/8 重复）。发版后（Task 14）用 install-matrix 扩展真正 `npm i @vane-rs/web` 测试价值更高。跳过，直接进 Task 11 主线。
- Task 11：in_progress — release.yml build-web job + publish @vane-rs/web + @vane-rs/dict-zh（M3 核心交付）。
  - 实现完成（commit 4c4e4b6，+59/-1）：DONE。build-web job + Stage web dist/ + @vane-rs/web publish + @vane-rs/dict-zh publish。actionlint exit 0 + yaml OK + node publish 链完整 + 冻结零改动。
  - Review（task reviewer）：Spec ✅ 8/8，quality Approved。C1/C2/C3/C5 确认合理。**C4 publish 顺序**：reviewer 同意编排者倾向，建议调换为 dict-zh 先 web 后（dict-zh 失败阻塞 web publish，避免断裂 optionalDep 上线）。
  - Fix round 1/5：C4 release.yml 两个 publish step 顺序对调（dict-zh 先 web 后）。
  - Fix round 1 re-review：C4 ADDRESSED + 无新 breakage（commit fabce74）。C1/C2/C3/C5 是 observation 非 open finding。
  - **Task 11：complete（commits 16f0400..fabce74，review clean）**。release.yml 四端扩展完成：build-web job + Stage dist + dict-zh 先 web 后 publish。无 deferred minors。
- Task 12：in_progress — bump 0.1.2→0.2.0（Cargo workspace @vane-rs/node + install-matrix；@vane-rs/web 已 0.2.0 + @vane-rs/dict-zh 已 2026.8.0 不动）。
  - 实现完成（commit ca19dc5，4 files +11/-11）：DONE。Cargo.toml + Cargo.lock + vane-node package.json + install-matrix.yml 全 0.2.0。零 .rs 改动。
  - 门禁全绿：fmt ✅ / clippy ✅ / cargo test 全 ok / npm test 17/17（implementer report）。
  - C1（release.yml:10 description 示例 "0.1.2"）：cosmetic，Task 13 顺手改。
  - **Task 12：complete（commit fabce74..ca19dc5，简化 review——version bump 机械改 + 门禁全绿 + version 同步已验证）**。
- Task 13：in_progress — SPEC v1.4→v1.5 修订（§12.2四端 + §12.3四渠道 + §13.2门禁 + changelog，**需用户批准**）。先派只读 Plan agent 出草案。
  - 草案完成（Plan/opus 只读）：7 节修订对照 + 2 决策点 + 4 额外矛盾。编排者审查通过。落 docs/plans/m3/task-13-spec-v1.5-draft.md。
  - **用户批准**：v1.5 修订 + Node prebuilt 标注"未实现，顺延"（决策点 1 方案 B）。
  - 实现完成（commits 5d092f8 + 9b8269f + 7277e4d + C1 typo fix）：SPEC.md L1 v1.5 + §12.1 补全 + §12.2 四端+未实现顺延 + §12.3 四渠道+scope 修正+删 slim + §12.4 四端+dict-zh 日历版 + §13.2 第5项 Web 门禁 + changelog v1.5 + release.yml:10 cosmetic 0.2.0。
  - C1 typo 修复：SPEC 两处 dict_tests.rs→dict_test.rs（文件名单数，0 残留）。
  - **Task 13：complete（commits ca19dc5..HEAD，简化 review——纯文档 + 用户已批准 + 7 验证点过 + C1 typo 已修）**。
  - ⚠️ 跨任务待验证（reviewer 标 Cannot verify）：@vane-rs/dict-zh npm 可用性(Task 5) / vite 集成(Task 7) / npm pack 产物(Task 4) / probe.js 对齐(Task 3) / worker.js 显式 URL(Task 3) / dict-zh exports(5)。无 Task 2 gap。
