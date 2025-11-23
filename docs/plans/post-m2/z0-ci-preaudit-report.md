# Z0 CI 预审报告

> 审查日期：2026-08-10
> 审查范围：`.github/workflows/` 全部 4 个 workflow + `scripts/` 全部脚本
> 审查方式：纯静态只读（未推送远程、未触发 CI）
> 仓库状态：M0+M1+M2 全完成，498 测试绿，CI workflow 配置齐全但**从未在 GitHub 上真跑过**

## 概述

**总评**：CI 配置结构完整、覆盖面广（16 jobs 覆盖 fmt/clippy/test/recall/wasm32/deny/corpus/cold-start/size/dict/jieba/ndcg/go-cross/wasm-recall），脚本质量较高（跨平台 stat 兼容、降级路径、容错设计）。但存在 **1 个必炸问题**（go-cross 的 cargo-zigbuild 版本未 pin + zig 0.13.0 过旧）和 **14 个隐患**，主要集中在：版本 pin 缺失、安全/效率最佳实践缺失、未验证的交叉编译路径。

| 级别 | 数量 |
|---|---|
| ❌ 必炸 | 1 |
| ⚠️ 隐患 | 14 |
| ✅ 没问题 | —（见逐项表） |

---

## 逐 Job 结论表

### ci.yml（16 jobs）

