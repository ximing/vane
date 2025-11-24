# 07-dict-distribution-node：@vane/dict-zh 数据包 + 主包 dependency + 体积门禁

> SPEC 引用：§12.3（词典分发 Node 侧）、§13.2-3（体积门禁 ≤1.5MB gzip）、§5.2（dict.bin 格式）。
> 前置依赖：05-jieba-lite（JiebaDict::load + dict.bin 格式）。
> M1 README 契约：`crates/vane-dict-zh`。

## Goal

`@vane/dict-zh` 平台无关数据包（仅含预编译 dict.bin），作 `@vane-rs/node` 主包正式 dependency（禁 postinstall）。包体 ≤1.5MB gzip（CI 门禁）。vane-node 增加 `loadDict()` API 注入词典。词典独立日历版本化（`2026.08`）。

## Architecture

- **`crates/vane-dict-zh`**：纯数据 crate，`include_bytes!("data/dict.bin")` 暴露 `DICT_BIN: &[u8]` + `DICT_VERSION: &str` + `sha256_prefix()`。无 Rust 逻辑（加载在 core 05）。
- **dict.bin 生成**（离线脚本 `scripts/gen_dict.rs` 或 `crates/vane-dict-zh/build.rs`）：jieba 开源词表剪枝 ~20 万词 → DAT 构建 → zstd 压缩 → 写 `data/dict.bin`。脚本不在 core 运行时；CI 生成或手工提交。
- **`@vane-rs/node` 集成**：`package.json` 声明 `vane-dict-zh`（npm 包名 `@vane/dict-zh`）为 dependency。vane-node 增加 `loadDict(): Buffer` 或直接在 `VaneCollection` 创建时若 `tokenizer:"jieba"` 自动加载。
- **降级**：词典加载失败（包缺失/损坏）→ fallback `CjkBigram` + console.warn（不抛错，SPEC §13.2-2 ④）。

## 涉及文件

- **Create**：
  - `crates/vane-dict-zh/Cargo.toml`
  - `crates/vane-dict-zh/src/lib.rs`（DICT_BIN/DICT_VERSION/sha256_prefix）
  - `crates/vane-dict-zh/data/dict.bin`（生成产物，~1.5MB gzip）
  - `crates/vane-dict-zh/build.rs`（或 `scripts/gen_dict.rs` 离线生成）
  - `crates/vane-dict-zh/README.md`（版本/格式说明）
- **Modify**：
  - `Cargo.toml`（workspace members 增 `crates/vane-dict-zh`）
  - `crates/vane-node/Cargo.toml`（增 `vane-dict-zh` dependency）
  - `crates/vane-node/src/lib.rs`（增 `loadDict` 导出）
  - `crates/vane-node/src/db.rs` 或 `collection.rs`（collection 创建时若 tokenizer=jieba 自动加载词典）
- **Test**：
  - `crates/vane-dict-zh/tests/dict_test.rs`
  - `crates/vane-node/src/dict_tests.rs`

## Interfaces

### Consumes from 05-jieba-lite

```rust
pub struct JiebaDict { ... }
impl JiebaDict {
    pub fn load(bytes: &[u8]) -> Result<Self>;
    pub fn version(&self) -> &str;
    pub fn sha256_prefix(&self) -> [u8; 8];
}
pub fn build_jieba_tokenizer(dict: Arc<JiebaDict>, user_dict: &[UserDictEntry]) -> Result<Box<dyn Tokenizer>>;
```

### Produces（见 README § 07 契约）

## TDD 任务清单

### Task 1：vane-dict-zh crate 骨架 + 测试词典

