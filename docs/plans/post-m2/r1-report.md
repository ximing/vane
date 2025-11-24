# post-M2 R1：Go module 路径修正

## 状态

已完成。`github.com/vane/vane` → `github.com/ximing/vane`，与实际仓库 `https://github.com/ximing/vane` 一致。

## 改动（3 处字符串替换）

| 文件 | 位置 | 改动 |
|------|------|------|
| `bindings/go/go.mod` | 第 1 行 | `module github.com/vane/vane/bindings/go` → `module github.com/ximing/vane/bindings/go` |
| `bindings/go/example/main.go` | 第 16 行 | `"github.com/vane/vane/bindings/go"` → `"github.com/ximing/vane/bindings/go"` |
| `bindings/go/example/main.go` | 第 17 行 | `"github.com/vane/vane/bindings/go/dict"` → `"github.com/ximing/vane/bindings/go/dict"` |

未改动其他文件、Go API 或逻辑。

## 自证结果

1. **grep 残留检查**
   ```
   grep -rn 'github.com/vane/vane' bindings/go/
   ```
   退出码 1（无匹配）—— 无残留。

2. **go vet ./...**
   退出码 0 —— import 路径解析通过，module 路径正确。

3. **go build ./...**
   退出码 0 —— 编译成功。
   链接器输出 macOS 版本不匹配的 warning（`libvane_ffi.a` 中部分 object file 构建于 macOS 26.1，链接目标为 15.0），属于已提交静态库的版本元数据问题，与 module 路径无关，不影响构建结果。

## 结论

发版阻断项已解除。Go 用户 `go get github.com/ximing/vane/bindings/go` 现可正确解析到本仓库。
