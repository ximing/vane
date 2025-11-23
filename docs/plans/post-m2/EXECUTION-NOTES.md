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
| 零 | Z0 CI 静态预审 | 进行中 | 只读 SubAgent |
| 零 | Z1 预修 + gitignore | 待 | 依赖 Z0 |
| 零 | Z2 release.yml 确认 | 待 | |
| 零 | Z3 建仓+push | 待 | 需用户确认仓库名/可见性 |
| 一 | CI 首跑+修复循环 | 待 | 依赖 Z3 |
| 二 | P1 export 下载 | 待 | |
| 二 | P2 5万 Playwright CI | 待 | |
| 二 | P3 词典 CDN 部署 | 待 | |
| 三 | R1 version 同步 | 待 | |
| 三 | R2 tag+prebuilt | 待 | |
| 三 | R3 发版 | 待 | 需用户确认 |

## 裁决记录
- 2026-08-10 启动。Z0 派只读预审 SubAgent（sonnet）。
- `.agents/` 倾向 gitignore（与 .superpowers/ 同类）；`AGENTS.md` 待用户决，攒批到建仓前汇报。
