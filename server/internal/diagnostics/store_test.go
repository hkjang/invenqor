package diagnostics

import (
	"context"
	"path/filepath"
	"testing"

	"github.com/hkjang/invenqor/server/internal/storage"
)

func TestStoreFiltersSharedDiagnosticsAndRedactsSecrets(t *testing.T) {
	runtime, err := storage.Open(context.Background(), storage.Options{
		SQLitePath: filepath.Join(t.TempDir(), "test.db"),
	})
	if err != nil {
		t.Fatal(err)
	}
	defer runtime.Close()
	store := NewStore(runtime.DB())
	if err := store.Record(context.Background(), Event{
		Level:     "error",
		Component: "agent_enrollment",
		EventCode: "AGENT_ENROLLMENT_FAILED",
		Message:   "failed with ivq_at_should-never-appear",
		RequestID: "request-123",
		AgentID:   "agent-123",
		SourceIP:  "10.20.30.40",
		Details: map[string]any{
			"error": "postgres://user:password@database/invenqor",
			"token": "ivq_et_should-never-appear",
		},
	}); err != nil {
		t.Fatal(err)
	}
	items, instances, err := store.List(context.Background(), Filter{
		Level: "error",
		Query: "request-123",
		Limit: 10,
	})
	if err != nil {
		t.Fatal(err)
	}
	if len(items) != 1 || len(instances) != 1 {
		t.Fatalf("items/instances = %d/%d", len(items), len(instances))
	}
	if items[0].Message != "failed with [REDACTED]" ||
		items[0].Details["token"] != "[REDACTED]" ||
		items[0].Details["error"] !=
			"postgres://user:[REDACTED]@database/invenqor" {
		t.Fatalf("diagnostic was not redacted: %#v", items[0])
	}
}
