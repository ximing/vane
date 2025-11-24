# R1B 报告：npm scope 修正 @vane/node → @vane-rs/node

## 背景

用户 npm scope 为 `@vane-rs`（非 `@vane`）。发版前置修正，否则 `npm publish` 失败。

## 改动策略

全仓字符串替换 `@vane/node` → `@vane-rs/node`。

安全性论证：
- `@vane/node` 是 `@vane/node-linux-x64-gnu`、`@vane/node-darwin-arm64`、`@vane/node-darwin-x64`、`@vane/node-win32-x64-msvc` 的前缀，单一 sed 即可将 4 个 optionalDependencies 子包名同步变为 `@vane-rs/node-*`。
- `@vane/dict-zh` 不匹配 `@vane/node` 模式，保持不动。
- `napi.config.json` 的 `napi.name = "vane"`（napi binary name）不在替换范围，保留；`packageName` 被替换为 `@vane-rs/node`。
- 未触及 Cargo.toml / Rust 源码。

## 改动清单（23 个文件）

### 功能性配置 / 源码（5 文件）
1. `crates/vane-node/package.json` — `name`、4 个 `optionalDependencies`、`napi.packageName`（共 6 处）
2. `crates/vane-node/napi.config.json` — `packageName`（1 处）
3. `crates/vane-node/main.js` — 注释（1 处）
4. `crates/vane-node/main.d.ts` — 注释（1 处）
5. `.github/workflows/install-matrix.yml` — description / npm install / yarn add / pnpm add / bun add / require（6 处）

### 示例 / demo（5 文件）
6. `examples/demo/package.json` — dependency（1 处）
7. `examples/demo/package-lock.json` — name / 4 optionalDeps / node_modules 路径键（7 处）
8. `examples/demo/compare.js` — import（1 处）
9. `examples/demo/load-wiki.js` — import（1 处）
10. `examples/demo/README.md` — 4 处说明文本

### 文档（13 文件，历史计划记录）
11. `docs/plans/m0/09-node-binding.md`（多处）
12. `docs/plans/m0/10-ci-gates.md`（多处）
13. `docs/plans/m0/11-demo.md`（多处）
14. `docs/plans/m0/README.md`
15. `docs/plans/m0/M0-SUMMARY.md`
16. `docs/plans/m0/EXECUTION-NOTES.md`
17. `docs/plans/m1/README.md`
18. `docs/plans/m1/modules/07-dict-distribution-node.md`
19. `docs/plans/m1/modules/07-dict-distribution-node-report.md`
20. `docs/plans/m1/modules/10-ci-m1-report.md`
21. `docs/plans/post-m2/EXECUTION-NOTES.md`
22. `docs/plans/post-m2/z0-ci-preaudit-report.md`
23. `docs/plans/post-m2/z2b-report.md`

合计：23 文件，84 行增 / 83 行删。

## 未改动项（按要求保留）

- `@vane/dict-zh` 全部引用（Rust crate vane-dict-zh 概念引用，Cargo.toml publish=false 不发 npm）—— grep 确认 19 处全部保留。
- `napi.config.json` 的 `napi.name = "vane"`（napi binary name，非 npm 包名）。
- Cargo crate 名（vane-core / vane-ffi 等无 @scope）。
- API / 逻辑 / version 均未改。

## 自证结果

1. **`@vane/node` 残留检查**：`grep -rn '@vane/node' . --exclude-dir={node_modules,target,.git}` → **NO RESIDUAL**（PASS）
2. **`@vane-rs/node` 新引用**：23 个文件含新引用（PASS）
3. **package.json 合法性**：`python3 -c "import json; json.load(open('crates/vane-node/package.json'))"` → PASS
   - `name = @vane-rs/node`
   - `optionalDependencies = [@vane-rs/node-linux-x64-gnu, @vane-rs/node-darwin-arm64, @vane-rs/node-darwin-x64, @vane-rs/node-win32-x64-msvc]`
   - `napi.packageName = @vane-rs/node`
4. **workflow YAML 合法性**：4 个 workflow（benchmark / ci / install-matrix / release）全部 `yaml.safe_load` PASS
5. **napi.config.json 合法性**：JSON 解析 PASS，`packageName = @vane-rs/node`，`napi.name = vane`（binary name 未变）

## 额外验证

- `release.yml` 无 `@vane` 直接引用（napi-rs 发布时动态读取 `packageName`，自动使用新 scope）。
- `@vane/dict-zh` 19 处引用全部保留未动（grep 确认）。
- git diff stat：23 文件，+84 / -83。