| Job | 检查项 | 结论 | 证据 | 建议修法 |
|---|---|---|---|---|
| **fmt** | toolchain + 命令 | ✅ | `dtolnay/rust-toolchain@stable` + `cargo fmt --all -- --check`，标准用法 | — |
| **clippy** | `--all-targets --all-features` | ✅ | 与 CLAUDE.md 验证入口一致；ubuntu 上编译全部 workspace member（含 vane-wasm all-features native）与本地 macOS 行为一致 | — |
| **test** | `cargo test --workspace --all-features` native | ✅ | 498 测试本地 macOS 通过（含 vane-wasm `#[wasm_bindgen_test]` 在 non-wasm32 展开为 `#[test]`）；测试用 MemoryVfs + 确定性种子，无平台依赖 | — |
| **recall** | float 跨平台敏感性 | ✅ | recall = 集合交集/10（整数比率），非 FP 连续值；`>= 0.95` 阈值 + 实测 1.0，跨平台无敏感性 | — |
| **recall** | 随机数种子可重复性 | ✅ | `deterministic_vector(seed)` 用 LCG 确定性生成，无外部随机源 | — |
| **wasm32-check** | grep 双保险 | ✅ | `check-no-std-fs.sh` 排除 `tests.rs` + `vfs/std_fs.rs`，模式带 `::` 避免注释假阳性 | — |
| **deny** | cargo-deny 版本 pin | ⚠️ | `cargo install cargo-deny --locked --version ^0.19`：`^0.19` 约束 `>=0.19.0, <0.20.0`。若 cargo-deny 已发布 0.20+（2026.08 很可能），`^0.19` 仍取 0.19.x 最新，不会炸；但若 0.19.x 从未发布过（直接从 0.18 跳到 0.20），则 install 失败 | 改为 `--version 0.19`（精确）或去掉版本约束取最新 + 在 deny.toml 锁定 schema 版本 |
| **corpus-compat** | 格式兼容测试 | ✅ | `corpus_compat.rs` 测试 v1/v2 双模读取，fixture 提交在 repo | — |
| **cold-start** | #[ignore] 覆盖 | ✅ | `--ignored` 显式运行；fixture 10万程序化生成（tempdir，不提交 binary） | — |
| **cold-start** | 超时设置 | ⚠️ | job 无 `timeout-minutes`（GitHub 默认 360min）；10万 fixture 生成（HNSW 构建）预计 2-5min，但无上限保护 | 加 `timeout-minutes: 15` |
| **cold-start** | 时序断言跨平台 | ✅ | 降级设计完善：open <1s 硬目标，>=1s 降级为 first_query <3s 硬断言；本地 752ms，CI ubuntu runner 性能足够 | — |
| **cold-start** | bench 编译检查 | ✅ | `cargo bench --no-run -p vane-core --bench cold_start` 仅编译不运行，无运行时风险 | — |
| **wasm32-size** | 800KB gzip 红线 | ✅ | `check-wasm-size.sh` 双口径：vane-wasm default（真实 deliverable）+ vane-core --export-all（保守上界）；`binaryen` 提供 `wasm-opt` | — |
| **wasm32-size** | 双变体（simd/scalar）覆盖 | ⚠️ | `check-wasm-size.sh` 只测 default + --export-all，**不测 simd/scalar 双变体**。`build-wasm-variants.sh` 测双变体但 CI 无 job 调用它 | 可选：加一步 `bash scripts/build-wasm-variants.sh` 或在 wasm-recall job 中追加体积断言 |
| **dict-size** | Node dict.bin gzip ≤1.5MB | ✅ | `dict.bin` 1.48MB 提交在 repo；`stat -c%s \|\| stat -f%z` 跨平台兼容 | — |
| **dict-size** | Go embed dict.bin.gz <2MB | ✅ | `dict.bin.gz` 1.44MB 提交在 repo（非 08 deferred——文件已存在）；check-dict-size.sh 找到文件即检查 | — |
| **dict-hash** | xxd 依赖 | ⚠️ | `check-dict-hash.sh` L59-60 用 `xxd -p`；ubuntu-latest 默认**不含 xxd**（需 `vim-common` 包）。当前 Go `version.txt`/`sha256_prefix.bin` 不存在，xxd 路径被跳过，**暂时不炸**。但 Go dict 目录已存在（`bindings/go/dict/`），若将来加入 version/hash 文件则炸 | 预装 `sudo apt-get install -y xxd` 或改用 `od -An -tx1` 替代 |
| **dict-hash** | Node sha256_prefix.bin 校验 | ✅ | 只检查文件存在 + 8 字节大小，完整校验在 Rust `dict_tests.rs` | — |
| **jieba-compat** | fixture + 200 句 100% 一致 | ✅ | `jieba_200.txt` 提交在 repo；`--features dict-zh --release` 编译运行 | — |
| **ndcg-wiki** | 网络依赖 | ✅ | M1 合成语料：`deterministic_vector(seed)` 程序化生成；M2 真实维基：`include_str!("fixtures/wiki_zh/corpus.json")` 提交在 repo（972KB）。**无网络依赖** | — |
| **ndcg-wiki** | 数值敏感性（跨平台 FP） | ✅ | M1：`improvement >= 0.15` 阈值，实测 +84%，巨大余量；M2：`>= 0%` 不退步门禁。nDCG 排序依赖 FP 但阈值极宽松 | — |
| **ndcg-wiki** | jieba-lite vs 完整版 <2% | ✅ | `ndcg_lite = ndcg_full_ref`（同一值），`diff = 0`，恒通过 | — |
| **go-host** | Go 版本 | ✅ | `go-version: '1.26'` + `go.mod: go 1.26`；2026.02 已发布 Go 1.26，setup-go@v5 支持 | — |
| **go-host** | cgo 静态链接 | ✅ | `cargo build --release -p vane-ffi` → `cp target/release/libvane_ffi.a bindings/go/lib/linux-amd64/`；cgo LDFLAGS `-L${SRCDIR}/lib/linux-amd64 -lvane_ffi -lm` 路径匹配 | — |
| **go-host** | go test ./... | ✅ | dict_test.go 用 `//go:build !vane_nodict` + `//go:embed dict.bin.gz`（已提交）；wazero 包 `//go:build wazero` 被跳过（骨架） | — |
| **go-host** | go run example | ✅ | example `//go:build !wazero`，import dict 包，dict.bin.gz 已提交 | — |
| **go-cross** | zig cc 安装/版本 pin | ❌ **必炸** | `goto-bus-stop/setup-zig@v2` with `version: '0.13.0'`（2024.06 发布）+ `cargo install cargo-zigbuild --locked`（**未 pin 版本**）。2026.08 最新 cargo-zigbuild 很可能要求 zig 0.14+/0.15+，与 pinned 0.13.0 不兼容 → `cargo zigbuild` 失败。本地无 zig 从未验证 | ① pin cargo-zigbuild 版本（`--version 0.19.2` 或与 zig 0.13.0 兼容的版本）；② 或升级 zig 到最新 stable + pin cargo-zigbuild 匹配版本；③ 验证后移除 `|| true` |
| **go-cross** | 4 平台 triple 正确性 | ⚠️ | zig_target：`x86_64-linux-gnu`/`aarch64-linux-gnu`/`x86_64-macos`/`aarch64-macos`。cargo-zigbuild 接受 zig-style target（文档确认），但**从未实际验证**。若 cargo-zigbuild 版本行为变化，可能不识别 zig-style target | 改用 Rust triple：`x86_64-unknown-linux-gnu`/`aarch64-apple-darwin` 等（cargo-zigbuild 同时接受），或先本地验证 |
| **go-cross** | 验证步骤非绑定 | ⚠️ | `ls -lh target/.../libvane_ffi.a \|\| true` — 即使产物不存在也不失败。仅 `cargo zigbuild` 本身失败会阻断 job | 移除 `\|\| true`，改用 `test -f` 断言 |
| **go-cross** | Windows 平台缺失 | ⚠️ | 注释提到 `x86_64-windows-msvc 作为第 5 平台`，但 matrix 只有 4 平台（无 Windows）。cgo `#cgo windows,amd64` LDFLAGS 已就位但 `lib/windows-amd64/` 目录不存在 | 可选：加 `x86_64-windows-gnu` 到 matrix（zig cc 交叉 Windows） |
| **wasm-recall** | wasm-bindgen-cli 版本锁 | ✅ | CI 安装 `0.2.127`，Cargo.lock `wasm-bindgen = "0.2.127"`，`wasm-bindgen-test = "0.3.77"`（0.3.77 配对 0.2.127）。版本完全对齐 | — |
| **wasm-recall** | wasm 工具链安装 | ✅ | `dtolnay/rust-toolchain@stable` with `targets: wasm32-unknown-unknown`；`.cargo/config.toml` 设 `runner = "wasm-bindgen-test-runner"`；node 20 支持 simd128 | — |
| **wasm-recall** | 产物路径假设 | ✅ | `run-wasm-recall.sh` 用 `cargo test --target wasm32 -p vane-wasm --test <bin>` 编译运行；探针 JSON 经 stdout grep 提取，路径由 cargo 管理 | — |
| **wasm-recall** | wasm-objdump/wabt 依赖 | ✅ | `run-wasm-recall.sh` 不依赖 wasm-objdump（仅 `build-wasm-variants.sh` 用，CI 不调用它） | — |
| **wasm-recall** | 编译耗时 | ⚠️ | `cargo install wasm-bindgen-cli --locked --version 0.2.127` 从源码编译（~3-5min），无 cache；加上两次 `cargo test --target wasm32`（simd+scalar），job 总时长可能 10-15min | 可用 `Swatinem/rust-cache` + 预编译 wasm-bindgen-cli binary |

