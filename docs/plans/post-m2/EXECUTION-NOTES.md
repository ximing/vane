# post-M2 编排者 Ledger

> 计划：docs/plans/post-m2/README.md
> 这是编排者的裁决与状态总账，compaction 后据此恢复。

## 现实基线（2026-08-10）
- git remote 空；gh 认证用户 ximing（repo/workflow scopes）；用户授权直接 gh 建仓+push。
- Cargo.lock 已提交；version 0.1.0；workflow 4 个（ci.yml 16 jobs/benchmark/install-matrix/release）。
- 未跟踪：`.agents/`（skill 缓存，建议 gitignore）、`AGENTS.md`（Codex 镜像，指向不存在的 .Codex/，待用户决）。

## 模块状态总表

| 阶段 | 模块 | 状态 | 备注 |
|------|------|------|------|
| 零 | Z0 CI 静态预审 | ✅完成 | 1必炸+14隐患，z0-ci-preaudit-report.md |
| 零 | Z1 预修 + gitignore | ✅完成 | commit 708e88d；go-cross pin zig0.15.2+czb0.21.4；8隐患修齐 |
| 零 | Z2 release.yml 确认 | ✅完成 | Z0确认Node就绪；用户决策补全三端 |
| 零 | Z3 建仓+push | ✅完成 | https://github.com/ximing/vane private |
| 一 | CI 首跑+修复循环 | ✅完成 | run 31366858024 全绿，1 轮修复 |
| 二 | P1 export 下载 | ✅完成 | readFile op + demo 下载闭环，CI 绿 |
| 二 | P2 5万 Playwright CI | ⏭跳过 | 用户决策不做浏览器验收，继续后续 |
| 二 | P3 词典 CDN 部署 | ✅完成 | jsdelivr gh URL；check-dict-hash.sh Go 渠道启用三层校验全绿 |
| 二 | Z2-补 Go/WASM 发布 | ✅完成 | release.yml 三端 prebuilt，CI 绿 |
| 三 | R1 version 同步 | ✅完成 | version 0.1.0 三端同步 + go.mod module 路径 vane/vane→ximing/vane |
| 三 | R1b npm scope | 进行中 | @vane-rs/node → @vane-rs/node（用户 scope 是 @vane-rs） |
| 三 | R2 tag+prebuilt | 进行中 | release.yml Node 发布重构（napi-rs 3.8.5 架构 bug）|
| 三 | R2-fix napi artifacts | 进行中 | build-go/build-wasm ✅；build(napi) 失败：napi artifacts --target 不支持+要求全 target+无 napi publish。重构 build job 上传 .node / release job artifacts+pre-publish+publish |
| 三 | R3 发版 | 待 | 需用户确认 |

## 裁决记录
- 2026-08-10 启动。Z0 派只读预审 SubAgent（sonnet）→ 1必炸(go-cross)+14隐患。
- Z1 预修 SubAgent（sonnet）→ commit 708e88d。go-cross pin zig0.15.2+czb0.21.4（changelog修zig0.15 macOS交叉），verify改test-f，8隐患修齐（permissions/rust-cache/timeout/concurrency/xxd/deny精确/critcmp/.agents gitignore）。YAML全绿，actionlint仅macos-13 pre-existing告警。
- 暂不修（裁决）：rust-toolchain pin(接受stable浮动)、百万heavy job(CI绿后)、双变体体积门禁(可选)。
- **用户决策(AskUserQuestion)**：①Go/WASM发布=补全三端（建仓先进行，CI绿后补Go4平台.a+WASM双产物）；②AGENTS.md=进仓并修正指向(.Codex→.claude, crates/*/AGENTS.md→CLAUDE.md)；③建仓=vane/private 起步。
- AGENTS.md 修正 SubAgent(haiku) 进行中。完成后 commit → gh repo create vane --private --source=. --remote=origin --push → 进阶段一。
- **建仓完成**：https://github.com/ximing/vane（private），commit 528855e push origin main。
- **CI 首跑 run 31364131339**：14 job success + go-cross 4 failure + cold-start cancelled。远好于预期，Z1 预修奏效。
  - go-cross 根因：`cargo zigbuild --target ${{ matrix.zig_target }}` 传 zig-style target，rustc 不认（Z0 标注的 zig_target ⚠️ 隐患，Z1 保留待验证→现验证失败）。修：--target 改用 matrix.target（Rust triple）+ cargo-zigbuild 0.21.4→0.23.0（WebSearch 验证 zig0.15.2 配对）。
  - cold-start 根因：10万 HNSW fixture 用 debug 模式跑（无 --release），卡死 30min timeout。修：加 --release（与 jieba/ndcg 一致）+ timeout 30→60min。
  - 修复 SubAgent(sonnet) 进行中。完成后 commit → push 重跑 → 监控。
- **阶段一完成 ✅**：run 31366858024 CI 全绿（16 job 含 go-cross 4 平台 + cold-start），1 轮修复。commit pushed。"本地绿→可信交付"达成。
- **P1 export 下载闭环 ✅**：原 SubAgent 完成 worker.rs readFile op（read_file_sync 流式 + read_file wasm_bindgen + 2 测试）+ demo/main.js + demo/worker.js 后进程中断；自证 SubAgent 又 API 报错（2 次失败换策略）→ 编排者直接跑自证全过（wasm32 check / 41 passed / clippy / 391KB gzip ≤800KB / demo 协议一致）。commit pushed，CI run 31371709686 16 job 全绿，P1 不回退确认。
- **Z2-补 Go/WASM 发布 ✅**：release.yml 补全三端——build-go（zig 0.15.2+czb0.23.0 交叉 4 平台 .a，复用 ci.yml go-cross 验证配置）+ build-wasm（binaryen+build-wasm-variants.sh 双变体）+ release job（聚合 npm publish + softprops/action-gh-release attach Go .a×4 + WASM .wasm×2，if tag）。顺带修原 publish 缺 napi install bug + macos-13→macos-15-intel。yaml/actionlint 通过。release.yml 真实验证留发版前 workflow_dispatch（R2）。
- **P3 词典 CDN ✅**：用户决策 jsdelivr gh。demo/main.js DICT_URL → `https://cdn.jsdelivr.net/gh/ximing/vane@main/crates/vane-dict-zh/data/dict.bin`。check-dict-hash.sh Go 渠道启用（源字节 sha256 一致 + DICT_VERSION 一致 + zstd 头部 prefix 直接比对，三层全绿）。private 仓库期间 jsdelivr 404 降级 bigram，转 public 自动生效。未触及 SPEC/dict_loader.rs/core。p3-report.md。
