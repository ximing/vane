# Z1 CI 预修报告

> 执行日期：2026-08-10
> 范围：按 Z0 预审报告，预修 `.github/workflows/*.yml`（4 个）+ 根 `.gitignore`
> 约束：只改 workflow YAML 与 .gitignore；不动 Rust/Go/JS 代码、不动 Cargo.toml
> 校验：PyYAML 全部解析通过；actionlint 仅 1 项 pre-existing 告警（macos-13，超范围）

## 修改总览（文件 × 改动项矩阵）

| 文件 | 改动项 |
|---|---|
| `ci.yml` | permissions(read) · concurrency · 16 jobs 加 timeout-minutes · 14 Rust job 加 rust-cache · go-cross zig+cargo-zigbuild 版本 pin + verify 移除 \|\|true · deny 版本精确 pin · dict-hash 预装 xxd |
| `benchmark.yml` | permissions(read) · timeout-minutes:30 · rust-cache · critcmp pin 0.1.8 |
| `install-matrix.yml` | permissions(read) · timeout-minutes:10 |
| `release.yml` | workflow 级 permissions(read) · publish job permissions(contents:write, packages:write) · build/publish timeout-minutes · build job rust-cache |
| `.gitignore` | 追加 `.agents/`（git ls-files .agents/ 为空，无误伤） |

共改 5 个文件、9 类改动项（跨 4 个 workflow × 多 job）。

## go-cross 版本查证结论

| 组件 | pin 版本 | 来源 |
|---|---|---|
| zig | **0.15.2**（2025-11-25 最新 stable） | Snyk ziglang 版本页 / ReleaseAlert / FreeBSD ports |
| cargo-zigbuild | **0.21.4**（2026-01-27 发布） | GitHub rust-cross/cargo-deny releases |

依据：cargo-zigbuild **v0.21.4** changelog 明确写有 `Fix zig 0.15 macOS cross-compilation issues` 与 `add missing libc++ defines for zig 0.15+ bindgen support`。本 go-cross matrix 恰含 `x86_64-macos`/`aarch64-macos` 两个 darwin target，该修复直接覆盖。0.21.4 是首个显式声明 zig 0.15 兼容的版本，故选其而非更新的 0.22.x/0.23.0（0.23.0 为 2026-06 最新，但无显式 zig-0.15.2 兼容声明，风险更不可控）。

cargo-zigbuild README 同时确认接受 zig-style target（`x86_64-linux-gnu` 等），本 matrix 的 `zig_target` 字段无需改动。

## 每项改动的 before → after

### 1. go-cross 必炸修复（Z0 必炸 #1 + 改进 6）
- before：`setup-zig version: '0.13.0'` + `cargo install cargo-zigbuild --locked`（未 pin）+ `ls -lh ... || true`
- after：`setup-zig version: '0.15.2'` + `cargo install cargo-zigbuild --locked --version 0.21.4` + `test -f .../libvane_ffi.a` 断言（block scalar，缺失即失败）

### 2. permissions 最小化（改进 1+8）
- before：4 个 workflow 均无 `permissions:` 块（GitHub 默认 write-all）
- after：
  - ci.yml / benchmark.yml / install-matrix.yml / release.yml 顶部均 `permissions: contents: read`
  - release.yml `publish` job 单独 `permissions: { contents: write, packages: write }`（npm publish 走 NPM_TOKEN，packages: write 为预留 GH packages 产物场景）
  - release.yml `build` job 继承 workflow 级 contents: read

### 3. rust-cache（改进 2）
- before：无 cache，每 job 从零编译 ~120 crates
- after：所有跑 cargo 命令的 job 在 toolchain 安装后、cargo 命令前插入 `Swatinem/rust-cache@v2`。dict-size / dict-hash（仅 bash 脚本，无 cargo）不加。

### 4. timeout-minutes（改进 3）
- before：所有 job 无超时（GitHub 默认 360min）
- after（按编排者裁决的档位）：
  - 10min：fmt · dict-size · dict-hash · install-matrix(smoke)
  - 15min：recall · wasm32-check · deny · corpus-compat
  - 20min：clippy · test · wasm32-size · jieba-compat · ndcg-wiki · go-host（release 编译较慢，给 20）
  - 30min：cold-start · go-cross · wasm-recall（heavy）· benchmark · release.build

### 5. .gitignore 补 .agents/（改进 4）
- before：缺 `.agents/`
- after：`.gitignore` 追加 `.agents/`。校验 `git ls-files .agents/` 为空，无误伤已跟踪文件。

