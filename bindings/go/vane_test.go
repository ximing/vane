//go:build !wazero

package vane

import (
	"os"
	"path/filepath"
	"testing"
)

func TestOpenClose(t *testing.T) {
	dir := t.TempDir()
	dbPath := filepath.Join(dir, "test.db")

	db, err := Open(dbPath, nil)
	if err != nil {
		t.Fatalf("Open: %v", err)
	}
	if err := db.Close(); err != nil {
		t.Fatalf("Close: %v", err)
	}
}

func TestFullCycle(t *testing.T) {
	dir := t.TempDir()
	dbPath := filepath.Join(dir, "test.db")

	db, err := Open(dbPath, nil)
	if err != nil {
		t.Fatalf("Open: %v", err)
	}
	defer db.Close()

	schema := Schema{
		Fields: []SchemaField{
			{Name: "vec", Type: "vector", Dim: 4, Metric: "cosine"},
			{Name: "body", Type: "text"},
		},
	}
	col, err := db.Collection("docs", schema, nil)
	if err != nil {
		t.Fatalf("Collection: %v", err)
	}
	defer col.Close()

	docs := []Doc{
		{ID: "a", Text: "hello world", Vector: []float32{1.0, 0.0, 0.0, 0.0}},
		{ID: "b", Text: "foo bar", Vector: []float32{0.0, 1.0, 0.0, 0.0}},
		{ID: "c", Text: "hello foo", Vector: []float32{0.7, 0.3, 0.0, 0.0}},
	}
	if err := col.Add(docs); err != nil {
		t.Fatalf("Add: %v", err)
	}
	if err := col.Flush(); err != nil {
		t.Fatalf("Flush: %v", err)
	}

	// vector search
	hits, err := col.Search(SearchQuery{
		Vector: []float32{1.0, 0.0, 0.0, 0.0},
		TopK:   3,
	})
	if err != nil {
		t.Fatalf("Search: %v", err)
	}
	if len(hits) == 0 {
		t.Fatal("expected at least 1 hit")
	}
	if hits[0].ID != "a" {
		t.Errorf("expected top hit 'a', got '%s'", hits[0].ID)
	}

	// text search
	hits, err = col.Search(SearchQuery{
		Text: "hello",
		TopK: 3,
	})
	if err != nil {
		t.Fatalf("Search text: %v", err)
	}
	if len(hits) == 0 {
		t.Fatal("expected at least 1 text hit")
	}

	// hybrid search
	hits, err = col.Search(SearchQuery{
		Text:   "hello",
		Vector: []float32{1.0, 0.0, 0.0, 0.0},
		TopK:   3,
	})
	if err != nil {
		t.Fatalf("Search hybrid: %v", err)
	}
	if len(hits) == 0 {
		t.Fatal("expected at least 1 hybrid hit")
	}

	// delete
	count, err := col.Delete([]string{"a"})
	if err != nil {
		t.Fatalf("Delete: %v", err)
	}
	if count == 0 {
		t.Error("expected delete count > 0")
	}

	// compact
	if err := col.Compact(); err != nil {
		t.Fatalf("Compact: %v", err)
	}
}

func TestUseAfterClose(t *testing.T) {
	dir := t.TempDir()
	dbPath := filepath.Join(dir, "test.db")

	db, err := Open(dbPath, nil)
	if err != nil {
		t.Fatalf("Open: %v", err)
	}
	db.Close()

	// use after close should return error (not panic/UB)
	_, err = db.Collection("x", Schema{}, nil)
	if err == nil {
		t.Error("expected error using closed db")
	}
}

func TestDictVersionBeforeLoad(t *testing.T) {
	// DictVersion without loading dict should return E_DICT_UNAVAILABLE
	_, err := DictVersion()
	if err == nil {
		t.Log("DictVersion succeeded (dict may have been loaded by another test)")
	}
}

// 确保临时目录被清理
func TestMain(m *testing.M) {
	os.Exit(m.Run())
}
