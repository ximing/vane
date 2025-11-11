# M2-11 Go cgo 绑定——报告

## 概要

实装 vane-ffi C ABI（替换 M0 占位 stub），手写 vane.h，Go cgo 包装 + 词典 go:embed + host demo + wazero build tag 骨架 + CI go-cross matrix。host 平台（darwin/arm64）全链路跑通。

## C ABI 逐函数实装（crates/vane-ffi/src/lib.rs）

| 函数 | 签名 | 实装说明 |
|---|---|---|
| `vane_open` | `(path_ptr, path_len, opts_json, opts_len, out_handle) -> i32` | 构造 StdFsVfs → Db::open → 注册句柄 |
| `vane_collection` | `(db_h, name, schema_json, opts_json, out_handle) -> i32` | 解析 Schema/CollectionOptions JSON → db.collection |
| `vane_add` | `(col_h, docs_json, docs_len) -> i32` | 解析 Doc[] JSON → col.add |
| `vane_flush` | `(col_h) -> i32` | col.flush |
| `vane_search` | `(col_h, query_json, out_arena, out_len) -> i32` | 解析 SearchQuery JSON → col.search → Hit[] JSON arena（vane_string_free 释放） |
| `vane_delete` | `(col_h, ids_json, out_count) -> i32` | 解析 string[] → col.delete → count |
| `vane_compact` | `(col_h) -> i32` | col.compact |
| `vane_reindex` | `(col_h, out_handle) -> i32` | col.reindex → ReindexHandle 句柄 |
| `vane_reindex_progress` | `(h, out_progress) -> i32` | rh.progress |
| `vane_reindex_wait` | `(h) -> i32` | rh.wait |
| `vane_load_dict` | `(h, dict_ptr, dict_len) -> i32` | JiebaDict::load_zstd → Db::set_jieba_dict 注入 |
| `vane_dict_version` | `(out_ptr, out_len) -> i32` | 返回 {version, sha256Prefix} JSON arena |
| `vane_export` | `(db_h, dest_ptr, dest_len) -> i32` | Db::export（M2-12 前返 E_UNSUPPORTED -10） |
| `vane_close` | `(handle) -> i32` | 注销句柄；注销后使用返 E_NOT_FOUND -3（非 UB，I-7） |
| `vane_last_error_message` | `(handle) -> *const u8` | 线程局部错误字符串（不需 free） |
| `vane_string_free` | `(ptr)` | 释放 arena（tracked Layout，I-7） |

### 句柄注册表
- `std::sync::RwLock<HashMap<u64, RegistryEntry>>`（非 dashmap，黑名单合规）
- 全局 AtomicU64 分配句柄（从 1 起）
- RegistryEntry 持 `Option<Arc<Db>> / Option<Arc<Collection>> / Option<ReindexHandle>`
- `vane_close` 移除句柄；lookup 返回 None → E_NOT_FOUND（I-7）

### 内存铁律 I-7
- arena 分配经 `arena_alloc_tracked`：记录 Layout 到全局 HashMap，`vane_string_free` 查 Layout 后 dealloc
- `vane_last_error_message` 返回线程局部 CString 指针，不需 free（随线程消亡）
- 句柄注销后使用 = 明确 E_NOT_FOUND，非 UB

### 错误处理
- 所有函数返回 i32（0=OK，负=错误码 SPEC §10）
- `fail(e: VaneError)` 写入 thread-local 错误描述 + 返回 e.code()
- `vane_last_error_message` 读取线程局部错误

## vane-core 最小改动（支持 FFI 运行时词典注入）

`DbInner.jieba_dict` 从 `Option<Arc<JiebaDict>>` 改为 `RwLock<Option<Arc<JiebaDict>>>`，新增 `Db::set_jieba_dict(&self, dict)` pub 方法。这是 additive 变更（不改现有 pub API 签名，DbInner 是 pub(crate) 内部结构）。FFI `vane_load_dict` 调此方法注入 Go embed 词典。

## cbindgen 方式

**手写 vane.h**（`bindings/go/vane.h`）。原因：
1. cbindgen CLI 本地不可用
2. cbindgen 作为 build-dep 会引入 `clap` → `regex`，与 deny.toml 黑名单冲突（regex wrappers 仅允许 napi-derive-backend / criterion）
3. SPEC 明确允许：「若 cbindgen 不可用，手写 vane.h（与 Rust extern "C" 签名严格一致）并在报告说明」

头文件与 `vane-ffi/src/lib.rs` 的 `#[no_mangle] extern "C"` 签名逐字对齐，含错误码注释 + 内存铁律 I-7 注释。

## Go cgo 包装（bindings/go/vane.go）

- `//go:build !wazero` build tag
- cgo CFLAGS/LDFLAGS 按 GOOS/GOARCH 分发到 `lib/<platform>/libvane_ffi.a`
- Go 类型封装：Db/Collection/ReindexHandle 句柄 + OpenOptions/Schema/Doc/SearchQuery/Hit JSON 结构
- 行为薄壳（I-8）：cgo 仅做参数搬运 + JSON 序列化，无检索逻辑
- arena 内存：`vane_search` 结果 `C.GoBytes` 拷贝后立即 `C.vane_string_free`（I-7）

