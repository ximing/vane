# Z2-补：release.yml 三端 prebuilt 发版补全

**状态**：完成（未 push，待编排者统一 commit+push）
**改动范围**：仅 `.github/workflows/release.yml`
**触及 SPEC**：否（SPEC §12.2 已定义三端 prebuilt，本次是实现补全，不改规范）

## 1. 设计：3 job 结构

原 release.yml 只有 `build`（4 平台 napi matrix）+ `publish`（npm 发布）。补全后为 4 job：

| job | runner | 作用 | 产物 |
|---|---|---|---|
| `build` | matrix（ubuntu/macos-14/macos-15-intel/windows） | napi-rs 4 平台 prebuilt | `vane-node-<platform>` artifact（.node + .tgz） |
| `build-go` | ubuntu-latest（zig 交叉） | vane-ffi staticlib 4 平台 | `vane-go-<lib_dir>` artifact（libvane_ffi-<lib_dir>.a） |
| `build-wasm` | ubuntu-latest | wasm 双变体 simd/scalar | `vane-wasm` artifact（2 个 .wasm） |
| `release` | ubuntu-latest | 聚合三端：npm publish + GitHub Release assets | npm 包 + Release .a/.wasm |

依赖：`release` `needs: [build, build-go, build-wasm]`。三 build job 并行，release 聚合。

## 2. build-go 实现摘要

完全复用 ci.yml `go-cross` job 的已验证配置：

- **matrix 4 平台**：`target`（Rust triple）+ `lib_dir`（bindings/go/lib 子目录名）
  - x86_64-unknown-linux-gnu → linux-amd64
  - aarch64-unknown-linux-gnu → linux-arm64
  - x86_64-apple-darwin → darwin-amd64
  - aarch64-apple-darwin → darwin-arm64
- **steps**：checkout → rust-toolchain(stable, targets: matrix.target) → rust-cache → `goto-bus-stop/setup-zig@v2` (0.15.2) → `cargo install cargo-zigbuild --locked --version 0.23.0` → `cargo zigbuild --release -p vane-ffi --target matrix.target` → verify `test -f` → 重命名 `libvane_ffi.a` 为 `libvane_ffi-<lib_dir>.a` → upload-artifact `vane-go-<lib_dir>`
- **关键不变量**：zig 0.15.2 + cargo-zigbuild 0.23.0 + Rust triple（非 zig-style），与 ci.yml go-cross 完全一致；`--target` 传 Rust triple，产物目录 `target/<triple>/release/`。

## 3. build-wasm 实现摘要

- **steps**：checkout → rust-toolchain(stable, targets: wasm32-unknown-unknown) → rust-cache → `sudo apt-get install -y binaryen`（wasm-opt）→ `FEATURES=worker bash scripts/build-wasm-variants.sh` → upload-artifact `vane-wasm`（path: `target/wasm-variants/vane_wasm_simd.wasm` + `vane_wasm_scalar.wasm`）
- 复用现有构建脚本（含 wasm-opt -Oz 优化 + gzip ≤800KB 体积门禁 + 特征校验），无重复逻辑。

## 4. release job 实现摘要

- **permissions**：job 级 `contents: write`（创建 Release + upload assets）；workflow 级保持 `contents: read`（最小化）。
- **download**：`actions/download-artifact@v4` 下载全部 artifacts 到 `artifacts/`（每个 artifact 一个子目录）。
- **npm publish**：将 `artifacts/vane-node-*` 拷贝到 `crates/vane-node/artifacts/`（恢复原 publish job 的目录结构，napi publish 从 `crates/vane-node` 工作目录扫描 *.tgz）；`napi publish --tag latest`，`NODE_AUTH_TOKEN: secrets.NPM_TOKEN`。
- **GitHub Release**：`softprops/action-gh-release@v2`，`files:` glob 上传 `artifacts/vane-go-*/libvane_ffi-*.a`（4 个）+ `artifacts/vane-wasm/*.wasm`（2 个）。
- **触发门控**：npm publish 与 Release 创建均 `if: startsWith(github.ref, 'refs/tags/')`——workflow_dispatch 仅构建验证，不发布不建 Release。
- 修复原 `publish` job 的潜在缺陷：补 `npm install -g @napi-rs/cli`（原 publish job 缺此步，`napi` 命令不可用）。

## 5. macOS runner 修复（Z1 标注 pre-existing）

- **问题**：`build` matrix 的 `macos-13`（x86_64-apple-darwin）已于 **2025-12-04/08 被 GitHub 正式退役**，CI 会失败。
- **查证**（WebSearch, 2026.08）：GitHub 于 2025-09-19 上线 `macos-15-intel` 作为 Intel x86_64 macOS 镜像的替代，退役计划 2027 秋（最后一个 x86_64 镜像）。
- **修改**：`macos-13` → `macos-15-intel`（仅 build job matrix 一处，x86_64-apple-darwin 行）。
- 来源：
  - [actions/runner-images#13045 — macOS 15 Sonoma Intel-based image](https://github.com/actions/runner-images/issues/13045)
  - [nextstrain/.github#150 — macOS 13 runner image deprecation/removal](https://github.com/nextstrain/.github/issues/150)

## 6. 校验

- **YAML 语法**：`python3 -c "import yaml; yaml.safe_load(open('.github/workflows/release.yml'))"` → **通过**
- **actionlint**：`actionlint -color .github/workflows/release.yml` → **exit 0，无 error/warning**（actionlint 1.7.12，brew 安装）

## 7. 遗留

- **NPM_TOKEN**：`secrets.NPM_TOKEN` 需在 GitHub repo Secrets 配置（@vane-rs/node npm 发布权限）。未配置时 npm publish step 会失败（tag 触发时）；build/build-go/build-wasm 不受影响。
- **workflow_dispatch 版本号**：`inputs.version` 当前未被任何 step 使用（原状），仅作为触发参数；如需用版本号打 tag/写 Release name，后续可补。
- **Go .a Windows 平台**：当前 4 平台不含 windows-msvc（与 ci.yml go-cross 一致；Windows cgo 链接 vane-ffi 另有方案，非本次范围）。
- **未 push**：按指令不 push，待编排者 P1 CI 绿后统一 commit+push。
