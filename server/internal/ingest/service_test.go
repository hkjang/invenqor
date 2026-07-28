package ingest

import (
	"context"
	"encoding/json"
	"path/filepath"
	"testing"
	"time"

	"github.com/google/uuid"
	"github.com/hkjang/invenqor/server/internal/agents"
	"github.com/hkjang/invenqor/server/internal/storage"
)

func TestInventoryIsIdempotentAndPreservesRawEvent(t *testing.T) {
	t.Parallel()
	runtime, agent, service := testService(t)
	envelope := inventoryEnvelope(agent.AgentID, []AssetRecord{
		record("host-source", "system", `{"hostname":"node-01","architecture":"x86_64"}`),
		record("pkg-source", "software.package", `{"name":"curl","version":"1"}`),
	})
	raw, _ := json.Marshal(envelope)
	result, err := service.Process(
		context.Background(), agent, envelope, raw, "0.1.0",
	)
	if err != nil {
		t.Fatalf("Process() error = %v", err)
	}
	if result.Duplicate {
		t.Fatal("first event reported as duplicate")
	}
	result, err = service.Process(
		context.Background(), agent, envelope, raw, "0.1.0",
	)
	if err != nil {
		t.Fatalf("duplicate Process() error = %v", err)
	}
	if !result.Duplicate {
		t.Fatal("second event was not reported as duplicate")
	}
	assertCount(t, runtime, "agent_events", 1)
	assertCount(t, runtime, "assets", 2)
	assertCount(t, runtime, "asset_sources", 2)
	assertCount(t, runtime, "asset_changes", 2)
	assertCount(t, runtime, "asset_relations", 1)
	var stored string
	if err := runtime.DB().QueryRow(
		"SELECT raw_event FROM agent_events",
	).Scan(&stored); err != nil {
		t.Fatal(err)
	}
	if stored != string(raw) {
		t.Fatalf("raw event was changed\n got: %s\nwant: %s", stored, raw)
	}
	var hostname, architecture, version string
	if err := runtime.DB().QueryRow(
		"SELECT hostname, architecture, version FROM agents WHERE id = $1",
		agent.ID,
	).Scan(&hostname, &architecture, &version); err != nil {
		t.Fatal(err)
	}
	if hostname != "node-01" || architecture != "x86_64" || version != "0.1.0" {
		t.Fatalf(
			"agent metadata = %q/%q/%q",
			hostname,
			architecture,
			version,
		)
	}
}

func TestCollectorErrorNeverInfersRemovalAndExplicitRemovalIsLogical(t *testing.T) {
	t.Parallel()
	runtime, agent, service := testService(t)
	first := inventoryEnvelope(agent.AgentID, []AssetRecord{
		record("host-source", "system", `{"hostname":"node-02"}`),
		record("pkg-source", "software.package", `{"name":"curl","version":"1"}`),
	})
	processEnvelope(t, service, agent, first)

	withError := inventoryEnvelope(agent.AgentID, nil)
	withError.CollectionErrors = []CollectionError{{
		Collector: "packages",
		Message:   "permission denied",
	}}
	processEnvelope(t, service, agent, withError)
	var active int
	if err := runtime.DB().QueryRow(
		`SELECT COUNT(*) FROM asset_sources
		 WHERE category = 'software.package' AND deleted_at IS NULL`,
	).Scan(&active); err != nil {
		t.Fatal(err)
	}
	if active != 1 {
		t.Fatal("collector error incorrectly removed an asset source")
	}
	assertCount(t, runtime, "collector_errors", 1)

	removed := inventoryEnvelope(agent.AgentID, nil)
	removed.Changes = []AssetChange{{
		Kind: "removed", AssetID: "pkg-source", Category: "software.package",
	}}
	processEnvelope(t, service, agent, removed)
	var status string
	var deleted any
	if err := runtime.DB().QueryRow(
		`SELECT a.status, a.deleted_at FROM assets a
		 JOIN asset_sources s ON s.asset_id = a.id
		 WHERE s.category = 'software.package'`,
	).Scan(&status, &deleted); err != nil {
		t.Fatal(err)
	}
	if status != "removed" || deleted == nil {
		t.Fatalf("removed asset status/deleted_at = %q/%v", status, deleted)
	}
	assertCountWhere(
		t,
		runtime,
		"asset_changes",
		"change_type = 'removed'",
		1,
	)
}