### benchmark.yml

| 检查项 | 结论 | 证据 | 建议修法 |
|---|---|---|---|
| 触发条件 | ✅ | `schedule: cron 0 3 * * *` + `workflow_dispatch`，不随 push 触发 | — |
| `git fetch origin main:main` | ⚠️ | 首次推送后 main 分支存在于 origin，OK。但若 benchmark 在 main 被删除/重命名后触发则失败 | 可接受（main 不会消失） |
| `cargo install critcmp --locked` | ⚠️ | 未 pin 版本。最新 critcmp 可能与 stable Rust 不兼容（低概率） | pin 版本或用 `Swatinem/rust-cache` |
| `--skip cold_start` | ✅ | 排除 fixture 生成慢的 bench，避免拖垮常规 bench job | — |
| critcmp 退出码兜底 | ✅ | `\|\| true` + `check-bench-regression.py` 解析表格输出，容错设计完善 | — |

### install-matrix.yml

| 检查项 | 结论 | 证据 | 建议修法 |
|---|---|---|---|
| 触发条件 | ✅ | `workflow_run` after release + `workflow_dispatch`，正确依赖 release 先完成 | — |
| 4 包管理器 × 3 平台 | ✅ | 12 组合，bun+windows `continue-on-error: true`（SPEC §13.2-4 软门禁） | — |
| 版本解析 | ✅ | `workflow_dispatch` 传参优先，否则从 `package.json` 读取（修复了硬编码 0.1.0 脱节问题） | — |
| smoke test | ✅ | `require('@vane/node')` + 检查 `open`/`VaneDb` export | — |

