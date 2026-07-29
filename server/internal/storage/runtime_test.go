package storage

import (
	"context"
	"io/fs"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/hkjang/invenqor/server/migrations"
)

func TestOpenWithoutPostgresUsesSQLiteFallback(t *testing.T) {
	t.Parallel()
	path := filepath.Join(t.TempDir(), "invenqor.db")
	runtime, err := Open(context.Background(), Options{
		SQLitePath: path,
		Timeout:    100 * time.Millisecond,
	})
	if err != nil {
		t.Fatalf("Open() error = %v", err)
	}
	defer runtime.Close()
	if runtime.Mode() != ModeSQLiteFallback {
		t.Fatalf("Mode() = %s, want %s", runtime.Mode(), ModeSQLiteFallback)
	}
	if runtime.PostgresFailure() != nil {
		t.Fatalf("PostgresFailure() = %#v, want nil", runtime.PostgresFailure())
	}
	if _, err := os.Stat(path); err != nil {
		t.Fatalf("SQLite database was not created: %v", err)
	}
	assertTableExists(t, runtime, "assets")
	assertTableExists(t, runtime, "audit_logs")
	assertTableExists(t, runtime, "db_migration_jobs")
}

func TestMalformedPostgresDSNFallsBackWithoutLeakingSecret(t *testing.T) {
	t.Parallel()
	path := filepath.Join(t.TempDir(), "invenqor.db")
	secret := "never-log-this-password"
	runtime, err := Open(context.Background(), Options{
		PostgresDSN: "postgres://user:" + secret + "@%zz/invenqor",
		SQLitePath:  path,
		Timeout:     100 * time.Millisecond,
	})
	if err != nil {
		t.Fatalf("Open() error = %v", err)
	}
	defer runtime.Close()
	if runtime.Mode() != ModeSQLiteFallback {
		t.Fatalf("Mode() = %s, want %s", runtime.Mode(), ModeSQLiteFallback)
	}
	failure := runtime.PostgresFailure()
	if failure == nil || failure.Code != FailureInvalidDSN {
		t.Fatalf("PostgresFailure() = %#v, want INVALID_DSN", failure)
	}
	if strings.Contains(failure.Summary, secret) || strings.Contains(failure.Host, secret) {
		t.Fatal("PostgreSQL failure metadata leaked the password")
	}
}

func TestUnavailablePostgresFallsBackToSQLite(t *testing.T) {
	t.Parallel()
	path := filepath.Join(t.TempDir(), "invenqor.db")
	runtime, err := Open(context.Background(), Options{
		PostgresDSN: "postgres://user:secret@127.0.0.1:1/invenqor?sslmode=disable",
		SQLitePath:  path,
		Timeout:     100 * time.Millisecond,
	})
	if err != nil {
		t.Fatalf("Open() error = %v", err)
	}
	defer runtime.Close()
	if runtime.Mode() != ModeSQLiteFallback {
		t.Fatalf("Mode() = %s, want %s", runtime.Mode(), ModeSQLiteFallback)
	}
	failure := runtime.PostgresFailure()
	if failure == nil || failure.Code != FailureConnection {
		t.Fatalf("PostgresFailure() = %#v, want CONNECTION_FAILURE", failure)
	}
}

func TestCheckPostgresRejectsMalformedDSNWithoutLeakingPassword(t *testing.T) {
	secret := "check-do-not-expose"
	failure := CheckPostgres(context.Background(), Options{
		PostgresDSN: "postgres://user:" + secret + "@%zz/invenqor",
		Timeout:     time.Second,
	})
	if failure == nil || failure.Code != FailureInvalidDSN {
		t.Fatalf("CheckPostgres() failure = %#v, want INVALID_DSN", failure)
	}
	if strings.Contains(failure.Summary, secret) ||
		strings.Contains(failure.Host, secret) {
		t.Fatal("CheckPostgres() failure leaked the PostgreSQL password")
	}
}

func TestSQLiteCreationFailureFailsStartup(t *testing.T) {
	t.Parallel()
	root := t.TempDir()
	parentFile := filepath.Join(root, "not-a-directory")
	if err := os.WriteFile(parentFile, []byte("x"), 0o600); err != nil {
		t.Fatalf("WriteFile() error = %v", err)
	}
	_, err := Open(context.Background(), Options{
		SQLitePath: filepath.Join(parentFile, "invenqor.db"),
		Timeout:    100 * time.Millisecond,
	})
	if err == nil {
		t.Fatal("Open() succeeded with an invalid SQLite path")
	}
}

func TestMigrationsAreIdempotent(t *testing.T) {
	t.Parallel()
	path := filepath.Join(t.TempDir(), "invenqor.db")
	first, err := Open(context.Background(), Options{SQLitePath: path})
	if err != nil {
		t.Fatalf("first Open() error = %v", err)
	}
	if err := first.Close(); err != nil {
		t.Fatalf("first Close() error = %v", err)
	}
	second, err := Open(context.Background(), Options{SQLitePath: path})
	if err != nil {
		t.Fatalf("second Open() error = %v", err)
	}
	defer second.Close()
	var versions int
	if err := second.DB().QueryRow("SELECT COUNT(*) FROM schema_migrations").Scan(&versions); err != nil {
		t.Fatalf("query schema_migrations error = %v", err)
	}
	// Counting a literal here means the number has to be updated with each
	// migration, which is the point: an accidentally re-runnable migration would
	// otherwise inflate this silently.
	expected := migrationFileCount(t, "sqlite")
	if versions != expected {
		t.Fatalf("migration versions = %d, want %d", versions, expected)
	}
	assertTableExists(t, second, "diagnostic_logs")
	assertTableExists(t, second, "asset_classification_rules")
}

// migrationFileCount counts the shipped migrations for a dialect so the
// idempotency check stays correct as the schema grows.
func migrationFileCount(t *testing.T, dialect string) int {
	t.Helper()
	entries, err := fs.ReadDir(migrations.Files, dialect)
	if err != nil {
		t.Fatalf("list %s migrations: %v", dialect, err)
	}
	count := 0
	for _, entry := range entries {
		if !entry.IsDir() && strings.HasSuffix(entry.Name(), ".sql") {
			count++
		}
	}
	return count
}

func assertTableExists(t *testing.T, runtime *Runtime, name string) {
	t.Helper()
	var count int
	if err := runtime.DB().QueryRow(
		"SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?",
		name,
	).Scan(&count); err != nil {
		t.Fatalf("query table %s error = %v", name, err)
	}
	if count != 1 {
		t.Fatalf("table %s does not exist", name)
	}
}
