# 文档站全站更新 Phase 2 报告

## 状态

**完成**。12 个文件的 LangTabs browser pane + 相关 prose 从老路径（`./pkg/vane_wasm.js` + `VaneWorker` + `worker.verb` + `JSON.parse` + `build.sh` + CDN）统一切到 @vane-rs/web npm 用法（`createVane` 工厂 + `vane.verb(col, ...)` 句柄式 + `dictData` 内联）。tsc exit 0，vite build 成功（16 routes），老路径残留扫描符合预期。

## Commits

| commit | 范围 | 文件数 |
|--------|------|--------|
| `893ec9b` | api 子页 7 个（Overview/Open/Collection/Documents/Search/Maintenance/Errors） | 7 |
| `ad5c32f` | 首接触页 3 个（QuickStart/Home/Examples） | 3 |
| `3467246` | guides 2 个（Tokenizers/Reindex） | 2 |

分支：`feat/docs-web-update`（Phase 1 已在此分支）。

## 测试摘要

`tsc --noEmit` exit 0；`npm run build` 成功（177 modules，16 routes，sitemap 含 /api/web）；老路径残留扫描 4 项中 3 项全空，1 项仅 WebIntegration.tsx + Web.tsx 的 informational `VaneWorker.create`（Phase 1 预期保留）。

## 12 文件改动清单

### api 子页 7 个

1. **api/Overview.tsx** — BROWSER_SNIPPET 切完整模板（createVane → open → collection → add → flush → search → close）；BROWSER_ERR 切 `vane.search`；verb table prose 从 "VaneWorker wasm glue" 改 "createVane 工厂 + 句柄式"；两个 LangTabs browser pane lang `js→ts`、title `worker.js/errors.js → main.ts/errors.ts`。

2. **api/Open.tsx** — BROWSER_SNIPPET 切 `createVane({dictData,vfs,dbPath}) + vane.open(path, opts)`；prose "browser worker rejects" → "browser rejects"；LangTabs lang/title `js→ts`。

3. **api/Collection.tsx** — BROWSER_SNIPPET 切 `vane.collection(name, schema, opts) → Promise<number>`（聚焦模板 + 顶部注释）；CollectionOptions 表删除 `dictData` 行（类型里无此字段）+ 加 Callout type='note' 标注 dictData 属 createVane；LangTabs lang/title `js→ts`。

4. **api/Documents.tsx** — BROWSER_SNIPPET 切 `vane.add(col, docs)→number / vane.flush(col) / vane.delete(col, ids)→number`（聚焦模板）；LangTabs lang/title `js→ts`。

5. **api/Search.tsx** — BROWSER_SNIPPET 切 `vane.search(col, query) → Hit[]`（**删 JSON.parse**）；加 `import type { Hit }`；LangTabs lang/title `js→ts`。

6. **api/Maintenance.tsx** — BROWSER_SNIPPET 切 `vane.compact/reindex/export/readFile/close`（聚焦模板）；prose 三处 `worker.→vane.`（reindex/readFile/close）；LangTabs lang/title `js→ts`。

7. **api/Errors.tsx** — IDL 注释 `worker Promise → Vane Promise`；BROWSER_SNIPPET 切 `vane.add` try/catch + 注释补 "dictData 内联后无 CDN 失败路径"；WASM note Callout 补 "dictData inlined from @vane-rs/dict-zh → no network fetch"；LangTabs lang/title `js→ts`。

### 首接触页 3 个

8. **QuickStart.tsx** — BROWSER_INSTALL 切 `npm install @vane-rs/web @vane-rs/dict-zh`；BROWSER_OPEN 切 `createVane + vane.open`；新增 BROWSER_INDEX（`vane.collection + vane.add + vane.flush`）+ BROWSER_SEARCH（`vane.search → Hit[]`）常量；4 步 LangTabs browser pane 全切句柄式 CodeBlock；"Choose your runtime" + install + open 段 CDN 文案改 dictData 内联优先；open 段加 WebIntegration 指南链接；加 `import { Link }`。

9. **Home.tsx** — BROWSER_SNIPPET 切 `npm install + createVane + open + collection + add + flush + search`（精简版完整模板，含 `import type { Hit }`）；LangTabs lang `bash→ts`、title `shell→main.ts`。

