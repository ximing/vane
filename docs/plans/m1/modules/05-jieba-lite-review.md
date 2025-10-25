# 05-jieba-lite 代码审查

**审查日期**：2026-08-09
**审查对象**：BASE=97823d0 → HEAD=19c03d1，`crates/vane-core/src/tokenizer/jieba/` + `id.rs` + `mod.rs` + `Cargo.toml` + `deny.toml`
**审查模式**：只读，全程中文

## 总体结论

**APPROVED_WITH_MINOR** — 算法正确性（DAG 最大概率切分 + HMM Viterbi + DAT 双数组）经逐行核查与 jieba-rs 原版一致，R-3 落实到位，M0 冻结签名零破坏。无阻塞项。存在 2 处需编排者裁决的疑点（is_cjk 代码复制 / UserTrie 重复词条语义），均为非阻塞 minor。

## 逐维度审查

### 1. R-3 落实 — ✅

- `id.rs:31` `BuiltinTokenizer::Jieba => b"jieba-fmt-v1"` 编译期常量。✅
- `id.rs:4` 模块注释「jieba 词典版本」→「jieba 词典**格式**版本」。✅
- `id.rs:21-26` `builtin_dict_version` 文档注释从「M0 占位空串；M1 填日历版本」改为「编译期**格式**常量，仅格式变更时递增」。✅
- `mod.rs:54` `JiebaTokenizer::new` 中 `id: compute_tokenizer_id(BuiltinTokenizer::Jieba, user_dict)` —— 直接用，无二次哈希。✅
- `mod.rs:107-109` `id()` 返回 `&self.id`，无额外计算。✅
- `tests.rs:395-409` `jieba_tokenizer_id_independent_of_dict_calendar_version`：两份不同内容的词典（仅加"新词"）→ 断言 `t1.id() == t2.id()`。✅
- `tests.rs:411-419` `jieba_tokenizer_id_uses_compute_tokenizer_id`：断言 `tok.id() == compute_tokenizer_id(Jieba, user_dict)`。✅

### 2. DAG 最大概率切分 — ✅

- **build_dag**（`seg.rs:103-122`）：对每个起始位置 i，调 `dict.prefix_search_freq(chars, i)` 获取内置词命中 + `user.prefix_search(chars, i)` 获取用户词命中。用户词按 end 匹配，同 end 覆盖内置 freq（`seg.rs:109`），不同 end 追加。无匹配时兜底 `(i+1, freq=0)`（`seg.rs:116`）。与 jieba `get_DAG` 一致。✅
- **calc**（`seg.rs:126-144`）：从末尾向前的 DP。`route[n]=(0.0, n)` 基例。权重 `log_p = ln(f) - ln(total) + route[end].0`，其中 `f = if freq==0 {1.0} else {freq}`。与 jieba `log(self.FREQ.get(x) or 1) - logtotal + route[x+1][0]` 完全一致。✅
- **歧义消解**：DP 取 max 累积 log 概率路径，无发明规则。✅
- **cut**（`seg.rs:72-100`）：走 route 路径，连续单字（`end-x==1`）进 buf，多字词先 flush buf 到 HMM 再输出。末尾 flush。与 jieba `__cut_DAG` 一致。✅
- **验证**：`dag_picks_higher_freq_path` 测试——"研究生命" → "研究/生命"（freq 100+200=300 优于 50+10=60），手算 DP 路径确认 route[0].1=2（"研究"），route[2].1=4（"生命"）。✅

### 3. HMM Viterbi — ✅

- **状态**：B=0, M=1, E=2, S=3（`hmm.rs:25-29`），与 jieba 一致。✅
- **参数反序列化**（`hmm.rs:39-72`）：`start_p[4]` + `trans[16]`（row-major `trans[from*4+to]`）+ `emit_counts[4]` + 各状态 `(char_code:u32, prob:f64)` 条目。非硬编码——从 hmm_blob 反序列化。测试夹具 `build_hmm_blob`（`tests.rs:158-215`）写入 jieba 原版 START_P/TRANS_P 常量（逐字比对：start_p[B]=-0.26268660809250016, start_p[S]=-1.4652633398537698, MIN_FLOAT=-3.14e100 均与 jieba 一致），生产 dict.bin 由 07 写入真实发射矩阵。✅
- **Viterbi**（`hmm.rs:85-141`）：t=0 `V[0][s]=start_p[s]+emit_p(s,chars[0])`；t=1..n `V[t][s]=max_p(V[t-1][p]+trans[p][s])+emit_p(s,chars[t])`。log 空间累加。✅
- **末位仅 E/S**（`hmm.rs:123-128`）：`best_state = if V[n-1][E] > V[n-1][S] {E} else {S}`。与 jieba `viterbi` 一致。✅
- **回溯**（`hmm.rs:131-137`）：`full_path[t-1]` 对应时间 t 的前驱。从 best_state 反向追溯到 t=0，reverse。逻辑正确。✅
- **decode_states**（`hmm.rs:145-178`）：B 起始找对应 E（`chars[i..=j]` 为一词），S 为单字词。M/E 异常位置单字兜底。正确。✅
- **emit_p 未知字**（`hmm.rs:75-80`）：返回 `MIN_FLOAT`，与 jieba 一致。✅

