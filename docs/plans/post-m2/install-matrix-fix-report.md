# install-matrix.yml yarn + bun setup 修复报告

## 状态
已完成（未 push）。

## 根因
1. **yarn 失败**：corepack 装 Yarn 4 (Berry)，默认 PnP 模式，`node -e require('@vane-rs/node')` 不加载 `.pnp.cjs` → Cannot find module。
2. **bun 失败**：`Setup pkg manager` step 中 `bun --version` 在 `echo "$HOME/.bun/bin" >> $GITHUB_PATH` 之后调用，但 `GITHUB_PATH` 只对后续 step 生效，当前 step 的 PATH 未含 bun → command not found。

## 改动（仅 `.github/workflows/install-matrix.yml` 2 处）

### 1. L41 — Setup pkg manager 的 bun 分支
新增 `export PATH="$HOME/.bun/bin:$PATH"`，让当前 step 的 `bun --version` 可用；`GITHUB_PATH` 保留供后续 step。

```diff
-bun)  curl -fsSL https://bun.sh/install | bash; echo "$HOME/.bun/bin" >> $GITHUB_PATH; bun --version ;;
+bun)  curl -fsSL https://bun.sh/install | bash; echo "$HOME/.bun/bin" >> $GITHUB_PATH; export PATH="$HOME/.bun/bin:$PATH"; bun --version ;;
```

### 2. L61 — Init test project 的 yarn 分支
`yarn init -y` 后写 `.yarnrc.yml` 配 `nodeLinker: node-modules`，让 Yarn 4 使用 node_modules 链接器（兼容 `node -e require`），再 `yarn add`。

```diff
-yarn) yarn init -y; yarn add @vane-rs/node@${{ steps.ver.outputs.version }} ;;
+yarn) yarn init -y; echo 'nodeLinker: node-modules' > .yarnrc.yml; yarn add @vane-rs/node@${{ steps.ver.outputs.version }} ;;
```

## 自证
1. **YAML 合法**：`python3 -c "import yaml; yaml.safe_load(open('.github/workflows/install-matrix.yml'))"` → `YAML OK`。
2. **grep 确认 2 处改动到位**：
   ```
   41:            bun)  curl -fsSL https://bun.sh/install | bash; echo "$HOME/.bun/bin" >> $GITHUB_PATH; export PATH="$HOME/.bun/bin:$PATH"; bun --version ;;
   61:            yarn) yarn init -y; echo 'nodeLinker: node-modules' > .yarnrc.yml; yarn add @vane-rs/node@${{ steps.ver.outputs.version }} ;;
   ```

## actionlint 备注
`actionlint` 报 1 条 shellcheck `SC2086:info`（L36 run 块内 `>> $GITHUB_PATH` 未加引号）。**此为既有问题**（`git stash` 后在原文件上同样复现），非本次改动引入，超出任务约束的 2 处范围，未改动。

## 约束遵守
- 只改 `install-matrix.yml` 这 2 处（bun `export PATH` + yarn `.yarnrc.yml`）。
- 未改其他步骤/分支/`ci.yml`。
- 未 push。
