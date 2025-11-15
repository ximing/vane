/**
 * Vane Dedicated Worker 入口 JS 胶水（SPEC §4.1 / §11）。
 *
 * 职责：
 * 1. 加载 vane-wasm wasm 模块（wasm-bindgen 生成）。
 * 2. 路由主页面 postMessage → VaneWorker 方法调用 → 结果回传。
 *
 * postMessage Promise 边界：主页面 `postMessage({op, ...})` → Worker 调对应
 * VaneWorker 方法（返 Promise）→ await → `postMessage({id, result/error})`。
 * Worker 内 core 调用全同步（REQUIREMENTS §4.1，I-8 薄壳）。
 *
 * 用法（主页面）：
 * ```js
 * const worker = new Worker(new URL("./worker.js", import.meta.url), { type: "module" });
 * worker.postMessage({ op: "create", opts: { vfs: "opfs", dbPath: "vane.db" } });
 * worker.onmessage = (e) => { /* e.data = { id, result | error } *\/ };
 * ```
 *
 * 浏览器手动验证标注（node 无 Worker/OPFS/IDB）：
 * - create 异步 init（OPFS/IDB 探针 + 词典 CDN fetch）。
 * - postMessage Promise 边界 round-trip。
 * - close 后调用拒绝。
 */

// wasm-bindgen 生成模块加载（路径由构建工具/bundler 解析）。
// 部署时：wasm-pack build → www/pkg/ + 此文件作为 Worker 入口。
import init, { VaneWorker } from "./vane_wasm.js";

let worker = null;
let nextMsgId = 1;
const pending = new Map(); // id → { resolve, reject }

self.onmessage = async (e) => {
  const msg = e.data;
  // 忽略非请求消息。
  if (!msg || typeof msg.op !== "string") return;

  const id = msg.id ?? nextMsgId++;

  try {
    // 首次 create：初始化 wasm 模块 + VaneWorker。
    if (msg.op === "create") {
      await init(); // 加载 wasm
      worker = await VaneWorker.create(msg.opts ?? {});
      self.postMessage({ id, result: true });
      return;
    }

    if (!worker) {
      self.postMessage({ id, error: "worker not created" });
      return;
    }

    let result;
    switch (msg.op) {
      case "open":
        await worker.open(msg.path ?? "vane.db", msg.opts ?? {});
        result = true;
        break;
      case "collection":
        result = await worker.collection(msg.name, msg.schema ?? {}, msg.opts ?? {});
        break;
      case "add":
        result = await worker.add(msg.col, msg.docs ?? []);
        break;
      case "flush":
        await worker.flush(msg.col);
        result = true;
        break;
      case "search":
        result = await worker.search(msg.col, msg.query ?? {});
        break;
      case "delete":
        result = await worker.delete(msg.col, msg.ids ?? []);
        break;
      case "compact":
        await worker.compact(msg.col);
        result = true;
        break;
      case "reindex":
        result = await worker.reindex(msg.col);
        break;
      case "export":
        await worker.export(msg.dest ?? "");
        result = true;
        break;
      case "close":
        await worker.close();
        worker = null;
        result = true;
        break;
      default:
        self.postMessage({ id, error: `unknown op: ${msg.op}` });
        return;
    }
    self.postMessage({ id, result });
  } catch (err) {
    // VaneWorker Promise reject → 透传错误（I-8 错误透传）。
    self.postMessage({ id, error: String(err) });
  }
};
