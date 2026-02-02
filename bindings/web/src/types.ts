// Vane Web 端类型定义（src/types.ts）。
//
// ⚠️ 维护红线：所有字段名必须与 crates/vane-wasm/src/worker.rs 的
// parse_worker_opts / parse_schema / parse_search_query / parse_open_opts /
// parse_collection_opts / extract_dict_data / hits_to_json 严格对齐（camelCase）。
// 设计文档 docs/plans/m3/task-1-design.md §6 的草案仅供参考，以 worker.rs 实现为准。

// ── VaneWorkerOpts（worker.rs parse_worker_opts + extract_dict_data）──────────

/** VFS 后端类型。worker.rs parse_worker_opts：默认 "opfs"（OPFS 不可用降级 IDB/memory）。 */
export type VfsKind = 'opfs' | 'idb' | 'memory';

/** 向量距离度量。worker.rs parse_schema：默认 "cosine"。 */
export type VectorMetric = 'cosine' | 'l2' | 'dot';

/** 分词器。worker.rs parse_collection_opts：默认 "standard"（空格/标点分词）。 */
export type TokenizerKind = 'jieba' | 'cjk_bigram' | 'standard';

/** 搜索模式。worker.rs parse_search_query：默认 "auto"（自动选择 vector/text/hybrid）。 */
export type SearchMode = 'hybrid' | 'vector' | 'text' | 'auto';

/** 融合策略。worker.rs parse_search_query：默认 "rrf"。 */
export type FusionSpec = 'rrf' | { linear: { alpha?: number } };

/** autoCommit 配置。worker.rs parse_auto_commit：默认 On{intervalMs:1000, maxDocs:1000}。 */
export type AutoCommit = 'off' | { intervalMs?: number; maxDocs?: number };

/**
 * createVane 工厂选项。
 *
 * - `vfs`：VFS 后端，默认 "opfs"（OPFS 不可用时自动降级 IDB → memory）。
 * - `dbPath`：数据库逻辑路径，默认 "vane.db"（OPFS 模式下也是文件名）。
 * - `dictData`：词典字节（zstd 压缩的 dict.bin），优先于 dictUrl。
 *   传入后以 transferable 零拷贝移交 Worker；**transfer 后主线程不可再访问该 buffer**。
 * - `dictUrl`：词典 CDN fallback URL。未提供 dictData 且未指定 dictUrl 时，
 *   @vane-rs/web 层自动填入 jsdelivr CDN 默认 URL。
 * - `dictSha256`：16 字符 hex（sha256 前 8 字节），用于 worker 内 verify_sha256_prefix 校验。
 */
export interface VaneWorkerOpts {
  vfs?: VfsKind;
  dbPath?: string;
  dictData?: Uint8Array | ArrayBuffer;
  dictUrl?: string;
  dictSha256?: string;
}

// ── Schema（worker.rs parse_schema）──────────────────────────────────────────

/** 文本字段。worker.rs parse_schema type="text" → FieldDef::Text。 */
export interface TextFieldSchema {
  name: string;
  type: 'text';
}

/** 向量字段。worker.rs parse_schema type="vector" → FieldDef::Vector{dim, metric}。 */
export interface VectorFieldSchema {
  name: string;
  type: 'vector';
  /** 向量维度（必填）。 */
  dim: number;
  /** 距离度量，默认 "cosine"。 */
  metric?: VectorMetric;
}

/** 标量字段。worker.rs parse_schema type="scalar" → FieldDef::Scalar{kind}。 */
export interface ScalarFieldSchema {
  name: string;
  type: 'scalar';
  /** 标量类型，默认 "keyword"。 */
  kind?: 'int' | 'float' | 'bool' | 'keyword';
}

/** 字段定义（判别联合：type 决定可选字段）。 */
export type FieldSchema = TextFieldSchema | VectorFieldSchema | ScalarFieldSchema;

/** Schema：字段列表。worker.rs parse_schema fields 数组。 */
export interface Schema {
  fields: FieldSchema[];
}

// ── Doc（worker.rs parse_docs）───────────────────────────────────────────────

/**
 * 文档。worker.rs parse_docs：id 必填，text/vector/meta 可选。
 * meta 值支持 number / boolean / string（映射到 ScalarValue）。
 */
export interface Doc {
  id: string;
  text?: string;
  vector?: number[];
  meta?: Record<string, number | boolean | string>;
}

// ── SearchQuery（worker.rs parse_search_query）───────────────────────────────