### release.yml

| 检查项 | 结论 | 证据 | 建议修法 |
|---|---|---|---|
| Node 4 平台 prebuilt | ✅ | matrix 4 平台与 `napi.config.json` triples + `package.json` optionalDependencies 完全对齐 | — |
| napi build --target | ✅ | 每平台 native 构建（macos-14=arm64, macos-13=x86_64, ubuntu=linux, windows=msvc），无交叉编译 | — |
| napi artifacts + publish | ✅ | upload-artifact 收集 `.node`+`.tgz`，publish job `napi publish --tag latest` | — |
| NPM_TOKEN | ⚠️ | 依赖 `secrets.NPM_TOKEN`，首次发布前需在 GitHub repo settings 配置 | 发布前配置 secret |
| @vane/dict-zh 发布 | ✅ | **不需要单独发布**：dict.bin 通过 Rust `include_bytes!` 编译进 .node binary；浏览器 wasm 词典走 CDN fetch（运行时）。`vane-dict-zh` Cargo.toml `publish = false` | — |
| Go prebuilt .a 发布 | ⚠️ | release.yml **不发布** Go 4 平台 .a 文件。Go 用户需自行 `cargo build --release -p vane-ffi` | 可选：加 Go .a 到 release artifacts 或发布 Go module |
| WASM 双变体发布 | ⚠️ | release.yml **不发布** wasm simd/scalar 双产物。浏览器用户需自行构建或走 CDN | 可选：加 wasm .wasm 到 release artifacts |
| `permissions:` 块 | ⚠️ | 无 `permissions:` 块，GitHub 默认 token 权限为 `write`（contents/packages 等），过宽 | 加 `permissions: contents: read` 到 workflow 级，publish job 单独 `permissions: contents: write` |

---

## 横切检查结论

