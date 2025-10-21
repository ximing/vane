# 09-go-cgo-binding：vane-ffi C ABI + Go cgo staticlib + zig cc 交叉 + wazero build tag

> SPEC 引用：§9（FFI C ABI 约定）、§12.1/§12.2（workspace + 目标矩阵）、§4.3（Go cgo 一等公民 + wazero 二等备选）。
> **本计划可后移**（REQUIREMENTS §7 风险 #15；燃尽图告急时优先让位，分词 Must 不让位）。
> 前置依赖：M0 `vane_core::api` 全部 pub API（已核查 git HEAD）。
> M1 README 契约：`crates/vane-ffi` + `bindings/go`。

## Goal

实装 `vane-ffi` C ABI（SPEC §9，cbindgen 生成 `vane.h`），句柄 `uint64_t` + 全局注册表（`std::sync::RwLock<HashMap>`，非 dashmap）。Go cgo 包装 `bindings/go`，zig cc 交叉编译全平台 `.a`。`CGO_ENABLED=0` 清晰报错引导 wazero；`-tags wazero` build tag 切换二等备选（同 API）。

## Architecture

- **vane-ffi**（`crates/vane-ffi/src/lib.rs`，M0 已有空占位）：
  - 全局注册表 `static REGISTRY: OnceLock<RwLock<HashMap<u64, HandleEntry>>>`；`HandleEntry` 枚举 `Db(Arc<Db>)/Collection(Arc<CollectionInner>)/Reindex(ReindexHandle)`。
  - 句柄分配 = `AtomicU64` 自增；`vane_close` 注销。
  - 所有函数返回 `i32`（0=OK，负值=错误码 §10）；详情经 `vane_last_error_message`（thread-local String，`vane_string_free` 释放）。
  - 参数/返回 JSON 序列化（binding 薄壳，§9.2）；`vane_search` arena 一次分配 + `vane_string_free`。
  - 内存铁律（I-7）：宿主传入 buffer 仅借用；C 侧返回 buffer 由 `vane_*_free` 释放。
  - cbindgen `cbindgen.toml` 生成 `bindings/go/vane.h`。
- **bindings/go**：
  - `vane.go`：cgo 包装，`#cgo CFLAGS`/`#cgo LDFLAGS` 链接 `libvane.a`。
  - Go 类型 `VaneDb`/`VaneCollection`/`VaneReindexHandle`，方法与 §4.1 IDL 一一对应。
  - `CGO_ENABLED=0` 时 `#error` 引导 wazero 包（`-tags wazero`）。
  - wazero 变体 `vane_wazero.go`（`//go:build wazero`）：加载 wasm32 产物，同 Go API。
- **交叉编译**（`.github/workflows/release.yml` 扩展，10 计划落地）：
  - zig cc 矩阵：linux-x64/linux-arm64/linux-musl/darwin-arm64/darwin-x64/win32-x64。
  - 产物 `libvane-{version}-{triple}.a` 发布 GitHub Release。

## 涉及文件

- **Create**：
  - `crates/vane-ffi/src/lib.rs`（实装，替换 M0 空占位）
  - `crates/vane-ffi/src/registry.rs`（句柄注册表）
  - `crates/vane-ffi/src/error.rs`（thread-local last_error）
  - `crates/vane-ffi/cbindgen.toml`
  - `crates/vane-ffi/tests/ffi_basic.rs`（Rust 侧 FFI 单元测试，调 C ABI 函数）
  - `bindings/go/vane.go`（cgo 包装）
  - `bindings/go/vane.h`（cbindgen 生成，CI 校验最新）
  - `bindings/go/vane_wazero.go`（`//go:build wazero`）
  - `bindings/go/go.mod`
  - `bindings/go/go_test.go`（Go 行为测试）
- **Modify**：
  - `crates/vane-ffi/Cargo.toml`（增 `cbindgen` build-dependency、`libc` 可选）
  - `Cargo.toml`（workspace 已含 vane-ffi，无需改）

## Interfaces