/**
 * 搜索查询。worker.rs parse_search_query：text 和 vector 至少提供一个。
 *
 * - `topK`：默认 10。
 * - `mode`：默认 "auto"。
 * - `fusion`：默认 "rrf"。Linear 融合 { linear: { alpha } }，alpha 默认 0.5。
 * - `candidateMultiplier`：默认 3。
 * - `filter`：⚠️ wasm 端不支持（worker.rs 返 VaneError），勿传。
 */
export interface SearchQuery {
  text?: string;
  vector?: number[];
  topK?: number;
  mode?: SearchMode;
  fusion?: FusionSpec;
  candidateMultiplier?: number;
}

// ── Hit（worker.rs hits_to_json）─────────────────────────────────────────────

/**
 * 搜索结果。worker.rs hits_to_json：id + score + fields（存储字段 map，可能为 null）。
 */
export interface Hit {
  id: string;
  score: number;
  fields: Record<string, string> | null;
}

// ── OpenOptions（worker.rs parse_open_opts）──────────────────────────────────

/** 持久化模式。worker.rs parse_open_opts：默认 "persistent"。 */
export type PersistenceMode = 'persistent' | 'best-effort';

/**
 * open() 选项。worker.rs parse_open_opts。
 * - `persistence`：默认 "persistent"。
 * - `autoCommit`：默认 On{intervalMs:1000, maxDocs:1000}。
 * - `pageCacheMb`：页缓存大小（MB），默认 32。
 */
export interface OpenOptions {
  persistence?: PersistenceMode;
  autoCommit?: AutoCommit;
  pageCacheMb?: number;
}

// ── CollectionOptions（worker.rs parse_collection_opts）──────────────────────

/** 用户词典条目。worker.rs parse_collection_opts userDict：字符串或 {term, freq}。 */
export type UserDictEntry = string | { term: string; freq: number };

/**
 * collection() 选项。worker.rs parse_collection_opts。
 * - `tokenizer`：默认 "standard"。jieba 无词典时自动降级 cjk_bigram（不抛错）。
 * - `userDict`：用户自定义词典条目列表。
 * - `autoCommit`：同 OpenOptions.autoCommit。
 */
export interface CollectionOptions {
  tokenizer?: TokenizerKind;
  userDict?: UserDictEntry[];
  autoCommit?: AutoCommit;
}

// ── Vane 接口（主线程 API，封装 Worker postMessage）─────────────────────────

/**
 * Vane 实例接口。createVane() 返回此接口的实现。
 *
 * 所有方法返回 Promise，内部通过 postMessage 路由到 Worker 内的 VaneWorker。
 * close() 后再调用任何方法 reject（I-7 句柄注销）。
 */
export interface Vane {
  /**
   * 打开数据库。
   * @param path 逻辑路径（OPFS 模式下也是文件名），默认 "vane.db"。应与 createVane 的 dbPath 一致。
   * @param opts 打开选项。
   */
  open(path?: string, opts?: OpenOptions): Promise<void>;

  /**
   * 创建或获取 collection。
   * @param name collection 名称（同名的 schema 必须一致）。
   * @param schema 字段定义。
   * @param opts 分词器等选项。
   * @returns collection 句柄（u32）。
   */
  collection(name: string, schema: Schema, opts?: CollectionOptions): Promise<number>;

  /**
   * 追加文档。
   * @returns accepted 数量（可能因 schema 约束少于传入数）。
   */
  add(col: number, docs: Doc[]): Promise<number>;

  /** 刷新缓冲区，持久化段。 */
  flush(col: number): Promise<void>;

  /**
   * 搜索。
   * @returns Hit[]（worker 内 JSON 序列化，主线程反序列化）。
   */
  search(col: number, query: SearchQuery): Promise<Hit[]>;

  /**
   * 删除文档。
   * @returns 已删除数量。
   */
  delete(col: number, ids: string[]): Promise<number>;

  /** 触发段合并。 */
  compact(col: number): Promise<void>;

  /**
   * 触发 reindex（同步执行）。
   * @returns progress（0.0–1.0，1.0 表示已完成）。
   */
  reindex(col: number): Promise<number>;

  /** 导出数据库快照到 VFS 容器内虚拟路径。配合 readFile() 读回字节下载。 */
  export(dest: string): Promise<void>;

  /** 读 VFS 容器内指定虚拟路径的文件字节（配合 export 后下载）。 */
  readFile(path: string): Promise<Uint8Array>;

  /** 关闭 Worker（flush 所有 collection + 注销句柄 + terminate worker 线程）。 */
  close(): Promise<void>;
}
