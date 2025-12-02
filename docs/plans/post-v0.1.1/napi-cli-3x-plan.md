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

### 3.4 napi Rust crate 配对升级（T1 实测阻塞 → 范围扩展，已落定）

**阻塞根因（T1 failure 1，task-1-report.md）**：@napi-rs/cli 3.x 设 `NAPI_TYPE_DEF_TMP_FOLDER` env var（cli.js:7168），但 `napi-derive =2.16.13` 读旧名 `TYPE_DEF_TMP_PATH`（napi-derive-2.16.13/src/expand/napi.rs:139,193）→ 名称不匹配 → napi-derive 宏不写 intermediate type def → CLI `generateTypeDef()` 空 → **不生成 `index.js` loader** → `main.js require('./index.js')` MODULE_NOT_FOUND → pub API 断裂。这是 **硬版本配对要求**：napi-derive 2.x + CLI 3.x = 静默不生 index.js；napi-derive 3.x + CLI 2.x = 编译期 panic（napi-derive 3.x 源码 `crates/macro/src/expand/typedef/type_def.rs` 显式 panic）。二者必须同升降。

**范围扩展决策**：原 T1 brief 禁触 Cargo.toml（"不动 Rust 代码"）。但 3.x CLI 升级**必然**要求 `napi`+`napi-derive` 同升 3.x——这是 plan defect（原范围低估耦合），非 SPEC 矛盾/阻塞/用户侧操作。按编排者授权（plan defect 修复 within authority），扩 T1 范围含 Cargo.toml napi 升级。

**napi 3.x 安全性查证（源码逐版对比，research report）**：
- `napi::Error` 结构体：`pub reason: String` + `pub status: S` 在 **2.16.13 / 3.0.0 / 3.6.x 全部 pub 不变**。S14 担忧（"reason 字段访问在新版本失效"）**未发生**。
- `Error::new<R: ToString>(status, reason)` 签名逐字节一致（2.16.13:131 / 3.0.0:173）。
- Vane 的 `e.reason` 直接读（error.rs ×5 + convert.rs ×5，**全 `#[cfg(test)]`**）+ `Error::new(status_of(code), format!(...))` 构造（error.rs:50）+ `NapiResult<T>` 别名 → **3.x 全部不变，零 Rust 源码改动**。
- bare `#[napi]`（无属性参数）语法 3.x 不变（Vane 全部 bare 用法，grep 确认零 `#[napi(`）。
- features `napi8`+`serde-json` 3.x 仍有效；`compat-mode` 在 3.x 不再是 default，但 Vane grep 确认零 compat 类型用法（`JsObject`/`JsBuffer`/`Reference`/`ThreadsafeFunction` 等零命中）→ 安全。
- `napi-build` **保持 2.x**（3.0.0-beta.0 yanked；2.4.1 仍最新且 emit 双 env var 兼容）；`napi_build::setup()` 签名不变。build.rs 不动。
- `ToNapiValue` 3.0.0 加 `: Sized` bound → Vane `Json` newtype 是 Sized，不受影响。
- 3.6.x 新增 `pub cause: Option<Box<Error>>` 字段 → Vane 用直接 `e.reason` 访问（非结构体 pattern），不受影响。**故 pin `napi = "3"`（semver range，不钉 3.6.x）**。
- env var 改名时间线：napi-derive 3.0.0-alpha.0 仍 `TYPE_DEF_TMP_PATH` → 3.0.0 stable（2025-07-17）改 `NAPI_TYPE_DEF_TMP_FOLDER` + 旧名 panic。

**版本 pin（配 @napi-rs/cli 3.8.5，同发版日 2026-04-15 配对：napi 3.8.5 + napi-derive 3.5.4；用 semver range 留更新空间）**：
```toml
napi = { version = "3", features = ["napi8", "serde-json"] }
napi-derive = "3"
napi-build = "2"   # 不变（2.4.1）
```
移除 `=2.16.13` 精确 pin；S14 注释改写为 3.x 现状（reason 仍 pub，配对要求说明）。

## 4. 任务分解（串行，T2 可与 T1 审查重叠）

### T1｜本地 3.x CLI 验证 + napi Rust crate 升级 + package.json/napi.config 配置【keystone】
- 清理 `crates/vane-node/` 旧产物：`rm -f index.js *.node`（陈旧 2.x 产物，packageName 陈旧）。
- **Cargo.toml napi 升级**（§3.4，解阻关键）：
  - `crates/vane-node/Cargo.toml:13` `napi = { version = "=2.16.13", features = ["napi8", "serde-json"] }` → `napi = { version = "3", features = ["napi8", "serde-json"] }`。
  - `:14` `napi-derive = "=2.16.13"` → `napi-derive = "3"`。
  - `:24` `napi-build = "2"` **不动**（2.4.1 兼容双 env var）。
  - `:12` S14 注释改写：2.16.13 锁定理由已失效（reason 仍 pub）→ 3.x 现状 + 配对要求（napi/napi-derive/@napi-rs/cli 必须同 3.x，env var `NAPI_TYPE_DEF_TMP_FOLDER`）。
- `cd crates/vane-node && npm install @napi-rs/cli@^3 --no-package-lock`（devDep 升级 + 本地装 3.x；勿生成 lockfile）。
- 本地实测 3.x 命令并记录产出：
  - `napi build --platform --release` → 确认产出 `vane.<platform>.node`（binaryName 激活）+ **再生 `index.js`**（napi-derive 3.x 读新 env var → type def 正常 → index.js 生成）；读 `index.js` 确认引用 `@vane-rs/node-<platform>`（非陈旧 `@vane/node-*`）+ `vane.<platform>.node`。
  - `node -e "require('./main.js')"` → 确认全链路通（main.js → index.js → vane.darwin-arm64.node）。
  - `napi create-npm-dirs`（复数，无 `-t`）→ 确认创建 `npm/<platform>/` + 平台包 `main` 指向 `vane.<platform>.node`。
  - `napi artifacts --help` → 确认 `--output-dir,-o,-d` + `--npm-dir`，无 `--target`/`--dir`。
  - `napi pre-publish --help` → 确认 `-t`=`--tag-style`(`npm|lerna`)；**勿真跑**（会发 npm/挂 GH Release）。
- 应用配置改动（package.json + napi.config.json，T1 failure 1 已实测正确，续用）：
  - `package.json` devDep `^2.18.0` → `^3`；scripts.prepublishOnly `prepublish` → `pre-publish`；napi 字段移除 `triples`，保留 `targets`+`binaryName:"vane"`+`packageName`。
  - `napi.config.json`：**删除**（§3.2，3.x 仅 `-c` 传参才读，本仓库无 `-c`；grep 确认无代码引用）。
  - `.gitignore` 追加 `npm/`（create-npm-dirs 产物）。
- binaryName 决策：T1 failure 1 已证 `vane`/`index` 均产正确 .node 且链路通断与 binaryName 无关（根因是 napi-derive env var）。napi-derive 升 3.x 后链路应通 → **保留 `vane`**（与 v0.1.1 命名解耦，3.x 激活 binaryName 为预期行为）。
- 报告：3.x 命令实测确认表 + Cargo.toml napi 升级 + build 产出 `vane.*.node` + **再生 index.js 的 require 行** + 链路验证输出 + binaryName 决策 + napi.config.json 删除 + 门禁。
- 门禁：`cd crates/vane-node && npm test`（ava，应全绿——index.js 再生）+ `cargo check -p vane-node` + `cargo test -p vane-node`（reason 字段测试应不变通过）+ `npm run check:thin`。

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
