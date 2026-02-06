# SPEC v1.4→v1.5 修订草案（Task 13）

> 来源：Task 13 只读设计 agent（Plan/opus）产出 + 编排者审查。
> 审查结论：✅ 所有修订准确对齐实现，scope 修正（R4）+ Node prebuilt 标注（R5）+ §12.1 历史遗漏补全都合理。
> **需用户批准（AskUserQuestion 检查点）。**

## 编排者审查结论

草案质量高，7 节修订对照完整，4 个额外矛盾（§12.1 漏列 vane-dict-zh / §12.4 三端未更新 / §13.2-3 scope / §13.2-4 门禁范围）都是合理的顺带修正。决策点 2（删 @vane/slim）是修正错误无需额外确认。决策点 1（Node prebuilt 追加目标）需用户拍板。

## 修订总览

| 节 | 修订类型 | 摘要 |
|---|---|---|
| L1 标题 | 版本号 | v1.4 → v1.5 |
| §12.1 Workspace | 补全 | 补 crates/vane-dict-zh（M1 起漏列）+ bindings/web [M3] |
| §12.2 目标矩阵 | 扩展 | 三端→四端，加 Web npm @vane-rs/web 行；Node prebuilt 追加标注"未实现，顺延" |
| §12.3 词典分发 | 扩展+修正 | 三渠道→四渠道；scope @vane/dict-zh→@vane-rs/dict-zh；删 @vane/slim；Node 通道修正（cargo path 内嵌）；新增 WASM npm dictData 第四渠道 |
| §12.4 版本与发布 | 扩展 | 三端→四端；补 @vane-rs/dict-zh 日历版例外 |
| §13.2 质量门禁 | 扩展+修正 | 加第 5 项 Web npm 安装门禁；§13.2-3 scope 修正；补 @vane-rs/web 双变体体积门禁 |
| Changelog | 新增 | v1.5 条目 |

## 修订对照（完整 7 节）

### 1. L1 标题
v1.4 → v1.5

### 2. §12.1 Workspace（补全）
补两行：
- `crates/vane-dict-zh` — jieba-lite 词典数据 crate（include_bytes + DAT 序列化）[M1]；兼 npm 数据包源 [M3]
- `bindings/web` — @vane-rs/web npm 包源（ESM glue + worker + dict_loader + TS 类型）[M3]

### 3. §12.2 目标矩阵（三端→四端）
新增第六行：
- Web npm @vane-rs/web | wasm-bindgen --target web ESM 双变体（simd/scalar）+ worker + dict_loader + TS 类型；vite/webpack 可 import；wasm ≤800KB gzip；npm publish（非 napi，直接 npm publish --access public） | M3

Node prebuilt 追加行加"（未实现，顺延）"标注（决策点 1）。

### 4. §12.3 词典分发（三渠道→四渠道）
- Node：修正为 `vane-dict-zh` cargo path 依赖 include_bytes 编译期内嵌（非 npm 数据包）；删 @vane/slim
- Go：不变
- WASM CDN [M2]：降级为 fallback（dictData 优先）
- WASM npm dictData [M3]（新）：@vane-rs/dict-zh npm 包 data/dict.bin，Web 端 import → fetch + arrayBuffer → VaneWorker dictData 内联（优先于 CDN）；零强制 CDN
- 末句三渠道→四渠道 + check-dict-hash.sh + dict_tests.rs 引用

### 5. §12.4 版本与发布（三端→四端）
- Node（npm @vane-rs/node）/ Go（GitHub Release .a）/ WASM（GitHub Release .wasm + npm @vane-rs/web）四端版本号严格同步
- @vane-rs/dict-zh 走独立日历版（YYYY.M.0），与库 semver 解耦

### 6. §13.2 质量门禁（加第 5 项）
- §13.2-3 scope 修正 @vane/dict-zh→@vane-rs/dict-zh + 补 @vane-rs/web 双变体 wasm 各 ≤800KB gzip
- §13.2-4 加"（@vane-rs/node）"限定
- 新增第 5 项：Web npm 安装门禁 [M3]——npm i @vane-rs/web @vane-rs/dict-zh 在 vite/webpack 可 import + build（install-matrix 扩展或 examples/vite + examples/webpack build 冒烟）

### 7. Changelog 新增 v1.5 条目
日期 2026-08-11，三处修订（§12.2 四端 + §12.3 四渠道 + §13.2 门禁）+ 附带补全（§12.1 + §12.4）+ 实现参照索引。

## 决策点

### 决策点 1（需用户拍板）：Node prebuilt 追加目标（musl/arm64-win）如何处理
SPEC v1.4 §12.2 第二行"Node prebuilt 追加 | linux-musl ×2、linux-arm64、win-arm64（可选） | M1"，但 M1 已完成，musl/arm64-win 从未实现（release.yml 仅 4 平台）。

- **(A) 保持原样不加标注**：SPEC 是规范非状态追踪，里程碑=M1 可解读为"M1 或以后"
- **(B) 加"（未实现，顺延）"标注（推荐）**：保留规范意图 + 诚实标注现状
- **(C) 删除该行**：放弃 musl/arm64-win 目标

### 决策点 2（修正错误，无需额外确认）：删 @vane/slim
@vane/slim 从未存在（R4），保留会误导。Node 端词典是编译期内嵌，无"无词典变体"概念。删除是修正错误。

## 额外矛盾（顺带修正）
1. §12.1 漏列 vane-dict-zh（M1 起存在，SPEC 历史遗漏）——v1.5 补全
2. §12.4 "三端"未随 M3 更新——重写为四端 + dict-zh 日历版例外
3. §13.2-3 scope @vane/dict-zh（R4 在 §13.2 体现）——同步修正
4. §13.2-4 安装矩阵门禁范围——加"（@vane-rs/node）"限定 + 新增第 5 项覆盖 Web

## 不触碰的边界
- §1-§11 API/数据模型/分词器/存储/查询/FFI/错误码——M3 不碰 core 语义
- §14 不变量 I-1~I-8 不变
- §15 里程碑验收 M0/M1/M2 不变（M3 是 post 里程碑不补入）
- §13.1 性能承诺 + §13.3 工程纪律门禁不变

## 完整修订文本
见 Task 13 设计 agent 产出（本文件是其编排者审查后的落盘版）。实际修订 SPEC.md 时编辑：L1 + §12.1 + §12.2 + §12.3 + §12.4 + §13.2 + Changelog 末尾。
