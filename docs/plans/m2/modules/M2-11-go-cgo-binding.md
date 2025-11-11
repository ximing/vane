# M2-11 Go cgo 绑定（vane-ffi C ABI 实装）

## 1. 目标
实装 `vane-ffi` C ABI（当前 M0 占位 stub `crates/vane-ffi/src/lib.rs:1`：`// M0 占位；FFI 实现见 M1 计划。`），落实 M1 README §09 契约（句柄注册表 `std::sync::RwLock<HashMap>`，非 dashmap），cbindgen 生成 `vane.h`，Go cgo staticlib + zig cc 交叉编译全平台 `.a`，wazero build tag 二等备选（SPEC §9/§12.2/§12.3，REQUIREMENTS §4.3，M1 按约后移至 M2）。

SPEC 节号：§9（C ABI 约定 + 函数面）、§12.2（Go staticlib 目标矩阵）、§12.3（Go 词典分发 go:embed）、§4.1（IDL 6 动词+4 管理）。

## 2. 涉及文件
- **Modify** `crates/vane-ffi/src/lib.rs`（当前占位 stub）：实装全部 C ABI 函数（M1 README §09 契约逐字落实）。
- **Modify** `crates/vane-ffi/Cargo.toml`：增 `cbindgen = { version = "0.27", optional = true }`（build-dep）；`[build-dependencies] cbindgen = "0.27"`；`[features] default = []`；`crate-type = ["cdylib", "staticlib", "rlib"]`（已存在）。
- **Create** `crates/vane-ffi/build.rs`：cbindgen 生成 `bindings/go/vane.h`。
- **Create** `crates/vane-ffi/cbindgen.toml`：cbindgen 配置（language="c", include_guard, usize usize）。
- **Modify** `crates/vane-ffi/src/lib.rs`：句柄注册表 `std::sync::RwLock<HashMap<u64, Arc<...>>>`（Db/Collection/ReindexHandle 三类句柄）；`vane_last_error_message` 线程局部错误；arena 分配/释放（`vane_search` out_arena）。
- **Create** `bindings/go/vane.go`：cgo 包装（`#cgo` + `vane.h` + Go 类型封装）。
- **Create** `bindings/go/dict/dict.go`：M1 README §08 契约（`go:embed dict.bin.gz` + `LoadDict()` + `vane_nodict` tag）—— M2-11 落地（M1 后移）。
- **Modify** `.github/workflows/release.yml` 或 `ci.yml`：Go staticlib matrix（zig cc 交叉，同 Node 矩阵）+ `CGO_ENABLED=0` 编译错误引导 wazero。
- **Create** `bindings/go/wazero/`：wazero build tag 二等备选（`-tags wazero`，同 Go API 切换）。**wazero 形态实现路径**（reviewer B-M4）：
  1. vane-core 编译为 wasm32-wasi 模块（`cargo build --target wasm32-wasi -p vane-core --lib`，产出 `vane_core.wasm`，与 cgo staticlib 形态完全不同）。
  2. Go wazero host 封装 `bindings/go/wazero/runtime.go`：`wazero.NewRuntime` + `InstantiateModule` 加载 `vane_core.wasm` + 导出 Go 函数桥接 wasm 内存（`Memory.Read/Write` + 导入函数）。
  3. Go API 对齐：`bindings/go/wazero/vane.go` 提供 `VaneOpen/Collection/Add/...` 同 cgo 包同名 API，内部调 wazero 实例而非 cgo。
  4. build tag 切换：`-tags wazero` 编译 `wazero/` 包，否则编译 cgo `vane.go`（`//go:build !wazero` / `//go:build wazero`）。
  5. 参考 M1 README §09 wazero 契约（性能劣化 2~4 倍，二等备选）。

## 3. 接口契约
### Consumes from
- M0/M1 `vane_core::api::{Db, Collection, ReindexHandle}`（全部 pub API）：
  - `Db::open(vfs: Arc<dyn Vfs>, path: &str, opts: OpenOptions)`（`api/db.rs:35`，首参 `vfs: Arc<dyn Vfs>`，reviewer A-I4；vane-ffi C ABI `vane_open` 内部构造 `StdFsVfs` 再传 `Db::open`）、`Db::collection(name, schema, opts)`、`Db::collections()`、`Db::export(dest)`（M2-12 接入）、`Db::close()`。
  - `Collection::{add, flush, search, delete, compact, reindex}`。
  - `ReindexHandle::{progress, wait}`（`api/reindex.rs:64,69`）。
- M1 `JiebaDict::load`（`tokenizer/jieba/dict.rs:46`）—— `vane_load_dict` 接入。
- M2-12 `Db::export` 实装（`vane_export` 接入）。

