# CI-Gates 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development`。步骤用 checkbox，每步独立可验证。

## Goal

为 Vane M0 建立 GitHub Actions CI/CD 门禁体系，覆盖 SPEC §13.3 工程纪律门禁（wasm32 check / cargo-deny / 依赖黑名单）、§13.2 质量门禁（benchmark 回退 >10% 报警、四包管理器安装矩阵）、§12.2 目标矩阵（4 平台 Node prebuilt 构建 + npm 发布）。所有 workflow 用真实 YAML，无占位符。

## Architecture

- **ci.yml**：主门禁 workflow，PR/push 触发。Job 矩阵：fmt → clippy → test → wasm32-check → cargo-deny，串行依赖以保证失败快速可见。
- **benchmark.yml**：夜间 cron + 手动触发。criterion 跑基准，与 main 分支基准对比，回退 >10% 以非零退出码失败。
- **release.yml**：tag 触发。4 平台 matrix 构建 vane-node napi prebuilt，上传 artifacts，统一发布到 npm。
- **install-matrix.yml**：release 后触发，验证 npm/yarn/pnpm/bun 四包管理器安装可用。
- **deny.toml**：cargo-deny 配置，bans 段列出 SPEC §13.3 黑名单 crate。
- **scripts/check-no-std-fs.sh**：grep 双保险，检测 core 源码出现 std::fs / std::net / mmap。

## Tech Stack

- GitHub Actions（ubuntu-latest / macos-14 / macos-13 / windows-latest）
- Rust toolchain（stable + wasm32-unknown-unknown target）
- `cargo-deny`、`cargo fmt`、`cargo clippy -D warnings`
- `criterion` 基准 + `critcmp` 对比（或 `--save-baseline`）
- `@napi-rs/cli` 的 `napi build` 命令构建 4 平台 prebuilt
- npm 发布（`@vane-rs/node` 包）

## SPEC 引用

- §13.3：`cargo check --target wasm32-unknown-unknown -p vane-core` core 出现 std::fs 即失败（M0 第一天）
- §13.3：cargo-deny + cargo bloat 周报
- §13.3：依赖黑名单 regex / tokio / prost / tonic / openssl / lindera / ndarray / wee_alloc
- §13.3：冻结 corpus 格式兼容测试（M0 占位，M1 落地）
- §13.2：benchmark CI 性能回退 >10% 报警
- §12.2：M0 Node prebuilt 4 个：x86_64-linux-gnu / aarch64-apple-darwin / x86_64-apple-darwin / x86_64-pc-windows-msvc
- §13.2-4：npm/yarn/pnpm/bun 四包管理器安装矩阵通过
- §4.1/§13.3：core crate 禁 std::fs/std::net/mmap，cfg 只在 VFS/Executor
- §14 不变量 I-5：核心零平台分支

## 前置依赖

- **00-workspace**：Cargo workspace 存在（`crates/vane-core`、`crates/vane-node` 等crate 目录与根 `Cargo.toml`），否则 CI 无可跑对象。
- **09-node-binding**：`crates/vane-node` napi-rs 绑定 + `package.json` + `build.rs` 存在，release.yml 才能构建 4 平台 prebuilt。
- 横跨依赖：wasm32 门禁部分（Task 1-3）只需 00-workspace 完成；release.yml（Task 5-6）需 09-node-binding 完成。

## 验收标准

1. PR 推送后 `ci.yml` 全绿：fmt 无变更、clippy 零 warning、`cargo test --workspace` 通过、wasm32 check 通过、`scripts/check-no-std-fs.sh` 退出 0、`cargo deny check` 通过。
2. 在 `crates/vane-core/src/` 任意文件加入 `use std::fs;` 后推送，CI 必须红（wasm32 check 或 grep 脚本失败）。
3. 在 `Cargo.toml` 加入 `regex` 依赖后推送，`cargo deny check bans` 必须失败。
4. `benchmark.yml` 夜间跑通；相对 main 基准任意指标回退 >10% 时 workflow 失败。
5. 打 tag `v0.1.0` 后 `release.yml` 在 4 平台均产出 `.node` prebuilt artifact，npm 发布 `@vane-rs/node@0.1.0` 含 4 个平台 optionalDependencies。
6. `install-matrix.yml` 在 npm/yarn/pnpm/bun 四个包管理器下 `install` + `require('@vane-rs/node')` 成功。
7. 全部 workflow 在仓库 Actions 页面可见，无占位 job。

