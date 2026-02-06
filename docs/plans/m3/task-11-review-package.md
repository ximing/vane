# Task 11 Review Package（16f0400..HEAD）

## Commits
4c4e4b6 feat(m3): release.yml 加 build-web job + 扩展 release job 发 @vane-rs/web + @vane-rs/dict-zh（三端→四端）

## Diff stat
 .github/workflows/release.yml | 60 ++++++++++++++++++++++++++++++++++++++++++-
 1 file changed, 59 insertions(+), 1 deletion(-)

## 完整 diff（release.yml）
diff --git a/.github/workflows/release.yml b/.github/workflows/release.yml
index aeb30ff..529a6ae 100644
--- a/.github/workflows/release.yml
+++ b/.github/workflows/release.yml
@@ -124,32 +124,65 @@ jobs:
         run: sudo apt-get install -y binaryen
       - name: Build WASM dual variants (simd + scalar)
         run: FEATURES=worker bash scripts/build-wasm-variants.sh
       - uses: actions/upload-artifact@v4
         with:
           name: vane-wasm
           path: |
             target/wasm-variants/vane_wasm_simd.wasm
             target/wasm-variants/vane_wasm_scalar.wasm
 
+  build-web:
+    # SPEC §12.2/§13.2-3 @vane-rs/web npm 包构建（wasm-bindgen --target web ESM 双变体 + tsc）。
+    # 复用 bindings/web/scripts/build-web.sh：cargo build 双变体（simd128/scalar）+
+    # wasm-bindgen --target web 后处理 + wasm-opt -Oz + tsc 编译 src/*.ts → dist/ + 体积门禁 gzip ≤800KB。
+    # 产出 bindings/web/dist/ 上传 artifact 供 release job npm publish。
+    # 与 build / build-go / build-wasm 并行，无相互依赖。
+    runs-on: ubuntu-latest
+    timeout-minutes: 30
+    steps:
+      - uses: actions/checkout@v4
+      - uses: dtolnay/rust-toolchain@stable
+        with:
+          targets: wasm32-unknown-unknown
+      - uses: Swatinem/rust-cache@v2
+      - name: Install binaryen (wasm-opt)
+        run: sudo apt-get install -y binaryen
+      - name: Install wasm-bindgen-cli
+        # 版本对齐 Cargo.lock wasm-bindgen 0.2.127（ci.yml wasm-recall job 同模式）。
+        # wasm-bindgen-cli 版本必须 == 编译进 .wasm 的 wasm-bindgen library 版本，否则后处理报版本不匹配。
+        run: cargo install wasm-bindgen-cli --locked --version 0.2.127
+      - uses: actions/setup-node@v4
+        with:
+          node-version: '20'
+      - name: Install web devDeps (typescript)
+        working-directory: bindings/web
+        run: npm ci
+      - name: Build @vane-rs/web (dist/)
+        run: bash bindings/web/scripts/build-web.sh
+      - uses: actions/upload-artifact@v4
+        with:
+          name: vane-web
+          path: bindings/web/dist/
+
   release:
     # 三端 prebuilt 聚合发布。Node 流水线遵循 napi-rs 官方发布流程
     # （napi.rs/docs/deep-dive/release），与 @napi-rs/cli 3.x 命令一致：
     #   1. 下载 4 平台 .node → 暂存到 crates/vane-node/（napi artifacts 扫描位置）
     #   2. napi create-npm-dirs：为每个 target 创建 npm/<platform>/ 平台包目录 + package.json
     #   3. napi artifacts --output-dir . --npm-dir npm：从 cwd 收集 .node 拷贝到 npm/<platform>/ 与包根
     #   4. npm publish（主包）：prepublishOnly 脚本 `napi pre-publish -t npm` 自动发布
     #      4 个 optionalDeps 平台包 + 上传 .node 到 GitHub Release
     # Go .a + WASM .wasm 由 softprops/action-gh-release 单独挂到同一 Release。
     # tag 触发：npm publish + GitHub Release。workflow_dispatch：仅构建 + create-npm-dirs +
     # artifacts 收集验证（publish/gh-release 通过 `if: startsWith(github.ref, 'refs/tags/')` 跳过）。
