# Task 6 Review Package（6857c70..HEAD）

## Commits
d750558 feat(dict): Task 6 扩展词典哈希校验至四渠道（加 Web npm dictData 通道）

## Diff stat
 crates/vane-dict-zh/tests/dict_test.rs |  40 +++++++++++++
 scripts/check-dict-hash.sh             | 101 +++++++++++++++++++++++++++------
 2 files changed, 125 insertions(+), 16 deletions(-)

## 完整 diff
diff --git a/crates/vane-dict-zh/tests/dict_test.rs b/crates/vane-dict-zh/tests/dict_test.rs
index 8a216df..75c5bfe 100644
--- a/crates/vane-dict-zh/tests/dict_test.rs
+++ b/crates/vane-dict-zh/tests/dict_test.rs
@@ -92,16 +92,56 @@ fn sha256_prefix_is_real_sha256_of_payload() {
 fn dict_has_substantial_vocab() {
     let dict = vane_core::tokenizer::jieba::JiebaDict::load_zstd(DICT_BIN).unwrap();
     // 验证常见词可查到词频
     assert!(dict.freq("的").is_some(), "「的」应在词典中");
     assert!(dict.freq("中国").is_some(), "「中国」应在词典中");
     assert!(dict.freq("是").is_some(), "「是」应在词典中");
 }
 
+/// Task 6：第四渠道（WASM npm dictData）package.json 元数据校验。
+///
+/// @vane-rs/dict-zh npm 包的 data/dict.bin 必须与源 crates/vane-dict-zh/data/dict.bin 同源
+/// （package.json files 字段直接引用源文件路径，非拷贝）。本测试校验 package.json files +
+/// exports 配置正确，确保 npm pack 产物引用源文件 → 第四渠道与第一渠道字节一致。
+/// 字节级 npm pack 比对在 scripts/check-dict-hash.sh 中（CI 门禁）。
+#[test]
+fn npm_package_json_references_source_dict_bin() {
+    use serde_json::Value;
+
+    let pkg_path = concat!(env!("CARGO_MANIFEST_DIR"), "/package.json");
+    let pkg = std::fs::read_to_string(pkg_path).expect("read package.json");
+    let v: Value = serde_json::from_str(&pkg).expect("parse package.json");
+
+    // files 字段含 data/dict.bin + data/sha256_prefix.bin
+    let files = v["files"].as_array().expect("files is array");
+    assert!(
+        files.iter().any(|f| f.as_str() == Some("data/dict.bin")),
+        "package.json files must include data/dict.bin"
+    );
+    assert!(
+        files
+            .iter()
+            .any(|f| f.as_str() == Some("data/sha256_prefix.bin")),
+        "package.json files must include data/sha256_prefix.bin"
+    );
+
+    // exports ./dict.bin → ./data/dict.bin（确保 import 解析到源文件）
+    assert_eq!(
+        v["exports"]["./dict.bin"].as_str(),
+        Some("./data/dict.bin"),
+        "exports ./dict.bin must point to ./data/dict.bin"
+    );
+    assert_eq!(
+        v["exports"]["./sha256_prefix.bin"].as_str(),
+        Some("./data/sha256_prefix.bin"),
+        "exports ./sha256_prefix.bin must point to ./data/sha256_prefix.bin"
+    );
+}
+
 /// gzip 体积估算（与 gen_dict.rs 同方法）。
 fn gzip_size(data: &[u8]) -> usize {
     use std::process::Command;
     let tmp = std::env::temp_dir().join(format!("vane_dict_test_{}.tmp", std::process::id()));
     if std::fs::write(&tmp, data).is_err() {
         return data.len();
     }
     let out = Command::new("gzip").args(["-c", "-9"]).arg(&tmp).output();
diff --git a/scripts/check-dict-hash.sh b/scripts/check-dict-hash.sh
index 177f495..ac3a077 100755
--- a/scripts/check-dict-hash.sh
+++ b/scripts/check-dict-hash.sh
@@ -1,25 +1,39 @@
 #!/usr/bin/env bash
-# SPEC §12.3：三渠道（Node/Go/内嵌）词典版本哈希一致性校验。
+# SPEC §12.3：四渠道词典版本哈希一致性校验。
 #
-# Node 渠道（07）：vane-dict-zh crate 的 DICT_VERSION + SHA256_PREFIX_BIN。
-# Go 渠道（08 deferred）：bindings/go/dict/ 产物，待 08 落地后启用。
+# 词典分发四渠道：
+# 1. Node：vane-dict-zh cargo path 依赖 include_bytes（crates/vane-dict-zh/data/dict.bin）
+# 2. Go：bindings/go/dict/dict.bin.gz（gzip 再压缩，go:embed）
+# 3. WASM CDN：fetch jsdelivr（fallback，运行时 sha256_prefix 校验，本脚本不覆盖）
+# 4. WASM npm dictData：@vane-rs/dict-zh npm 包 data/dict.bin（Web 端 import asset url 传 dictData）
 #
 # 本脚本校验：
-# 1. Node 侧 dict.bin 的 sha256_prefix 与 sha256_prefix.bin 一致
+# 1. Node 侧 dict.bin 的 sha256_prefix 与 sha256_prefix.bin 一致（存在性 + 字节数）
 # 2. DICT_VERSION 格式合法（YYYY.MM）
-# 3. Go 侧（若有）版本哈希一致
+# 3. Go 侧版本哈希一致（gunzip ↔ Node 源字节 sha256 + DictVersion + zstd 头部 prefix）
+# 4. npm 包侧 @vane-rs/dict-zh 产物 dict.bin 与源 data/dict.bin 字节一致
+#    （package.json files + exports 元数据 + npm pack 字节级比对）
 set -euo pipefail
 
 cd "$(dirname "$0")/.."
 
-echo "=== 三渠道词典哈希一致性校验（SPEC §12.3）==="
+echo "=== 四渠道词典哈希一致性校验（SPEC §12.3）==="
 
-# --- Node 渠道 ---
+# sha256 计算工具（GNU sha256sum / BSD shasum -a 256），供 Go 与 npm 渠道共用。
+compute_sha256() {
+  if command -v sha256sum &>/dev/null; then
+    sha256sum | awk '{print $1}'
+  else
+    shasum -a 256 | awk '{print $1}'
+  fi
+}
+
+# --- 第一渠道：Node（vane-dict-zh include_bytes）---
 DICT_BIN="crates/vane-dict-zh/data/dict.bin"
 SHA_FILE="crates/vane-dict-zh/data/sha256_prefix.bin"
 
 if [ ! -f "$DICT_BIN" ]; then
   echo "FAIL: $DICT_BIN not found"
   exit 1
 fi
 
@@ -44,33 +58,26 @@ else
   # Cargo.toml version 是 semver（2026.8.0），DICT_VERSION 是 YYYY.MM（2026.08）
   # 两者格式不同但日期部分应一致
   echo "INFO: Cargo.toml version = $DICT_VERSION (semver)"
 fi
 
 # --- Rust 测试覆盖 ---
 echo "INFO: 完整 sha256 一致性校验在 Rust 测试 dict_tests.rs 中（编译期 include_bytes vs 运行时 load_zstd）"
 
-# --- Go 渠道（08 已落地：dict.bin.gz 已提交到 bindings/go/dict/）---
+# --- 第二渠道：Go（dict.bin.gz gzip 再压缩，go:embed）---
 # Go 侧用 go:embed dict.bin.gz（gzip 再压缩），解压后与 Node 侧 dict.bin 同源。
 # 校验逻辑：
 #   1. gunzip Go dict.bin.gz → 与 Node dict.bin 比对 sha256（源字节一致 → prefix 隐含一致）
 #   2. 版本一致性：Go DictVersion const vs Rust DICT_VERSION
 #   3. 若 zstd 可用：解压 dict.bin → 读头部 [8..16] 直接比对 sha256_prefix.bin
 GO_DICT_DIR="bindings/go/dict"
 GO_DICT_GZ="$GO_DICT_DIR/dict.bin.gz"
 if [ -f "$GO_DICT_GZ" ]; then
   # 1. 源字节 sha256 一致性（最强校验：字节相同 → prefix 必相同）
-  compute_sha256() {
-    if command -v sha256sum &>/dev/null; then
-      sha256sum | awk '{print $1}'
-    else
-      shasum -a 256 | awk '{print $1}'
-    fi
-  }
   GO_SHA=$(gunzip -c "$GO_DICT_GZ" | compute_sha256)
   NODE_SHA=$(compute_sha256 < "$DICT_BIN")
   echo "Go dict.bin (gunzipped) sha256: $GO_SHA"
   echo "Node dict.bin sha256:           $NODE_SHA"
   if [ "$GO_SHA" != "$NODE_SHA" ]; then
     echo "FAIL: Go ↔ Node dict.bin sha256 mismatch (SPEC §12.3)"
     exit 1
   fi
@@ -104,9 +111,71 @@ if [ -f "$GO_DICT_GZ" ]; then
     echo "OK: Go ↔ Node sha256_prefix 一致（zstd 解压头部直接比对）"
   else
     echo "INFO: zstd 不可用，跳过头部 prefix 直接比对（源字节一致已隐含证明）"
   fi
 else
   echo "SKIP: Go dict.bin.gz not found ($GO_DICT_GZ)"
 fi
 
-echo "=== 三渠道词典哈希一致性校验通过 ==="
+# --- 第四渠道：WASM npm dictData（@vane-rs/dict-zh npm 包）---
+# Task 6：Web 端通过 import dictBinUrl from '@vane-rs/dict-zh/dict.bin' 取 dict.bin 字节传 VaneWorker dictData。
+# npm 包的 data/dict.bin 就是 crates/vane-dict-zh/data/dict.bin（package.json files 字段直接引用源文件路径，
+# 非拷贝），故第四渠道与第一渠道（Node include_bytes）同源。
+# 校验：
+#   1. package.json files 含 data/dict.bin + data/sha256_prefix.bin（确保 npm 包引用源文件）
+#   2. package.json exports ./dict.bin → ./data/dict.bin + ./sha256_prefix.bin → ./data/sha256_prefix.bin
+#   3. 若 npm 可用：npm pack 实际产物 → tar 提取 dict.bin → sha256 比对（字节级最严谨）
+PKG_JSON="crates/vane-dict-zh/package.json"
+if [ ! -f "$PKG_JSON" ]; then
+  echo "FAIL: $PKG_JSON not found"
+  exit 1
+fi
+
+# 1. files 字段校验（grep -F 字面匹配，避免 . 被当正则元字符）
+if grep -Fq '"data/dict.bin"' "$PKG_JSON" && grep -Fq '"data/sha256_prefix.bin"' "$PKG_JSON"; then
+  echo "OK: npm 包 files 含 data/dict.bin + data/sha256_prefix.bin"
+else
+  echo "FAIL: npm 包 files 缺 data/dict.bin 或 data/sha256_prefix.bin"
+  exit 1
+fi
+
+# 2. exports 字段校验
+if grep -Fq '"./dict.bin": "./data/dict.bin"' "$PKG_JSON"; then
+  echo "OK: npm 包 exports ./dict.bin → ./data/dict.bin"
+else
+  echo "FAIL: npm 包 exports ./dict.bin 未指向 ./data/dict.bin"
+  exit 1
+fi
+if grep -Fq '"./sha256_prefix.bin": "./data/sha256_prefix.bin"' "$PKG_JSON"; then
+  echo "OK: npm 包 exports ./sha256_prefix.bin → ./data/sha256_prefix.bin"
+else
+  echo "FAIL: npm 包 exports ./sha256_prefix.bin 未指向 ./data/sha256_prefix.bin"
+  exit 1
+fi
+
+# 3. npm pack 字节级比对（npm 可用时）
+if command -v npm &>/dev/null; then
+  TMPDIR_PACK=$(mktemp -d)
+  # npm pack 产物路径前缀为 package/（tarball 内结构：package/data/dict.bin）
+  (cd crates/vane-dict-zh && npm pack --pack-destination "$TMPDIR_PACK" >/dev/null 2>&1)
+  TARBALL=$(ls "$TMPDIR_PACK"/*.tgz 2>/dev/null | head -1)
+  if [ -z "$TARBALL" ]; then
+    echo "FAIL: npm pack 未生成 tarball"
+    rm -rf "$TMPDIR_PACK"
+    exit 1
+  fi
+  # tar -O 提取指定路径到 stdout（GNU tar / BSD tar 均支持）
+  NPM_DICT_SHA=$(tar -xzf "$TARBALL" -O package/data/dict.bin 2>/dev/null | compute_sha256)
+  rm -rf "$TMPDIR_PACK"
+  NODE_SHA_NPM=$(compute_sha256 < "$DICT_BIN")
+  echo "npm pack dict.bin sha256: $NPM_DICT_SHA"
+  echo "Node dict.bin sha256:     $NODE_SHA_NPM"
+  if [ "$NPM_DICT_SHA" != "$NODE_SHA_NPM" ]; then
+    echo "FAIL: npm pack dict.bin ↔ Node dict.bin sha256 mismatch (SPEC §12.3 第四渠道)"
+    exit 1
+  fi
+  echo "OK: npm pack 产物 dict.bin ↔ Node dict.bin sha256 一致（第四渠道字节同源）"
+else
+  echo "INFO: npm 不可用，跳过 npm pack 字节比对（files + exports 元数据校验已间接保证同源）"
+fi
+
+echo "=== 四渠道词典哈希一致性校验通过 ==="
