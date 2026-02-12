# M4 阶段一 a — cargo-fuzz 集成 review

> Task reviewer 产出（opus，只读，禁编辑源码）。
> 审查对象：commits 0458942..d4a94d8（10 files +667 -3），`feat/m4-prod-readiness` 分支。
> 输入：`docs/plans/m4/phase0-design.md` §3.2（spec）+ `task-cargo-fuzz-report.md`（implementer report）+ `task-cargo-fuzz-review-package.md`（diff）。
> Reviewer 独立 grep 了 vane-core API 签名、TOPK_MAX、cosine_score 零向量处理、deny.toml licenses allow 列表，复核 implementer 自报。

## A. Spec 合规

### A.1 crate 结构（§3.2 Cargo.toml 模板）✅

`crates/vane-fuzz/Cargo.toml` 逐字段核对：

| 字段 | spec §3.2 | 实现 | 判定 |
|---|---|---|---|
| `name = "vane-fuzz"` | ✓ | ✓ | ✅ |
| `version = "0.0.0"` | ✓ | ✓ | ✅ |
| `edition = "2021"` | ✓ | ✓ | ✅ |
| `publish = false` | ✓ | ✓ | ✅ |
| `vane-core = { path = "../vane-core" }` | ✓ | ✓ | ✅ |
| `libfuzzer-sys = "0.4"` | ✓ | ✓ | ✅ |
| 5 `[[bin]]` targets | ✓ | brute/hnsw/persist_roundtrip/merge/dict_load | ✅ |
| `license = "Apache-2.0"` | 未列 | 增补（cargo-deny 要求，§4c report） | ✅ 必要 |
| `[package.metadata] cargo-fuzz = true` | 未列 | 增补（cargo-fuzz 0.13+ 检测，§4b report） | ✅ 必要 |

**增补判定**：`license` 和 `cargo-fuzz = true` 是 spec 未预见但实现必需的增补（cargo-deny 报 unlicensed / cargo-fuzz 报 "not a cargo-fuzz manifest"），不改 spec 语义，合理。

**`arbitrary` 未直接引**：spec §3.2 Cargo.toml 只列 libfuzzer-sys，未列 arbitrary。implementer 自研 ByteCursor（~70 行）替代 arbitrary。`arbitrary v1.4.2` 仅作 libfuzzer-sys 传递依赖进 Cargo.lock（不直接 dep）。合理取舍（少一份 deny 风险）。

### A.2 workspace Cargo.toml ✅

- `members` 加 `"crates/vane-fuzz"` ✅
- `default-members` 排除 vane-fuzz（双保险：`cargo build`/`cargo test` 默认不含 fuzz）✅
- 注释清晰说明 `--workspace` 仍需 `--exclude` ✅

### A.3 CI `.github/workflows/ci.yml` ✅

- **test job**（line 61）：`cargo test --workspace --all-features --exclude vane-fuzz` ✅（spec §3.2 取舍段原文）
- **clippy job**（line 48）：`cargo clippy --all-targets --all-features --workspace --exclude vane-fuzz -- -D warnings` ✅
  - 增补 `--workspace`：spec §3.2 取舍段只说 test job 加 `--exclude`，未提 clippy job 需 `--workspace`。但 `--exclude` 须配 `--workspace`（cargo 报错），implementer 增补合理（§4d report），default-members 双保险。
- 不动其他 14 jobs ✅
- fuzz-smoke/fuzz-long 新 job 是 Phase 6（非本任务）✅

### A.4 5 fuzz targets 覆盖 §3.2 target 表 ✅

| spec §3.2 target | 实现 | 不变量覆盖 | 判定 |
|---|---|---|---|
| brute_search_fuzz | `brute_search_fuzz.rs` | search_brute_baseline 不 panic + topK≤top_k + score 非 NaN | ✅ |
| hnsw_search_fuzz | `hnsw_search_fuzz.rs` | search 不 panic + topK + score 非 NaN + id∈已知集 + brute 基线对照；不做严格 recall（可接受，proptest §3.3 覆盖） | ✅ |
| persist_roundtrip_fuzz | `persist_roundtrip_fuzz.rs` | open→add→flush→search→close→reopen→search + id∈原集 + 前后 id 集一致 | ✅（见 Minor #2 vacuous pass 风险） |
| merge_fuzz | `merge_fuzz.rs` | 多段→delete→compact→search + tombstoned 不可见 + 无 phantom + 无重复 + live 全可见 | ✅（见 Minor #3 HNSW/unwrap 风险） |
| dict_load_fuzz | `dict_load_fuzz.rs` | Jieba→不 panic + CjkBigram→Ok + Standard→Ok + set_user_dict→不 panic | ✅（见 Minor #4 未断言 Err） |

