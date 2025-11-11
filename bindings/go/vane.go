// Package vane provides Go bindings for the Vane hybrid search engine
// (vector + BM25) via cgo linking to the native Rust staticlib (libvane_ffi.a).
//
// M2-11：C ABI 薄壳（I-8）——cgo 仅做参数搬运，无检索逻辑，行为测试在 core。
//
// 构建前提：cargo build --release -p vane-ffi 产出 target/release/libvane_ffi.a
// 编译：CGO_ENABLED=1 go build ./...
//
//go:build !wazero

package vane

/*
#cgo CFLAGS: -I${SRCDIR}
#cgo darwin,arm64 LDFLAGS: -L${SRCDIR}/lib/darwin-arm64 -lvane_ffi -lm
#cgo darwin,amd64 LDFLAGS: -L${SRCDIR}/lib/darwin-amd64 -lvane_ffi -lm
#cgo linux,arm64  LDFLAGS: -L${SRCDIR}/lib/linux-arm64 -lvane_ffi -lm
#cgo linux,amd64  LDFLAGS: -L${SRCDIR}/lib/linux-amd64 -lvane_ffi -lm
#cgo windows,amd64 LDFLAGS: -L${SRCDIR}/lib/windows-amd64 -lvane_ffi -lm

#include <stdlib.h>
#include "vane.h"
*/
import "C"

import (
	"encoding/json"
	"fmt"
	"unsafe"
)

// 错误码（SPEC §10）
const (
	OK                = 0
	EIO               = -1
	ESchema           = -2
	ENotFound         = -3
	ECorrupt          = -4
	EVersion          = -5
	ETokenizerMismatch = -6
	EDictTooLarge     = -7
	EDictUnavailable  = -8
	EBusy             = -9
	EUnsupported      = -10
	EInvalidArg       = -11
)

// VaneError 包装 C ABI 返回的错误码 + 描述。
type VaneError struct {
	Code    int32
	Message string
}

func (e *VaneError) Error() string {
	return fmt.Sprintf("vane error %d: %s", e.Code, e.Message)
}

// Db 句柄。
type Db struct {
	handle uint64
}

// Collection 句柄。
type Collection struct {
	handle uint64
}

// ReindexHandle 句柄。
type ReindexHandle struct {
	handle uint64
}

// OpenOptions 对应 core OpenOptions（JSON 序列化给 C ABI）。
type OpenOptions struct {
	Persistence string         `json:"persistence,omitempty"`
	AutoCommit  interface{}    `json:"autoCommit,omitempty"`
	PageCacheMB uint32         `json:"pageCacheMb,omitempty"`
}

// CollectionOptions 对应 core CollectionOptions。
type CollectionOptions struct {
	Tokenizer string        `json:"tokenizer,omitempty"`
	UserDict  []UserDictEntry `json:"userDict,omitempty"`
	AutoCommit interface{}   `json:"autoCommit,omitempty"`
}

// UserDictEntry 对应 core UserDictEntry。
type UserDictEntry struct {
	Term string `json:"term"`
	Freq uint32 `json:"freq"`
}

// SchemaField 对应 core FieldDef。
type SchemaField struct {
	Name   string `json:"name"`
	Type   string `json:"type"`   // "text" | "vector" | "scalar"
	Dim    uint32 `json:"dim,omitempty"`
	Metric string `json:"metric,omitempty"` // "cosine" | "l2" | "dot"
	Kind   string `json:"kind,omitempty"`   // "int" | "float" | "bool" | "keyword"
}

// Schema 对应 core Schema。
type Schema struct {
	Fields []SchemaField `json:"fields"`
}

// Doc 对应 core Doc。
type Doc struct {
	ID     string                 `json:"id"`
	Text   string                 `json:"text,omitempty"`
	Vector []float32              `json:"vector,omitempty"`
	Meta   map[string]interface{} `json:"meta,omitempty"`
}