---

## Global Constraints

| 约束 | 值 | 来源 |
|---|---|---|
| core 禁 `std::fs`/`std::net`/mmap | CI 门禁，M0 第一天 | §6.1/§13.3 |
| `cfg` 只允许在 VFS/Executor 实现 | 核心算法零 cfg | §11/不变量 I-5 |
| 依赖黑名单 | regex / tokio / prost / tonic / openssl / lindera / ndarray / wee_alloc | §4.1/§13.3 |
| wasm32 check 命令 | `cargo check --target wasm32-unknown-unknown -p vane-core` | §13.3 |
| M0 Node prebuilt 平台 | x86_64-linux-gnu / aarch64-apple-darwin / x86_64-apple-darwin / x86_64-pc-windows-msvc | §12.2 |
| benchmark 回退阈值 | >10% 报警（失败） | §13.2 |
| 四包管理器矩阵 | npm / yarn / pnpm / bun | §13.2-4 |
| Rust 工具链 | stable（M0 不强制 nightly） | — |
| workflow 触发分支 | `main` + `pull_request` | — |

**wasm32 check 命令说明**：CI 直接使用 SPEC §13.3 字面命令 `cargo check --target wasm32-unknown-unknown -p vane-core`。01-vfs 计划中 `std_fs.rs` 已通过 `#[cfg(not(target_arch = "wasm32"))]` 隔离 `std::fs`（符合 §11 "cfg 只允许在 VFS/Executor"），core 其余代码不触碰 std::fs，因此 core 默认即可编译到 wasm32，无需 feature 切换。SPEC §12.1 的 `std/wasm` feature 划分留待 M1（jieba feature）落地，M0 不引入。`scripts/check-no-std-fs.sh` 作为 grep 双保险，即使 cfg 隔离被绕过也能拦住裸 `std::fs` 字面。

---

## File Structure

```
vane/
├── .github/
│   └── workflows/
│       ├── ci.yml                 # Task 1-3 产出
│       ├── benchmark.yml          # Task 4 产出
│       ├── release.yml            # Task 5 产出
│       └── install-matrix.yml     # Task 6 产出
├── rustfmt.toml                   # Task 1 产出（如需统一风格）
└── scripts/
    └── check-no-std-fs.sh         # 01-vfs 产出（Task 2 引用）
```

---

## 任务清单（bite-sized TDD）

### Task 1 — ci.yml 基础门禁（fmt + clippy + test）

**Files:**
- `.github/workflows/ci.yml`（新建）
- `rustfmt.toml`（新建）

**Interfaces:**
- Consumes from 00: Cargo workspace（根 `Cargo.toml` + `crates/vane-core` 等）
- Produces: `ci.yml` 的 fmt/clippy/test 三个 job

**SPEC 引用:** §13.3 工程纪律门禁；不变量 I-5。

- [ ] **Step 1**：创建 `rustfmt.toml`（I15 裁决：rustfmt.toml 只写 stable 子集，删除 `imports_granularity`/`group_imports` 等 nightly 选项（M0 锁定 stable toolchain）。若未来切换 nightly，可重新启用这两项。）：
  ```toml
  edition = "2021"
  max_width = 100
  use_field_init_shorthand = true
  use_try_shorthand = true
  ```

- [ ] **Step 2**：创建 `.github/workflows/ci.yml`，定义 `on: [push, pull_request]`，`jobs.ci.runs-on: ubuntu-latest`。第一步 `actions/checkout@v4`，第二步 `dtolnay/rust-toolchain@stable` 安装 stable。

- [ ] **Step 3**：在 ci.yml 增加 `fmt` job：`cargo fmt --all -- --check`。失败即门禁红。

