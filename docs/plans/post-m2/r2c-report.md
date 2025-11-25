# R2C Report: napi-rs 3.8.5 配置格式迁移（`triples` → `targets`）

## 状态

已完成。发版阻断项修复。

## 根因

napi-rs 3.8.5 的 `napi artifacts` 读取配置的 **`targets`** 字段（数组），不再识别旧格式
`triples: { defaults, additional }`。`crates/vane-node/napi.config.json` 仍用旧 `triples`，
导致 3.8.5 认为无配置 target，release job 报：

```
Internal Error: Artifacts were found for unconfigured targets: ...
```

## 改动

### `crates/vane-node/napi.config.json`

`napi` 对象内：

- 删除 `"triples": { "defaults": false, "additional": [...] }`
- 新增 `"targets": [...]`，4 个 triple 值与原 `additional` 完全一致：
  - `x86_64-unknown-linux-gnu`
  - `aarch64-apple-darwin`
  - `x86_64-apple-darwin`
  - `x86_64-pc-windows-msvc`
- 保留 `binaryName` / `packageName` / `packageVersion` / `napi.name`

改后 `napi` 对象：

```json
"napi": {
  "name": "vane",
  "targets": [
    "x86_64-unknown-linux-gnu",
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-pc-windows-msvc"
  ]
}
```

### `crates/vane-node/package.json`

检查结果：`napi` 字段仅含 `binaryName` 与 `packageName`，**无 `triples`**，无需改动。

## 未改动

- `.github/workflows/release.yml` / `ci.yml`：不动。
- 代码：不动。
- 4 个 triple 值：与原 `additional` 一致，未改。

## 自证

1. **JSON 合法性**：
   ```
   $ python3 -c "import json; json.load(open('crates/vane-node/napi.config.json'))"
   JSON OK
   ```

2. **`triples` 残留检查**（应为空）：
   ```
   $ grep -n 'triples' crates/vane-node/napi.config.json crates/vane-node/package.json
   exit=1   # 无匹配
   ```
   两个文件均无 `triples` 残留。

3. **`targets` 数组到位**：
   ```
   $ grep -n 'targets' crates/vane-node/napi.config.json
   7:    "targets": [
   ```

4. **napi build 行为不变**：`napi build` 通过 `--target` 命令行驱动多目标构建，不读 config 的
   `triples`/`targets`，故 build 行为不受影响（CI 已验证 build ✅）。`targets` 仅驱动
   `napi artifacts` / pre-publish 的包装阶段。