10. **Examples.tsx** — Browser 卡从 "demo/ + jsdelivr CDN + 截图" 切 "examples/vite + examples/webpack integration"；BROWSER_RUN 常量替换为 BROWSER_VITE_RUN + BROWSER_WEBPACK_RUN；How to run 两子节（`npm run dev` / `npm run serve`）；Callout type='note' 指向 WebIntegration 指南；移除 `<img>` demo 截图；加 `import { Link }`。

### guides 2 个

11. **guides/Tokenizers.tsx** — dict distribution 表 Browser 行 CDN 文案改 "Inline dictData from @vane-rs/dict-zh (zero-CDN, transferable); jsdelivr CDN fallback"；custom-dict LangTabs browser pane 从 `col.setUserDict` + `db.collection({dictData})`（Web 无此 API + dictData 错位置）切到 `vane.collection('docs', schema, {tokenizer:'jieba', userDict:[...]})` + Callout type='gap' 标注 "Web has no runtime setUserDict"（域词须 collection 创建时传 userDict）；lang `js→ts`。

12. **guides/Reindex.tsx** — best-practices LangTabs browser pane 从 `db.collection` 切 `vane.collection`（lang `js→ts`）；reindex.mjs 独立 Node snippet 保留不改 + 其后加 Callout type='note' 标注 "Web: vane.reindex(col) returns a number, not a handle"（Web 端返 `Promise<number>`，无 ReindexHandle）。

## 老路径残留扫描结果

```bash
# scan 1: pkg/vane_wasm + VaneWorker.create + demo/build.sh + build-wasm-variants
grep -rn 'pkg/vane_wasm\|VaneWorker\.create\|demo/build\.sh\|build-wasm-variants' website/src/
```
结果：`pkg/vane_wasm` / `demo/build.sh` / `build-wasm-variants` 全空。`VaneWorker.create` 仅出现在：
- `guides/WebIntegration.tsx:314` — informational CodeBlock "inside createVane"
- `api/Web.tsx:248` + `:881` — worker-internals 节

这三处是 Phase 1 已完成文件的 informational 节（描述 createVane 内部实现，非用户调用），预期保留。

```bash
# scan 2: worker.verb
grep -rn 'worker\.\(open\|add\|search\|collection\|flush\|delete\|compact\|reindex\|close\|readFile\)' website/src/
```
结果：**全空**。

```bash
# scan 3: col.setUserDict / db.collection.*dictData
grep -rn 'col\.setUserDict\|db\.collection.*dictData' website/src/pages/
```
结果：`col.setUserDict` 出现在：
- `guides/Tokenizers.tsx:197` — **Node pane** `user-dict.mjs`（Node binding 真实 API，任务要求不改 Node pane）
- `guides/Reindex.tsx:159` — **独立 Node snippet** `stage-dict.mjs`（任务要求保留）

`db.collection.*dictData` 全空。browser pane 里无 `col.setUserDict` 实际调用（Tokenizers browser pane 已切 `vane.collection(..., {userDict})` + Callout gap 标注）。

```bash
# scan 4: JSON.parse(await
grep -rn 'JSON\.parse(await' website/src/
```
结果：**全空**。

## concerns

1. **scan 3 的 `col.setUserDict` 在 Node pane**：Tokenizers.tsx:197 和 Reindex.tsx:159 的 `col.setUserDict` 是 Node binding 的真实 API（Node pane 不改），非 browser pane 残留。任务的验证标准说"应全空"，但与"不改 Node/Go pane"矛盾。当前保留 Node pane 的 `col.setUserDict` 是正确做法——Node binding 确实有此 API，browser pane 已用 Callout gap 如实标注 Web 端无此 API。

2. **Tokenizers.tsx 的 setUserDict Callout note**：:233-238 有一段 Callout type='note' title="setUserDict stages, it does not apply"，描述 Node 的 setUserDict staged 行为。这段在 LangTabs 之后（所有 runtime 共享），对 Web 端用户可能略困惑（Web 无 setUserDict）。但 browser pane 内已有 Callout gap 标注，Web 端用户先看到 gap 标注再看此 note，上下文足够清晰。未改此 Callout 以遵守"最小改动"原则。

3. **Examples.tsx 的 screenshots 资源**：移除了 `<img src="screenshots/browser-markdown-demo.jpg">` 引用，但 `public/screenshots/` 目录下的图片文件未删除（不在本次 12 文件范围内）。不影响 build。

4. **Phase 3 未做**：本 Phase 只改 browser pane + 相关 prose。api/Overview.tsx 的 verb table 加第 4 列 "JS (Web)"、根 README.md 的 Browser 段更新属于 Phase 3，未在本次实施范围。