- [ ] **Step 4**：增加 `clippy` job：`cargo clippy --all-targets --all-features -- -D warnings`。零 warning 容忍。

- [ ] **Step 5**：增加 `test` job：`cargo test --workspace --all-features`。依赖 00-09 的单元测试存在；若早期阶段测试尚未齐备，此 job 仍保留（空测试集 cargo test 退出 0）。

- [ ] **Step 6**：设置三个 job 的依赖关系：`clippy needs fmt`、`test needs clippy`（串行链，失败快速停止）。或改为并行 + `needs` 汇总——M0 采用串行以减少 Actions 并发额度占用。

- [ ] **Step 7**：在 ci.yml 增加 `recall` job（I8 裁决：M0 暴力口径 recall 门禁 trivially 满足（hybrid=暴力双路+RRF 基线，recall=1.0）。`recall` job 跑 07-api-core 产出的 `tests/recall.rs`。M1 HNSW 落地后补真实回归 job。）：
  ```yaml
  recall:
    needs: test
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: Run recall gate
        run: cargo test --test recall -p vane-core
  ```

**验证命令:**
```bash
# 本地预跑
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
# 用 act 本地预览 CI（可选）
act -j fmt
```
推送后看 GitHub Actions 页面三个 job 绿灯。

---

### Task 2 — wasm32 check 门禁 + grep 双保险

**Files:**
- `.github/workflows/ci.yml`（追加 `wasm32-check` job）

**Interfaces:**
- Consumes from 00: `crates/vane-core` 带 `wasm` feature
- Consumes from 01-vfs: `scripts/check-no-std-fs.sh`（由 01-vfs 计划创建，单一事实源）
- Produces: wasm32-check job

**SPEC 引用:** §13.3 wasm32 check；§6.1 core 禁 std::fs/std::net/mmap；§14 不变量 I-5 核心零平台分支。

- [ ] **Step 1**：`scripts/check-no-std-fs.sh` 由 **01-vfs** 计划创建（单一事实源，含 `grep -v 'crates/vane-core/src/vfs/std_fs.rs'` 排除）。本计划不重复创建该脚本。CI 直接调用 `bash scripts/check-no-std-fs.sh`。

- [ ] **Step 2**：在 ci.yml 追加 `wasm32-check` job，`needs: test`，`runs-on: ubuntu-latest`。步骤：
  1. `actions/checkout@v4`
  2. `dtolnay/rust-toolchain@stable` with `targets: wasm32-unknown-unknown`
  3. `cargo check --target wasm32-unknown-unknown -p vane-core`
  4. `run: bash scripts/check-no-std-fs.sh`

- [ ] **Step 3**：验证 wasm32 target 已在 toolchain 中安装：`rustup target add wasm32-unknown-unknown`（dtolnay action 的 `targets` 字段会自动处理）。

- [ ] **Step 4**：在本地构造负样本测试：临时在 `crates/vane-core/src/lib.rs` 末尾加 `// use std::fs;`（注释形式，确保 grep 能命中 `std::fs` 字面），运行 `bash scripts/check-no-std-fs.sh` 应退出 1。移除后应退出 0。这是对本脚本 TDD 的负/正样本验证。

- [ ] **Step 5**：验证 wasm32 cargo check 正样本：`cargo check --target wasm32-unknown-unknown -p vane-core` 本地退出 0（前提 00-workspace 已正确隔离 feature）。

**验证命令:**
```bash
rustup target add wasm32-unknown-unknown
cargo check --target wasm32-unknown-unknown -p vane-core
bash scripts/check-no-std-fs.sh
# 负样本
echo 'use std::fs;' >> crates/vane-core/src/lib.rs
bash scripts/check-no-std-fs.sh; echo "exit=$?"   # 期望 1
git checkout crates/vane-core/src/lib.rs
```
推送后 GitHub Actions `wasm32-check` job 绿灯。

---

### Task 3 — cargo-deny 依赖审查

**Files:**
- `.github/workflows/ci.yml`（追加 `deny` job）