### Consumes from M0（已核查 git HEAD）

```rust
// crates/vane-core/src/api/db.rs
pub struct Db { /* Arc<DbInner> */ }
impl Db {
    pub fn open(vfs: Arc<dyn Vfs>, path: &str, opts: OpenOptions) -> Result<Self>;
    pub fn collection(&self, name: &str, schema: Schema, opts: CollectionOptions) -> Result<Collection>;
    pub fn collections(&self) -> Vec<String>;
    pub fn export(&self, dest: &str) -> Result<()>;  // M0 占位，M1 保留（SPEC §15 M2）
    pub fn close(&self) -> Result<()>;
}
impl Clone for Db {}

// crates/vane-core/src/api/collection.rs
impl Collection {
    pub fn add(&self, docs: &[Doc]) -> Result<AddReport>;
    pub fn flush(&self) -> Result<()>;
    pub fn search(&self, query: &SearchQuery) -> Result<Vec<Hit>>;
    pub fn delete(&self, ids: &[String]) -> Result<u64>;       // 02 计划实装
    pub fn compact(&self) -> Result<()>;                        // 02 计划实装
    pub fn reindex(&self) -> Result<ReindexHandle>;            // 06 计划实装（签名变更）
}
impl Clone for Collection {}

// crates/vane-core/src/vfs/std_fs.rs
pub struct StdFsVfs;
impl StdFsVfs { pub fn new() -> Self; }
```

### Produces（见 README § 09-go-cgo-binding 契约）

## TDD 任务清单

### Task 1：句柄注册表 + open/close roundtrip

**测试**（`crates/vane-ffi/tests/ffi_basic.rs`）：
```rust
use std::ffi::c_void;
extern "C" {
    fn vane_open(path: *const u8, path_len: usize, opts_json: *const u8, opts_len: usize, out_handle: *mut u64) -> i32;
    fn vane_close(handle: u64) -> i32;
    fn vane_last_error_message(handle: u64) -> *const u8;
    fn vane_string_free(ptr: *mut u8);
}

#[test]
fn ffi_open_close_roundtrip() {
    let path = std::ffi::CString::new("/tmp/vane_ffi_test").unwrap();
    let opts = b"{}";
    let mut handle: u64 = 0;
    unsafe {
        let rc = vane_open(path.as_ptr() as *const u8, path.to_bytes().len(), opts.as_ptr(), opts.len(), &mut handle);
        assert_eq!(rc, 0, "open failed");
        assert_ne!(handle, 0);
        let rc = vane_close(handle);
        assert_eq!(rc, 0);
    }
}

#[test]
fn ffi_close_twice_returns_error() {
    // 句柄注销后使用 = 明确错误非 UB（I-7）
    let path = std::ffi::CString::new("/tmp/vane_ffi_test2").unwrap();
    let mut handle: u64 = 0;
    unsafe {
        vane_open(path.as_ptr() as *const u8, path.to_bytes().len(), b"{}".as_ptr(), 2, &mut handle);
        vane_close(handle);
        let rc = vane_close(handle);  // 二次 close
        assert!(rc < 0);  // 错误码
    }
}
```
验证失败：链接错误（函数未实装）。
最小实现：`registry.rs` 全局 `RwLock<HashMap<u64, HandleEntry>>` + `AtomicU64` 分配；`lib.rs` 实装 `vane_open`（StdFsVfs + Db::open + 注册）、`vane_close`（注销）、`vane_last_error_message`（thread-local）、`vane_string_free`。
commit：`ffi: implement handle registry and open/close`。

### Task 2：collection + add + flush + search

