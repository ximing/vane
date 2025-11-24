# M2-14 Demo 人工验收清单

> Playwright 在本环境不可用（`npx playwright` 安装失败）。以下验收项经 **csi 真实 Chrome e2e**（见 `demo/e2e/run-smoke.mjs` + 浏览器 evaluate 脚本）或人工清单验证。csi e2e 已覆盖 9 项中的 7 项自动化；剩余 2 项（拖入文件夹、1000 文件规模）人工执行。

## 自动化验证（csi 真实 Chrome，已通过）

| # | 验收项 | 验证方式 | 结果 |
|---|--------|----------|------|
| 2 | 建库 | csi e2e：`VaneWorker.create(vfs:opfs)` + `open` + `collection(jieba)` | ✅ created=true, col=1 |
| 3 | 中文搜索 | csi e2e：`search({text:"人工智能",mode:"hybrid"})` | ✅ top1="ai"（jieba 分词命中） |
| 4 | 混合搜索 | csi e2e：`search({text:"学习",vector:[...],mode:"hybrid"})` | ✅ hybridHits=8（RRF 融合） |
| 5 | 持久化 | csi e2e：worker1 写入+close → worker2 open+search | ✅ persistTop1="p1"（OPFS 跨会话保留） |
| 6 | SIMD 探针 | csi e2e：`WebAssembly.validate(simd128_test_module)` | ✅ simd=true（Chrome 支持 simd128 → 加载 simd 产物） |
| 7 | 词典加载 | csi e2e：`create(dictUrl:"https://cdn.jsdelivr.net/gh/ximing/vane@main/crates/vane-dict-zh/data/dict.bin")` + sha256 校验 | ✅ created=true（jieba 词典加载成功，CDN fetch + sha256 校验通过；private 仓库期间 jsdelivr 不生效则降级 bigram） |
| 8 | 词典降级 | csi e2e：`create(无 dictUrl)` + `collection(jieba)` + 中文搜索 | ✅ degradeTop1="d1"（降级 bigram，中文搜索仍可用） |
| 9 | export 快照 | csi e2e：`export("backup.vane")` | ✅ exported=true（快照写入 OPFS） |

## 人工验收（2 项）

### 1. 拖入文件夹

**前置**：`bash demo/build.sh` + `python3 -m http.server 8765` + Chrome 打开 `http://localhost:8765/index.html`。

**步骤**：
1. 点击"加载示例"按钮 → 验证日志显示 `[index] 完成：accepted=5`。
2. 或：从文件管理器拖入 `demo/samples/` 文件夹到拖入区 → 验证日志显示 `[index] 完成：accepted=N`。
3. 搜索框输入"人工智能" → 验证结果列表显示 `samples/01-ai-intro.md` 排在前面。
4. 搜索框输入"向量检索" → 验证结果列表显示 `samples/02-vector-search.md` 排在前面。

**预期**：拖入 → 自动解析 .md → 索引 → 搜索结果实时显示。

### 2. 规模（1000 markdown 文件 <500ms）

**前置**：准备含 1000+ markdown 文件的文件夹（可用 `for i in $(seq 1 1000); do echo "# doc $i 人工智能向量检索"; done > docs/$i.md` 生成）。

**步骤**：
1. 拖入 1000 文件文件夹 → 验证索引完成。
2. 搜索框输入查询 → 观察结果列表上方的耗时（`top N（Xms）`）。
3. 验证 X < 500（SPEC §13.1 WASM 放宽 3~5 倍）。

**预期**：搜索 <500ms。

## node smoke（API 路径）

`node demo/e2e/run-smoke.mjs` 验证 wasm 产物可加载 + VaneWorker API 路径通（memory vfs + jieba + dictData + sha256 + 中文搜索 + 混合搜索 + export）。12/12 全绿。
