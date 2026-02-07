// Vane Worker 入口（src/worker.ts → dist/worker.js）。
//
// 职责（§5 worker 入口策略）：
//   1. SIMD128 探针 → 选择 simd/scalar .wasm 产物。
//   2. init(wasmUrl) 显式传参加载 wasm（覆盖 vane_wasm.js 默认 bg.wasm URL）。
//   3. VaneWorker.create(opts) 初始化实例。
//   4. postMessage 路由：主页面 {op, id, ...} → VaneWorker 方法 → {id, result|error}。
//
// 基于 demo/worker.js（M2-14）模式，适配 @vane-rs/web 包结构：
//   - wasm 路径用 new URL('./vane_wasm_{simd,scalar}.wasm', import.meta.url)
//     （vite/webpack 识别 new URL 为 wasm asset）。
//   - 探针从 ./probe.js import（非内联）。
//
// postMessage 协议（与 crates/vane-wasm/src/worker.js + demo/worker.js 一致）：
//   主页面 postMessage({op, id, ...payload}) → Worker 调 VaneWorker 方法 →
//   postMessage({id, result | error})。

import init, { VaneWorker } from './vane_wasm.js';
import { simd128Supported } from './probe.js';

// Worker 运行在 DedicatedWorkerGlobalScope，非 Window。
// lib 同时含 DOM + WebWorker 时 self 类型冲突，显式 cast 到 Worker 上下文。
const ctx = self as unknown as DedicatedWorkerGlobalScope;

/** Worker 内 VaneWorker 实例（create 后赋值，close 后置 null）。 */
let worker: VaneWorker | null = null;

/**
 * 加载 wasm 模块（按 SIMD 探针结果动态选 simd/scalar 产物）。
 *
 * init(wasmUrl) 显式传 URL，覆盖 vane_wasm.js 默认的 new URL('vane_wasm_bg.wasm', ...)。
 * vite/webpack 识别 new URL('./vane_wasm_{simd,scalar}.wasm', import.meta.url) 为 wasm asset。
 */
async function loadWasm(): Promise<void> {
  const simd = simd128Supported();
  const wasmUrl = simd
    ? new URL('./vane_wasm_simd.wasm', import.meta.url)
    : new URL('./vane_wasm_scalar.wasm', import.meta.url);
  await init(wasmUrl);
}

ctx.onmessage = async (e: MessageEvent): Promise<void> => {
  const msg = e.data;
  // 忽略非请求消息。
  if (!msg || typeof msg.op !== 'string') return;

  const id: number | undefined = msg.id;

  try {
    // 首次 create：加载 wasm + init VaneWorker。
    if (msg.op === 'create') {
      await loadWasm();
      worker = await VaneWorker.create(msg.opts ?? {});
      ctx.postMessage({ id, result: true });
      return;
    }

    if (!worker) {
      ctx.postMessage({ id, error: 'worker not created (send {op:"create"} first)' });
      return;
    }

    let result: unknown;
    switch (msg.op) {
      case 'open':
        await worker.open(msg.path ?? 'vane.db', msg.opts ?? {});
        result = true;
        break;
      case 'collection':
        result = await worker.collection(msg.name, msg.schema ?? {}, msg.opts ?? {});
        break;
      case 'add':
        result = await worker.add(msg.col, msg.docs ?? []);
        break;
      case 'flush':
        await worker.flush(msg.col);
        result = true;
        break;
      case 'search':
        result = await worker.search(msg.col, msg.query ?? {});
        break;
      case 'delete':
        result = await worker.delete(msg.col, msg.ids ?? []);
        break;
      case 'compact':
        await worker.compact(msg.col);
        result = true;
        break;
      case 'reindex':
        result = await worker.reindex(msg.col);
        break;
      case 'export':
        await worker.export(msg.dest ?? '');
        result = true;
        break;
      case 'readFile':
        result = await worker.readFile(msg.path ?? '');
        break;
      case 'close':
        await worker.close();
        worker = null;
        result = true;
        break;
      default:
        ctx.postMessage({ id, error: `unknown op: ${msg.op}` });
        return;
    }
    ctx.postMessage({ id, result });
  } catch (err) {
    // VaneWorker Promise reject → 透传错误（I-8 错误透传）。
    ctx.postMessage({ id, error: String(err) });
  }
};
