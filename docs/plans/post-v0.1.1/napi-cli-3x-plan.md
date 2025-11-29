# @napi-rs/cli 2.x → 3.x 升级计划（post-v0.1.1）

> 分支：`chore/napi-cli-3x`（从 main 切出，串行提交，发版前 ff-merge 回 main 打 tag）。
> 编排者：纯 Orchestrator（SubAgent 调度 + 任务管理，禁写代码，唯一例外本文档目录）。
> 关联：`docs/plans/post-m2/EXECUTION-NOTES.md`、`r2e-report.md`（6 轮调试正解）。

## 1. 背景与动机

v0.1.1 已发版，但 `@napi-rs/cli` 锁 `^2.18.0`（`crates/vane-node/package.json` devDependencies + `release.yml` 两处 pin）。post-M2 发版时 napi-rs 发布流程经 **6 轮调试**（2.x/3.x 不兼容），根因是 `package.json` 锁 2.x 而 `release.yml` 未 pin（拉到 3.8.5 latest）导致 2.x/3.x 混用。最终正解：**回退 2.x + 官方流程 + `publishConfig.access=public` + napi 双字段（`triples` 2.x 必读 + `targets` 3.x 兼容）**。

本任务消除 2.x 锁定遗留：升级 3.x，重测全发布流程，发 **v0.1.2**（patch；npm 不允许 republish v0.1.1）。3.x 最新版经 `npm view @napi-rs/cli version` 确认为 **3.8.5**，与 post-M2 源码分析版本一致——post-M2 的 3.8.5 源码查证结论仍然 current。

## 2. 当前状态（升级前，实测于 chore/napi-cli-3x 起点）

| 项 | 文件:行 | 当前值 |
|---|---|---|
| CLI devDep | `crates/vane-node/package.json:14` | `"@napi-rs/cli": "^2.18.0"` |
| CLI pin（build job） | `release.yml:57` | `npm install -g @napi-rs/cli@^2.18.0` |
| CLI pin（release job） | `release.yml:157` | `npm install -g @napi-rs/cli@^2.18.0` |
| napi 字段 | `package.json:24-42` + `napi.config.json` | `targets:[4 triples]` + `triples:{defaults:false, additional:[4 triples]}` 双字段 |
| prepublishOnly | `package.json:22` | `"napi prepublish -t npm"` |
| create-npm-dir | `release.yml:176` | `napi create-npm-dir -t .`（2.x 单数） |
| artifacts | `release.yml:182` | `napi artifacts --dir .`（2.x `--dir`） |
| publish | `release.yml:189` | `npm publish`（触发 prepublishOnly；2.x/3.x 均无 `napi publish`） |
| publishConfig | `package.json:43-45` | `{ "access": "public" }`（保留） |
| index.js stage | `release.yml:62-66,168-170` | build upload `.node`+`index.js`；release stage 两者（v0.1.1 修复，保留） |
| workspace 版本 | `Cargo.toml:6` | `version = "0.1.1"`（core/ffi/node/wasm 均 `version.workspace = true`） |
| Cargo.lock | 4 条 `0.1.1`（行 835/866/875/887） | 待 `cargo check` 重生成 |
| package.json 版本 | `package.json:3,8-11` | `0.1.1` + 4 optionalDeps `0.1.1` |
| install-matrix 默认 | `install-matrix.yml:12` | `default: '0.1.1'` |
| vane-dict-zh | `crates/vane-dict-zh/Cargo.toml:3` | `2026.8.0`（日历版，独立，**不动**） |

4 triples（targets/triples 两字段一致）：`x86_64-unknown-linux-gnu`、`aarch64-apple-darwin`、`x86_64-apple-darwin`、`x86_64-pc-windows-msvc`。

## 3. 3.x 查证结论（源码验证 @napi-rs/cli@3.8.5，post-M2 r2-fix/r2c/r2e）

