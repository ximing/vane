# 07-dict-distribution-node 代码审查

> 审查基线：BASE=b28b1f4 → HEAD（4 commits：11b477c / f56f1ab / 5e58823 / b82fcb4）。
> 审查方式：只读 diff + 代码审查，未运行 cargo（编排者跑门禁）。
> SPEC 引用：§5.2（dict.bin 格式）、§12.3（Node 词典分发）、§13.1（冷加载<150ms）、§13.2-3（≤1.5MB gzip）、§13.2-2 ④（降级不抛错）。

## 逐维度结论

### 1. vane-dict-zh crate ✅

- `crates/vane-dict-zh/src/lib.rs:23`：`pub const DICT_BIN: &[u8] = include_bytes!("../data/dict.bin")`。
- `lib.rs:27`：`pub const DICT_VERSION: &str = "2026.08"`。
- `lib.rs:35-42`：`SHA256_PREFIX_BIN`（include_bytes）+ `sha256_prefix() -> [u8;8]`。
- `Cargo.toml:14`：`[dependencies]` 空 — 运行期零依赖，纯数据 crate。
- 根 `Cargo.toml:2`：workspace members 已含 `crates/vane-dict-zh`。
- **缺失**：计划「涉及文件」列 `crates/vane-dict-zh/README.md`，实际未创建（`ls` 无 README）。轻微偏离，不阻塞。

### 2. dict.bin 完整性 ✅

- **完整 20 万词（非 placeholder）**：`data/source/dict.txt` 349,045 行（jieba-rs 原版词表）；`gen_dict.rs:128-145` 剪枝逻辑 = 全部单字 + top 20 万多字词 → 211,580 词条。报告数字一致。
- **格式合规（§5.2）**：`gen_dict.rs:428-459` `serialize_dict_bin` 写入 `magic(4)="VNDT" | format_version(4 LE)=1 | sha256_prefix(8) | dict_version_len(2 LE) | dict_version | total_freq(8 LE) | dat_len(4 LE) | base[i32] | check[i32] | values[i32] | hmm_blob_len(4 LE) | hmm_blob`。16 字节头（magic+version+sha256_prefix）完整。与 `vane-core/src/tokenizer/jieba/dict.rs:1-17` 文档一致。
- **core 可加载**：`tests/dict_test.rs:26-30` `JiebaDict::load_zstd(DICT_BIN)` 断言 version 一致；`dict_tests.rs:15-20` 同证。
- **HMM 参数**：`gen_dict.rs:316-374` 从 `hmm.json` 构建完整 4 状态发射概率（35,224 条目），非 fixture 占位。
- **zstd magic 校验**：`dict.bin` 首字节 `28 b5 2f fd`（zstd magic），测试 `dict_test.rs:13-16` 断言。

### 3. 体积门禁 ✅（余量极紧，需关注）

- `data/dict.bin` = 1,479,454 bytes（zstd 压缩）。
- **实测 gzip**：`gzip -c -9 dict.bin | wc -c` = **1,477,877 bytes** ≤ 1,500,000 ✅。
- 报告称 1,477,892（差 15 bytes，gzip 版本差异，无影响）。
- `tests/dict_test.rs:42-49` 断言 `gzip_size(DICT_BIN) <= 1_500_000`。
- **⚠️ 余量仅 ~22KB（1.5%）**：任何词条增长或 zstd/gzip 版本变化都可能突破。建议 10-ci-m1 将门禁收紧到 1.45MB 留 buffer，或接受现状但监测。
- `gzip_size` 测试函数（`dict_test.rs:62-74`）shell out 到 `gzip` CLI；gzip 不可用时退化返回 `data.len()`（1.48M 仍 < 1.5M 通过）。CI 环境 gzip 必装，可接受。

### 4. 冷加载 <150ms ✅

- `crates/vane-dict-zh/benches/dict_load.rs`：criterion bench 调 `JiebaDict::load_zstd(DICT_BIN)`，结构正确。
- 报告称 29.7ms — 合理（ruzstd 解压 1.48MB + DAT 数组拷贝 ~21 万词，<30ms 量级可信）。
- 未运行 cargo 验证；编排者 `cargo bench -p vane-dict-zh` 确认。

### 5. Node loadDict + 自动加载 ✅

