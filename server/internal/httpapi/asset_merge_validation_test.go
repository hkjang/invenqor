package httpapi

import (
	"encoding/json"
	"net/http"
	"testing"
	"time"

	"github.com/google/uuid"
)

// openapi.yaml promises `400 Primary or secondary assets missing` for
// POST /api/v1/assets/merge, but the handler never looked the ids up. A
// secondary that is not an asset updated no rows and still answered 200 while
// writing a "merged" asset_changes row and a successful audit entry; a primary
// that is not an asset only surfaced as the asset_changes foreign key failing,
// i.e. a 500. Both are checked here through the real router.

// mergeAssetsResponse posts to the production merge route.
func mergeAssetsResponse(
	t *testing.T, server *Server, cookie *http.Cookie, csrf, primaryID string,
	secondaryIDs ...string,
) (int, string) {
	t.Helper()
	response := performAuthenticatedJSON(t, server, http.MethodPost,
		"/api/v1/assets/merge",
		map[string]any{
			"primary_id": primaryID, "secondary_ids": secondaryIDs,
			"reason": "duplicate host",
		}, cookie, csrf)
	var payload struct {
		Error struct {
			Code string `json:"code"`
		} `json:"error"`
	}
	_ = json.Unmarshal(response.Body.Bytes(), &payload)
	return response.Code, payload.Error.Code
}

func countRows(t *testing.T, server *Server, query string, args ...any) int {
	t.Helper()
	var count int
	if err := server.database.DB().QueryRow(query, args...).Scan(&count); err != nil {
		t.Fatalf("count %q: %v", query, err)
	}
	return count
}

// assertMergeWroteNothing checks the two tables a merge writes to, because a
// response code alone would not catch a rolled-back handler that had already
// recorded a successful merge.
func assertMergeWroteNothing(t *testing.T, server *Server) {
	t.Helper()
	if changes := countRows(t, server,
		`SELECT COUNT(*) FROM asset_changes WHERE change_type='merged'`,
	); changes != 0 {
		t.Fatalf("asset_changes 'merged' rows = %d, want 0", changes)
	}
	if entries := countRows(t, server,
		`SELECT COUNT(*) FROM audit_logs WHERE action='asset.merge'`,
	); entries != 0 {
		t.Fatalf("audit_logs 'asset.merge' rows = %d, want 0", entries)
	}
}

func assetStatus(t *testing.T, server *Server, assetID string) string {
	t.Helper()
	var status string
	if err := server.database.DB().QueryRow(
		`SELECT status FROM assets WHERE id=$1`, assetID,
	).Scan(&status); err != nil {
		t.Fatalf("read status of %s: %v", assetID, err)
	}
	return status
}

func TestMergeAssetsRejectsPrimaryThatIsNotAnAsset(t *testing.T) {
	runtime := newRuntime(t)
	server := testServer(t, runtime)
	cookie, csrf := authenticateInitialAdmin(t, server, runtime)
	now := time.Now().UTC()
	secondaryID := insertSoftwareTestAsset(t, server, "orphan-secondary",
		"orphan-secondary", "host", `{}`, 1, now)

	status, code := mergeAssetsResponse(t, server, cookie, csrf, uuid.NewString(),
		secondaryID)
	if status != http.StatusBadRequest || code != "INVALID_MERGE" {
		t.Fatalf("merge into a missing primary = %d %q, want 400 INVALID_MERGE",
			status, code)
	}
	if got := assetStatus(t, server, secondaryID); got != "active" {
		t.Fatalf("secondary status = %q, want it untouched (active)", got)
	}
	assertMergeWroteNothing(t, server)
}

// The whole request rolls back: a secondary that exists alongside one that does
// not stays unmerged, rather than half the request succeeding.
func TestMergeAssetsRejectsMissingSecondaryWithoutMergingTheOthers(t *testing.T) {
	runtime := newRuntime(t)
	server := testServer(t, runtime)
	cookie, csrf := authenticateInitialAdmin(t, server, runtime)
	now := time.Now().UTC()
	primaryID := insertSoftwareTestAsset(t, server, "keep-primary", "keep-primary",
		"host", `{}`, 1, now)
	realSecondaryID := insertSoftwareTestAsset(t, server, "keep-secondary",
		"keep-secondary", "host", `{}`, 1, now.Add(-time.Minute))

	status, code := mergeAssetsResponse(t, server, cookie, csrf, primaryID,
		realSecondaryID, uuid.NewString())
	if status != http.StatusBadRequest || code != "INVALID_MERGE" {
		t.Fatalf("merge with a missing secondary = %d %q, want 400 INVALID_MERGE",
			status, code)
	}
	if got := assetStatus(t, server, realSecondaryID); got != "active" {
		t.Fatalf("the existing secondary was merged anyway: status = %q", got)
	}
	if got := assetStatus(t, server, primaryID); got != "active" {
		t.Fatalf("primary status = %q, want active", got)
	}
	assertMergeWroteNothing(t, server)
}

// assets.id is a UUID column on PostgreSQL and TEXT on the SQLite fallback, so
// an id that is not a UUID would make the lookup itself fail on one dialect
// only. Both must answer the same code for the same input.
func TestMergeAssetsRejectsIdsThatAreNotUUIDs(t *testing.T) {
	runtime := newRuntime(t)
	server := testServer(t, runtime)
	cookie, csrf := authenticateInitialAdmin(t, server, runtime)
	now := time.Now().UTC()
	primaryID := insertSoftwareTestAsset(t, server, "shape-primary",
		"shape-primary", "host", `{}`, 1, now)

	status, code := mergeAssetsResponse(t, server, cookie, csrf, primaryID,
		"not-a-uuid")
	if status != http.StatusBadRequest || code != "INVALID_MERGE" {
		t.Fatalf("merge with a malformed secondary id = %d %q, want 400 INVALID_MERGE",
			status, code)
	}
	assertMergeWroteNothing(t, server)
}
