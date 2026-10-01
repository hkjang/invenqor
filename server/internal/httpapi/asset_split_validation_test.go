package httpapi

import (
	"encoding/json"
	"net/http"
	"testing"
	"time"

	"github.com/google/uuid"
)

// openapi.yaml promises only `400 Invalid split or source does not belong to
// the original asset` for POST /api/v1/assets/{assetID}/split, but the handler
// put the path parameter and every source_ids element straight into
// `UPDATE asset_sources SET asset_id=$1 WHERE id=$2 AND asset_id=$3`.
// asset_sources.id and asset_sources.asset_id are UUID columns on PostgreSQL
// and TEXT on the SQLite fallback, so an id that is not a UUID failed the query
// itself with SQLSTATE 22P02 (a 500) on PostgreSQL while the fallback matched no
// rows and answered 400. These tests run through the real router, which had no
// coverage of this write path at all.

// splitAssetResponse posts to the production split route.
func splitAssetResponse(
	t *testing.T, server *Server, cookie *http.Cookie, csrf, assetID, name string,
	sourceIDs ...string,
) (int, string, string) {
	t.Helper()
	response := performAuthenticatedJSON(t, server, http.MethodPost,
		"/api/v1/assets/"+assetID+"/split",
		map[string]any{
			"source_ids": sourceIDs, "name": name, "type": "host",
			"reason": "the evidence belongs to its own host",
		}, cookie, csrf)
	var payload struct {
		AssetID string `json:"asset_id"`
		Error   struct {
			Code string `json:"code"`
		} `json:"error"`
	}
	_ = json.Unmarshal(response.Body.Bytes(), &payload)
	return response.Code, payload.Error.Code, payload.AssetID
}

// insertTestAssetSource adds one collected-evidence row for an asset, following
// the column list classification_test.go uses.
func insertTestAssetSource(t *testing.T, server *Server, assetID, sourceName string) string {
	t.Helper()
	id := uuid.NewString()
	if _, err := server.database.DB().Exec(
		`INSERT INTO asset_sources(
			id, asset_id, category, source_asset_id, source_name, payload_json,
			collected_at, first_seen_at, last_seen_at
		 ) VALUES($1,$2,'system','legacy',$3,'{}',
		          CURRENT_TIMESTAMP,CURRENT_TIMESTAMP,CURRENT_TIMESTAMP)`,
		id, assetID, sourceName,
	); err != nil {
		t.Fatalf("insert asset_sources for %s: %v", assetID, err)
	}
	return id
}

func sourceOwner(t *testing.T, server *Server, sourceID string) string {
	t.Helper()
	var owner string
	if err := server.database.DB().QueryRow(
		`SELECT asset_id FROM asset_sources WHERE id=$1`, sourceID,
	).Scan(&owner); err != nil {
		t.Fatalf("read owner of source %s: %v", sourceID, err)
	}
	return owner
}

// assertSplitWroteNothing checks the three tables a split writes to, because a
// response code alone would not catch a handler that had already created the new
// asset or recorded a successful split.
func assertSplitWroteNothing(t *testing.T, server *Server) {
	t.Helper()
	if created := countRows(t, server,
		`SELECT COUNT(*) FROM assets WHERE source='manual'`,
	); created != 0 {
		t.Fatalf("assets created by the split = %d, want 0", created)
	}
	if changes := countRows(t, server,
		`SELECT COUNT(*) FROM asset_changes WHERE change_type='split'`,
	); changes != 0 {
		t.Fatalf("asset_changes 'split' rows = %d, want 0", changes)
	}
	if entries := countRows(t, server,
		`SELECT COUNT(*) FROM audit_logs WHERE action='asset.split'`,
	); entries != 0 {
		t.Fatalf("audit_logs 'asset.split' rows = %d, want 0", entries)
	}
}

// The {assetID} path parameter reaches asset_sources.asset_id, a UUID column on
// PostgreSQL, so a malformed one has to be refused before any query runs rather
// than answering 400 on one dialect and 500 on the other.
func TestSplitAssetRejectsAssetIDThatIsNotAUUID(t *testing.T) {
	runtime := newRuntime(t)
	server := testServer(t, runtime)
	cookie, csrf := authenticateInitialAdmin(t, server, runtime)
	now := time.Now().UTC()
	originalID := insertSoftwareTestAsset(t, server, "shape-original",
		"shape-original", "host", `{}`, 1, now)
	sourceID := insertTestAssetSource(t, server, originalID, "shape-source")

	status, code, _ := splitAssetResponse(t, server, cookie, csrf, "not-a-uuid",
		"split-of-nothing", sourceID)
	if status != http.StatusBadRequest || code != "INVALID_SPLIT" {
		t.Fatalf("split of a malformed asset id = %d %q, want 400 INVALID_SPLIT",
			status, code)
	}
	if got := sourceOwner(t, server, sourceID); got != originalID {
		t.Fatalf("source moved to %q, want it left on %q", got, originalID)
	}
	assertSplitWroteNothing(t, server)
}

