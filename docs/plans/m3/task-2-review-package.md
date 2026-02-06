# Task 2 Review Package（bcdd6c3..HEAD）

## Commits
1d442d7 fix(web): LICENSE 对齐仓库 Apache-2.0（M3 Task 2 C1 修复）
9659a9e docs(plans): M3 Task 2 report + Task 1 设计文档入库
520a00e feat(web): @vane-rs/web 包骨架 + build-web.sh wasm 双变体构建脚本——M3 Task 2

## Diff stat
 bindings/web/.gitignore           |   3 +
 bindings/web/LICENSE              | 202 +++++++++++++++++++
 bindings/web/README.md            | 102 ++++++++++
 bindings/web/package.json         |  22 +++
 bindings/web/scripts/build-web.sh | 167 ++++++++++++++++
 docs/plans/m3/task-1-design.md    | 394 ++++++++++++++++++++++++++++++++++++++
 docs/plans/m3/task-2-report.md    | 115 +++++++++++
 7 files changed, 1005 insertions(+)

## 完整 diff（bindings/web/ + scripts/，聚焦代码，排除 docs/plans 编排者产出）
diff --git a/bindings/web/.gitignore b/bindings/web/.gitignore
new file mode 100644
index 0000000..136f5f2
--- /dev/null
+++ b/bindings/web/.gitignore
@@ -0,0 +1,3 @@
+# @vane-rs/web 构建产物（bash scripts/build-web.sh 产出）
+dist/
+.build-tmp/
diff --git a/bindings/web/LICENSE b/bindings/web/LICENSE
new file mode 100644
index 0000000..d645695
--- /dev/null
+++ b/bindings/web/LICENSE
@@ -0,0 +1,202 @@
+
+                                 Apache License
+                           Version 2.0, January 2004
+                        http://www.apache.org/licenses/
+
+   TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION
+
+   1. Definitions.
+
+      "License" shall mean the terms and conditions for use, reproduction,
+      and distribution as defined by Sections 1 through 9 of this document.
+
+      "Licensor" shall mean the copyright owner or entity authorized by
+      the copyright owner that is granting the License.
+
+      "Legal Entity" shall mean the union of the acting entity and all
+      other entities that control, are controlled by, or are under common
+      control with that entity. For the purposes of this definition,
+      "control" means (i) the power, direct or indirect, to cause the
+      direction or management of such entity, whether by contract or
+      otherwise, or (ii) ownership of fifty percent (50%) or more of the
+      outstanding shares, or (iii) beneficial ownership of such entity.
+
+      "You" (or "Your") shall mean an individual or Legal Entity
+      exercising permissions granted by this License.
+
+      "Source" form shall mean the preferred form for making modifications,
+      including but not limited to software source code, documentation
+      source, and configuration files.
+
+      "Object" form shall mean any form resulting from mechanical
+      transformation or translation of a Source form, including but
+      not limited to compiled object code, generated documentation,
+      and conversions to other media types.
+
+      "Work" shall mean the work of authorship, whether in Source or
+      Object form, made available under the License, as indicated by a
+      copyright notice that is included in or attached to the work
+      (an example is provided in the Appendix below).
+
+      "Derivative Works" shall mean any work, whether in Source or Object
+      form, that is based on (or derived from) the Work and for which the
+      editorial revisions, annotations, elaborations, or other modifications
+      represent, as a whole, an original work of authorship. For the purposes
+      of this License, Derivative Works shall not include works that remain
+      separable from, or merely link (or bind by name) to the interfaces of,
+      the Work and Derivative Works thereof.
+
+      "Contribution" shall mean any work of authorship, including
+      the original version of the Work and any modifications or additions
+      to that Work or Derivative Works thereof, that is intentionally
+      submitted to Licensor for inclusion in the Work by the copyright owner
+      or by an individual or Legal Entity authorized to submit on behalf of
+      the copyright owner. For the purposes of this definition, "submitted"
+      means any form of electronic, verbal, or written communication sent
+      to the Licensor or its representatives, including but not limited to
+      communication on electronic mailing lists, source code control systems,
+      and issue tracking systems that are managed by, or on behalf of, the
+      Licensor for the purpose of discussing and improving the Work, but
+      excluding communication that is conspicuously marked or otherwise
+      designated in writing by the copyright owner as "Not a Contribution."
+
+      "Contributor" shall mean Licensor and any individual or Legal Entity
+      on behalf of whom a Contribution has been received by Licensor and
+      subsequently incorporated within the Work.
+
+   2. Grant of Copyright License. Subject to the terms and conditions of
+      this License, each Contributor hereby grants to You a perpetual,
+      worldwide, non-exclusive, no-charge, royalty-free, irrevocable
+      copyright license to reproduce, prepare Derivative Works of,
+      publicly display, publicly perform, sublicense, and distribute the
+      Work and such Derivative Works in Source or Object form.
+
+   3. Grant of Patent License. Subject to the terms and conditions of
+      this License, each Contributor hereby grants to You a perpetual,
+      worldwide, non-exclusive, no-charge, royalty-free, irrevocable
+      (except as stated in this section) patent license to make, have made,
+      use, offer to sell, sell, import, and otherwise transfer the Work,
+      where such license applies only to those patent claims licensable
+      by such Contributor that are necessarily infringed by their
+      Contribution(s) alone or by combination of their Contribution(s)
+      with the Work to which such Contribution(s) was submitted. If You
+      institute patent litigation against any entity (including a
+      cross-claim or counterclaim in a lawsuit) alleging that the Work
+      or a Contribution incorporated within the Work constitutes direct
+      or contributory patent infringement, then any patent licenses
+      granted to You under this License for that Work shall terminate
+      as of the date such litigation is filed.
+
+   4. Redistribution. You may reproduce and distribute copies of the
+      Work or Derivative Works thereof in any medium, with or without
+      modifications, and in Source or Object form, provided that You
+      meet the following conditions:
+
+      (a) You must give any other recipients of the Work or
+          Derivative Works a copy of this License; and
+
+      (b) You must cause any modified files to carry prominent notices
+          stating that You changed the files; and
+
+      (c) You must retain, in the Source form of any Derivative Works
+          that You distribute, all copyright, patent, trademark, and
+          attribution notices from the Source form of the Work,
+          excluding those notices that do not pertain to any part of
+          the Derivative Works; and
+
+      (d) If the Work includes a "NOTICE" text file as part of its
+          distribution, then any Derivative Works that You distribute must
+          include a readable copy of the attribution notices contained
+          within such NOTICE file, excluding those notices that do not
+          pertain to any part of the Derivative Works, in at least one
+          of the following places: within a NOTICE text file distributed
+          as part of the Derivative Works; within the Source form or
+          documentation, if provided along with the Derivative Works; or,
+          within a display generated by the Derivative Works, if and
+          wherever such third-party notices normally appear. The contents
+          of the NOTICE file are for informational purposes only and
+          do not modify the License. You may add Your own attribution
+          notices within Derivative Works that You distribute, alongside
+          or as an addendum to the NOTICE text from the Work, provided
+          that such additional attribution notices cannot be construed
+          as modifying the License.
+
+      You may add Your own copyright statement to Your modifications and
+      may provide additional or different license terms and conditions
+      for use, reproduction, or distribution of Your modifications, or
+      for any such Derivative Works as a whole, provided Your use,
+      reproduction, and distribution of the Work otherwise complies with
+      the conditions stated in this License.
+
+   5. Submission of Contributions. Unless You explicitly state otherwise,
+      any Contribution intentionally submitted for inclusion in the Work
+      by You to the Licensor shall be under the terms and conditions of
+      this License, without any additional terms or conditions.
+      Notwithstanding the above, nothing herein shall supersede or modify
+      the terms of any separate license agreement you may have executed
+      with Licensor regarding such Contributions.
+
+   6. Trademarks. This License does not grant permission to use the trade
+      names, trademarks, service marks, or product names of the Licensor,
+      except as required for reasonable and customary use in describing the
+      origin of the Work and reproducing the content of the NOTICE file.
+
+   7. Disclaimer of Warranty. Unless required by applicable law or
+      agreed to in writing, Licensor provides the Work (and each
+      Contributor provides its Contributions) on an "AS IS" BASIS,
+      WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or
+      implied, including, without limitation, any warranties or conditions
+      of TITLE, NON-INFRINGEMENT, MERCHANTABILITY, or FITNESS FOR A
+      PARTICULAR PURPOSE. You are solely responsible for determining the
+      appropriateness of using or redistributing the Work and assume any
+      risks associated with Your exercise of permissions under this License.
+
+   8. Limitation of Liability. In no event and under no legal theory,
+      whether in tort (including negligence), contract, or otherwise,
+      unless required by applicable law (such as deliberate and grossly
+      negligent acts) or agreed to in writing, shall any Contributor be
+      liable to You for damages, including any direct, indirect, special,
+      incidental, or consequential damages of any character arising as a
+      result of this License or out of the use or inability to use the
+      Work (including but not limited to damages for loss of goodwill,
+      work stoppage, computer failure or malfunction, or any and all
+      other commercial damages or losses), even if such Contributor
+      has been advised of the possibility of such damages.
+
+   9. Accepting Warranty or Additional Liability. While redistributing
+      the Work or Derivative Works thereof, You may choose to offer,
+      and charge a fee for, acceptance of support, warranty, indemnity,
+      or other liability obligations and/or rights consistent with this
+      License. However, in accepting such obligations, You may act only
+      on Your own behalf and on Your sole responsibility, not on behalf
+      of any other Contributor, and only if You agree to indemnify,
+      defend, and hold each Contributor harmless for any liability
+      incurred by, or claims asserted against, such Contributor by reason
+      of your accepting any such warranty or additional liability.
+
+   END OF TERMS AND CONDITIONS
+
+   APPENDIX: How to apply the Apache License to your work.
+
+      To apply the Apache License to your work, attach the following
+      boilerplate notice, with the fields enclosed by brackets "[]"
+      replaced with your own identifying information. (Don't include
+      the brackets!)  The text should be enclosed in the appropriate
+      comment syntax for the file format. We also recommend that a
+      file or class name and description of purpose be included on the
+      same "printed page" as the copyright notice for easier
+      identification within third-party archives.
+
+   Copyright [yyyy] [name of copyright owner]
+
+   Licensed under the Apache License, Version 2.0 (the "License");
+   you may not use this file except in compliance with the License.
+   You may obtain a copy of the License at
+
+       http://www.apache.org/licenses/LICENSE-2.0
+
+   Unless required by applicable law or agreed to in writing, software
+   distributed under the License is distributed on an "AS IS" BASIS,
+   WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
+   See the License for the specific language governing permissions and
+   limitations under the License.
diff --git a/bindings/web/README.md b/bindings/web/README.md
new file mode 100644
index 0000000..c785445
--- /dev/null
+++ b/bindings/web/README.md
@@ -0,0 +1,102 @@
+# @vane-rs/web
+
+Vane 混合检索库的 Web 端 npm 包：向量检索 + BM25 + RRF 融合，跑在浏览器 Worker 内，
+通过 wasm-bindgen `--target web` 产出 ESM 双变体（SIMD128 / scalar），运行时探针自动选择。
+
+- **双变体**：SIMD128 加速 + scalar 兜底，一份 JS 胶水共享。
+- **Worker 模式**：主线程零阻塞，wasm 在 Dedicated Worker 内运行。
+- **VFS**：OPFS / IndexedDB / memory 三后端，持久化到浏览器存储。
+- **词典**：`@vane-rs/dict-zh` 作 optionalDep，`dictData` 内联 transferable 零拷贝。
+
+> 状态：M3 阶段一 Task 2 产出 wasm 产物构建脚本 + 包骨架。JS/TS 源（`src/*.ts` → `dist/index.js` / `worker.js` / `probe.js`）由 Task 3 补全，下方 API 节为占位。
+
+## 安装
+
+```bash
+npm install @vane-rs/web
+# optionalDep @vane-rs/dict-zh 自动安装；CDN fallback 或自带词典时：
+npm install @vane-rs/web --no-optional
+```
+
+## vite 集成（零配置）
+
+```ts
+import { createVane } from '@vane-rs/web';
+import dictBinUrl from '@vane-rs/dict-zh/dict.bin';
+import dictSha256Url from '@vane-rs/dict-zh/sha256_prefix.bin';
+
+const dictData = new Uint8Array(await (await fetch(dictBinUrl)).arrayBuffer());
+const sha256Hex = Array.from(new Uint8Array(await (await fetch(dictSha256Url)).arrayBuffer()))
+  .map(b => b.toString(16).padStart(2, '0')).join('');
+
+const vane = await createVane({ vfs: 'opfs', dbPath: 'vane.db', dictData, dictSha256: sha256Hex });
+await vane.open();
+const col = await vane.collection('docs', { fields: [{ name: 'text', type: 'text' }] }, { tokenizer: 'jieba' });
+```
+
+vite 6+ 原生识别 `new Worker(new URL('@vane-rs/web/worker', import.meta.url), {type:'module'})` 与 `new URL('./x.wasm', import.meta.url)`，无需 wasm/worker plugin。
+
+## webpack 5 集成
+
+需开启 ESM 输出：
+
+```js
+// webpack.config.js
+export default {
+  experiments: { outputModule: true },
+};
+```
+
+webpack 5 原生支持 `new URL(..., import.meta.url)` asset 与 ESM Worker。`init(wasmUrl)` 显式 fetch 加载 wasm，不依赖 `experiments.asyncWebAssembly`。
+
+## API
+
+> Task 3 补全。下方为设计草案（见 `docs/plans/m3/task-1-design.md` §4/§6），最终以 Task 3 产出 `dist/index.d.ts` 为准。
+
+### `createVane(opts?): Promise<Vane>`
+
+```ts
+interface VaneWorkerOpts {
+  vfs?: 'opfs' | 'idb' | 'memory';
+  dbPath?: string;
+  dictData?: Uint8Array | ArrayBuffer;  // 优先于 dictUrl；transferable 零拷贝
+  dictUrl?: string;                     // CDN fallback
+  dictSha256?: string;                  // 16 字符 hex
+}
+```
+
+### `Vane` 接口
+
+`open(path?, opts?)` / `collection(name, schema, opts?)` / `add(col, docs)` / `flush(col)` / `search(col, query)` / `delete(col, ids)` / `compact(col)` / `reindex(col)` / `export(dest)` / `readFile(path)` / `close()`
+
+### SIMD 探针
+
+```ts
+import { simd128Supported, SIMD128_TEST_MODULE } from '@vane-rs/web/probe';
+simd128Supported();  // boolean，WebAssembly.validate 测试模块
+```
+
+## 构建
+
+`dist/` 是构建产物，由 `scripts/build-web.sh` 产出：
+
+```bash
+bash scripts/build-web.sh
+```
+
+流程：cargo build 双变体（simd128 / scalar）→ wasm-bindgen `--target web` → wasm-opt `-Oz` → 拷贝到 `dist/` → `cp vane_wasm_scalar.wasm vane_wasm_bg.wasm` 别名（默认 URL 兼容）→ gzip 体积门禁 ≤800KB。
+
+产物：
+
+| 文件 | 来源 | 说明 |
+|------|------|------|
+| `dist/vane_wasm.js` | wasm-bindgen 生成 | ESM 胶水，含 `__wbg_init` |
+| `dist/vane_wasm.d.ts` | wasm-bindgen 生成 | TS 类型 |
+| `dist/vane_wasm_simd.wasm` | cargo build + wasm-opt | SIMD128 加速变体 |
+| `dist/vane_wasm_scalar.wasm` | cargo build + wasm-opt | scalar 兜底变体 |
+| `dist/vane_wasm_bg.wasm` | cp scalar 别名 | wasm-bindgen 默认 URL 兼容 |
+| `dist/index.js` / `worker.js` / `probe.js` | Task 3 手写 TS | 主线程 API + Worker 入口 + 探针 |
+
+## License
+
+MIT（见 [LICENSE](./LICENSE)）。
diff --git a/bindings/web/package.json b/bindings/web/package.json
new file mode 100644
index 0000000..e018c59
--- /dev/null
+++ b/bindings/web/package.json
@@ -0,0 +1,22 @@
+{
+  "name": "@vane-rs/web",
+  "version": "0.2.0",
+  "description": "Vane 混合检索库 Web 端 npm 包（wasm-bindgen --target web ESM 双变体 + worker + dictData 内联）",
+  "license": "Apache-2.0",
+  "type": "module",
+  "main": "./dist/index.js",
+  "module": "./dist/index.js",
+  "types": "./dist/index.d.ts",
+  "sideEffects": ["./dist/worker.js", "./dist/vane_wasm.js", "**/*.wasm"],
+  "files": ["dist/", "README.md", "LICENSE"],
+  "exports": {
+    ".": { "types": "./dist/index.d.ts", "import": "./dist/index.js", "default": "./dist/index.js" },
+    "./worker": { "types": "./dist/worker.d.ts", "import": "./dist/worker.js", "default": "./dist/worker.js" },
+    "./probe": { "types": "./dist/probe.d.ts", "import": "./dist/probe.js", "default": "./dist/probe.js" },
+    "./vane_wasm.js": "./dist/vane_wasm.js",
+    "./package.json": "./package.json"
+  },
+  "optionalDependencies": { "@vane-rs/dict-zh": "2026.8.0" },
+  "publishConfig": { "access": "public" },
+  "engines": { "node": ">=16" }
+}
diff --git a/bindings/web/scripts/build-web.sh b/bindings/web/scripts/build-web.sh
new file mode 100755
index 0000000..31a4fb3
--- /dev/null
+++ b/bindings/web/scripts/build-web.sh
@@ -0,0 +1,167 @@
+#!/usr/bin/env bash
+# @vane-rs/web 构建脚本（M3 阶段一 Task 2）：wasm-bindgen --target web ESM 双变体产物。
+#
+# 流程（对应 docs/plans/m3/task-1-design.md §7.4）：
+#   1. 每变体：cargo build（simd128 / scalar，worker feature）
+#   2. 每变体：wasm-bindgen --target web 后处理（产出 _bg.wasm + glue .js + .d.ts）
+#   3. 每变体：wasm-opt -Oz 优化 _bg.wasm → vane_wasm_{simd,scalar}.wasm
+#   4. 拷贝 JS 胶水 + .d.ts 到 dist/（双变体共享一份，导出一致）
+#   5. cp vane_wasm_scalar.wasm vane_wasm_bg.wasm 别名（§7.3 默认 URL 兼容）
+#   6. W8 校验：vane_wasm.js 含 __wbg_init + new URL(..., import.meta.url)
+#   7. 体积门禁（gzip ≤ 800KB，SPEC §13.2-3）
+#
+# 不含 tsc 编译 src/*.ts（Task 3 扩展；src/ 尚不存在）。
+#
+# 技术说明（与 task brief 第 2 步的差异）：
+#   task brief 称"scalar 不需要再跑 wasm-bindgen"。但 raw .wasm 的 __wbindgen_*
+#   导入需经 wasm-bindgen 重写为 __wbg_* 才匹配 vane_wasm.js glue 的 import object
+#   （键名 __wbg_*），否则 WebAssembly.instantiate 报 TypeError。故双变体都必须
+#   跑 wasm-bindgen 后处理。glue 只拷一份（simd 与 scalar 的 glue 相同，导出一致）。
+#   与 demo/build.sh 同模式（已验证可用）。
+#
+# 用法：
+#   bash bindings/web/scripts/build-web.sh
+#   FEATURES=worker bash bindings/web/scripts/build-web.sh
+set -euo pipefail
+
+cd "$(dirname "$0")/../../.."  # bindings/web/scripts/ → bindings/web/ → bindings/ → 仓库根
+
+TARGET="wasm32-unknown-unknown"
+PKG_CRATE="vane-wasm"
+PKG_FILE="vane_wasm"   # cargo build 产物文件名（- 替换为 _）
+FEATURES="${FEATURES:-worker}"
+DIST="bindings/web/dist"
+TMP="bindings/web/.build-tmp"
+
+MAX=$((800 * 1024))
+
+# 保存 simd 变体的 glue 路径（双变体共享一份 glue，只拷 simd 的）
+JS_GLUE=""
+DTS_GLUE=""
+
+# ---- 辅助：wasm-opt 优化或拷贝 ----
+optimize() {
+  local src="$1" dst="$2"
+  if command -v wasm-opt &>/dev/null; then
+    wasm-opt -Oz "$src" -o "$dst"
+    echo "(wasm-opt -Oz applied)"
+  else
+    cp "$src" "$dst"
+    echo "(wasm-opt not available, copying unoptimized)" >&2
+  fi
+}
+
+# ---- 单变体构建：cargo build → wasm-bindgen → wasm-opt ----
+# 参数：$1=label (simd|scalar)  $2=extra_rustflags
+build_variant() {
+  local label="$1"
+  local extra_flags="$2"
+
+  echo "=== [$label] cargo build (RUSTFLAGS='$extra_flags', features=$FEATURES) ==="
+  RUSTFLAGS="$extra_flags" \
+    cargo build --release --target "$TARGET" -p "$PKG_CRATE" --features "$FEATURES"
+
+  local src="target/$TARGET/release/${PKG_FILE}.wasm"
+  [ -f "$src" ] || { echo "FAIL: $src not found" >&2; exit 1; }
+
+  # ⚠️ 必须在 cargo build 后立即跑 wasm-bindgen：下一变体的 cargo build 会覆盖
+  # target/.../vane_wasm.wasm（同路径），先拿到的 src 指向的是当前变体字节。
+  echo "=== [$label] wasm-bindgen --target web ==="
+  rm -rf "$TMP/$label"
+  wasm-bindgen "$src" --out-dir "$TMP/$label" --target web
+
+  local bg="$TMP/$label/${PKG_FILE}_bg.wasm"
+  local js="$TMP/$label/${PKG_FILE}.js"
+  local dts="$TMP/$label/${PKG_FILE}.d.ts"
+  for f in "$bg" "$js" "$dts"; do
+    [ -f "$f" ] || {
+      echo "FAIL: $f missing (wasm-bindgen 产出不完整)" >&2
+      ls -la "$TMP/$label" >&2
+      exit 1
+    }
+  done
+
+  # wasm-opt 优化 → dist/vane_wasm_{label}.wasm
+  local dst="$DIST/vane_wasm_${label}.wasm"
+  optimize "$bg" "$dst"
+  echo "→ $dst"
+
+  # simd 变体记录 glue 路径（双变体 glue 相同，只拷一份）
+  if [ "$label" = "simd" ]; then
+    JS_GLUE="$js"
+    DTS_GLUE="$dts"
+  fi
+}
+
+# ---- 清理 + 建目录 ----
+rm -rf "$TMP" "$DIST"
+mkdir -p "$TMP" "$DIST"
+
+# ============================================================
+# 1-3. 双变体构建（每变体：cargo build → wasm-bindgen → wasm-opt）
+# ============================================================
+build_variant simd  "-Ctarget-feature=+simd128"
+echo ""
+build_variant scalar ""
+
+# ============================================================
+# 4. 拷贝 JS 胶水 + .d.ts（双变体共享一份）
+# ============================================================
+cp "$JS_GLUE" "$DIST/vane_wasm.js"
+cp "$DTS_GLUE" "$DIST/vane_wasm.d.ts"
+echo "→ $DIST/vane_wasm.js"
+echo "→ $DIST/vane_wasm.d.ts"
+
+# ============================================================
+# 5. cp vane_wasm_scalar.wasm vane_wasm_bg.wasm 别名（§7.3 默认 URL 兼容）
+#    wasm-bindgen 生成的 vane_wasm.js 末尾默认 new URL('vane_wasm_bg.wasm', import.meta.url)。
+#    双变体重命名为 _simd/_scalar 后无 _bg.wasm，bundler 静态分析会报错。
+#    cp scalar 别名保守默认 scalar；worker.js 显式传 URL 覆盖默认。
+# ============================================================
+cp "$DIST/vane_wasm_scalar.wasm" "$DIST/vane_wasm_bg.wasm"
+echo "→ $DIST/vane_wasm_bg.wasm (scalar 别名，默认 URL 兼容)"
+
+# ============================================================
+# 6. W8 校验：vane_wasm.js 含 __wbg_init + new URL(..., import.meta.url)
+# ============================================================
+echo ""
+echo "=== W8 wasm-bindgen 生成校验 ==="
+if ! grep -q '__wbg_init' "$DIST/vane_wasm.js"; then
+  echo "FAIL: vane_wasm.js 缺 __wbg_init（wasm-bindgen 生成结构异常，W8）" >&2
+  exit 1
+fi
+if ! grep -qE 'new URL\([^)]*import\.meta\.url\)' "$DIST/vane_wasm.js"; then
+  echo "FAIL: vane_wasm.js 缺 new URL(..., import.meta.url)（默认 URL 解析异常，W8）" >&2
+  exit 1
+fi
+echo "OK: __wbg_init + new URL(..., import.meta.url) 均存在"
+
+# ============================================================
+# 7. 体积门禁（gzip ≤ 800KB，SPEC §13.2-3）
+# ============================================================
+echo ""
+echo "=== Size gate (gzip ≤ 800KB) ==="
+FAIL=0
+for v in simd scalar; do
+  f="$DIST/vane_wasm_${v}.wasm"
+  size=$(gzip -c "$f" | wc -c | tr -d ' ')
+  raw=$(stat -f%z "$f" 2>/dev/null || stat -c%s "$f")
+  echo "$v: raw=$raw bytes, gzip=$size bytes (max $MAX)"
+  if [ "$size" -gt "$MAX" ]; then
+    echo "FAIL: $v gzip > 800KB" >&2
+    FAIL=1
+  fi
+done
+
+# bg.wasm 别名体积（= scalar，仅日志，不入门禁）
+BG_SIZE=$(gzip -c "$DIST/vane_wasm_bg.wasm" | wc -c | tr -d ' ')
+echo "bg (alias of scalar): gzip=$BG_SIZE bytes (不入门禁，别名)"
+
+echo ""
+echo "=== dist 产出 ==="
+ls -la "$DIST"
+
+# 清理临时目录
+rm -rf "$TMP"
+
+exit $FAIL
