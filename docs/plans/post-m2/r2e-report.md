# R2E 报告：release.yml Node 发布按 napi-rs 官方流程重写

## 状态

完成。仅改动 `release.yml` + `crates/vane-node/package.json` + `crates/vane-node/napi.config.json`，未 push。

## 官方发布流程（napi.rs/docs/deep-dive/release）

1. **build job（per-platform matrix）**：`napi build --target <triple> --platform --release` → upload `.node`（**不跑 `napi artifacts`**）
2. **release job**：download all `.node` → 暂存到 `crates/vane-node/` → `napi create-npm-dir`（为每个 target 创建 `npm/<platform>/` 平台包目录 + package.json）→ `napi artifacts`（收集 `.node` 到 `npm/<platform>/` 与包根）→ `npm publish`（主包，其 `prepublishOnly` 脚本 `napi prepublish -t npm` 自动发布平台包 + 上传 `.node` 到 GitHub Release）

## 改动

### 1. `crates/vane-node/package.json`：napi 字段加 targets + triples

`napi` 对象新增 `targets` 数组（4 triple）与 `triples` 对象（`{defaults:false, additional:[4 triple]}`），保留既有 `binaryName`/`packageName`：

```json
"napi": {
  "binaryName": "vane",
  "packageName": "@vane-rs/node",
  "targets": [4 triple],
  "triples": { "defaults": false, "additional": [4 triple] }
}
```

4 triple：`x86_64-unknown-linux-gnu`、`aarch64-apple-darwin`、`x86_64-apple-darwin`、`x86_64-pc-windows-msvc`（与原 napi.config.json additional 一致，值未变）。

### 2. `crates/vane-node/napi.config.json`：napi 字段同步 targets + triples

napi.config.json 在 2.x 下不被任何命令自动加载（无 `-c` 传入，grep 确认仅 release.yml 注释引用），属文档性配置。为避免与 package.json 漂移，将其 napi 字段对齐为与 package.json 完全一致（`binaryName`/`packageName`/`targets`/`triples`），去掉旧的非标准顶层 `binaryName`/`packageName`/`packageVersion` 与 `napi.name`。

### 3. `.github/workflows/release.yml`：Node 流程按官方重写

**build job**：
- `napi build --target ${{ matrix.target }} --platform --release`（不变）
- **删除** `napi artifacts --target` 步骤（2.x/3.x 均不支持 `--target`，R2d 误加）
- upload-artifact 只上传 `crates/vane-node/*.node`（去掉 `*.tgz`）
- pin `@napi-rs/cli@^2.18.0`（保留 R2d）

**release job**：
- download all artifacts（node `.node` + go `.a` + wasm `.wasm`）
- `cp artifacts/vane-node-*/*.node crates/vane-node/`（暂存 4 个 `.node` 到 napi artifacts 扫描位置）
- `napi create-npm-dir -t .`（working-directory `crates/vane-node`，创建 `npm/<platform>/` 平台包目录 + package.json）—— **必须在 artifacts 之前**（artifacts 只写文件不建目录）
- `napi artifacts --dir .`（working-directory `crates/vane-node`，收集 `.node` 到 `npm/<platform>/` 与包根）
- `npm publish`（if tag，working-directory `crates/vane-node`，env `NODE_AUTH_TOKEN` + `GITHUB_TOKEN`）—— 触发 `prepublishOnly: napi prepublish -t npm`，发布 4 个 optionalDeps 平台包 + 上传 `.node` 到 GitHub Release
- softprops/action-gh-release（if tag，保留）：挂 Go `.a` + WASM `.wasm` 到同一 Release
- workflow_dispatch：`npm publish`/gh-release 通过 `if: startsWith(github.ref, 'refs/tags/')` 跳过；`create-npm-dir` + `artifacts` 可跑（验证收集）

**build-go / build-wasm**：不变。

## 2.x 命令查证（源码 + 实测）

通过 `gh api` 拉取 `@napi-rs/cli@2.18.0`（与 `@napi-rs/cli@2.18.4`，即 `^2.18.0` 解析上限）源码逐项确认，并本地安装实测：

| 项 | 任务/传闻 | 实际（2.x 源码 + 实测） | 处理 |
|---|---|---|---|
| `napi artifacts` 选项 | `[-d,--dir] [--dist] [-c,--config]` | ✅ 一致（`cli/src/artifacts.ts`：`Option.String('-d,--dir','artifacts')` + `--dist` 默认 `npm` + `-c,--config`） | 用 `--dir .` |
| `napi artifacts --target` | 2.x 支持（R2d） | ❌ **不支持**，2.x 选项无 `--target` | 删除该步 |
| `napi publish` | 2.x 有（R2d） | ❌ **2.x 无此命令**，`cli/src/index.ts` 注册：artifacts/build/create-npm-dir/prepublish/version/universal/new/rename | 用 `npm publish` |
| `napi create-npm-dirs`（复数） | 任务写法 | ❌ 2.x 是 `create-npm-dir`（**单数**），`cli/src/create-npm-dir.ts` `static paths = [['create-npm-dir']]` | 用单数 |
| `napi prepublish -t npm` | prepublishOnly | ✅ `cli/src/pre-publish.ts` `static paths = [['prepublish']]`，`-t` = `--tagstyle`，`npm` 用 `v${version}` tag | 保留 package.json prepublishOnly |
| targets 读取 | 2.x 读 `napi.targets` | ❌ **2.x 读 `napi.triples`**，`cli/src/consts.ts:getNapiConfig` 读 `napi?.triples?.additional` + `napi?.triples?.defaults`；**忽略 `napi.targets`** | 保留 triples |
| binaryName 读取 | 2.x 读 `napi.binaryName` | ❌ 2.x 读 `napi?.name ?? 'index'`；忽略 `napi.binaryName` | 见下 |

