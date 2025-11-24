# P3 词典 CDN 真实托管（jsdelivr gh）+ 三渠道哈希一致校验

## 状态

完成。jsdelivr gh CDN URL 已落地到 demo；check-dict-hash.sh Go 渠道校验已启用并跑通。

## 改动文件

| 文件 | 改动 |
|------|------|
| `demo/main.js` | `DICT_URL` 从 `"./pkg/dict.bin"` 改为 jsdelivr gh URL，加注释说明 private/public 生效条件与降级链路 |
| `scripts/check-dict-hash.sh` | Go 渠道校验从 deferred 改为启用（三层校验：源字节 sha256 / DICT_VERSION / sha256_prefix 头部直接比对） |
| `demo/README.md` | 功能描述 + 构建产物 + 技术细节三处更新 CDN URL 为 jsdelivr gh，说明 private/public 条件与离线备用 |
| `demo/MANUAL-CHECKLIST.md` | 验收项 7 词典加载 CDN URL 更新为 jsdelivr gh |
| `docs/plans/post-m2/p3-report.md` | 本报告 |

未改文件：`crates/vane-wasm/src/dict_loader.rs`（cdn_url 调用方传参，已支持任意 URL）、core、SPEC。

## DICT_URL 改动

```js
const DICT_URL =
  "https://cdn.jsdelivr.net/gh/ximing/vane@main/crates/vane-dict-zh/data/dict.bin";
```

- private 仓库期间 jsdelivr 返回 404 → CDN fetch 失败 → 降级 bigram（M2-04 铁律不抛错）。
- 转 public 后自动生效。
- `loadDictSha256()` 仍 fetch 本地 `./pkg/sha256_prefix.bin`（build.sh 拷贝，8 字节，同源 dict.bin → prefix 一致）。
- 链路完整：CDN fetch → `verify_sha256_prefix` → OPFS 缓存（二次零网络）→ 降级 bigram。

## check-dict-hash.sh Go 渠道启用

Go 渠道（`bindings/go/dict/dict.bin.gz`，1.44MB，已提交）校验已启用，三层校验：

1. **源字节 sha256**：`gunzip -c dict.bin.gz | sha256` vs Node `dict.bin` sha256 → `efa4eee3...` 一致（最强校验：字节相同 → prefix 必相同）。
2. **DICT_VERSION**：Go `DictVersion` const vs Rust `DICT_VERSION` → `2026.08` 一致。
3. **sha256_prefix 直接比对**（zstd 可用时）：`gunzip | zstd -d` → 读头部 `[8..16]` → `ae2d123049c4bcb4` = `sha256_prefix.bin` 一致。zstd 不可用时跳过（源字节一致已隐含证明）。

CI ubuntu 兼容：`gunzip`（coreutils）、`sha256sum`（coreutils）、`xxd`（Z1 已预装）、`zstd`（可选，不可用则跳过第 3 层）。

## 自证结果

| 验证项 | 结果 |
|--------|------|
| `bash scripts/check-dict-hash.sh` | 通过（Node + Go 三层校验全绿，EXIT=0） |
| `cargo check --target wasm32-unknown-unknown -p vane-wasm` | 通过（dict_loader 未改） |
| `cargo clippy --target wasm32-unknown-unknown -p vane-wasm -- -D warnings` | 通过（无 warning） |
| demo DICT_URL 链路审查 | CDN fetch → verify_sha256_prefix → OPFS 缓存 → 降级 bigram 链路完整 |

## SPEC 触及

未触及 SPEC。SPEC §12.3 仅规定 "WASM CDN URL fetch → sha256 校验 → OPFS 缓存"，不硬编码具体 CDN URL（URL 是部署决策，由调用方传参）。

## 遗留

- **private 仓库期间 CDN 不生效**：当前仓库为 private，jsdelivr gh 无法访问 → CDN fetch 404 → 降级 bigram。转 public 后自动生效，无需代码改动。
- **本地离线开发**：`build.sh` 仍拷贝 `dict.bin` + `sha256_prefix.bin` 到 `demo/pkg/`，可将 `DICT_URL` 临时改回 `"./pkg/dict.bin"`（同源，prefix 一致）。