**测试**：
```rust
#[test]
fn ffi_collection_add_search_roundtrip() {
    let (db_h, _tmp) = open_tmp_db();
    let schema = br#"{"fields":[{"name":"body","type":"text"},{"name":"v","type":"vector","dim":4,"metric":"cosine"}]}"#;
    let mut col_h: u64 = 0;
    unsafe {
        let rc = vane_collection(db_h, b"docs\0".as_ptr(), 4, schema.as_ptr(), schema.len(), &mut col_h);
        assert_eq!(rc, 0);
        let docs = br#"[{"id":"d1","text":"hello world","vector":[1.0,0.0,0.0,0.0]}]"#;
        let rc = vane_add(col_h, docs.as_ptr(), docs.len());
        assert_eq!(rc, 0);
        let rc = vane_flush(col_h);
        assert_eq!(rc, 0);
        // search
        let q = br#"{"vector":[1.0,0.0,0.0,0.0],"topK":10,"mode":"vector"}"#;
        let mut arena: *mut u8 = std::ptr::null_mut();
        let mut len: usize = 0;
        let rc = vane_search(col_h, q.as_ptr(), q.len(), &mut arena, &mut len);
        assert_eq!(rc, 0);
        let s = std::slice::from_raw_parts(arena, len);
        let v: serde_json::Value = serde_json::from_slice(s).unwrap();
        assert!(v.as_array().unwrap().len() >= 1);
        vane_string_free(arena);
    }
}
```
最小实现：`vane_collection`（Db::collection + 注册 Collection handle）、`vane_add`（JSON parse → Doc[] → add）、`vane_flush`、`vane_search`（JSON parse → SearchQuery → search → JSON arena）。arena 用 `CString::into_raw` + `vane_string_free` 回收。
commit：`ffi: implement collection/add/flush/search with JSON arena`。

### Task 3：delete + compact + reindex（消费 02/06 产物）

**测试**：
```rust
#[test]
fn ffi_delete_returns_count() {
    let (db_h, col_h, _tmp) = setup_with_docs();
    let ids = br#"["d1"]"#;
    let mut count: u64 = 0;
    unsafe {
        let rc = vane_delete(col_h, ids.as_ptr(), ids.len(), &mut count);
        assert_eq!(rc, 0);
        assert_eq!(count, 1);
    }
}

#[test]
fn ffi_reindex_returns_handle() {
    let (db_h, col_h, _tmp) = setup_with_docs();
    let mut ri_h: u64 = 0;
    unsafe {
        let rc = vane_reindex(col_h, &mut ri_h);
        assert_eq!(rc, 0);
        let mut prog: f32 = 0.0;
        vane_reindex_progress(ri_h, &mut prog);
        vane_reindex_wait(ri_h);
    }
}
```
最小实现：`vane_delete`/`vane_compact`/`vane_reindex`/`vane_reindex_progress`/`vane_reindex_wait`。**依赖 02 计划 delete/compact 实装 + 06 计划 reindex 实装**——本 Task 在 02/06 完成后接入，若 09 后移则此 Task 顺延。
commit：`ffi: wire delete/compact/reindex to core`。

### Task 4：错误码透传 + last_error_message

**测试**：
```rust
#[test]
fn ffi_error_code_propagates() {
    // 传非法 schema → E_SCHEMA (-2)
    let (db_h, _tmp) = open_tmp_db();
    let bad_schema = br#"{"fields":[]}"#;  // 无 vector 字段
    let mut col_h: u64 = 0;
    unsafe {
        let rc = vane_collection(db_h, b"bad\0".as_ptr(), 3, bad_schema.as_ptr(), bad_schema.len(), &mut col_h);
        assert_eq!(rc, -2);  // E_SCHEMA
        let msg = vane_last_error_message(db_h);
        let s = std::ffi::CStr::from_ptr(msg as *const i8).to_str().unwrap();
        assert!(s.contains("E_SCHEMA"));
    }
}
```
最小实现：`error.rs` thread-local `RefCell<String>`；每个 vane_* 函数 catch `VaneError` → 写 last_error + 返回 code。
commit：`ffi: propagate error codes and last_error_message`。

### Task 5：cbindgen 生成 vane.h + Go cgo 包装

