package config

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestValidateAcceptsSafeBootstrapConfig(t *testing.T) {
	t.Parallel()
	root := t.TempDir()
	config := Config{
		ListenAddress:   "127.0.0.1:7070",
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
		ListenAddress:   "127.0.0.1:7070",
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
		ListenAddress:   "127.0.0.1:7070",
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

func TestValidateRequiresCompleteBootstrapCredentials(t *testing.T) {
	config := Config{
		ListenAddress:          "127.0.0.1:7070",
		StateDir:               t.TempDir(),
		SQLitePath:             "/tmp/invenqor.db",
		DatabaseSchema:         "public",
		DatabaseTimeout:        time.Second,
		ShutdownTimeout:        time.Second,
		BootstrapAdmin:         "bootstrap.admin",
		BootstrapAdminPassword: "",
	}
	if err := config.Validate(); err == nil {
		t.Fatal("Validate() accepted incomplete bootstrap credentials")
	}
}

func TestLoadReadsBootstrapCredentialsFromCanonicalEnvironment(t *testing.T) {
	clearBootstrapEnvironment(t)
	t.Setenv("INVENQOR_BOOTSTRAP_ADMIN", "bootstrap.admin")
	t.Setenv("INVENQOR_BOOTSTRAP_ADMIN_PASSWORD", "CorrectHorse!42")
	config, err := Load()
	if err != nil {
		t.Fatalf("Load() error = %v", err)
	}
	if config.BootstrapAdmin != "bootstrap.admin" ||
		config.BootstrapAdminPassword != "CorrectHorse!42" {
		t.Fatal("Load() returned unexpected bootstrap credentials")
	}
}

func TestLoadReadsBootstrapPasswordFromSecretFile(t *testing.T) {
	clearBootstrapEnvironment(t)
	path := filepath.Join(t.TempDir(), "password")
	if err := os.WriteFile(path, []byte("CorrectHorse!42\n"), 0o600); err != nil {
		t.Fatalf("write password file: %v", err)
	}
	t.Setenv("bootstrap_admin", "file.admin")
	t.Setenv("bootstrap_admin_password_file", path)
	config, err := Load()
	if err != nil {
		t.Fatalf("Load() error = %v", err)
	}
	if config.BootstrapAdmin != "file.admin" ||
		config.BootstrapAdminPassword != "CorrectHorse!42" {
		t.Fatal("Load() returned unexpected file bootstrap credentials")
	}
}

func TestLoadRejectsDirectAndFileBootstrapPasswords(t *testing.T) {
	clearBootstrapEnvironment(t)
	path := filepath.Join(t.TempDir(), "password")
	if err := os.WriteFile(path, []byte("CorrectHorse!42\n"), 0o600); err != nil {
		t.Fatalf("write password file: %v", err)
	}
	t.Setenv("BOOTSTRAP_ADMIN", "bootstrap.admin")
	t.Setenv("BOOTSTRAP_ADMIN_PASSWORD", "CorrectHorse!42")
	t.Setenv("BOOTSTRAP_ADMIN_PASSWORD_FILE", path)
	if _, err := Load(); err == nil ||
		!strings.Contains(err.Error(), "cannot both be configured") {
		t.Fatalf("Load() error = %v, want conflicting password sources", err)
	}
}

func clearBootstrapEnvironment(t *testing.T) {
	t.Helper()
	for _, name := range []string{
		"INVENQOR_BOOTSTRAP_ADMIN",
		"BOOTSTRAP_ADMIN",
		"bootstrap_admin",
		"INVENQOR_BOOTSTRAP_ADMIN_PASSWORD",
		"BOOTSTRAP_ADMIN_PASSWORD",
		"bootstrap_admin_password",
		"INVENQOR_BOOTSTRAP_ADMIN_PASSWORD_FILE",
		"BOOTSTRAP_ADMIN_PASSWORD_FILE",
		"bootstrap_admin_password_file",
	} {
		t.Setenv(name, "")
	}
}
