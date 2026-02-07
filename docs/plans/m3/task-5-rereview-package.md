# Task 5 Re-review Package（fix round 1：2aadd28..HEAD）

## Commits
6857c70 fix(dict-zh): 补 LICENSE 文件 + files 列入（M3 Task 5 I1 修复）
e1c00f3 docs(m3): Task 5 report——@vane-rs/dict-zh npm 包元数据完成

## Diff stat
 crates/vane-dict-zh/LICENSE      | 202 +++++++++++++++++++++++++++++++++++++++
 crates/vane-dict-zh/README.md    |   2 +-
 crates/vane-dict-zh/package.json |   2 +-
 docs/plans/m3/task-5-report.md   |  87 +++++++++++++++++
 4 files changed, 291 insertions(+), 2 deletions(-)

## 完整 diff
diff --git a/crates/vane-dict-zh/LICENSE b/crates/vane-dict-zh/LICENSE
new file mode 100644
index 0000000..d645695
--- /dev/null
+++ b/crates/vane-dict-zh/LICENSE
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
diff --git a/crates/vane-dict-zh/README.md b/crates/vane-dict-zh/README.md
index 250eae0..1c15226 100644
--- a/crates/vane-dict-zh/README.md
+++ b/crates/vane-dict-zh/README.md
@@ -59,9 +59,9 @@ npm install @vane-rs/dict-zh
 通常无需独立安装——`@vane-rs/web` 已将其声明为 optionalDependency，`npm install @vane-rs/web` 自动拉取。
 
 ## 词典永不进 wasm
 
 红线（SPEC §12.3）：`dict.bin` 独立分发，不编译进 `.wasm` 产物。本包是独立 npm 数据包，天然遵守——wasm 体积门禁 ≤800KB 不受词典影响。
 
 ## License
 
