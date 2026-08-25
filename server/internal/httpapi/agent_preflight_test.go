package httpapi

import (
	"database/sql"

	"context"
	"encoding/json"
	"fmt"
	_ "github.com/jackc/pgx/v5/stdlib"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"sync/atomic"
	"testing"

	"github.com/google/uuid"
	"github.com/hkjang/invenqor/server/internal/storage"
)

// newRuntime opens the store these tests run against.
//
// SQLite by default, because it needs nothing installed. Set
// INVENQOR_TEST_POSTGRES_DSN to run the same tests against a real PostgreSQL -
// see scripts/test-postgres.sh.
//
// That switch is not a nicety. The two engines disagree in ways that make a
// SQLite-only suite report success over a broken deployment: JSONB has no text
// operators, so a query that works here fails there with SQLSTATE 42883, and
// SQLite's LIKE ignores ASCII case while PostgreSQL's does not, so a search that
// matches here returns nothing there. Both of those shipped.
//
// Each test gets its own schema so they do not collide when run in parallel and
// so a failing test leaves its rows behind for inspection.
func newRuntime(t *testing.T) *storage.Runtime {
	t.Helper()
	options := storage.Options{SQLitePath: filepath.Join(t.TempDir(), "invenqor.db")}
	if dsn := os.Getenv("INVENQOR_TEST_POSTGRES_DSN"); dsn != "" {
		schema := testSchemaName(t)
		createTestSchema(t, dsn, schema)
		options = storage.Options{PostgresDSN: dsn, Schema: schema}
	}
	runtime, err := storage.Open(context.Background(), options)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { runtime.Close() })
	return runtime
}

// testStateDir is where this test's master.key, sealed bootstrap values and
// initial admin token live.
//
// In SQLite mode that is the directory holding the database file. A PostgreSQL
// runtime has no SQLite path, and filepath.Dir("") is ".", so deriving it that
// way put key material in the package directory and gave every test one shared
// bootstrap store - a secret written by one test answered another test's lookup.
// Memoised so every caller for a given runtime gets the same directory.
// Production takes this from INVENQOR_STATE_DIR and never derives it.
var testStateDirs sync.Map

func testStateDir(t *testing.T, runtime *storage.Runtime) string {
	t.Helper()
	if path := runtime.SQLitePath(); path != "" {
		return filepath.Dir(path)
	}
	if existing, ok := testStateDirs.Load(runtime); ok {
		return existing.(string)
	}
	directory := t.TempDir()
	actual, _ := testStateDirs.LoadOrStore(runtime, directory)
	return actual.(string)
}

// createTestSchema makes the schema the runtime will migrate into. The server
// sets search_path but does not create the schema, so this mirrors what an
// operator does once before pointing the server at a non-public schema.
func createTestSchema(t *testing.T, dsn, schema string) {
	t.Helper()
	database, err := sql.Open("pgx", dsn)
	if err != nil {
		t.Fatal(err)
	}
	defer database.Close()
	if _, err := database.Exec(`DROP SCHEMA IF EXISTS "` + schema + `" CASCADE`); err != nil {
		t.Fatal(err)
	}
	if _, err := database.Exec(`CREATE SCHEMA "` + schema + `"`); err != nil {
		t.Fatal(err)
	}
}

// testSchemaName derives a PostgreSQL schema identifier from the test name.
var testSchemaCounter atomic.Uint64

func testSchemaName(t *testing.T) string {
	t.Helper()
	safe := strings.Map(func(r rune) rune {
		if (r >= 'a' && r <= 'z') || (r >= '0' && r <= '9') || r == '_' {
			return r
		}
		if r >= 'A' && r <= 'Z' {
			return r + ('a' - 'A')
		}
		return '_'
	}, t.Name())
	if len(safe) > 40 {
		safe = safe[:40]
	}
	return fmt.Sprintf("t_%s_%d", safe, testSchemaCounter.Add(1))
}

func getJSON(
	t *testing.T,
	server *Server,
	path string,
	headers map[string]string,
) *httptest.ResponseRecorder {
	t.Helper()
	request := httptest.NewRequest(http.MethodGet, path, nil)
	for name, value := range headers {
		request.Header.Set(name, value)
	}
	response := httptest.NewRecorder()
	server.Handler().ServeHTTP(response, request)
	return response
}

func decodePreflight(
	t *testing.T,
	response *httptest.ResponseRecorder,
) map[string]any {
	t.Helper()
	if response.Code != http.StatusOK {
		t.Fatalf("preflight status = %d body = %s", response.Code, response.Body.String())
	}
	var payload map[string]any
	if err := json.Unmarshal(response.Body.Bytes(), &payload); err != nil {
		t.Fatalf("decode preflight: %v", err)
	}
	return payload
}

func TestPreflightReportsAnOpenPolicyAndTheObservedSourceAddress(t *testing.T) {
	server := testServer(t, newRuntime(t))
	payload := decodePreflight(t, getJSON(t, server, "/v1/agent/preflight", nil))

	if payload["observed_source_ip"] != "192.0.2.1" {
		t.Fatalf("observed_source_ip = %v", payload["observed_source_ip"])
	}
	enrollment := payload["enrollment"].(map[string]any)
	if enrollment["would_enroll"] != true {
		t.Fatalf("would_enroll = %v", enrollment["would_enroll"])
	}
	if enrollment["reason"] != "AGENT_ENROLLMENT_READY" {
		t.Fatalf("reason = %v", enrollment["reason"])
	}
	credential := payload["credential"].(map[string]any)
	if credential["state"] != "absent" {
		t.Fatalf("credential state = %v", credential["state"])
	}
	if payload["request_id"] == "" {
		t.Fatal("preflight omitted the request identifier used for correlation")
	}
}