## B. 代码质量

### B.1 API 签名正确性（reviewer 独立 grep 复核）

implementer 无法编译 targets（cfg(fuzzing) 门控 → E0432），API 误用风险被宏不展开掩盖。reviewer 逐个 grep 确认全部签名：

| 调用 | 源文件:行 | reviewer 确认 |
|---|---|---|
| `Db::open(vfs, path, opts)` | db.rs:41 `pub fn open(vfs: Arc<dyn Vfs>, path: &str, opts: OpenOptions) -> Result<Self>` | ✅ |
| `Db::close(&self)` | db.rs:178 `pub fn close(&self) -> Result<()>`（`&self` 非 `self`，reopen 后可再调） | ✅ |
| `db.collection(name, schema, opts)` | db.rs:98 `pub fn collection(&self, name: &str, schema: Schema, opts: CollectionOptions) -> Result<Collection>` | ✅ |
| `col.add(&[Doc])` | collection.rs:253 `pub fn add(&self, docs: &[Doc]) -> Result<AddReport>` | ✅ |
| `col.flush()` | collection.rs:303 `pub fn flush(&self) -> Result<()>` | ✅ |
| `col.search(&query)` | collection.rs:637 `pub fn search(&self, query: &SearchQuery) -> Result<Vec<Hit>>` | ✅ |
| `col.search_brute_baseline(&query)` | collection.rs:648 `#[doc(hidden)] pub fn search_brute_baseline(&self, query: &SearchQuery) -> Result<Vec<Hit>>`（doc-hidden 仍 pub） | ✅ |
| `col.delete(&[String])` | collection.rs:1019 `pub fn delete(&self, ids: &[String]) -> Result<u64>` | ✅ |
| `col.compact()` | collection.rs:1076 `pub fn compact(&self) -> Result<()>` | ✅ |
| `col.set_user_dict(&[UserDictEntry])` | collection.rs:1129 `pub fn set_user_dict(&self, dict: &[UserDictEntry]) -> Result<()>` | ✅ |
| `build_tokenizer(kind, &dict)` | tokenizer/mod.rs:74 `pub fn build_tokenizer(kind: BuiltinTokenizer, user_dict: &[UserDictEntry]) -> Result<Box<dyn Tokenizer>>` | ✅ |
| `BuiltinTokenizer::Jieba` 非 cfg-gated | tokenizer/mod.rs:30-33 `pub enum BuiltinTokenizer { Standard, CjkBigram, Jieba }`（无 `#[cfg(feature="jieba")]`） | ✅ |
| `build_tokenizer(Jieba, ..)` 返 Err | tokenizer/mod.rs:87 `BuiltinTokenizer::Jieba => Err(VaneError::DictUnavailable)`（无 jieba feature 时硬编码） | ✅ |
| `UserDictEntry::Word(String)` / `WordWithFreq { term, freq }` | tokenizer/mod.rs:41-42 | ✅ |
| `Doc { id, text, vector, meta }` | types.rs:114-119 字段名/类型全匹配 | ✅ |
| `SearchQuery { text, vector, top_k, mode, fusion, filter, candidate_multiplier }` | types.rs:79-87 全字段名匹配 | ✅ |
| `Hit { id: String, score: f32, fields }` | types.rs:103-107 `h.id`/`h.score` 直接访问 f32 | ✅ |
| `FieldDef::Text` / `Vector { dim: u32, metric: Metric }` | types.rs:184-185 | ✅ |
| `Schema::new(Vec<(String, FieldDef)>) -> Result<Self>` | types.rs:196 | ✅ |
| `Metric::Cosine` | types.rs:119 | ✅ |
| `FusionSpec::Rrf` | types.rs:54 | ✅ |
| `OpenOptions::default()` / `CollectionOptions::default()` | types.rs:17,33 impl Default | ✅ |
| `MemoryVfs::new()` | vfs/memory.rs:15 | ✅ |
| `TOPK_MAX = 1000` | types.rs:6 `pub const TOPK_MAX: u32 = 1000` | ✅ merge_fuzz `top_k: 1000` 过 `> TOPK_MAX`（1000>1000=false） |

