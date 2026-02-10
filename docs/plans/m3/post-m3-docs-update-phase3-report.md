# Phase 3 Report：Overview verb table Web 列 + 根 README 更新

## 状态

✅ 完成。Overview.tsx verb table 从三列扩为四列（IDL / JS (Node) / JS (Web) / Go），根 README.md 全站更新到 v0.2.0/M3。

## Commits

| Commit | 描述 |
|--------|------|
| `b6f4684` | `feat(docs): Overview verb table 加 JS (Web) 列 — vane.verb(col,...) 句柄式 + 返回类型差异` |
| `0babed0` | `docs(readme): 根 README 全站更新到 v0.2.0/M3 — @vane-rs/web npm 包 + Web 列 + examples` |

## 测试摘要

`tsc --noEmit` exit 0；`npm run build` 成功（16 routes sitemap）；README grep `npm install @vane-rs/web` 存在、`demo/build.sh` / `build-wasm-variants` 残留为空；git diff 只改 Overview.tsx + README.md（+ 本报告）。

## Overview verb table Web 列内容

| Operation | JS (Web) |
|-----------|----------|
| Open a database | `createVane(opts)` → `Promise<Vane>`; `vane.open(path, opts)` |
| Create / open a collection | `vane.collection(name, schema, opts)` → `Promise<number>` |
| List collections | — (not yet exposed) |
| Add documents | `vane.add(col, docs)` → `Promise<number>` |
| Make writes visible | `vane.flush(col)` → `Promise<void>` |
| Search | `vane.search(col, query)` → `Promise<Hit[]>` |
| Delete by id | `vane.delete(col, ids)` → `Promise<number>` |
| Compact segments | `vane.compact(col)` → `Promise<void>` |
| Reindex | `vane.reindex(col)` → `Promise<number>` |
| Export snapshot | `vane.export(dest)` → `Promise<void>` |
| Close | `vane.close()` → `Promise<void>` |

### Web 列关键准确性

1. **句柄式**：所有 verb 形态为 `vane.<verb>(col, ...)`，非 `col.<verb>(...)`。`col` 是 `collection()` 返回的 `number` 句柄。
2. **返回类型差异**：
   - `add` → `Promise<number>`（accepted 计数），vs Node 的 `Promise<{accepted, visibleAfterFlush}>`
   - `reindex` → `Promise<number>`（progress 0.0–1.0），vs Node 的 `Promise<VaneReindexHandle>`
   - `search` → `Promise<Hit[]>`（VaneImpl 内部已 JSON.parse，用户无需手动 parse）
3. **Open 行特殊**：Web 入口是 `createVane(opts)` 工厂（封装 Worker），返回 `Vane` 实例后调 `vane.open(path, opts)`。
4. **表下方说明段**：加了 Web 入口 createVane() 工厂说明 + collection 返 number 句柄 + add/reindex 返 Promise<number> + 指向 `/api/web` 类型参考页。

### Prose 改动

- verb table intro：`three-side` → `four-side`；`through a createVane factory` → `through the @vane-rs/web package's createVane() factory; collections are addressed by number handle`

## README 改动清单

### 1. Browser install 段（原 :142-154）

- **旧**：`bash scripts/build-wasm-variants.sh` + SIMD/scalar 变体说明 + CDN 词典说明
- **新**：`npm install @vane-rs/web @vane-rs/dict-zh` + ESM 说明（wasm-bindgen 双变体 + Worker + dictData 内联，vite 6+/webpack 5 原生识别 `new URL(..., import.meta.url)`，无需插件）+ 指向文档站 `https://ximing.github.io/vane/guides/web-integration`

### 2. Quick start Browser 段（原 :262-275）

- **旧**：`bash demo/build.sh` + `python3 -m http.server 8765` + 指向 `demo/README.md`
- **新**：`npm install @vane-rs/web @vane-rs/dict-zh` + 完整 TS 用法模板（createVane + dictData + open + collection(jieba) + add + flush + search → Hit[]）+ 指向 `examples/vite/` + `examples/webpack/` + 文档站 web-integration 指南

### 3. API reference 表（原 :282-294）

- **旧**：三列（Operation / Node.js / Go）
- **新**：四列（Operation / Node.js / Web / Go），Web 列每行用 `vane.<verb>(col, ...)` 形态 + 返回类型差异（collection → number, add → number, reindex → number, search → Hit[]）。表下加 `>` blockquote 注 Web 句柄式说明 + 指向 `https://ximing.github.io/vane/api/web`

### 4. Status 段（原 :422-435）

- `v0.1.0` → `v0.2.0`；`M0–M2` → `M0–M3`
- Browser 行：`(wasm-bindgen + Worker, OPFS/IDB, SIMD dual variants)` → `(@vane-rs/web npm package — wasm-bindgen + Worker, OPFS/IDB, SIMD dual variants)`
- 新增 M3 条目：`✅ @vane-rs/web + @vane-rs/dict-zh npm packages (ESM, vite 6+/webpack 5 native, dictData inline transfer, zero CDN)`

### 5. Examples 段（原 :437-448）

- **旧**：`demo/` — Browser 示例
- **新**：`examples/vite/`（推荐）+ `examples/webpack/`（webpack 5 `outputModule` + asset/resource 配置），移除 `demo/` 引用

## Concerns

无。所有验证通过，改动范围严格限定在 Overview.tsx + README.md（+ 本报告）。未触碰 bindings/web/、crates/、website/ 其他页面、api/Web.tsx、SearchDemo / demo-results.json。
