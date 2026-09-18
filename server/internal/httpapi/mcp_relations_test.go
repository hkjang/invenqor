package httpapi

import (
	"encoding/json"
	"net/http/httptest"
	"testing"
	"time"

	"github.com/google/uuid"
)

// A relation row used to carry only the two UUIDs at its ends. A model asked
// "what runs on this host" then had to call asset_get once per edge to learn a
// single name, so a host with fifty children cost fifty-one tool calls, and a
// model that skipped them answered with UUIDs nobody can read. The row now
// names both ends and says which way the edge points from the asset asked
// about.

type mcpRelationEnd struct {
	ID     string `json:"id"`
	Name   string `json:"name"`
	Type   string `json:"type"`
	Status string `json:"status"`
}

type mcpNamedRelation struct {
	SourceAssetID string         `json:"source_asset_id"`
	TargetAssetID string         `json:"target_asset_id"`
	RelationType  string         `json:"relation_type"`
	Direction     string         `json:"direction"`
	SourceAsset   mcpRelationEnd `json:"source_asset"`
	TargetAsset   mcpRelationEnd `json:"target_asset"`
}

func mcpNamedRelations(t *testing.T, server *Server, assetID string) []mcpNamedRelation {
	t.Helper()
	result, err := server.mcpAssetRelations(
		httptest.NewRequest("POST", "/mcp", nil),
		mcpArgumentsForTest("asset_relations", []byte(`{"asset_id":"`+assetID+`"}`)),
	)
	if err != nil {
		t.Fatal(err)
	}
	encoded, _ := json.Marshal(result)
	var page struct {
		Relations []mcpNamedRelation `json:"relations"`
	}
	if err := json.Unmarshal(encoded, &page); err != nil {
		t.Fatal(err)
	}
	return page.Relations
}

func seedRelation(t *testing.T, server *Server, sourceID, relationType, targetID string) {
	t.Helper()
	if _, err := server.database.DB().Exec(
		`INSERT INTO asset_relations(
			id,source_asset_id,relation_type,target_asset_id,source,confidence
		 ) VALUES($1,$2,$3,$4,'manual',1.0)`,
		uuid.NewString(), sourceID, relationType, targetID,
	); err != nil {
		t.Fatalf("seed relation %s: %v", relationType, err)
	}
}

func TestMCPAssetRelationsNamesBothEndsAndTheDirection(t *testing.T) {
	runtime := newRuntime(t)
	server := testServer(t, runtime)
	now := time.Now().UTC()
	hostID := insertSoftwareTestAsset(t, server, "named-host", "db-01", "host", `{}`, 1, now)
	processID := insertSoftwareTestAsset(t, server, "named-process", "postgres",
		"process", `{}`, 1, now)
	clusterID := insertSoftwareTestAsset(t, server, "named-cluster", "prod-cluster",
		"cluster", `{}`, 1, now)
	// One edge leaves the host, one arrives at it.
	seedRelation(t, server, hostID, "runs_on", processID)
	seedRelation(t, server, clusterID, "member_of", hostID)

	relations := mcpNamedRelations(t, server, hostID)
	if len(relations) != 2 {
		t.Fatalf("host relations = %+v", relations)
	}
	byType := map[string]mcpNamedRelation{}
	for _, relation := range relations {
		byType[relation.RelationType] = relation
	}

	outbound := byType["runs_on"]
	if outbound.Direction != "outbound" ||
		outbound.SourceAsset != (mcpRelationEnd{ID: hostID, Name: "db-01", Type: "host", Status: "active"}) ||
		outbound.TargetAsset != (mcpRelationEnd{ID: processID, Name: "postgres", Type: "process", Status: "active"}) {
		t.Fatalf("edge leaving the host = %+v", outbound)
	}
	inbound := byType["member_of"]
	if inbound.Direction != "inbound" ||
		inbound.SourceAsset != (mcpRelationEnd{ID: clusterID, Name: "prod-cluster", Type: "cluster", Status: "active"}) ||
		inbound.TargetAsset != (mcpRelationEnd{ID: hostID, Name: "db-01", Type: "host", Status: "active"}) {
		t.Fatalf("edge arriving at the host = %+v", inbound)
	}
	// The UUID columns the earlier response consisted of are still there, so a
	// client written against them keeps working.
	if outbound.SourceAssetID != hostID || outbound.TargetAssetID != processID ||
		inbound.SourceAssetID != clusterID || inbound.TargetAssetID != hostID {
		t.Fatalf("relation ids: outbound=%+v inbound=%+v", outbound, inbound)
	}

	// The same edge seen from the other end points the other way.
	fromProcess := mcpNamedRelations(t, server, processID)
	if len(fromProcess) != 1 || fromProcess[0].Direction != "inbound" ||
		fromProcess[0].SourceAsset.Name != "db-01" {
		t.Fatalf("the runs_on edge seen from the process = %+v", fromProcess)
	}
}

// Deleting or merging an asset leaves its edges open, and asset_get answers
// "asset not found" for it. The row says so through the other end's status,
// so the model is not sent to fetch an asset that is gone.
func TestMCPAssetRelationsReportsADeletedCounterpart(t *testing.T) {
	runtime := newRuntime(t)
	server := testServer(t, runtime)
	now := time.Now().UTC()
	hostID := insertSoftwareTestAsset(t, server, "keeping-host", "keeping-host",
		"host", `{}`, 1, now)
	goneID := insertSoftwareTestAsset(t, server, "gone-process", "gone-process",
		"process", `{}`, 1, now)
	seedRelation(t, server, hostID, "runs_on", goneID)
	if _, err := server.database.DB().Exec(
		`UPDATE assets SET deleted_at=$1, status='deleted' WHERE id=$2`, now, goneID,
	); err != nil {
		t.Fatal(err)
	}

	relations := mcpNamedRelations(t, server, hostID)
	if len(relations) != 1 || relations[0].TargetAsset.Status != "deleted" ||
		relations[0].TargetAsset.Name != "gone-process" {
		t.Fatalf("edge to a deleted asset = %+v", relations)
	}
}
