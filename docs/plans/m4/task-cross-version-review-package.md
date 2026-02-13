
## Commits 1fc03af..6300392

6300392 test(core): cross_version_compat v0.1.0 fixture + v1/v2 共存（M4 阶段三 a）

## Diff stat

 crates/vane-core/tests/cross_version_compat.rs     | 446 +++++++++++++++++++++
 crates/vane-core/tests/fixtures/compat/README.md   |  72 ++++
 .../tests/fixtures/compat/v0.1.0/db/manifest.json  |   1 +
 .../seg_01KZRQ9VAJ0000000000000000/header.bin      | Bin 0 -> 91 bytes
 .../seg_01KZRQ9VAJ0000000000000000/hnsw.bin        | Bin 0 -> 158 bytes
 .../seg_01KZRQ9VAJ0000000000000000/idmap.bin       | Bin 0 -> 107 bytes
 .../seg_01KZRQ9VAJ0000000000000000/inverted.bin    | Bin 0 -> 709 bytes
 .../seg_01KZRQ9VAJ0000000000000000/scalars.col     | Bin 0 -> 54 bytes
 .../seg_01KZRQ9VAJ0000000000000000/stored.bin      | Bin 0 -> 321 bytes
 .../seg_01KZRQ9VAJ0000000000000000/vectors.bin     | Bin 0 -> 92 bytes
 .../tests/fixtures/compat/v0.1.0/db/wal.log        |   1 +
 docs/plans/m4/format-freeze-note.md                |  68 ++++
 docs/plans/m4/task-cross-version-report.md         | 191 +++++++++
 scripts/gen_compat_fixture.rs                      | 124 ++++++
 14 files changed, 903 insertions(+)

## Full diff (U10)

