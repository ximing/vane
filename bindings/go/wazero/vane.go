// Package vane（wazero 变体）：纯 Go 备选，通过 wazero 运行 vane_core.wasm。
//
// M2-11 wazero build tag 二等备选（SPEC §4.3 / §12.2）：
// - CGO_ENABLED=0 编译时若未加 -tags wazero，cgo 变体编译错误引导（见 vane.go
//   //go:build !wazero 守护）。
// - -tags wazero 编译此包，同 Go API 但内部调 wazero 实例而非 cgo。
// - 性能劣化 2~4 倍（wasm 解释执行），作为无 cgo 环境的降级路径。
//
// 实装路径（M2 后续完善，当前为骨架）：
//  1. vane-core 编译为 wasm32-wasi 模块（cargo build --target wasm32-wasi -p vane-core --lib）
//  2. wazero host 封装：NewRuntime + InstantiateModule 加载 vane_core.wasm
//  3. Go API 对齐：VaneOpen/Collection/Add/... 同 cgo 包同名 API
//  4. 内存桥接：wazero Memory.Read/Write + 导入函数
//
// 当前骨架仅提供类型声明 + ErrWazeroNotImplemented，标注 M2 后续实装。

//go:build wazero

package vane

import (
	"errors"
)

// ErrWazeroNotImplemented wazero 变体尚未实装（M2 后续）。
var ErrWazeroNotImplemented = errors.New("vane: wazero variant not yet implemented (M2 future)")

// Db 占位类型（与 cgo 变体同名 API 对齐）。
type Db struct{}

// Collection 占位类型。
type Collection struct{}

// ReindexHandle 占位类型。
type ReindexHandle struct{}

// Open 占位（wazero 未实装）。
func Open(path string, opts *OpenOptions) (*Db, error) {
	return nil, ErrWazeroNotImplemented
}

// Close 占位。
func (db *Db) Close() error { return ErrWazeroNotImplemented }

// Collection 占位。
func (db *Db) Collection(name string, schema Schema, opts *CollectionOptions) (*Collection, error) {
	return nil, ErrWazeroNotImplemented
}

// LoadDict 占位。
func (db *Db) LoadDict(dictBytes []byte) error { return ErrWazeroNotImplemented }

// Export 占位。
func (db *Db) Export(dest string) error { return ErrWazeroNotImplemented }

// Add 占位。
func (c *Collection) Add(docs []Doc) error { return ErrWazeroNotImplemented }

// Flush 占位。
func (c *Collection) Flush() error { return ErrWazeroNotImplemented }

// Search 占位。
func (c *Collection) Search(query SearchQuery) ([]Hit, error) {
	return nil, ErrWazeroNotImplemented
}

// Delete 占位。
func (c *Collection) Delete(ids []string) (uint64, error) {
	return 0, ErrWazeroNotImplemented
}

// Compact 占位。
func (c *Collection) Compact() error { return ErrWazeroNotImplemented }

// Reindex 占位。
func (c *Collection) Reindex() (*ReindexHandle, error) {
	return nil, ErrWazeroNotImplemented
}

// Progress 占位。
func (rh *ReindexHandle) Progress() (float32, error) {
	return 0, ErrWazeroNotImplemented
}

// Wait 占位。
func (rh *ReindexHandle) Wait() error { return ErrWazeroNotImplemented }

// Close 占位。
func (rh *ReindexHandle) Close() error { return ErrWazeroNotImplemented }

// Close 占位。
func (c *Collection) Close() error { return ErrWazeroNotImplemented }

// DictVersion 占位。
func DictVersion() (string, error) {
	return "", ErrWazeroNotImplemented
}
