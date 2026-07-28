package spool

import (
	"os"
	"path/filepath"
	"testing"
)

func TestAppendIsSecureAndIdempotent(t *testing.T) {
	manager, err := Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	duplicate, err := manager.Append("agent-1", "event-1", []byte(`{"one":1}`))
	if err != nil || duplicate {
		t.Fatalf("first Append() = %v/%v", duplicate, err)
	}
	duplicate, err = manager.Append("agent-1", "event-1", []byte(`{"two":2}`))
	if err != nil || !duplicate {
		t.Fatalf("second Append() = %v/%v", duplicate, err)
	}
	paths, err := manager.Pending()
	if err != nil || len(paths) != 1 {
		t.Fatalf("Pending() = %#v/%v", paths, err)
	}
	bytes, err := manager.Read(paths[0])
	if err != nil {
		t.Fatal(err)
	}
	if string(bytes) != `{"one":1}` {
		t.Fatalf("spool segment was overwritten: %s", bytes)
	}
	info, err := os.Stat(paths[0])
	if err != nil {
		t.Fatal(err)
	}
	if info.Mode().Perm() != 0o600 {
		t.Fatalf("segment mode = %o, want 600", info.Mode().Perm())
	}
	directoryInfo, err := os.Stat(filepath.Dir(paths[0]))
	if err != nil {
		t.Fatal(err)
	}
	if directoryInfo.Mode().Perm() != 0o700 {
		t.Fatalf("spool directory mode = %o, want 700", directoryInfo.Mode().Perm())
	}
	if err := manager.Acknowledge(paths[0]); err != nil {
		t.Fatal(err)
	}
}
