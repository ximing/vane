# 08-dict-distribution-go：go:embed dict.bin.gz + vane_nodict tag + DictVersion

> SPEC 引用：§12.3（词典分发 Go 侧）、§4.3（go:embed 内嵌）。
> 前置依赖：05-jieba-lite（dict.bin 格式）；09-go-cgo-binding（Go cgo 绑定 C ABI）。
> M1 README 契约：`bindings/go`。

## Goal

Go 侧 `go:embed dict.bin.gz` 内嵌词典（+1.5MB 换单文件部署体验）；`//go:build vane_nodict` 裁剪 tag 退化 bigram；`vane.DictVersion()` 可查。embed 二进制增量 <2MB（CI 门禁）。三渠道版本哈希一致才发版。

## Architecture

- **`bindings/go/dict/`**：
  - `dict.bin.gz`：zstd/gzip 压缩的 dict.bin（与 Node `@vane/dict-zh` 同源同版本）。
  - `dict.go`（`//go:build !vane_nodict`）：`//go:embed dict.bin.gz` + `LoadDict()` 调 C ABI 加载。
  - `dict_nodict.go`（`//go:build vane_nodict`）：`LoadDict()` 返回 `ErrDictUnavailable`，引导 bigram。
  - `version.go`：`DictVersion() string` 返回 `"2026.08"`。
- **C ABI 对接**：core 暴露 `vane_load_dict(bytes_ptr, len) -> i32` + `vane_dict_version() -> *const u8`（09 计划 FFI 扩展，或 Go 侧直接调 `JiebaDict::load`——但 Go 不能直接调 Rust，须经 C ABI）。

## 涉及文件

- **Create**：
  - `bindings/go/dict/dict.bin.gz`（生成产物）
  - `bindings/go/dict/dict.go`（embed + LoadDict）
  - `bindings/go/dict/dict_nodict.go`（vane_nodict 降级）
  - `bindings/go/dict/version.go`（DictVersion）
  - `bindings/go/dict/dict_test.go`
- **Modify**：
  - `crates/vane-ffi/src/lib.rs`（增 `vane_load_dict` / `vane_dict_version` C ABI 函数——09 计划扩展）
  - `bindings/go/vane.go`（collection 创建时若 tokenizer=jieba 自动调 LoadDict）

## Interfaces

### Consumes from 05-jieba-lite

dict.bin 格式（zstd 压缩 DAT + HMM + 16 字节头）。

### Consumes from 09-go-cgo-binding

```c
// vane.h（09 产出）
int32_t vane_load_dict(uint64_t db_h, const uint8_t* bytes, uintptr_t len);
const uint8_t* vane_dict_version();
```

### Produces（见 README § 08 契约）

## TDD 任务清单

### Task 1：embed + LoadDict + DictVersion

**测试**（`bindings/go/dict/dict_test.go`）：
```go
package dict

import "testing"

func TestLoadDictReturnsVersion(t *testing.T) {
    v := DictVersion()
    if v != "2026.08" {
        t.Fatalf("expected 2026.08, got %s", v)
    }
}

func TestEmbeddedDictNonEmpty(t *testing.T) {
    // dict.bin.gz 非空
    if len(dictBinGz) == 0 { t.Fatal("embedded dict empty") }
}
```
最小实现：`dict.go`：`//go:embed dict.bin.gz` `var dictBinGz []byte`；`LoadDict()` 解压 gzip → 调 `C.vane_load_dict` 传字节 → 返回 error。`version.go`：`func DictVersion() string { return "2026.08" }`。
commit：`go: add embed dict.bin.gz with LoadDict and DictVersion`。

### Task 2：vane_nodict tag 降级