**Interfaces:**
- Consumes from 00: workspace `Cargo.lock` + `deny.toml`（由 00-workspace 计划创建，单一事实源）
- Produces: cargo-deny CI job

**SPEC 引用:** §13.3 cargo-deny + 依赖黑名单；§4.1 WASM 依赖黑名单。

- [ ] **Step 1**：`deny.toml` 由 **00-workspace** 计划创建（单一事实源）。本计划不重复创建。如需补 licenses.allow 增量，在 00-workspace 维护。CI 的 `deny` job 直接使用 00 产出的 deny.toml。

- [ ] **Step 2**：在 ci.yml 追加 `deny` job，`needs: wasm32-check`，`runs-on: ubuntu-latest`。步骤：
  1. `actions/checkout@v4`
  2. `dtolnay/rust-toolchain@stable`
  3. `cargo install cargo-deny --locked --version ^0.16`
  4. `cargo deny check --workspace`（同时跑 advisories / bans / licenses / sources 四类）

- [ ] **Step 3**：本地安装并预跑：
  ```bash
  cargo install cargo-deny --locked
  cargo deny check --workspace
  ```
  确保 Cargo.lock 中无黑名单 crate（M0 依赖应只有 `roaring`、`serde`、`serde_json`、`sha2`、`napi`、`criterion` 等白名单内 crate）。

- [ ] **Step 4**：负样本验证：临时在 `crates/vane-core/Cargo.toml` 加 `regex = "1"`，运行 `cargo deny check bans`，期望报错 `ban: regex`。移除后恢复通过。这是 cargo-deny 门禁的 TDD 验证。

- [ ] **Step 5**：在 ci.yml 末尾记录"门禁汇总"步骤（可选）：用 `if: always()` 聚合 fmt/clippy/test/wasm32-check/deny 五个 job 状态，便于 PR 检查页一眼可见。

**验证命令:**
```bash
cargo install cargo-deny --locked
cargo deny check --workspace
# 负样本
sed -i 's/\]/]\nregex = "1"/' crates/vane-core/Cargo.toml
cargo deny check bans; echo "exit=$?"   # 期望非 0
git checkout crates/vane-core/Cargo.toml
```
推送后 GitHub Actions `deny` job 绿灯。

---

### Task 4 — benchmark.yml 性能回退门禁

**Files:**
- `.github/workflows/benchmark.yml`（新建）

**Interfaces:**
- Consumes from 06/05/03: criterion 基准测试目标（`benches/` 目录）
- Produces: benchmark CI workflow，回退 >10% 失败

**SPEC 引用:** §13.2 质量门禁-未列条目（benchmark CI 性能回退 >10% 报警，见 §13.3）；§13.1 性能承诺。

- [ ] **Step 1**：创建 `.github/workflows/benchmark.yml`：
  ```yaml
  name: benchmark
  on:
    schedule:
      - cron: '0 3 * * *'   # 每日 UTC 03:00（北京时间 11:00）
    workflow_dispatch:       # 支持手动触发
  jobs:
    bench:
      runs-on: ubuntu-latest
      steps:
        - uses: actions/checkout@v4
          with:
            fetch-depth: 0   # 需要历史以对比 main
        - uses: dtolnay/rust-toolchain@stable
        - name: Install critcmp
          run: cargo install critcmp --locked
        - name: Run benchmark on current branch
          run: cargo bench --workspace -- --save-baseline current
        - name: Checkout main baseline
          run: |
            git fetch origin main:main
            git worktree add ../vane-main main
        - name: Run benchmark on main
          working-directory: ../vane-main
          run: cargo bench --workspace -- --save-baseline main
        - name: Compare and fail on >10% regression
          run: |
            # critcmp 输出表格；用脚本解析回退比例
            critcmp main current > compare.txt 2>&1 || true
            cat compare.txt
            # 解析每个基准：若 current 相对 main 回退 >10% 则失败
            python3 scripts/check-bench-regression.py compare.txt 0.10
  ```

