// Package vane（wazero 变体）共享类型定义（与 cgo 变体对齐）。
//
//go:build wazero

package vane

// OpenOptions 对应 core OpenOptions。
type OpenOptions struct {
	Persistence string      `json:"persistence,omitempty"`
	AutoCommit  interface{} `json:"autoCommit,omitempty"`
	PageCacheMB uint32      `json:"pageCacheMb,omitempty"`
}

// CollectionOptions 对应 core CollectionOptions。
type CollectionOptions struct {
	Tokenizer  string          `json:"tokenizer,omitempty"`
	UserDict   []UserDictEntry `json:"userDict,omitempty"`
	AutoCommit interface{}     `json:"autoCommit,omitempty"`
}

// UserDictEntry 对应 core UserDictEntry。
type UserDictEntry struct {
	Term string `json:"term"`
	Freq uint32 `json:"freq"`
}

// SchemaField 对应 core FieldDef。
type SchemaField struct {
	Name   string `json:"name"`
	Type   string `json:"type"`
	Dim    uint32 `json:"dim,omitempty"`
	Metric string `json:"metric,omitempty"`
	Kind   string `json:"kind,omitempty"`
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
	Text                string      `json:"text,omitempty"`
	Vector              []float32   `json:"vector,omitempty"`
	TopK                uint32      `json:"topK"`
	Mode                string      `json:"mode,omitempty"`
	Fusion              interface{} `json:"fusion,omitempty"`
	CandidateMultiplier uint32      `json:"candidateMultiplier,omitempty"`
}

// Hit 对应 core Hit。
type Hit struct {
	ID     string            `json:"id"`
	Score  float32           `json:"score"`
	Fields map[string]string `json:"fields,omitempty"`
}

// VaneError 占位（wazero 未实装）。
type VaneError struct {
	Code    int32
	Message string
}

func (e *VaneError) Error() string {
	return e.Message
}
