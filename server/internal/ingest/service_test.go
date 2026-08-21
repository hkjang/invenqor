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
		record("nic-source", "network.interface", `{"interface":"eth0","mac":"aa:bb"}`),
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
	assertCount(t, runtime, "assets", 3)
	assertCount(t, runtime, "asset_sources", 3)
	assertCount(t, runtime, "asset_changes", 3)
	// Exactly one relationship: the interface is part of its host. A plain OS
	// package earns no edge - a host with thousands of packages would otherwise
	// produce a graph nobody can read.
	assertCount(t, runtime, "asset_relations", 1)
	assertCountWhere(
		t, runtime, "asset_relations",
		"relation_type = 'part_of' AND source = 'inferred' AND status = 'active'",
		1,
	)
	// Classification runs during ingest, with the rule trail recorded.
	var hostType, packageType, classificationSource string
	var confidence float64
	if err := runtime.DB().QueryRow(
		`SELECT a.type, a.classification_source, a.classification_confidence
		   FROM assets a JOIN asset_sources s ON s.asset_id = a.id
		  WHERE s.category = 'system'`,
	).Scan(&hostType, &classificationSource, &confidence); err != nil {
		t.Fatal(err)
	}
	if hostType != "host" || classificationSource != "rule" || confidence <= 0 {
		t.Fatalf(
			"host classification = %q/%q/%v",
			hostType, classificationSource, confidence,
		)
	}
	if err := runtime.DB().QueryRow(
		`SELECT a.type FROM assets a JOIN asset_sources s ON s.asset_id = a.id
		  WHERE s.category = 'software.package'`,
	).Scan(&packageType); err != nil {
		t.Fatal(err)
	}
	if packageType != "software" {
		t.Fatalf("package type = %q, want software", packageType)
	}
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

func TestWindowsSystemMetadataUpdatesRegisteredAgent(t *testing.T) {
	t.Parallel()
	runtime, agent, service := testService(t)
	envelope := inventoryEnvelope(agent.AgentID, []AssetRecord{
		record(
			"windows-host",
			"system",
			`{"hostname":"win-ops-01","os_family":"windows","os_name":"Windows 11 Enterprise","os_version":"24H2","os_build":"26100.4652","architecture":"x86_64"}`,
		),
	})
	processEnvelope(t, service, agent, envelope)

	var hostname, osName, architecture string
	if err := runtime.DB().QueryRow(
		"SELECT hostname, os_name, architecture FROM agents WHERE id = $1",
		agent.ID,
	).Scan(&hostname, &osName, &architecture); err != nil {
		t.Fatal(err)
	}
	if hostname != "win-ops-01" || osName != "Windows 11 Enterprise" ||
		architecture != "x86_64" {
		t.Fatalf(
			"Windows metadata = %q/%q/%q",
			hostname, osName, architecture,
		)
	}
}

func TestWindowsFamilyIsSafeMetadataFallback(t *testing.T) {
	t.Parallel()
	agentID := uuid.NewString()
	envelope := inventoryEnvelope(agentID, []AssetRecord{
		record(
			"windows-host",
			"system",
			`{"hostname":"win-core","os_family":"Windows","architecture":"aarch64"}`,
		),
	})
	hostname, osName, architecture := agentMetadata(envelope)
	if hostname != "win-core" || osName != "Windows" || architecture != "aarch64" {
		t.Fatalf("fallback metadata = %q/%q/%q", hostname, osName, architecture)
	}
}

func TestWindowsMetadataRecoversFromDeltaAfterAgentUpgrade(t *testing.T) {
	t.Parallel()
	runtime, agent, service := testService(t)
	system := record(
		"windows-host",
		"system",
		`{"hostname":"win-upgraded","os_family":"windows","os_name":"Windows Server 2022 Datacenter","architecture":"x86_64"}`,
	)
	envelope := inventoryEnvelope(agent.AgentID, nil)
	envelope.Changes = []AssetChange{{
		Kind: "updated", AssetID: system.AssetID,
		Category: system.Category, Record: &system,
	}}
	processEnvelope(t, service, agent, envelope)

	var hostname, osName string
	if err := runtime.DB().QueryRow(
		"SELECT hostname, os_name FROM agents WHERE id = $1", agent.ID,
	).Scan(&hostname, &osName); err != nil {
		t.Fatal(err)
	}
	if hostname != "win-upgraded" || osName != "Windows Server 2022 Datacenter" {
		t.Fatalf("delta metadata = %q/%q", hostname, osName)
	}
}

