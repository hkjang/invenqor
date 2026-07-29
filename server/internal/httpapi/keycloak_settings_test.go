package httpapi

import (
	"context"
	"net/http"
	"path/filepath"
	"strings"
	"testing"

	"github.com/hkjang/invenqor/server/internal/auth"
	"github.com/hkjang/invenqor/server/internal/storage"
)

func TestKeycloakSettingsRequireSecretBeforeEnable(t *testing.T) {
	runtime, err := storage.Open(context.Background(), storage.Options{
		SQLitePath: filepath.Join(t.TempDir(), "invenqor.db"),
	})
	if err != nil {
		t.Fatalf("storage.Open() error = %v", err)
	}
	defer runtime.Close()
	server := testServer(t, runtime)
	cookie, csrf := authenticateInitialAdmin(t, server, runtime)
	settings := auth.DefaultOIDCSettings()
	settings.Enabled = true
	settings.IssuerURL = "https://keycloak.example.test"
	settings.ClientID = "invenqor"
	settings.RedirectURI = "https://invenqor.example.test/api/v1/auth/keycloak/callback"
	response := performAuthenticatedJSON(
		t,
		server,
		http.MethodPatch,
		"/api/v1/admin/settings/keycloak",
		map[string]any{
			"settings": settings,
			"reason":   "enable SSO",
		},
		cookie,
		csrf,
	)
	if response.Code != http.StatusBadRequest {
		t.Fatalf(
			"status = %d body = %s",
			response.Code,
			response.Body.String(),
		)
	}
	if !strings.Contains(response.Body.String(), "KEYCLOAK_SECRET_REQUIRED") {
		t.Fatalf("response = %s, want KEYCLOAK_SECRET_REQUIRED", response.Body.String())
	}
}

func TestKeycloakSettingsRejectUnknownMappedRole(t *testing.T) {
	runtime, err := storage.Open(context.Background(), storage.Options{
		SQLitePath: filepath.Join(t.TempDir(), "invenqor.db"),
	})
	if err != nil {
		t.Fatalf("storage.Open() error = %v", err)
	}
	defer runtime.Close()
	server := testServer(t, runtime)
	cookie, csrf := authenticateInitialAdmin(t, server, runtime)
	settings := auth.DefaultOIDCSettings()
	settings.RoleMappings = map[string]string{"realm-user": "unknown-role"}
	response := performAuthenticatedJSON(
		t,
		server,
		http.MethodPatch,
		"/api/v1/admin/settings/keycloak",
		map[string]any{
			"settings": settings,
			"reason":   "configure mapping",
		},
		cookie,
		csrf,
	)
	if response.Code != http.StatusBadRequest ||
		!strings.Contains(response.Body.String(), "INVALID_KEYCLOAK_ROLE") {
		t.Fatalf(
			"status = %d body = %s, want INVALID_KEYCLOAK_ROLE",
			response.Code,
			response.Body.String(),
		)
	}
}
