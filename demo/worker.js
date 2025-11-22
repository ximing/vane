/**
 * Vane Demo Worker 入口（M2-14，基于 M2-04 VaneWorker 壳扩展）。
 *
 * 相对 crates/vane-wasm/src/worker.js 的扩展：
 *   1. 嵌入 SIMD128 测试模块（与 simd_probe.rs SIMD128_TEST_MODULE 一致），
 *      运行时 `WebAssembly.validate` 探测 simd128 支持。
 *   2. 据探针结果动态加载 vane_wasm_simd.wasm 或 vane_wasm_scalar.wasm
 *      （两产物共享同一份 vane_wasm.js 胶水，导出一致）。
 *   3. console.log 探针结果（验收项 6）。
 *
 * postMessage 协议（与 M2-04 worker.js 一致）：
 *   主页面 postMessage({op, id?, ...}) → Worker 调 VaneWorker 方法 →
 *   postMessage({id, result | error})。
 *
 * VaneWorker API（M2-04）：
 *   create(opts) → Promise<VaneWorker>
 *     opts: { vfs: "opfs"|"idb"|"memory", dbPath, dictUrl, dictSha256, dictData }
 *   open(path, opts) / collection(name, schema, opts) / add(col, docs) /
 *   flush(col) / search(col, query) / delete(col, ids) / compact(col) /
 *   reindex(col) / export(dest) / close()
 */

import init, { VaneWorker } from "./pkg/vane_wasm.js";

// SIMD128 测试模块（与 crates/vane-wasm/src/simd_probe.rs SIMD128_TEST_MODULE 一致）。
// 含 v128.const 指令（opcode FD 0C + 16 字节立即数），仅 simd128 运行时 validate 通过。
const SIMD128_TEST_MODULE = new Uint8Array([
  // [magic + version] 8 bytes
  0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00,
  // [type section (id=1)] 1 type: () -> ()
  0x01, 0x04, 0x01, 0x60, 0x00, 0x00,
  // [function section (id=3)] 1 function, type idx 0
  0x03, 0x02, 0x01, 0x00,
  // [export section (id=7)] "t" -> function 0
  0x07, 0x05, 0x01, 0x01, 0x74, 0x00, 0x00,
  // [code section (id=10)] 1 body, body_size=0x15, 0 locals
  0x0a, 0x17, 0x01, 0x15, 0x00,
  // v128.const (opcode FD 0C) + 16-byte immediate (all zeros)
  0xfd, 0x0c, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
  0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
  // drop (0x1A) + end (0x0B)
  0x1a, 0x0b,
]);

/** 探测运行时是否支持 WebAssembly SIMD128。 */
function simd128Supported() {
  try {
    return WebAssembly.validate(SIMD128_TEST_MODULE);
  } catch {
    return false;
  }
}

let worker = null;
let nextMsgId = 1;
const pending = new Map(); // id → { resolve, reject }

/** 加载 wasm 模块（按 SIMD 探针结果动态选 simd/scalar 产物）。 */
async function loadWasm() {
  const simd = simd128Supported();
  const wasmUrl = simd ? "./pkg/vane_wasm_simd.wasm" : "./pkg/vane_wasm_scalar.wasm";
  console.log(
    `[vane-demo] SIMD128 ${simd ? "supported" : "not supported"} → loading ${simd ? "simd" : "scalar"} wasm: ${wasmUrl}`
  );
  await init(wasmUrl);
}

self.onmessage = async (e) => {
  const msg = e.data;
  // 忽略非请求消息。
  if (!msg || typeof msg.op !== "string") return;

  const id = msg.id ?? nextMsgId++;

  try {
    // 首次 create：加载 wasm + init VaneWorker。
    if (msg.op === "create") {
      await loadWasm();
      worker = await VaneWorker.create(msg.opts ?? {});
      self.postMessage({ id, result: true });
      return;
    }

    if (!worker) {
      self.postMessage({ id, error: "worker not created (send {op:'create'} first)" });
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
        await worker.export(msg.dest ?? "backup.vane");
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