-    needs: [build, build-go, build-wasm]
+    needs: [build, build-go, build-wasm, build-web]
     runs-on: ubuntu-latest
     timeout-minutes: 15
     permissions:
       contents: write
     steps:
       - uses: actions/checkout@v4
       - uses: actions/setup-node@v4
         with:
           node-version: '20'
           registry-url: 'https://registry.npmjs.org'
@@ -161,20 +194,28 @@ jobs:
           path: artifacts
       - name: Stage node .node + index.js into crates/vane-node/
         # napi artifacts --output-dir . 从 cwd（crates/vane-node/）扫描 *.node；
         # 将 4 个 per-platform .node 暂存到此目录。
         # index.js 是 napi-rs 生成的平台切换 loader（main.js require './index.js'），
         # 被 .gitignore 忽略不入库，需从 build artifact 暂存（4 平台生成相同文件，取 linux 即可）。
         run: |
           cp artifacts/vane-node-*/*.node crates/vane-node/
           cp artifacts/vane-node-linux-x64-gnu/index.js crates/vane-node/
           ls -lh crates/vane-node/*.node crates/vane-node/index.js
+      - name: Stage web dist/ into bindings/web/
+        # build-web artifact 上传 dist/ 目录内容到 artifact 根（upload-artifact 对目录取内容），
+        # 下载后位于 artifacts/vane-web/。bindings/web/dist/ 被 .gitignore 忽略（checkout 后不存在），
+        # 需 mkdir + cp 重建。package.json + README + LICENSE 已由 checkout 提供，不需 artifact 携带。
+        run: |
+          mkdir -p bindings/web/dist
+          cp -r artifacts/vane-web/. bindings/web/dist/
+          ls -la bindings/web/dist/
       - name: Create npm platform package dirs
         # 3.x 命令为 `create-npm-dirs`（复数，无 -t flag）；从 package.json napi.targets 读 4 平台，
         # 为每个 target 创建 npm/<platform>/ + package.json（main 指向 vane.<platform>.node，
         # binaryName "vane" 激活；loader index.js 由 napi build 生成再暂存）。
         # 必须在 napi artifacts 之前运行（后者只写文件，不建目录）。
         # working-directory 提供 cwd（crates/vane-node/），--npm-dir 默认 npm。
         working-directory: crates/vane-node
         run: napi create-npm-dirs
       - name: Collect .node into platform packages
         # 3.x napi artifacts 选项：--output-dir,-o,-d（3.x 拒 --dir）+ --npm-dir。
@@ -192,10 +233,27 @@ jobs:
         env:
           NODE_AUTH_TOKEN: ${{ secrets.NPM_TOKEN }}
           GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
       - name: Create GitHub Release + attach Go/WASM assets
         if: startsWith(github.ref, 'refs/tags/')
         uses: softprops/action-gh-release@v2
         with:
           files: |
             artifacts/vane-go-*/libvane_ffi-*.a
             artifacts/vane-wasm/*.wasm
+      - name: Publish @vane-rs/web to npm
+        # @vane-rs/web 非 napi 包，直接 npm publish --access public（package.json publishConfig.access=public 已设）。
+        # dist/ 由 build-web artifact 暂存；package.json + README + LICENSE 由 checkout 提供。
+        # setup-node registry-url 已在 release job 顶部配置（npm publish 读 ~/.npmrc 认证）。
+        if: startsWith(github.ref, 'refs/tags/')
+        working-directory: bindings/web
+        run: npm publish --access public
+        env:
+          NODE_AUTH_TOKEN: ${{ secrets.NPM_TOKEN }}
+      - name: Publish @vane-rs/dict-zh to npm
+        # @vane-rs/dict-zh 纯数据包，非 napi，不需 build job。源目录 data/ + package.json + LICENSE
+        # 已在仓库（package.json files：data/dict.bin + data/sha256_prefix.bin + LICENSE）。
+        if: startsWith(github.ref, 'refs/tags/')
+        working-directory: crates/vane-dict-zh
+        run: npm publish --access public
+        env:
+          NODE_AUTH_TOKEN: ${{ secrets.NPM_TOKEN }}
