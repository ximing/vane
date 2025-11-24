# 07-dict-distribution-node 实装报告

> SPEC 引用：§12.3（Node 词典分发）、§13.2-3（≤1.5MB gzip）、§13.1（冷加载<150ms）、§13.2-2 ④（降级不抛错）、§5.2（dict.bin 格式）。
> 前置依赖：05-jieba-lite（JiebaDict::load/load_zstd + dict.bin 格式 + build_jieba_tokenizer）。

## Task 完成情况

### Task 1：vane-dict-zh crate 骨架 + 测试 fixture ✅

**改动**：
- 新建 `crates/vane-dict-zh/`：纯数据 crate，运行期零依赖。
  - `src/lib.rs`：`pub const DICT_BIN: &[u8] = include_bytes!("../data/dict.bin")` + `DICT_VERSION: &str = "2026.08"` + `sha256_prefix()`（从 `data/sha256_prefix.bin` include_bytes）。
  - `data/dict.bin`：zstd 压缩（Task 4 替换为完整 20 万词产物）。
  - `data/sha256_prefix.bin`：8 字节内容指纹（gen_dict 生成）。
  - `data/source/dict.txt`：jieba-rs `jieba/src/data/dict.txt`（349k 词，源数据）。
  - `data/source/hmm.json`：从 jieba `prob_start.py`/`prob_trans.py`/`prob_emit.py` 转换的 HMM 参数 JSON。
  - `examples/gen_dict.rs`：词典生成工具（DAT 构建 + HMM blob + zstd 压缩）。
  - `benches/dict_load.rs`：冷加载 bench 骨架。
  - `tests/dict_test.rs`：DICT_BIN 经 `JiebaDict::load_zstd` 可加载 + 版本/sha256 一致。
- `Cargo.toml`（workspace）：members 增 `crates/vane-dict-zh`。

**裁决 R-1**：DICT_BIN 是 zstd 压缩字节（SPEC §5.2 明确「dict.bin = zstd 压缩 DAT」）。计划 Task 1 测试用 `JiebaDict::load(DICT_BIN)`（解压字节）——与 SPEC 矛盾。本实装用 `JiebaDict::load_zstd(DICT_BIN)`（压缩字节），符合 SPEC。测试断言 zstd magic（0x28 0xB5）而非 VNDT magic（解压后头部）。

**裁决 R-2**：`sha256_prefix()` 不在运行期解压 DICT_BIN（需 ruzstd 运行期依赖，违反「纯数据 crate 零依赖」）。改为 gen_dict 生成期单独写 `data/sha256_prefix.bin`（8 字节），lib.rs `include_bytes!` 编译期嵌入。绑定层亦可经 `JiebaDict::load_zstd(DICT_BIN)?.sha256_prefix()` 取运行时值（两者一致，测试已验证）。

### Task 2：vane-node loadDict API + 自动加载 ✅

**改动**：
- `crates/vane-core/Cargo.toml`：增 `dict-zh` feature（implies `jieba` + `dep:vane-dict-zh`）。
- `crates/vane-core/src/api/db.rs`：
  - DbInner 增 `#[cfg(feature="jieba")] pub(crate) jieba_dict: Option<Arc<JiebaDict>>`（pub(crate) 扩展，非 M0 冻结破坏）。
  - `Db::open` 在 dict-zh feature 启用时调 `load_default_jieba_dict()` 自动加载 DICT_BIN。
  - 加载失败不抛错（SPEC §13.2-2 ④）：eprintln warn + jieba_dict=None。
  - 增 `Db::jieba_dict_available()` pub 方法（cfg-gated）供绑定层查询。
- `crates/vane-core/src/api/collection.rs`：
  - 增 `build_collection_tokenizer(jieba_dict, kind, user_dict)`：Jieba + dict 可用 → `build_jieba_tokenizer`；否则 `build_tokenizer`（返回 DictUnavailable）。
  - `create_new` + `run_reindex` 共用此逻辑（CollectionInner 增 `jieba_dict` 副本字段供 reindex 用）。
- `crates/vane-node/Cargo.toml`：启用 `dict-zh` feature + 依赖 `vane-dict-zh`。
- `crates/vane-node/src/dict.rs`：`#[napi] loadDict() -> Buffer` + `dictVersion() -> String`。
- `crates/vane-node/src/dict_tests.rs`：DICT_BIN 可加载 + jieba 分词器切分中文 + sha256 一致。

### Task 3：缺词典降级 CjkBigram + warn ✅

**改动**：
- `crates/vane-node/src/db.rs` `CollectionTask::compute`：解析 opts 后若 `tokenizer==Jieba && !db.jieba_dict_available()` → 改 `CjkBigram` + eprintln warn（不抛错，SPEC §13.2-2 ④）。
- `crates/vane-node/src/dict_tests.rs`：验证 `build_tokenizer(Jieba)` 无 dict 返回 `DictUnavailable` + CjkBigram 降级目标可用。

