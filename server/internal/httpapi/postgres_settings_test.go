package httpapi

import (
	"context"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"strings"
	"testing"

	"github.com/hkjang/invenqor/server/internal/storage"
)

func TestPostgresSettingsRejectInvalidDSNWithoutSecretLeak(t *testing.T) {
	runtime, err := storage.Open(context.Background(), storage.Options{
		SQLitePath: filepath.Join(t.TempDir(), "invenqor.db"),
	})
	if err != nil {
		t.Fatalf("storage.Open() error = %v", err)
	}
	defer runtime.Close()
	server := testServer(t, runtime)
	cookie, csrf := authenticateInitialAdmin(t, server, runtime)
	secret := "postgres-settings-secret"
	response := performAuthenticatedJSON(
		t,
		server,
		http.MethodPost,
		"/api/v1/admin/settings/postgresql/test",
		map[string]string{
			"dsn": "postgres://user:" + secret + "@%zz/invenqor",
		},
		cookie,
		csrf,
	)
	if response.Code != http.StatusBadGateway {
		t.Fatalf(
			"test status = %d body = %s",
			response.Code,
			response.Body.String(),
		)
	}
	if strings.Contains(response.Body.String(), secret) {
		t.Fatal("PostgreSQL settings response exposed the password")
	}
}

func TestPostgresSettingsStatusRequiresAuthentication(t *testing.T) {
	runtime, err := storage.Open(context.Background(), storage.Options{
		SQLitePath: filepath.Join(t.TempDir(), "invenqor.db"),
	})
	if err != nil {
		t.Fatalf("storage.Open() error = %v", err)
	}
	defer runtime.Close()
	server := testServer(t, runtime)
	request := httptest.NewRequest(
		http.MethodGet,
		"/api/v1/admin/settings/postgresql",
		nil,
	)
	response := httptest.NewRecorder()
	server.Handler().ServeHTTP(response, request)
	if response.Code != http.StatusUnauthorized {
		t.Fatalf("status = %d, want 401", response.Code)
	}
}