### Produces for
M1 README §09 契约逐字落实（见 README M2-11 节函数清单）：
```rust
pub fn vane_open(path_ptr: *const u8, path_len: usize, opts_json: *const u8, opts_len: usize, out_handle: *mut u64) -> i32;
pub fn vane_collection(db_h: u64, name: *const u8, name_len: usize, schema_json: *const u8, schema_len: usize, opts_json: *const u8, opts_len: usize, out_handle: *mut u64) -> i32;
pub fn vane_add(col_h: u64, docs_json: *const u8, docs_len: usize) -> i32;
pub fn vane_flush(col_h: u64) -> i32;
pub fn vane_search(col_h: u64, query_json: *const u8, query_len: usize, out_arena: *mut *mut u8, out_len: *mut usize) -> i32;
pub fn vane_delete(col_h: u64, ids_json: *const u8, ids_len: usize, out_count: *mut u64) -> i32;
pub fn vane_compact(col_h: u64) -> i32;
pub fn vane_reindex(col_h: u64, out_handle: *mut u64) -> i32;
pub fn vane_reindex_progress(h: u64, out_progress: *mut f32) -> i32;
pub fn vane_reindex_wait(h: u64) -> i32;
pub fn vane_load_dict(h: u64, dict_ptr: *const u8, dict_len: usize) -> i32;
pub fn vane_dict_version(out_ptr: *mut *mut u8, out_len: *mut usize) -> i32;
pub fn vane_export(db_h: u64, dest_ptr: *const u8, dest_len: usize) -> i32;  // M2-12 接入
pub fn vane_close(handle: u64) -> i32;
pub fn vane_last_error_message(handle: u64) -> *const u8;
pub fn vane_string_free(ptr: *mut u8);
```
下游：`bindings/go` cgo 包装 + Go 词典分发（M1 README §08 落地）。

### 句柄注册表（M1 README §09 + S2 修订）
- `std::sync::RwLock<HashMap<u64, Arc<Db>>>`、`HashMap<u64, Arc<Collection>>`、`HashMap<u64, ReindexHandle>`。
- 句柄 u64 由全局原子计数器分配。
- `vane_close(h)` 注销（remove）；注销后使用 = `Err(NotFound)` 明确错误，非 UB（I-7）。

## 4. TDD 测试清单
1. **vane_open + vane_close**：open 返回 handle>0；close 返回 0；close 后再用该 handle 调任何函数返 `E_NOT_FOUND`（-3，I-7）。
2. **vane_collection**：open → collection(schema) 返回 col handle；同名同 schema 幂等返既有。
3. **vane_add + vane_flush**：add JSON docs → flush → 返回 0。
4. **vane_search**：add+flush → search → out_arena 非 null，out_len>0，JSON 解析为 Hit[]。
5. **vane_search arena free**：`vane_string_free(out_arena)` 后内存释放（valgrind/dhat 无泄漏，I-7）。
6. **vane_delete**：delete ids → out_count 正确。
7. **vane_compact**：compact 返回 0。
8. **vane_reindex + progress + wait**：reindex 返回 handle；progress 0..1；wait 返回 0。
9. **vane_load_dict**：load dict.bin → jieba collection 可用（中文分词生效）。
10. **vane_dict_version**：返回词典日历版本 + sha256 前缀（JSON）。
11. **vane_export**（M2-12 接入后）：export → dest 文件存在；返 0（M2-12 前返 E_UNSUPPORTED -10）。
12. **vane_last_error_message**：失败后调用返回错误描述 C 字符串；`vane_string_free` 释放。
13. **句柄注册表线程安全**：多线程并发 open/close/search，RwLock 无死锁/UB。
14. **错误码透传**：core `Err(VaneError::Schema)` → 返回 -2；`Err(Busy)` → -9（SPEC §10 透传，I-8）。
15. **cbindgen 生成 vane.h**：`build.rs` 产出 `bindings/go/vane.h`，C 编译通过。
16. **Go cgo 包装**：`bindings/go/vane.go` 调用 `VaneOpen/Collection/Add/...`，Go 行为测试与 core 等价（I-8 薄壳）。
17. **Go 词典 go:embed**：`bindings/go/dict/dict.go` `LoadDict()` 解压 dict.bin.gz → `vane_load_dict`；`vane_nodict` tag 返 ErrDictUnavailable。
18. **zig cc 交叉矩阵**：`.a` 产物 x86_64-linux-gnu/aarch64-apple-darwin/x86_64-apple-darwin/x86_64-pc-windows-msvc 全部编译通过。
19. **CGO_ENABLED=0 错误引导**：无 cgo 时编译错误指向 wazero 包（不静默降级，REQUIREMENTS §4.3）。
20. **wazero build tag**：`-tags wazero` 切换 wazero 形态，同 Go API（二等备选，性能劣化 2~4 倍可接受）。

## 5. 验收标准
- 全部 20 测试绿（Rust unit + Go 行为 + 交叉矩阵 CI）。
- cbindgen 生成 `vane.h` C 编译通过。
- Go staticlib 4 平台 `.a` 编译通过（zig cc）。
- 句柄注销后使用 = E_NOT_FOUND（非 UB，I-7）。
- arena 一次 free（无泄漏，I-7）。
- `cargo deny check` ok（cbindgen 不进运行时；无黑名单依赖）。
- clippy clean。

## 6. 前置依赖
- M1 既有（vane-ffi 占位 stub 存在）。
- M2-12 协同（`vane_export` 接入；可先返 E_UNSUPPORTED，M2-12 后接入）。

## 7. 不变量覆盖
- **I-7 FFI 内存铁律**：句柄注销后使用=明确错误非 UB；arena 一次 free；谁分配谁释放。测试 1+5+13 守护。
- **I-8 binding 薄壳**：cgo 无检索逻辑，行为测试在 core。测试 16 守护。
- **§9 C ABI 约定**：句柄 u64 + RwLock<HashMap>（非 dashmap，S2 修订）；错误 i32 + last_error_message；JSON 序列化。测试 1-14 守护。
- **§12.2 Go staticlib 矩阵**：测试 18 守护。
- **§4.3 wazero 二等备选**：测试 19+20 守护。
- **黑名单**：无 dashmap（用 std::sync::RwLock）；无 parking_lot。测试 13 守护。