| 维度 | 2.x（2.18.0/2.18.4） | 3.x（3.8.5） | 来源 |
|---|---|---|---|
| 配置字段 | 读 `napi.triples.{defaults,additional}`（`cli/src/consts.ts:getNapiConfig`）；忽略 `targets` | 读 `napi.targets: [...]`（`cli/src/utils/config.ts:UserNapiConfig.targets`）；`triples` 弃用 | r2e §关键偏差 |
| binaryName | 读 `napi.name`（缺省 `'index'`）；**忽略 `binaryName`** | 读 `napi.binaryName` | r2e §binaryName 说明 |
| npm-dir 命令 | `create-npm-dir`（**单数**，2.x 接受 `-t .`） | `create-npm-dirs`（**复数**，**无 `-t` flag**；drop `-t .`） | r2e §命令查证 + web |
| artifacts 选项 | `-d,--dir`（默认 `artifacts`）+ `--dist`（默认 `npm`） | `--output-dir,-o,-d`（默认 `./artifacts`）+ `--npm-dir`（3.x **拒 `--dir`**） | r2-fix §问题 1 + web |
| `--target` | 不支持 | 不支持（2.x/3.x 均无） | r2 第 1 轮失败 |
| prepublish 命令 | `prepublish`（无连字符） | `pre-publish`（canonical；`prepublish` 别名仍可用）；`-t`=`--tag-style`(`npm\|lerna`) | r2b/r2d + web |
| `napi publish` | **不存在** | **不存在**（2.x/3.x 均无；用 `npm publish` + prepublishOnly） | r2e 纠正 r2d |
| 3.x 注册命令集 | — | `new/build/create-npm-dirs/artifacts/universalize/rename/pre-publish/version/help` | r2-fix §问题 2 |

### 3.1 关键风险：`binaryName` 激活（核心 landmine）

当前 `napi.binaryName: "vane"` 在 2.x 下是 **no-op**（2.x 读 `napi.name`，缺省 → `'index'`），故 v0.1.1 产出 `index.<platform>.node` + `index.js` loader（loader 内 `existsSync('index.<platform>.node')` + 包回退 `require('@vane-rs/node-<platform>')`）。

升 3.x 后 `binaryName: "vane"` **激活**：
- `napi build` 产出 `vane.<platform>.node`（非 `index.<platform>.node`）。
- 再生成的 `index.js` loader 的本地文件探测改为 `vane.<platform>.node`；包回退仍由 `packageName: "@vane-rs/node"` 驱动 → `require('@vane-rs/node-<platform>')`（**与 binaryName 无关，不受影响**）。
- `create-npm-dirs` + `artifacts` 将 `vane.<platform>.node` 打入 `npm/<platform>/`，平台包 `main` 指向它。

**发布链路**（root 包 `files` 不含 `.node`，故 loader 本地探测必失败 → 走包回退）：
`main.js → index.js（探测 vane.<platform>.node 本地→false→require @vane-rs/node-<platform>）→ 平台包 main → vane.<platform>.node`

包回退路径与 binaryName 无关，故**理论上链路自洽**，但**必须本地实测**：3.x `napi build` 产出的 `index.js` 是否正确引用 `@vane-rs/node-<platform>`（而非陈旧 `@vane/node-*`）+ `vane.<platform>.node` 文件名 + 平台包 `main` 指向正确。本地现存 `index.js`/`index.darwin-arm64.node` 为 **2.x 旧产物且 packageName 陈旧**（`@vane/node-*`），T1 须先清理再重生成。

r2e §待办原文：「若未来从 2.x 升级到 3.x：`targets` 已就位可直接用，但 `napi.binaryName: "vane"` 会在 3.x 生效，产出 `vane.*.node`（需配套重新生成 `index.js`）」。

**回退方案**（若 T1 实测链路断裂）：将 `binaryName` 改 `"index"`（或移除让 3.x 缺省 `index`）以保留 v0.1.1 命名——`.node` 文件名是包内部细节（用户不直接 require），不属于冻结的 JS 契约。此为 T1 实测后的决策点。

### 3.2 napi.config.json 去留（web 源码落定：删除）

3.x `readNapiConfig(path, configPath?)` 读 `package.json` 的 `napi` 字段；独立配置文件**仅经 `--config-path`/`-c` 显式传参才读**（无自动发现），且若两者并存独立文件 win（3.5.1 起硬化，打印黄色警告）。本仓库 release.yml 所有 napi 命令**均未传 `-c`**，故 `napi.config.json` 在 2.x/3.x 下**均不生效**（documentary）。grep 确认仅 release.yml 注释引用，无代码/脚本依赖。

**决策：删除 `crates/vane-node/napi.config.json`**（消除双份配置漂移；`package.json` napi 字段为唯一真相源）。T1 先 grep 复确认全仓无 `napi.config.json` / `-c napi` 引用再删。

### 3.3 web 源码交叉验证 + 运行时要求

web 查证（`napi-rs/napi-rs` cli/src 源码 + `package-template` + `website` MDX，2026-08-10）与 post-M2 3.8.5 源码分析**完全一致**：