**结论**：全部 API 签名匹配，无 API 误用。

### B.2 ByteCursor decode 健壮性 ✅

`common.rs` ByteCursor 逐方法审：

| 方法 | panic 风险 | 判定 |
|---|---|---|
| `u8()` | `get`+`unwrap_or(&0)`+`saturating_add` | ✅ 不 panic |
| `u32_le()` | 同上 | ✅ |
| `small_string()` | `String::from_utf8_lossy`（lossy 替换，不 panic） | ✅ |
| `f32_vec(n)` | NaN/Inf 过滤→0.0 | ✅ |
| `bool()` | `u8() & 1` | ✅ |
| `build_schema()` | `Schema::new(fields).expect(...)` — 可 panic，但 schema 始终含 1 Vector 字段（dim 1..=16），必合法 | ✅ 合理（fuzz infra bug 非 fuzz input） |

**结论**：decoder 在任意字节输入下不 panic。

### B.3 不变量 exercise 审查（vacuous 判定）

逐 target 判定是否真正 exercise 不变量（非 vacuous）：

1. **brute_search_fuzz** ✅ NON-VACUOUS
   - 调 `search_brute_baseline` + 断言 `hits.len() ≤ top_k` + 断言每 `hit.score` 非 NaN
   - Err 时 `return`（不检不变量）——可接受：Err 是合法非 panic 结局

2. **hnsw_search_fuzz** ✅ NON-VACUOUS
   - 调 `search`（HNSW）+ 断言 topK/score/id∈已知集 + 跑 brute 基线同断言
   - 不做严格 recall（文档说明合理，HNSW 近似，proptest 覆盖 recall）

3. **persist_roundtrip_fuzz** ⚠️ 低风险 vacuous pass（见 Minor #2）
   - `unwrap_or_default()` 将 search Err 转 empty → 若前后均 Err，round-trip 断言 trivially pass
   - 但实际：valid docs+query → search 应 Ok，风险低

4. **merge_fuzz** ✅ NON-VACUOUS（见 Minor #3）
   - 4 个不变量断言（tombstoned 不可见/无 phantom/无重复/live 全可见）
   - `unwrap_or_default()` + HNSW 路径有假绿风险（≤20 docs 实际极低）

5. **dict_load_fuzz** ✅ NON-VACUOUS（见 Minor #4）
   - 断言 CjkBigram/Standard 必须 Ok + Jieba/set_user_dict 不 panic

**无 vacuous harness**（无 Critical）。

### B.4 cosine_score 零向量安全性 ✅

reviewer 独立确认 `cosine_score`（vector/mod.rs:92-95）零向量返 0.0（非 NaN），有专门测试 `cosine_zero_vector_returns_zero`（mod.rs:456-462）。`brute_search` 进一步将非有限 score 映射为 `Keyf32(NEG_INFINITY)`（mod.rs:328-332）。fuzz targets 的 `!h.score.is_nan()` 断言安全。

## C. Findings

### Critical

无。

### Important

**I-1. NCSA license 阻断 CI deny job**
- `deny.toml` `[licenses] allow` 列表（reviewer 确认：Apache-2.0/MIT/MIT-0/BSD-2-Clause/BSD-3-Clause/ISC/Unicode-DFS-2016/Unicode-3.0/Zlib/CC0-1.0，不含 NCSA）
- libfuzzer-sys v0.4.13 license = `(MIT OR Apache-2.0) AND NCSA`，NCSA 不在 allow → cargo-deny 报 `error[rejected]: failed to satisfy license requirements`
- **失败场景**：CI deny job 在每次 push/PR 时 FAIL（非 flaky，确定性失败）
- **fix**：`deny.toml` `[licenses] allow` 加 `"NCSA"`（1 行，trivial，OSI+FSF approved，不改 ban 语义）
- **定性**：Important，须 fix。fix trivial 安全（仅加 license allow，不碰 bans 黑名单）。但 deny.toml 不在 implementer commit scope 内，须编排者批准。

### Minor