- [ ] **Step 2**：创建 `scripts/check-bench-regression.py`（I14 裁决：提供完整可执行 Python 脚本（解析 critcmp 输出 / 回退 >10% 以非零退出码报警）。critcmp 输出格式可能变化，脚本容错：解析失败时 warn 并 exit 0（不阻断））：
  ```python
  #!/usr/bin/env python3
  """check-bench-regression.py — 解析 critcmp 输出，回退 > 阈值则 exit 1。
  SPEC §13.2: benchmark CI 性能回退 >10% 报警。
  """
  import sys
  import re

  def parse_critcmp(text):
      """解析 critcmp 表格输出，返回 [(name, main_ms, current_ms), ...]"""
      results = []
      lines = text.strip().split('\n')
      for line in lines:
          # critcmp 行格式：bench_name  main     X ms   current  Y ms
          # 或：bench_name  main: X ms  current: Y ms
          # 尝试匹配 "main" 和 "current" 列的时间值
          m = re.search(r'([\d.]+)\s*(ms|µs|ns|s)\s+.*current.*?([\d.]+)\s*(ms|µs|ns|s)', line, re.IGNORECASE)
          if m:
              main_val = float(m.group(1))
              main_unit = m.group(2).lower()
              curr_val = float(m.group(3))
              curr_unit = m.group(4).lower()
              # 归一化到 ms
              unit_factor = {'s': 1000, 'ms': 1, 'µs': 0.001, 'us': 0.001, 'ns': 0.000001}
              main_ms = main_val * unit_factor.get(main_unit, 1)
              curr_ms = curr_val * unit_factor.get(curr_unit, 1)
              name = line.split()[0] if line.split() else "unknown"
              results.append((name, main_ms, curr_ms))
      return results

  def main():
      if len(sys.argv) < 3:
          print("Usage: check-bench-regression.py <compare.txt> <threshold>", file=sys.stderr)
          sys.exit(2)
      filepath = sys.argv[1]
      threshold = float(sys.argv[2])
      
      try:
          with open(filepath) as f:
              text = f.read()
      except IOError as e:
          print(f"WARN: cannot read {filepath}: {e}", file=sys.stderr)
          sys.exit(0)  # 容错：解析失败不阻断
      
      results = parse_critcmp(text)
      if not results:
          print("WARN: no benchmark results parsed, skipping regression check", file=sys.stderr)
          sys.exit(0)
      
      failures = []
      for name, main_ms, curr_ms in results:
          if main_ms > 0:
              regression = (curr_ms - main_ms) / main_ms
              if regression > threshold:
                  failures.append((name, main_ms, curr_ms, regression))
      
      if failures:
          print(f"FAIL: {len(failures)} benchmark(s) regressed > {threshold*100:.0f}%:", file=sys.stderr)
          for name, main_ms, curr_ms, reg in failures:
              print(f"  {name}: main={main_ms:.4f}ms current={curr_ms:.4f}ms regression={reg*100:.1f}%", file=sys.stderr)
          sys.exit(1)
      else:
          print(f"OK: no benchmark regressed > {threshold*100:.0f}%")
          sys.exit(0)

  if __name__ == '__main__':
      main()
  ```

- [ ] **Step 3**：若 critcmp 解析复杂度超 M0 预算，备选方案用 criterion 原生 `--save-baseline` + `cargo bench --bench xxx -- --baseline main --threshold 10`。criterion 自带 `PercentageChange` 判定，超过阈值会以非零退出码失败。备选 workflow 片段：
  ```yaml
  - name: Bench against main baseline
    run: |
      # 先在 main 上建立 baseline（artifact 缓存）
      cargo bench --workspace -- --save-baseline main
      # 切回当前分支，对比
      git checkout -
      cargo bench --workspace -- --baseline main
  ```
  > criterion 默认在变化 >10% 时打印 `[regressed]`，但不自动非零退出。需配合 `--threshold` 或解析输出。M0 采用 Python 解析脚本方案（Step 2）以保证确定性。

