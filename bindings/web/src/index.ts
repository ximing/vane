// Vane 主线程 API（src/index.ts → dist/index.js）。
//
// §5 createVane 工厂：封装 new Worker + postMessage Promise 边界 + dictData transferable。
// §6 TS 类型：手写强类型 Vane 接口，不直接 re-export wasm-bindgen 的 VaneWorker（opts 是 any）。
//
// 用法：
//   import { createVane } from '@vane-rs/web';
//   const vane = await createVane({ vfs: 'opfs', dbPath: 'vane.db' });
//   await vane.open();
//   const col = await vane.collection('docs', schema, { tokenizer: 'jieba' });

import type {
  VaneWorkerOpts,
  Schema,
  Doc,
  Hit,
  SearchQuery,
  OpenOptions,
  CollectionOptions,
  Vane,
} from './types.js';

// §4 CDN fallback 默认 URL（@vane-rs/web 层默认值，不改 dict_loader.rs）。
// 用户未提供 dictData 且未指定 dictUrl 时自动填入；dict_loader fetch 失败降级 bigram。
const DEFAULT_DICT_URL = 'https://cdn.jsdelivr.net/npm/@vane-rs/dict-zh@2026.8.0/dict.bin';

/**
 * Vane 实例实现（内部类，不导出）。
 *
 * 封装 Worker 通信：
 * - `pending` Map：id → {resolve, reject}，postMessage Promise 边界。
 * - `call(op, payload, transfer)`：发 {op, id, ...payload} → 等 {id, result|error}。
 * - `close()` 后 reject 所有后续调用（I-7 句柄注销）。
 */
class VaneImpl implements Vane {
  private closed = false;
  private readonly pending = new Map<
    number,
    { resolve: (v: unknown) => void; reject: (e: Error) => void }
  >();
  private nextId = 1;

  private constructor(private readonly worker: Worker) {
    // 接收 Worker 响应：{id, result} 或 {id, error}。
    worker.onmessage = (e: MessageEvent): void => {
      const { id, result, error } = e.data;
      if (id == null) return;
      const p = this.pending.get(id);
      if (!p) return;
      this.pending.delete(id);
      if (error) p.reject(new Error(String(error)));
      else p.resolve(result);
    };

    // Worker 级别错误（加载失败、未捕获异常），reject 所有 pending。
    worker.onerror = (e: ErrorEvent): void => {
      for (const [, p] of this.pending) p.reject(new Error(e.message));
      this.pending.clear();
    };
  }

  /**
   * 工厂：创建 Worker + 发 create 消息（含 dictData transferable）。
   * 由 createVane() 调用，用户不直接使用。
   */
  static async create(opts: VaneWorkerOpts): Promise<Vane> {
    // §5 worker 入口策略：new Worker(new URL('./worker.js', import.meta.url), {type:'module'})
    // vite 6+ / webpack 5 原生识别此模式，打包 worker 为独立 chunk。
    const worker = new Worker(new URL('./worker.js', import.meta.url), { type: 'module' });
    const impl = new VaneImpl(worker);
    await impl.sendCreate(opts);
    return impl;
  }

  /**
   * 发 create 消息（dictData transferable 零拷贝）。
   *
   * §4 dictData 接口：
   * - Uint8Array → transfer .buffer（整个 backing buffer）。
   * - ArrayBuffer → transfer 本身。
   * - 未提供 dictData/dictUrl → 自动填 CDN fallback 默认 URL。
   *
   * ⚠️ W3 transferable detached 坑：transfer 后主线程不可再访问该 buffer。
   * 用户每次 fetch 新建 buffer 或用 slice() 拷贝。
   */
  private async sendCreate(opts: VaneWorkerOpts): Promise<void> {
    const { dictData, dictUrl, ...rest } = opts;
    const transfer: Transferable[] = [];
    const createOpts: Record<string, unknown> = { ...rest };

    // CDN fallback 默认 URL
    if (!dictData && !dictUrl) {
      createOpts.dictUrl = DEFAULT_DICT_URL;
    } else if (dictUrl) {
      createOpts.dictUrl = dictUrl;
    }

    // dictData transferable 零拷贝
    if (dictData instanceof Uint8Array) {
      // ⚠️ transfer 整个 backing buffer；若 Uint8Array 是大 buffer 的部分视图，
      // 需用户先 .slice() 拷贝。典型用法 new Uint8Array(await (await fetch()).arrayBuffer())
      // 的 buffer 恰好是完整词典字节，无此问题。
      const buf = dictData.buffer as ArrayBuffer;
      createOpts.dictData = buf;
      transfer.push(buf);
    } else if (dictData instanceof ArrayBuffer) {
      createOpts.dictData = dictData;
      transfer.push(dictData);
    }

    await this.call('create', { opts: createOpts }, transfer);
  }