**M-1. persist_roundtrip_fuzz vacuous pass 风险**
- `crates/vane-fuzz/fuzz_targets/persist_roundtrip_fuzz.rs:802,831` | `col.search(&query).unwrap_or_default()` 将 search Err 转 empty Vec，若前后均 Err 则 round-trip id 集断言 trivially pass
- **假绿场景**：reopen 后数据损坏导致 search 返 Err，baseline 也 Err（或 add/flush 失败 → 无数据 → 前后均空），round-trip 断言 pass 但未真正验证数据持久化
- **实际风险**：低。valid docs（唯一 id + 正确 dim 向量）+ valid query（正确 dim + finite 值）→ search 应 Ok。`col.add`/`col.flush` 用 `let _ =` 忽略 Err，但 valid 输入下应成功
- **建议**（非阻断）：加 `assert!(baseline_id_set.len() > 0 || n_docs == 0, "baseline empty despite docs")` 或用 `expect` 替 `unwrap_or_default`

**M-2. merge_fuzz 用 HNSW search 验"live 全可见"+ `unwrap_or_default`**
- `crates/vane-fuzz/fuzz_targets/merge_fuzz.rs:699` | `col.search(&query).unwrap_or_default()` 用 HNSW 路径（近似）+ Err→empty，"live 全可见"断言（`hit_ids.len() == live_ids.len()`）可能假绿/假红
- **假红场景**：HNSW recall < 100% 漏返某 live doc → 断言"live missing"crash（非真数据丢失）；search 返 Err → empty hits → 断言"count mismatch"crash（非真数据丢失）
- **实际风险**：极低。≤20 docs + ef≈3000（top_k=1000 × candidate_multiplier=3）→ HNSW 遍历全图。但 `search_brute_baseline`（保 100% recall）更稳健
- **建议**（非阻断）：用 `search_brute_baseline` 替 `search` 验"live 全可见"不变量

**M-3. dict_load_fuzz 未断言 Jieba 返 Err**
- `crates/vane-fuzz/fuzz_targets/dict_load_fuzz.rs:483-484` | `let jieba_tok = build_tokenizer(Jieba, &user_dict); drop(jieba_tok);` 接受 Ok/Err，未断言 Err
- **假绿场景**：若 `build_tokenizer(Jieba, ..)` 回归为 Ok（如 feature unification 误启 jieba），fuzz 不捕获
- **实际风险**：极低。无 jieba feature 时 `BuiltinTokenizer::Jieba => Err(DictUnavailable)` 硬编码（tokenizer/mod.rs:87），不会变
- **建议**（非阻断）：加 `assert!(jieba_tok.is_err(), "Jieba without dict must Err")`

### Non-findings（reviewer 确认 OK）

- **Cargo.lock 入 commit**：项目追踪 Cargo.lock，新增 crate 的 lockfile 更新是直接后果，不入 commit 留仓库状态不一致。合理。
- **CI clippy job 增 `--workspace`**：`--exclude` 须配 `--workspace`（cargo 报错），default-members 双保险。合理。
- **vane-fuzz 不在 wasm32-check**：`-p vane-core`/`-p vane-wasm` 不含 vane-fuzz，无 wasm 污染。
- **libfuzzer-sys + arbitrary 不触 bans 黑名单**：deny `bans ok` 确认（reviewer 复核传递依赖链：libfuzzer-sys → arbitrary + cc，无 regex/tokio/prost/tonic/openssl/lindera/ndarray/wee_alloc/dashmap/parking_lot）。

## D. 已知 concerns 定性

### D.1 NCSA license → **Important 须 fix（trivial）**

libfuzzer-sys 的 NCSA license 不在 deny.toml allow 列表 → CI deny job 确定性 FAIL。

- **fix**：deny.toml `[licenses] allow` 加 `"NCSA"`（1 行）
- **安全性**：NCSA = University of Illinois/NCSA Open Source License（LLVM/libFuzzer 许可证），OSI approved + FSF Free/Libre（cargo-deny 自报）。仅加 license allow，不改 bans 黑名单语义
- **scope**：deny.toml 不在 implementer commit scope（`crates/vane-fuzz/** + Cargo.toml + Cargo.lock + ci.yml`），须编排者批准
- **不 fix 后果**：CI deny job 在每次 push/PR FAIL → 阻断合并

### D.2 nightly fuzz 延 CI Phase 6 → **acceptable**

- stable `cargo check -p vane-fuzz` 因 `cfg(fuzzing)` 门控 FAIL（预期，cargo-fuzz 惯例）
- 5 targets 编译 + run 验证延 Phase 6 CI nightly fuzz-smoke job
- API 签名 reviewer 独立 grep 全部确认（§B.1）
- 宏展开后的完整编译验证未做，但 targets 代码直白（无复杂宏嵌套），风险低
- M4-PLAN Phase 6 有 fuzz-smoke CI job（nightly + cargo-fuzz install + 60s/target）
- 本地 nightly 网络不稳，延 CI 合理