### 4. DAT 双数组 — ✅

- **真 DAT**（`tests.rs:74-154`）：Aoe BFS 构建算法——先建 trie，BFS 遍历 trie 节点，`find_base`（`tests.rs:139-154`）线性探测无冲突 base 值，设置 `base[dat_id]` + `check[t]=dat_id`。非有序数组二分冒充。✅
- **DAT 转移**（`dict.rs:19-20, 104-113`）：`t = base[node] + char_code`；`check[t] == node` 则合法后继。`values[t] >= 0` 为终态（存词频）。char_code = Unicode 标量值。正确。✅
- **common_prefix_search**（`dict.rs:99-118`）：从 node=0 开始逐字转移，终态节点收集前缀 `chars[0..=i]`。正确。✅
- **prefix_search_freq**（`dict.rs:122-143`）：DAG 用，返回 `(end_exclusive=i+1, freq)`。正确。✅
- **freq**（`dict.rs:88-96`）：traverse 到词末节点，检查 `values[node] >= 0`。正确。✅
- **注意**：find_base 从 1 线性探测，char_code=Unicode 标量值（CJK ~0x4E00+），数组会有稀疏空洞。测试夹具可接受；07 生产构建需 char remapping 压缩（报告偏离 4 已记录）。非阻塞。

### 5. 中英混排 + position 连续 — ✅（minor：is_cjk 复制）

- `mod.rs:66-105`：按 `is_cjk` 切 run。CJK run → `seg::cut`（DAG+HMM）；非 CJK run → `unicode_words` + `to_lowercase` + `stemmer.stem`。position 跨 run 累积（`position += 1` 每个 token）。✅
- `tests.rs:337-346` `mixed_script_positions_continuous`：断言 pos0="机器学习"，pos1="run"（stemmed）。✅
- `tests.rs:349-356` `latin_run_uses_standard_pipeline`：断言 "Running"→"run"，"RUNNERS"→"runner"。✅
- **minor**：`mod.rs:113-127` 的 `is_cjk` 与 `cjk_bigram.rs:97-111` 的 `is_cjk` **完全相同**（10 个 Unicode range 逐字一致），但为**复制**非复用。计划说"复用 M0 cjk_bigram::is_cjk"，但 cjk_bigram 的 `is_cjk` 是模块私有（`fn is_cjk`，无 `pub`），jieba 无法引用。理想做法是将 cjk_bigram 的 `is_cjk` 改为 `pub(crate)` 并共享，避免未来两份副本漂移。功能正确，但违反计划字面承诺。→ 疑点①

### 6. 用户词表优先级（§5.3） — ✅

- **用户词覆盖内置**（`seg.rs:108-114`）：用户词按 end 匹配，同 end 覆盖内置 freq。✅
- **MAX_USER_DICT_ENTRIES**（`mod.rs:37-39`）：`JiebaTokenizer::new` 校验 `user_dict.len() > MAX_USER_DICT_ENTRIES → DictTooLarge`。`build_jieba_tokenizer`（`mod.rs:82-84`）也校验。✅
- **缺省 freq**（`mod.rs:44`）：`UserDictEntry::Word(t) => (t, max_freq)`，`max_freq = dict.max_freq()`。与 SPEC §5.3「缺省 freq = 内置词典最高频值」一致。✅
- **max_freq 计算**（`dict.rs:213-219`）：从 values 数组中 `v >= 0` 的最大值。正确。✅
- **"同为用户词 freq 高者优先"**：`UserTrie::insert`（`seg.rs:30-47`）对同词重复插入是 last-write-wins（覆盖），非 max-freq-wins。但 §5.3 此条的语义实指 DAG 歧义路径选择中高频路径胜出——由 DP `calc` 自然保证（高频→高 log_p→被选）。UserTrie 的覆盖仅影响相同 (start,end) 词条的 freq 值，极边缘场景（同一用户词重复声明）。功能上不阻塞。→ 疑点②
- `tests.rs:362-388`：`user_dict_overrides_builtin` + `user_dict_new_word_single_token`（"布地奈德"单 token）均真实断言。✅

