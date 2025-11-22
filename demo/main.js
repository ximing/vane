/**
 * Vane Demo 主页面 JS（M2-14）。
 *
 * 职责：
 * 1. 启动 Worker（demo/worker.js）。
 * 2. 拖入 markdown 文件夹 → 递归解析 .md 文件为 {id, text, vector}。
 * 3. 调 VaneWorker API（create/open/collection/add/flush/search/export）。
 * 4. UI 交互（搜索框、结果列表、导出按钮、状态日志）。
 *
 * vector 用占位 hash 向量（SPEC Won't-have：Vane 不内置 embedding）：
 * 简单 char unigram/bigram bucket → 64 维 L2 归一化。同文本同向量、共享字符的文本
 * cosine 相似度更高，足以演示 vector 召回路与 hybrid 融合。生产应替换为真实 embedding
 * API（transformers.js / OpenAI / Cohere / 本地模型）。
 */

const DIM = 64;
const DB_PATH = "vane.db";
const COL_NAME = "docs";
const DICT_URL = "./pkg/dict.bin"; // demo 自托管 dict.bin（亦可换 CDN URL）

// =========================================================================
// Worker 通信封装（postMessage Promise 边界）
// =========================================================================

const worker = new Worker(new URL("./worker.js", import.meta.url), { type: "module" });
let nextId = 1;
const pending = new Map();

worker.onmessage = (e) => {
  const { id, result, error } = e.data;
  if (id == null) return;
  const p = pending.get(id);
  if (!p) return;
  pending.delete(id);
  if (error) p.reject(new Error(error));
  else p.resolve(result);
};

worker.onerror = (e) => {
  log(`[worker error] ${e.message}`, "error");
};

function call(op, payload = {}) {
  const id = nextId++;
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject });
    worker.postMessage({ op, id, ...payload });
  });
}

// =========================================================================
// 占位 hash 向量（SPEC Won't-have：不内置 embedding）
// =========================================================================

function hashVector(text, dim = DIM) {
  const vec = new Float32Array(dim);
  const chars = [...text];
  for (let i = 0; i < chars.length; i++) {
    const code = chars[i].codePointAt(0) || 0;
    vec[code % dim] += 1;
    if (i > 0) {
      const prev = chars[i - 1].codePointAt(0) || 0;
      const bucket = (prev * 31 + code) % dim;
      vec[bucket] += 1;
    }
  }
  let norm = 0;
  for (let i = 0; i < dim; i++) norm += vec[i] * vec[i];
  norm = Math.sqrt(norm) || 1;
  for (let i = 0; i < dim; i++) vec[i] /= norm;
  return Array.from(vec);
}

// =========================================================================
// 拖入 markdown 文件夹处理（webkitGetAsEntry 递归遍历）
// =========================================================================

async function handleDrop(dataTransfer) {
  const items = dataTransfer.items;
  if (!items) return [];
  const docs = [];
  const entries = [];
  for (const item of items) {
    const entry = item.webkitGetAsEntry?.();
    if (entry) entries.push(entry);
  }
  if (entries.length === 0) {
    // 降级：直接文件（无目录结构）
    for (const file of dataTransfer.files) {
      if (file.name.endsWith(".md")) {
        const text = await file.text();
        docs.push({ id: file.name, text, vector: hashVector(text) });
      }
    }
    return docs;
  }
  for (const entry of entries) {
    await walkEntry(entry, "", docs);
  }
  return docs;
}

function walkEntry(entry, prefix, docs) {
  return new Promise((resolve) => {
    if (entry.isFile) {
      if (!entry.name.endsWith(".md")) return resolve();
      entry.file(async (file) => {
        try {
          const text = await file.text();
          const id = prefix + entry.name;
          docs.push({ id, text, vector: hashVector(text) });
        } catch (err) {
          log(`[skip] ${prefix}${entry.name}: ${err}`, "warn");
        }
        resolve();
      }, () => resolve());
    } else if (entry.isDirectory) {
      const reader = entry.createReader();
      readAllEntries(reader, async (allEntries) => {
        for (const e of allEntries) {
          await walkEntry(e, prefix + entry.name + "/", docs);
        }
        resolve();
      });
    } else {
      resolve();
    }
  });
}