- [ ] **Step 4**：本地预跑（需有 benches 目标，依赖 06-vector-brute / 05-bm25 的 criterion bench 存在）：
  ```bash
  cargo bench --workspace -- --save-baseline current
  critcmp main current || true
  ```
  若 M0 早期 benches 尚未齐备，benchmark.yml 仍可推送但 job 会因 `cargo bench` 无目标而 exit 0（criterion 无 bench 时无操作）。Task 4 验收以"workflow 语法正确、能在 Actions 页面成功运行（即使无 bench 目标也不报错）"为准。

- [ ] **Step 5**：构造负样本：手动改某 bench 让其 sleep 20%（模拟回退），触发 workflow_dispatch，期望 `check-bench-regression.py` exit 1、job 红。恢复后绿。

**验证命令:**
```bash
# 语法校验
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/benchmark.yml'))"
# 本地 bench 预跑（需 benches 存在）
cargo bench --workspace -- --save-baseline current
# 手动触发 CI
gh workflow run benchmark.yml
```

---

### Task 5 — release.yml 4 平台 Node prebuilt 构建

**Files:**
- `.github/workflows/release.yml`（新建）

**Interfaces:**
- Consumes from 09: `crates/vane-node` napi-rs 绑定 + `package.json` + `build.rs`
- Produces: tag 触发的 4 平台 matrix 构建 + npm 发布 workflow

**SPEC 引用:** §12.2 M0 Node prebuilt 4 平台；§12.4 版本与发布；§13.2-4 四包管理器安装矩阵（Task 6 消费本 Task 产物）。

- [ ] **Step 1**：创建 `.github/workflows/release.yml`：
  ```yaml
  name: release
  on:
    push:
      tags:
        - 'v*'    # 仅 tag 触发
    workflow_dispatch:
      inputs:
        version:
          description: '版本号（如 0.1.0）'
          required: true
  jobs:
    build:
      strategy:
        fail-fast: false
        matrix:
          include:
            # SPEC §12.2 M0 Node prebuilt 4 平台
            - os: ubuntu-latest
              target: x86_64-unknown-linux-gnu
              node_platform: linux-x64-gnu
            - os: macos-14
              target: aarch64-apple-darwin
              node_platform: darwin-arm64
            - os: macos-13
              target: x86_64-apple-darwin
              node_platform: darwin-x64
            - os: windows-latest
              target: x86_64-pc-windows-msvc
              node_platform: win32-x64-msvc
      runs-on: ${{ matrix.os }}
      steps:
        - uses: actions/checkout@v4
        - uses: dtolnay/rust-toolchain@stable
          with:
            targets: ${{ matrix.target }}
        - uses: actions/setup-node@v4
          with:
            node-version: '20'
            registry-url: 'https://registry.npmjs.org'
        - name: Install @napi-rs/cli
          run: npm install -g @napi-rs/cli
        - name: Build napi prebuilt
          working-directory: crates/vane-node
          run: napi build --target ${{ matrix.target }} --platform --release
        - name: Package artifact
          working-directory: crates/vane-node
          run: napi artifacts --target ${{ matrix.target }}
        - uses: actions/upload-artifact@v4
          with:
            name: vane-node-${{ matrix.node_platform }}
            path: |
              crates/vane-node/*.node
              crates/vane-node/*.tgz

    publish:
      needs: build
      runs-on: ubuntu-latest
      steps:
        - uses: actions/checkout@v4
        - uses: actions/setup-node@v4
          with:
            node-version: '20'
            registry-url: 'https://registry.npmjs.org'
        - uses: actions/download-artifacts@v4
          with:
            path: crates/vane-node/artifacts
        - name: Publish to npm
          working-directory: crates/vane-node
          run: napi publish --tag latest
          env:
            NODE_AUTH_TOKEN: ${{ secrets.NPM_TOKEN }}
  ```
  > 注：`napi build --platform` 会自动按 target 生成 `vane-node.<platform>.node` 文件名。`napi artifacts` 收集 prebuilt 供 `napi publish` 统一发布为 optionalDependencies。

