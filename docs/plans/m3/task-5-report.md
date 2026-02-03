# Task 5 Report：@vane-rs/dict-zh npm 包元数据

## 状态

✅ 完成。crates/vane-dict-zh/ 已具备独立 npm 数据包的全部元数据（package.json + .npmignore + README.md），`npm pack --dry-run` 验证产物内容、体积、exports map 全部对齐。

## Commits

- `2aadd28` feat(dict-zh): @vane-rs/dict-zh npm 包元数据（package.json + .npmignore + README）

## 测试摘要

`npm pack --dry-run` 产物仅 4 文件（README.md 2.9kB + data/dict.bin 1.5MB + data/sha256_prefix.bin 8B + package.json 733B），体积 1.5MB；exports `./dict.bin` → `./data/dict.bin` + `./sha256_prefix.bin` → `./data/sha256_prefix.bin`；version=2026.8.0，license=Apache-2.0；未触碰 vane-wasm/ vane-dict-zh src/data/Cargo.toml。

## npm pack --dry-run 产物清单

```
npm notice 📦  @vane-rs/dict-zh@2026.8.0
npm notice Tarball Contents
npm notice 2.9kB   README.md
npm notice 1.5MB   data/dict.bin
npm notice 8B      data/sha256_prefix.bin
npm notice 733B    package.json
npm notice Tarball Details
npm notice name:          @vane-rs/dict-zh
npm notice version:       2026.8.0
npm notice filename:      vane-rs-dict-zh-2026.8.0.tgz
npm notice package size:  1.5 MB
npm notice unpacked size: 1.5 MB
npm notice total files:   4
```

无 src/、tests/、benches/、examples/、Cargo.toml、target/、*.rs——纯数据包。

## exports map

```json
{
  "./dict.bin": "./data/dict.bin",
  "./sha256_prefix.bin": "./data/sha256_prefix.bin",
  "./package.json": "./package.json"
}
```

- `import dictBinUrl from '@vane-rs/dict-zh/dict.bin'` → vite/webpack 解析为 `data/dict.bin` 资源 URL
- `import dictSha256Url from '@vane-rs/dict-zh/sha256_prefix.bin'` → 解析为 `data/sha256_prefix.bin` 资源 URL
- 无 `"."` 根导出（纯数据包，无 JS 入口）；`require('@vane-rs/dict-zh')` 会 ERR_PACKAGE_PATH_NOT_EXPORTED，符合预期
- 无 main/module/types 字段

## 关键字段对齐

| 字段 | 值 | 对齐目标 |
|------|-----|----------|
| name | `@vane-rs/dict-zh` | @vane-rs/web optionalDep |
| version | `2026.8.0` | Cargo.toml version + @vane-rs/web optionalDep `2026.8.0` + CDN URL `@vane-rs/dict-zh@2026.8.0` |
| license | `Apache-2.0` | 仓库 workspace.package.license |
| files | `["data/dict.bin", "data/sha256_prefix.bin"]` | 只发数据，不发 Rust 代码 |
| publishConfig.access | `public` | @vane-rs scope 私有，必须 public |

## 冻结路径验证

- `crates/vane-wasm/`：未触碰（git status 空）✅
- `crates/vane-dict-zh/src/`：未触碰 ✅
- `crates/vane-dict-zh/data/`：未触碰（dict.bin/sha256_prefix.bin 字节冻结）✅
- `crates/vane-dict-zh/Cargo.toml`：未触碰（publish=false 保留，Rust crate 不发 crates.io，package.json 是独立 npm 通道）✅

## 产出文件

| 文件 | 行数 | 说明 |
|------|------|------|
| `crates/vane-dict-zh/package.json` | 21 | npm 包元数据，exports + files + publishConfig |
| `crates/vane-dict-zh/.npmignore` | 20 | 排除 Rust 产物（src/tests/benches/examples/Cargo.*/target/*.rs），belt-and-suspenders |
| `crates/vane-dict-zh/README.md` | 73 | 纯数据包说明 + vite/webpack 用法 + 与 @vane-rs/web 配合 + dict.bin 格式 + 永不进 wasm 红线 |

## Concerns

1. **无 LICENSE 文件**：package.json 声明 `license: "Apache-2.0"`，但 crates/vane-dict-zh/ 目录下无 LICENSE 文件（仓库根有）。npm pack 产物不含 LICENSE（npm 只从包目录找）。metadata 字段已声明 license，对 npm registry 显示足够；但若要产物内含 LICENSE 文本，需后续从仓库根 cp 一份（非本任务范围，Task 11 发版前可评估）。@vane-rs/web 的 files 字段显式含 "LICENSE"——若一致性要求高，Task 11 可补。
2. **exports 无根导出**：`require('@vane-rs/dict-zh')` 会抛 ERR_PACKAGE_PATH_NOT_EXPORTED。这是纯数据包的预期行为（无 JS 入口），但若未来想加运行期 JS helper（如 `dictVersion` / `sha256PrefixHex`），需新增 `"."` 导出——当前无此需求。
3. **.npmignore 与 files 冗余**：`files` allowlist 已是主防线（npm 优先于 .npmignore），.npmignore 是 belt-and-suspenders。两者一致，无冲突。若后续维护时只改其一，需注意同步。

## 不做项确认（后续任务范围）

- 四渠道哈希校验扩展 → Task 6
- SPEC §12.3 修订 → Task 13（合并 v1.5）
- release.yml npm publish → Task 11
- Cargo.toml publish=false 未改 → Rust crate 仍不发 crates.io
- src/lib.rs / data/dict.bin 未改 → 字节冻结
