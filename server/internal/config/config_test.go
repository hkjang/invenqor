package config

import (
	"path/filepath"
	"testing"
	"time"
)

func TestValidateAcceptsSafeBootstrapConfig(t *testing.T) {
	t.Parallel()
	root := t.TempDir()
	config := Config{
		ListenAddress:   "127.0.0.1:8080",
		BaseURL:         "https://invenqor.example.test",
		StateDir:        root,
		SQLitePath:      filepath.Join(root, "invenqor.db"),
		DatabaseSchema:  "invenqor",
		DatabaseTimeout: time.Second,
		ShutdownTimeout: time.Second,
	}
	if err := config.Validate(); err != nil {
		t.Fatalf("Validate() error = %v", err)
	}
}

func TestValidateRejectsRelativeStateDirectory(t *testing.T) {
	t.Parallel()
	config := Config{
		ListenAddress:   "127.0.0.1:8080",
		StateDir:        "data",
		SQLitePath:      "/tmp/invenqor.db",
		DatabaseSchema:  "public",
		DatabaseTimeout: time.Second,
		ShutdownTimeout: time.Second,
	}
	if err := config.Validate(); err == nil {
		t.Fatal("Validate() accepted a relative state directory")
	}
}

func TestValidateRejectsUnsafeSchemaName(t *testing.T) {
	t.Parallel()
	root := t.TempDir()
	config := Config{
		ListenAddress:   "127.0.0.1:8080",
		StateDir:        root,
		SQLitePath:      filepath.Join(root, "invenqor.db"),
		DatabaseSchema:  `public"; DROP TABLE users; --`,
		DatabaseTimeout: time.Second,
		ShutdownTimeout: time.Second,
	}
	if err := config.Validate(); err == nil {
		t.Fatal("Validate() accepted an unsafe schema name")
	}
}
