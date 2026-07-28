package httpapi

import (
	"bytes"
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/hkjang/invenqor/server/internal/auth"
	"github.com/hkjang/invenqor/server/internal/storage"
)

func TestInitialAdminLoginSessionAndCSRF(t *testing.T) {
	runtime, err := storage.Open(context.Background(), storage.Options{
		SQLitePath: filepath.Join(t.TempDir(), "invenqor.db"),
	})
	if err != nil {
		t.Fatalf("storage.Open() error = %v", err)
	}
	defer runtime.Close()
	server := testServer(t, runtime)
	tokenPath := filepath.Join(filepath.Dir(runtime.SQLitePath()), "initial-admin.token")
	tokenBytes, err := os.ReadFile(tokenPath)
	if err != nil {
		t.Fatalf("read bootstrap token error = %v", err)
	}

	createBody := `{
		"username":"Admin.User",
		"password":"CorrectHorse!42",
		"display_name":"Initial Administrator",
		"email":"admin@example.test"
	}`
	createRequest := httptest.NewRequest(
		http.MethodPost,
		"/api/v1/bootstrap/admin",
		strings.NewReader(createBody),
	)
	createRequest.Header.Set("Content-Type", "application/json")
	createRequest.Header.Set(
		"X-Invenqor-Bootstrap-Token",
		strings.TrimSpace(string(tokenBytes)),
	)
	createResponse := httptest.NewRecorder()
	server.Handler().ServeHTTP(createResponse, createRequest)
	if createResponse.Code != http.StatusCreated {
		t.Fatalf(
			"create initial admin status = %d body = %s",
			createResponse.Code,
			createResponse.Body.String(),
		)
	}
	if _, err := os.Stat(tokenPath); !os.IsNotExist(err) {
		t.Fatalf("bootstrap token file remains after setup: %v", err)
	}

	loginResponse := performJSON(
		t,
		server,
		http.MethodPost,
		"/api/v1/auth/local/login",
		map[string]string{
			"username": "admin.user",
			"password": "CorrectHorse!42",
		},
		nil,
	)
	if loginResponse.Code != http.StatusOK {
		t.Fatalf(
			"login status = %d body = %s",
			loginResponse.Code,
			loginResponse.Body.String(),
		)
	}
	var loginPayload struct {
		CSRFToken string    `json:"csrf_token"`
		User      auth.User `json:"user"`
	}
	if err := json.Unmarshal(loginResponse.Body.Bytes(), &loginPayload); err != nil {
		t.Fatalf("decode login response error = %v", err)
	}
	if loginPayload.CSRFToken == "" {
		t.Fatal("login response omitted CSRF token")
	}
	if !loginPayload.User.SuperAdmin ||
		!contains(loginPayload.User.Roles, "super_admin") ||
		!contains(loginPayload.User.Permissions, "settings.write") {
		t.Fatalf("initial administrator permissions = %#v", loginPayload.User)
	}
	var sessionCookie *http.Cookie
	for _, cookie := range loginResponse.Result().Cookies() {
		if cookie.Name == auth.SessionCookie {
			sessionCookie = cookie
			break
		}
	}
	if sessionCookie == nil {
		t.Fatal("login response omitted session cookie")
	}
	if !sessionCookie.HttpOnly || !sessionCookie.Secure ||
		sessionCookie.SameSite != http.SameSiteStrictMode {
		t.Fatalf("session cookie flags = %#v", sessionCookie)
	}

	meRequest := httptest.NewRequest(http.MethodGet, "/api/v1/auth/me", nil)
	meRequest.AddCookie(sessionCookie)
	meResponse := httptest.NewRecorder()
	server.Handler().ServeHTTP(meResponse, meRequest)
	if meResponse.Code != http.StatusOK {
		t.Fatalf("me status = %d body = %s", meResponse.Code, meResponse.Body.String())
	}

	logoutWithoutCSRF := httptest.NewRequest(http.MethodPost, "/api/v1/auth/logout", nil)
	logoutWithoutCSRF.AddCookie(sessionCookie)
	logoutWithoutCSRFResponse := httptest.NewRecorder()
	server.Handler().ServeHTTP(logoutWithoutCSRFResponse, logoutWithoutCSRF)
	if logoutWithoutCSRFResponse.Code != http.StatusForbidden {
		t.Fatalf("logout without CSRF status = %d, want 403", logoutWithoutCSRFResponse.Code)
	}

	logoutRequest := httptest.NewRequest(http.MethodPost, "/api/v1/auth/logout", nil)
	logoutRequest.AddCookie(sessionCookie)
	logoutRequest.Header.Set("X-CSRF-Token", loginPayload.CSRFToken)
	logoutResponse := httptest.NewRecorder()
	server.Handler().ServeHTTP(logoutResponse, logoutRequest)
	if logoutResponse.Code != http.StatusOK {
		t.Fatalf(
			"logout status = %d body = %s",
			logoutResponse.Code,
			logoutResponse.Body.String(),
		)
	}

	expiredRequest := httptest.NewRequest(http.MethodGet, "/api/v1/auth/me", nil)
	expiredRequest.AddCookie(sessionCookie)
	expiredResponse := httptest.NewRecorder()
	server.Handler().ServeHTTP(expiredResponse, expiredRequest)
	if expiredResponse.Code != http.StatusUnauthorized {
		t.Fatalf("revoked session status = %d, want 401", expiredResponse.Code)
	}

	var auditEntries int
	if err := runtime.DB().QueryRow(
		"SELECT COUNT(*) FROM audit_logs",
	).Scan(&auditEntries); err != nil {
		t.Fatalf("count audit logs error = %v", err)
	}
	if auditEntries < 3 {
		t.Fatalf("audit entry count = %d, want at least 3", auditEntries)
	}
}