func TestWindowsMetadataRecoversFromStoredSystemSourceOnHeartbeat(t *testing.T) {
	t.Parallel()
	runtime, agent, service := testService(t)
	processEnvelope(t, service, agent, inventoryEnvelope(agent.AgentID, []AssetRecord{
		record(
			"windows-host",
			"system",
			`{"hostname":"win-existing","os_family":"windows","os_name":"Windows Server 2019 Datacenter","architecture":"x86_64"}`,
		),
	}))
	// v0.2.13 retained the raw source but failed to project these Windows fields
	// onto the agents row. The next heartbeat must repair that existing state.
	if _, err := runtime.DB().Exec(
		`UPDATE agents SET hostname = '', os_name = '', architecture = ''
		  WHERE id = $1`,
		agent.ID,
	); err != nil {
		t.Fatal(err)
	}
	legacyAgent := agent
	legacyAgent.Hostname = ""
	legacyAgent.OSName = ""
	legacyAgent.Architecture = ""
	processEnvelope(t, service, legacyAgent, heartbeatEnvelope(agent.AgentID))

	var hostname, osName, architecture string
	if err := runtime.DB().QueryRow(
		"SELECT hostname, os_name, architecture FROM agents WHERE id = $1",
		agent.ID,
	).Scan(&hostname, &osName, &architecture); err != nil {
		t.Fatal(err)
	}
	if hostname != "win-existing" || osName != "Windows Server 2019 Datacenter" ||
		architecture != "x86_64" {
		t.Fatalf("heartbeat metadata recovery = %q/%q/%q", hostname, osName, architecture)
	}
}

func TestStoredMetadataFallbackSkipsLookupForCompleteAgent(t *testing.T) {
	t.Parallel()
	hostname, osName, architecture, err := agentMetadataWithStoredFallback(
		context.Background(),
		nil, // A complete agent must return before touching the transaction.
		agents.Agent{
			Hostname: "complete-host", OSName: "Windows 11", Architecture: "x86_64",
		},
		heartbeatEnvelope(uuid.NewString()),
	)
	if err != nil || hostname != "" || osName != "" || architecture != "" {
		t.Fatalf(
			"complete agent fallback = %q/%q/%q, error = %v",
			hostname, osName, architecture, err,
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

func TestFirstInventoryPromotesEnrollmentHostWithoutDuplicateAsset(
	t *testing.T,
) {
	t.Parallel()
	runtime, err := storage.Open(context.Background(), storage.Options{
		SQLitePath: filepath.Join(t.TempDir(), "test.db"),
	})
	if err != nil {
		t.Fatal(err)
	}
	defer runtime.Close()
	agentService := agents.NewService(runtime.DB())
	externalID := uuid.NewString()
	provisioned, err := agentService.AutoEnroll(
		context.Background(),
		externalID,
		"enrollment-name",
		"ivq_ec_0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
		"10.50.60.70",
	)
	if err != nil {
		t.Fatal(err)
	}
	assertCount(t, runtime, "assets", 1)
	envelope := inventoryEnvelope(externalID, []AssetRecord{
		record(
			"host-source",
			"system",
			`{"hostname":"inventory-name","architecture":"x86_64"}`,
		),
		record(
			"pkg-source",
			"software.package",
			`{"name":"curl","version":"8"}`,
		),
	})
	processEnvelope(
		t,
		NewService(runtime.DB()),
		provisioned.Agent,
		envelope,
	)
	assertCount(t, runtime, "assets", 2)
	assertCount(t, runtime, "asset_sources", 3)
	var name, status, source string
	var sourceCount int
	if err := runtime.DB().QueryRow(
		`SELECT a.name,a.status,a.source,COUNT(s.id)
		 FROM assets a
		 JOIN asset_sources s ON s.asset_id=a.id
		 WHERE a.asset_key=$1
		 GROUP BY a.name,a.status,a.source`,
		"agent:"+externalID,
	).Scan(&name, &status, &source, &sourceCount); err != nil {
		t.Fatal(err)
	}
	if name != "inventory-name" || status != "active" ||
		source != "agent" || sourceCount != 2 {
		t.Fatalf(
			"promoted asset = %q/%q/%q with %d sources",
			name,
			status,
			source,
			sourceCount,
		)
	}
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