function readAllEntries(reader, callback) {
  const all = [];
  const readBatch = () => {
    reader.readEntries((batch) => {
      if (!batch.length) {
        callback(all);
      } else {
        all.push(...batch);
        readBatch();
      }
    }, () => callback(all));
  };
  readBatch();
}

// =========================================================================
// Vane 初始化 + 索引 + 搜索
// =========================================================================

let colId = null;
let dictSha256Hex = null;

async function loadDictSha256() {
  // 读 sha256_prefix.bin（8 字节）→ 16 hex 字符串，传给 VaneWorker 做校验。
  try {
    const resp = await fetch("./pkg/sha256_prefix.bin");
    if (!resp.ok) return null;
    const buf = new Uint8Array(await resp.arrayBuffer());
    if (buf.length !== 8) return null;
    return Array.from(buf)
      .map((b) => b.toString(16).padStart(2, "0"))
      .join("");
  } catch {
    return null;
  }
}

async function initVane() {
  log("[init] creating VaneWorker (OPFS + jieba)...");
  dictSha256Hex = await loadDictSha256();
  const opts = {
    vfs: "opfs",
    dbPath: DB_PATH,
    dictUrl: DICT_URL,
  };
  if (dictSha256Hex) opts.dictSha256 = dictSha256Hex;
  await call("create", { opts });
  await call("open", { path: DB_PATH });
  const schema = {
    fields: [
      { name: "text", type: "text" },
      { name: "vec", type: "vector", dim: DIM, metric: "cosine" },
    ],
  };
  colId = await call("collection", {
    name: COL_NAME,
    schema,
    opts: { tokenizer: "jieba" },
  });
  log(`[init] ready (collection=${colId}). 拖入 markdown 文件夹开始索引。`);
  setStatus("ready");
  document.getElementById("search").disabled = false;
}

async function indexDocs(docs) {
  if (!colId) throw new Error("collection not ready");
  log(`[index] 灌入 ${docs.length} 篇 markdown...`);
  const t0 = performance.now();
  const accepted = await call("add", { col: colId, docs });
  await call("flush", { col: colId });
  const dt = (performance.now() - t0).toFixed(1);
  log(`[index] 完成：accepted=${accepted}，用时 ${dt}ms`);
}

async function search(query) {
  if (!colId) throw new Error("collection not ready");
  const vector = hashVector(query);
  const t0 = performance.now();
  const result = await call("search", {
    col: colId,
    query: { text: query, vector, topK: 10, mode: "hybrid" },
  });
  const dt = (performance.now() - t0).toFixed(1);
  const hits = typeof result === "string" ? JSON.parse(result) : result;
  renderResults(hits, query, dt);
}

async function exportBackup() {
  if (!colId) throw new Error("collection not ready");
  log("[export] 导出快照 backup.vane → OPFS...");
  await call("export", { dest: "backup.vane" });
  log("[export] 快照已写入 OPFS（backup.vane）");
}

// =========================================================================
// UI 渲染
// =========================================================================

function renderResults(hits, query, dtMs) {
  const list = document.getElementById("results");
  if (!hits.length) {
    list.innerHTML = `<li class="empty">无结果（${dtMs}ms）</li>`;
    return;
  }
  const items = hits
    .map((h, i) => {
      const score = typeof h.score === "number" ? h.score.toFixed(4) : h.score;
      return `<li class="hit">
        <div class="hit-head"><span class="rank">#${i + 1}</span><span class="id">${escapeHtml(h.id)}</span><span class="score">${score}</span></div>
        <div class="snippet">${escapeHtml(snippet(query, h.id))}</div>
      </li>`;
    })
    .join("");
  list.innerHTML = `<li class="meta">top ${hits.length}（${dtMs}ms）</li>${items}`;
}

