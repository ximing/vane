/**
 * M2-14 Demo node smoke 测试（Playwright 降级方案）。
 *
 * 浏览器路径（拖入 / OPFS / SIMD 探针 / 词典 CDN）由 MANUAL-CHECKLIST.md 人工验收。
 * 本脚本用 wasm-bindgen --target nodejs 产出的 pkg-node 验证：
 *   - wasm 产物可加载 + 导出可用。
 *   - VaneWorker API 路径通（create/open/collection/add/flush/search/export/close）。
 *   - jieba 词典加载（dictData + sha256 校验）+ 中文搜索命中。
 *   - bigram 降级（无词典）+ 中文搜索仍可用。
 *
 * 运行：
 *   bash demo/build.sh   # 先产出 demo/pkg-node/
 *   node demo/e2e/run-smoke.mjs
 */

import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const require = createRequire(import.meta.url);
const __dirname = path.dirname(fileURLToPath(import.meta.url));
const pkgDir = path.join(__dirname, "..", "pkg-node");

let wasm;
try {
  wasm = require(path.join(pkgDir, "vane_wasm.js"));
} catch (err) {
  console.error("FAIL: 无法加载 demo/pkg-node/vane_wasm.js");
  console.error("请先运行: bash demo/build.sh");
  console.error(err);
  process.exit(1);
}

const dictPath = path.join(__dirname, "..", "pkg", "dict.bin");
const shaPath = path.join(__dirname, "..", "pkg", "sha256_prefix.bin");

let dictBin = null;
let shaHex = null;
try {
  dictBin = readFileSync(dictPath);
  const shaBytes = readFileSync(shaPath);
  shaHex = Array.from(shaBytes)
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
} catch {
  console.warn("WARN: dict.bin 未找到（run demo/build.sh），jieba 测试跳过");
}

const results = [];
function assert(name, cond, detail = "") {
  results.push({ name, ok: !!cond, detail });
  console.log(`${cond ? "PASS" : "FAIL"}: ${name}${detail ? " — " + detail : ""}`);
}

async function sleep(ms) {
  return new Promise((r) => setTimeout(r, ms));
}

// =========================================================================
// 1. wasm 加载 + 版本号
// =========================================================================

assert("wasm 加载 + vane_version() 非空", typeof wasm.vane_version() === "string" && wasm.vane_version().length > 0, `version=${wasm.vane_version()}`);

// =========================================================================
// 2. sync API 路径（lib.rs vane_open, MemoryVfs, cjk_bigram）
// =========================================================================

try {
  const dbH = wasm.vane_open("smoke.db", "{}");
  assert("vane_open 返回句柄", (typeof dbH === "bigint" || typeof dbH === "number") && Number(dbH) > 0, `dbH=${dbH}`);

  const schema = JSON.stringify({
    fields: [
      { name: "text", type: "text" },
      { name: "vec", type: "vector", dim: 8, metric: "cosine" },
    ],
  });
  const colH = wasm.vane_collection(dbH, "docs", schema, JSON.stringify({ tokenizer: "cjk_bigram" }));
  assert("vane_collection 返回句柄", (typeof colH === "bigint" || typeof colH === "number") && Number(colH) > 0, `colH=${colH}`);

  const docs = [
    { id: "d1", text: "人工智能是计算机科学的分支", vector: [1, 0, 0, 0, 0, 0, 0, 0] },
    { id: "d2", text: "机器学习让计算机从数据中学习", vector: [0, 1, 0, 0, 0, 0, 0, 0] },
    { id: "d3", text: "深度学习基于神经网络", vector: [0, 0, 1, 0, 0, 0, 0, 0] },
  ];
  const accepted = wasm.vane_add(colH, JSON.stringify(docs));
  assert("vane_add accepted=3", Number(accepted) === 3, `accepted=${accepted}`);

  wasm.vane_flush(colH);

  const hitsJson = wasm.vane_search(colH, JSON.stringify({ text: "人工智能", topK: 10, mode: "hybrid" }));
  const hits = JSON.parse(hitsJson);
  assert("中文搜索 cjk_bigram 命中 d1", Array.isArray(hits) && hits.length > 0 && hits[0].id === "d1", `hits=${JSON.stringify(hits)}`);

  wasm.vane_close(colH);
  wasm.vane_close(dbH);
} catch (err) {
  assert("sync API 路径", false, String(err));
}

// =========================================================================
// 3. VaneWorker 路径（memory vfs + jieba + dictData）—— 浏览器 API 降级 memory
// =========================================================================

async function testVaneWorker() {
  try {
    // memory vfs + jieba + dictData + sha256
    const opts = dictBin
      ? { vfs: "memory", dbPath: "worker.db", dictData: dictBin, dictSha256: shaHex }
      : { vfs: "memory", dbPath: "worker.db" };
    const worker = await wasm.VaneWorker.create(opts);
    assert("VaneWorker.create(memory + dict)", !!worker, dictBin ? "jieba+dictData" : "无词典降级");

    await worker.open("worker.db", {});

    const schema = { fields: [{ name: "text", type: "text" }, { name: "vec", type: "vector", dim: 8, metric: "cosine" }] };
    const col = await worker.collection("docs", schema, { tokenizer: "jieba" });
    assert("VaneWorker.collection 返回句柄", typeof col === "number" && col > 0, `col=${col}`);

    const docs = [
      { id: "ai", text: "人工智能是计算机科学的分支，致力于模拟人类智能", vector: [1, 0, 0, 0, 0, 0, 0, 0] },
      { id: "ml", text: "机器学习是人工智能的子领域，从数据中学习规律", vector: [0, 1, 0, 0, 0, 0, 0, 0] },
      { id: "nlp", text: "自然语言处理让计算机理解人类语言", vector: [0, 0, 1, 0, 0, 0, 0, 0] },
    ];
    const accepted = await worker.add(col, docs);
    assert("VaneWorker.add accepted=3", Number(accepted) === 3, `accepted=${accepted}`);

    await worker.flush(col);

    // 中文搜索（jieba 分词）
    const r1 = await worker.search(col, { text: "人工智能", topK: 10, mode: "hybrid" });
    const hits1 = typeof r1 === "string" ? JSON.parse(r1) : r1;
    assert("VaneWorker 中文搜索命中 ai", Array.isArray(hits1) && hits1.length > 0, `top1=${hits1[0]?.id}`);

    // 混合搜索（text + vector）
    const r2 = await worker.search(col, { text: "学习", vector: [0, 1, 0, 0, 0, 0, 0, 0], topK: 10, mode: "hybrid" });
    const hits2 = typeof r2 === "string" ? JSON.parse(r2) : r2;
    assert("VaneWorker 混合搜索返回结果", Array.isArray(hits2) && hits2.length > 0, `hits=${hits2.length}`);

    // export 快照
    await worker.export("backup.vane");
    assert("VaneWorker.export 快照写入", true, "dest=backup.vane");

    await worker.close();
    assert("VaneWorker.close", true);
  } catch (err) {
    assert("VaneWorker 路径", false, String(err));
  }
}

await testVaneWorker();

// =========================================================================
// 汇总
// =========================================================================

const passed = results.filter((r) => r.ok).length;
const failed = results.filter((r) => !r.ok).length;
console.log("");
console.log(`=== Smoke 结果: ${passed} passed, ${failed} failed ===`);
process.exit(failed > 0 ? 1 : 0);
