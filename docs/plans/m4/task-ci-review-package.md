## Commits f793e93..b4aa743 (6a CI)

b4aa743 ci: 新增 fuzz-smoke/fuzz-long/compat/stress/crash-recovery job（M4 阶段六 a）

## Diff stat

 .github/workflows/ci.yml | 124 +++++++++++++++++++++++++++++++++++++++++++++++
 1 file changed, 124 insertions(+)

## Full diff (U10)

diff --git a/.github/workflows/ci.yml b/.github/workflows/ci.yml
index 0e03f3b..75ca3ef 100644
--- a/.github/workflows/ci.yml
+++ b/.github/workflows/ci.yml
@@ -1,18 +1,24 @@
 name: ci
 
 on:
   push:
     branches: [main]
     paths-ignore: ['website/**', 'docs/plans/docs-site-*.md']
   pull_request:
     paths-ignore: ['website/**', 'docs/plans/docs-site-*.md']
+  schedule:
+    # M4 §3.2 fuzz-long：每周日 03:00 UTC 触发长跑。注意 schedule 不受 paths-ignore
+    # 约束——届时全 workflow（含现有 job）都会跑（public repo CI 免费且可作周度
+    # 回归/flaky 检测；fuzz-long 本身用 if 门控只在 schedule/dispatch 跑）。
+    - cron: '0 3 * * 0'
+  workflow_dispatch:
 
 permissions:
   contents: read
 
 concurrency:
   group: ci-${{ github.ref }}
   cancel-in-progress: true
 
 env:
   CARGO_TERM_COLOR: always
@@ -316,10 +322,128 @@ jobs:
         with:
           targets: wasm32-unknown-unknown
       - uses: Swatinem/rust-cache@v2
       - uses: actions/setup-node@v4
         with:
           node-version: '20'
       - name: Install wasm-bindgen-cli
         run: cargo install wasm-bindgen-cli --locked --version 0.2.127
       - name: Run dual-variant recall regression (simd + scalar + Jaccard)
         run: bash scripts/run-wasm-recall.sh
