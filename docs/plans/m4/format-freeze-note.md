# 格式冻结承诺（供 Phase 6 SPEC §6.2 修订）

> 来源：M4 阶段三 a（跨版本兼容 fixture）产出。
> 供 Phase 6 SPEC §6.2 修订参考——列 per-file format_version 哪些冻结（v1 不可变）、
> 哪些可演进（v2 zstd 等）、迁移策略。
> 实证基础：v0.1.0 tag 真实 fixture + 当前版本双模读取测试（`tests/cross_version_compat.rs`）。

## 1. per-file format_version 冻结/演进矩阵

| 段文件 | 常量 | 当前版本 | 冻结状态 | 演进策略 |
|---|---|---|---|---|
| `header.bin` | `HEADER_FORMAT_V1` | 1 | **冻结**（v1 不可变） | header 是段元数据入口（magic+version+tokenizer_id+docid_range+tombstone）。格式变更破坏所有段读取，故冻结。如需演进须 bump version + 双模读 + 迁移器。 |
| `vectors.bin` | `VECTORS_FORMAT_V1` / `VECTORS_FORMAT_V2` | 1(读)/2(写) | **v1 冻结，v2 可演进** | v1=8 字节头（magic+version，无 dim，懒加载从 payload 反推）。v2=12 字节头（含 dim，M2-07 open 期预存 dim）。当前版本始终写 v2，v1 读取保留（双模）。v0.1.0 fixture 实测：v0.1.0 始终写 v2。v1 读路径覆盖预发布历史数据（无已发布 v1 vectors.bin 产物，但读路径保留）。 |
| `stored.bin` | `STORED_FORMAT_V1` / `STORED_FORMAT_V2` | 1(读)/1或2(写) | **v1 冻结，v2 可演进** | v1=裸 JSON（magic+version+count+entries）。v2=zstd 块压缩（magic+version+raw_len+zstd_len+zstd_block，M2-08）。zstd-encode feature 启用时写 v2，否则写 v1。v0.1.0 fixture 实测：无 zstd-encode → 写 v1。双模读取（decode_stored 按 version 分支），旧 v1 段只读服务至段合并自然清除（不做原地迁移）。 |
| `idmap.bin` | `IDMAP_FORMAT_V1` | 1 | **冻结**（v1 不可变） | external_id → docid 映射。格式简单（magic+version+count+entries），无演进需求。 |
| `scalars.col` | `SCALARS_FORMAT_V1` | 1 | **冻结**（v1 不可变） | 标量列存（magic+version+count+columns）。格式简单，无演进需求。 |
| `inverted.bin` | `FORMAT_VERSION=1` | 1 | **冻结**（v1 不可变） | 倒排索引（magic+version+count+postings）。BM25 参数 k1=1.2/b=0.75 冻结（§6.2）。格式变更破坏 BM25 排序，故冻结。 |
| `hnsw.bin` | `HNSW_FORMAT_V1` | 1 | **冻结**（v1 不可变） | HNSW 图结构（magic+version+dim+graph）。格式变更破坏 HNSW 导航，须 bump + 迁移。fallback brute 路径在 hnsw 缺失/损坏时降级。 |
| `manifest.json` | `version=1` | 1 | **冻结**（schema 版本） | manifest schema（version+collections）。collection schema 变更须 reindex（新分词身份），非格式 version。 |
| `wal.log` | N/A（行 JSON） | N/A | **冻结**（WAL 记录格式） | WAL 是 JSON 行（AddSegment/DeleteSegment/AddTombstone）。记录格式变更须双模读 + recover 兼容。 |

## 2. 迁移策略

### 2.1 当前状态（v0.1.0 → v0.2.0）

- **无需迁移**：v0.1.0 与当前版本（v0.2.0）的段文件格式**完全一致**（segment/mod.rs diff 为空，persistence/wal diff 为空）。
- **双模读取**：v1/v2 format_version 双模读取已实现（M2-08），覆盖 stored.bin v1(裸JSON)/v2(zstd) + vectors.bin v1(8字节头)/v2(12字节头含dim)。
- **v0.1.0 fixture 实测**：v0.1.0 产物为 header.bin V1 + vectors.bin V2 + stored.bin V1 + idmap.bin V1 + scalars.col V1 + inverted.bin V1 + hnsw.bin V1。当前版本读此 fixture 数据一致（`reads_v0_1_0_fixture` 测试通过）。