- [ ] **Step 2**：确认 `crates/vane-node/package.json` 已配置 napi-rs 标准字段（`napi.name`、`napi.triples`、`optionalDependencies` 占位）。若 09-node-binding 未配置 triples，本 Task 在 `package.json` 的 `napi` 段补齐 4 平台：
  ```json
  {
    "napi": {
      "name": "vane-node",
      "triples": {
        "defaults": false,
        "additional": [
          "x86_64-unknown-linux-gnu",
          "aarch64-apple-darwin",
          "x86_64-apple-darwin",
          "x86_64-pc-windows-msvc"
        ]
      }
    }
  }
  ```

- [ ] **Step 3**：在 GitHub 仓库 Settings → Secrets 添加 `NPM_TOKEN`（npm automation token）。此步为人工配置，计划中标注为前置条件。

- [ ] **Step 4**：本地验证 napi build 命令在当前平台跑通：
  ```bash
  cd crates/vane-node
  npm install
  napi build --release
  ls *.node   # 应看到当前平台的 .node 文件
  node -e "require('./').open ? console.log('ok') : console.log('loaded')"
  ```

- [ ] **Step 5**：打测试 tag `v0.0.0-rc.1` 触发 release.yml，观察 4 平台 build job 全绿、publish job 成功发布到 npm（pre-release tag 不污染 latest）。验证后在 npmjs.com 确认 `@vane-rs/node@0.0.0-rc.1` 含 4 个平台 optionalDependencies 包。

- [ ] **Step 6**：若某平台 build 失败，按错误信息排查（常见：windows MSVC 缺少 `link.exe`、linux 缺少 `gcc`，napi-rs action 通常已预装）。`fail-fast: false` 保证 4 平台独立失败可见。

**验证命令:**
```bash
# YAML 语法
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/release.yml'))"
# 本地 napi build
cd crates/vane-node && napi build --release
# 触发 release（需 tag）
git tag v0.0.0-rc.1 && git push origin v0.0.0-rc.1
gh run watch
```

---

### Task 6 — install-matrix.yml 四包管理器安装矩阵

**Files:**
- `.github/workflows/install-matrix.yml`（新建）

**Interfaces:**
- Consumes from Task 5: npm 上已发布的 `@vane-rs/node` 版本
- Produces: npm/yarn/pnpm/bun 安装验证 workflow

**SPEC 引用:** §13.2-4 平台四包管理器（npm/yarn/pnpm/bun）安装矩阵通过。

- [ ] **Step 1**：创建 `.github/workflows/install-matrix.yml`：
  ```yaml
  name: install-matrix
  on:
    workflow_run:
      workflows: ["release"]
      types: [completed]
    workflow_dispatch:
      inputs:
        version:
          description: '@vane-rs/node 版本号'
          required: true
          default: '0.0.0-rc.1'
  jobs:
    install:
      strategy:
        fail-fast: false
        matrix:
          pkg_manager: [npm, yarn, pnpm, bun]
          os: [ubuntu-latest, macos-14, windows-latest]
      runs-on: ${{ matrix.os }}
      steps:
        - uses: actions/checkout@v4
        - uses: actions/setup-node@v4
          with:
            node-version: '20'
        - name: Setup pkg manager
          run: |
            case "${{ matrix.pkg_manager }}" in
              npm)  npm --version ;;
              yarn) corepack enable; yarn --version ;;
              pnpm) npm install -g pnpm; pnpm --version ;;
              bun)  curl -fsSL https://bun.sh/install | bash; echo "$HOME/.bun/bin" >> $GITHUB_PATH; bun --version ;;
            esac
        - name: Init test project
          run: |
            mkdir test-install && cd test-install
            case "${{ matrix.pkg_manager }}" in
              npm)  npm init -y; npm install @vane-rs/node@${{ github.event.inputs.version || '0.0.0-rc.1' }} ;;
              yarn) yarn init -y; yarn add @vane-rs/node@${{ github.event.inputs.version || '0.0.0-rc.1' }} ;;
              pnpm) pnpm init; pnpm add @vane-rs/node@${{ github.event.inputs.version || '0.0.0-rc.1' }} ;;
              bun)  bun init -y; bun add @vane-rs/node@${{ github.event.inputs.version || '0.0.0-rc.1' }} ;;
            esac
        - name: Require and smoke test
          run: |
            cd test-install
            node -e "const vane = require('@vane-rs/node'); console.log('loaded:', typeof vane); if (typeof vane.open !== 'function' && typeof vane.VaneDb === 'undefined') { console.error('FAIL: no open/VaneDb export'); process.exit(1); } console.log('OK')"
      # 注：windows 上 bun 可能不支持，matrix 仍跑但允许失败标记为继续
      continue-on-error: ${{ matrix.os == 'windows-latest' && matrix.pkg_manager == 'bun' }}
  ```
  > 注：bun 在 windows 上的支持仍在演进，`continue-on-error` 标注该组合为软门禁（报警但不阻断）。SPEC §13.2-4 要求"四包管理器安装矩阵通过"，主体验证 npm/yarn/pnpm 三者 × 三平台 + bun × (linux+macos) 共 11 个组合硬通过；bun × windows 软通过。