// SearchQuery 对应 core SearchQuery。
type SearchQuery struct {
	Text               string      `json:"text,omitempty"`
	Vector             []float32   `json:"vector,omitempty"`
	TopK               uint32      `json:"topK"`
	Mode               string      `json:"mode,omitempty"`
	Fusion             interface{} `json:"fusion,omitempty"`
	CandidateMultiplier uint32     `json:"candidateMultiplier,omitempty"`
}

// Hit 对应 core Hit。
type Hit struct {
	ID     string            `json:"id"`
	Score  float32           `json:"score"`
	Fields map[string]string `json:"fields,omitempty"`
}

// cBytes 把 Go []byte 转为 C 指针+长度（不拷贝，调用期间指针有效）。
func cBytes(b []byte) (*C.uint8_t, C.size_t) {
	if len(b) == 0 {
		return nil, 0
	}
	return (*C.uint8_t)(unsafe.Pointer(&b[0])), C.size_t(len(b))
}

// cStr 把 Go string 转为 C 指针+长度。
func cStr(s string) (*C.uint8_t, C.size_t) {
	return cBytes([]byte(s))
}

// checkError 把非零 rc 转为 VaneError。
func checkError(handle uint64, rc C.int32_t) error {
	if rc == 0 {
		return nil
	}
	msg := lastErrorMessage(handle)
	return &VaneError{Code: int32(rc), Message: msg}
}

func lastErrorMessage(handle uint64) string {
	ptr := C.vane_last_error_message(C.uint64_t(handle))
	if ptr == nil {
		return ""
	}
	// 线程局部 C 字符串，不需 free。用 C.GoString 读取。
	return C.GoString((*C.char)(unsafe.Pointer(ptr)))
}

// Open 打开数据库。
func Open(path string, opts *OpenOptions) (*Db, error) {
	var optsJSON []byte
	if opts != nil {
		b, err := json.Marshal(opts)
		if err != nil {
			return nil, fmt.Errorf("marshal opts: %w", err)
		}
		optsJSON = b
	}
	var handle C.uint64_t
	p, pl := cStr(path)
	op, ol := cBytes(optsJSON)
	rc := C.vane_open(p, pl, op, ol, &handle)
	if rc != 0 {
		return nil, checkError(0, rc)
	}
	return &Db{handle: uint64(handle)}, nil
}

// Collection 创建或获取 collection。
func (db *Db) Collection(name string, schema Schema, opts *CollectionOptions) (*Collection, error) {
	schemaJSON, err := json.Marshal(schema)
	if err != nil {
		return nil, fmt.Errorf("marshal schema: %w", err)
	}
	var optsJSON []byte
	if opts != nil {
		b, err := json.Marshal(opts)
		if err != nil {
			return nil, fmt.Errorf("marshal opts: %w", err)
		}
		optsJSON = b
	}
	var handle C.uint64_t
	np, nl := cStr(name)
	sp, sl := cBytes(schemaJSON)
	op, ol := cBytes(optsJSON)
	rc := C.vane_collection(C.uint64_t(db.handle), np, nl, sp, sl, op, ol, &handle)
	if rc != 0 {
		return nil, checkError(db.handle, rc)
	}
	return &Collection{handle: uint64(handle)}, nil
}

// Export 导出快照（M2-12 接入前返 E_UNSUPPORTED）。
func (db *Db) Export(dest string) error {
	dp, dl := cStr(dest)
	rc := C.vane_export(C.uint64_t(db.handle), dp, dl)
	return checkError(db.handle, rc)
}

// LoadDict 加载 jieba 词典（zstd 压缩 dict.bin 字节）。
func (db *Db) LoadDict(dictBytes []byte) error {
	dp, dl := cBytes(dictBytes)
	rc := C.vane_load_dict(C.uint64_t(db.handle), dp, dl)
	return checkError(db.handle, rc)
}

// Close 关闭 Db 句柄。
func (db *Db) Close() error {
	rc := C.vane_close(C.uint64_t(db.handle))
	return checkError(db.handle, rc)
}