  /**
   * postMessage Promise 边界：发 {op, id, ...payload} → 等 {id, result|error}。
   * @param transfer transferable 对象列表（零拷贝移交，detached 后主线程不可访问）。
   */
  private call(
    op: string,
    payload: Record<string, unknown> = {},
    transfer: Transferable[] = [],
  ): Promise<unknown> {
    if (this.closed) return Promise.reject(new Error('vane worker closed'));
    const id = this.nextId++;
    return new Promise((resolve, reject): void => {
      this.pending.set(id, { resolve, reject });
      this.worker.postMessage({ op, id, ...payload }, transfer);
    });
  }

  // ── Vane 接口实现 ──────────────────────────────────────────────────────────

  async open(path = 'vane.db', opts?: OpenOptions): Promise<void> {
    await this.call('open', { path, opts: opts ?? {} });
  }

  async collection(
    name: string,
    schema: Schema,
    opts?: CollectionOptions,
  ): Promise<number> {
    const result = await this.call('collection', { name, schema, opts: opts ?? {} });
    return Number(result);
  }

  async add(col: number, docs: Doc[]): Promise<number> {
    const result = await this.call('add', { col, docs });
    return Number(result);
  }

  async flush(col: number): Promise<void> {
    await this.call('flush', { col });
  }

  async search(col: number, query: SearchQuery): Promise<Hit[]> {
    const result = await this.call('search', { col, query });
    // worker.rs search 返回 Hit[] JSON 字符串，主线程反序列化。
    return typeof result === 'string' ? (JSON.parse(result) as Hit[]) : (result as Hit[]);
  }

  async delete(col: number, ids: string[]): Promise<number> {
    const result = await this.call('delete', { col, ids });
    return Number(result);
  }

  async compact(col: number): Promise<void> {
    await this.call('compact', { col });
  }

  async reindex(col: number): Promise<number> {
    const result = await this.call('reindex', { col });
    return Number(result);
  }

  async export(dest: string): Promise<void> {
    await this.call('export', { dest });
  }

  async readFile(path: string): Promise<Uint8Array> {
    const result = await this.call('readFile', { path });
    return result as Uint8Array;
  }

  async close(): Promise<void> {
    if (this.closed) return;
    try {
      await this.call('close');
    } finally {
      this.closed = true;
      this.worker.terminate();
    }
  }
}

/**
 * 创建 Vane 实例（主线程 API）。
 *
 * 内部：new Worker → postMessage create → 返回 Vane 代理。
 * Worker 内自动：SIMD 探针 → 选 wasm 变体 → init → VaneWorker.create(opts)。
 *
 * @param opts VFS / 词典 / dbPath 选项。未提供 dictData/dictUrl 时自动填 CDN fallback URL。
 * @returns Vane 实例，所有方法返回 Promise。
 *
 * @example
 * ```ts
 * import { createVane } from '@vane-rs/web';
 *
 * const vane = await createVane({ vfs: 'opfs', dbPath: 'vane.db' });
 * await vane.open();
 * const col = await vane.collection('docs', {
 *   fields: [{ name: 'text', type: 'text' }],
 * }, { tokenizer: 'jieba' });
 * await vane.add(col, [{ id: 'd1', text: 'hello' }]);
 * await vane.flush(col);
 * const hits = await vane.search(col, { text: 'hello', topK: 10 });
 * await vane.close();
 * ```
 */
export async function createVane(opts?: VaneWorkerOpts): Promise<Vane> {
  return VaneImpl.create(opts ?? {});
}

// ── 类型 re-export ──────────────────────────────────────────────────────────

export type {
  VaneWorkerOpts,
  VfsKind,
  VectorMetric,
  TokenizerKind,
  SearchMode,
  FusionSpec,
  AutoCommit,
  Schema,
  FieldSchema,
  TextFieldSchema,
  VectorFieldSchema,
  ScalarFieldSchema,
  Doc,
  SearchQuery,
  Hit,
  OpenOptions,
  PersistenceMode,
  CollectionOptions,
  UserDictEntry,
  Vane,
} from './types.js';

// ── SIMD 探针 re-export（§3，高级用户可选）──────────────────────────────────

export { simd128Supported, SIMD128_TEST_MODULE } from './probe.js';