- `crates/vane-node/src/dict.rs:14-18`：`#[napi] pub fn load_dict() -> napi::Result<Buffer>` 返回 `DICT_BIN` Buffer 副本。
- `dict.rs:21-25`：`#[napi] dict_version() -> String`。
- **自动加载**：`vane-core/src/api/db.rs:39-41` `Db::open` 在 `dict-zh` feature 启用时调 `load_default_jieba_dict()`（`db.rs:174-198`）加载 `vane_dict_zh::DICT_BIN` → `JiebaDict::load_zstd`。
- **collection 注入**：`collection.rs:88-122` `build_collection_tokenizer`：Jieba + dict 可用 → `build_jieba_tokenizer`；否则 `build_tokenizer`（返回 DictUnavailable）。
- `create_new`（`collection.rs:132-149`）+ `run_reindex`（`collection.rs:1045-1062`）共用此逻辑；`CollectionInner.jieba_dict`（`collection.rs:68-69`）存 Arc 副本供 reindex 用。
- **DbInner.jieba_dict**（`db.rs:26-31`）：`#[cfg(feature="jieba")] pub(crate) jieba_dict: Option<Arc<JiebaDict>>` — pub(crate) 扩展，非 M0 冻结破坏。
- **Db::jieba_dict_available()**（`db.rs:155-160`）：新增 pub 方法，cfg-gated，additive 非破坏。

### 6. 降级（§13.2-2 ④）✅

- `crates/vane-node/src/db.rs:60-70` `CollectionTask::compute`：解析 opts 后若 `tokenizer==Jieba && !db.jieba_dict_available()` → 改 `CjkBigram` + `eprintln!` warn（不抛错）。
- `db.rs:185-191`：`load_default_jieba_dict` 加载失败亦 eprintln + None（不抛错）。
- **convert.rs 未改**：降级逻辑放在 `db.rs` CollectionTask 而非计划说的 `convert.rs`。功能等价（parse_collection_opts 仍透传 Jieba，降级在 task 层），可接受。`convert.rs:79-80` 仍解析 `jieba` → `BuiltinTokenizer::Jieba`。
- 测试：`dict_tests.rs:41-47` 验证 `build_tokenizer(Jieba)` 无 dict 返回 `DictUnavailable`；`dict_tests.rs:51-54` 验证 CjkBigram 可用。

### 7. SHA-256 严格性（裁决项）❌ 必须修

**结论：必须修为真 SHA-256。**

**证据**：
- `crates/vane-dict-zh/examples/gen_dict.rs:463-476` `compute_sha256_prefix`：
  ```rust
  use std::collections::hash_map::DefaultHasher;  // SipHash 1-3
  use std::hash::{Hash, Hasher};
  let mut h = DefaultHasher::new();
  // ... hash words + hmm_blob ...
  h.finish().to_le_bytes()
  ```
  注释明承认：「此处用 SipHash（DefaultHasher）作内容指纹前 8 字节。完整 sha256 需 sha2 crate」。

**必须修的理由**：
1. **SPEC §5.2 字面契约**：「头部 16 字节：`magic(4) | format_version(4) | sha256(8 前缀)`」— 字面 "sha256"，字段名 `sha256_prefix`。SipHash 不是 SHA-256。
2. **三渠道一致性（§12.3）**：若 Go（08）或 WASM（M2）用 `crypto/sha256` / WebCrypto `SubtleCrypto.digest("SHA-256")` 独立计算内容指纹，SipHash 前缀与之不匹配 → 发版阻断。
3. **WASM M2「sha256 校验」**（§12.3 表）：CDN fetch 后 sha256 校验，隐含真 SHA-256。
4. **sha2 已是 workspace 依赖**（根 `Cargo.toml:14` `sha2 = "0.10"`）— 加为 `vane-dict-zh` dev-dep 零成本，gen_dict 是 dev 工具不进运行时。
5. **修复范围小**：`gen_dict.rs` 改 ~10 行（`DefaultHasher` → `sha2::Sha256`，对序列化后的 words+hmm 字节算 hash 取前 8 字节），重新生成 `dict.bin` + `sha256_prefix.bin`。

**修复路径**：编入 07-fix（合并前修）或 10-ci-m1（硬门禁：CI 校验 `sha256_prefix` == 真 SHA-256 前 8 字节）。推荐 07-fix — 改动小、sha2 已就绪、避免 10-ci 才发现需重新生成 dict.bin。

**注意**：core `dict.rs:68-70` `sha256_prefix()` 仅返回存储值，不计算 — 所以 core 无需改，只需 gen_dict 改算法 + 重新生成产物。

### 8. 词典永不进 wasm ✅