+
+  # ===========================================================================
+  # M4 阶段六 a：5 个新 CI job（fuzz-smoke / fuzz-long / compat / stress /
+  # crash-recovery）。前置 Phase 1-5 已就位（vane-fuzz crate + cross_version_compat
+  # + stress_concurrency + crash_recovery）。本批只改 ci.yml，不碰源码/SPEC。
+  # ===========================================================================
+
+  fuzz-smoke:
+    # M4 §3.2 fuzz-smoke：每 target 60s 短跑（push/PR）。nightly + cargo-fuzz
+    # （-Z sanitizer）。独立 nightly toolchain，不依赖 test job（stable）——fuzz 是
+    # 不同验证维度，并行跑省时。vane-fuzz 不在 default-members，CI 直接到
+    # crates/vane-fuzz 跑 cargo fuzz。cron（周日 03:00）由 fuzz-long 深跑覆盖，
+    # smoke 不重复 → if: event_name != schedule。
+    if: github.event_name != 'schedule'
+    runs-on: ubuntu-latest
+    timeout-minutes: 15
+    steps:
+      - uses: actions/checkout@v4
+      # pin nightly（cargo-fuzz 需 -Z sanitizer，stable 不支持）。@master 配
+      # toolchain: 输入是 dtolnay/rust-toolchain pin 特定 nightly 的标准用法
+      # （@stable/@nightly tag 自选 toolchain，不读 toolchain 输入）。
+      - uses: dtolnay/rust-toolchain@master
+        with:
+          toolchain: nightly-2026-07-01
+      - uses: Swatinem/rust-cache@v2
+      - name: Install cargo-fuzz
+        run: cargo install cargo-fuzz --locked
+      - name: Fuzz smoke (60s per target, 5 targets)
+        run: |
+          for target in brute_search_fuzz hnsw_search_fuzz persist_roundtrip_fuzz merge_fuzz dict_load_fuzz; do
+            cargo fuzz run $target -- -max_total_time=60 -max_len=4096
+          done
+        working-directory: crates/vane-fuzz
+
+  fuzz-long:
+    # M4 §3.2 fuzz-long：每 target 10min 长跑（cron 周日 03:00 UTC + 手动 dispatch）。
+    # nightly + cargo-fuzz，-max_total_time=600 -max_len=65536。|| true 容错——
+    # crash 不阻断 job（长跑发现 crash 是预期的，人工分析），但上传 crash artifact。
+    # if: always() + if-no-files-found: ignore 保证无 crash 时不上传、不失败。
+    # timeout 60min（5 targets × 10min + install/compile overhead）。
+    if: ${{ github.event_name == 'schedule' || github.event_name == 'workflow_dispatch' }}
+    runs-on: ubuntu-latest
+    timeout-minutes: 60
+    steps:
+      - uses: actions/checkout@v4
+      - uses: dtolnay/rust-toolchain@master
+        with:
+          toolchain: nightly-2026-07-01
+      - uses: Swatinem/rust-cache@v2
+      - name: Install cargo-fuzz
+        run: cargo install cargo-fuzz --locked
+      - name: Fuzz long (10min per target, 5 targets)
+        run: |
+          for target in brute_search_fuzz hnsw_search_fuzz persist_roundtrip_fuzz merge_fuzz dict_load_fuzz; do
+            cargo fuzz run $target -- -max_total_time=600 -max_len=65536 || true
+          done
+        working-directory: crates/vane-fuzz
+      - name: Upload crash artifacts
+        if: always()
+        uses: actions/upload-artifact@v4
+        with:
+          name: fuzz-crash
+          path: crates/vane-fuzz/fuzz/artifacts/
+          if-no-files-found: ignore
+
+  compat:
+    # M4 §3.4 跨版本持久化兼容：当前版本读 v0.1.0 tag 真实 fixture。
+    # test job 已跑 cross_version_compat（debug + --all-features）；本 job 加
+    # --release 更彻底 + 独立失败信号（cross-version vs same-version round-trip
+    # 分离）。取舍：独立 job（非合并 corpus-compat）——DoD 5 job 1:1 映射 +
+    # 独立可见性 + --release thoroughness。--all-features 覆盖 zstd-encode 分支
+    # （cross_version_compat.rs:335 #[cfg(feature="zstd-encode")] 断言）。
+    needs: test
+    runs-on: ubuntu-latest
+    timeout-minutes: 15
+    steps:
+      - uses: actions/checkout@v4
+      - uses: dtolnay/rust-toolchain@stable
+      - uses: Swatinem/rust-cache@v2
+      - name: Cross-version compat (v0.1.0 fixture, --release)
+        run: cargo test --test cross_version_compat -p vane-core --all-features --release
+
+  stress:
+    # M4 阶段四 并发压测：stress_concurrency --release + 3 次 multi-run。
+    # test job 已跑 stress_concurrency（debug，单次，--all-features）；本 job 加
+    # --release + 3 次 multi-run 更彻底（验 release 优化下并发安全 + 降低竞态
+    # flaky 漏检）。default features（stress 不需 fault-injection/zstd——并发安全
+    # 与存储格式正交）。3 次 multi-run 捕捉低概率竞态。
+    needs: test
+    runs-on: ubuntu-latest
+    timeout-minutes: 20
+    steps:
+      - uses: actions/checkout@v4
+      - uses: dtolnay/rust-toolchain@stable
+      - uses: Swatinem/rust-cache@v2
+      - name: Stress concurrency (--release, 3 multi-runs)
+        run: |
+          for i in 1 2 3; do
+            echo "=== stress run $i/3 ==="
+            cargo test --test stress_concurrency -p vane-core --release
+          done
+
+  crash-recovery:
+    # M4 阶段二 崩溃恢复：crash_recovery 5 场景（meta_slot/WAL/merge/ENOSPC/
+    # 部分写，FaultVfs 注入）--release。test job 已跑 crash_recovery（--all-features
+    # 含 fault-injection，debug）；本 job 加 --release 更彻底（验 release 优化下
+    # 崩溃恢复一致性）。crash_recovery.rs 门控 #![cfg(feature = "fault-injection")]
+    # → 须 --features fault-injection。仅 fault-injection（不需 zstd-encode——
+    # crash 场景用 v1 stored 格式，本地 --no-run 编译验证通过）。
+    needs: test
+    runs-on: ubuntu-latest
+    timeout-minutes: 15
+    steps:
+      - uses: actions/checkout@v4
+      - uses: dtolnay/rust-toolchain@stable
+      - uses: Swatinem/rust-cache@v2
+      - name: Crash recovery (5 scenarios, FaultVfs, --release)
+        run: cargo test --test crash_recovery -p vane-core --features fault-injection --release