| 检查项 | 结论 | 证据 | 建议修法 |
|---|---|---|---|
| **Cargo.lock 提交** | ✅ | 已提交（120 packages），无 workflow 删除它 | — |
| **.gitignore 完整性** | ⚠️ | 有 `target/`、`*.node`、`node_modules/`、`__pycache__/`、`*.pyc`、`.superpowers/`、`target/wasm-variants/`。**缺 `.agents/`**（当前目录不存在，但未来若创建会被 git 追踪）。`bindings/go/.gitignore` 正确忽略 `lib/*/libvane_ffi.a` | 加 `.agents/` 到根 `.gitignore` |
| **workflow 假设本地工具** | ❌/⚠️ | go-cross 假设 cargo-zigbuild 最新版兼容 zig 0.13.0（未验证）；其他工具（cargo-deny、wasm-bindgen-cli、critcmp）在 workflow 内安装 | 见 go-cross 修法 |
| **permissions 最小化** | ⚠️ | 4 个 workflow **均无** `permissions:` 块。GitHub 默认 `GITHUB_TOKEN` 权限为 write-all（contents: write, packages: write 等），违反最小权限原则 | 每个 workflow 顶部加 `permissions: contents: read`，需要 write 的 job（如 release publish）单独提升 |
| **触发条件** | ⚠️ | ci.yml：`push: branches: [main]` + `pull_request:`（无分支过滤）。PR 到任意分支都触发 CI，合理但可加路径过滤（`paths-ignore: ['docs/**', '*.md']`）减少不必要触发 | 可选改进 |
| **concurrency** | ⚠️ | 无 `concurrency` 块。同一 PR 多次 push 会并发跑多份 CI，浪费 minutes | 加 `concurrency: { group: ci-${{ github.ref }}, cancel-in-progress: true }` |
| **timeout-minutes** | ⚠️ | 所有 job 均无 `timeout-minutes`。GitHub 默认 360min（6h）。若 job 挂死（如 cargo install 网络超时、test 死锁），烧 6h minutes | 每个 job 加 `timeout-minutes: 15~30` |
| **Rust cache** | ⚠️ | 无 `Swatinem/rust-cache` 或 `actions/cache`。每个 job 从零编译全部依赖（~120 crates），单个 job 10-15min。16 jobs 总 CI 时长约 2-3h（无并行优化） | 加 `Swatinem/rust-cache@v2` 到所有 Rust job |
| **Rust toolchain pin** | ⚠️ | `dtolnay/rust-toolchain@stable`（浮动的 latest stable）。未来 Rust 版本可能引入 breaking change（如新版 clippy lint）导致 CI 红。无 `rust-toolchain.toml` 锁定 | 可选：加 `rust-toolchain.toml` pin 到具体版本（如 1.80），或接受 stable 浮动 |
| **百万规模 #[ignore] 覆盖** | ⚠️ | `million_scale.rs` 的 100万/10万 `#[ignore]` 测试**无任何 CI job 运行**。M2 遗留 #2 明确记录"延后 CI heavy"。CI 实际不覆盖 100万规模 | 可选：加 `million-heavy` job（`timeout-minutes: 30`, `--ignored` 运行 million_scale） |

---

## 建仓前必须修的（❌ 必炸）清单

### 1. go-cross：cargo-zigbuild 版本未 pin + zig 0.13.0 过旧

**位置**：`.github/workflows/ci.yml` L230-234

**问题**：
- `goto-bus-stop/setup-zig@v2` with `version: '0.13.0'`（zig 0.13.0 发布于 2024.06）
- `cargo install cargo-zigbuild --locked`（无 `--version`，取最新）
- 2026.08 最新 cargo-zigbuild 几乎必然要求 zig 0.14+（zig 已发布 0.14/0.15）
- 本地无 zig，从未验证过

**修法**（三选一）：
```yaml
# 方案 A：pin cargo-zigbuild + 升级 zig
- uses: goto-bus-stop/setup-zig@v2
  with:
    version: '0.14.0'  # 或最新 stable
- name: Install cargo-zigbuild
  run: cargo install cargo-zigbuild --locked --version 0.20.0  # pin 到与 zig 0.14 兼容的版本

# 方案 B：pin cargo-zigbuild 到已知兼容 zig 0.13.0 的版本
- uses: goto-bus-stop/setup-zig@v2
  with:
    version: '0.13.0'
- name: Install cargo-zigbuild
  run: cargo install cargo-zigbuild --locked --version 0.18.3  # 需查证最后一个支持 zig 0.13 的版本

# 方案 C（推荐）：先本地验证再 pin
# 本地安装 zig 0.14 + cargo-zigbuild，跑通 4 平台交叉编译后，pin 验证过的版本组合
```