- `vane-core/Cargo.toml:16-20`：`vane-dict-zh = { path = "../vane-dict-zh", optional = true }`，`dict-zh = ["jieba", "dep:vane-dict-zh"]` — optional dep，默认不启用。
- `vane-node/Cargo.toml:18`：`vane-core = { workspace = true, features = ["dict-zh"] }` — 仅 vane-node 启用。
- wasm32 check（`cargo check --target wasm32-unknown-unknown -p vane-core`）不启用 dict-zh → vane-dict-zh 不被引入。
- core 内 `vane_dict_zh::` 引用仅在 `db.rs:182`（`#[cfg(feature="dict-zh")]` 块内）— wasm32 构建不可达。

### 9. 禁 postinstall ✅

- 纯 `include_bytes!`，无 `build.rs`，无 postinstall 下载脚本。
- dict.bin 编译期嵌入 native binary；npm 侧无网络依赖。

### 10. M0 签名零破坏 ✅

- `CollectionOptions` 未加 dict 实例字段（M0 冻结保持）。
- `DbInner.jieba_dict` pub(crate) cfg-gated — 内部结构扩展，非 pub API。
- `Db::jieba_dict_available()` 新增 pub 方法 — additive，非破坏。
- `CollectionInner.jieba_dict` cfg-gated 新字段 — pub(crate) 内部。
- vane-node pub API：`load_dict` / `dict_version` 新增导出 — additive。

### 11. 三渠道版本 ✅

- `DICT_VERSION = "2026.08"`（YYYY.MM，7 字符）— 测试 `dict_test.rs:20-23` 断言。
- Cargo.toml `version = "2026.8.0"`（semver 化日历版本）。
- Go embed（08，待做）一致才发版 — 10-ci-m1 校验。

### 12. 测试质量 ✅

- **vane-dict-zh tests**（`tests/dict_test.rs`，6 项）：zstd magic / 版本格式 / core 可加载 / sha256 一致性 / gzip 体积 / 词条完整性（「的」「中国」「是」可查）。真实断言，非空壳。
- **vane-node dict_tests**（`dict_tests.rs`，5 项）：Buffer 可加载 / jieba 切分中文 / sha256 一致 / 无词典 DictUnavailable / CjkBigram 降级可用。真实断言。
- **缺失**：JS 侧行为测试（`__tests__/` 未扩展 `loadDict` / 自动加载 / 降级 warn）。报告遗留 #3 承认。Rust 侧已覆盖核心逻辑，JS 侧可在 10-ci 补。

## 其他发现

- **package.json 未声明 `@vane/dict-zh` dependency**：当前 dict.bin 经 `include_bytes` 嵌入 native binary，运行期无 npm 包依赖。报告遗留 #2 承认。架构上满足「禁 postinstall」+「≤1.5MB」，但未落地独立 `@vane/dict-zh` npm 包（SPEC §12.3 字面「主包正式 dependency」）。**需编排者裁决**：本期嵌入方式是否可接受，或需独立 npm 包。
- **source 数据入库**：`data/source/dict.txt`（349k 行）+ `hmm.json`（970KB）提交 git。repo 体积增加 ~1.4MB 文本，但保证 gen_dict 可复现。可接受。
- **gzip 体积余量极紧**（见维度 3）：1,477,877 / 1,500,000 = 98.5%。建议监测。

## 裁决疑点（需编排者定夺）

1. **SHA-256 修复时机**：07-fix（推荐，改动小、sha2 已就绪）还是 10-ci-m1（硬门禁）？
2. **package.json / npm `@vane/dict-zh` 包**：嵌入 binary 是否满足 §12.3，还是必须独立 npm 包？
3. **JS 侧行为测试**：本期补还是 10-ci 补？
4. **gzip 门禁余量**：是否收紧到 1.45MB 留 buffer？

## Verdict

**APPROVED_WITH_MINOR**

阻塞项（合并前或 10-ci 前必须修）：
- **SHA-256 严格性**：`gen_dict.rs:463-476` 用 SipHash 而非真 SHA-256，违反 SPEC §5.2 字面契约 + §12.3 三渠道一致性。sha2 已是 workspace dep，修复成本极低。**必须修为真 SHA-256**（07-fix 或 10-ci-m1 硬门禁，编排者定时机）。

非阻塞建议：
- 缺 README.md（计划列项）。
- gzip 余量紧（98.5%），建议监测。
- JS 侧测试 + package.json 待编排者裁决。