diff --git a/crates/vane-core/tests/cross_version_compat.rs b/crates/vane-core/tests/cross_version_compat.rs
new file mode 100644
index 0000000..ee4c19f
--- /dev/null
+++ b/crates/vane-core/tests/cross_version_compat.rs
@@ -0,0 +1,446 @@
+// tests/cross_version_compat.rs — M4 §3.4 跨版本持久化兼容测试
+//
+// 验证当前版本能读 v0.1.0 tag 真实生成的 fixture（非当前代码模拟）。
+// fixture 生成方式见 tests/fixtures/compat/README.md + scripts/gen_compat_fixture.rs。
+//
+// fixture 段文件 per-file format_version（v0.1.0 产物，当前版本双模读取）：
+//   header.bin   V1   vectors.bin  V2   stored.bin   V1   idmap.bin   V1
+//   scalars.col  V1   inverted.bin V1   hnsw.bin     V1
+//
+// 已知文档集（fixture-gen 确定性输入，baseline 断言用）：
+//   v010-d0 vec=[1,0,0,0] tag=a "向量检索 混合搜索 hybrid search engine"
+//   v010-d1 vec=[0,1,0,0] tag=b "BM25 ranking text retrieval"
+//   v010-d2 vec=[0,0,1,0] tag=a "机器学习 与 搜索引擎 ranking"
+//   v010-d3 vec=[1,1,0,0] tag=c "cosine similarity vector space"
+//   v010-d4 vec=[0,0,0,1] tag=b "全文检索 inverted index 倒排"
+//
+// 向量搜索 [1,0,0,0] baseline（cosine_score，score 降序，同分 docid 升序）：
+//   d0 score=1.0 > d3 score≈0.707 > d1=d2=d4 score=0.0（按 docid 升序）
+
+use std::sync::atomic::{AtomicU64, Ordering};
+use std::sync::Arc;
+
+use vane_core::api::{
+    CollectionOptions, Db, Doc, FusionSpec, OpenOptions, SearchMode, SearchQuery,
+};
+use vane_core::types::{FieldDef, Metric, ScalarKind, Schema};
+use vane_core::vfs::std_fs::StdFsVfs;
+use vane_core::vfs::Vfs;
+
+/// fixture 段 ULID（v0.1.0 确定性产物，当前版本不应修改）。
+const FIXTURE_SEG_ULID: &str = "01KZRQ9VAJ0000000000000000";
+
+/// 已知文档 external_id 集合（fixture-gen 确定性输入）。
+const KNOWN_IDS: [&str; 5] = ["v010-d0", "v010-d1", "v010-d2", "v010-d3", "v010-d4"];
+
+fn fixture_root() -> String {
+    format!(
+        "{}/tests/fixtures/compat/v0.1.0",
+        env!("CARGO_MANIFEST_DIR")
+    )
+}
+
+fn build_schema() -> Schema {
+    Schema::new(vec![
+        ("body".into(), FieldDef::Text),
+        (
+            "v".into(),
+            FieldDef::Vector {
+                dim: 4,
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
+fn unique_dir(label: &str) -> std::path::PathBuf {
+    static COUNTER: AtomicU64 = AtomicU64::new(0);
+    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
+    std::env::temp_dir().join(format!(
+        "vane-cross-version-{}-{}-{}-{}",
+        label,
+        std::process::id(),
+        n,
+        std::time::SystemTime::now()
+            .duration_since(std::time::UNIX_EPOCH)
+            .unwrap()
+            .as_nanos()
+    ))
+}
+
+/// 递归复制目录（跨平台，不依赖 cp 命令）。
+fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
+    std::fs::create_dir_all(dst)?;
+    for entry in std::fs::read_dir(src)? {
+        let entry = entry?;
+        let path = entry.path();
+        let dst_path = dst.join(entry.file_name());
+        if path.is_dir() {
+            copy_dir_recursive(&path, &dst_path)?;
+        } else {
+            std::fs::copy(&path, &dst_path)?;
+        }
+    }
+    Ok(())
+}
+
+/// 复制 fixture 到临时目录（保持 fixture 源不被修改），返回临时目录路径。
+fn copy_fixture_to_temp(label: &str) -> std::path::PathBuf {
+    let dir = unique_dir(label);
+    let _ = std::fs::remove_dir_all(&dir);
+    std::fs::create_dir_all(&dir).unwrap();
+    let root = fixture_root();
+    let src = std::path::Path::new(&root);
+    copy_dir_recursive(src, &dir).expect("复制 fixture 失败");
+    dir
+}
+
+/// 读段文件 format_version（LE u32，offset=4，magic 之后 4 字节）。
+fn read_format_version(vfs: &Arc<dyn Vfs>, path: &str) -> u32 {
+    let mut buf = [0u8; 8];
+    let n = vfs.read_at(path, &mut buf, 0).expect("read_at 失败");
+    assert!(n >= 8, "{} 不足 8 字节", path);
+    assert_eq!(&buf[0..4], b"VANE", "{} magic 错误", path);
+    u32::from_le_bytes(buf[4..8].try_into().unwrap())
+}
+
+/// 收集 hit 的 external_id 集合。
+fn hit_ids(hits: &[vane_core::api::Hit]) -> std::collections::HashSet<String> {
+    hits.iter().map(|h| h.id.clone()).collect()
+}
+
+// =============================================================================
+// 测试 1：当前版本读 v0.1.0 fixture
+// =============================================================================
+
+/// 验证当前版本能读 v0.1.0 tag 真实生成的 fixture：
+/// - manifest restore：collection "docs" 可见
+/// - 段文件 format_version 与 fixture 一致（非 vacuous）
+/// - external_id 全回填 == 已知集
+/// - vector search baseline 一致（d0 排第一 score≈1.0，d3 第二 score≈0.707）
+/// - text/hybrid search 命中（非 vacuous）
+#[test]
+fn reads_v0_1_0_fixture() {
+    let dir = copy_fixture_to_temp("reads");
+    let vfs = Arc::new(StdFsVfs::with_root(dir.to_str().unwrap())) as Arc<dyn Vfs>;
+
+    let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
+    // manifest restore：collections 含 "docs"
+    assert!(
+        db.collections().iter().any(|c| c == "docs"),
+        "manifest 应 restore collection 'docs'"
+    );
+    let col = db
+        .collection("docs", build_schema(), CollectionOptions::default())
+        .unwrap();
+
+    // 段文件 format_version 与 fixture 一致（非 vacuous：检 per-file version 常量）
+    let seg_path = format!("db/segments/seg_{}", FIXTURE_SEG_ULID);
+    assert_eq!(
+        read_format_version(&vfs, &format!("{}/header.bin", seg_path)),
+        vane_core::types::HEADER_FORMAT_V1,
+        "header.bin V1"
+    );
+    assert_eq!(
+        read_format_version(&vfs, &format!("{}/vectors.bin", seg_path)),
+        vane_core::types::VECTORS_FORMAT_V2,
+        "vectors.bin V2（v0.1.0 始终写 v2）"
+    );
+    assert_eq!(
+        read_format_version(&vfs, &format!("{}/stored.bin", seg_path)),
+        vane_core::types::STORED_FORMAT_V1,
+        "stored.bin V1（v0.1.0 无 zstd-encode）"
+    );
+    assert_eq!(
+        read_format_version(&vfs, &format!("{}/idmap.bin", seg_path)),
+        vane_core::types::IDMAP_FORMAT_V1,
+        "idmap.bin V1"
+    );
+    assert_eq!(
+        read_format_version(&vfs, &format!("{}/scalars.col", seg_path)),
+        vane_core::types::SCALARS_FORMAT_V1,
+        "scalars.col V1"
+    );
+    assert_eq!(
+        read_format_version(&vfs, &format!("{}/hnsw.bin", seg_path)),
+        vane_core::types::HNSW_FORMAT_V1,
+        "hnsw.bin V1"
+    );
+
+    // ---- vector search baseline：query=[1,0,0,0] top_k=10 ----
+    let hits = col
+        .search(&SearchQuery {
+            text: None,
+            vector: Some(vec![1.0, 0.0, 0.0, 0.0]),
+            top_k: 10,
+            mode: SearchMode::Vector,
+            fusion: FusionSpec::Rrf,
+            filter: None,
+            candidate_multiplier: 3,
+        })
+        .unwrap();
+
+    // 非空断言：应返回全部 5 文档
+    assert!(!hits.is_empty(), "vector search 应返回结果");
+    assert_eq!(hits.len(), 5, "vector search 应返回全部 5 文档");
+
+    // external_id 全回填 == 已知集
+    let ids = hit_ids(&hits);
+    for id in &KNOWN_IDS {
+        assert!(ids.contains(*id), "缺少已知文档 {}", id);
+    }
+    assert_eq!(ids.len(), 5, "external_id 集合应恰好 5 条");
+
+    // baseline 顺序：d0(score≈1.0) > d3(score≈0.707) > d1=d2=d4(score=0.0, docid 升序)
+    assert_eq!(hits[0].id, "v010-d0", "d0 应排第一（cosine=1.0）");
+    assert!(
+        (hits[0].score - 1.0).abs() < 1e-6,
+        "d0 score 应为 1.0，实际 {}",
+        hits[0].score
+    );
+    assert_eq!(hits[1].id, "v010-d3", "d3 应排第二（cosine≈0.707）");
+    let expected_d3 = 1.0_f32 / 2.0_f32.sqrt();
+    assert!(
+        (hits[1].score - expected_d3).abs() < 1e-6,
+        "d3 score 应为 {}，实际 {}",
+        expected_d3,
+        hits[1].score
+    );
+    // 同分 docid 升序：d1 < d2 < d4
+    assert_eq!(hits[2].id, "v010-d1");
+    assert_eq!(hits[3].id, "v010-d2");
+    assert_eq!(hits[4].id, "v010-d4");
+
+    // stored fields 回填（tag 字段）
+    for h in &hits {
+        assert!(h.fields.is_some(), "stored fields 应回填 for {}", h.id);
+        let fields = h.fields.as_ref().unwrap();
+        assert!(fields.contains_key("tag"), "tag 字段应回填 for {}", h.id);
+    }
+
+    // ---- text search：query="检索"（d0/d2/d4 含"检索"）----
+    let text_hits = col
+        .search(&SearchQuery {
+            text: Some("检索".into()),
+            vector: None,
+            top_k: 10,
+            mode: SearchMode::Text,
+            fusion: FusionSpec::Rrf,
+            filter: None,
+            candidate_multiplier: 3,
+        })
+        .unwrap();
+    assert!(!text_hits.is_empty(), "text search 应有命中（非 vacuous）");
+    let text_ids = hit_ids(&text_hits);
+    // "检索" 出现在 d0, d2, d4
+    assert!(text_ids.contains("v010-d0"), "text search 应命中 d0");
+    assert!(text_ids.contains("v010-d2"), "text search 应命中 d2");
+    assert!(text_ids.contains("v010-d4"), "text search 应命中 d4");
+
+    // ---- hybrid search：text="检索" + vector=[1,0,0,0] ----
+    let hybrid_hits = col
+        .search(&SearchQuery {
+            text: Some("检索".into()),
+            vector: Some(vec![1.0, 0.0, 0.0, 0.0]),
+            top_k: 10,
+            mode: SearchMode::Hybrid,
+            fusion: FusionSpec::Rrf,
+            filter: None,
+            candidate_multiplier: 3,
+        })
+        .unwrap();
+    assert!(
+        !hybrid_hits.is_empty(),
+        "hybrid search 应有命中（非 vacuous）"
+    );
+    let hybrid_ids = hit_ids(&hybrid_hits);
+    for id in &KNOWN_IDS {
+        assert!(hybrid_ids.contains(*id), "hybrid search 应包含 {}", id);
+    }
+
+    db.close().unwrap();
+    let _ = std::fs::remove_dir_all(&dir);
+}
+
+// =============================================================================
+// 测试 2：v1/v2 段共存（fixture v1 stored.bin + 当前 flush 新段）
+// =============================================================================
+
+/// 同一 DB 内混合 v1（旧 fixture 段 stored.bin V1）+ v2（当前 flush 新段 stored.bin V2
+/// 仅 zstd-encode feature 启用时；否则 V1）。search 结果应包含两段文档，一致。
+#[test]
+fn v1_and_v2_segments_coexist() {
+    let dir = copy_fixture_to_temp("v1v2");
+    let vfs = Arc::new(StdFsVfs::with_root(dir.to_str().unwrap())) as Arc<dyn Vfs>;
+
+    let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
+    let col = db
+        .collection("docs", build_schema(), CollectionOptions::default())
+        .unwrap();
+
+    // ---- 旧段（fixture）格式版本确认 ----
+    let segs_before = vfs.list("db/segments").unwrap();
+    assert_eq!(
+        segs_before.len(),
+        1,
+        "初始应有 1 段（fixture 段 {}）",
+        FIXTURE_SEG_ULID
+    );
+    let old_seg = segs_before[0].clone();
+    assert_eq!(old_seg, format!("seg_{}", FIXTURE_SEG_ULID));
+    let old_stored_ver = read_format_version(&vfs, &format!("db/segments/{}/stored.bin", old_seg));
+    assert_eq!(
+        old_stored_ver,
+        vane_core::types::STORED_FORMAT_V1,
+        "fixture 段 stored.bin 应为 V1"
+    );
+
+    // ---- 添加新文档 + flush → 新段 ----
+    let new_docs = vec![
+        Doc {
+            id: "v010-d5".into(),
+            text: Some("新文档 cross version compatibility".into()),
+            vector: Some(vec![1.0, 1.0, 1.0, 0.0]),
+            meta: None,
+        },
+        Doc {
+            id: "v010-d6".into(),
+            text: Some("compatibility test segment coexist".into()),
+            vector: Some(vec![0.0, 1.0, 1.0, 1.0]),
+            meta: None,
+        },
+    ];
+    let report = col.add(&new_docs).unwrap();
+    assert_eq!(report.accepted, 2);
+    col.flush().unwrap();
+
+    // ---- 确认 2 段 ----
+    let segs_after = vfs.list("db/segments").unwrap();
+    assert_eq!(segs_after.len(), 2, "flush 后应有 2 段");
+    let new_seg = segs_after
+        .iter()
+        .find(|s| **s != old_seg)
+        .expect("应有新段")
+        .clone();
+
+    // 新段 stored.bin 版本：zstd-encode 启用 → V2（真 v1/v2 共存）；否则 V1
+    let new_stored_ver = read_format_version(&vfs, &format!("db/segments/{}/stored.bin", new_seg));
+    #[cfg(feature = "zstd-encode")]
+    {
+        assert_eq!(
+            new_stored_ver,
+            vane_core::types::STORED_FORMAT_V2,
+            "新段 stored.bin 应为 V2（zstd-encode 启用）→ 真 v1/v2 共存"
+        );
+    }
+    #[cfg(not(feature = "zstd-encode"))]
+    {
+        assert_eq!(
+            new_stored_ver,
+            vane_core::types::STORED_FORMAT_V1,
+            "新段 stored.bin V1（无 zstd-encode）→ 双段同格式共存"
+        );
+    }
+
+    // ---- search 一致：应返回全部 7 文档（5 旧 + 2 新）----
+    let hits = col
+        .search(&SearchQuery {
+            text: None,
+            vector: Some(vec![1.0, 0.0, 0.0, 0.0]),
+            top_k: 10,
+            mode: SearchMode::Vector,
+            fusion: FusionSpec::Rrf,
+            filter: None,
+            candidate_multiplier: 3,
+        })
+        .unwrap();
+    assert_eq!(hits.len(), 7, "应返回全部 7 文档（5 旧 + 2 新）");
+
+    // 旧文档全部可见
+    let ids = hit_ids(&hits);
+    for id in &KNOWN_IDS {
+        assert!(ids.contains(*id), "旧文档 {} 应仍可见", id);
+    }
+    // 新文档可见
+    assert!(ids.contains("v010-d5"), "新文档 v010-d5 应可见");
+    assert!(ids.contains("v010-d6"), "新文档 v010-d6 应可见");
+
+    // d0 仍排第一（score=1.0，来自旧段）
+    assert_eq!(hits[0].id, "v010-d0", "d0 仍应排第一");
+    assert!((hits[0].score - 1.0).abs() < 1e-6, "d0 score 仍应为 1.0");
+
+    // ---- hybrid search 也一致 ----
+    let hybrid_hits = col
+        .search(&SearchQuery {
+            text: Some("检索 compatibility".into()),
+            vector: Some(vec![1.0, 0.0, 0.0, 0.0]),
+            top_k: 10,
+            mode: SearchMode::Hybrid,
+            fusion: FusionSpec::Rrf,
+            filter: None,
+            candidate_multiplier: 3,
+        })
+        .unwrap();
+    assert!(!hybrid_hits.is_empty(), "hybrid search 应有命中");
+    let hybrid_ids = hit_ids(&hybrid_hits);
+    // d0 来自旧段（含"检索"），d5/d6 来自新段（含"compatibility"）
+    assert!(hybrid_ids.contains("v010-d0"), "hybrid 应命中旧段 d0");
+    assert!(
+        hybrid_ids.contains("v010-d5") || hybrid_ids.contains("v010-d6"),
+        "hybrid 应命中新段文档"
+    );
+
+    db.close().unwrap();
+    let _ = std::fs::remove_dir_all(&dir);
+}
+
+// =============================================================================
+// 测试 3：迁移占位（未来格式升级时实现迁移器）
+// =============================================================================
+
+/// 当前 v1 不需迁移（双模读取 v1/v2 已覆盖兼容）。
+/// 未来格式升级（如 v3 stored/vectors）时实现迁移器：
+/// 1. 遍历所有旧格式段
+/// 2. 读 v1/v2 数据
+/// 3. 用新格式重写（flush 新段 + manifest 切换 + WAL）
+/// 4. 删旧段
+///
+/// 当前标 `#[ignore]`——骨架在此，实现留待未来格式 bump。
+#[test]
+#[ignore = "当前 v1/v2 双模读取覆盖兼容；未来格式升级（v3+）时实现迁移器"]
+fn migrates_v0_1_0_via_reindex() {
+    let dir = copy_fixture_to_temp("migrate");
+    let vfs = Arc::new(StdFsVfs::with_root(dir.to_str().unwrap())) as Arc<dyn Vfs>;
+
+    let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
+    let col = db
+        .collection("docs", build_schema(), CollectionOptions::default())
+        .unwrap();
+
+    // 未来迁移器调用骨架：
+    // col.migrate_segments(FormatMigrationTarget::V3).unwrap();
+
+    // 当前：验证 v1 段可读（无需迁移）
+    let hits = col
+        .search(&SearchQuery {
+            text: None,
+            vector: Some(vec![1.0, 0.0, 0.0, 0.0]),
+            top_k: 10,
+            mode: SearchMode::Vector,
+            fusion: FusionSpec::Rrf,
+            filter: None,
+            candidate_multiplier: 3,
+        })
+        .unwrap();
+    assert_eq!(hits.len(), 5, "v1 段可读，5 文档全可见");
+
+    db.close().unwrap();
+    let _ = std::fs::remove_dir_all(&dir);
+}
diff --git a/crates/vane-core/tests/fixtures/compat/README.md b/crates/vane-core/tests/fixtures/compat/README.md
new file mode 100644
index 0000000..06403a5
--- /dev/null
+++ b/crates/vane-core/tests/fixtures/compat/README.md
@@ -0,0 +1,72 @@
+# 跨版本兼容 fixture
+
+> 来源：M4 阶段三 a（跨版本持久化兼容，§3.4）。
+> 此目录提交真实 v0.1.0 tag 生成的段文件 fixture，供 `tests/cross_version_compat.rs` 读取验证。
+
+## 目录结构
+
+```
+compat/
+├── v0.1.0/
+│   └── db/                              # Db::open(vfs, "db", ...) 路径
+│       ├── manifest.json                # v0.1.0 manifest（collection "docs" schema + segment_ulids）
+│       ├── segments/
+│       │   └── seg_01KZRQ9VAJ0000000000000000/
+│       │       ├── header.bin          # HEADER_FORMAT_V1
+│       │       ├── vectors.bin          # VECTORS_FORMAT_V2（v0.1.0 始终写 v2，含 dim 头）
+│       │       ├── stored.bin           # STORED_FORMAT_V1（无 zstd-encode，裸 JSON）
+│       │       ├── idmap.bin            # IDMAP_FORMAT_V1
+│       │       ├── scalars.col          # SCALARS_FORMAT_V1
+│       │       ├── inverted.bin         # FORMAT_VERSION=1
+│       │       └── hnsw.bin             # HNSW_FORMAT_V1
+│       └── wal.log                      # AddSegment 记录
+└── README.md                            # 本文件
+```
+
+## fixture 来源
+
+- **生成方式**：用 v0.1.0 tag 的 vane-core API 真实生成（非当前代码模拟）。
+- **生成步骤**（可复现）：
+  1. `git worktree add --detach /tmp/vane-v010 v0.1.0`
+  2. 在 worktree 写 `crates/vane-core/tests/gen_compat_fixture.rs`（用 v0.1.0 API 创建 DB + 加已知文档集 + flush）。
+  3. `cd /tmp/vane-v010 && cargo test -p vane-core --test gen_compat_fixture`。
+  4. 产物在 `/tmp/v010-fixture/db/`，拷贝至 `crates/vane-core/tests/fixtures/compat/v0.1.0/db/`。
+  5. `git worktree remove /tmp/vane-v010 --force`（清理 worktree）。
+- **生成脚本镜像**：`scripts/gen_compat_fixture.rs`（非 workspace 编译目标，仅供文档/复现参考）。
+
+## 已知文档集（baseline 断言用）
+
+fixture 含 5 篇中英混排确定性文档（fixture-gen 确定性输入，非随机）：
+
+| external_id | tag | text | vector |
+|---|---|---|---|
+| v010-d0 | a | "向量检索 混合搜索 hybrid search engine" | [1,0,0,0] |
+| v010-d1 | b | "BM25 ranking text retrieval" | [0,1,0,0] |
+| v010-d2 | a | "机器学习 与 搜索引擎 ranking" | [0,0,1,0] |
+| v010-d3 | c | "cosine similarity vector space" | [1,1,0,0] |
+| v010-d4 | b | "全文检索 inverted index 倒排" | [0,0,0,1] |
+
+schema：`body=Text, v=Vector{dim=4, Cosine}, tag=Scalar{Keyword}`。
+
+## 格式版本
+
+fixture 段文件 per-file format_version（v0.1.0 产物，当前版本双模读取）：
+
+| 文件 | format_version | 说明 |
+|---|---|---|
+| header.bin | V1 | 段元数据入口 |
+| vectors.bin | V2 | v0.1.0 始终写 v2（含 dim 头，12 字节） |
+| stored.bin | V1 | v0.1.0 无 zstd-encode → 裸 JSON |
+| idmap.bin | V1 | external_id → docid 映射 |
+| scalars.col | V1 | 标量列存 |
+| inverted.bin | 1 | 倒排索引 |
+| hnsw.bin | V1 | HNSW 图 |
+
+## 体积约束
+
+- fixture 总体积 36KB（<100KB 约束满足）。
+- 最大文件：inverted.bin 709B。最小文件：scalars.col 54B。
+
+## 不要修改
+
+fixture 是冻结的 v0.1.0 产物，**禁止修改**。测试用 `copy_fixture_to_temp()` 复制到临时目录后操作，保持 fixture 源不被修改。如需重新生成，按上述步骤在 v0.1.0 tag worktree 离线运行。
diff --git a/crates/vane-core/tests/fixtures/compat/v0.1.0/db/manifest.json b/crates/vane-core/tests/fixtures/compat/v0.1.0/db/manifest.json
new file mode 100644
index 0000000..c644ef0
--- /dev/null
+++ b/crates/vane-core/tests/fixtures/compat/v0.1.0/db/manifest.json
@@ -0,0 +1 @@
+{"version":1,"collections":{"docs":{"schema":{"fields":[["body","Text"],["v",{"Vector":{"dim":4,"metric":"Cosine"}}],["tag",{"Scalar":{"kind":"Keyword"}}]]},"tokenizer_kind":"Standard","tokenizer_id":[2,239,175,198,55,74,249,62,221,85,48,237,150,3,122,142,123,93,234,87,101,141,201,186,22,64,67,252,206,105,33,166],"user_dict":[],"segment_ulids":["01KZRQ9VAJ0000000000000000"]}}}
\ No newline at end of file
diff --git a/crates/vane-core/tests/fixtures/compat/v0.1.0/db/segments/seg_01KZRQ9VAJ0000000000000000/header.bin b/crates/vane-core/tests/fixtures/compat/v0.1.0/db/segments/seg_01KZRQ9VAJ0000000000000000/header.bin
new file mode 100644
index 0000000..5b96331
Binary files /dev/null and b/crates/vane-core/tests/fixtures/compat/v0.1.0/db/segments/seg_01KZRQ9VAJ0000000000000000/header.bin differ
diff --git a/crates/vane-core/tests/fixtures/compat/v0.1.0/db/segments/seg_01KZRQ9VAJ0000000000000000/hnsw.bin b/crates/vane-core/tests/fixtures/compat/v0.1.0/db/segments/seg_01KZRQ9VAJ0000000000000000/hnsw.bin
new file mode 100644
index 0000000..ebdcb48
Binary files /dev/null and b/crates/vane-core/tests/fixtures/compat/v0.1.0/db/segments/seg_01KZRQ9VAJ0000000000000000/hnsw.bin differ
diff --git a/crates/vane-core/tests/fixtures/compat/v0.1.0/db/segments/seg_01KZRQ9VAJ0000000000000000/idmap.bin b/crates/vane-core/tests/fixtures/compat/v0.1.0/db/segments/seg_01KZRQ9VAJ0000000000000000/idmap.bin
new file mode 100644
index 0000000..2c27ff5
Binary files /dev/null and b/crates/vane-core/tests/fixtures/compat/v0.1.0/db/segments/seg_01KZRQ9VAJ0000000000000000/idmap.bin differ
diff --git a/crates/vane-core/tests/fixtures/compat/v0.1.0/db/segments/seg_01KZRQ9VAJ0000000000000000/inverted.bin b/crates/vane-core/tests/fixtures/compat/v0.1.0/db/segments/seg_01KZRQ9VAJ0000000000000000/inverted.bin
new file mode 100644
index 0000000..bfb602a
Binary files /dev/null and b/crates/vane-core/tests/fixtures/compat/v0.1.0/db/segments/seg_01KZRQ9VAJ0000000000000000/inverted.bin differ
diff --git a/crates/vane-core/tests/fixtures/compat/v0.1.0/db/segments/seg_01KZRQ9VAJ0000000000000000/scalars.col b/crates/vane-core/tests/fixtures/compat/v0.1.0/db/segments/seg_01KZRQ9VAJ0000000000000000/scalars.col
new file mode 100644
index 0000000..879245c
Binary files /dev/null and b/crates/vane-core/tests/fixtures/compat/v0.1.0/db/segments/seg_01KZRQ9VAJ0000000000000000/scalars.col differ
diff --git a/crates/vane-core/tests/fixtures/compat/v0.1.0/db/segments/seg_01KZRQ9VAJ0000000000000000/stored.bin b/crates/vane-core/tests/fixtures/compat/v0.1.0/db/segments/seg_01KZRQ9VAJ0000000000000000/stored.bin
new file mode 100644
index 0000000..ebbf875
Binary files /dev/null and b/crates/vane-core/tests/fixtures/compat/v0.1.0/db/segments/seg_01KZRQ9VAJ0000000000000000/stored.bin differ
diff --git a/crates/vane-core/tests/fixtures/compat/v0.1.0/db/segments/seg_01KZRQ9VAJ0000000000000000/vectors.bin b/crates/vane-core/tests/fixtures/compat/v0.1.0/db/segments/seg_01KZRQ9VAJ0000000000000000/vectors.bin
new file mode 100644
index 0000000..157e5e7
Binary files /dev/null and b/crates/vane-core/tests/fixtures/compat/v0.1.0/db/segments/seg_01KZRQ9VAJ0000000000000000/vectors.bin differ
diff --git a/crates/vane-core/tests/fixtures/compat/v0.1.0/db/wal.log b/crates/vane-core/tests/fixtures/compat/v0.1.0/db/wal.log
new file mode 100644
index 0000000..f234797
--- /dev/null
+++ b/crates/vane-core/tests/fixtures/compat/v0.1.0/db/wal.log
@@ -0,0 +1 @@
+{"AddSegment":{"collection":"docs","ulid":"01KZRQ9VAJ0000000000000000"}}
diff --git a/docs/plans/m4/format-freeze-note.md b/docs/plans/m4/format-freeze-note.md
new file mode 100644
index 0000000..3b58d14
--- /dev/null
+++ b/docs/plans/m4/format-freeze-note.md
@@ -0,0 +1,68 @@
+# 格式冻结承诺（供 Phase 6 SPEC §6.2 修订）
+
+> 来源：M4 阶段三 a（跨版本兼容 fixture）产出。
+> 供 Phase 6 SPEC §6.2 修订参考——列 per-file format_version 哪些冻结（v1 不可变）、
+> 哪些可演进（v2 zstd 等）、迁移策略。
+> 实证基础：v0.1.0 tag 真实 fixture + 当前版本双模读取测试（`tests/cross_version_compat.rs`）。
+
+## 1. per-file format_version 冻结/演进矩阵
+
+| 段文件 | 常量 | 当前版本 | 冻结状态 | 演进策略 |
+|---|---|---|---|---|
+| `header.bin` | `HEADER_FORMAT_V1` | 1 | **冻结**（v1 不可变） | header 是段元数据入口（magic+version+tokenizer_id+docid_range+tombstone）。格式变更破坏所有段读取，故冻结。如需演进须 bump version + 双模读 + 迁移器。 |
+| `vectors.bin` | `VECTORS_FORMAT_V1` / `VECTORS_FORMAT_V2` | 1(读)/2(写) | **v1 冻结，v2 可演进** | v1=8 字节头（magic+version，无 dim，懒加载从 payload 反推）。v2=12 字节头（含 dim，M2-07 open 期预存 dim）。当前版本始终写 v2，v1 读取保留（双模）。v0.1.0 fixture 实测：v0.1.0 始终写 v2。v1 读路径覆盖预发布历史数据（无已发布 v1 vectors.bin 产物，但读路径保留）。 |
+| `stored.bin` | `STORED_FORMAT_V1` / `STORED_FORMAT_V2` | 1(读)/1或2(写) | **v1 冻结，v2 可演进** | v1=裸 JSON（magic+version+count+entries）。v2=zstd 块压缩（magic+version+raw_len+zstd_len+zstd_block，M2-08）。zstd-encode feature 启用时写 v2，否则写 v1。v0.1.0 fixture 实测：无 zstd-encode → 写 v1。双模读取（decode_stored 按 version 分支），旧 v1 段只读服务至段合并自然清除（不做原地迁移）。 |
+| `idmap.bin` | `IDMAP_FORMAT_V1` | 1 | **冻结**（v1 不可变） | external_id → docid 映射。格式简单（magic+version+count+entries），无演进需求。 |
+| `scalars.col` | `SCALARS_FORMAT_V1` | 1 | **冻结**（v1 不可变） | 标量列存（magic+version+count+columns）。格式简单，无演进需求。 |
+| `inverted.bin` | `FORMAT_VERSION=1` | 1 | **冻结**（v1 不可变） | 倒排索引（magic+version+count+postings）。BM25 参数 k1=1.2/b=0.75 冻结（§6.2）。格式变更破坏 BM25 排序，故冻结。 |
+| `hnsw.bin` | `HNSW_FORMAT_V1` | 1 | **冻结**（v1 不可变） | HNSW 图结构（magic+version+dim+graph）。格式变更破坏 HNSW 导航，须 bump + 迁移。fallback brute 路径在 hnsw 缺失/损坏时降级。 |
+| `manifest.json` | `version=1` | 1 | **冻结**（schema 版本） | manifest schema（version+collections）。collection schema 变更须 reindex（新分词身份），非格式 version。 |
+| `wal.log` | N/A（行 JSON） | N/A | **冻结**（WAL 记录格式） | WAL 是 JSON 行（AddSegment/DeleteSegment/AddTombstone）。记录格式变更须双模读 + recover 兼容。 |
+
+## 2. 迁移策略
+
+### 2.1 当前状态（v0.1.0 → v0.2.0）
+
+- **无需迁移**：v0.1.0 与当前版本（v0.2.0）的段文件格式**完全一致**（segment/mod.rs diff 为空，persistence/wal diff 为空）。
+- **双模读取**：v1/v2 format_version 双模读取已实现（M2-08），覆盖 stored.bin v1(裸JSON)/v2(zstd) + vectors.bin v1(8字节头)/v2(12字节头含dim)。
+- **v0.1.0 fixture 实测**：v0.1.0 产物为 header.bin V1 + vectors.bin V2 + stored.bin V1 + idmap.bin V1 + scalars.col V1 + inverted.bin V1 + hnsw.bin V1。当前版本读此 fixture 数据一致（`reads_v0_1_0_fixture` 测试通过）。
+
+### 2.2 未来格式升级策略（v3+）
+
+当未来版本需演进某个 per-file format_version（如 stored.bin v3 新压缩算法）时：
+
+1. **bump version**：`STORED_FORMAT_V3 = 3`。
+2. **双模读取**：`decode_stored` 加 v3 分支（v1/v2/v3 三模读），旧段只读服务至段合并自然清除。
+3. **不做原地迁移**：旧段不可变（SPEC §6.2 铁律），迁移通过 merge 自然完成（merge 写新格式段，manifest 切换后旧段删除）。
+4. **迁移器占位**：`migrates_v0_1_0_via_reindex` 测试标 `#[ignore]`，骨架已建。未来 v3+ 启用时，实现迁移器调用（遍历旧段 → 读 v1/v2 → flush 新段 → manifest 切换 → WAL）。
+5. **corpus 兼容测试**：新格式须通过 `tests/corpus_compat.rs`（v2 roundtrip）+ `tests/cross_version_compat.rs`（v0.1.0 fixture 读取）。
+
+### 2.3 格式冻结承诺
+
+- **v1 不可变**：所有 `*_FORMAT_V1` 常量对应的格式**永不变更**（v1 读路径冻结）。任何 v1 文件（header/v1-vectors/v1-stored/idmap/scalars/inverted/hnsw）当前版本及未来版本必须能读。
+- **v2 可演进**：`VECTORS_FORMAT_V2` / `STORED_FORMAT_V2` 可演进为 v3，但 v2 读路径冻结（v2 文件必须能读）。演进通过 bump version + 双模读 + merge 自然迁移。
+- **manifest/wal 冻结**：manifest schema version=1 冻结（collection schema 变更经 reindex）；WAL 记录格式冻结（recover 双模读）。
+- **BM25 参数冻结**：k1=1.2/b=0.75 冻结（进 format_version 语义），变更须 bump inverted.bin version + 迁移。
+
+## 3. cross_version_compat.rs 测试覆盖
+
+| 测试 | 覆盖内容 | 格式断言 |
+|---|---|---|
+| `reads_v0_1_0_fixture` | 当前版本读 v0.1.0 fixture（真实 tag 产物） | header V1 + vectors V2 + stored V1 + idmap V1 + scalars V1 + hnsw V1 + external_id 全回填 + search baseline 一致 |
+| `v1_and_v2_segments_coexist` | 同 DB 混合 v1 段（fixture stored V1）+ v2 段（当前 flush stored V2 仅 zstd-encode） | stored V1(旧) + V2(新, zstd-encode) 或 V1(新, 无 zstd-encode)；search 7 文档全可见 |
+| `migrates_v0_1_0_via_reindex` | 占位（`#[ignore]`） | 未来 v3+ 迁移器骨架 |
+
+## 4. fixture 生成方式（可复现）
+
+- **v0.1.0 tag 离线生成**：`git worktree add --detach /tmp/vane-v010 v0.1.0` → 在 worktree 写 `crates/vane-core/tests/gen_compat_fixture.rs`（用 v0.1.0 API）→ `cargo test -p vane-core --test gen_compat_fixture` → 产物在 `/tmp/v010-fixture/db/` → 拷贝至 `crates/vane-core/tests/fixtures/compat/v0.1.0/db/`。
+- **生成脚本镜像**：`scripts/gen_compat_fixture.rs`（非 workspace 编译目标，仅供文档/复现参考）。
+- **已知文档集**：5 篇中英混排文档（v010-d0..v010-d4），确定性输入，baseline 见 `cross_version_compat.rs` 注释。
+- **fixture 体积**：36KB（manifest 379B + 7 段文件各 54-709B + wal.log 73B），<100KB 约束满足。
+- **ULID**：`01KZRQ9VAJ0000000000000000`（v0.1.0 确定性产物，不随当前版本变化）。
+
+## 5. Phase 6 SPEC §6.2 修订建议
+
+- 补列 per-file format_version 冻结/演进矩阵（§1）。
+- 补列迁移策略（§2：双模读 + merge 自然迁移 + 迁移器占位）。
+- 补列格式冻结承诺（§3：v1 不可变 / v2 可演进 / manifest/wal/BM25 冻结）。
+- 补列 cross_version_compat.rs 测试覆盖（§4）。
diff --git a/docs/plans/m4/task-cross-version-report.md b/docs/plans/m4/task-cross-version-report.md
new file mode 100644
index 0000000..167e666
--- /dev/null
+++ b/docs/plans/m4/task-cross-version-report.md
@@ -0,0 +1,191 @@
+# M4 阶段三 a：跨版本持久化兼容 — 实现报告
+
+> 阶段：M4 阶段三 a（跨版本持久化兼容，§3.4）
+> 分支：feat/m4-prod-readiness
+> BASE：1fc03af（Phase 1 docs commit）
+> 实现者：SubAgent（sonnet）
+
+## 1. fixture 生成方式
+
+### 1.1 cross-tag build
+- `git worktree add --detach /tmp/vane-v010 v0.1.0`（detached HEAD at v0.1.0 tag）。
+- 在 v0.1.0 worktree 写 `crates/vane-core/tests/gen_compat_fixture.rs`（用 v0.1.0 API）。
+- `cd /tmp/vane-v010 && cargo test -p vane-core --test gen_compat_fixture` → 产物在 `/tmp/v010-fixture/db/`。
+- 拷贝至主工作树 `crates/vane-core/tests/fixtures/compat/v0.1.0/db/`。
+- `git worktree remove /tmp/vane-v010 --force`（清理，无残留）。
+
+### 1.2 已知文档集（确定性输入，非随机）
+5 篇中英混排文档，schema=`body=Text, v=Vector{dim:4, Cosine}, tag=Scalar{Keyword}`：
+
+| external_id | tag | text | vector |
+|---|---|---|---|
+| v010-d0 | a | "向量检索 混合搜索 hybrid search engine" | [1,0,0,0] |
+| v010-d1 | b | "BM25 ranking text retrieval" | [0,1,0,0] |
+| v010-d2 | a | "机器学习 与 搜索引擎 ranking" | [0,0,1,0] |
+| v010-d3 | c | "cosine similarity vector space" | [1,1,0,0] |
+| v010-d4 | b | "全文检索 inverted index 倒排" | [0,0,0,1] |
+
+### 1.3 fixture 格式版本（v0.1.0 实测）
+| 文件 | format_version | 说明 |
+|---|---|---|
+| header.bin | V1 | HEADER_FORMAT_V1 |
+| vectors.bin | V2 | VECTORS_FORMAT_V2（v0.1.0 始终写 v2，含 dim 头 12 字节） |
+| stored.bin | V1 | STORED_FORMAT_V1（无 zstd-encode，裸 JSON） |
+| idmap.bin | V1 | IDMAP_FORMAT_V1 |
+| scalars.col | V1 | SCALARS_FORMAT_V1 |
+| inverted.bin | 1 | FORMAT_VERSION=1 |
+| hnsw.bin | V1 | HNSW_FORMAT_V1 |
+
+ULID：`01KZRQ9VAJ0000000000000000`（v0.1.0 确定性产物）。
+体积：36KB（<100KB 约束满足）。
+
+### 1.4 v0.1.0 API 与当前差异
+`diff -rq v0.1.0/current` vane-core/src：
+- `segment/header.rs`：Phase 2 fix（`<8`→`<9` off-by-one → panic-on-corrupt），decode-only，不影响 fixture 格式。
+- `vector/mod.rs`/`vector/sq8.rs`：SIMD/scalar 路径注释 + SQ8 微调，不影响 fixture 格式。
+- `vfs/fault.rs`：新文件（FaultVfs，test-only，不进生产）。
+- `vfs/mod.rs`：FaultVfs mod 声明（test-only）。
+- `persistence/`/`wal/`：**完全一致**（diff 为空）。
+- `api/`（db.rs/collection.rs/types.rs）：**完全一致**。
+
+结论：v0.1.0 与当前版本的持久化格式（段文件格式）完全一致，fixture 是真实 v0.1.0 产物。
+
+## 2. 3 测试实现摘要
+
+### 2.1 `reads_v0_1_0_fixture`
+- `copy_fixture_to_temp()` → `StdFsVfs::with_root(temp)` → `Db::open(vfs, "db", ...)`。
+- 验 manifest restore（collection "docs" 可见）。
+- 验段文件 format_version 与 fixture 一致（6 文件 per-file version 常量断言：header V1 + vectors V2 + stored V1 + idmap V1 + scalars V1 + hnsw V1）。
+- vector search `[1,0,0,0]` top_k=10：5 文档全回填（external_id == 已知集）+ baseline 顺序（d0 score=1.0 > d3 score≈0.707 > d1=d2=d4 score=0.0 按 docid 升序）+ stored fields（tag）回填。
+- text search "检索"：命中 d0/d2/d4（非 vacuous）。
+- hybrid search "检索" + [1,0,0,0]：5 文档全可见（非 vacuous）。
+
+### 2.2 `v1_and_v2_segments_coexist`
+- 复制 fixture 到 temp → open → 验旧段 stored.bin V1。
+- add 2 新文档（v010-d5/d6）+ flush → 新段。
+- 验新段 stored.bin 版本：`#[cfg(feature="zstd-encode")]` V2（真 v1/v2 共存）；`#[cfg(not(...))]` V1（双段同格式共存）。
+- vector search [1,0,0,0] top_k=10：7 文档全可见（5 旧 + 2 新）+ d0 仍排第一 score=1.0（来自旧段）。
+- hybrid search "检索 compatibility" + [1,0,0,0]：旧段 d0（含"检索"）+ 新段 d5/d6（含"compatibility"）均命中。
+
+### 2.3 `migrates_v0_1_0_via_reindex`
+- `#[ignore]` 占位，骨架已建。
+- 验 v1 段可读（无需迁移）。
+- 注释：未来 v3+ 格式升级时实现迁移器（遍历旧段 → 读 v1/v2 → flush 新段 → manifest 切换 → WAL → 删旧段）。
+
+## 3. format-freeze note 摘要
+`docs/plans/m4/format-freeze-note.md` 供 Phase 6 SPEC §6.2 修订：
+- per-file format_version 冻结/演进矩阵（header/idmap/scalars/inverted/hnsw 冻结 V1 不可变；vectors/stored v1 冻结 v2 可演进）。
+- 迁移策略（双模读 + merge 自然迁移 + 迁移器占位；不做原地迁移）。
+- 格式冻结承诺（v1 不可变 / v2 可演进 / manifest/wal/BM25 冻结）。
+- cross_version_compat.rs 测试覆盖表。
+- fixture 生成方式（可复现）。
+
+## 4. 文件清单
+| 文件 | 类型 | 说明 |
+|---|---|---|
+| `crates/vane-core/tests/fixtures/compat/v0.1.0/db/manifest.json` | fixture | v0.1.0 manifest |
+| `crates/vane-core/tests/fixtures/compat/v0.1.0/db/segments/seg_01KZRQ9VAJ0000000000000000/header.bin` | fixture | HEADER_FORMAT_V1 |
+| `crates/vane-core/tests/fixtures/compat/v0.1.0/db/segments/seg_01KZRQ9VAJ0000000000000000/vectors.bin` | fixture | VECTORS_FORMAT_V2 |
+| `crates/vane-core/tests/fixtures/compat/v0.1.0/db/segments/seg_01KZRQ9VAJ0000000000000000/stored.bin` | fixture | STORED_FORMAT_V1 |
+| `crates/vane-core/tests/fixtures/compat/v0.1.0/db/segments/seg_01KZRQ9VAJ0000000000000000/idmap.bin` | fixture | IDMAP_FORMAT_V1 |
+| `crates/vane-core/tests/fixtures/compat/v0.1.0/db/segments/seg_01KZRQ9VAJ0000000000000000/scalars.col` | fixture | SCALARS_FORMAT_V1 |
+| `crates/vane-core/tests/fixtures/compat/v0.1.0/db/segments/seg_01KZRQ9VAJ0000000000000000/inverted.bin` | fixture | FORMAT_VERSION=1 |
+| `crates/vane-core/tests/fixtures/compat/v0.1.0/db/segments/seg_01KZRQ9VAJ0000000000000000/hnsw.bin` | fixture | HNSW_FORMAT_V1 |
+| `crates/vane-core/tests/fixtures/compat/v0.1.0/db/wal.log` | fixture | AddSegment 记录 |
+| `crates/vane-core/tests/fixtures/compat/README.md` | doc | fixture 来源 + 生成方式 + 已知文档集 + 格式版本 |
+| `crates/vane-core/tests/cross_version_compat.rs` | test | 3 测试（reads_v0_1_0_fixture + v1_and_v2_segments_coexist + migrates 占位） |
+| `scripts/gen_compat_fixture.rs` | script | fixture-gen 镜像（非 workspace 编译目标） |
+| `docs/plans/m4/format-freeze-note.md` | doc | 格式冻结承诺（供 Phase 6 SPEC §6.2 修订） |
+| `docs/plans/m4/task-cross-version-report.md` | doc | 本报告 |
+
+## 5. 各门禁真实输出
+
+### 5.1 `cargo fmt --all -- --check`
+```
+（无输出，rc=0）
+```
+
+### 5.2 `cargo clippy --all-targets --all-features --workspace --exclude vane-fuzz -- -D warnings`
+```
+Checking vane-core v0.2.0
+Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.90s
+（rc=0，0 warnings）
+```
+
+### 5.3 `cargo test -p vane-core --all-features --test cross_version_compat`
+```
+running 3 tests
+test migrates_v0_1_0_via_reindex ... ignored, 当前 v1/v2 双模读取覆盖兼容；未来格式升级（v3+）时实现迁移器
+test reads_v0_1_0_fixture ... ok
+test v1_and_v2_segments_coexist ... ok
+
+test result: ok. 2 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.27s
+```
+
+### 5.4 `cargo test --workspace --all-features --exclude vane-fuzz`
+```
+test result: ok. 322 passed; 0 failed; 1 ignored; ... (vane-core unit)
+test result: ok. 2 passed; 0 failed; 1 ignored; ... (cross_version_compat)
+（全集成测试 0 FAILED，含 proptest 3 不变量 + crash_recovery 5 场景 + corpus_compat + recall 等）
+```
+
+### 5.5 `cargo deny check`
+```
+advisories ok, bans ok, licenses ok, sources ok
+（1 pre-existing warning: regex/napi-derive-backend unmatched wrapper，非本任务引入）
+```
+
+### 5.6 `cargo check --target wasm32-unknown-unknown -p vane-core`
+```
+Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.08s
+（rc=0，fixture/tests 不进 wasm）
+```
+
+### 5.7 fixture + worktree 确认
+- fixture 文件：9 文件 in `crates/vane-core/tests/fixtures/compat/v0.1.0/db/`，36KB（<100KB）。
+- `git worktree list`：仅 `/Users/ximing/project/mygithub/vane 1fc03af [feat/m4-prod-readiness]`，无 v0.1.0 worktree 残留。
+- `/tmp/vane-v010` 不存在（已清理）。
+- `git status` 无 Cargo.lock 变化（fixture 是数据非依赖）。
+
+## 6. commit hash
+`c07600e`（`c07600e1146252e38f4f9f4a0af942bc3d6f49e5`，amend 后）
+- 分支：`feat/m4-prod-readiness`
+- 提交信息：`test(core): cross_version_compat v0.1.0 fixture + v1/v2 共存（M4 阶段三 a）`
+- 无 Co-Authored-By trailer
+- 无 push
+
+## 7. 自审
+
+### 7.1 v0.1.0 API 与当前差异
+- v0.1.0 与当前的 `api/`（db.rs/collection.rs/types.rs）、`persistence/`、`wal/` 完全一致（diff 为空）。
+- `segment/header.rs` 差异是 Phase 2 fix（decode-only `<8`→`<9`），不影响 fixture 格式（fixture header 91 字节 ≥9 字节门限）。
+- `segment/mod.rs` 完全一致（diff 为空）→ v0.1.0 始终写 vectors.bin V2，与当前一致。
+- 结论：v0.1.0 fixture 是真实 v0.1.0 产物，格式与当前版本一致，当前版本双模读取通过。
+
+### 7.2 fixture 真实性确认
+- fixture 由 v0.1.0 tag 的 vane-core API 真实生成（非当前代码模拟）。
+- v0.1.0 worktree 编译成功（`cargo build -p vane-core --tests` rc=0）。
+- fixture-gen test 在 v0.1.0 运行通过（`gen_v0_1_0_fixture ... ok`）。
+- 段文件 format_version 经 xxd 确认（header V1 / vectors V2 / stored V1 / 其余 V1）。
+- ULID `01KZRQ9VAJ0000000000000000` 是 v0.1.0 确定性产物。
+
+### 7.3 worktree 清理确认
+- `git worktree remove /tmp/vane-v010 --force` 成功。
+- `git worktree list` 仅显示主工作树。
+- `/tmp/vane-v010` 目录不存在。
+- `/tmp/v010-fixture` 临时产物已删除。
+- 主工作树 git 状态无 v0.1.0 worktree 残留污染。
+
+### 7.4 测试非 vacuous 确认
+- `reads_v0_1_0_fixture`：6 per-file format_version 常量断言 + 5 external_id 集合断言 + 3 模式 search 命中断言 + baseline 顺序断言（d0/d3 位置 + score 精确值）→ 全 non-vacuous。
+- `v1_and_v2_segments_coexist`：旧段 V1 + 新段 V2（zstd-encode 时）格式版本断言 + 7 文档全可见 + 旧段 d0 仍排第一 → non-vacuous。
+- `migrates_v0_1_0_via_reindex`：`#[ignore]` 占位，骨架验证 v1 段可读（5 文档）。
+
+### 7.5 不改 M0-M3 冻结 pub API
+- 无 Rust 源码修改（仅新增 fixture + 测试 + 脚本 + 文档）。
+- 无 Cargo.toml 修改（无新依赖）。
+- 无 SPEC.md / CI yml / fault.rs / crash_recovery.rs / vane-fuzz / proptest 修改。
+
+### 7.6 concerns
+1. **v0.1.0 vectors.bin 实际为 V2（非 V1）**：设计文档 §3.4 称 "v1 fixture（v0.1.0 产物）" 措辞不精确——v0.1.0 的 `segment/mod.rs` 始终写 vectors.bin V2（与当前一致），不存在已发布的 V1 vectors.bin 产物。VECTORS_FORMAT_V1 读路径保留但无已发布 fixture 覆盖（仅 types.rs 单测常量断言）。本任务的 "v1/v2 共存" 在 stored.bin 维度（fixture V1 + 新 flush V2 仅 zstd-encode 时）。这是格式事实，非缺陷——fixture 真实反映 v0.1.0 产物，测试覆盖当前版本双模读取。
+2. **`v1_and_v2_segments_coexist` 无 zstd-encode 时双段同格式**：不启 zstd-encode 时新段 stored.bin 也是 V1，"v1/v2 共存" 退化为 "双段共存"。CI `--all-features` 含 zstd-encode，真 v1/v2 共存被测；非 zstd-encode 配置下测双段共存（仍 non-vacuous）。这是 feature 门控的预期行为，非缺陷。
diff --git a/scripts/gen_compat_fixture.rs b/scripts/gen_compat_fixture.rs
new file mode 100644
index 0000000..74b8254
--- /dev/null
+++ b/scripts/gen_compat_fixture.rs
@@ -0,0 +1,124 @@
+// scripts/gen_compat_fixture.rs — v0.1.0 fixture generator（M4 §3.4 跨版本兼容）
+//
+// 此文件**不是 workspace 编译目标**——放在 scripts/ 目录，不被 cargo build 识别。
+// 它是 v0.1.0 tag `crates/vane-core/tests/gen_compat_fixture.rs` 的镜像副本，
+// 仅供文档/复现参考。在 v0.1.0 tag 离线 worktree 运行，产物拷贝至主工作树
+// `crates/vane-core/tests/fixtures/compat/v0.1.0/` 后入仓。
+//
+// 流程：StdFsVfs rooted at /tmp/v010-fixture/ → Db::open → 声明 collection →
+// 灌入确定性已知文档集（5 docs，vector+text+stored meta）→ flush → close。
+//
+// 产物段文件格式（v0.1.0 per-file format_version）：
+//   header.bin   HEADER_FORMAT_V1
+//   vectors.bin  VECTORS_FORMAT_V2（v0.1.0 始终写 v2，含 dim 头）
+//   stored.bin   STORED_FORMAT_V1（无 zstd-encode feature，裸 JSON）
+//   idmap.bin     IDMAP_FORMAT_V1
+//   scalars.col   SCALARS_FORMAT_V1
+//   inverted.bin  FORMAT_VERSION=1
+//
+// 已知文档集 baseline（cross_version_compat.rs 断言用）：
+//   external_id ∈ {v010-d0, v010-d1, v010-d2, v010-d3, v010-d4}
+//   d0/d2 tag=a, d1/d4 tag=b, d3 tag=c
+//   d0 vec=[1,0,0,0], d1=[0,1,0,0], d2=[0,0,1,0], d3=[1,1,0,0], d4=[0,0,0,1]
+
+use std::sync::Arc;
+
+use vane_core::api::{CollectionOptions, Db, Doc, OpenOptions};
+use vane_core::types::{FieldDef, Metric, ScalarKind, Schema};
+use vane_core::vfs::std_fs::StdFsVfs;
+use vane_core::vfs::Vfs;
+
+fn build_schema() -> Schema {
+    Schema::new(vec![
+        ("body".into(), FieldDef::Text),
+        (
+            "v".into(),
+            FieldDef::Vector {
+                dim: 4,
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
+fn known_docs() -> Vec<Doc> {
+    let mk_meta = |tag: &str| {
+        let mut m = std::collections::HashMap::new();
+        m.insert(
+            "tag".to_string(),
+            vane_core::api::ScalarValue::Keyword(tag.into()),
+        );
+        m
+    };
+    vec![
+        Doc {
+            id: "v010-d0".into(),
+            text: Some("向量检索 混合搜索 hybrid search engine".into()),
+            vector: Some(vec![1.0, 0.0, 0.0, 0.0]),
+            meta: Some(mk_meta("a")),
+        },
+        Doc {
+            id: "v010-d1".into(),
+            text: Some("BM25 ranking text retrieval".into()),
+            vector: Some(vec![0.0, 1.0, 0.0, 0.0]),
+            meta: Some(mk_meta("b")),
+        },
+        Doc {
+            id: "v010-d2".into(),
+            text: Some("机器学习 与 搜索引擎 ranking".into()),
+            vector: Some(vec![0.0, 0.0, 1.0, 0.0]),
+            meta: Some(mk_meta("a")),
+        },
+        Doc {
+            id: "v010-d3".into(),
+            text: Some("cosine similarity vector space".into()),
+            vector: Some(vec![1.0, 1.0, 0.0, 0.0]),
+            meta: Some(mk_meta("c")),
+        },
+        Doc {
+            id: "v010-d4".into(),
+            text: Some("全文检索 inverted index 倒排".into()),
+            vector: Some(vec![0.0, 0.0, 0.0, 1.0]),
+            meta: Some(mk_meta("b")),
+        },
+    ]
+}
+
+#[test]
+fn gen_v0_1_0_fixture() {
+    let root = "/tmp/v010-fixture";
+    let _ = std::fs::remove_dir_all(root);
+    std::fs::create_dir_all(root).unwrap();
+    let vfs = Arc::new(StdFsVfs::with_root(root)) as Arc<dyn Vfs>;
+
+    let db = Db::open(vfs.clone(), "db", OpenOptions::default()).unwrap();
+    let col = db
+        .collection("docs", build_schema(), CollectionOptions::default())
+        .unwrap();
+    let report = col.add(&known_docs()).unwrap();
+    assert_eq!(report.accepted, 5);
+    col.flush().unwrap();
+    db.close().unwrap();
+
+    // 验证段文件存在
+    let segs = vfs.list("db/segments").unwrap();
+    assert!(!segs.is_empty(), "应至少有一个段目录");
+    let seg_dir = segs
+        .iter()
+        .find(|s| s.starts_with("seg_"))
+        .expect("应存在 seg_<ulid> 目录")
+        .clone();
+    let seg_path = format!("db/segments/{}", seg_dir);
+    for fname in ["header.bin", "vectors.bin", "stored.bin", "idmap.bin", "scalars.col", "inverted.bin"] {
+        let path = format!("{}/{}", seg_path, fname);
+        let mut tmp = [0u8; 1];
+        let _ = vfs.read_at(&path, &mut tmp, 0).expect(&format!("{} 可读", fname));
+    }
+}