- [ ] **Step 2**：在 release.yml publish 成功后自动触发 install-matrix（已通过 `workflow_run` 配置）。也可手动 `workflow_dispatch` 指定版本号触发。

- [ ] **Step 3**：本地预验证（以 npm 为例）：
  ```bash
  mkdir /tmp/test-install && cd /tmp/test-install
  npm init -y
  npm install @vane-rs/node@0.0.0-rc.1
  node -e "const v = require('@vane-rs/node'); console.log(typeof v)"
  ```
  确认 napi prebuilt 在当前平台正确加载。

- [ ] **Step 4**：触发 install-matrix workflow_dispatch（version 指向 Task 5 发布的 rc 版本），观察矩阵全绿（windows+bun 软门禁除外）。

- [ ] **Step 5**：若某组合失败，排查常见原因：① prebuilt 缺该平台（回 Task 5 检查 triples）；② 包管理器锁文件冲突（test 项目无锁文件即可）；③ node ABI 版本不匹配（确认 setup-node 版本与 napi build 一致）。

**验证命令:**
```bash
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/install-matrix.yml'))"
gh workflow run install-matrix.yml -f version=0.0.0-rc.1
gh run watch
```

---

## 完成判据（Task 1-6 全部满足）

1. 推送任意 PR → `ci.yml` 五个 job（fmt/clippy/test/wasm32-check/deny）全绿。
2. 在 core 引入 `std::fs` 或黑名单依赖 → CI 必红（wasm32-check 或 deny job 失败）。
3. 夜间 `benchmark.yml` 自动运行；回退 >10% → job 失败。
4. 打 tag → `release.yml` 4 平台构建 + npm 发布成功。
5. release 后 `install-matrix.yml` 11 个硬组合全绿（windows+bun 软通过）。
6. 仓库 `.github/workflows/` 含 4 个 workflow 文件 + `rustfmt.toml`（引用 00-workspace 产出 deny.toml）+ `scripts/check-no-std-fs.sh`（引用 01-vfs 产出）+ `scripts/check-bench-regression.py`，无占位内容。

## 备注：冻结 corpus 格式兼容测试（M0 占位）

SPEC §13.3 要求"旧版本写出的库必须被新版本打开"。M0 阶段格式仍在成型，此门禁以占位 README 标注，M1 落地具体 corpus fixture 与 `corpus-compat` CI job。本计划不为 M0 产出该 job，但预留 ci.yml 中注释位：
```yaml
  # corpus-compat:  # SPEC §13.3 — M1 落地
  #   needs: test
  #   run: cargo test --test corpus_compat
```

## 备注：cargo bloat 周报

SPEC §13.3 提及 `cargo bloat` 周报。M0 不作为硬门禁（wasm 产物在 M2 才交付），仅作为可观测项。可在 benchmark.yml 追加 `cargo bloat --release --crates` 步骤上传 artifact 供人工查阅。M0 计划暂不产出，M2 前补齐。
