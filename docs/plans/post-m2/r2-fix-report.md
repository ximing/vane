# R2 Fix Report：release.yml `napi artifacts` 命令用法修复

## 状态

已完成（语法修复）。但发现更深层架构问题需后续 task 处理（见下文"已知后续问题"）。

## 根因

`release.yml` build job 步骤 `napi artifacts --target ${{ matrix.target }}` 失败。`@napi-rs/cli` 新版（当前 3.8.5）的 `napi artifacts` 命令不支持 `--target` 选项。

CI 错误：
```
Unknown Syntax Error: Unsupported option name ("--target").
$ napi artifacts [--cwd #0] [--config-path,-c #0] [--package-json-path #0] [--output-dir,-o,-d #0] [--npm-dir #0] [--build-output-dir #0]
```

## `napi artifacts` 正确用法查证

### 来源

1. **官方文档（napi-rs monorepo）**：
   - `cli/docs/artifacts.md`：https://github.com/napi-rs/napi-rs/blob/main/cli/docs/artifacts.md
   - `cli/docs/build.md`：https://github.com/napi-rs/napi-rs/blob/main/cli/docs/build.md
   - `cli/README.md`：https://github.com/napi-rs/napi-rs/blob/main/cli/README.md

2. **源码验证（npm pack @napi-rs/cli@3.8.5 本地反编译）**：
   - `src/def/artifacts.ts`：选项定义
   - `src/api/artifacts.ts`：`collectArtifacts` 实现
   - `src/commands/artifacts.ts`：命令注册

### `napi artifacts` 支持的选项（3.8.5）

| CLI 选项 | 默认值 | 说明 |
|---|---|---|
| `--cwd` | `process.cwd()` | 命令执行工作目录 |
| `--config-path,-c` | — | napi config JSON 路径 |
| `--package-json-path` | `package.json` | package.json 路径 |
| `--output-dir,-o,-d` | `./artifacts` | 扫描 `.node`/`.wasm` 文件的目录（递归，跳过 node_modules） |
| `--npm-dir` | `npm` | npm 平台包输出目录 |
| `--build-output-dir` | — | 仅 WASI target 需要 |

**不支持 `--target`。** `--build-output-dir` 仅用于 WASI target，非本场景。

### 行为（源码确认）

1. 递归扫描 `--output-dir`（默认 `./artifacts`）下的 `.node`/`.wasm` 文件。
2. 从 `napi.config.json` 读取**所有**配置的 triples，要求**全部** target 的 `.node` 文件都在 `--output-dir` 下存在，否则抛 `Missing artifacts for configured targets`。
3. 将 `.node` 文件复制到 `--npm-dir/<platform-arch-abi>/` 和 package root。
4. **不生成 `.tgz`**（`.tgz` 由 `napi pre-publish` / `npm pack` 生成，非 `napi artifacts`）。

### `napi build` 输出位置

`napi build` 的 `--output-dir` 默认为 crate folder（cwd）。当前 release.yml build 步骤 `napi build --target ${{ matrix.target }} --platform --release` 未指定 `--output-dir`，所以 `.node` 文件产出在 cwd（`crates/vane-node/`）。

### 正确用法

```sh
napi artifacts --output-dir .
```

`--output-dir .` 指向 crate folder（cwd），即 `napi build` 产出 `.node` 文件的位置。这符合文档描述："same as `--output-dir` of build command"——build 默认 crate folder，artifacts 需指向同一位置。

## 修改

**文件**：`.github/workflows/release.yml`

**第 60 行**：
```diff
-        run: napi artifacts --target ${{ matrix.target }}
+        run: napi artifacts --output-dir .
```

**未改动**：
- `napi build` 命令（第 57 行，成功，不动）。
- 其他所有步骤。

## YAML 校验

```
$ python3 -c "import yaml; yaml.safe_load(open('.github/workflows/release.yml'))"
YAML OK
```

## SPEC 影响

无。`docs/SPEC.md` §12.2 仅定义 Node prebuilt 目标矩阵（4 平台 triples），不涉及 `napi artifacts` 命令调用、`.tgz` 或 `upload-artifact` 细节。本次修改不触及 SPEC。

## 已知后续问题（超出本 task 范围，需后续 task 处理）

### 问题 1：`napi artifacts` 要求所有 target 同时存在

`napi.config.json` 配置了 4 个 triples（linux-x64-gnu、darwin-arm64、darwin-x64、win32-x64-msvc）。当前 build job 是 per-platform matrix（每个 job 只产 1 个 `.node` 文件），但 `napi artifacts`（新版 cli）要求 4 个 `.node` 文件同时存在于 `--output-dir` 下。

**预测**：修复 `--target` 语法错误后，`napi artifacts --output-dir .` 会因缺少其他 3 个 target 的 `.node` 文件而抛 `Missing artifacts for configured targets`。

**正确架构**（napi-rs 官方推荐模式）：
1. Build job：仅 `napi build` + 上传 `.node` 文件（去掉 `napi artifacts` 步骤）。
2. Release job：下载全部 4 平台 `.node` 文件到同一目录 → 运行一次 `napi artifacts` → 生成 npm 平台包结构 → 发布。

### 问题 2：`napi publish` 命令不存在

`@napi-rs/cli` 3.8.5 注册的命令：`new`、`build`、`create-npm-dirs`、`artifacts`、`universalize`、`rename`、`pre-publish`（别名 `prepublish`）、`version`、`help`。

**无 `publish` 命令。** release.yml 第 164 行 `napi publish --tag latest` 将失败。应改为 `napi pre-publish` 或直接用 `npm publish`。

### 问题 3：upload-artifact 预期 `.tgz` 但 `napi artifacts` 不生成 `.tgz`

upload-artifact 步骤（第 64-66 行）收集 `*.node` 和 `*.tgz`。新版 `napi artifacts` 只复制 `.node` 文件到 npm 包目录结构，不生成 `.tgz`。`.tgz` 需由 `napi pre-publish` 或 `npm pack` 生成。

---

以上 3 个后续问题需在后续 task 中统一重构 release.yml 的 Node 发布流水线（build → collect → artifacts → pre-publish/publish）。