**测试**（`crates/vane-dict-zh/tests/dict_test.rs`）：
```rust
use vane_dict_zh::{DICT_BIN, DICT_VERSION, sha256_prefix};

#[test]
fn dict_bin_non_empty() {
    assert!(!DICT_BIN.is_empty());
    // magic 校验
    assert_eq!(&DICT_BIN[0..4], b"VANE");
}

#[test]
fn dict_version_is_calendar_format() {
    assert!(DICT_VERSION.starts_with("20"));
    assert_eq!(DICT_VERSION.len(), 7);  // YYYY.MM
}

#[test]
fn dict_loadable_by_core() {
    let dict = vane_core::tokenizer::jieba::JiebaDict::load(DICT_BIN).expect("core must load bundled dict");
    assert_eq!(dict.version(), DICT_VERSION);
}
```
验证失败：crate 不存在。
最小实现：`crates/vane-dict-zh/Cargo.toml`（`name = "vane-dict-zh"`，`[lib]`，依赖 `vane-core` 仅 dev）；`src/lib.rs`：`pub const DICT_BIN: &[u8] = include_bytes!("data/dict.bin"); pub const DICT_VERSION: &str = "2026.08"; pub fn sha256_prefix() -> [u8;8] { /* 从 DICT_BIN 头解析 */ }`。`data/dict.bin` 先用 05 计划的测试 fixture（小规模），完整词典在 Task 4 生成。
commit：`dict-zh: add crate skeleton with test fixture dict.bin`。

### Task 2：vane-node loadDict API + 自动加载

**测试**（`crates/vane-node/src/dict_tests.rs`）：
```rust
use napi_derive::napi;

#[napi]
pub fn load_dict() -> napi::Result<Buffer> {
    Ok(Buffer::from(vane_dict_zh::DICT_BIN))
}

#[test]
fn jieba_collection_auto_loads_dict() {
    // collection 创建时 tokenizer=jieba，自动加载 @vane/dict-zh
    // （集成测试在 JS 侧跑，Rust 侧验证 build_jieba_tokenizer 可用）
    let dict = std::sync::Arc::new(vane_core::tokenizer::jieba::JiebaDict::load(vane_dict_zh::DICT_BIN).unwrap());
    let tok = vane_core::tokenizer::build_jieba_tokenizer(dict, &[]).unwrap();
    assert!(tok.tokenize("我爱北京").len() >= 1);
}
```
最小实现：vane-node `src/lib.rs` 增 `pub mod dict;` + `#[napi] fn load_dict() -> Buffer`；`collection.rs` 的 `VaneCollection` 创建（db.rs CollectionTask）若 `opts.tokenizer=="jieba"` → 加载 `vane_dict_zh::DICT_BIN` → `JiebaDict::load` → 传给 CollectionInner。**core CollectionOptions 不含 dict 实例**（M0 冻结）——**裁决**：CollectionInner 构造时若 `tokenizer_kind==Jieba` 且有全局词典注入（通过 DbInner 持 `Option<Arc<JiebaDict>>`），则 build_jieba_tokenizer；否则 DictUnavailable。DbInner 增 `pub(crate) jieba_dict: Option<Arc<JiebaDict>>`（扩展，非 M0 冻结破坏——DbInner 是 pub(crate) 内部结构）。Db::open 时若 feature 启用则默认加载 vane-dict-zh。
commit：`node: add loadDict API and auto-load jieba dict in collection`。

### Task 3：缺词典降级 bigram + warn（不抛错）

**测试**（`crates/vane-node/src/dict_tests.rs`）：
```rust
#[test]
fn jieba_dict_missing_falls_back_to_bigram() {
    // 模拟词典加载失败：build_tokenizer(Jieba) 无 dict → DictUnavailable
    // → 绑定层 catch → fallback CjkBigram + console.warn
    // Rust 侧验证：build_tokenizer(Jieba) 返回 DictUnavailable（无 dict 注入）
    let r = vane_core::tokenizer::build_tokenizer(vane_core::tokenizer::BuiltinTokenizer::Jieba, &[]);
    assert!(matches!(r, Err(vane_core::types::VaneError::DictUnavailable)));
    // 绑定层 convert：解析 collection opts 时若 tokenizer=jieba 且 dict 不可用
    // → 自动改用 CjkBigram + 记录 warn
    // （JS 侧测试验证 console.warn 调用）
}
```
最小实现：vane-node `convert.rs` parse_collection_opts：若 `tokenizer=="jieba"` 且 DbInner.jieba_dict=None → 改 `BuiltinTokenizer::CjkBigram` + `eprintln!("[vane] jieba dict unavailable, falling back to cjk_bigram")`（console.warn 在 JS 侧通过 napi env，M1 先 eprintln，M2 浏览器侧 console.warn）。
commit：`node: fallback to cjk_bigram with warning when jieba dict missing`。

