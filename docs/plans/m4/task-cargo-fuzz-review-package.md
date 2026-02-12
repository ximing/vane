## Commits 0458942..d4a94d8 (1a cargo-fuzz)

d4a94d8 feat(fuzz): vane-fuzz crate + 5 targets + CI --exclude vane-fuzz（M4 阶段一 a）

## Diff stat

 .github/workflows/ci.yml                           |  10 +-
 Cargo.lock                                         |  24 ++++
 Cargo.toml                                         |   7 +-
 crates/vane-fuzz/Cargo.toml                        |  39 +++++++
 crates/vane-fuzz/fuzz_targets/brute_search_fuzz.rs |  96 ++++++++++++++++
 crates/vane-fuzz/fuzz_targets/common.rs            |  90 +++++++++++++++
 crates/vane-fuzz/fuzz_targets/dict_load_fuzz.rs    |  78 +++++++++++++
 crates/vane-fuzz/fuzz_targets/hnsw_search_fuzz.rs  |  89 +++++++++++++++
 crates/vane-fuzz/fuzz_targets/merge_fuzz.rs        | 126 +++++++++++++++++++++
 .../fuzz_targets/persist_roundtrip_fuzz.rs         | 111 ++++++++++++++++++
 10 files changed, 667 insertions(+), 3 deletions(-)

## Full diff (U10)

diff --git a/.github/workflows/ci.yml b/.github/workflows/ci.yml
index 762b128..0e03f3b 100644
--- a/.github/workflows/ci.yml
+++ b/.github/workflows/ci.yml
@@ -34,32 +34,38 @@ jobs:
     needs: fmt
     runs-on: ubuntu-latest
     timeout-minutes: 20
     steps:
       - uses: actions/checkout@v4
       - uses: dtolnay/rust-toolchain@stable
         with:
           components: clippy
       - uses: Swatinem/rust-cache@v2
       - name: Clippy (-D warnings)
-        run: cargo clippy --all-targets --all-features -- -D warnings
+        # vane-fuzz 需 nightly + libfuzzer-sys，stable clippy 编不了 → --exclude。
+        # --exclude 须配 --workspace（否则 cargo 报 "--exclude can only be used
+        # together with --workspace"）。default-members 也排除 vane-fuzz，双保险。
+        # fuzz 自身的 lint 由 nightly fuzz-smoke job 负责（Phase 6）。
+        run: cargo clippy --all-targets --all-features --workspace --exclude vane-fuzz -- -D warnings
 
   test:
     needs: clippy
     runs-on: ubuntu-latest
     timeout-minutes: 20
     steps:
       - uses: actions/checkout@v4
       - uses: dtolnay/rust-toolchain@stable
       - uses: Swatinem/rust-cache@v2
       - name: Test workspace
-        run: cargo test --workspace --all-features
+        # vane-fuzz 是 fuzz crate（#![no_main] + libfuzzer-sys），非普通测试 target，
+        # stable cargo test 编不了 → --exclude。fuzz 验证由 nightly fuzz-smoke job 负责。
+        run: cargo test --workspace --all-features --exclude vane-fuzz
 
   recall:
     # SPEC §13.2-1 真实回归门禁：HNSW vs 暴力双路+RRF 基线，
     # 五档选择率（0.1%/1%/10%/50%/99%）× 三模式（vector/text/hybrid）recall@10 ≥0.95。
     # M0 recall.rs 保留作冒烟（trivially 1.0）；recall_regression 为 M1 硬门禁。
     needs: test
     runs-on: ubuntu-latest
     timeout-minutes: 15
     steps:
       - uses: actions/checkout@v4
diff --git a/Cargo.lock b/Cargo.lock
index 3a45ba2..0128c32 100644
--- a/Cargo.lock
+++ b/Cargo.lock
@@ -16,20 +16,26 @@ name = "anes"
 version = "0.1.6"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "4b46cbb362ab8752921c97e041f5e366ee6297bd428a31275b9fcf1e380f7299"
 
 [[package]]
 name = "anstyle"
 version = "1.0.14"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "940b3a0ca603d1eade50a4846a2afffd5ef57a9feac2c0e2ec2e14f9ead76000"
 
+[[package]]
+name = "arbitrary"
+version = "1.4.2"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "c3d036a3c4ab069c7b410a2ce876bd74808d2d0888a82667669f8e783a898bf1"
+
 [[package]]
 name = "async-trait"
 version = "0.1.92"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "82f6aeea286b8eb4dd3431a1be1b59d290ace00f5bfd8e2a159bc2a05e2c1667"
 dependencies = [
  "proc-macro2",
  "quote",
  "syn 3.0.3",
 ]