func TestUpdatedChangeKeepsAssetHistory(t *testing.T) {
	t.Parallel()
	runtime, agent, service := testService(t)
	first := inventoryEnvelope(agent.AgentID, []AssetRecord{
		record("pkg-source", "software.package", `{"name":"curl","version":"1"}`),
	})
	processEnvelope(t, service, agent, first)
	updatedRecord := record(
		"pkg-source",
		"software.package",
		`{"name":"curl","version":"2"}`,
	)
	updated := inventoryEnvelope(agent.AgentID, nil)
	updated.Changes = []AssetChange{{
		Kind: "updated", AssetID: updatedRecord.AssetID,
		Category: updatedRecord.Category, Record: &updatedRecord,
	}}
	processEnvelope(t, service, agent, updated)
	assertCount(t, runtime, "assets", 1)
	assertCount(t, runtime, "asset_snapshots", 2)
	assertCountWhere(
		t,
		runtime,
		"asset_changes",
		"change_type = 'updated'",
		1,
	)
}

func TestEnvelopeValidationRejectsIdentityAndHeartbeatPayload(t *testing.T) {
	t.Parallel()
	valid := inventoryEnvelope(uuid.NewString(), nil)
	valid.EventID = "not-a-uuid"
	if err := valid.Validate(); err == nil {
		t.Fatal("invalid event UUID was accepted")
	}
	valid = inventoryEnvelope(uuid.NewString(), nil)
	valid.Kind = "heartbeat"
	valid.Changes = []AssetChange{{
		Kind: "removed", AssetID: "x", Category: "process",
	}}
	if err := valid.Validate(); err == nil {
		t.Fatal("heartbeat with changes was accepted")
	}
}

func testService(t *testing.T) (*storage.Runtime, agents.Agent, *Service) {
	t.Helper()
	runtime, err := storage.Open(context.Background(), storage.Options{
		SQLitePath: filepath.Join(t.TempDir(), "test.db"),
	})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { runtime.Close() })
	agentService := agents.NewService(runtime.DB())
	provisioned, err := agentService.ProvisionBearer(
		context.Background(), uuid.NewString(), "test-host", "",
	)
	if err != nil {
		t.Fatal(err)
	}
	return runtime, provisioned.Agent, NewService(runtime.DB())
}

func inventoryEnvelope(agentID string, records []AssetRecord) Envelope {
	now := uint64(time.Now().Unix())
	var snapshot *Snapshot
	if records != nil {
		snapshot = &Snapshot{
			SchemaVersion: 1,
			AgentID:       agentID,
			CollectedAt:   now,
			Records:       records,
		}
	}
	return Envelope{
		SchemaVersion: 1,
		EventID:       uuid.NewString(),
		AgentID:       agentID,
		CreatedAt:     now,
		Kind:          "inventory",
		SnapshotHash:  uuid.NewString(),
		Snapshot:      snapshot,
	}
}

func record(id string, category string, payload string) AssetRecord {
	return AssetRecord{
		AssetID: id, Category: category, Source: "test",
		CollectedAt: uint64(time.Now().Unix()), Payload: json.RawMessage(payload),
	}
}

func processEnvelope(
	t *testing.T,
	service *Service,
	agent agents.Agent,
	envelope Envelope,
) {
	t.Helper()
	if err := envelope.Validate(); err != nil {
		t.Fatal(err)
	}
	raw, _ := json.Marshal(envelope)
	if _, err := service.Process(
		context.Background(), agent, envelope, raw, "test",
	); err != nil {
		t.Fatalf("Process() error = %v", err)
	}
}

func assertCount(
	t *testing.T,
	runtime *storage.Runtime,
	table string,
	want int,
) {
	t.Helper()
	var got int
	if err := runtime.DB().QueryRow(
		"SELECT COUNT(*) FROM " + table,
	).Scan(&got); err != nil {
		t.Fatal(err)
	}
	if got != want {
		t.Fatalf("%s count = %d, want %d", table, got, want)
	}
}

func assertCountWhere(
	t *testing.T,
	runtime *storage.Runtime,
	table string,
	condition string,
	want int,
) {
	t.Helper()
	var got int
	if err := runtime.DB().QueryRow(
		"SELECT COUNT(*) FROM " + table + " WHERE " + condition,
	).Scan(&got); err != nil {
		t.Fatal(err)
	}
	if got != want {
		t.Fatalf("%s count = %d, want %d", table, got, want)
	}
}