**测试**（`bindings/go/dict/dict_nodict_test.go`，`//go:build vane_nodict`）：
```go
package dict

import "testing"

func TestNoDictReturnsError(t *testing.T) {
    _, err := LoadDict()
    if err == nil { t.Fatal("expected error in vane_nodict mode") }
}
```
最小实现：`dict_nodict.go`（`//go:build vane_nodict`）：`LoadDict()` 返回 `ErrDictUnavailable`。
commit：`go: add vane_nodict build tag fallback`。

### Task 3：collection 自动加载词典

**测试**（`bindings/go/go_test.go` 扩展）：
```go
func TestJiebaCollectionSearch(t *testing.T) {
    db, _ := Open("/tmp/vane_go_jieba", nil)
    defer db.Close()
    col, err := db.Collection("docs", Schema{
        Fields: []FieldDef{
            {Name: "body", Type: "text"},
            {Name: "v", Type: "vector", Dim: 4, Metric: "cosine"},
        },
    }, &CollectionOptions{Tokenizer: "jieba"})
    if err != nil { t.Fatal(err) }
    col.Add([]Doc{{ID: "d1", Text: "我爱北京天安门", Vector: []float32{1,0,0,0}}})
    col.Flush()
    hits, _ := col.Search(SearchQuery{Text: "北京", TopK: 10, Mode: "text"})
    if len(hits) < 1 { t.Fatal("no hits") }
}
```
最小实现：`vane.go` 的 `Collection` 创建时若 `opts.Tokenizer=="jieba"` → 调 `dict.LoadDict()` 注入；失败 → fallback `cjk_bigram` + log.Printf warn。
commit：`go: auto-load jieba dict in collection with fallback`。

### Task 4：embed 体积 <2MB 门禁

**测试**（CI 脚本，10-ci-m1 跑）：
```bash
# bindings/go/dict/dict.bin.gz 体积检查
SIZE=$(stat -c%s bindings/go/dict/dict.bin.gz)
if [ "$SIZE" -gt 2097152 ]; then
  echo "Go embed dict >2MB: $SIZE"; exit 1
fi
```
最小实现：确保 `dict.bin.gz` <2MB（与 Node ≤1.5MB 同源，gzip 后应 <1.5MB，Go embed 门禁更宽松 <2MB）。
commit：`go: assert embed dict <2MB gate`。

### Task 5：三渠道版本哈希一致校验

**测试**（CI 脚本，10-ci-m1 跑）：
```bash
# Node dict.bin sha256
NODE_HASH=$(sha256sum crates/vane-dict-zh/data/dict.bin | cut -d' ' -f1)
# Go dict.bin.gz 解压后 sha256
GO_HASH=$(gzip -dc bindings/go/dict/dict.bin.gz | sha256sum | cut -d' ' -f1)
if [ "$NODE_HASH" != "$GO_HASH" ]; then
  echo "dict hash mismatch: node=$NODE_HASH go=$GO_HASH"; exit 1
fi
```
最小实现：CI release 前校验三渠道（Node/Go/wasm CDN M2）词典版本哈希一致，不一致阻断发版。
commit：`ci: assert dict version hash consistency across channels`。

## 验收标准

- **SPEC §12.3**：go:embed dict.bin.gz；`//go:build vane_nodict` tag；`vane.DictVersion()`；embed <2MB。
- **SPEC §4.3**：embed 天然钉版；CGO_ENABLED=0 + vane_nodict 退化 bigram。
- **SPEC §13.2-3**：Go embed 增量 <2MB（Task 4）。
- **三渠道一致**：Node/Go 词典版本哈希一致才发版（Task 5）。

## 前置依赖

- 05-jieba-lite（dict.bin 格式）。
- 09-go-cgo-binding（C ABI `vane_load_dict`/`vane_dict_version`，Task 1 依赖——若 09 后移，08 的 C ABI 对接部分顺延，但 embed + DictVersion + 体积门禁可先行）。

## Global Constraints

词典永不进 wasm（Go embed 是 Go 侧，不影响 core wasm32）；vane_nodict tag 明确退化；embed <2MB CI 门禁；三渠道版本哈希一致。
