# vane 文档站（website/）

## 首页 demo 数据（src/data/demo-results.json）

由真实 `vane-node` 本地构建生成（`provenance: 'vane-node'`），不是手写数据。

**重新生成**：

```bash
cd crates/vane-node && npm install && npx napi build --platform   # 构建本地 native 模块（只需一次）
node website/scripts/gen-demo-data.mjs                             # 重新生成 JSON（含 shape 自校验）
node website/scripts/gen-demo-data.mjs --check                     # 只校验现有 JSON 的 shape
```

**何时需要重跑**：

- 修改了 `scripts/gen-demo-data.mjs` 中的文档语料、预置 query 或伪向量主题轴；
- vane-core 检索/排序行为升级（BM25 参数、RRF 常数、分词器、词典版本变化）导致排序结果漂移；
- `DemoData` 契约（`src/components/contracts.ts`）变更——此时需同步改生成器与校验逻辑。

**降级说明**：若本地 napi 构建不可用，契约允许手写 JSON（`provenance: 'manual'`），页面会据此渲染"示例数据"标注。当前数据为主路径产物，无偏离。