### 2.2 未来格式升级策略（v3+）

当未来版本需演进某个 per-file format_version（如 stored.bin v3 新压缩算法）时：

1. **bump version**：`STORED_FORMAT_V3 = 3`。
2. **双模读取**：`decode_stored` 加 v3 分支（v1/v2/v3 三模读），旧段只读服务至段合并自然清除。
3. **不做原地迁移**：旧段不可变（SPEC §6.2 铁律），迁移通过 merge 自然完成（merge 写新格式段，manifest 切换后旧段删除）。
4. **迁移器占位**：`migrates_v0_1_0_via_reindex` 测试标 `#[ignore]`，骨架已建。未来 v3+ 启用时，实现迁移器调用（遍历旧段 → 读 v1/v2 → flush 新段 → manifest 切换 → WAL）。
5. **corpus 兼容测试**：新格式须通过 `tests/corpus_compat.rs`（v2 roundtrip）+ `tests/cross_version_compat.rs`（v0.1.0 fixture 读取）。

### 2.3 格式冻结承诺

- **v1 不可变**：所有 `*_FORMAT_V1` 常量对应的格式**永不变更**（v1 读路径冻结）。任何 v1 文件（header/v1-vectors/v1-stored/idmap/scalars/inverted/hnsw）当前版本及未来版本必须能读。
- **v2 可演进**：`VECTORS_FORMAT_V2` / `STORED_FORMAT_V2` 可演进为 v3，但 v2 读路径冻结（v2 文件必须能读）。演进通过 bump version + 双模读 + merge 自然迁移。
- **manifest/wal 冻结**：manifest schema version=1 冻结（collection schema 变更经 reindex）；WAL 记录格式冻结（recover 双模读）。
- **BM25 参数冻结**：k1=1.2/b=0.75 冻结（进 format_version 语义），变更须 bump inverted.bin version + 迁移。

## 3. cross_version_compat.rs 测试覆盖

| 测试 | 覆盖内容 | 格式断言 |
|---|---|---|
| `reads_v0_1_0_fixture` | 当前版本读 v0.1.0 fixture（真实 tag 产物） | header V1 + vectors V2 + stored V1 + idmap V1 + scalars V1 + hnsw V1 + external_id 全回填 + search baseline 一致 |
| `v1_and_v2_segments_coexist` | 同 DB 混合 v1 段（fixture stored V1）+ v2 段（当前 flush stored V2 仅 zstd-encode） | stored V1(旧) + V2(新, zstd-encode) 或 V1(新, 无 zstd-encode)；search 7 文档全可见 |
| `migrates_v0_1_0_via_reindex` | 占位（`#[ignore]`） | 未来 v3+ 迁移器骨架 |

## 4. fixture 生成方式（可复现）

- **v0.1.0 tag 离线生成**：`git worktree add --detach /tmp/vane-v010 v0.1.0` → 在 worktree 写 `crates/vane-core/tests/gen_compat_fixture.rs`（用 v0.1.0 API）→ `cargo test -p vane-core --test gen_compat_fixture` → 产物在 `/tmp/v010-fixture/db/` → 拷贝至 `crates/vane-core/tests/fixtures/compat/v0.1.0/db/`。
- **生成脚本镜像**：`scripts/gen_compat_fixture.rs`（非 workspace 编译目标，仅供文档/复现参考）。
- **已知文档集**：5 篇中英混排文档（v010-d0..v010-d4），确定性输入，baseline 见 `cross_version_compat.rs` 注释。
- **fixture 体积**：36KB（manifest 379B + 7 段文件各 54-709B + wal.log 73B），<100KB 约束满足。
- **ULID**：`01KZRQ9VAJ0000000000000000`（v0.1.0 确定性产物，不随当前版本变化）。

## 5. Phase 6 SPEC §6.2 修订建议

- 补列 per-file format_version 冻结/演进矩阵（§1）。
- 补列迁移策略（§2：双模读 + merge 自然迁移 + 迁移器占位）。
- 补列格式冻结承诺（§3：v1 不可变 / v2 可演进 / manifest/wal/BM25 冻结）。
- 补列 cross_version_compat.rs 测试覆盖（§4）。