func TestBootstrapTokenCannotBeReused(t *testing.T) {
	runtime, err := storage.Open(context.Background(), storage.Options{
		SQLitePath: filepath.Join(t.TempDir(), "invenqor.db"),
	})
	if err != nil {
		t.Fatalf("storage.Open() error = %v", err)
	}
	defer runtime.Close()
	server := testServer(t, runtime)
	token, err := os.ReadFile(
		filepath.Join(filepath.Dir(runtime.SQLitePath()), "initial-admin.token"),
	)
	if err != nil {
		t.Fatalf("read bootstrap token error = %v", err)
	}
	input := map[string]string{
		"username":     "first.admin",
		"password":     "CorrectHorse!42",
		"display_name": "First",
	}
	headers := map[string]string{
		"X-Invenqor-Bootstrap-Token": strings.TrimSpace(string(token)),
	}
	first := performJSON(t, server, http.MethodPost, "/api/v1/bootstrap/admin", input, headers)
	if first.Code != http.StatusCreated {
		t.Fatalf("first setup status = %d body = %s", first.Code, first.Body.String())
	}
	second := performJSON(t, server, http.MethodPost, "/api/v1/bootstrap/admin", input, headers)
	if second.Code != http.StatusConflict {
		t.Fatalf("second setup status = %d body = %s", second.Code, second.Body.String())
	}
}

func performJSON(
	t *testing.T,
	server *Server,
	method string,
	path string,
	body any,
	headers map[string]string,
) *httptest.ResponseRecorder {
	t.Helper()
	bytesBody, err := json.Marshal(body)
	if err != nil {
		t.Fatalf("json.Marshal() error = %v", err)
	}
	request := httptest.NewRequest(method, path, bytes.NewReader(bytesBody))
	request.Header.Set("Content-Type", "application/json")
	for name, value := range headers {
		request.Header.Set(name, value)
	}
	response := httptest.NewRecorder()
	server.Handler().ServeHTTP(response, request)
	return response
}

func contains(values []string, target string) bool {
	for _, value := range values {
		if value == target {
			return true
		}
	}
	return false
}
