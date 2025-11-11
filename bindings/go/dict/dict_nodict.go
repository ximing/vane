// Package dict（vane_nodict 变体）：无嵌入词典。
//
// vane_nodict build tag：不嵌入 dict.bin.gz，DictBytes 返回 ErrDictUnavailable。
// 引导调用方降级 CjkBigram 分词器（SPEC §13.2-2 ④）。
// 编译：go build -tags vane_nodict ./...

//go:build vane_nodict

package dict

import (
	"errors"
)

// DictVersion 词典日历版本（vane_nodict 下仍返回版本字符串供诊断）。
const DictVersion = "2026.08"

// ErrDictUnavailable 无嵌入词典时返回。
var ErrDictUnavailable = errors.New("dict: no embedded dictionary (built with -tags vane_nodict)")

// DictBytes 在 vane_nodict 构建下返回 ErrDictUnavailable。
func DictBytes() ([]byte, error) {
	return nil, ErrDictUnavailable
}
