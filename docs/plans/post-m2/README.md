# Vane post-M2 收尾与 v0.1.0 发版

> 编排者（Orchestrator）执行计划。M0+M1+M2 已完成（HEAD f7ee06f，498 测试绿，SPEC v1.3）。
> 本次目标：建仓推送 → CI 跑通修复 → 产品化收尾 → 发版 v0.1.0（三端 prebuilt）。

## 现实基线（2026-08-10 核查）

- **git remote 为空** —— CI 从未在远程真跑过，所有门禁仅本地验证。
- **gh 已认证**（用户 ximing，scopes: repo/workflow）—— 用户授权直接用 `gh` 建仓 + push。
- Cargo.lock 已提交 ✅；version 0.1.0 ✅；4 个 workflow（ci.yml 16 jobs + benchmark + install-matrix + release）。
- 工作树未跟踪项：`.agents/`（skill 缓存）、`AGENTS.md`（CLAUDE.md 的 Codex 镜像，指向不存在的 `.Codex/`）—— 建仓前待决。

## 阶段分解

### 阶段零：CI 配置预审 + 建仓准备
- **Z0** CI workflow 静态预审（只读）—— 跨平台/环境隐患清单。
- **Z1** 预修明显问题 + .gitignore 补全（.agents/ 等）。
- **Z2** 确认 release.yml 三端 prebuilt 配置就绪。
- **Z3** 建仓 + push（向用户确认仓库名/可见性后 `gh repo create` + `git push`）。

### 阶段一：CI 首跑 + 修复循环
- push 后 `gh run watch` 监控 → 每个失败 job 派 SubAgent 读日志诊断修复 → 重跑 → main CI 全绿。
- 重点预期失败：go-cross(zig)、wasm-recall(wasm-bindgen-cli)、跨平台 float、heavy job 超时。
- 同一 job 修 2 次仍失败 → 换策略（pin 版本/换镜像/降级手动触发）或上报。

### 阶段二：产品化收尾
- **P1** 浏览器 export 下载闭环（worker.js readFile op → 主线程下载 backup.vane）。
- **P2** 5 万文档 Playwright CI 验收（OPFS+IDB 双后端）。
- **P3** 词典 CDN 真实部署 + 三渠道哈希一致校验。
- f32 SIMD 不做（post-发版可选，涉 SPEC 修订）。

### 阶段三：发版 v0.1.0
- **R1** version 三端同步确认（Cargo.toml + package.json + go.mod）。
- **R2** 打 tag v0.1.0 → release.yml 三端 prebuilt 产出。
- **R3** 三渠道词典哈希一致 + 发版说明（引 M2-SUMMARY）。**发版需用户确认。**

## 完成定义（DoD）
- main CI 全绿（16 jobs 远程真跑，含 go-cross/wasm-recall/cold-start/ndcg-wiki）；
- 浏览器 export 下载可用（demo 端到端下载 backup.vane）；
- 5 万 Playwright CI 验收通过（OPFS+IDB 双后端）；
- 词典 CDN 真实部署 + 三渠道哈希一致；
- v0.1.0 tag 发版 + 三端 prebuilt 产出；
- 本目录计划标记完成 + post-m2 总结报告。

## 约束（继承 post-M2 prompt）
- 纯编排者：禁止写代码，唯一例外维护本目录 + 任务看板。
- 串行+审查/实现重叠（worktree 不可用）。
- MoSCoW 即合同；Won't-have（内置 embedding/GPU/SQL/分布式）不得触碰。
- 词典永不进 wasm（800KB gzip 红线）。
- core 禁 std::fs/std::net/mmap；cfg 仅 VFS/Executor/vane-wasm binding（I-5，SPEC v1.3）。
- 依赖黑名单；rayon 仅 Executor impl。
- 不改 M0/M1/M2 已冻结 pub API；矛盾走 SPEC 修订向用户提议。
- 建仓/push 已授权；发版需用户确认。
