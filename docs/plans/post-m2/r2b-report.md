# R2B Report：release.yml Node（napi-rs）发布流程重构

## 状态

已完成。仅改 `.github/workflows/release.yml`，未 push。YAML + actionlint 均通过。

## napi pre-publish 是否自动 publish（查证结论）

**是，但只发布 optionalDependencies 平台包，不发布根包。**

源码查证（`cli/src/api/pre-publish.ts`，3.8.5 main 分支）：

- `prePublish` 遍历 `releasePackages`（每个配置 target 一个，staged 自 `npmDir/<platformArchABI>`），对每个执行：
  ```js
  execSync(`${npmClient} publish`, { cwd: publicationPackageDir, env: process.env, stdio: 'pipe' })
  ```
  即自动 `npm publish` 所有 `@vane-rs/node-*` 平台包（optionalDependencies）。
- **根包不发布**：源码注释明确 "prePublish publishes only the generated per-platform packages; the root package is packed and published by whoever called us." 无根目录的 publish 调用。
- `--skip-optional-publish`（默认 false）：跳过平台包 `npm publish`，但其余步骤（dry-run guard、artifact 检查、GitHub release 上传）仍执行。
- `--gh-release`（clipanion Boolean，默认 true）：napi 自建 GitHub Release 并上传 .node assets。`--no-gh-release` 禁用。
- `--root-publisher`：仅声明哪个 PM 发布根包（用于 export map 校验），不触发根包 publish。

**结论**：release job 需 `napi pre-publish`（自动发平台包）+ 单独 `npm publish --access public`（发根包 @vane-rs/node）。

### 参考来源

- https://github.com/napi-rs/napi-rs/blob/main/cli/docs/pre-publish.md
- https://github.com/napi-rs/napi-rs/blob/main/cli/docs/artifacts.md
- https://github.com/napi-rs/napi-rs/blob/main/cli/src/api/pre-publish.ts（execSync `${npmClient} publish`）
- https://github.com/napi-rs/napi-rs/blob/main/cli/src/def/pre-publish.ts（Option.Boolean('--gh-release', true)）
- https://deepwiki.com/napi-rs/napi-rs/5.3-release-process

## 重构设计

### 三个根因 → 三个修复

| # | 根因（r2-fix-report.md） | 修复 |
|---|---|---|
| 1 | `napi artifacts --target` 不支持 --target | build job 移除 artifacts 步骤；release job 用 `--output-dir . --npm-dir npm` |
| 2 | `napi artifacts` 要求所有 4 target .node 同时存在（per-platform 只有 1 个） | artifacts 移到 release job，聚合 4 平台 .node 后执行一次 |
| 3 | `napi publish` 不存在；artifacts 不生成 .tgz | 用 `napi pre-publish`（自动发平台包）+ `npm publish`（发根包）；upload 去掉 `*.tgz` |

### build job（per-platform matrix，4 平台）改动

- 移除 `napi artifacts --output-dir .` 步骤（per-platform 只有 1 .node，artifacts 要求全部 4 个）。
- `napi build --target ${{ matrix.target }} --platform --release` 不变（已验证成功）。
- upload-artifact：只上传 `crates/vane-node/*.node`，移除 `*.tgz`（artifacts 不在 build job 跑，无 .tgz）。

### release job 改动

新增/替换步骤序列：

1. checkout + setup-node（registry-url）+ install @napi-rs/cli（不变）。
2. download-artifact（path: artifacts）：获取 4 .node + go .a + wasm .wasm。
3. **Stage node .node files**（新增）：`cp artifacts/vane-node-*/*.node crates/vane-node/` —— 放回 napi build 默认产出位置，满足 artifacts 扫描要求。
4. **napi artifacts --output-dir . --npm-dir npm**（新增，working-directory: crates/vane-node）：收集 4 .node 到 `npm/<platform>/` 平台包目录。workflow_dispatch 也执行（仅收集验证，不 publish）。
5. **napi pre-publish -t npm --no-gh-release**（替换原 `napi publish`，if: tag，env NODE_AUTH_TOKEN）：更新 package.json + 复制 addon + 自动 `npm publish` 4 个 optionalDeps 平台包。`--no-gh-release` 禁用 napi 自建 Release，避免与 softprops 冲突。`-t npm` 匹配 `v0.1.0` tag 风格。
6. **npm publish --access public**（新增，if: tag，env NODE_AUTH_TOKEN，working-directory: crates/vane-node）：发布根包 @vane-rs/node（pre-publish 不发根包）。`--access public` 因 scoped 包首次发布需显式 public。
7. **softprops/action-gh-release**（保留，if: tag）：Go .a + WASM .wasm 作为 GitHub Release assets，逻辑不变。

### workflow_dispatch 行为

- build/build-go/build-wasm 全跑（构建验证）。
- release job：artifacts 执行（验证 4 .node 聚合），pre-publish / npm publish / gh-release 全部 `if: startsWith(github.ref, 'refs/tags/')` 跳过。

### permissions

- 顶层 `contents: read`（不变）。
- release job `contents: write`（不变，gh-release 需要）。

## 自证

### 1. YAML 校验

```
$ python3 -c "import yaml; d=yaml.safe_load(open('.github/workflows/release.yml')); print('YAML OK'); print('jobs:', list(d['jobs'].keys()))"
YAML OK
jobs: ['build', 'build-go', 'build-wasm', 'release']
```

### 2. actionlint

```
$ actionlint .github/workflows/release.yml
$ echo "actionlint exit: $?"
actionlint exit: 0
```

actionlint 1.7.12，零告警零错误。

### 3. 逻辑审查

- build job（4 平台各自）：`napi build` → upload 1 个 `*.node`（去掉 artifacts + .tgz）✓
- release job：download 4 .node → stage 到 crates/vane-node/ → `napi artifacts`（4 .node 齐全，不抛 Missing artifacts）→ `napi pre-publish`（自动发 4 平台包）→ `npm publish`（发根包）→ gh-release（Go/WASM）✓
- workflow_dispatch：artifacts 跑（验证），publish/release 全 if:tag 跳过 ✓
- tag 触发：pre-publish（平台包）+ npm publish（根包）+ gh-release（Go/WASM assets）✓
- napi 自建 gh-release 已用 `--no-gh-release` 禁用，不与 softprops 冲突 ✓

## SPEC 影响

无。`docs/SPEC.md` §12.2 定义 Node prebuilt 4 平台 triples 目标矩阵（与 napi.config.json 一致，未改），不涉及 napi CLI 命令调用、artifacts/pre-publish 流水线或 upload-artifact 细节。本次仅改 workflow，不动 ci.yml/代码/napi.config.json/package.json。

## 未改动

- `.github/workflows/ci.yml`、crates/vane-node 代码、napi.config.json、package.json 均未触碰。
- build-go / build-wasm job 完全保留（已验证 ✅）。
- release job 的 Go .a + WASM .wasm GitHub Release assets 逻辑（softprops）保留。