// Add 追加文档。
func (c *Collection) Add(docs []Doc) error {
	b, err := json.Marshal(docs)
	if err != nil {
		return fmt.Errorf("marshal docs: %w", err)
	}
	dp, dl := cBytes(b)
	rc := C.vane_add(C.uint64_t(c.handle), dp, dl)
	return checkError(c.handle, rc)
}

// Flush 刷新缓冲区。
func (c *Collection) Flush() error {
	rc := C.vane_flush(C.uint64_t(c.handle))
	return checkError(c.handle, rc)
}

// Search 搜索。
func (c *Collection) Search(query SearchQuery) ([]Hit, error) {
	b, err := json.Marshal(query)
	if err != nil {
		return nil, fmt.Errorf("marshal query: %w", err)
	}
	var arena *C.uint8_t
	var arenaLen C.size_t
	qp, ql := cBytes(b)
	rc := C.vane_search(C.uint64_t(c.handle), qp, ql, &arena, &arenaLen)
	if rc != 0 {
		return nil, checkError(c.handle, rc)
	}
	if arena == nil || arenaLen == 0 {
		return []Hit{}, nil
	}
	// 拷贝到 Go 内存后立即释放 C arena（I-7：arena 一次 free）。
	goBytes := C.GoBytes(unsafe.Pointer(arena), C.int(arenaLen))
	C.vane_string_free(arena)
	var hits []Hit
	if err := json.Unmarshal(goBytes, &hits); err != nil {
		return nil, fmt.Errorf("unmarshal hits: %w", err)
	}
	return hits, nil
}

// Delete 删除文档。
func (c *Collection) Delete(ids []string) (uint64, error) {
	b, err := json.Marshal(ids)
	if err != nil {
		return 0, fmt.Errorf("marshal ids: %w", err)
	}
	var count C.uint64_t
	ip, il := cBytes(b)
	rc := C.vane_delete(C.uint64_t(c.handle), ip, il, &count)
	if rc != 0 {
		return 0, checkError(c.handle, rc)
	}
	return uint64(count), nil
}

// Compact 段合并。
func (c *Collection) Compact() error {
	rc := C.vane_compact(C.uint64_t(c.handle))
	return checkError(c.handle, rc)
}

// Reindex 触发 reindex。
func (c *Collection) Reindex() (*ReindexHandle, error) {
	var handle C.uint64_t
	rc := C.vane_reindex(C.uint64_t(c.handle), &handle)
	if rc != 0 {
		return nil, checkError(c.handle, rc)
	}
	return &ReindexHandle{handle: uint64(handle)}, nil
}

// Progress 查询 reindex 进度。
func (rh *ReindexHandle) Progress() (float32, error) {
	var p C.float
	rc := C.vane_reindex_progress(C.uint64_t(rh.handle), &p)
	if rc != 0 {
		return 0, checkError(rh.handle, rc)
	}
	return float32(p), nil
}

// Wait 阻塞等待 reindex 完成。
func (rh *ReindexHandle) Wait() error {
	rc := C.vane_reindex_wait(C.uint64_t(rh.handle))
	return checkError(rh.handle, rc)
}

// Close 关闭 ReindexHandle 句柄。
func (rh *ReindexHandle) Close() error {
	rc := C.vane_close(C.uint64_t(rh.handle))
	return checkError(rh.handle, rc)
}

// Close 关闭 Collection 句柄。
func (c *Collection) Close() error {
	rc := C.vane_close(C.uint64_t(c.handle))
	return checkError(c.handle, rc)
}

// DictVersion 查询已加载词典版本信息（JSON: {"version":"...","sha256Prefix":"..."}）。
// 未加载词典返 E_DICT_UNAVAILABLE。
func DictVersion() (string, error) {
	var arena *C.uint8_t
	var arenaLen C.size_t
	rc := C.vane_dict_version(&arena, &arenaLen)
	if rc != 0 {
		return "", checkError(0, rc)
	}
	if arena == nil || arenaLen == 0 {
		return "", nil
	}
	goBytes := C.GoBytes(unsafe.Pointer(arena), C.int(arenaLen))
	C.vane_string_free(arena)
	return string(goBytes), nil
}
