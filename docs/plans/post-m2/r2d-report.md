# R2D 报告：release.yml Node 发布版本不兼容修复

## 状态

完成。仅改动 `release.yml` + `napi.config.json`，未 push。

## 根因

`crates/vane-node/package.json` devDependencies 锁定 `@napi-rs/cli: "^2.18.0"`（**2.x**），scripts 使用 2.x 命令 `napi prepublish -t npm`。但 `.github/workflows/release.yml` 中 `npm install -g @napi-rs/cli`（无版本号 → 解析安装 3.8.5 最新版），导致 3.8.5 与项目 2.x 配置/命令不兼容：

- 3.x `napi artifacts` 报 "unconfigured targets"（3.x 用 `targets`，但配置语义/校验变了）
- 3.x 无 `napi publish` 命令（2.x 才有）
- `triples` → `targets` 字段迁移后仍卡

项目原本是 2.x（M0/Z2-补 均用 `napi publish` + `triples`），R2 重构误升级到 3.8.5 但未同步 package.json devDependencies。修复方向：回退 2.x，与 devDependencies 对齐。

## 改动

### 1. `.github/workflows/release.yml`：pin @napi-rs/cli@^2.18.0

build job + release job 的 `npm install -g @napi-rs/cli` → `npm install -g @napi-rs/cli@^2.18.0`（与 `crates/vane-node/package.json` devDependencies 一致）。

### 2. `crates/vane-node/napi.config.json`：targets → triples（2.x 格式）

`napi.targets: [...]` → `napi.triples: { "defaults": false, "additional": [...] }`，4 个 triple 值不变：
- x86_64-unknown-linux-gnu
- aarch64-apple-darwin
- x86_64-apple-darwin
- x86_64-pc-windows-msvc

### 3. release.yml build job：恢复 2.x per-platform artifacts

- `napi build --target ${{ matrix.target }} --platform --release`（不变）
- 恢复 `napi artifacts --target ${{ matrix.target }}` 步骤（2.x 支持 --target，per-platform 产 .tgz）
- upload-artifact 恢复 `*.node` + `*.tgz`

### 4. release.yml release job：回退 2.x Node 流程

- download 所有 artifacts（node .node/.tgz + go .a + wasm .wasm）
- Stage：`cp -r artifacts/vane-node-* crates/vane-node/artifacts/`（napi publish 扫描 .tgz）
- Node 发布：`napi publish --tag latest`（2.x 一键发布主包 + optionalDeps 平台包，if tag，NPM_TOKEN，working-directory crates/vane-node）
- 移除 3.8.5 的 `napi artifacts --output-dir . --npm-dir npm` / `napi pre-publish -t npm --no-gh-release` / `npm publish --access public` 步骤
- 保留 Z2-补 的 Go .a + WASM .wasm GitHub Release assets（softprops/action-gh-release，if tag）
- workflow_dispatch：publish/gh-release 通过 `if: startsWith(github.ref, 'refs/tags/')` 跳过

### 5. build-go / build-wasm：不变（Z2-补 保留）

## 自证

1. **YAML 合法**：`python3 -c "import yaml; yaml.safe_load(open('.github/workflows/release.yml'))"` → OK
2. **JSON 合法**：`python3 -c "import json; json.load(open('crates/vane-node/napi.config.json'))"` → OK
3. **actionlint**：`actionlint .github/workflows/release.yml` → exit 0（无错误）
4. **CLI pin**：`grep -n '@napi-rs/cli' .github/workflows/release.yml` → 两处均 `@napi-rs/cli@^2.18.0`（line 56 build + line 155 release）
5. **triples 2.x 格式**：`grep -n 'triples\|targets' crates/vane-node/napi.config.json` → `"triples"` + `"defaults": false` + `"additional"`，无 `targets`
6. **逻辑审查**：
   - build job：`napi build --target` + `napi artifacts --target` + upload `.node`/`.tgz` ✓
   - release job：stage `.tgz` → `napi publish --tag latest`（2.x）+ softprops Go/WASM assets ✓
   - build-go / build-wasm 不变 ✓

## SPEC 触及

未触及。`docs/SPEC.md` 不引用 @napi-rs/cli 版本、napi.config.json 字段格式（targets/triples）或 napi 子命令（publish/artifacts/prepublish），故无需同步更新规范。

## 约束遵守

- 只改 release.yml + napi.config.json ✓
- 未改 ci.yml / 代码 / package.json ✓
- 保留 build-go / build-wasm / release 的 Go/WASM assets 逻辑 ✓
- 4 triple 值不变 ✓
- YAML + JSON 合法，actionlint 通过 ✓
- 未 push ✓
