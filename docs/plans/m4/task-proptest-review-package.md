## Commits 9e262db..f849c7b (1b proptest)

f849c7b test(core): proptest 3 不变量（检索稳定/round-trip/merge 不丢）（M4 阶段一 b）

## Diff stat

 Cargo.lock                                     | 206 +++++++++++-
 crates/vane-core/Cargo.toml                    |   6 +
 crates/vane-core/proptest-regressions/.gitkeep |   0
 crates/vane-core/tests/proptest_invariants.rs  | 421 +++++++++++++++++++++++++
 4 files changed, 631 insertions(+), 2 deletions(-)

## Full diff (U10)

diff --git a/Cargo.lock b/Cargo.lock
index 0128c32..2fb789e 100644
--- a/Cargo.lock
+++ b/Cargo.lock
@@ -39,20 +39,35 @@ dependencies = [
  "quote",
  "syn 3.0.3",
 ]
 
 [[package]]
 name = "autocfg"
 version = "1.5.1"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "f2032f911046de80f0a198e0901378627c33f59ea0ac00e363d481118bd70a53"
 
+[[package]]
+name = "bit-set"
+version = "0.8.0"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "08807e080ed7f9d5433fa9b275196cfc35414f66a0c79d864dc51a0d825231a3"
+dependencies = [
+ "bit-vec",
+]
+
+[[package]]
+name = "bit-vec"
+version = "0.8.0"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "5e764a1d40d510daf35e07be9eb06e75770908c27d411ee6c92109c9840eaaf7"
+
 [[package]]
 name = "bitflags"
 version = "2.13.1"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "b588b76d00fde79687d7646a9b5bdf3cc0f655e0bbd080335a95d7e96f3587da"
 
 [[package]]
 name = "block-buffer"
 version = "0.10.4"
 source = "registry+https://github.com/rust-lang/crates.io-index"
@@ -276,26 +291,48 @@ dependencies = [
  "block-buffer",
  "crypto-common",
 ]
 
 [[package]]
 name = "either"
 version = "1.17.0"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "9e5e8f6c15a24b9a3ee5efec809ccd006d3b30e8b3bb63c39af737c7f87daa1d"
 
+[[package]]
+name = "errno"
+version = "0.3.14"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "39cab71617ae0d63f51a36d69f866391735b51691dbda63cf6f96d042b63efeb"
+dependencies = [
+ "libc",
+ "windows-sys",
+]
+
+[[package]]
+name = "fastrand"
+version = "2.5.0"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "da7c62ceae207dd37ea5b845da6a0696c799f85e97da1ab5b7910be3c1c80223"
+
 [[package]]
 name = "find-msvc-tools"
 version = "0.1.10"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "26b73573e6edcd2af0cdf47bd6cb58f0b3839491263c314eaad1ccf24430e1de"
 