function snippet(query, text) {
  // 简单片段：取 query 周围 80 字符。demo 侧重链路展示，非高亮精度。
  const idx = text.indexOf(query);
  if (idx < 0) return text.slice(0, 120);
  const start = Math.max(0, idx - 40);
  const end = Math.min(text.length, idx + query.length + 40);
  return (start > 0 ? "…" : "") + text.slice(start, end) + (end < text.length ? "…" : "");
}

function escapeHtml(s) {
  return String(s)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function log(msg, level = "info") {
  const el = document.getElementById("log");
  const line = document.createElement("div");
  line.className = `log-${level}`;
  line.textContent = `[${new Date().toLocaleTimeString()}] ${msg}`;
  el.appendChild(line);
  el.scrollTop = el.scrollHeight;
}

function setStatus(s) {
  document.getElementById("status").dataset.status = s;
  document.getElementById("status").textContent = s;
}

// =========================================================================
// 事件绑定
// =========================================================================

const dropZone = document.getElementById("dropzone");
const searchInput = document.getElementById("search");
const exportBtn = document.getElementById("export");

dropZone.addEventListener("dragover", (e) => {
  e.preventDefault();
  dropZone.classList.add("drag");
});
dropZone.addEventListener("dragleave", () => dropZone.classList.remove("drag"));
dropZone.addEventListener("drop", async (e) => {
  e.preventDefault();
  dropZone.classList.remove("drag");
  setStatus("indexing");
  try {
    const docs = await handleDrop(e.dataTransfer);
    if (!docs.length) {
      log("[drop] 未找到 .md 文件（请拖入含 markdown 的文件夹）", "warn");
      setStatus("ready");
      return;
    }
    await indexDocs(docs);
  } catch (err) {
    log(`[drop error] ${err.message}`, "error");
    setStatus("ready");
  }
});

let searchTimer = null;
searchInput.addEventListener("input", () => {
  clearTimeout(searchTimer);
  const q = searchInput.value.trim();
  if (!q) {
    document.getElementById("results").innerHTML = "";
    return;
  }
  searchTimer = setTimeout(async () => {
    try {
      await search(q);
    } catch (err) {
      log(`[search error] ${err.message}`, "error");
    }
  }, 200);
});

exportBtn.addEventListener("click", async () => {
  try {
    await exportBackup();
  } catch (err) {
    log(`[export error] ${err.message}`, "error");
  }
});

// =========================================================================
// 加载示例（demo 便捷入口 + e2e/截图辅助）
// =========================================================================

async function loadSamples() {
  if (!colId) throw new Error("collection not ready");
  const files = [
    "samples/01-ai-intro.md",
    "samples/02-vector-search.md",
    "samples/03-jieba-tokenizer.md",
    "samples/04-opfs-persistence.md",
    "samples/05-simd-probe.md",
  ];
  const docs = [];
  for (const f of files) {
    try {
      const resp = await fetch(f);
      if (!resp.ok) continue;
      const text = await resp.text();
      docs.push({ id: f, text, vector: hashVector(text) });
    } catch {
      // skip
    }
  }
  if (!docs.length) {
    log("[samples] 未找到示例文件", "warn");
    return;
  }
  await indexDocs(docs);
}

document.getElementById("samples").addEventListener("click", () => {
  loadSamples().catch((err) => log(`[samples error] ${err.message}`, "error"));
});

// 暴露 demo API（e2e/截图辅助，非生产 API）
window.__vaneDemo = { call, indexDocs, search, loadSamples, hashVector, get colId() { return colId; } };

// =========================================================================
// 启动
// =========================================================================

initVane().catch((err) => {
  log(`[init error] ${err.message}`, "error");
  setStatus("error");
});
