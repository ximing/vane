// Package dict provides the embedded jieba Chinese dictionary for Go bindings.
//
// M2-11（M1 README §08 落地）：go:embed dict.bin.gz + LoadDict() + vane_nodict tag。
// 词典数据来自 crates/vane-dict-zh/data/dict.bin（zstd 压缩），经 gzip 再压缩嵌入。
// Go 侧 LoadDict 解 gzip → 传 zstd 字节给 C ABI vane_load_dict（core JiebaDict::load_zstd）。
//
// 体积门禁：gzip 后 <2MB（SPEC §12.3）。
// 三渠道版本一致性：dict.bin 同源 → vane-dict-zh / Node @vane/dict-zh / Go embed。

//go:build !vane_nodict

package dict

import (
	"bytes"
	"compress/gzip"
	_ "embed"
	"fmt"
	"io"
)

//go:embed dict.bin.gz
var dictBinGz []byte

// DictVersion 词典日历版本（YYYY.MM），与 vane-dict-zh DICT_VERSION 一致。
// 编译期常量，不依赖 C ABI（vane_nodict 也可用）。
const DictVersion = "2026.08"

// DictBytes 解压 gzip，返回 zstd 压缩的 dict.bin 字节（传给 vane_load_dict）。
func DictBytes() ([]byte, error) {
	gz, err := gzip.NewReader(bytes.NewReader(dictBinGz))
	if err != nil {
		return nil, fmt.Errorf("dict: gzip reader: %w", err)
	}
	defer gz.Close()
	b, err := io.ReadAll(gz)
	if err != nil {
		return nil, fmt.Errorf("dict: gunzip: %w", err)
	}
	return b, nil
}