+[[package]]
+name = "fnv"
+version = "1.0.7"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "3f9eec918d3f24069decb9af1554cad7c880e2da24a9afd88aca000531ab82c1"
+
 [[package]]
 name = "futures"
 version = "0.3.33"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "a88cf1f829d945f548cf8fec32c61b1f202b6d93b45848602fc02af4b12ad218"
 dependencies = [
  "futures-channel",
  "futures-core",
  "futures-executor",
  "futures-io",
@@ -380,29 +417,41 @@ dependencies = [
 [[package]]
 name = "generic-array"
 version = "0.14.7"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "85649ca51fd72272d7821adaf274ad91c288277713d9c18820d8499a7ff69e9a"
 dependencies = [
  "typenum",
  "version_check",
 ]
 
+[[package]]
+name = "getrandom"
+version = "0.3.4"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "899def5c37c4fd7b2664648c28120ecec138e4d395b459e5ca34f9cce2dd77fd"
+dependencies = [
+ "cfg-if",
+ "libc",
+ "r-efi 5.3.0",
+ "wasip2",
+]
+
 [[package]]
 name = "getrandom"
 version = "0.4.3"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "300e883d756b2e4ec94e02791f39b04b522276138852cfc41d9fb7e904106099"
 dependencies = [
  "cfg-if",
  "libc",
- "r-efi",
+ "r-efi 6.0.0",
 ]
 
 [[package]]
 name = "half"
 version = "2.7.1"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "6ea2d84b969582b4b1864a92dc5d27cd2b77b622a8d79306834f1be5ba20d84b"
 dependencies = [
  "cfg-if",
  "crunchy",
@@ -440,21 +489,21 @@ name = "itoa"
 version = "1.0.18"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "8f42a60cbdf9a97f5d2305f08a87dc4e09308d1276d28c869c684d7777685682"
 
 [[package]]
 name = "jobserver"
 version = "0.1.35"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "1c00acbd29eabad4a2392fa0e921c874934dbbf4194312ad20f04a0ed67a3cb3"
 dependencies = [
- "getrandom",
+ "getrandom 0.4.3",
  "libc",
 ]
 
 [[package]]
 name = "js-sys"
 version = "0.3.104"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "0e0c1080212aad755ea003d18543e8768dd432c48819efd73a7bf1e39b7a5a3a"
 dependencies = [
  "cfg-if",
@@ -487,20 +536,26 @@ dependencies = [
  "cfg-if",
  "windows-link",
 ]
 
 [[package]]
 name = "libm"
 version = "0.2.16"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "b6d2cec3eae94f9f509c767b45932f1ada8350c4bdb85af2fcab4a3c14807981"
 
+[[package]]
+name = "linux-raw-sys"
+version = "0.12.1"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "32a66949e030da00e8c7d4434b251670a91556f4144941d37452769c25d58a53"
+
 [[package]]
 name = "memchr"
 version = "2.8.3"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "cf8baf1c55e62ffcace7a9f06f4bd9cd3f0c4beb022d3b367256b91b87513d98"
 
 [[package]]
 name = "minicov"
 version = "0.3.8"
 source = "registry+https://github.com/rust-lang/crates.io-index"
@@ -640,44 +695,122 @@ checksum = "df42e13c12958a16b3f7f4386b9ab1f3e7933914ecea48da7139435263a4172a"
 
 [[package]]
 name = "plotters-svg"
 version = "0.3.7"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "51bae2ac328883f7acdfea3d66a7c35751187f870bc81f94563733a154d7a670"
 dependencies = [
  "plotters-backend",
 ]
 
+[[package]]
+name = "ppv-lite86"
+version = "0.2.21"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "85eae3c4ed2f50dcfe72643da4befc30deadb458a9b590d720cde2f2b1e97da9"
+dependencies = [
+ "zerocopy",
+]
+
 [[package]]
 name = "proc-macro2"
 version = "1.0.107"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "985e7ec9bb745e6ce6535b544d84d6cd6f7ad8bd711c398938ae983b91a766d9"
 dependencies = [
  "unicode-ident",
 ]
 
+[[package]]
+name = "proptest"
+version = "1.11.0"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "4b45fcc2344c680f5025fe57779faef368840d0bd1f42f216291f0dc4ace4744"
+dependencies = [
+ "bit-set",
+ "bit-vec",
+ "bitflags",
+ "num-traits",
+ "rand",
+ "rand_chacha",
+ "rand_xorshift",
+ "regex-syntax",
+ "rusty-fork",
+ "tempfile",
+ "unarray",
+]
+
+[[package]]
+name = "quick-error"
+version = "1.2.3"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "a1d01941d82fa2ab50be1e79e6714289dd7cde78eba4c074bc5a4374f650dfe0"
+
 [[package]]
 name = "quote"
 version = "1.0.47"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "1fbf4db142a473a8d80c26bbf18454ed458bf8d26c8219c331daecfdbd079001"
 dependencies = [
  "proc-macro2",
 ]
 
+[[package]]
+name = "r-efi"
+version = "5.3.0"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "69cdb34c158ceb288df11e18b4bd39de994f6657d83847bdffdbd7f346754b0f"
+
 [[package]]
 name = "r-efi"
 version = "6.0.0"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "f8dcc9c7d52a811697d2151c701e0d08956f92b0e24136cf4cf27b57a6a0d9bf"
 
+[[package]]
+name = "rand"
+version = "0.9.5"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "b9ef1d0d795eb7d84685bca4f72f3649f064e6641543d3a8c415898726a57b41"
+dependencies = [
+ "rand_chacha",
+ "rand_core",
+]
+
+[[package]]
+name = "rand_chacha"
+version = "0.9.0"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "d3022b5f1df60f26e1ffddd6c66e8aa15de382ae63b3a0c1bfc0e4d3e3f325cb"
+dependencies = [
+ "ppv-lite86",
+ "rand_core",
+]
+
+[[package]]
+name = "rand_core"
+version = "0.9.5"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "76afc826de14238e6e8c374ddcc1fa19e374fd8dd986b0d2af0d02377261d83c"
+dependencies = [
+ "getrandom 0.3.4",
+]
+
+[[package]]
+name = "rand_xorshift"
+version = "0.4.0"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "513962919efc330f829edb2535844d1b912b0fbe2ca165d613e4e8788bb05a5a"
+dependencies = [
+ "rand_core",
+]
+
 [[package]]
 name = "rayon"
 version = "1.12.0"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "fb39b166781f92d482534ef4b4b1b2568f42613b53e5b6c160e24cfbfa30926d"
 dependencies = [
  "either",
  "rayon-core",
 ]
 
@@ -739,26 +872,51 @@ dependencies = [
  "serde",
  "serde_derive",
 ]
 
 [[package]]
 name = "rustc-hash"
 version = "2.1.3"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "6b1e7f9a428571be2dc5bc0505c13fb6bf936822b894ec87abf8a08a4e51742d"
 
+[[package]]
+name = "rustix"
+version = "1.1.4"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "b6fe4565b9518b83ef4f91bb47ce29620ca828bd32cb7e408f0062e9930ba190"
+dependencies = [
+ "bitflags",
+ "errno",
+ "libc",
+ "linux-raw-sys",
+ "windows-sys",
+]
+
 [[package]]
 name = "rustversion"
 version = "1.0.23"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "cf54715a573b99ac80df0bc206da022bcd442c974952c7b9720069370852e21f"
 
+[[package]]
+name = "rusty-fork"
+version = "0.3.1"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "cc6bf79ff24e648f6da1f8d1f011e9cac26491b619e6b9280f2b47f1774e6ee2"
+dependencies = [
+ "fnv",
+ "quick-error",
+ "tempfile",
+ "wait-timeout",
+]
+
 [[package]]
 name = "ruzstd"
 version = "0.5.0"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "58c4eb8a81997cf040a091d1f7e1938aeab6749d3a0dfa73af43cdc32393483d"
 dependencies = [
  "byteorder",
  "derive_more",
  "twox-hash",
 ]
@@ -865,20 +1023,33 @@ dependencies = [
 name = "syn"
 version = "3.0.3"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "53e9bae58849f64dfa4f5d5ae372c8341f7305f82a3868709269343628b659a3"
 dependencies = [
  "proc-macro2",
  "quote",
  "unicode-ident",
 ]
 
+[[package]]
+name = "tempfile"
+version = "3.27.0"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "32497e9a4c7b38532efcdebeef879707aa9f794296a4f0244f6f69e9bc8574bd"
+dependencies = [
+ "fastrand",
+ "getrandom 0.4.3",
+ "once_cell",
+ "rustix",
+ "windows-sys",
+]
+
 [[package]]
 name = "tinytemplate"
 version = "1.2.1"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "be4d6b5f19ff7664e8c98d03e2139cb510db9b0a60b55f8e8709b689d939b6bc"
 dependencies = [
  "serde",
  "serde_json",
 ]
 
@@ -900,37 +1071,44 @@ checksum = "b6f5e870be6c3b371b77fe0ee0bafb859fa4964b4404c27de1d380043c4dda20"
 
 [[package]]
 name = "ulid"
 version = "1.2.1"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "470dbf6591da1b39d43c14523b2b469c86879a53e8b758c8e090a470fe7b1fbe"
 dependencies = [
  "web-time",
 ]
 
+[[package]]
+name = "unarray"
+version = "0.1.4"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "eaea85b334db583fe3274d12b4cd1880032beab409c0d774be044d4480ab9a94"
+
 [[package]]
 name = "unicode-ident"
 version = "1.0.24"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "e6e4313cd5fcd3dad5cafa179702e2b244f760991f45397d14d4ebf38247da75"
 
 [[package]]
 name = "unicode-segmentation"
 version = "1.13.3"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "c6f5d3c3b1bf09027a88a6bc961fc00497d651009560b5463668dc81b0fa87a8"
 
 [[package]]
 name = "vane-core"
 version = "0.2.0"
 dependencies = [
  "criterion",
+ "proptest",
  "rayon",
  "roaring",
  "rust-stemmers",
  "ruzstd",
  "serde",
  "serde_json",
  "sha2",
  "ulid",
  "unicode-segmentation",
  "vane-dict-zh",
@@ -993,30 +1171,48 @@ dependencies = [
  "wasm-bindgen-test",
  "web-sys",
 ]
 
 [[package]]
 name = "version_check"
 version = "0.9.5"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "0b928f33d975fc6ad9f86c8f283853ad26bdd5b10b7f1542aa2fa15e2289105a"
 
+[[package]]
+name = "wait-timeout"
+version = "0.2.1"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "09ac3b126d3914f9849036f826e054cbabdc8519970b8998ddaf3b5bd3c65f11"
+dependencies = [
+ "libc",
+]
+
 [[package]]
 name = "walkdir"
 version = "2.5.0"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "29790946404f91d9c5d06f9874efddea1dc06c5efe94541a7d6863108e3a5e4b"
 dependencies = [
  "same-file",
  "winapi-util",
 ]
 
+[[package]]
+name = "wasip2"
+version = "1.0.4+wasi-0.2.12"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "b67efb37e106e55ce722a510d6b5f9c17f083e5fc79afc2badeb12cc313d9487"
+dependencies = [
+ "wit-bindgen",
+]
+
 [[package]]
 name = "wasm-bindgen"
 version = "0.2.127"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "1b70935747edd64d89de3efa29d73789b806c15798f8e7dca4d8ac356b50ce70"
 dependencies = [
  "cfg-if",
  "once_cell",
  "rustversion",
  "wasm-bindgen-macro",
@@ -1141,20 +1337,26 @@ checksum = "f0805222e57f7521d6a62e36fa9163bc891acd422f971defe97d64e70d0a4fe5"
 
 [[package]]
 name = "windows-sys"
 version = "0.61.2"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "ae137229bcbd6cdf0f7b80a31df61766145077ddf49416a728b02cb3921ff3fc"
 dependencies = [
  "windows-link",
 ]
 
+[[package]]
+name = "wit-bindgen"
+version = "0.57.1"
+source = "registry+https://github.com/rust-lang/crates.io-index"
+checksum = "1ebf944e87a7c253233ad6766e082e3cd714b5d03812acc24c318f549614536e"
+
 [[package]]
 name = "zerocopy"
 version = "0.8.56"
 source = "registry+https://github.com/rust-lang/crates.io-index"
 checksum = "556764e583adb45a9f8d413c2a147fa7e8d821e48e12b14fd560b607998b75eb"
 dependencies = [
  "zerocopy-derive",
 ]
 
 [[package]]
diff --git a/crates/vane-core/Cargo.toml b/crates/vane-core/Cargo.toml
index 7f384aa..a23cd2a 100644
--- a/crates/vane-core/Cargo.toml
+++ b/crates/vane-core/Cargo.toml
@@ -59,20 +59,26 @@ sq8 = []
 # cfg(target_arch) 仅在 executor/mod.rs（I-5 不变量核心）。
 executor-native = ["dep:rayon"]
 # fault-injection：FaultVfs 故障注入 VFS（M4 §3.1）。
 # dev/optional，默认不启用。cfg(test) 或本 feature 启用时编译 fault.rs，
 # 供崩溃恢复测试精确模拟 IO 错误 / 部分写 / ENOSPC / 延迟。
 # 绝不进生产/wasm 二进制——wasm32 check 不启此 feature、不设 test cfg。
 fault-injection = []
 
 [dev-dependencies]
 criterion = "0.5"
+# proptest：property-based 不变量测试（M4 §3.3）。
+# dev-dep，不进 wasm/native 生产构建（wasm32 check 不含 dev-deps）。
+# 传递依赖无黑名单项（regex/tokio/prost/tonic/openssl/lindera/ndarray/
+# wee_alloc/dashmap/parking_lot）——默认 features 不含 regex/regex-syntax，
+# cargo deny check 守护。Strategy 用 a-z 字符生成（非 string_regex），零 regex 路径。
+proptest = "1"
 
 [[bench]]
 name = "hybrid_search"
 harness = false
 
 [[bench]]
 name = "batch_add"
 harness = false
 
 [[bench]]
diff --git a/crates/vane-core/proptest-regressions/.gitkeep b/crates/vane-core/proptest-regressions/.gitkeep
new file mode 100644
index 0000000..e69de29
diff --git a/crates/vane-core/tests/proptest_invariants.rs b/crates/vane-core/tests/proptest_invariants.rs
new file mode 100644
index 0000000..6400434
--- /dev/null
+++ b/crates/vane-core/tests/proptest_invariants.rs
@@ -0,0 +1,421 @@
+// tests/proptest_invariants.rs — M4 阶段一 b：proptest property-based 不变量测试
+//
+// 设计 §3.3：3 不变量（检索排序稳定合法 / persist round-trip 一致 / merge 不丢文档）。
+// proptest! 宏将测试体包入传给 TestRunner::run 的闭包，rustc dead_code 分析
+// 无法穿越闭包追踪 helper 调用 → 文件级 allow(dead_code) 消除假告警（helper 实际
+// 被闭包内的测试体调用）。不影响 clippy 其他门禁。
+#![allow(dead_code)]
+// proptest 默认 256 cases，失败 seed 持久化到 proptest-regressions/ 确保 CI 复现。
+//
+// proptest 是 dev-dep，不进 wasm/native 生产构建（wasm32 check 不含 dev-deps）。
+// 传递依赖无黑名单项（regex/tokio/prost/tonic/openssl/lindera/ndarray/
+// wee_alloc/dashmap/parking_lot），cargo deny check 守护。
+//
+// Strategy 设计（§3.3 骨架 + 零 regex 路径）：
+// - arb_letter/arb_word/arb_text：a-z 字符生成，绕开 proptest string_regex 的
+//   regex-syntax 可选依赖路径（默认 features 不启用 regex feature）。
+// - arb_finite_f32/arb_vector：f32 NaN/Inf 过滤 + 非全零（避 cosine 0/0 退化 NaN score）。
+// - Strategy 返回 Debug 可格式化元组（Doc/SearchQuery 未 derive Debug，proptest! 宏
+//   需值类型实现 Debug 以打印失败输入），测试体内构造 API 类型。
+//
+// 不变量 3 用 search_brute_baseline（非 HNSW 近似）验证活文档全集，避假红/绿
+// （1a merge_fuzz review M2 建议）。
+
+use proptest::prelude::*;
+use std::sync::Arc;
+
+use vane_core::api::{
+    CollectionOptions, Db, Doc, FusionSpec, OpenOptions, ScalarValue, SearchMode, SearchQuery,
+};
+use vane_core::types::{FieldDef, Metric, ScalarKind, Schema};
+use vane_core::vfs::memory::MemoryVfs;
+
+/// 维度——小 dim 保 CI 速度（256 cases × 多 mode × round-trip + merge）。
+const DIM: usize = 4;
+/// 单批次最大文档数——小规模保 proptest 速度（round-trip 每例 close+reopen，
+/// 256 cases × MAX_DOCS=8 约 50s），足够覆盖排序/round-trip/merge 不变量。
+const MAX_DOCS: usize = 8;
+
+// ---------------------------------------------------------------------------
+// Strategy 设计
+// ---------------------------------------------------------------------------
+
+/// 生成 a-z 随机字符（零 regex 依赖——绕开 proptest string_regex 的 regex-syntax 路径）。
+fn arb_letter() -> impl Strategy<Value = char> {
+    (0u8..26u8).prop_map(|i| char::from(b'a' + i))
+}
+
+/// 生成 1..max_len 长度的 a-z 字符串。
+fn arb_word(max_len: usize) -> impl Strategy<Value = String> {
+    prop::collection::vec(arb_letter(), 1..max_len).prop_map(|cs| cs.into_iter().collect())
+}
+
+/// 生成空格分隔的 a-z 文本（1..max_words 个词，每词 1..max_word_len 字符）。
+fn arb_text(max_words: usize, max_word_len: usize) -> impl Strategy<Value = String> {
+    prop::collection::vec(arb_word(max_word_len), 1..max_words).prop_map(|ws| ws.join(" "))
+}
+
+/// 有限 f32（过滤 NaN/Inf，避 score 退化与排序异常）。
+fn arb_finite_f32() -> impl Strategy<Value = f32> {
+    prop::num::f32::ANY.prop_filter("finite", |x| x.is_finite())
+}
+
+/// 随机向量（dim 维，有限值，非全零避 cosine 0/0 退化 NaN score）。
+fn arb_vector(dim: usize) -> impl Strategy<Value = Vec<f32>> {
+    prop::collection::vec(arb_finite_f32(), dim..=dim)
+        .prop_filter("not_all_zero", |v| v.iter().any(|x| *x != 0.0))
+}
+
+/// 随机查询组件（text + vector + topK + mode）。返回 Debug 元组，测试体内构造 SearchQuery。
+fn arb_query_components(dim: usize) -> impl Strategy<Value = (String, Vec<f32>, u32, SearchMode)> {
+    (
+        arb_text(4, 8),
+        arb_vector(dim),
+        1u32..=8u32,
+        prop_oneof![
+            Just(SearchMode::Hybrid),
+            Just(SearchMode::Vector),
+            Just(SearchMode::Text),
+        ],
+    )
+}
+
+/// 随机文档体组件批次（text + vector + tag char）。返回 Debug 元组 Vec，
+/// 测试体内构造 Doc（顺序 id d0..d{n-1}，保 id 唯一；含 meta tag scalar 供 stored_json round-trip）。
+fn arb_doc_bodies(
+    dim: usize,
+    max_docs: usize,
+) -> impl Strategy<Value = Vec<(String, Vec<f32>, char)>> {
+    prop::collection::vec((arb_text(8, 8), arb_vector(dim), arb_letter()), 1..max_docs)
+}
+
+/// merge 场景组件批次（text + vector + tag + delete_flag）。返回 Debug 元组 Vec，
+/// 测试体内构造 (Vec<Doc>, Vec<bool>)——并行删除标志位（同长度，一一对应）。
+fn arb_merge_bodies(
+    dim: usize,
+    max_docs: usize,
+) -> impl Strategy<Value = Vec<(String, Vec<f32>, char, bool)>> {
+    prop::collection::vec(
+        (
+            arb_text(8, 8),
+            arb_vector(dim),
+            arb_letter(),
+            prop::bool::ANY,
+        ),
+        1..max_docs,
+    )
+}
+
+/// 从组件批次构造 Doc Vec（顺序 id 保唯一 + meta tag scalar）。
+fn build_docs(bodies: &[(String, Vec<f32>, char)]) -> Vec<Doc> {
+    bodies
+        .iter()
+        .enumerate()
+        .map(|(i, (text, vec, tag))| {
+            let mut meta = std::collections::HashMap::new();
+            meta.insert("tag".to_string(), ScalarValue::Keyword(tag.to_string()));
+            Doc {
+                id: format!("d{}", i),
+                text: Some(text.clone()),
+                vector: Some(vec.clone()),
+                meta: Some(meta),
+            }
+        })
+        .collect()
+}
+
+/// 从 merge 组件批次构造 (Vec<Doc>, Vec<bool>)。
+fn build_merge_scenario(bodies: &[(String, Vec<f32>, char, bool)]) -> (Vec<Doc>, Vec<bool>) {
+    let docs: Vec<Doc> = bodies
+        .iter()
+        .enumerate()
+        .map(|(i, (text, vec, tag, _))| {
+            let mut meta = std::collections::HashMap::new();
+            meta.insert("tag".to_string(), ScalarValue::Keyword(tag.to_string()));
+            Doc {
+                id: format!("d{}", i),
+                text: Some(text.clone()),
+                vector: Some(vec.clone()),
+                meta: Some(meta),
+            }
+        })
+        .collect();
+    let delete_flags: Vec<bool> = bodies.iter().map(|(_, _, _, del)| *del).collect();
+    (docs, delete_flags)
+}
+
+/// 从组件构造 SearchQuery。
+fn build_query((text, vector, top_k, mode): (String, Vec<f32>, u32, SearchMode)) -> SearchQuery {
+    SearchQuery {
+        text: Some(text),
+        vector: Some(vector),
+        top_k,
+        mode,
+        fusion: FusionSpec::Rrf,
+        filter: None,
+        candidate_multiplier: 3,
+    }
+}
+
+fn build_schema(dim: usize) -> Schema {
+    Schema::new(vec![
+        ("body".into(), FieldDef::Text),
+        (
+            "v".into(),
+            FieldDef::Vector {
+                dim: dim as u32,
+                metric: Metric::Cosine,
+            },
+        ),
+        (
+            "tag".into(),
+            FieldDef::Scalar {
+                kind: ScalarKind::Keyword,
+            },
+        ),
+    ])
+    .unwrap()
+}
+
+/// 全量 vector 查询（topK=n，取回所有文档）。
+fn vector_query_all(n: usize) -> SearchQuery {
+    SearchQuery {
+        text: None,
+        vector: Some(vec![1.0; DIM]),
+        top_k: n as u32,
+        mode: SearchMode::Vector,
+        fusion: FusionSpec::Rrf,
+        filter: None,
+        candidate_multiplier: 3,
+    }
+}
+
+/// 捕获 (id, score, tag) 三元组用于一致性比对（tag 来自 stored.bin meta JSON）。
+fn capture(hits: &[vane_core::api::Hit]) -> Vec<(String, f32, Option<String>)> {
+    hits.iter()
+        .map(|h| {
+            let tag = h.fields.as_ref().and_then(|f| f.get("tag")).cloned();
+            (h.id.clone(), h.score, tag)
+        })
+        .collect()
+}
+
+// ---------------------------------------------------------------------------
+// 不变量 1：检索排序稳定合法
+// ---------------------------------------------------------------------------
+
+proptest! {
+    #[test]
+    fn search_returns_stable_topk(
+        bodies in arb_doc_bodies(DIM, MAX_DOCS),
+        q_components in arb_query_components(DIM),
+    ) {
+        let docs = build_docs(&bodies);
+        let q = build_query(q_components);
+
+        let vfs = Arc::new(MemoryVfs::new());
+        let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
+        let col = db
+            .collection("docs", build_schema(DIM), CollectionOptions::default())
+            .unwrap();
+        col.add(&docs).unwrap();
+        col.flush().unwrap();
+
+        let hits1 = col.search(&q).unwrap();
+        let hits2 = col.search(&q).unwrap();
+
+        // 不变量 1a：结果数 ≤ min(topK, total_docs)。
+        let upper = (q.top_k as usize).min(docs.len());
+        prop_assert!(
+            hits1.len() <= upper,
+            "hits1.len() {} exceeds min(topK={}, total={})",
+            hits1.len(), q.top_k, docs.len()
+        );
+        prop_assert_eq!(hits1.len(), hits2.len(), "same query must return same count");
+
+        // 不变量 1b：score 单调非递增，且全部有限。
+        for w in hits1.windows(2) {
+            prop_assert!(
+                w[0].score >= w[1].score,
+                "scores not monotonically non-increasing: {} then {}",
+                w[0].score, w[1].score
+            );
+        }
+        for h in &hits1 {
+            prop_assert!(
+                h.score.is_finite(),
+                "score not finite: id={} score={}", h.id, h.score
+            );
+        }
+
+        // 不变量 1c：同 query 二次检索 (id, score, tag) 完全一致。
+        let cap1 = capture(&hits1);
+        let cap2 = capture(&hits2);
+        prop_assert_eq!(cap1, cap2, "same query must yield identical (id, score, tag) sequence");
+    }
+}
+
+// ---------------------------------------------------------------------------
+// 不变量 2：persist round-trip 一致
+// ---------------------------------------------------------------------------
+
+proptest! {
+    #[test]
+    fn persist_roundtrip_consistent(
+        bodies in arb_doc_bodies(DIM, MAX_DOCS),
+    ) {
+        let docs = build_docs(&bodies);
+        let total = docs.len();
+        let vfs = Arc::new(MemoryVfs::new());
+
+        // 第一次 open：建库 + 灌数据 + flush + 基线搜索 + close。
+        let baseline = {
+            let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
+            let col = db
+                .collection("docs", build_schema(DIM), CollectionOptions::default())
+                .unwrap();
+            col.add(&docs).unwrap();
+            col.flush().unwrap();
+
+            // 全量基线：vector 模式 topK=total，取回所有文档。
+            let q = vector_query_all(total);
+            let hits = col.search(&q).unwrap();
+            // 期望全部文档可见（无 delete）。
+            prop_assert_eq!(
+                hits.len(), total,
+                "baseline must return all {} docs, got {}", total, hits.len()
+            );
+            let baseline = capture(&hits);
+            db.close().unwrap();
+            baseline
+        };
+
+        // 第二次 open：同 vfs，验证 manifest/segment/WAL 恢复。
+        let db2 = Db::open(vfs, "db", OpenOptions::default()).unwrap();
+        prop_assert!(
+            db2.collections().iter().any(|c| c == "docs"),
+            "collection 'docs' not restored after reopen"
+        );
+        let col2 = db2
+            .collection("docs", build_schema(DIM), CollectionOptions::default())
+            .unwrap();
+        let q = vector_query_all(total);
+        let hits2 = col2.search(&q).unwrap();
+
+        // 不变量 2a：external_id 全回填——结果数 == total，且 id 集合等于原文档 id 集合。
+        prop_assert_eq!(
+            hits2.len(), total,
+            "reopen must return all {} docs, got {}", total, hits2.len()
+        );
+        let expected_ids: std::collections::HashSet<String> =
+            docs.iter().map(|d| d.id.clone()).collect();
+        let got_ids: std::collections::HashSet<String> =
+            hits2.iter().map(|h| h.id.clone()).collect();
+        prop_assert_eq!(got_ids, expected_ids, "external_id set mismatch after reopen");
+
+        // 不变量 2b：stored tag 一致——每条 hit 的 tag 字段非空且为合法 JSON 字符串
+        // （stored.bin meta JSON round-trip；单字符 tag 回填为 "\"x\"" 3 字符）。
+        for h in &hits2 {
+            let tag = h.fields.as_ref().and_then(|f| f.get("tag"));
+            prop_assert!(tag.is_some(), "stored tag missing for id={} after reopen", h.id);
+            let t = tag.unwrap();
+            prop_assert!(
+                t.len() >= 3 && t.starts_with('"') && t.ends_with('"'),
+                "stored tag not a JSON string: {}", t
+            );
+        }
+
+        // 不变量 2c：search 结果集 (id, score, tag) 与基线完全一致。
+        let after = capture(&hits2);
+        prop_assert_eq!(after, baseline, "round-trip (id, score, tag) differs from baseline");
+
+        db2.close().unwrap();
+    }
+}
+
+// ---------------------------------------------------------------------------
+// 不变量 3：merge 不丢文档
+// ---------------------------------------------------------------------------
+
+proptest! {
+    #[test]
+    fn merge_preserves_live_docs(
+        bodies in arb_merge_bodies(DIM, MAX_DOCS),
+        chunk_size in 1u32..=4u32,
+    ) {
+        let (docs, delete_flags) = build_merge_scenario(&bodies);
+        let total = docs.len();
+
+        let vfs = Arc::new(MemoryVfs::new());
+        let db = Db::open(vfs, "db", OpenOptions::default()).unwrap();
+        let col = db
+            .collection("docs", build_schema(DIM), CollectionOptions::default())
+            .unwrap();
+
+        // 多段灌入：按 chunk_size 分批 add+flush，制造多个段。
+        for chunk_docs in docs.chunks(chunk_size as usize) {
+            col.add(chunk_docs).unwrap();
+            col.flush().unwrap();
+        }
+
+        // 删除标志位对应的文档。
+        let delete_ids: Vec<String> = docs
+            .iter()
+            .zip(delete_flags.iter())
+            .filter(|(_, &del)| del)
+            .map(|(d, _)| d.id.clone())
+            .collect();
+        if !delete_ids.is_empty() {
+            col.delete(&delete_ids).unwrap();
+        }
+
+        // 期望活文档集合（未被 delete 的）。
+        let expected_live: std::collections::HashSet<String> = docs
+            .iter()
+            .zip(delete_flags.iter())
+            .filter(|(_, &del)| !del)
+            .map(|(d, _)| d.id.clone())
+            .collect();
+
+        // compact 合并所有段 + 物理清 tombstone。
+        col.compact().unwrap();
+
+        // 用 brute baseline（Vector 模式，topK=total）验证活文档全集——
+        // 绕过 HNSW 近似，确保 docid 不重叠/不丢失的确定性验证。
+        let q = vector_query_all(total);
+        let hits = col.search_brute_baseline(&q).unwrap();
+        let hit_ids: std::collections::HashSet<&String> =
+            hits.iter().map(|h| &h.id).collect();
+
+        // 不变量 3a：活文档全可见——结果数 == 期望活文档数，且 id 集合相等。
+        prop_assert_eq!(
+            hits.len(),
+            expected_live.len(),
+            "merge lost docs: got {} hits, expected {} live (deleted={}, total={})",
+            hits.len(), expected_live.len(), delete_ids.len(), total
+        );
+        for id in &expected_live {
+            prop_assert!(
+                hit_ids.contains(id),
+                "live doc {} not visible after merge+compact", id
+            );
+        }
+
+        // 不变量 3b：tombstoned 文档不可见。
+        for id in &delete_ids {
+            prop_assert!(
+                !hit_ids.contains(id),
+                "tombstoned doc {} visible after merge+compact", id
+            );
+        }
+
+        // 不变量 3c：无重复 docid——hits.len() == unique id count。
+        prop_assert_eq!(
+            hits.len(),
+            hit_ids.len(),
+            "duplicate docids: {} hits, {} unique ids", hits.len(), hit_ids.len()
+        );
+
+        db.close().unwrap();
+    }
+}