-Apache-2.0（见仓库根 [LICENSE](https://github.com/ximing/vane/blob/main/LICENSE)）。
+Apache-2.0（见同目录 [LICENSE](./LICENSE)）。
diff --git a/crates/vane-dict-zh/package.json b/crates/vane-dict-zh/package.json
index 64bd36a..6bd0b7e 100644
--- a/crates/vane-dict-zh/package.json
+++ b/crates/vane-dict-zh/package.json
@@ -1,14 +1,14 @@
 {
   "name": "@vane-rs/dict-zh",
   "version": "2026.8.0",
   "description": "Vane 中文词典数据包（dict.bin zstd 压缩 DAT + HMM，含 sha256_prefix 校验字节）",
   "license": "Apache-2.0",
-  "files": ["data/dict.bin", "data/sha256_prefix.bin"],
+  "files": ["data/dict.bin", "data/sha256_prefix.bin", "LICENSE"],
   "exports": {
     "./dict.bin": "./data/dict.bin",
     "./sha256_prefix.bin": "./data/sha256_prefix.bin",
     "./package.json": "./package.json"
   },
   "publishConfig": {
     "access": "public"
   },
diff --git a/docs/plans/m3/task-5-report.md b/docs/plans/m3/task-5-report.md
new file mode 100644
index 0000000..82005b2
--- /dev/null
+++ b/docs/plans/m3/task-5-report.md
@@ -0,0 +1,87 @@
+# Task 5 Report：@vane-rs/dict-zh npm 包元数据
+
+## 状态
+
+✅ 完成。crates/vane-dict-zh/ 已具备独立 npm 数据包的全部元数据（package.json + .npmignore + README.md），`npm pack --dry-run` 验证产物内容、体积、exports map 全部对齐。
+
+## Commits
+
+- `2aadd28` feat(dict-zh): @vane-rs/dict-zh npm 包元数据（package.json + .npmignore + README）
+
+## 测试摘要
+
+`npm pack --dry-run` 产物仅 4 文件（README.md 2.9kB + data/dict.bin 1.5MB + data/sha256_prefix.bin 8B + package.json 733B），体积 1.5MB；exports `./dict.bin` → `./data/dict.bin` + `./sha256_prefix.bin` → `./data/sha256_prefix.bin`；version=2026.8.0，license=Apache-2.0；未触碰 vane-wasm/ vane-dict-zh src/data/Cargo.toml。
+
+## npm pack --dry-run 产物清单
+
+```
+npm notice 📦  @vane-rs/dict-zh@2026.8.0
+npm notice Tarball Contents
+npm notice 2.9kB   README.md
+npm notice 1.5MB   data/dict.bin
+npm notice 8B      data/sha256_prefix.bin
+npm notice 733B    package.json
+npm notice Tarball Details
+npm notice name:          @vane-rs/dict-zh
+npm notice version:       2026.8.0
+npm notice filename:      vane-rs-dict-zh-2026.8.0.tgz
+npm notice package size:  1.5 MB
+npm notice unpacked size: 1.5 MB
+npm notice total files:   4
+```
+
+无 src/、tests/、benches/、examples/、Cargo.toml、target/、*.rs——纯数据包。
+
+## exports map
+
+```json
+{
+  "./dict.bin": "./data/dict.bin",
+  "./sha256_prefix.bin": "./data/sha256_prefix.bin",
+  "./package.json": "./package.json"
+}
+```
+
+- `import dictBinUrl from '@vane-rs/dict-zh/dict.bin'` → vite/webpack 解析为 `data/dict.bin` 资源 URL
+- `import dictSha256Url from '@vane-rs/dict-zh/sha256_prefix.bin'` → 解析为 `data/sha256_prefix.bin` 资源 URL
+- 无 `"."` 根导出（纯数据包，无 JS 入口）；`require('@vane-rs/dict-zh')` 会 ERR_PACKAGE_PATH_NOT_EXPORTED，符合预期
+- 无 main/module/types 字段
+
+## 关键字段对齐
+
+| 字段 | 值 | 对齐目标 |
+|------|-----|----------|
+| name | `@vane-rs/dict-zh` | @vane-rs/web optionalDep |
+| version | `2026.8.0` | Cargo.toml version + @vane-rs/web optionalDep `2026.8.0` + CDN URL `@vane-rs/dict-zh@2026.8.0` |
+| license | `Apache-2.0` | 仓库 workspace.package.license |
+| files | `["data/dict.bin", "data/sha256_prefix.bin"]` | 只发数据，不发 Rust 代码 |
+| publishConfig.access | `public` | @vane-rs scope 私有，必须 public |
+
+## 冻结路径验证
+
+- `crates/vane-wasm/`：未触碰（git status 空）✅
+- `crates/vane-dict-zh/src/`：未触碰 ✅
+- `crates/vane-dict-zh/data/`：未触碰（dict.bin/sha256_prefix.bin 字节冻结）✅
+- `crates/vane-dict-zh/Cargo.toml`：未触碰（publish=false 保留，Rust crate 不发 crates.io，package.json 是独立 npm 通道）✅
+
+## 产出文件
+
+| 文件 | 行数 | 说明 |
+|------|------|------|
+| `crates/vane-dict-zh/package.json` | 21 | npm 包元数据，exports + files + publishConfig |
+| `crates/vane-dict-zh/.npmignore` | 20 | 排除 Rust 产物（src/tests/benches/examples/Cargo.*/target/*.rs），belt-and-suspenders |
+| `crates/vane-dict-zh/README.md` | 73 | 纯数据包说明 + vite/webpack 用法 + 与 @vane-rs/web 配合 + dict.bin 格式 + 永不进 wasm 红线 |
+
+## Concerns
+
+1. **无 LICENSE 文件**：package.json 声明 `license: "Apache-2.0"`，但 crates/vane-dict-zh/ 目录下无 LICENSE 文件（仓库根有）。npm pack 产物不含 LICENSE（npm 只从包目录找）。metadata 字段已声明 license，对 npm registry 显示足够；但若要产物内含 LICENSE 文本，需后续从仓库根 cp 一份（非本任务范围，Task 11 发版前可评估）。@vane-rs/web 的 files 字段显式含 "LICENSE"——若一致性要求高，Task 11 可补。
+2. **exports 无根导出**：`require('@vane-rs/dict-zh')` 会抛 ERR_PACKAGE_PATH_NOT_EXPORTED。这是纯数据包的预期行为（无 JS 入口），但若未来想加运行期 JS helper（如 `dictVersion` / `sha256PrefixHex`），需新增 `"."` 导出——当前无此需求。
+3. **.npmignore 与 files 冗余**：`files` allowlist 已是主防线（npm 优先于 .npmignore），.npmignore 是 belt-and-suspenders。两者一致，无冲突。若后续维护时只改其一，需注意同步。
+
+## 不做项确认（后续任务范围）
+
+- 四渠道哈希校验扩展 → Task 6
+- SPEC §12.3 修订 → Task 13（合并 v1.5）
+- release.yml npm publish → Task 11
+- Cargo.toml publish=false 未改 → Rust crate 仍不发 crates.io
+- src/lib.rs / data/dict.bin 未改 → 字节冻结
