package httpapi

import (
	"context"
	"encoding/json"
	"io"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"strings"
	"testing"

	"github.com/hkjang/invenqor/server/internal/auth"
	"github.com/hkjang/invenqor/server/internal/bootstrap"
	"github.com/hkjang/invenqor/server/internal/storage"
)

func TestHealthEndpointsReportSQLiteFallback(t *testing.T) {
	runtime, err := storage.Open(context.Background(), storage.Options{
		SQLitePath: filepath.Join(t.TempDir(), "invenqor.db"),
	})
	if err != nil {
		t.Fatalf("storage.Open() error = %v", err)
	}
	defer runtime.Close()
	server := testServer(t, runtime)

	for _, path := range []string{"/health/live", "/health/ready", "/health/database"} {
		request := httptest.NewRequest(http.MethodGet, path, nil)
		response := httptest.NewRecorder()
		server.Handler().ServeHTTP(response, request)
		if response.Code != http.StatusOK {
			t.Errorf("%s status = %d, want 200", path, response.Code)
		}
	}

	request := httptest.NewRequest(http.MethodGet, "/health/database", nil)
	response := httptest.NewRecorder()
	server.Handler().ServeHTTP(response, request)
	var payload map[string]any
	if err := json.Unmarshal(response.Body.Bytes(), &payload); err != nil {
		t.Fatalf("decode response error = %v", err)
	}
	if payload["mode"] != string(storage.ModeSQLiteFallback) {
		t.Fatalf("mode = %v, want %s", payload["mode"], storage.ModeSQLiteFallback)
	}
}

func TestDatabaseHealthDoesNotExposePostgresPassword(t *testing.T) {
	secret := "do-not-expose-this"
	runtime, err := storage.Open(context.Background(), storage.Options{
		PostgresDSN: "postgres://user:" + secret + "@%zz/invenqor",
		SQLitePath:  filepath.Join(t.TempDir(), "invenqor.db"),
	})
	if err != nil {
		t.Fatalf("storage.Open() error = %v", err)
	}
	defer runtime.Close()
	server := testServer(t, runtime)
	request := httptest.NewRequest(http.MethodGet, "/health/database", nil)
	response := httptest.NewRecorder()
	server.Handler().ServeHTTP(response, request)
	if strings.Contains(response.Body.String(), secret) {
		t.Fatal("database health response exposed the PostgreSQL password")
	}
	if !strings.Contains(response.Body.String(), "INVALID_DSN") {
		t.Fatalf("database health response = %s, want INVALID_DSN", response.Body.String())
	}
}

func TestSecurityHeadersAreApplied(t *testing.T) {
	runtime, err := storage.Open(context.Background(), storage.Options{
		SQLitePath: filepath.Join(t.TempDir(), "invenqor.db"),
	})
	if err != nil {
		t.Fatalf("storage.Open() error = %v", err)
	}
	defer runtime.Close()
	server := testServer(t, runtime)
	request := httptest.NewRequest(http.MethodGet, "/", nil)
	response := httptest.NewRecorder()
	server.Handler().ServeHTTP(response, request)
	for _, header := range []string{
		"Content-Security-Policy",
		"X-Content-Type-Options",
		"X-Frame-Options",
	} {
		if response.Header().Get(header) == "" {
			t.Errorf("%s header is missing", header)
		}
	}
}

func testServer(t *testing.T, runtime *storage.Runtime) *Server {
	t.Helper()
	stateDir := filepath.Dir(runtime.SQLitePath())
	bootstrapManager := auth.NewBootstrapManager(runtime.DB(), stateDir)
	if _, err := bootstrapManager.Ensure(context.Background()); err != nil {
		t.Fatalf("BootstrapManager.Ensure() error = %v", err)
	}
	bootstrapStore, err := bootstrap.Open(stateDir)
	if err != nil {
		t.Fatalf("bootstrap.Open() error = %v", err)
	}
	totpService := auth.NewTOTPService(runtime.DB(), bootstrapStore)
	authOptions := auth.DefaultServiceOptions()
	authOptions.TOTP = totpService
	authService, err := auth.NewService(runtime.DB(), authOptions)
	if err != nil {
		t.Fatalf("auth.NewService() error = %v", err)
	}
	oidcService := auth.NewOIDCService(runtime.DB(), bootstrapStore, authService)
	return New(Options{
		Database:         runtime,
		AuthService:      authService,
		OIDCService:      oidcService,
		TOTPService:      totpService,
		BootstrapManager: bootstrapManager,
		BootstrapStore:   bootstrapStore,
		Logger:           slog.New(slog.NewTextHandler(io.Discard, nil)),
	})
}