**targets 实测**（本地 `@napi-rs/cli@2.18.0` + `2.18.4`，`napi create-npm-dir -t .`）：
- 仅 `targets`（无 `triples`）→ 创建 `darwin-x64`/`linux-x64-gnu`/`win32-x64-msvc`（DefaultPlatforms），**漏 darwin-arm64**（aarch64 不在 defaults）
- `triples: {defaults:false, additional:[4]}` → 创建 `darwin-arm64`/`darwin-x64`/`linux-x64-gnu`/`win32-x64-msvc`（正好 4 个）

**端到端实测**（新 package.json 配置 + 4 个 `index.<platform>.node` 假文件）：
- `napi create-npm-dir -t .` → 正好 4 个 `npm/<platform>/`（含 darwin-arm64，无多余）
- `napi artifacts --dir .` → 4 个 `.node` 拷入 `npm/<platform>/` + 包根
- 平台包 `npm/linux-x64-gnu/package.json`：`"name":"@vane-rs/node-linux-x64-gnu"`、`"main":"index.linux-x64-gnu.node"`、`"libc":["glibc"]` ✅

## 关键偏差：保留 triples（任务要求"去掉 triples"的修正）

任务指令"napi.config.json 的 napi 字段也加 targets 数组（与 package.json 一致），去掉 triples"。**本实现保留 `triples` 并同时加 `targets`**，理由（源码 + 实测证据）：

- 2.x `getNapiConfig`（`cli/src/consts.ts`）只读 `napi.triples`，**完全不识别 `napi.targets`**。去掉 `triples` 后 `triples.defaults` 为 `undefined`（≠ `false`）→ 启用 DefaultPlatforms → `napi artifacts`/`create-npm-dir` 会为默认全平台建目录，**漏掉 darwin-arm64**（aarch64-apple-darwin 不在 DefaultPlatforms），发版即崩。
- `targets` 是 3.x 字段（`cli/src/utils/config.ts:UserNapiConfig.targets`，3.x `readNapiConfig` 优先读 `targets`，`triples` 为 deprecated 兼容）。
- 保留两者：2.x 读 `triples`（4 平台正确），3.x 读 `targets`（4 平台正确）——**双版本安全**。`targets` 同时满足任务"加 targets"诉求与未来 3.x 升级前向兼容。
- 4 triple 值在两个字段中完全一致，无冲突。

## binaryName 说明

2.x 读 `napi.name`（不存在 → `'index'`），忽略 `napi.binaryName: "vane"`。故 `napi build` 产出 `index.<platform>.node`，与仓库既有 `index.js`（napi-rs 生成，require `./index.<platform>.node`）与 `index.darwin-arm64.node` 一致。保留 `napi.binaryName: "vane"`（3.x 字段，2.x 忽略，无害；3.x 升级时会重新生成 `vane.*.node` + 配套 `index.js`，属版本切换的正常产物刷新）。未加 `napi.name`（避免 2.x binaryName 变为 `vane`，破坏既有 loader）。

## 自证

1. **JSON 合法**：`python3 -c "import json; json.load(open('crates/vane-node/package.json'))"` → OK；`napi.config.json` → OK
2. **YAML 合法**：`python3 -c "import yaml; yaml.safe_load(open('.github/workflows/release.yml'))"` → OK
3. **actionlint**：`actionlint .github/workflows/release.yml` → exit 0
4. **targets 到位**：`grep -n 'targets' crates/vane-node/package.json crates/vane-node/napi.config.json` → 两文件各有 `"targets": [`
5. **triples 保留**：`grep -n 'triples' ...` → 两文件各有 `"triples": {`
6. **无残留旧命令**：release.yml 无 `napi publish`、无 `napi artifacts --target`、无 `*.tgz`、无 `create-npm-dirs`（复数）
7. **逻辑审查**：
   - build job：`napi build --target` → upload `.node` ✓
   - release job：download → stage `.node` → `create-npm-dir -t .` → `artifacts --dir .` → `npm publish`（prepublishOnly）→ softprops Go/WASM ✓
   - build-go / build-wasm 不变 ✓
8. **端到端实测**：2.x CLI + 新配置 + 假 `.node` → `create-npm-dir` 正好 4 平台目录 + `artifacts --dir .` 收集成功 + 平台包 package.json 正确 ✓

## SPEC 触及

未触及。`docs/SPEC.md` 仅在第 329/370 行架构性提及 napi-rs（"vane-node 用 napi-rs 直连 core"），不引用 `@napi-rs/cli` 版本、`napi.triples`/`napi.targets` 字段格式或 `napi` 子命令（publish/artifacts/prepublish/create-npm-dir），故无需同步更新规范。

## 约束遵守

- 只改 release.yml + package.json + napi.config.json ✓
- 未改 ci.yml / 代码 / 4 triple 值 ✓
- 保留 build-go / build-wasm / softprops Go/WASM ✓
- YAML + JSON 合法，actionlint 通过 ✓
- 未 push ✓

## 待办（非本轮范围）

- 首次 tag 发版前需确认 `NPM_TOKEN` secret 已配置、`@vane-rs/node` npm scope 所有权就绪。
- `napi prepublish -t npm` 会创建/复用 `v<version>` GitHub Release 并上传 `.node`；softprops 随后追加 Go `.a` + WASM `.wasm`。两者均 `if tag` 触发，顺序为 npm publish → softprops，softprops 对已存在 Release 追加 asset（不覆盖）。
- 若未来从 2.x 升级到 3.x：`targets` 已就位可直接用，但 `napi.binaryName: "vane"` 会在 3.x 生效，产出 `vane.*.node`（需配套重新生成 `index.js`）；或升级时统一为 3.x 字段并移除 `triples`。