### D.3 dict_load_fuzz 不启 jieba feature → **acceptable 设计取舍**

- 启 jieba → workspace feature unification → vane-core 在所有 `-p vane-core` 构建（含 wasm32-check）带 jieba → 污染生产构建
- 不启 jieba：dict_load_fuzz 验 M2-04 API 层降级不变量（Jieba→Err(DictUnavailable) 硬编码 + CjkBigram→Ok + Standard→Ok + set_user_dict 不 panic）
- 未覆盖：`JiebaDict::load`/`load_zstd` 畸形字节→Err 路径（需 jieba feature + ruzstd）
- defer Phase 6（vane-fuzz 加 optional `jieba` feature + cfg-gated `JiebaDict::load` 调用 + fuzz-smoke `--features jieba`）
- **判定**：acceptable。API 层降级不变量已验证；`JiebaDict::load` 畸形字节是二进制格式健壮性（不同关注点），defer 合理

### D.4 cfg(fuzzing) stable check fail → **cargo-fuzz 惯例，非问题**

- `libfuzzer-sys 0.4` 在 `cfg(fuzzing)` 下才导出 `libfuzzer` 模块
- `cargo fuzz` 构建时设 `--cfg fuzzing`（RUSTFLAGS），plain `cargo check` 不设
- stable `cargo check -p vane-fuzz` 报 E0432（`unresolved import libfuzzer`）——预期
- 这是所有 cargo-fuzz 项目的标准行为，非 vane-fuzz 特有问题
- implementer 的判断正确：非 API 错误、非 nightly 问题（nightly 也不设 cfg(fuzzing)，须 `cargo fuzz`）
- **判定**：cargo-fuzz 惯例，非真问题

## E. ⚠️ 无法从 diff 验证项

1. **nightly fuzz run**：5 targets 在 `cargo fuzz run` 下的实际编译 + 运行（宏展开后完整编译、libFuzzer 链接、seed corpus 行为）——须 Phase 6 CI nightly 验证。API 签名 + decode 健壮性 + 不变量逻辑已 reviewer 确认，但宏展开 + C++ libFuzzer 链接的编译正确性无法从 diff 验证。
2. **deny.toml NCSA fix 后 CI 绿**：deny.toml 未在 implementer diff 中，NCSA 加入后 deny job 是否全绿（无其他 license 缺口）须 CI 验证。
3. **fuzz-smoke CI job（Phase 6）**：spec §3.2 草案的 fuzz-smoke/fuzz-long workflow 未在本 task 实现（Phase 6），其正确性无法审。

## F. 总体判定

| 维度 | 判定 |
|---|---|
| Spec 合规（§3.2） | ✅ crate 结构 + workspace + CI + 5 targets 全匹配 |
| API 签名正确性 | ✅ 全部 20+ 调用 grep 确认 |
| ByteCursor 健壮性 | ✅ 任意字节不 panic |
| 不变量 exercise | ✅ 无 vacuous harness（无 Critical） |
| Findings | 0 Critical / 1 Important / 3 Minor |
| NCSA license | Important 须 fix（trivial 加 license，须编排者批准 deny.toml） |
| nightly 延 CI | acceptable |
| dict_load 不启 jieba | acceptable 设计取舍 |
| cfg(fuzzing) stable fail | cargo-fuzz 惯例，非问题 |

**是否进 fix 循环**：**否**（针对 vane-fuzz crate 本身）。implementer 的 commit（crate + 5 targets + workspace + CI --exclude）spec 合规、API 正确、不变量 non-vacuous。唯一阻断项是 NCSA license（deny.toml 加 1 行），但该 fix 在 implementer scope 外（deny.toml 未在其 diff），须编排者批准后单独改 deny.toml——不改 vane-fuzz crate 本身。3 个 Minor 是非阻断建议（vacuous pass 风险低 + HNSW/Err 假绿风险低 + Jieba Err 未断言），可 defer 或后续微调，不构成本 task 返工理由。

**建议**：
1. 编排者批准 deny.toml `[licenses] allow` 加 `"NCSA"`（I-1，阻断 CI，须先修）
2. Minor #1-3 可在 Phase 6 fuzz-smoke CI 跑通后视假绿/假红情况微调（非阻断）
3. Phase 6 CI fuzz-smoke job 验证 nightly 编译 + 60s/target 运行（覆盖 §E.1 无法从 diff 验证项）