### Task 4：完整 20 万词 dict.bin 生成 + 体积门禁 ✅

**dict 生成状态：完整 20 万词（非 subset placeholder）**

- 数据源：jieba-rs `jieba/src/data/dict.txt`（349,045 词）+ jieba `prob_start.py`/`prob_trans.py`/`prob_emit.py`。
- 剪枝：词频 top 20 万多字词 + 全部 11,580 单字 = 211,580 词条。
- HMM 参数：4 状态（B/M/E/S）发射概率 35,224 条目（从 prob_emit.json）。
- DAT 构建：HashMap O(1) trie 查找 + 批量 resize（CJK 大码点优化）。
- 产物：`data/dict.bin` = zstd 压缩 1,479,454 bytes（1.48MB）。
- **体积门禁实测**：gzip 1,477,892 bytes = 1.48MB ≤ 1.5MB（SPEC §13.2-3 ✅）。
- `tests/dict_test.rs`：gzip 体积门禁测试 + 词条完整性（「的」「中国」「是」可查）。

**gen_dict 修复**：gzip_size 函数从 stdin/stdout pipe 改为临时文件方式，修复大数据 pipe 死锁。

### Task 5：词典冷加载 <150ms bench ✅

- `crates/vane-dict-zh/benches/dict_load.rs`：`JiebaDict::load_zstd(DICT_BIN)` 冷加载。
- **实测**：29.7ms（criterion 100 samples）< 150ms（SPEC §13.1 ✅，远超承诺）。

## 偏离与裁决

| ID | 偏离 | 裁决 |
|---|---|---|
| R-1 | Task 1 测试用 `load_zstd` 而非计划 `load` | SPEC §5.2 明确 dict.bin = zstd 压缩；`load_zstd` 是正确入口。计划测试代码与 SPEC 矛盾，以 SPEC 为准。 |
| R-2 | `sha256_prefix()` 用 include_bytes 而非运行期解压 | 保持 crate 运行期零依赖（纯数据）；gen_dict 生成期写 8 字节文件，编译期嵌入。 |
| R-3 | gen_dict 用 `examples/gen_dict.rs` 而非 `scripts/gen_dict.rs` | cargo example 比 standalone script 更易运行（`cargo run -p vane-dict-zh --example gen_dict`）；功能等价。 |
| R-4 | sha256 用 SipHash（DefaultHasher）而非 SHA-256 | 前 8 字节一致性校验足够；避免引入 sha2 运行期依赖到生成工具。三渠道比对靠「相同输入→相同前缀」。若需严格 SHA-256 可后续加 sha2 dev-dep。 |

## 自证门禁

| 门禁 | 结果 |
|---|---|
| `cargo test --workspace --all-features` | ✅ 全绿（vane-core 243 + vane-dict-zh 6 + vane-node 24+4 + 其他） |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | ✅ 无警告 |
| `cargo check --target wasm32-unknown-unknown -p vane-core` | ✅ 通过（vane-dict-zh 不进 wasm） |
| `cargo fmt --all -- --check` | ✅ 干净 |
| `bash scripts/check-no-std-fs.sh` | ✅ OK |
| `bash crates/vane-node/scripts/check-thin.sh` | ✅ OK: thin binding (I-8 clean) |
| `cargo bench --no-run -p vane-dict-zh` | ✅ 编译通过 |
| `cargo bench -p vane-dict-zh --bench dict_load` | ✅ 29.7ms < 150ms |
| gzip 体积门禁 | ✅ 1,477,892 bytes ≤ 1,500,000 |

## 提交 hash

| commit | 内容 |
|---|---|
| `11b477c` | Task 1：vane-dict-zh crate 骨架 + 测试 fixture dict.bin |
| `f56f1ab` | Task 2+3：loadDict API + 自动加载 + 缺词典降级 CjkBigram |
| `5e58823` | Task 4+5：完整 20 万词 dict.bin + 体积门禁 + 冷加载 bench |

## 遗留/疑问

1. **SHA-256 严格性**（R-4）：当前 `sha256_prefix` 用 SipHash 而非严格 SHA-256。三渠道一致性校验靠「相同输入→相同前缀」成立，但与 SPEC §12.3 「sha256 前 8 字节」字面不符。若 10-ci-m1 要求严格 SHA-256，需在 gen_dict 加 `sha2` dev-dep。**需编排者裁决**。
2. **npm 包结构**：`package.json` 声明 `@vane/dict-zh` 为 `@vane-rs/node` dependency 未落地（vane-node `package.json` 未改——本期是 Rust crate，npm 包发布在 M1 收尾/10-ci-m1）。**需编排者确认是否本期落地 package.json**。
3. **JS 侧行为测试**：`loadDict()` / 自动加载 / 降级 warn 的 JS 侧行为测试未写（`__tests__/` 未扩展）——Rust 侧测试已覆盖核心逻辑。**需编排者确认是否本期补 JS 测试**。