### 7. feature 隔离 — ✅

- `Cargo.toml:15-16`：`ruzstd = { version="0.5", optional=true }` + `[features] jieba = ["ruzstd"]`。✅
- `mod.rs:5-6`：`#[cfg(feature="jieba")] pub mod jieba;`。✅
- `mod.rs:76-86`：`build_jieba_tokenizer` 在 `#[cfg(feature="jieba")]` 下。✅
- `mod.rs:67`：`build_tokenizer(Jieba)` 分支始终返回 `Err(DictUnavailable)`（无 feature 时也如此）。✅
- **wasm32 check**：`cargo check --target wasm32-unknown-unknown -p vane-core` 通过（jieba feature 默认关）。✅
- `tests.rs:184-190` `jieba_without_feature_returns_dict_unavailable`：无 feature 时断言 DictUnavailable。✅

### 8. M0 冻结签名零破坏 — ✅

- `compute_tokenizer_id(kind, user_dict) -> TokenizerId`（`id.rs:63`）：签名未变。✅
- `build_tokenizer(kind, user_dict) -> Result<Box<dyn Tokenizer>>`（`mod.rs:54-69`）：签名未变，Jieba 分支仍 `Err(DictUnavailable)`。✅
- `Token`/`Tokenizer`/`BuiltinTokenizer`/`UserDictEntry`/`MAX_USER_DICT_ENTRIES`：未变。✅
- 仅新增 `build_jieba_tokenizer`（`mod.rs:77-86`）。✅
- `id.rs::builtin_dict_version` 内部返回值 Jieba 从 `b""` 改为 `b"jieba-fmt-v1"`（私有函数，返回类型 `&'static [u8]` 未变）。✅

### 9. dict.bin 格式（§5.2） — ✅（minor：owned Vec 非"零拷贝"）

- **SPEC §5.2**：头部 16 字节 `magic(4) | format_version(4) | sha256(8 前缀)`。✅
- **实际格式**（`dict.rs:3-17`）：magic(4) + format_version(4) + sha256_prefix(8) = 16 字节头，其后扩展 dict_version_len(u16) + dict_version(UTF-8) + total_freq(u64) + dat_len(u32) + base/check/values([i32;dat_len]) + hmm_blob_len(u32) + hmm_blob。头部 16 字节与 SPEC 一致；后续字段为内容扩展。✅
- **parse**（`dict.rs:165-231`）：逐字段解析，每步 `ok_or_else(Corrupt)`，format_version 校验。正确。✅
- **minor**：计划说「零拷贝引用 bytes 切片」，实际为 owned Vec（`take_i32_slice` 拷贝到 Vec）。报告偏离 2 已记录，~4MB 数组拷贝 <10ms，<150ms 目标可达。非阻塞。✅
- **load_zstd**（`dict.rs:51-60`）：ruzstd StreamingDecoder 解压后调 load。绑定层用。✅

### 10. 红线 — ✅

- **算法与 jieba-rs 一致**：DAG（前缀搜索+DP 最大概率）、HMM（B/M/E/S Viterbi 末位 E/S）、参数从 blob 反序列化（非硬编码在运行时路径）。无发明规则。✅
- **词典不进 wasm**：jieba feature 默认关，wasm32 check 不带 `--features jieba`。✅
- **ruzstd 非黑名单**：`cargo tree -p vane-core --features jieba -e normal -i ruzstd` → 仅 `ruzstd v0.5.0`，无 regex/ndarray 传递依赖。✅
- **core 禁 std::fs**：`grep std::fs/std::net/std::path` 在 jieba/ 目录无命中。`bash scripts/check-no-std-fs.sh` → OK。✅
- **cfg 仅在 feature 门**：`cfg(feature="jieba")` 是 feature 非 target cfg，不违反 I-5。✅

### 11. 测试质量 — ✅

