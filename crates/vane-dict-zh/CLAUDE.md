# `vane-dict-zh` 指令

## 职责

- 此 crate 是纯中文词典数据包：`include_bytes!` 暴露预编译 `dict.bin`（zstd 压缩 DAT + HMM）与 `sha256_prefix.bin`（8 字节校验前缀）。
- 运行期零依赖；词典加载、分词与匹配逻辑在 `vane-core`（`jieba` feature），本 crate 只提供数据，不复制任何解析或业务逻辑。

## 核心约束

- `dict.bin` 物理格式（SPEC §5.2，解压后 magic `VNDT`）与 `sha256_prefix.bin` 属于兼容性协议。改动前核对 `docs/SPEC.md`；格式或校验前缀变更必须同步 `vane-core::tokenizer::jieba::JiebaDict::load_zstd` 与四渠道分发校验。
- 不要手改 `data/dict.bin` 或 `sha256_prefix.bin`。重新生成走 `cargo run --release -p vane-dict-zh --example gen_dict -- [--small|--full]`；生成器负责计算真 SHA-256 前缀并写入。
- 词典日历版本（`YYYY.M.x`，如 `2026.8.0`）与 Vane 库 semver 解耦。词典升级仅警告不强制重建索引（SPEC §3.3）；升级词典时更新 `DICT_VERSION` 并跑 `scripts/check-dict-hash.sh` 确认四渠道（Node/Go/WASM CDN/npm 包）一致。
- 体积门禁见 `scripts/check-dict-size.sh`：Node `dict.bin` gzip ≤ 1.5MB，Go embed `< 2MB`。新增源数据后必须复核体积不破线。
- npm 包（`@vane-rs/dict-zh`）无 JS 入口、无 postinstall；`package.json` 的 `files`/`exports` 元数据变更需与 `scripts/check-dict-hash.sh` 的 npm pack 字节级比对保持一致。

## 验证

- 运行 `cargo test -p vane-dict-zh`（含 `dict.bin` 可加载性校验）。
- 重新生成词典后必须运行 `scripts/check-dict-hash.sh` 与 `scripts/check-dict-size.sh`，确认哈希一致且体积达标。
- 冷加载性能回归跑 `cargo bench -p vane-dict-zh`（SPEC §13.1 < 150ms）。