**测试**（`bindings/go/go_test.go`）：
```go
package vane

import "testing"

func TestOpenClose(t *testing.T) {
    db, err := Open("/tmp/vane_go_test", nil)
    if err != nil { t.Fatal(err) }
    if err := db.Close(); err != nil { t.Fatal(err) }
}

func TestAddSearch(t *testing.T) {
    db, _ := Open("/tmp/vane_go_test2", nil)
    defer db.Close()
    col, err := db.Collection("docs", Schema{
        Fields: []FieldDef{
            {Name: "body", Type: "text"},
            {Name: "v", Type: "vector", Dim: 4, Metric: "cosine"},
        },
    }, nil)
    if err != nil { t.Fatal(err) }
    col.Add([]Doc{{ID: "d1", Text: "hello", Vector: []float32{1,0,0,0}}})
    col.Flush()
    hits, err := col.Search(SearchQuery{Vector: []float32{1,0,0,0}, TopK: 10, Mode: "vector"})
    if err != nil { t.Fatal(err) }
    if len(hits) < 1 { t.Fatal("no hits") }
}
```
最小实现：`cbindgen.toml` 配置；`build.rs` 调 cbindgen 生成 `vane.h`；`vane.go` cgo 包装。Go 测试需 `CGO_ENABLED=1` + 本地 `libvane.a`（CI 跑，本地可选）。
commit：`go: add cgo bindings and basic tests`。

### Task 6：CGO_ENABLED=0 清晰报错 + wazero build tag

**测试**（`bindings/go/build_tag_test.go`）：
```go
//go:build !cgo
package vane

import "testing"

func TestNoCgoErrorsClearly(t *testing.T) {
    // CGO_ENABLED=0 编译时，vane.go 的 #cgo 阻止编译，
    // 应有清晰错误指向 wazero。此处用 go vet 验证 build tag 隔离。
    // 实际验证在 CI：CGO_ENABLED=0 go build 应失败并提示 -tags wazero。
}
```
最小实现：`vane.go` 顶部 `// +build cgo`；`vane_nocgo_error.go`（`//go:build !cgo`）含 `#error` 等价 Go 编译错误引导 wazero。`vane_wazero.go`（`//go:build wazero`）实装同 API 走 wasm32 产物（M1 可只骨架，wazero 完整实装可后移——标注）。
commit：`go: add CGO_ENABLED=0 error guidance and wazero build tag`。

## 验收标准

- **SPEC §9.1**：句柄 uint64_t + 全局注册表；`vane_close` 后使用 = 明确错误非 UB（I-7，Task 1）。
- **SPEC §9.2**：函数面与 §4.1 一一对应；JSON 序列化；`vane_search` arena 一次 free（Task 2）。
- **SPEC §10**：错误码透传，不吞并/重编（Task 4）。
- **SPEC §4.3**：cgo 一等公民；`CGO_ENABLED=0` 清晰报错引导 wazero；`-tags wazero` 同 API 切换（Task 6）。
- **SPEC §12.2**：Go staticlib 矩阵（zig cc 交叉）——10-ci-m1 跑交叉编译 job。
- **不变量 I-7/I-8**：FFI 内存铁律 + binding 薄壳（行为测试在 core，Go 测试仅验证调用链）。

## 前置依赖

- M0 `vane_core::api`（已合并）。
- 02-tombstone-merge（delete/compact 实装，Task 3 依赖）。
- 06-userdict-reindex（reindex 实装，Task 3 依赖）。
- **可后移**：燃尽图告急时，Task 3 的 reindex 部分可拆出，09 主体（open/collection/add/flush/search）先行。

## Global Constraints

core 禁 std::fs（vane-ffi 可用 std::fs? **否**——vane-ffi 是独立 crate，但 SPEC §13.3 "core 出现 std::fs 即失败" 仅指 vane-core；vane-ffi 调 StdFsVfs 已封装，不直接 std::fs）。并发原语 `std::sync::RwLock`（非 dashmap，注册表）。依赖黑名单（不引 dashmap/parking_lot）。cbindgen 生成 vane.h 进 CI 校验最新（10-ci-m1）。Go embed 词典见 08 计划，本计划不内嵌词典。