func TestPreflightAgreesWithARejectedEnrollment(t *testing.T) {
	server := testServer(t, newRuntime(t))
	if _, _, err := server.updateAgentEnrollmentPolicy(
		context.Background(),
		"test",
		func(policy *agentEnrollmentPolicy) error {
			policy.Enabled = true
			policy.NetworkMode = "allowlist"
			policy.AllowedNetworks = []string{"10.10.0.0/16"}
			return nil
		},
	); err != nil {
		t.Fatal(err)
	}

	payload := decodePreflight(t, getJSON(t, server, "/v1/agent/preflight", nil))
	enrollment := payload["enrollment"].(map[string]any)
	if enrollment["would_enroll"] != false {
		t.Fatalf("would_enroll = %v", enrollment["would_enroll"])
	}
	if enrollment["reason"] != "AGENT_SOURCE_NOT_ALLOWED" {
		t.Fatalf("reason = %v", enrollment["reason"])
	}
	if enrollment["network_allowed"] != false {
		t.Fatalf("network_allowed = %v", enrollment["network_allowed"])
	}

	// The real endpoint must reject the same request for the same reason,
	// otherwise a preflight "OK" would send operators looking elsewhere.
	enroll := performJSON(
		t, server, http.MethodPost, "/v1/agent/enroll",
		map[string]string{
			"agent_id":    uuid.NewString(),
			"hostname":    "rejected-host",
			"claim_token": "ivq_ec_0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
		},
		nil,
	)
	if enroll.Code != http.StatusForbidden {
		t.Fatalf("enrollment status = %d body = %s", enroll.Code, enroll.Body.String())
	}
	var rejection struct {
		Error struct {
			Code string `json:"code"`
		} `json:"error"`
	}
	if err := json.Unmarshal(enroll.Body.Bytes(), &rejection); err != nil {
		t.Fatal(err)
	}
	if rejection.Error.Code != enrollment["reason"] {
		t.Fatalf(
			"enrollment code %q disagrees with preflight reason %v",
			rejection.Error.Code,
			enrollment["reason"],
		)
	}
}

func TestPreflightValidatesAStoredDeviceCredential(t *testing.T) {
	runtime := newRuntime(t)
	server := testServer(t, runtime)
	enroll := performJSON(
		t, server, http.MethodPost, "/v1/agent/enroll",
		map[string]string{
			"agent_id":    uuid.NewString(),
			"hostname":    "credentialed-host",
			"claim_token": "ivq_ec_0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
		},
		nil,
	)
	if enroll.Code != http.StatusCreated {
		t.Fatalf("enrollment status = %d body = %s", enroll.Code, enroll.Body.String())
	}
	var enrolled struct {
		Token string `json:"token"`
	}
	if err := json.Unmarshal(enroll.Body.Bytes(), &enrolled); err != nil {
		t.Fatal(err)
	}

	valid := decodePreflight(t, getJSON(t, server, "/v1/agent/preflight", map[string]string{
		"Authorization": "Bearer " + enrolled.Token,
	}))
	credential := valid["credential"].(map[string]any)
	if credential["state"] != "valid" {
		t.Fatalf("credential state = %v", credential["state"])
	}
	if credential["hostname"] != "credentialed-host" {
		t.Fatalf("credential hostname = %v", credential["hostname"])
	}

	invalid := decodePreflight(t, getJSON(t, server, "/v1/agent/preflight", map[string]string{
		"Authorization": "Bearer ivq_at_not-a-real-credential",
	}))
	if state := invalid["credential"].(map[string]any)["state"]; state != "invalid" {
		t.Fatalf("rejected credential state = %v", state)
	}
}

func TestAgentPathMistakesAnswerJSONAndAreRecorded(t *testing.T) {
	runtime := newRuntime(t)
	server := testServer(t, runtime)

	// A base URL with a stray path prefix must not receive the console SPA.
	strayPath := performJSON(
		t, server, http.MethodPost, "/invenqor/v1/agent/enroll",
		map[string]string{"agent_id": uuid.NewString()}, nil,
	)
	if strayPath.Code != http.StatusNotFound {
		t.Fatalf("stray path status = %d body = %s", strayPath.Code, strayPath.Body.String())
	}
	if got := strayPath.Header().Get("Content-Type"); got != "application/json; charset=utf-8" {
		t.Fatalf("stray path content type = %q", got)
	}
	wrongMethod := getJSON(t, server, "/v1/agent/enroll", nil)
	if wrongMethod.Code != http.StatusMethodNotAllowed {
		t.Fatalf("wrong method status = %d", wrongMethod.Code)
	}

	cookie, _ := authenticateInitialAdmin(t, server, runtime)
	logs := performAuthenticatedJSON(
		t, server, http.MethodGet,
		"/api/v1/admin/diagnostics/logs?component=agent_transport",
		nil, cookie, "",
	)
	if logs.Code != http.StatusOK {
		t.Fatalf("diagnostics status = %d body = %s", logs.Code, logs.Body.String())
	}
	body := logs.Body.String()
	for _, code := range []string{
		"AGENT_ENDPOINT_NOT_FOUND",
		"AGENT_ENDPOINT_METHOD_NOT_ALLOWED",
	} {
		if !strings.Contains(body, code) {
			t.Fatalf("diagnostics omitted %s: %s", code, body)
		}
	}
	// The remedy has to travel with the event; the code alone is not actionable.
	if !strings.Contains(body, "scheme, host and port only") {
		t.Fatalf("diagnostics omitted the remediation text: %s", body)
	}
}
