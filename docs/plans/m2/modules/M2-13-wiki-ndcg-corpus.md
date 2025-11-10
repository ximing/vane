# M2-13 真实维基 nDCG corpus

## 1. 目标
接入真实中文维基 500 篇 + 50 查询 fixture，落实 SPEC §13.2-2 验收②（jieba-lite 相对完整版 nDCG@10 差 <2%，相对 bigram 提升 ≥15%）。M1 用代表性边界歧义语料（+84% 达标），M2 切换到真实维基 corpus 作主验收，M1 语料保留为回归对照（scoping §2.2 方案，M2-00 已备方案）。

SPEC 节号：§13.2-2（中文维基 500 篇 + 50 查询，jieba-lite vs 完整版 <2%，vs bigram ≥15%）。

## 2. 涉及文件
- **Create** `crates/vane-core/tests/fixtures/wiki_zh/corpus.json`：500 篇 `{id, text}`（200~2000 字，科技/历史/地理多领域）。
- **Create** `crates/vane-core/tests/fixtures/wiki_zh/queries.json`：50 查询 `{qid, text}`（2~4 字，实体名/概念词/边界歧义词）。
- **Create** `crates/vane-core/tests/fixtures/wiki_zh/qrels.json`：`{qid: {docid: rel}}`（top-10 人工/半自动标注 relevance）。
- **Create** `crates/vane-core/tests/ndcg_wiki_zh.rs`：加载 fixture → jieba collection → 50 查询 → nDCG@10 + 对比基线（bigram + jieba-rs 完整版 dev-dep）。
- **Modify** `.github/workflows/ci.yml`（jieba-nDCG job，line 168 区间）：切换到维基 fixture（M1 边界歧义语料保留为回归对照 job）。
- **离线脚本** `scripts/gen_wiki_fixture.rs`（或 Python）：从 `zhwiki-latest-pages-articles.xml.bz2` 抽取 500 篇 + 构造 50 查询 + qrels（开发机离线，不进 CI 运行时）。

## 3. 接口契约
### Consumes from
- M1 `vane_core::tokenizer::jieba::{JiebaDict, JiebaTokenizer}`（`tokenizer/jieba/dict.rs:46` `JiebaDict::load`、`tokenizer/jieba/mod.rs:41` `JiebaTokenizer::new`）。
- M1 `vane_core::api::Db`/`Collection`（建 jieba collection + search）。
- M1 nDCG 计算工具（M1 既有 `tests/ndcg_*.rs` 方法论，复用 nDCG@10 函数）。
- jieba-rs 完整版（dev-dep，feature gated，对比基线）。

### Produces for
- CI jieba-nDCG job 切换到维基 fixture（SPEC §13.2-2 主验收）。
- M1 边界歧义语料保留为回归对照（验证 jieba 切分质量不退步）。
- fixture 提交仓库（~1.5MB，500 篇 × 平均 3KB 中文 UTF-8）。

## 4. TDD 测试清单
1. **fixture 完整性**：`corpus.json` 含 500 篇，每篇 `id` 唯一 + `text` 200~2000 字；`queries.json` 含 50 查询；`qrels.json` 每查询 top-10 relevance 标注。
2. **nDCG@10 计算**：加载 fixture → jieba collection → 50 查询 search → 算 nDCG@10（用 qrels）。
3. **jieba vs bigram**：jieba collection nDCG@10 - bigram collection nDCG@10 ≥ 0.15（提升 ≥15%，SPEC §13.2-2）。
4. **jieba vs jieba-rs 完整版**：jieba-lite nDCG@10 - jieba-rs 完整版 nDCG@10 差 <0.02（<2%，SPEC §13.2-2）。
5. **M1 边界歧义语料回归**：M1 既有边界歧义语料 nDCG 测试保留通过（jieba 切分质量不退步）。
6. **CI 集成**：`jieba-nDCG` job 跑维基 fixture，失败阻断 PR。
7. **fixture 体积**：`tests/fixtures/wiki_zh/` 总体积 ≤1.5MB（reviewer B-M6 修正：中文 UTF-8 每 3 字节/字，500 篇 × 平均 3KB ≈ 1.5MB，原估 ~500KB 偏乐观）。
8. **领域覆盖**（reviewer B-M7 自动化）：fixture 含科技/历史/地理多领域——自动断言 fixture 关键词分布（科技/历史/地理三领域关键词各 ≥30 篇命中，非人工抽检）。
9. **边界歧义查询覆盖**：50 查询含 ≥10 边界歧义词（如"人工智能""机器学习""区块链"——可被 bigram 错切）。

## 5. 验收标准
- jieba vs bigram nDCG@10 提升 ≥15%（SPEC §13.2-2 硬门禁）。
- jieba vs jieba-rs 完整版差 <2%（SPEC §13.2-2 硬门禁）。
- CI jieba-nDCG job 跑维基 fixture 通过。
- M1 边界歧义语料回归测试保留通过。
- fixture 提交仓库，体积 ≤1.5MB。
- fixture 离线生成脚本可重复执行（开发机，非 CI）。

## 6. 前置依赖
- M2-00 corpus 方案（已备）。
- M1 jieba-lite（既有）。

## 7. 不变量覆盖
- **§13.2-2 中文分词验收②**：本模块直接落实。测试 3+4 守护。
- **§13.3 工程纪律**：fixture 提交仓库，CI 稳定运行。测试 6+7 守护。
- **I-8 binding 薄壳**：nDCG 测试在 core，无 binding 依赖。
- **无新依赖**：jieba-rs 完整版作 dev-dep（M1 已有，对比基线），不进运行时。