### Task 4：完整词典生成 + 体积门禁

**测试**（`scripts/gen_dict.rs` 或 build.rs）：
```rust
// scripts/gen_dict.rs：从 jieba 开源词表剪枝生成 dict.bin
// 输入：jieba dict.txt（~350k 词）
// 剪枝：保留词频 top 20 万 + 全部单字 + 词频
// 输出：crates/vane-dict-zh/data/dict.bin（DAT + zstd）

#[test]
fn dict_bin_size_under_1_5mb_gzip() {
    let gzip_size = gzip_size(vane_dict_zh::DICT_BIN);
    assert!(gzip_size <= 1_500_000, "dict.bin gzip {} > 1.5MB", gzip_size);
}
```
最小实现：`scripts/gen_dict.rs`：读 jieba 词表 → 排序剪枝 top 20 万 → 构建 DAT（复用 05 的 DAT 代码或独立）→ 序列化（magic+version+sha256+words+dat+hmm）→ zstd 压缩 → 写 `data/dict.bin`。HMM 参数从 jieba 原版常量提取（转移矩阵 4x4 + 发射概率，压缩 ~200KB）。CI 门禁：`gzip -c data/dict.bin | wc -c` ≤1.5MB（10-ci-m1 跑）。
commit：`dict-zh: generate full 200k-word dict with zstd compression`。

### Task 5：词典冷加载 <150ms

**测试**（`crates/vane-dict-zh/benches/dict_load.rs`）：
```rust
use criterion::{criterion_group, criterion_main, Criterion};
use vane_dict_zh::DICT_BIN;

fn bench_load(c: &mut Criterion) {
    c.bench_function("dict_load", |b| {
        b.iter(|| {
            vane_core::tokenizer::jieba::JiebaDict::load(DICT_BIN).unwrap();
        });
    });
}
criterion_group!(benches, bench_load);
criterion_main!(benches);
```
验收：`cargo bench -p vane-dict-zh` 平均 <150ms（SPEC §13.1）。若超 → 优化 DAT 反序列化（零拷贝切片引用）。
commit：`dict-zh: add cold-load bench (<150ms)`。

## 验收标准

- **SPEC §12.3**：`@vane/dict-zh` 平台无关数据包，主包正式 dependency，禁 postinstall；≤1.5MB gzip。
- **SPEC §13.2-3**：CI 门禁 ≤1.5MB gzip（Task 4）。
- **SPEC §13.1**：词典冷加载 <150ms（Task 5）。
- **SPEC §13.2-2 ④**：缺词典自动降级 bigram + warn 不抛错（Task 3）。
- **SPEC §5.2**：dict.bin 格式 = zstd 压缩 DAT + HMM + 16 字节头。
- **三渠道版本一致**：`DICT_VERSION` 与 Go embed（08）一致才发版（10-ci-m1 校验）。

## 前置依赖

- 05-jieba-lite（JiebaDict::load + dict.bin 格式）。

## Global Constraints

词典永不进 wasm（vane-dict-zh 是独立 crate，wasm32 构建不依赖它）；禁 postinstall（pnpm/企业断网友好）；zstd 用 ruzstd（纯 Rust，非黑名单）；词典版本日历化（YYYY.MM），与库 semver 解耦。