// Same for source_ids: one malformed element made the UPDATE itself fail on
// PostgreSQL. Nothing may be left behind, including the new asset row the
// handler inserts before it looks at the sources.
func TestSplitAssetRejectsSourceIDsThatAreNotUUIDs(t *testing.T) {
	runtime := newRuntime(t)
	server := testServer(t, runtime)
	cookie, csrf := authenticateInitialAdmin(t, server, runtime)
	now := time.Now().UTC()
	originalID := insertSoftwareTestAsset(t, server, "shape-source-original",
		"shape-source-original", "host", `{}`, 1, now)
	goodSourceID := insertTestAssetSource(t, server, originalID, "good-source")

	status, code, _ := splitAssetResponse(t, server, cookie, csrf, originalID,
		"split-of-malformed", goodSourceID, "not-a-uuid")
	if status != http.StatusBadRequest || code != "INVALID_SOURCE" {
		t.Fatalf("split with a malformed source id = %d %q, want 400 INVALID_SOURCE",
			status, code)
	}
	if got := sourceOwner(t, server, goodSourceID); got != originalID {
		t.Fatalf("the well-formed source moved to %q anyway, want %q", got, originalID)
	}
	assertSplitWroteNothing(t, server)
}

// The first regression test for a successful split: the new asset, the moved
// evidence, the asset_changes entry on the original and the audit record.
func TestSplitAssetMovesTheSourceAndRecordsTheChange(t *testing.T) {
	runtime := newRuntime(t)
	server := testServer(t, runtime)
	cookie, csrf := authenticateInitialAdmin(t, server, runtime)
	now := time.Now().UTC()
	originalID := insertSoftwareTestAsset(t, server, "split-original",
		"split-original", "host", `{}`, 1, now)
	movedSourceID := insertTestAssetSource(t, server, originalID, "moved-source")
	keptSourceID := insertTestAssetSource(t, server, originalID, "kept-source")

	status, code, newAssetID := splitAssetResponse(t, server, cookie, csrf,
		originalID, "split-result", movedSourceID)
	if status != http.StatusCreated {
		t.Fatalf("split = %d %q, want 201", status, code)
	}
	if _, err := uuid.Parse(newAssetID); err != nil {
		t.Fatalf("asset_id in the response = %q, want a uuid", newAssetID)
	}
	if got := sourceOwner(t, server, movedSourceID); got != newAssetID {
		t.Fatalf("split source owner = %q, want the new asset %q", got, newAssetID)
	}
	if got := sourceOwner(t, server, keptSourceID); got != originalID {
		t.Fatalf("the source that was not split moved to %q, want %q", got, originalID)
	}
	if got := assetStatus(t, server, newAssetID); got != "active" {
		t.Fatalf("new asset status = %q, want active", got)
	}
	if changes := countRows(t, server,
		`SELECT COUNT(*) FROM asset_changes WHERE change_type='split' AND asset_id=$1`,
		originalID,
	); changes != 1 {
		t.Fatalf("asset_changes 'split' rows on the original = %d, want 1", changes)
	}
	if entries := countRows(t, server,
		`SELECT COUNT(*) FROM audit_logs WHERE action='asset.split' AND resource_id=$1`,
		originalID,
	); entries != 1 {
		t.Fatalf("audit_logs 'asset.split' rows for the original = %d, want 1", entries)
	}
}

// Existing behaviour that the shape checks must not disturb: a well-formed
// source id belonging to a different asset is still refused, and the new asset
// the handler had already inserted is rolled back with it.
func TestSplitAssetRejectsSourceOwnedByAnotherAsset(t *testing.T) {
	runtime := newRuntime(t)
	server := testServer(t, runtime)
	cookie, csrf := authenticateInitialAdmin(t, server, runtime)
	now := time.Now().UTC()
	originalID := insertSoftwareTestAsset(t, server, "foreign-original",
		"foreign-original", "host", `{}`, 1, now)
	otherID := insertSoftwareTestAsset(t, server, "foreign-other", "foreign-other",
		"host", `{}`, 1, now.Add(-time.Minute))
	foreignSourceID := insertTestAssetSource(t, server, otherID, "foreign-source")

	status, code, _ := splitAssetResponse(t, server, cookie, csrf, originalID,
		"split-of-foreign", foreignSourceID)
	if status != http.StatusBadRequest || code != "INVALID_SOURCE" {
		t.Fatalf("split of another asset's source = %d %q, want 400 INVALID_SOURCE",
			status, code)
	}
	if got := sourceOwner(t, server, foreignSourceID); got != otherID {
		t.Fatalf("the foreign source moved to %q, want it left on %q", got, otherID)
	}
	assertSplitWroteNothing(t, server)
}