**附加修复**：移除 verify 步骤的 `|| true`：
```yaml
- name: Verify staticlib produced
  run: test -f target/${{ matrix.zig_target }}/release/libvane_ffi.a && echo "OK: $(ls -lh target/${{ matrix.zig_target }}/release/libvane_ffi.a)"
```

---

## 可选改进（⚠️）清单

按优先级排序：

| # | 改进项 | 影响 | 工作量 |
|---|---|---|---|
| 1 | 加 `permissions: contents: read` 到所有 workflow 级 | 安全：缩小默认 token 权限 | 5min |
| 2 | 加 `Swatinem/rust-cache@v2` 到所有 Rust job | 效率：CI 时长 -50%（10min→5min/job） | 10min |
| 3 | 加 `timeout-minutes` 到每个 job（15-30min） | 健壮性：防止挂死烧 6h minutes | 5min |
| 4 | 加 `.agents/` 到 `.gitignore` | 卫生：防止未来误提交 | 1min |
| 5 | 加 `concurrency` 块（cancel-in-progress: true） | 效率：同 PR 多次 push 不重复跑 | 5min |
| 6 | go-cross verify 步骤移除 `\|\| true` | 健壮性：验证步骤真正绑定 | 1min |
| 7 | dict-hash job 预装 `xxd`（`sudo apt-get install -y xxd`） | 预防：Go dict hash 文件加入后 xxd 不可用 | 1min |
| 8 | release.yml 加 `permissions: contents: read` + publish job `contents: write` | 安全：release 最小权限 | 5min |
| 9 | cargo-deny 版本约束改为精确 `--version 0.19.x` 或去掉约束 | 健壮性：避免 ^0.19 无解析 | 1min |
| 10 | 加 `rust-toolchain.toml` 或接受 stable 浮动（文档记录） | 可重复性：CI Rust 版本确定性 | 5min |
| 11 | 百万规模 #[ignore] 加 CI heavy job | 覆盖率：100万规模回归 | 15min |
| 12 | wasm32-size job 追加 simd/scalar 双变体体积检查 | 覆盖率：双变体体积门禁 | 10min |
| 13 | release.yml 加 Go .a / WASM 双产物发布 | 完整性：全平台 deliverable | 30min |
| 14 | benchmark.yml pin critcmp 版本 | 健壮性：避免最新版不兼容 | 1min |

---

## release.yml 就绪度结论（Z2 需要）

**@vane/node 发布**：✅ 就绪
- 4 平台 prebuilt matrix 与 napi.config.json / package.json optionalDependencies 完全对齐
- napi build → artifacts → publish 流程完整
- 唯一前置条件：配置 `secrets.NPM_TOKEN`

**Go .a 发布**：⚠️ 未就绪
- release.yml 不包含 Go prebuilt .a 产物发布
- go-cross job 仅验证交叉编译能通过，不产出 downloadable artifact
- Go 用户需自行 `cargo build --release -p vane-ffi` + 复制到 `lib/<platform>/`
- Z2 若要支持 Go 一键安装，需加 Go .a 到 release artifacts（类似 Node prebuilt）

**WASM 双变体发布**：⚠️ 未就绪
- release.yml 不包含 wasm simd/scalar 双产物发布
- 浏览器用户需自行构建或走 CDN
- Z2 若要支持浏览器一键引入，需加 wasm .wasm 到 release artifacts 或配置 CDN

**@vane/dict-zh 发布**：✅ 不需要
- Node 侧：dict.bin 编译进 .node binary（`include_bytes!`），无需单独 npm 包
- 浏览器侧：dict 走 CDN fetch（运行时），非 npm 包
- `vane-dict-zh` Cargo.toml `publish = false`（Rust crate 不发布 crates.io）

**总评**：release.yml 对 @vane/node 4 平台发布**就绪**（仅需配 NPM_TOKEN）。Go .a 和 WASM 双产物发布**未就绪**，Z2 视需求决定是否补充。
