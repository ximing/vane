# @vane-rs/dict-zh

Vane 中文词典数据包：`dict.bin`（zstd 压缩 DAT + HMM 参数）+ `sha256_prefix.bin`（sha256 前 8 字节校验）。
纯数据 npm 包，无 JavaScript 入口，供 `@vane-rs/web` 作 optionalDependency 使用。

- **纯数据**：`dict.bin` 是预编译 zstd 压缩帧，`sha256_prefix.bin` 是 8 字节校验前缀。无 JS 代码、无 postinstall、企业断网友好。
- **vite/webpack asset url**：`import dictBinUrl from '@vane-rs/dict-zh/dict.bin'` 由打包器解析为资源 URL。
- **与 `@vane-rs/web` 配合**：`@vane-rs/web` 将其声明为 optionalDependency，安装后零 CDN 依赖；未安装时 `createVane` 自动 fallback jsdelivr CDN。
- **日历版本化**：`2026.8.0`（YYYY.M.x），与 Vane 库 semver 解耦——词典升级仅警告不强制重建索引（SPEC §3.3）。

## 内容

| 路径 | 体积 | 说明 |
|------|------|------|
| `data/dict.bin` | ~1.41 MB | zstd 压缩词典帧，解压后头部 magic `VNDT` |
| `data/sha256_prefix.bin` | 8 bytes | dict.bin 解压后内容的 sha256 前 8 字节（编译期生成） |

dict.bin 物理格式（SPEC §5.2，解压后）：

```text
magic(4)="VNDT" | format_version(4 LE) | sha256_prefix(8) |
dict_version_len(2 LE) | dict_version | total_freq(8 LE) |
dat_len(4 LE) | base[i32] | check[i32] | values[i32] |
hmm_blob_len(4 LE) | hmm_blob
```

整体经 zstd 压缩；由 `vane_core::tokenizer::jieba::JiebaDict::load_zstd` 解析。

## 用法（配合 @vane-rs/web）

```ts
import { createVane } from '@vane-rs/web';
import dictBinUrl from '@vane-rs/dict-zh/dict.bin';
import dictSha256Url from '@vane-rs/dict-zh/sha256_prefix.bin';

// 1. 加载词典字节（@vane-rs/dict-zh optionalDep，零 CDN）
const dictData = new Uint8Array(await (await fetch(dictBinUrl)).arrayBuffer());
const sha256Hex = Array.from(new Uint8Array(await (await fetch(dictSha256Url)).arrayBuffer()))
  .map(b => b.toString(16).padStart(2, '0')).join('');

// 2. 创建 Vane 实例（dictData transferable 零拷贝到 Worker）
const vane = await createVane({
  vfs: 'opfs',
  dbPath: 'vane.db',
  dictData,
  dictSha256: sha256Hex,
});
```

> **vite**：`import dictBinUrl from '@vane-rs/dict-zh/dict.bin'` 原生解析为资源 URL，无需额外配置。
> **webpack 5**：需 `experiments: { outputModule: true }`，`new URL` / asset 模块原生支持。

## 安装（独立使用）

```bash
npm install @vane-rs/dict-zh
```

通常无需独立安装——`@vane-rs/web` 已将其声明为 optionalDependency，`npm install @vane-rs/web` 自动拉取。

## 词典永不进 wasm

红线（SPEC §12.3）：`dict.bin` 独立分发，不编译进 `.wasm` 产物。本包是独立 npm 数据包，天然遵守——wasm 体积门禁 ≤800KB 不受词典影响。

## License

Apache-2.0（见仓库根 [LICENSE](https://github.com/ximing/vane/blob/main/LICENSE)）。