- `targets` flat array（3.x 读；`triples` 弃用但回退读）✅
- `create-npm-dirs` 复数，**无 `-t` flag**（2.x `create-npm-dir -t .` → 3.x `create-npm-dirs`，drop `-t .`）✅
- `artifacts --output-dir,-o,-d` + `--npm-dir`；无 `--target`/`--dir`（3.x 拒 `--dir`）✅
- `pre-publish` canonical，`prepublish` 别名仍可用；`-t`=`--tag-style`（`npm|lerna`，缺省 `lerna`）✅
- 无 `napi publish`；`npm publish` + `prepublishOnly` ✅
- `pre-publish --gh-release` 缺省 `true`（创建/挂 GH Release + 上传 .node）→ softprops 追加 Go/WASM，与 v0.1.1 流程一致 ✅
- 3.x CLI 运行时：Node `^20.17.0 || ^22.13.0 || >=23.5.0`（本地 20.20.1 ✅；release.yml `node-version: '20'` 解析最新 20.x ✅）；Rust `>=1.88`（stable ✅）。
- `binaryName` 3.x 缺省 `"index"`；本仓库 `"vane"` 激活 → `vane.<platform>.node`（见 §3.1）。
- 迁移指南其他 breaking（`universal`→`universalize`、`--cargo-cwd`→`--manifest-path`、`napi.package.name`→`packageName`）本仓库均不涉及（无 universalize、用 working-directory 非 `--cargo-cwd`、已有 `packageName`）。

## 4. 任务分解（串行，T2 可与 T1 审查重叠）

### T1｜本地 3.x CLI 验证 + package.json/napi.config 配置升级【keystone】
- 清理 `crates/vane-node/` 旧产物：`rm -f index.js *.node`（陈旧 2.x 产物）。
- `cd crates/vane-node && npm install @napi-rs/cli@^3`（devDep 升级 + 本地装 3.x）。
- 本地实测 3.x 命令并记录产出：
  - `napi build --platform --release` → 确认产出 `vane.<platform>.node`（非 `index.*`）+ 再生成 `index.js`；读 `index.js` 确认引用 `@vane-rs/node-<platform>`（非陈旧 `@vane/node-*`）+ `vane.<platform>.node`。
  - `napi create-npm-dirs`（复数）→ 确认命令存在 + 创建 `npm/<platform>/` 目录。
  - `napi artifacts --output-dir . --npm-dir npm` → 确认收集 `vane.<platform>.node` 到 `npm/<platform>/` + 平台包 `main` 指向正确。
  - `npm publish --dry-run`（触发 `prepublishOnly`）→ 确认 `napi pre-publish -t npm` 在 dry-run 下不报错（需先把 scripts.prepublishOnly 改 `pre-publish`，见下）。
- 应用配置改动：
  - `package.json:14` devDep `^2.18.0` → `^3`。
  - `package.json:22` scripts.prepublishOnly `napi prepublish -t npm` → `napi pre-publish -t npm`。
  - `package.json:24-42` napi 字段：**移除 `triples`**（3.x 不读），保留 `targets`（3.x 读）。`binaryName` 暂留 `"vane"`（实测链路通则保留；断则回退 `"index"` 并在报告标注）。
  - `napi.config.json`：按 §3.2 决策——删除文件（若 3.x 不读）或同步移除 `triples`。
- 报告：3.x 命令名/flags 实测确认表 + 产出文件名 + `index.js` 关键 require 行 + binaryName 决策结论 + napi.config.json 去留结论。
- 门禁：`cd crates/vane-node && npm test`（ava）+ `cargo check -p vane-node`（无 Rust 改动，确认不破）。`npm run check:thin`（分发结构）。

### T2｜release.yml 3.x 命令适配（依赖 T1 实测命令确认）
- `release.yml:57`（build job）+ `:157`（release job）：`@napi-rs/cli@^2.18.0` → `@napi-rs/cli@^3`。
- `release.yml:176`：`napi create-npm-dir -t .` → `napi create-npm-dirs`（3.x **无 `-t` flag**，drop `-t .`；`--npm-dir` 默认 `npm`，`working-directory: crates/vane-node` 提供 cwd）。T1 实测 `napi create-npm-dirs --help` 复确认。
- `release.yml:182`：`napi artifacts --dir .` → `napi artifacts --output-dir . --npm-dir npm`（T1 实测确认）。
- `release.yml:21-24,135-144,163-166,171-174,178-181` 等注释：2.x 口径 → 3.x 口径（`create-npm-dir`→`create-npm-dirs`、`--dir`→`--output-dir`、`prepublish`→`pre-publish`、移除"2.x"措辞）。
- stage 步骤（`:62-66,168-170`）：`.node` glob `*.node` 仍匹配 `vane.*.node`（通配，无需改）；`index.js` stage 保留（v0.1.1 修复）。若 T1 决定 `binaryName: "index"`，则 .node 回 `index.*.node`，glob 同样匹配。
- 门禁：`actionlint`（若可用）+ YAML 语法 + 人工对照 T1 实测命令复核。

