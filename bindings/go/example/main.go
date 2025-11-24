// Vane Go cgo 端到端 demo（M2-11）：
// open → collection → add → flush → search → close
//
// 前提：cargo build --release -p vane-ffi，staticlib 已复制到 bindings/go/lib/<platform>/。
//
//go:build !wazero

package main

import (
	"fmt"
	"log"
	"os"
	"path/filepath"

	"github.com/ximing/vane/bindings/go"
	"github.com/ximing/vane/bindings/go/dict"
)

func main() {
	// 临时数据库目录
	tmpDir, err := os.MkdirTemp("", "vane-go-demo-*")
	if err != nil {
		log.Fatalf("mkdir temp: %v", err)
	}
	defer os.RemoveAll(tmpDir)

	dbPath := filepath.Join(tmpDir, "demo.db")

	// 1. Open
	db, err := vane.Open(dbPath, nil)
	if err != nil {
		log.Fatalf("Open: %v", err)
	}
	defer db.Close()
	fmt.Println("[demo] opened db at", dbPath)

	// 2. LoadDict（jieba 中文分词）
	dictBytes, err := dict.DictBytes()
	if err != nil {
		log.Printf("[demo] dict unavailable: %v (will use standard tokenizer)", err)
	} else if err := db.LoadDict(dictBytes); err != nil {
		log.Printf("[demo] LoadDict failed: %v (will use standard tokenizer)", err)
	} else {
		fmt.Printf("[demo] loaded jieba dict version=%s\n", dict.DictVersion)
	}

	// 3. Collection
	schema := vane.Schema{
		Fields: []vane.SchemaField{
			{Name: "vec", Type: "vector", Dim: 4, Metric: "cosine"},
			{Name: "body", Type: "text"},
		},
	}
	colOpts := &vane.CollectionOptions{Tokenizer: "jieba"}
	col, err := db.Collection("docs", schema, colOpts)
	if err != nil {
		// jieba 词典加载失败会降级，但 collection 创建不应失败
		// 若失败则尝试 standard tokenizer
		log.Printf("[demo] jieba collection failed: %v, retry with standard", err)
		colOpts = &vane.CollectionOptions{Tokenizer: "standard"}
		col, err = db.Collection("docs", schema, colOpts)
		if err != nil {
			log.Fatalf("Collection: %v", err)
		}
	}
	defer col.Close()
	fmt.Println("[demo] created collection 'docs'")

	// 4. Add
	docs := []vane.Doc{
		{ID: "a", Text: "hello world", Vector: []float32{1.0, 0.0, 0.0, 0.0}},
		{ID: "b", Text: "foo bar baz", Vector: []float32{0.0, 1.0, 0.0, 0.0}},
		{ID: "c", Text: "hello foo", Vector: []float32{0.7, 0.3, 0.0, 0.0}},
	}
	if err := col.Add(docs); err != nil {
		log.Fatalf("Add: %v", err)
	}
	fmt.Println("[demo] added 3 docs")

	// 5. Flush
	if err := col.Flush(); err != nil {
		log.Fatalf("Flush: %v", err)
	}
	fmt.Println("[demo] flushed")

	// 6. Search (vector)
	fmt.Println("[demo] --- vector search ---")
	hits, err := col.Search(vane.SearchQuery{
		Vector: []float32{1.0, 0.0, 0.0, 0.0},
		TopK:   3,
	})
	if err != nil {
		log.Fatalf("Search vector: %v", err)
	}
	for _, h := range hits {
		fmt.Printf("  hit: id=%s score=%.4f\n", h.ID, h.Score)
	}

	// 7. Search (text)
	fmt.Println("[demo] --- text search ---")
	hits, err = col.Search(vane.SearchQuery{
		Text: "hello",
		TopK: 3,
	})
	if err != nil {
		log.Fatalf("Search text: %v", err)
	}
	for _, h := range hits {
		fmt.Printf("  hit: id=%s score=%.4f\n", h.ID, h.Score)
	}

	// 8. Search (hybrid)
	fmt.Println("[demo] --- hybrid search ---")
	hits, err = col.Search(vane.SearchQuery{
		Text:   "hello",
		Vector: []float32{1.0, 0.0, 0.0, 0.0},
		TopK:   3,
	})
	if err != nil {
		log.Fatalf("Search hybrid: %v", err)
	}
	for _, h := range hits {
		fmt.Printf("  hit: id=%s score=%.4f\n", h.ID, h.Score)
	}

	fmt.Println("[demo] done")
}
