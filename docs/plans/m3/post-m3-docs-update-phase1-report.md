# M3 文档站全站更新 Phase 1 报告

## 状态

已完成。新建 `website/src/pages/api/Web.tsx`（@vane-rs/web API + 类型签名专题页），routes.ts + nav.ts 注册到位，验证全绿。

## Commits

- `89f9a9c` docs(website): 新建 api/web.tsx——@vane-rs/web API + 类型签名专题页

分支：`feat/docs-web-update`（从 main 切出）。

## 测试摘要

- `cd website && npx tsc --noEmit` → exit 0（无错误）
- `cd website && npm run build` → 成功（177 modules，16 routes 写入 sitemap.xml）
- sitemap 含 `https://ximing.github.io/vane/api/web`
- grep 确认 routes.ts（import + route entry）+ nav.ts（API Reference section）注册
- git diff 仅改 `website/src/pages/api/Web.tsx`（新建）+ `website/src/routes.ts` + `website/src/nav.ts`

## Concerns

1. **未新建 Web.css**：复用 `api.css` 的 `.api-page` / `.api-lead` / `.api-table-wrap` / `.api-table` 约定，与 Overview/Collection/Search 等 API 页保持一致。无需额外样式。
2. **类型签名来源**：全部从 `bindings/web/src/types.ts` 直接提取，包括 VaneWorkerOpts / Schema / FieldSchema 判别联合 / Doc / SearchQuery / Hit / OpenOptions / CollectionOptions / Vane 接口 / 全部辅助类型别名（VfsKind / VectorMetric / TokenizerKind / SearchMode / FusionSpec / AutoCommit / PersistenceMode / UserDictEntry）。未臆造、未简化到失真。
3. **Callout 使用**：3 处——`#vane-interface` 用 warning（collection 返 Promise<number> 句柄语义）、`#searchquery` 用 gap（filter wasm 端不支持）、`#collectionoptions` 用 note（无 dictData 字段）、`#worker-internals` 用 note（用户无需手写 Worker）。
4. **代码注释中文**：与 WebIntegration 指南一致，正文英文叙述 + 代码注释中文。
5. **Phase 2/3 范围**：未改其他 website 页面、未改 bindings/ / crates/、未改 SearchDemo / demo-results.json、未改 package.json / vite.config.ts。

## 文件清单

| 文件 | 操作 | 说明 |
|------|------|------|
| `website/src/pages/api/Web.tsx` | 新建 | 12 节 h2，@vane-rs/web API + 类型签名专题页 |
| `website/src/routes.ts` | 修改 | import ApiWeb + route entry（errors 后、Examples 前） |
| `website/src/nav.ts` | 修改 | API Reference section 加 web 条目（errors 后） |

## 12 节清单

1. `#createvane` — createVane 工厂签名 + 最小示例（dictData transferable + Worker 封装）
2. `#vane-interface` — Vane 接口完整签名（10 方法）+ 句柄语义 Callout + handle 用法示例
3. `#vane-workeropts` — VaneWorkerOpts 类型定义 + 5 字段表格（vfs/dbPath/dictData/dictUrl/dictSha256）
4. `#schema` — Schema & FieldSchema 判别联合类型定义 + 3 分支表格 + 三种字段 schema 示例
5. `#doc` — Doc 类型定义 + add 示例
6. `#searchquery` — SearchQuery 类型定义 + 6 字段表格 + FusionSpec 2 分支表格 + filter gap Callout
7. `#hit` — Hit 类型定义
8. `#openoptions` — OpenOptions 类型定义 + 3 字段表格 + AutoCommit 2 分支表格
9. `#collectionoptions` — CollectionOptions 类型定义 + 3 字段表格 + 无 dictData 字段 note Callout
10. `#simd-probe` — simd128Supported() + SIMD128_TEST_MODULE 签名 + 可选用法示例 + 探针原理
11. `#dictdata` — dictData 内联优先 + CDN fallback + dictSha256 校验说明（两种方式 CodeBlock）
12. `#worker-internals` — Worker 内部实现信息块 + note Callout（用户无需手写 Worker）+ 指向 WebIntegration 指南