### T3｜版本 bump 0.1.1 → 0.1.2（三端同步）
- `Cargo.toml:6`：`0.1.1` → `0.1.2`。
- `cargo check --workspace`（重生成 Cargo.lock 4 条 → `0.1.2`）。
- `package.json:3` version + `:8-11` 4 optionalDeps → `0.1.2`。
- `install-matrix.yml:12` default `0.1.1` → `0.1.2`。
- `release.yml:11` dispatch input description 示例 `0.1.1` → `0.1.2`（cosmetic 顺手）。
- `vane-dict-zh` 不动（`2026.8.0` 独立）。
- 门禁：`cargo fmt --all -- --check` + `cargo check --workspace --all-features`（确认 Cargo.lock 一致）。

### T4｜全量本地门禁
- `cargo fmt --all -- --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-features`
- `cargo check --target wasm32-unknown-unknown -p vane-core` + `-p vane-wasm`
- `cd crates/vane-node && npm test` + `npm run check:thin`
- Go：构建 `vane-ffi` + `cd bindings/go && go test ./...`
- 本地 3.x `napi build --platform --release` 复确认（T1 已做，T4 回归）。

### T5｜发版验证（打 tag 需用户确认）
1. `workflow_dispatch` release.yml on `chore/napi-cli-3x`：验证 build job（4 平台 `napi build`）+ release job（`create-npm-dirs` + `artifacts --output-dir` 收集验证）；`npm publish`/gh-release 经 `if: startsWith(github.ref,'refs/tags/')` 跳过。**3.x 命令全流程绿**。
2. ff-merge `chore/napi-cli-3x` → `main`（线性历史，无 merge commit）。
3. **用户确认** → 打 tag `v0.1.2` → release.yml 正式发版：`npm publish`（主包触发 `pre-publish` 发 4 平台包 + 上传 .node）+ softprops 挂 Go `.a`/WASM `.wasm`。
4. install-matrix 12/12 验证（`workflow_run` 自动触发或手动 dispatch）。
5. CI 16 jobs 不回退确认（ci.yml on main）。

### T6｜文档
- 本计划（持续更新，T1/T5 决策落账）。
- 总结报告 `docs/plans/post-v0.1.1/napi-cli-3x-summary.md`（发版后：升级历程 + 3.x 正解 + 实测结论 + 发版结果 + 遗留）。
- EXECUTION-NOTES 风格记录关键决策（binaryName、napi.config.json 去留、3.x 命令实测）。

## 5. 约束与风险

- **不破坏 v0.1.1 产物**：发 v0.1.2，v0.1.1 保留；npm 不允许 republish 同版本。
- **冻结 pub API**：`@vane-rs/node` JS 导出不变（`main.js`/`index.d.ts` 契约）。`.node` 文件名属内部细节，非 JS 契约。
- **SPEC §12.2** 三端 prebuilt 不变；napi-rs CLI 版本是实现细节，SPEC 不引用 CLI 版本/命令——**预计无需 SPEC 修订**。若触及 I-5 或 pub API，走 SPEC 修订流程向用户提议。
- **binaryName 激活**：核心风险，T1 实测兜底；回退方案 `binaryName: "index"`。
- **2 次 failure 换策略**：任一任务连续 2 次失败（同策略）→ 换策略（fresh agent / 不同方法 / 上报）。
- **用户打扰**：仅 SPEC 矛盾 / 阻塞 / 发版确认（打 tag）。

## 6. 完成定义（DoD）

- [ ] @napi-rs/cli 3.x 升级（devDep + release.yml 两处 pin + napi 配置 + scripts.prepublishOnly）。
- [ ] release.yml 3.x 命令流程验证（workflow_dispatch build + release job 绿）。
- [ ] 打 tag v0.1.2 发版 + `@vane-rs/node@0.1.2` + 4 平台包 published。
- [ ] install-matrix 12/12 全绿。
- [ ] CI 16 jobs 不回退。
- [ ] 本计划 + 总结报告落 `docs/plans/post-v0.1.1/`。