- **17 jieba 测试 + 2 factory 测试全绿**（`cargo test -p vane-core --features jieba jieba` → 17 passed）。✅
- **全量**：`cargo test --workspace --all-features` → 231 全绿。✅
- **clippy**：`cargo clippy -p vane-core --features jieba --all-targets -- -D warnings` → 无警告。✅
- **真实断言**：`dag_segment_known_words`（精确切分序列）、`dag_picks_higher_freq_path`（路径选择）、`dict_lookup_word_freq`/`dict_prefix_match`（DAT 查询）、`user_dict_new_word_single_token`（生造词单 token）、`jieba_tokenizer_id_independent_of_dict_calendar_version`（R-3）等均有实质断言。✅
- **hmm_recognizes_unknown_word** 断言较弱（仅 `!is_empty()`），但 200 句与 jieba-rs 100% 一致验收在 10-ci-m1（fixture 方案，jieba-rs 因 regex 黑名单不引入）。可接受。✅
- **四验收**：①②在 10-ci-m1（fixture 离线生成）；③核心逻辑覆盖（"布地奈德"单 token）；④覆盖（DictUnavailable）。路径就位。✅

### 12. deny.toml 改动 — ✅

- **[advisories] version=2**（`deny.toml:3`）：cargo-deny 0.16 不兼容旧格式（`vulnerability`/`unmaintained`/`notice` 字段已移除）。升级合理。`yanked="deny"` 保留。未放松 bans。✅
- **regex 黑名单来源**：`cargo tree -i regex` → `criterion v0.5.1`（dev-dep of vane-core）+ `napi-derive-backend v1.0.75`（build-dep of vane-node）。均非 05-jieba 引入。ruzstd 无 regex 依赖。✅
- **advisories CVSS 4.0 解析失败**：advisory-db 含 RUSTSEC-2026-0073 用 CVSS 4.0，cargo-deny 0.16.4 不支持。基础设施问题，非本模块引入。✅

## 需编排者裁决的疑点

### 疑点①：is_cjk 代码复制（minor，非阻塞）

`jieba/mod.rs:113-127` 复制了 `cjk_bigram.rs:97-111` 的 `is_cjk` 函数（10 个 Unicode range 逐字相同）。计划承诺"复用 M0 cjk_bigram::is_cjk"，但 cjk_bigram 的 `is_cjk` 是模块私有，无法跨模块引用。

**裁决建议**：将 `cjk_bigram::is_cjk` 改为 `pub(crate) fn is_cjk`，jieba 模块引用之，删除副本。这是 ~3 行改动，可在 06-userdict-reindex 或独立 minor commit 中做。若编排者接受复制（功能等价、范围冻结），也可标记为已知偏差不再处理。

### 疑点②：UserTrie 重复词条 last-write-wins（minor，非阻塞）

`UserTrie::insert`（`seg.rs:46`）对同一词重复插入时 `self.freqs[node] = freq` 是覆盖语义（last-write-wins），非"freq 高者优先"。SPEC §5.3 说"同为用户词则 freq 高者优先"。

**裁决建议**：经核查，§5.3 此条的语义实指 DAG 歧义路径选择中高频路径胜出——由 DP `calc` 自然保证。UserTrie 的覆盖仅影响同一 (start,end) 词条的 freq 值，即用户词表中有完全相同的词重复声明两次的极边缘场景。实际不影响分词正确性。若编排者认为需严格实现 max-freq-wins，可改为 `self.freqs[node] = self.freqs[node].max(freq as i32)`（1 行改动）。否则标记为已知偏差。

## 验证命令复现

```
cargo test -p vane-core --features jieba jieba        # 17 passed
cargo test --workspace --all-features                 # 231 passed
cargo clippy -p vane-core --features jieba --all-targets -- -D warnings  # clean
cargo check --target wasm32-unknown-unknown -p vane-core                # ok
bash scripts/check-no-std-fs.sh                                       # OK
cargo tree -p vane-core --features jieba -e normal -i ruzstd           # ruzstd only
cargo tree -i regex                                                   # criterion + napi-derive
```

## 文件清单

| 文件 | 角色 |
|---|---|
| `crates/vane-core/src/tokenizer/jieba/mod.rs` | JiebaTokenizer + 中英混排 |
| `crates/vane-core/src/tokenizer/jieba/dict.rs` | JiebaDict + DAT load/查询 |
| `crates/vane-core/src/tokenizer/jieba/seg.rs` | DAG build_dag + calc DP + cut |
| `crates/vane-core/src/tokenizer/jieba/hmm.rs` | HmmParams 反序列化 + Viterbi |
| `crates/vane-core/src/tokenizer/jieba/tests.rs` | 17 测试 + DAT/HMM/dict.bin 夹具构建器 |
| `crates/vane-core/src/tokenizer/id.rs` | builtin_dict_version(Jieba)=b"jieba-fmt-v1" |
| `crates/vane-core/src/tokenizer/mod.rs` | build_jieba_tokenizer 新增 + cfg 门控 |
| `crates/vane-core/Cargo.toml` | ruzstd optional + [features] jieba |
| `deny.toml` | [advisories] version=2 |
