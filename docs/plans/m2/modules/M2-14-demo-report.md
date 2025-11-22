# M2-14 Demo（纯前端 markdown 搜索）— 报告

## 1. 概述

M2 收尾 capstone：纯前端 markdown 搜索 demo，展示 Vane 浏览器交付闭环（SPEC §15 M2 Demo）。拖入 markdown 文件夹 → 本地混合检索（jieba 中文 + OPFS 持久化 + SIMD 双产物），无后端。

## 2. demo 结构

```
demo/
  index.html           # 拖入区 + 搜索框 + 结果列表 + 导出/加载示例按钮 + 日志区
  main.js              # 主页面 JS（Worker 调用 + 拖入处理 + UI + 占位 hashVector）
  worker.js            # Worker 入口（SIMD 探针 + 动态选 wasm + postMessage 路由）
  build.sh             # 构建 wasm 双产物 + JS 胶水 + dict.bin（wasm-bindgen --target web）
  README.md            # 使用说明 + 截图 + 结构
  MANUAL-CHECKLIST.md  # 9 验收项清单（7 自动化 + 2 人工）
  screenshot.png       # demo 截图
  e2e/
    run-smoke.mjs      # node smoke 测试（12/12 全绿）
  samples/             # 示例 markdown（5 篇中文）
  pkg/                 # 构建产物（gitignore）
  pkg-node/            # nodejs 构建产物（gitignore，smoke 用）
```

## 3. 前置 bug 修复（binding 层 vane-wasm，非 core）

实现 demo 时发现并修复 M2-04 两个 binding 层 bug（阻断 jieba 中文搜索路径）：

### Bug 1：worker.rs 用 `JiebaDict::load`（应 `load_zstd`）
- `dict.bin` 是 zstd 压缩（SPEC §5.2），`JiebaDict::load` 解析未压缩格式，`load_zstd` 才解压。
- vane-ffi/vane-node 均用 `load_zstd`，worker.rs 误用 `load` → 词典加载恒失败 → 永远降级 bigram。
- **修复**：`crates/vane-wasm/src/worker.rs` `JiebaDict::load` → `JiebaDict::load_zstd`。

### Bug 2：dict_loader sha256 校验语义不匹配
- `verify_sha256_prefix(bytes, expected)` 计算压缩字节完整 sha256；但 `gen_dict` 产出的 `sha256_prefix.bin` 是解压后 payload `[16..]` 的 sha256 前 8 字节。
- 两端语义不一致 → 传 `dictSha256` 时校验恒失败 → 降级 bigram。
- **修复**：`verify_sha256_prefix` 改为 `JiebaDict::load_zstd(bytes)` 解压后取 `dict.sha256_prefix()` 比对（三渠道一致：Node/Go/WASM 各端解压 dict.bin → 同一 sha256_prefix）。
- 测试更新：用 `vane-dict-zh` dev-dependency 提供真实 `dict.bin` 验证（dev-dep 不进 wasm 产物，红线安全）。

## 4. e2e / smoke 结果

### node smoke（`demo/e2e/run-smoke.mjs`，12/12 全绿）

用 wasm-bindgen `--target nodejs` 产出验证：
- wasm 产物可加载 + `vane_version()` 非空
- sync API 路径（`vane_open` → `vane_collection` → `vane_add` → `vane_flush` → `vane_search`，cjk_bigram）
- VaneWorker 路径（`create(memory+dictData)` → `open` → `collection(jieba)` → `add` → `flush` → `search` → `export` → `close`）
- jieba 词典加载（dictData + sha256 校验通过）+ 中文搜索命中
- 混合搜索（text + vector）+ export 快照

### 浏览器 e2e（csi 真实 Chrome，9 验收项 7 项自动化）

通过 csi 驱动真实 Chrome 在 `http://localhost:8765` 验证：

| # | 验收项 | 结果 |
|---|--------|------|
| 2 | 建库（OPFS + jieba） | ✅ created=true, col=1 |
| 3 | 中文搜索"人工智能" | ✅ top1="ai"（jieba 分词命中） |
| 4 | 混合搜索（text+vector, RRF） | ✅ hybridHits=8 |
| 5 | OPFS 持久化（跨 worker 会话） | ✅ worker1 写入 → worker2 搜索命中 p1 |
| 6 | SIMD 探针 | ✅ simd=true（Chrome 支持 simd128 → 加载 simd 产物） |
| 7 | 词典加载（dictUrl fetch + sha256） | ✅ created=true（jieba 词典加载成功） |
| 8 | 词典降级（无 dictUrl → bigram） | ✅ degradeTop1="d1"（中文搜索仍可用） |
| 9 | export 快照 | ✅ exported=true（backup.vane 写入 OPFS） |

### 人工验收（2 项，MANUAL-CHECKLIST.md）

- **拖入文件夹**：拖入 `demo/samples/` → 解析 → 索引 → 搜索（walkEntry 递归 webkitGetAsEntry 实现，node smoke 验证 add 路径）。
- **规模**：1000 markdown 文件搜索 <500ms（SPEC §13.1 WASM 放宽档）。

## 5. 自证门禁结果

| # | 门禁 | 结果 |
|---|------|------|
| 1 | `cargo test --workspace --all-features` | ✅ 498 passed, 0 failed（基线 496 → 498，新增 dict_loader 真实数据测试 +2） |
| 2 | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | ✅ clean |
| 3 | `cargo fmt --all -- --check` | ✅ clean |
| 4 | `bash scripts/build-wasm-variants.sh` 双产物 | ✅ simd gzip 409KB / scalar 412KB（<800KB），simd 指令 3118 条，scalar 0 条 |
| 5 | demo 静态可加载 | ✅ 8 文件 HTTP 200（index/main/worker/wasm×2/dict/sha） |
| 6 | e2e / node smoke | ✅ node smoke 12/12 + 浏览器 csi e2e 7/9 自动化 |
| 7 | README | ✅ 完整（构建 + 启动 + 操作 + 结构 + 占位向量说明） |
| 8 | MANUAL-CHECKLIST | ✅ 9 验收项（7 自动化 + 2 人工） |

## 6. 体积门禁

- `demo/pkg/vane_wasm_simd.wasm` gzip ~312KB（wasm-bindgen + wasm-opt -Oz 后）
- `demo/pkg/vane_wasm_scalar.wasm` gzip ~314KB
- 远低于 SPEC §13.2-3 的 800KB 门禁。

## 7. 遗留

- **Playwright CI 集成**：本环境 Playwright 不可用（`npx playwright` 安装失败），浏览器 e2e 经 csi 真实 Chrome 驱动验证（非 CI 可重复）。后续可加 Playwright CI job（`demo/e2e/playwright.spec.ts`）将 7 项自动化 e2e 固化为 CI 门禁。
- **M2-04 两个 binding bug 已修复**（worker.rs `load`→`load_zstd` + dict_loader sha256 语义），但 M2-04 报告未记录这两个 bug。本次修复属 demo 前置依赖，core 零改动。
- **export 快照下载**：`VaneWorker.export(dest)` 写入 OPFS 容器内逻辑路径（非独立 OPFS 文件），用户无法直接下载。demo 当前仅 console/UI 提示"快照已写入 OPFS"。如需下载到本地，需扩展 worker.js 加 "readFile" op（从 VFS 读快照字节 → Blob 下载），属后续增强。
- **占位向量**：demo 用 hash 向量（char bucket → 64 维 L2 归一化），非真实 embedding。README 已明示生产应替换为 transformers.js/OpenAI 等。
