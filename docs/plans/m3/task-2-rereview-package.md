# Task 2 Re-review Package（fix round 1：1d442d7..HEAD）

## Commits
ef8cc04 fix(web): README License 同步 Apache-2.0（M3 Task 2 I1 修复）

## Diff stat
 bindings/web/README.md | 2 +-
 1 file changed, 1 insertion(+), 1 deletion(-)

## 完整 diff
diff --git a/bindings/web/README.md b/bindings/web/README.md
index c785445..a79950d 100644
--- a/bindings/web/README.md
+++ b/bindings/web/README.md
@@ -94,9 +94,9 @@ bash scripts/build-web.sh
 | `dist/vane_wasm.d.ts` | wasm-bindgen 生成 | TS 类型 |
 | `dist/vane_wasm_simd.wasm` | cargo build + wasm-opt | SIMD128 加速变体 |
 | `dist/vane_wasm_scalar.wasm` | cargo build + wasm-opt | scalar 兜底变体 |
 | `dist/vane_wasm_bg.wasm` | cp scalar 别名 | wasm-bindgen 默认 URL 兼容 |
 | `dist/index.js` / `worker.js` / `probe.js` | Task 3 手写 TS | 主线程 API + Worker 入口 + 探针 |
 
 ## License
 
-MIT（见 [LICENSE](./LICENSE)）。
+Apache-2.0（见 [LICENSE](./LICENSE)）。
