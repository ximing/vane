# Task 13 Report：SPEC v1.4→v1.5 修订

## 状态

✅ 完成。SPEC.md v1.4→v1.5 修订（7 节）+ release.yml:10 cosmetic 全部落地，7/7 验证点通过。

## Commits

| # | SHA | 类型 | 摘要 |
|---|-----|------|------|
| 1 | `5d092f8` | `docs(spec)` | SPEC v1.4→v1.5 修订——M3 四端+四渠道+Web门禁 |
| 2 | `9b8269f` | `chore(release)` | dispatch version hint 0.1.2→0.2.0（Task 12 C1 deferred） |

## 测试摘要

7/7 验证点通过：L1 版本号=v1.5 / §12.2 含 Web npm @vane-rs/web 行 + Node prebuilt 追加「未实现，顺延」/ §12.3 四渠道 + scope @vane-rs + 无 @vane/slim（normative 节清干净）+ WASM npm dictData 行 / §13.2 第 5 项 Web npm 安装门禁 / Changelog 末尾 v1.5 条目 / release.yml:10 description "0.2.0" / git diff 确认只改 docs/SPEC.md + .github/workflows/release.yml。纯文档任务，无 cargo/clippy/test 需要。

## SPEC.md 改动节清单

| 节 | 修订类型 | 改动摘要 |
|---|---------|---------|
| L1 标题 | 版本号 | `SPEC v1.4` → `SPEC v1.5` |
| §12.1 Workspace | 补全 | 补 `crates/vane-dict-zh` [M1]（M1 起漏列）+ `bindings/web` [M3] |
| §12.2 目标矩阵 | 扩展 | 加第六行 Web npm `@vane-rs/web`（wasm-bindgen --target web ESM 双变体 + worker + dict_loader + TS 类型）；Node prebuilt 追加行加「（未实现，顺延）」标注 |
| §12.3 词典分发 | 扩展+修正 | 三渠道→四渠道；Node 通道修正为 cargo path include_bytes 编译期内嵌；删 @vane/slim；scope @vane/dict-zh→@vane-rs/dict-zh；WASM CDN 降级为 fallback；新增 WASM npm dictData 第四渠道；末句四渠道 + check-dict-hash.sh + dict_tests.rs 引用 |
| §12.4 版本与发布 | 扩展 | 三端→四端（crates.io / @vane-rs/node / GitHub Release / @vane-rs/web）；@vane-rs/dict-zh 日历版例外（YYYY.M.0） |
| §13.2 质量门禁 | 扩展+修正 | §13.2-3 scope 修正 + 补 @vane-rs/web 双变体 wasm 各 ≤800KB gzip；§13.2-4 加「（@vane-rs/node）」限定；新增第 5 项 Web npm 安装门禁 [M3] |
| Changelog | 新增 | 末尾追加 v1.5 条目（2026-08-11，五处修订 S1-S5） |

不触碰：§1-§11 / §13.1 / §13.3 / §14 / §15（M3 不碰 core 语义、不变量、性能承诺、工程纪律门禁、里程碑验收）。

## Changelog v1.5 条目确认

条目已追加至 SPEC.md L467，内容含：
- 日期 2026-08-11
- 五处修订 S1-S5（§12.1 补全 / §12.2 四端 / §12.3 四渠道 / §12.4 四端 / §13.2 门禁）
- 决策点 1（Node prebuilt 追加标注，用户拍板方案 B）
- 决策点 2（删 @vane/slim，修正错误）
- R4 scope 修正（@vane/dict-zh→@vane-rs/dict-zh）
- 边界声明（不触碰 §1-§11 / §13.1 / §13.3 / §14 / §15）

## Concerns

1. **文件名漂移（minor）**：SPEC §12.3 末句 + Changelog v1.5 条目引用 `crates/vane-dict-zh/tests/dict_tests.rs`（复数），但实际文件为 `dict_test.rs`（单数）。此引用来自草案 §4，propagated per draft。crates/ 冻结中不可改名；如需修正可改 SPEC 引用为 `dict_test.rs`（单数），或留待后续 task 统一。不影响 Task 13 验收。
2. **spec 前向引用**：§13.2-5 Web npm 安装门禁引用 `examples/vite` + `examples/webpack` build 冒烟——`examples/vite` 已存在，`examples/webpack` 已存在。§12.3 末句引用 `scripts/check-dict-hash.sh`——已存在。仅 `dict_tests.rs` 文件名有上述漂移。
3. **无代码/CI 影响**：纯文档任务，不涉及 Rust 代码、bindings、CI workflow 逻辑（release.yml 仅 description 文本 cosmetic）。无需跑 cargo test / clippy / wasm check。
