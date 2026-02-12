## Commits f849c7b..34a9b11 (1b fix r1)

34a9b11 test(core): proptest 不变量 1 加非空 guard + 修 Cargo.toml 注释（M4 阶段一 b fix r1）

## Diff stat

 crates/vane-core/Cargo.toml                   |  5 +++--
 crates/vane-core/tests/proptest_invariants.rs | 14 ++++++++++++++
 2 files changed, 17 insertions(+), 2 deletions(-)

## Full diff (U10)

diff --git a/crates/vane-core/Cargo.toml b/crates/vane-core/Cargo.toml
index a23cd2a..e3b3d9a 100644
--- a/crates/vane-core/Cargo.toml
+++ b/crates/vane-core/Cargo.toml
@@ -62,22 +62,23 @@ executor-native = ["dep:rayon"]
 # dev/optional，默认不启用。cfg(test) 或本 feature 启用时编译 fault.rs，
 # 供崩溃恢复测试精确模拟 IO 错误 / 部分写 / ENOSPC / 延迟。
 # 绝不进生产/wasm 二进制——wasm32 check 不启此 feature、不设 test cfg。
 fault-injection = []
 
 [dev-dependencies]
 criterion = "0.5"
 # proptest：property-based 不变量测试（M4 §3.3）。
 # dev-dep，不进 wasm/native 生产构建（wasm32 check 不含 dev-deps）。
 # 传递依赖无黑名单项（regex/tokio/prost/tonic/openssl/lindera/ndarray/
-# wee_alloc/dashmap/parking_lot）——默认 features 不含 regex/regex-syntax，
-# cargo deny check 守护。Strategy 用 a-z 字符生成（非 string_regex），零 regex 路径。
+# wee_alloc/dashmap/parking_lot）。proptest default 拉 regex-syntax（独立
+# regex 解析器，非 deny 黑名单的 regex crate），不拉 regex crate。
+# cargo deny check 守护（bans ok）。Strategy 用 a-z 字符生成（非 string_regex）。
 proptest = "1"
 
 [[bench]]
 name = "hybrid_search"
 harness = false
 
 [[bench]]
 name = "batch_add"
 harness = false
 
diff --git a/crates/vane-core/tests/proptest_invariants.rs b/crates/vane-core/tests/proptest_invariants.rs
index 6400434..ce3ae7a 100644
--- a/crates/vane-core/tests/proptest_invariants.rs
+++ b/crates/vane-core/tests/proptest_invariants.rs
@@ -226,20 +226,34 @@ proptest! {
 
         // 不变量 1a：结果数 ≤ min(topK, total_docs)。
         let upper = (q.top_k as usize).min(docs.len());
         prop_assert!(
             hits1.len() <= upper,
             "hits1.len() {} exceeds min(topK={}, total={})",
             hits1.len(), q.top_k, docs.len()
         );
         prop_assert_eq!(hits1.len(), hits2.len(), "same query must return same count");
 
+        // 不变量 1a-guard：Vector/Hybrid 模式非空——docs 非空 + query 有效向量
+        // → search 必返 ≥1 hit（cosine 对非零向量有定义，RRF 不过滤结果）。
+        // Text 模式 query 文本可能不命中任何文档，0 hits 合法，不强制非空。
+        // 此 guard 捕获 search 返 0 hits 的 bug（否则 windows(2) 空、cap1==cap2
+        // 两空全过 = 假绿）。
+        if matches!(q.mode, SearchMode::Vector | SearchMode::Hybrid) {
+            prop_assert!(
+                !hits1.is_empty(),
+                "Vector/Hybrid search returned 0 hits with {} docs, topK={}",
+                docs.len(),
+                q.top_k
+            );
+        }
+
         // 不变量 1b：score 单调非递增，且全部有限。
         for w in hits1.windows(2) {
             prop_assert!(
                 w[0].score >= w[1].score,
                 "scores not monotonically non-increasing: {} then {}",
                 w[0].score, w[1].score
             );
         }
         for h in &hits1 {
             prop_assert!(
