//go:build !vane_nodict

package dict

import "testing"

func TestDictBytesNonEmpty(t *testing.T) {
	b, err := DictBytes()
	if err != nil {
		t.Fatalf("DictBytes: %v", err)
	}
	if len(b) == 0 {
		t.Fatal("expected non-empty dict bytes")
	}
	// dict.bin 经 zstd 压缩，magic bytes 0x28 0xB5 0x2F 0xFD
	if len(b) < 4 {
		t.Fatal("dict bytes too short")
	}
	if b[0] != 0x28 || b[1] != 0xB5 || b[2] != 0x2F || b[3] != 0xFD {
		t.Errorf("expected zstd magic (28 B5 2F FD), got %02x %02x %02x %02x",
			b[0], b[1], b[2], b[3])
	}
}

func TestDictVersion(t *testing.T) {
	if DictVersion != "2026.08" {
		t.Errorf("expected 2026.08, got %s", DictVersion)
	}
}

func TestDictGzSize(t *testing.T) {
	// SPEC §12.3：Go embed dict 增量 <2MB
	if len(dictBinGz) >= 2*1024*1024 {
		t.Errorf("dict.bin.gz size %d >= 2MB limit", len(dictBinGz))
	}
	t.Logf("dict.bin.gz size: %d bytes (%.2f MB)", len(dictBinGz), float64(len(dictBinGz))/1024/1024)
}