@@ -455,20 +461,30 @@ dependencies = [
  "futures-util",
  "wasm-bindgen",
 ]
 
 [[package]]
 name = "libc"
 version = "0.2.189"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "3eaf3ede3fee6db1a4c2ee091bf8a8b4dccdc6d17f656fb07896ee72867612f2"
 
+[[package]]
+name = "libfuzzer-sys"
+version = "0.4.13"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "a9fd2f41a1cba099f79a0b6b6c35656cf7c03351a7bae8ff0f28f25270f929d2"
+dependencies = [
+ "arbitrary",
+ "cc",
+]
+
 [[package]]
 name = "libloading"
 version = "0.9.0"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "754ca22de805bb5744484a5b151a9e1a8e837d5dc232c2d7d8c2e3492edc8b60"
 dependencies = [
  "cfg-if",
  "windows-link",
 ]
 
@@ -936,20 +952,28 @@ dependencies = [
 
 [[package]]
 name = "vane-ffi"
 version = "0.2.0"
 dependencies = [
  "serde_json",
  "vane-core",
  "vane-dict-zh",
 ]
 
+[[package]]
+name = "vane-fuzz"
+version = "0.0.0"
+dependencies = [
+ "libfuzzer-sys",
+ "vane-core",
+]
+
 [[package]]
 name = "vane-node"
 version = "0.2.0"
 dependencies = [
  "napi",
  "napi-build",
  "napi-derive",
  "serde_json",
  "vane-core",
  "vane-dict-zh",
diff --git a/Cargo.toml b/Cargo.toml
index a6ed685..d0b6c93 100644
--- a/Cargo.toml
+++ b/Cargo.toml
@@ -1,12 +1,17 @@
 [workspace]
-members = ["crates/vane-core", "crates/vane-ffi", "crates/vane-node", "crates/vane-dict-zh", "crates/vane-wasm"]
+members = ["crates/vane-core", "crates/vane-ffi", "crates/vane-node", "crates/vane-dict-zh", "crates/vane-wasm", "crates/vane-fuzz"]
+# vane-fuzz 需 nightly + libfuzzer-sys，stable cargo build/test/clippy 编不了；
+# default-members 排除 vane-fuzz，确保 `cargo build`/`cargo test`（不带 --workspace）
+# 的默认范围不含 fuzz。`cargo test --workspace` 显式列全部 members，仍需 --exclude
+# vane-fuzz（见 .github/workflows/ci.yml test/clippy job）。
+default-members = ["crates/vane-core", "crates/vane-ffi", "crates/vane-node", "crates/vane-dict-zh", "crates/vane-wasm"]
 resolver = "2"
 
 [workspace.package]
 version = "0.2.0"
 edition = "2021"
 license = "Apache-2.0"
 authors = ["Vane Contributors"]
 
 [workspace.dependencies]
 vane-core = { path = "crates/vane-core" }
diff --git a/crates/vane-fuzz/Cargo.toml b/crates/vane-fuzz/Cargo.toml
new file mode 100644
index 0000000..bd505e5
--- /dev/null
+++ b/crates/vane-fuzz/Cargo.toml
@@ -0,0 +1,39 @@
+[package]
+name = "vane-fuzz"
+version = "0.0.0"
+edition = "2021"
+publish = false
+license = "Apache-2.0"
+# vane-fuzz 是 cargo-fuzz 集成 crate：libfuzzer-sys 绑定 C++ libFuzzer，
+# 需 nightly（-Z sanitizer）。绝不进 vane-core/wasm/ffi 生产构建——
+# workspace default-members 排除 vane-fuzz，CI test/clippy job 加 --exclude。
+# CI 的 fuzz-smoke/fuzz-long（nightly）由 Phase 6 新增 job 负责。
+[package.metadata]
+# cargo-fuzz 0.13+ 要求显式标记本 crate 为 fuzz crate（检测机制），
+# 否则 `cargo fuzz build` 报 "does not look like a cargo-fuzz manifest"。
+cargo-fuzz = true
+
+[dependencies]
+vane-core = { path = "../vane-core" }
+libfuzzer-sys = "0.4"
+
+# fuzz targets（每个独立 [[bin]]，#![no_main] + libfuzzer::fuzz_target!）
+[[bin]]
+name = "brute_search_fuzz"
+path = "fuzz_targets/brute_search_fuzz.rs"
+
+[[bin]]
+name = "hnsw_search_fuzz"
+path = "fuzz_targets/hnsw_search_fuzz.rs"
+
+[[bin]]
+name = "persist_roundtrip_fuzz"
+path = "fuzz_targets/persist_roundtrip_fuzz.rs"
+
+[[bin]]
+name = "merge_fuzz"
+path = "fuzz_targets/merge_fuzz.rs"
+
+[[bin]]
+name = "dict_load_fuzz"
+path = "fuzz_targets/dict_load_fuzz.rs"
diff --git a/crates/vane-fuzz/fuzz_targets/brute_search_fuzz.rs b/crates/vane-fuzz/fuzz_targets/brute_search_fuzz.rs
new file mode 100644
index 0000000..67e2bb1
--- /dev/null
+++ b/crates/vane-fuzz/fuzz_targets/brute_search_fuzz.rs
@@ -0,0 +1,96 @@
+//! Fuzz target：暴力检索不 panic / topK 合法 / score 非 NaN。
+//!
+//! 输入 decode：ByteCursor → dim（1..=16）+ n_docs（0..=8）+ top_k（1..=10）
+//!   + mode（Vector/Text/Hybrid）+ 各 doc 的 text+vector + query 的 text+vector。
+//! 不变量：search_brute_baseline 不 panic；hits.len() ≤ top_k；每 hit.score 非 NaN。
+//!
+//! 设计 §3.2 target 表第 1 行。recall 质量由 proptest §3.3 覆盖；本 target 只验
+//! 暴力路径在随机输入下不 crash。
+
+#![no_main]
+
+mod common;
+
+use std::sync::Arc;
+
+use libfuzzer::fuzz_target;
+
+use common::{build_schema, ByteCursor};
+use vane_core::api::{
+    CollectionOptions, Db, Doc, FusionSpec, OpenOptions, SearchMode, SearchQuery,
+};
+use vane_core::vfs::memory::MemoryVfs;
+use vane_core::vfs::Vfs;
+
+fuzz_target!(|data: &[u8]| {
+    let mut c = ByteCursor::new(data);
+
+    // dim 1..=16（≤ DIM_MAX=4096，Schema::new 必过）。
+    let dim = (c.u8() as u32).max(1).min(16);
+    let n_docs = (c.u8() as usize).min(8);
+    let top_k = (c.u8() as u32).max(1).min(10);
+    let mode = match c.u8() % 3 {
+        0 => SearchMode::Vector,
+        1 => SearchMode::Text,
+        _ => SearchMode::Hybrid,
+    };
+
+    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
+    let schema = build_schema(true, dim);
+    let db = Db::open(vfs, "db", OpenOptions::default()).expect("Db::open on MemoryVfs");
+    let col = db
+        .collection("c", schema, CollectionOptions::default())
+        .expect("collection create with valid schema");
+
+    let mut docs = Vec::with_capacity(n_docs);
+    for i in 0..n_docs {
+        let text = c.small_string();
+        docs.push(Doc {
+            id: format!("d{i}"),
+            text: if text.is_empty() { None } else { Some(text) },
+            vector: Some(c.f32_vec(dim as usize)),
+            meta: None,
+        });
+    }
+    if !docs.is_empty() {
+        let _ = col.add(&docs);
+    }
+    let _ = col.flush();
+
+    let q_text = if c.bool() {
+        Some(c.small_string())
+    } else {
+        None
+    };
+    let q_vec = if c.bool() {
+        Some(c.f32_vec(dim as usize))
+    } else {
+        None
+    };
+    let query = SearchQuery {
+        text: q_text,
+        vector: q_vec,
+        top_k,
+        mode,
+        fusion: FusionSpec::Rrf,
+        filter: None,
+        candidate_multiplier: 3,
+    };
+
+    // 暴力检索（search_brute_baseline 强制 f32 brute，bypass HNSW/SQ8）。
+    let hits = match col.search_brute_baseline(&query) {
+        Ok(h) => h,
+        Err(_) => return,
+    };
+    // 不变量 1：topK 合法（≤ top_k）。
+    assert!(
+        hits.len() <= top_k as usize,
+        "topK overflow: {} > {}",
+        hits.len(),
+        top_k
+    );
+    // 不变量 2：score 非 NaN。
+    for h in &hits {
+        assert!(!h.score.is_nan(), "NaN score from brute search");
+    }
+});
diff --git a/crates/vane-fuzz/fuzz_targets/common.rs b/crates/vane-fuzz/fuzz_targets/common.rs
new file mode 100644
index 0000000..e413bcc
--- /dev/null
+++ b/crates/vane-fuzz/fuzz_targets/common.rs
@@ -0,0 +1,90 @@
+//! vane-fuzz targets 的共享字节→结构 decoder。
+//!
+//! 设计取舍：不引 `arbitrary` crate（设计 §3.2 Cargo.toml 只列 libfuzzer-sys；
+//! arbitrary 虽大概率不触黑名单，但多一个传递依赖多一份 deny 风险）。自研轻量
+//! ByteCursor：从 libfuzzer 提供的 `&[u8]` 确定性地消费字节构造结构化输入；
+//! 字节耗尽时返回 0（libfuzzer corpus 普遍短，0 字节是合法边界输入）。
+//!
+//! 不变量：decoder 自身绝不 panic（全用 `get`+`unwrap_or`+`saturating_add`）。
+
+/// 从 fuzzer 字节流确定性消费的游标。
+pub struct ByteCursor<'a> {
+    data: &'a [u8],
+    pos: usize,
+}
+
+impl<'a> ByteCursor<'a> {
+    pub fn new(data: &'a [u8]) -> Self {
+        Self { data, pos: 0 }
+    }
+
+    /// 消费 1 字节；耗尽时返 0。
+    pub fn u8(&mut self) -> u8 {
+        let b = *self.data.get(self.pos).unwrap_or(&0);
+        self.pos = self.pos.saturating_add(1);
+        b
+    }
+
+    /// 消费至多 4 字节（LE）为 u32；不足补 0。
+    pub fn u32_le(&mut self) -> u32 {
+        let mut buf = [0u8; 4];
+        for i in 0..4 {
+            buf[i] = *self.data.get(self.pos + i).unwrap_or(&0);
+        }
+        self.pos = self.pos.saturating_add(4);
+        u32::from_le_bytes(buf)
+    }
+
+    /// 消费 1 长度前缀字节（cap 32）+ len 字节为 String。
+    /// lossy UTF-8 转换：畸形 unicode 不 panic（返回替换字符）。
+    pub fn small_string(&mut self) -> String {
+        let len = (self.u8() as usize).min(32);
+        let mut buf = Vec::with_capacity(len);
+        for _ in 0..len {
+            buf.push(self.u8());
+        }
+        String::from_utf8_lossy(&buf).into_owned()
+    }
+
+    /// 消费 n×4 字节为 Vec<f32>（每 4 字节 LE）。
+    /// NaN/Inf 过滤为 0.0——保 score 算术良定义（设计 §3.3 proptest 同考量）。
+    pub fn f32_vec(&mut self, n: usize) -> Vec<f32> {
+        (0..n)
+            .map(|_| {
+                let mut buf = [0u8; 4];
+                for i in 0..4 {
+                    buf[i] = *self.data.get(self.pos + i).unwrap_or(&0);
+                }
+                self.pos = self.pos.saturating_add(4);
+                let v = f32::from_le_bytes(buf);
+                if v.is_nan() || v.is_infinite() {
+                    0.0
+                } else {
+                    v
+                }
+            })
+            .collect()
+    }
+
+    /// 消费 1 字节 LSB 为 bool。
+    pub fn bool(&mut self) -> bool {
+        self.u8() & 1 == 1
+    }
+}
+
+/// 构造简单 schema：1 个 Vector 字段（给定 dim + Cosine 度量），可选 1 个 Text 字段。
+/// dim 经调用方 clamp 到合法区间（1..=16），Schema::new 不会 Err。
+pub fn build_schema(with_text: bool, dim: u32) -> vane_core::types::Schema {
+    let mut fields: Vec<(String, vane_core::types::FieldDef)> = Vec::new();
+    if with_text {
+        fields.push(("body".into(), vane_core::types::FieldDef::Text));
+    }
+    fields.push((
+        "v".into(),
+        vane_core::types::FieldDef::Vector {
+            dim,
+            metric: vane_core::types::Metric::Cosine,
+        },
+    ));
+    vane_core::types::Schema::new(fields).expect("schema with 1 vector field is valid")
+}
diff --git a/crates/vane-fuzz/fuzz_targets/dict_load_fuzz.rs b/crates/vane-fuzz/fuzz_targets/dict_load_fuzz.rs
new file mode 100644
index 0000000..5bb811c
--- /dev/null
+++ b/crates/vane-fuzz/fuzz_targets/dict_load_fuzz.rs
@@ -0,0 +1,78 @@
+//! Fuzz target：畸形词典字节 → 降级 bigram 不抛错（M2-04 铁律）。
+//!
+//! 输入 decode：ByteCursor → n_entries（0..=16）+ 各 entry 的 word 字符串 + freq。
+//! 不变量（M2-04）：分词器构造路径不 panic——
+//!   - build_tokenizer(Jieba, ..) 无 dict 实例 → Err(DictUnavailable / DictTooLarge)，不 panic；
+//!   - build_tokenizer(CjkBigram, ..) 降级路径 → Ok，不 panic；
+//!   - build_tokenizer(Standard, ..) → Ok，不 panic；
+//!   - Collection::set_user_dict(fuzzer entries) → Ok 或 Err（DictTooLarge / Busy），不 panic。
+//!
+//! 设计 §3.2 target 表第 5 行。
+//! 取舍：JiebaDict::load/load_zstd 的畸形字节→Err 路径需 jieba feature
+//! （ruzstd）。设计 §3.2 Cargo.toml 按"字面采用"不启 jieba（避 workspace
+//! feature unification 触 wasm32-check / 其他门禁）。本 target 验 M2-04 的
+//! API 层降级不变量（Jieba→Err→CjkBigram→Ok 不 panic）；JiebaDict::load 的
+//! 畸形字节 fuzz defer Phase 6（如需，vane-fuzz 加 optional `jieba` feature
+//! + cfg-gated JiebaDict::load 调用）。
+
+#![no_main]
+
+mod common;
+
+use std::sync::Arc;
+
+use libfuzzer::fuzz_target;
+
+use common::{build_schema, ByteCursor};
+use vane_core::api::{CollectionOptions, Db, OpenOptions};
+use vane_core::tokenizer::{build_tokenizer, BuiltinTokenizer, UserDictEntry};
+use vane_core::vfs::memory::MemoryVfs;
+use vane_core::vfs::Vfs;
+
+fuzz_target!(|data: &[u8]| {
+    let mut c = ByteCursor::new(data);
+
+    // 畸形词典字节 → 结构化 UserDictEntry（lossy UTF-8，不 panic）。
+    let n_entries = (c.u8() as usize).min(16);
+    let mut user_dict: Vec<UserDictEntry> = Vec::with_capacity(n_entries);
+    for _ in 0..n_entries {
+        let word = c.small_string();
+        if word.is_empty() {
+            continue;
+        }
+        if c.bool() {
+            user_dict.push(UserDictEntry::Word(word));
+        } else {
+            user_dict.push(UserDictEntry::WordWithFreq {
+                term: word,
+                freq: c.u32_le(),
+            });
+        }
+    }
+
+    // M2-04 不变量 1：Jieba 路径无 dict → Err（DictUnavailable 或 DictTooLarge），不 panic。
+    //    （user_dict.len() ≤ 16 < 100k → 不会 DictTooLarge；应返 DictUnavailable。）
+    let jieba_tok = build_tokenizer(BuiltinTokenizer::Jieba, &user_dict);
+    drop(jieba_tok); // 接受 Ok 或 Err——不 panic 即满足。
+
+    // M2-04 不变量 2：CjkBigram 降级路径 → Ok，不 panic。
+    let bigram_tok = build_tokenizer(BuiltinTokenizer::CjkBigram, &user_dict);
+    assert!(bigram_tok.is_ok(), "CjkBigram fallback must succeed");
+    drop(bigram_tok);
+
+    // M2-04 不变量 3：Standard 路径 → Ok，不 panic。
+    let std_tok = build_tokenizer(BuiltinTokenizer::Standard, &user_dict);
+    assert!(std_tok.is_ok(), "Standard tokenizer must succeed");
+    drop(std_tok);
+
+    // 端到端：Collection::set_user_dict 不 panic（可能 Err，不 panic 即满足）。
+    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
+    let db = Db::open(vfs, "db", OpenOptions::default()).expect("Db::open");
+    let schema = build_schema(true, 4);
+    let col = db
+        .collection("c", schema, CollectionOptions::default())
+        .expect("collection create");
+    // set_user_dict：>100k → DictTooLarge；Rebuilding → Busy。此处 ≤16，应 Ok。
+    let _ = col.set_user_dict(&user_dict);
+    // 不 panic 即满足 M2-04。
+});
diff --git a/crates/vane-fuzz/fuzz_targets/hnsw_search_fuzz.rs b/crates/vane-fuzz/fuzz_targets/hnsw_search_fuzz.rs
new file mode 100644
index 0000000..ea19b7f
--- /dev/null
+++ b/crates/vane-fuzz/fuzz_targets/hnsw_search_fuzz.rs
@@ -0,0 +1,89 @@
+//! Fuzz target：HNSW build+search 不 panic / score 非 NaN / hit id 全已知。
+//!
+//! 输入 decode：ByteCursor → dim（2..=8）+ n_docs（1..=20）+ top_k（1..=5）
+//!   + 各 doc 的 vector + query 的 vector。
+//! 不变量：search（HNSW 路径）不 panic；hits.len() ≤ top_k；每 hit.score 非 NaN；
+//!   每 hit.id ∈ 已添加 doc id 集合（无 phantom id）。
+//!
+//! 设计 §3.2 target 表第 2 行。recall 与暴力一致性由 proptest §3.3 覆盖；
+//! 本 target 不做严格 recall 断言（HNSW 近似，随机小图 recall 未必 100%，
+//! 严格断言易误报）。仅验 HNSW 路径不 crash + 结构合法。
+//! 双重路径：search_brute_baseline 也跑一次，保证 brute 不 panic（基线对照）。
+
+#![no_main]
+
+mod common;
+
+use std::sync::Arc;
+
+use libfuzzer::fuzz_target;
+
+use common::{build_schema, ByteCursor};
+use vane_core::api::{
+    CollectionOptions, Db, Doc, FusionSpec, OpenOptions, SearchMode, SearchQuery,
+};
+use vane_core::vfs::memory::MemoryVfs;
+use vane_core::vfs::Vfs;
+
+fuzz_target!(|data: &[u8]| {
+    let mut c = ByteCursor::new(data);
+
+    let dim = (c.u8() as u32).max(2).min(8);
+    let n_docs = (c.u8() as usize % 20) + 1; // 1..=20
+    let top_k = (c.u8() as u32).max(1).min(5);
+
+    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
+    // hnsw 路径只需 Vector 字段（无 Text 字段）。
+    let schema = build_schema(false, dim);
+    let db = Db::open(vfs, "db", OpenOptions::default()).expect("Db::open on MemoryVfs");
+    let col = db
+        .collection("c", schema, CollectionOptions::default())
+        .expect("collection create");
+
+    let known_ids: std::collections::HashSet<String> =
+        (0..n_docs).map(|i| format!("d{i}")).collect();
+    let mut docs = Vec::with_capacity(n_docs);
+    for i in 0..n_docs {
+        docs.push(Doc {
+            id: format!("d{i}"),
+            text: None,
+            vector: Some(c.f32_vec(dim as usize)),
+            meta: None,
+        });
+    }
+    let _ = col.add(&docs);
+    let _ = col.flush();
+
+    let query = SearchQuery {
+        text: None,
+        vector: Some(c.f32_vec(dim as usize)),
+        top_k,
+        mode: SearchMode::Vector,
+        fusion: FusionSpec::Rrf,
+        filter: None,
+        candidate_multiplier: 3,
+    };
+
+    // HNSW 路径（search 允 HNSW；若 HNSW 缺失自动 brute 回退，不 panic）。
+    let hnsw_hits = match col.search(&query) {
+        Ok(h) => h,
+        Err(_) => return,
+    };
+    // 不变量：topK 合法、score 非 NaN、id 全已知（无 phantom）。
+    assert!(hnsw_hits.len() <= top_k as usize, "HNSW topK overflow");
+    for h in &hnsw_hits {
+        assert!(!h.score.is_nan(), "HNSW NaN score");
+        assert!(known_ids.contains(&h.id), "HNSW phantom id: {}", h.id);
+    }
+
+    // Brute 基线对照（不 panic + 结构合法）。
+    let brute_hits = match col.search_brute_baseline(&query) {
+        Ok(h) => h,
+        Err(_) => return,
+    };
+    assert!(brute_hits.len() <= top_k as usize, "brute topK overflow");
+    for h in &brute_hits {
+        assert!(!h.score.is_nan(), "brute NaN score");
+        assert!(known_ids.contains(&h.id), "brute phantom id: {}", h.id);
+    }
+});
diff --git a/crates/vane-fuzz/fuzz_targets/merge_fuzz.rs b/crates/vane-fuzz/fuzz_targets/merge_fuzz.rs
new file mode 100644
index 0000000..646bc57
--- /dev/null
+++ b/crates/vane-fuzz/fuzz_targets/merge_fuzz.rs
@@ -0,0 +1,126 @@
+//! Fuzz target：merge 不丢文档（除 tombstone）/ docid 连续。
+//!
+//! 输入 decode：ByteCursor → dim（1..=8）+ n_flushes（1..=4）+ docs_per_flush（1..=5）
+//!   + n_delete 选择（cursor 驱动）+ 各 doc 的 vector + query 向量。
+//! 流程：多轮 add+flush（多段）→ 按字节选择 delete（tombstone）→ compact（merge 全段）
+//!   → search top_k=1000。
+//! 不变量：tombstoned id 不在 hits；hit id 全已知（无 phantom）；hits 无重复 id
+//!   （docid 连续）；live id 全可见（不丢文档）。
+//!
+//! 设计 §3.2 target 表第 4 行。compact() 合并全段 + 物理清 tombstone（collection.rs:1076）。
+
+#![no_main]
+
+mod common;
+
+use std::sync::Arc;
+
+use libfuzzer::fuzz_target;
+
+use common::{build_schema, ByteCursor};
+use vane_core::api::{
+    CollectionOptions, Db, Doc, FusionSpec, OpenOptions, SearchMode, SearchQuery,
+};
+use vane_core::vfs::memory::MemoryVfs;
+use vane_core::vfs::Vfs;
+
+fuzz_target!(|data: &[u8]| {
+    let mut c = ByteCursor::new(data);
+
+    let dim = (c.u8() as u32).max(1).min(8);
+    let n_flushes = (c.u8() as usize % 4) + 1; // 1..=4
+    let docs_per_flush = (c.u8() as usize % 5) + 1; // 1..=5
+
+    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
+    let schema = build_schema(false, dim);
+    let db = Db::open(vfs, "db", OpenOptions::default()).expect("Db::open");
+    let col = db
+        .collection("c", schema, CollectionOptions::default())
+        .expect("collection create");
+
+    let mut added_ids: Vec<String> = Vec::new();
+    let mut deleted_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
+
+    // 多轮 add+flush → 多段结构。
+    for flush_idx in 0..n_flushes {
+        let mut docs = Vec::with_capacity(docs_per_flush);
+        for j in 0..docs_per_flush {
+            let id = format!("f{flush_idx}_d{j}");
+            added_ids.push(id.clone());
+            docs.push(Doc {
+                id,
+                text: None,
+                vector: Some(c.f32_vec(dim as usize)),
+                meta: None,
+            });
+        }
+        let _ = col.add(&docs);
+        let _ = col.flush();
+    }
+
+    // 按字节选择 delete（tombstone）：删 n_delete % (added+1) 个 id。
+    let total_added = added_ids.len();
+    let n_delete = (c.u8() as usize) % (total_added + 1);
+    for _ in 0..n_delete {
+        if added_ids.is_empty() {
+            break;
+        }
+        let idx = (c.u8() as usize) % added_ids.len();
+        let id = added_ids[idx].clone();
+        let _ = col.delete(&[id.clone()]);
+        deleted_ids.insert(id);
+    }
+
+    let live_ids: std::collections::HashSet<&String> = added_ids
+        .iter()
+        .filter(|id| !deleted_ids.contains(*id))
+        .collect();
+
+    // compact = merge 全段 + 物理 tombstone 清除。
+    let _ = col.compact();
+
+    // search top_k=TOPK_MAX=1000：live docs 应全可见。
+    let query = SearchQuery {
+        text: None,
+        vector: Some(c.f32_vec(dim as usize)),
+        top_k: 1000,
+        mode: SearchMode::Vector,
+        fusion: FusionSpec::Rrf,
+        filter: None,
+        candidate_multiplier: 3,
+    };
+    let hits = col.search(&query).unwrap_or_default();
+    let hit_ids: std::collections::HashSet<&String> = hits.iter().map(|h| &h.id).collect();
+
+    // 不变量 1：tombstoned id 不可见。
+    for did in &deleted_ids {
+        assert!(
+            !hit_ids.contains(did),
+            "tombstoned id visible after compact: {}",
+            did
+        );
+    }
+    // 不变量 2：hit id 全已知（无 phantom）。
+    for hid in &hit_ids {
+        assert!(added_ids.contains(hid), "unknown id after compact: {}", hid);
+    }
+    // 不变量 3：无重复 id（docid 连续——compact 后段内无重复）。
+    assert_eq!(hits.len(), hit_ids.len(), "duplicate ids after compact");
+    // 不变量 4：live docs 全可见（不丢文档）。
+    //    top_k=1000 > total live（≤20）→ search 应返回所有 live docs。
+    assert_eq!(
+        hit_ids.len(),
+        live_ids.len(),
+        "live doc count mismatch after compact: hits={} live={} (deleted={})",
+        hit_ids.len(),
+        live_ids.len(),
+        deleted_ids.len()
+    );
+    for live_id in &live_ids {
+        assert!(
+            hit_ids.contains(live_id),
+            "live id missing after compact: {}",
+            live_id
+        );
+    }
+});
diff --git a/crates/vane-fuzz/fuzz_targets/persist_roundtrip_fuzz.rs b/crates/vane-fuzz/fuzz_targets/persist_roundtrip_fuzz.rs
new file mode 100644
index 0000000..d05ef00
--- /dev/null
+++ b/crates/vane-fuzz/fuzz_targets/persist_roundtrip_fuzz.rs
@@ -0,0 +1,111 @@
+//! Fuzz target：persist round-trip 数据一致 / external_id 全回填。
+//!
+//! 输入 decode：ByteCursor → dim（1..=8）+ n_docs（1..=9）+ query 向量
+//!   + 各 doc 的 text+vector。
+//! 流程：open → add → flush → search（基线）→ close → reopen → search（对照）。
+//! 不变量：reopen 后 topK 合法、score 非 NaN、hit id 全在原 id 集合（external_id
+//!   回填后可读）、reopen 前后 id 集合相同（round-trip 一致）。
+//!
+//! 设计 §3.2 target 表第 3 行。MemoryVfs 保跨 open 调用数据持久（虚拟持久化）。
+
+#![no_main]
+
+mod common;
+
+use std::sync::Arc;
+
+use libfuzzer::fuzz_target;
+
+use common::{build_schema, ByteCursor};
+use vane_core::api::{
+    CollectionOptions, Db, Doc, FusionSpec, OpenOptions, SearchMode, SearchQuery,
+};
+use vane_core::vfs::memory::MemoryVfs;
+use vane_core::vfs::Vfs;
+
+fuzz_target!(|data: &[u8]| {
+    let mut c = ByteCursor::new(data);
+
+    let dim = (c.u8() as u32).max(1).min(8);
+    let n_docs = (c.u8() as usize % 9) + 1; // 1..=9
+                                            // 先捕获 query 向量（reopen 后复用同一向量 → round-trip 可比）。
+    let query_vec = c.f32_vec(dim as usize);
+
+    let vfs = Arc::new(MemoryVfs::new()) as Arc<dyn Vfs>;
+    let schema = build_schema(true, dim);
+    let original_ids: Vec<String> = (0..n_docs).map(|i| format!("d{i}")).collect();
+
+    // Phase 1：open → add → flush → search（基线）→ close。
+    let baseline_id_set = {
+        let db = Db::open(vfs.clone(), "db", OpenOptions::default()).expect("Db::open");
+        let col = db
+            .collection("c", schema.clone(), CollectionOptions::default())
+            .expect("collection create");
+        let docs: Vec<Doc> = (0..n_docs)
+            .map(|i| Doc {
+                id: format!("d{i}"),
+                text: Some(c.small_string()),
+                vector: Some(c.f32_vec(dim as usize)),
+                meta: None,
+            })
+            .collect();
+        let _ = col.add(&docs);
+        let _ = col.flush();
+        let query = SearchQuery {
+            text: None,
+            vector: Some(query_vec.clone()),
+            top_k: n_docs as u32,
+            mode: SearchMode::Vector,
+            fusion: FusionSpec::Rrf,
+            filter: None,
+            candidate_multiplier: 3,
+        };
+        let hits = col.search(&query).unwrap_or_default();
+        for h in &hits {
+            assert!(!h.score.is_nan(), "baseline NaN score");
+            assert!(
+                original_ids.contains(&h.id),
+                "baseline unknown id: {}",
+                h.id
+            );
+        }
+        let id_set: std::collections::HashSet<String> = hits.iter().map(|h| h.id.clone()).collect();
+        let _ = db.close();
+        id_set
+    };
+
+    // Phase 2：reopen → search（对照）→ close。
+    {
+        let db = Db::open(vfs.clone(), "db", OpenOptions::default()).expect("Db::reopen");
+        let col = db
+            .collection("c", schema, CollectionOptions::default())
+            .expect("collection reopen");
+        let query = SearchQuery {
+            text: None,
+            vector: Some(query_vec.clone()),
+            top_k: n_docs as u32,
+            mode: SearchMode::Vector,
+            fusion: FusionSpec::Rrf,
+            filter: None,
+            candidate_multiplier: 3,
+        };
+        let hits = col.search(&query).unwrap_or_default();
+        // 不变量 1：topK 合法。
+        assert!(hits.len() <= n_docs, "reopen topK overflow");
+        // 不变量 2：score 非 NaN。
+        for h in &hits {
+            assert!(!h.score.is_nan(), "reopen NaN score");
+            // 不变量 3：external_id 全回填 —— hit id 必在原 id 集合。
+            assert!(original_ids.contains(&h.id), "reopen unknown id: {}", h.id);
+        }
+        // 不变量 4：round-trip id 集合一致（关闭前后 search 返回同一 id 集）。
+        let reopened_id_set: std::collections::HashSet<String> =
+            hits.iter().map(|h| h.id.clone()).collect();
+        assert_eq!(
+            baseline_id_set, reopened_id_set,
+            "round-trip id set mismatch: baseline={:?} reopened={:?}",
+            baseline_id_set, reopened_id_set
+        );
+        let _ = db.close();
+    }
+});