### 6. concurrency（改进 5）
- before：无 concurrency 块
- after：ci.yml 顶部 `concurrency: { group: ci-${{ github.ref }}, cancel-in-progress: true }`。benchmark/release/install-matrix 触发频率低或串行依赖，不加。

### 7. dict-hash xxd 预装（改进 7）
- before：`check-dict-hash.sh` 用 `xxd -p`，ubuntu-latest 默认不含 xxd
- after：dict-hash job 增加 `Install xxd` 步骤 `sudo apt-get install -y xxd`（Ubuntu 22.04+ 有独立 `xxd` 包）。

### 8. cargo-deny 版本（改进 9）
- before：`cargo install cargo-deny --locked --version ^0.19`
- after：`cargo install cargo-deny --locked --version 0.19.9`（查证 0.19.x 最新，2026-06-15 发布；0.19.x 系列自 2026-01 起共 10 个版本，`^0.19` 原本也能解析，精确 pin 更确定）

### 9. critcmp pin（改进 14）
- before：`cargo install critcmp --locked`（未 pin）
- after：`cargo install critcmp --locked --version 0.1.8`（critcmp 最新且唯一活跃版本，2023-07 发布，此后无新版本；长期稳定）

## YAML 校验结果

```
$ python3 -c "import yaml,glob; [yaml.safe_load(open(f)) for f in glob.glob('.github/workflows/*.yml')]"
YAML OK: all workflows parse
```

actionlint（`go install github.com/rhysd/actionlint/cmd/actionlint@latest` 后运行）：
- 仅 1 项告警：`release.yml:33` runner label `macos-13` unknown（actionlint 标签库已无 macos-13，GitHub 列出 macos-15-intel / macos-26-intel 等）。
- **此为 pre-existing 问题**（原 release.yml matrix 即为 `macos-13`），不在 Z0 必修/改进清单内，本次未动。
- 其余 4 个 workflow 0 告警。

## 遗留 / 待 CI 验证项

1. **go-cross 版本组合待首跑验证**：zig 0.15.2 + cargo-zigbuild 0.21.4 的兼容性基于 v0.21.4 changelog 文字声明（修复 zig 0.15 macOS 交叉），未本地实测。若 CI 首跑 4 个 target 任一失败，按优先级回退尝试：① cargo-zigbuild 0.22.3（2026-04）→ ② 0.23.0（2026-06 最新）→ ③ 若 zig 0.15.2 与 cargo-zigbuild 任何版本均有问题，降级 zig 0.14.1 + 对应 cargo-zigbuild。
2. **release.yml `macos-13` runner 可能已下线**（actionlint 告警）：GitHub 已将 intel macos runner 推进到 macos-15-intel / macos-26-intel。release.yml 的 `macos-13` 在首次 tag 发布时可能报 `no runner found`。建议后续 release 阶段（Z2）改为 `macos-13-large` 或 `macos-15-intel`。**超出本任务范围，未改。**
3. **zig_target zig-style 格式**（Z0 ⚠️）：保留 `x86_64-linux-gnu` 等 zig-style target（cargo-zigbuild README 确认接受）。若首跑发现不识别，再改 Rust triple。未改、待验证。
4. **NPM_TOKEN secret**（Z0 ⚠️）：release.yml publish 依赖 `secrets.NPM_TOKEN`，首次发布前需在 GitHub repo settings 配置。配置项，非代码改动。
5. 不修项（编排者裁决）：rust-toolchain pin / 百万 heavy job / 双变体体积门禁 / Go .a 与 WASM 发布——均按裁决不动。

## 建议 commit message

```
ci: 预修必炸项与高性价比隐患（Z1）

- go-cross: pin zig 0.15.2 + cargo-zigbuild 0.21.4，移除 verify || true
- 4 workflow 加 permissions: contents: read；release publish 提权 write
- 所有 Rust job 加 Swatinem/rust-cache@v2
- 所有 job 加 timeout-minutes（10/15/20/30 分档）
- ci.yml 加 concurrency cancel-in-progress
- .gitignore 补 .agents/
- dict-hash 预装 xxd
- cargo-deny 精确 pin 0.19.9；critcmp pin 0.1.8

YAML 全绿；actionlint 仅 macos-13 pre-existing 告警（超范围未动）。
go-cross 版本组合待 CI 首跑验证。

Refs: docs/plans/post-m2/z0-ci-preaudit-report.md
```