## Go 词典 embed（bindings/go/dict/）

- `dict.go`（`//go:build !vane_nodict`）：`go:embed dict.bin.gz`（1.41MB < 2MB 门禁）+ `DictBytes()` 解 gzip 返回 zstd 字节 + `DictVersion = "2026.08"`
- `dict_nodict.go`（`//go:build vane_nodict`）：`DictBytes()` 返 `ErrDictUnavailable`，引导降级 CjkBigram
- dict.bin 来源：`crates/vane-dict-zh/data/dict.bin`（zstd 压缩），经 gzip -9 再压缩嵌入
- 三渠道版本一致：dict.bin 同源（vane-dict-zh / Node @vane/dict-zh / Go embed）

## wazero build tag 骨架（bindings/go/wazero/）

- `vane.go`（`//go:build wazero`）：API 对齐 cgo 变体，方法返回 `ErrWazeroNotImplemented`
- `types.go`（`//go:build wazero`）：共享类型定义（与 cgo 变体对齐）
- 实装路径文档化（M2 后续）：vane-core → wasm32-wasi → wazero host 封装
- 性能劣化 2~4 倍（SPEC §4.3 二等备选）

## host demo（bindings/go/example/main.go）

open → LoadDict(jieba) → collection → add → flush → vector search → text search → hybrid search → close

```
[demo] opened db at /var/folders/.../demo.db
[demo] loaded jieba dict version=2026.08
[demo] created collection 'docs'
[demo] added 3 docs
[demo] flushed
[demo] --- vector search ---
  hit: id=a score=1.0000
  hit: id=c score=0.9191
  hit: id=b score=0.0000
[demo] --- text search ---
  hit: id=a score=0.4992
  hit: id=c score=0.4992
[demo] --- hybrid search ---
  hit: id=a score=0.0333
  hit: id=c score=0.0328
  hit: id=b score=0.0161
[demo] done
```

## CI matrix 配置（.github/workflows/ci.yml）

- `go-host` job：ubuntu-latest，cargo build vane-ffi → go build/test/run demo（远程 CI 触发）
- `go-cross` job：4 平台 zig cc 交叉矩阵（x86_64/aarch64-linux-gnu + x86_64/aarch64-apple-darwin），cargo-zigbuild
- 标注：本地 zig 不可用，go-cross 仅远程 CI 触发

## 自证门禁结果

| # | 门禁 | 结果 |
|---|---|---|
| 1 | `cargo test --workspace --all-features` 全绿 | PASS（基线 370+7=377 测试，0 回退） |
| 2 | `cargo test -p vane-ffi` 全绿 | PASS（7 测试：open/close roundtrip, collection+add+flush+search, delete+compact, last_error, export unsupported, dict_version unavailable, thread safety） |
| 3 | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | PASS（clean） |
| 4 | `cargo fmt --all -- --check` | PASS（clean） |
| 5 | `cargo check --target wasm32-unknown-unknown -p vane-core` | PASS（vane-ffi 不影响 core wasm） |
| 6 | `cargo build --release -p vane-ffi` staticlib | PASS（`target/release/libvane_ffi.a` 21MB；复制到 `bindings/go/lib/darwin-arm64/`） |
| 7 | vane.h 头文件 | 手写（`bindings/go/vane.h`），与 extern "C" 签名严格一致 |
| 8 | `cd bindings/go && go build ./...` | PASS（host darwin/arm64） |
| 9 | `cd bindings/go && go test ./...` | PASS（4 vane 测试 + 3 dict 测试） |
| 10 | `cd bindings/go/example && go run main.go` | PASS（open→search→close 端到端） |
| 11 | Go embed dict 增量 <2MB | PASS（dict.bin.gz = 1,477,876 bytes = 1.41MB） |
| 12 | `cargo deny check` | PASS（advisories ok, bans ok, licenses ok, sources ok） |
| 13 | CI workflow go-cross matrix 配置写入 | PASS（ci.yml go-host + go-cross 4 平台 zig cc） |

## 遗留

1. **多平台交叉编译待 CI 触发**：本地 zig 不可用，4 平台 `.a` 交叉编译仅 CI workflow 配置就绪，需远程 CI 触发验证。
2. **wazero 实装骨架**：`bindings/go/wazero/` 仅 API 骨架 + `ErrWazeroNotImplemented`，完整实装（wasm32-wasi 编译 + wazero host 封装）留 M2 后续。
3. **vane_export E_UNSUPPORTED**：M2-12 接入后改为真实导出实装。
4. **CGO_ENABLED=0 错误引导**：`//go:build !wazero` 在 cgo 变体上确保 CGO_ENABLED=0 时编译失败（cgo import C 需 CGO_ENABLED=1），引导用户加 `-tags wazero`。
