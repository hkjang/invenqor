package httpapi

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"reflect"
	"strings"
	"testing"
	"time"
)

const mcpValidationAssetID = "abcdefab-cdef-4abc-8def-abcdefabcdef"

func insertMCPValidationAsset(t *testing.T, server *Server, id, name string) {
	t.Helper()
	generatedID := insertSoftwareTestAsset(t, server, name, name, "process", `{}`, 1, time.Now().UTC())
	// Fix the ID before creating references so case-folding coverage always
	// includes letters, independently of random UUID generation.
	if _, err := server.database.DB().Exec(`UPDATE assets SET id=$1 WHERE id=$2`, id, generatedID); err != nil {
		t.Fatal(err)
	}
}

func callMCPValidationTool(t *testing.T, server *Server, secret, tool, assetID string, modern bool) map[string]any {
	t.Helper()
	params := map[string]any{"name": tool, "arguments": map[string]any{"asset_id": assetID}}
	var headers map[string]string
	if modern {
		params["_meta"] = modernMCPMetadata()
		headers = map[string]string{
			"MCP-Protocol-Version": mcpModernProtocolVersion,
			"Mcp-Method":           "tools/call",
			"Mcp-Name":             tool,
		}
	}
	response := performMCPRequest(t, server, secret, map[string]any{
		"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": params,
	}, headers)
	if response.Code != http.StatusOK {
		t.Fatalf("HTTP status = %d, body = %s", response.Code, response.Body)
	}
	var envelope map[string]any
	if err := json.Unmarshal(response.Body.Bytes(), &envelope); err != nil {
		t.Fatal(err)
	}
	if envelope["error"] != nil || envelope["jsonrpc"] != "2.0" || envelope["id"] != float64(1) {
		t.Fatalf("unexpected JSON-RPC envelope: %s", response.Body)
	}
	return decodeMCPResult(t, response)
}

func mcpValidationErrorText(t *testing.T, result map[string]any) string {
	t.Helper()
	if result["isError"] != true {
		t.Fatalf("isError = %v, want true; result = %#v", result["isError"], result)
	}
	content, ok := result["content"].([]any)
	if !ok || len(content) != 1 {
		t.Fatalf("error content = %#v", result["content"])
	}
	block, ok := content[0].(map[string]any)
	if !ok || block["type"] != "text" {
		t.Fatalf("error content block = %#v", content[0])
	}
	message, ok := block["text"].(string)
	if !ok {
		t.Fatalf("error text = %#v", block["text"])
	}
	return message
}

func assertMCPValidationError(t *testing.T, tool, message string) {
	t.Helper()
	for _, part := range []string{tool, "asset_id", "36", "hyphen", "UUID"} {
		if !strings.Contains(message, part) {
			t.Errorf("validation error = %q, want %q", message, part)
		}
	}
	for _, part := range []string{"sqlstate", "pq:", "pgx", "sqlite", "database", "syntax"} {
		if strings.Contains(strings.ToLower(message), part) {
			t.Errorf("validation error leaked database details: %q", message)
		}
	}
}

func mcpNonCanonicalAssetIDs() map[string]string {
	return map[string]string{
		"not_uuid":     "not-a-uuid",
		"urn":          "urn:uuid:" + mcpValidationAssetID,
		"braces":       "{" + mcpValidationAssetID + "}",
		"unhyphenated": strings.ReplaceAll(mcpValidationAssetID, "-", ""),
	}
}

func TestMCPAssetIDsRequireCanonicalUUID(t *testing.T) {
	runtime := newRuntime(t)
	server := testServer(t, runtime)
	cookie, csrf := authenticateInitialAdmin(t, server, runtime)
	secret := createAPIKeySecret(t, server, cookie, csrf, "mcp-uuid", "mcp.access", "assets.read", "relations.read")
	insertMCPValidationAsset(t, server, mcpValidationAssetID, "uuid-asset")
	peerID := "bcdefabc-defa-4bcd-8efa-bcdefabcdefa"
	insertMCPValidationAsset(t, server, peerID, "uuid-peer")
	for _, edge := range []struct {
		id, source, target, relationType string
		ended                            any
	}{
		{"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa", mcpValidationAssetID, peerID, "runs_on", nil},
		{"bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb", peerID, mcpValidationAssetID, "runs_on", nil},
		{"cccccccc-cccc-4ccc-8ccc-cccccccccccc", mcpValidationAssetID, peerID, "depends_on", time.Now().UTC()},
	} {
		if _, err := server.database.DB().Exec(`INSERT INTO asset_relations
			(id,source_asset_id,relation_type,target_asset_id,source,confidence,valid_to)
			VALUES($1,$2,$3,$4,'manual',1.0,$5)`, edge.id, edge.source, edge.relationType, edge.target, edge.ended); err != nil {
			t.Fatal(err)
		}
	}
	for _, protocol := range []string{"legacy", "modern"} {
		t.Run(protocol, func(t *testing.T) {
			for _, tool := range []string{"asset_get", "asset_relations"} {
				t.Run(tool, func(t *testing.T) {
					baseline := callMCPValidationTool(t, server, secret, tool, mcpValidationAssetID, protocol == "modern")
					if baseline["isError"] != false {
						t.Fatalf("canonical lookup = %#v", baseline)
					}
					data := baseline["structuredContent"].(map[string]any)
					if tool == "asset_get" {
						if asset := data["asset"].(map[string]any); asset["id"] != mcpValidationAssetID {
							t.Fatalf("asset = %#v", asset)
						}
					} else {
						edges := data["relations"].([]any)
						if len(edges) != 2 || data["count"] != float64(2) || data["has_more"] != false {
							t.Fatalf("active relation page = %#v", data)
						}
						for i, wantSource := range []string{mcpValidationAssetID, peerID} {
							edge := edges[i].(map[string]any)
							wantTarget := peerID
							if i == 1 {
								wantTarget = mcpValidationAssetID
							}
							if edge["source_asset_id"] != wantSource || edge["target_asset_id"] != wantTarget || edge["valid_to"] != nil {
								t.Fatalf("active edge = %#v", edge)
							}
						}
					}
					for name, id := range map[string]string{
						"lower": mcpValidationAssetID, "upper": strings.ToUpper(mcpValidationAssetID),
						"mixed":   "aBcDeFaB-cDeF-4aBc-8dEf-AbCdEfAbCdEf",
						"trimmed": " \t" + strings.ToUpper(mcpValidationAssetID) + "\r\n",
					} {
						t.Run(name, func(t *testing.T) {
							result := callMCPValidationTool(t, server, secret, tool, id, protocol == "modern")
							if result["isError"] != false || !reflect.DeepEqual(result["structuredContent"], data) {
								t.Fatalf("canonical-equivalent lookup differs: %#v", result)
							}
						})
					}
					for name, id := range mcpNonCanonicalAssetIDs() {
						t.Run(name, func(t *testing.T) {
							result := callMCPValidationTool(t, server, secret, tool, id, protocol == "modern")
							assertMCPValidationError(t, tool, mcpValidationErrorText(t, result))
						})
					}
					t.Run("missing", func(t *testing.T) {
						result := callMCPValidationTool(t, server, secret, tool, "dddddddd-dddd-4ddd-8ddd-dddddddddddd", protocol == "modern")
						if tool == "asset_get" {
							if message := mcpValidationErrorText(t, result); message != "asset not found" {
								t.Fatalf("missing asset error = %q", message)
							}
						} else {
							want := map[string]any{"relations": []any{}, "limit": float64(50), "offset": float64(0),
								"count": float64(0), "has_more": false, "next_offset": float64(0)}
							if result["isError"] != false || !reflect.DeepEqual(result["structuredContent"], want) {
								t.Fatalf("missing asset relation page = %#v", result)
							}
						}
					})
				})
			}
		})
	}
}

func TestMCPAssetGetCanonicalUUIDFindsMergedAsset(t *testing.T) {
	runtime := newRuntime(t)
	server := testServer(t, runtime)
	cookie, csrf := authenticateInitialAdmin(t, server, runtime)
	secret := createAPIKeySecret(t, server, cookie, csrf, "mcp-merged-uuid", "mcp.access", "assets.read", "relations.read")
	primaryID := "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee"
	insertMCPValidationAsset(t, server, primaryID, "uuid-primary")
	insertMCPValidationAsset(t, server, mcpValidationAssetID, "uuid-secondary")
	mergeAssetsThroughREST(t, server, cookie, csrf, primaryID, mcpValidationAssetID)
	for _, id := range []string{mcpValidationAssetID, strings.ToUpper(mcpValidationAssetID)} {
		t.Run(id, func(t *testing.T) {
			result := callMCPValidationTool(t, server, secret, "asset_get", id, false)
			if result["isError"] != false {
				t.Fatalf("merged asset lookup = %#v", result)
			}
			data := result["structuredContent"].(map[string]any)
			if data["merged_into"] != primaryID || data["asset"] != nil {
				t.Fatalf("merge hint = %#v", data)
			}
		})
	}
}

func TestMCPAssetIDsRejectNonCanonicalUUIDBeforeQuery(t *testing.T) {
	runtime := newRuntime(t)
	server := testServer(t, runtime)
	if err := runtime.Close(); err != nil {
		t.Fatal(err)
	}
	// A closed real database makes any attempted SQL fail, even for spellings
	// that a live PostgreSQL UUID column would silently accept.
	for tool, handler := range map[string]func(*http.Request, *mcpArguments) (any, error){
		"asset_get": server.mcpAssetGet, "asset_relations": server.mcpAssetRelations,
	} {
		for name, id := range mcpNonCanonicalAssetIDs() {
			t.Run(tool+"/"+name, func(t *testing.T) {
				raw, err := json.Marshal(map[string]any{"asset_id": id})
				if err != nil {
					t.Fatal(err)
				}
				_, err = handler(httptest.NewRequest(http.MethodPost, "/mcp", nil), mcpArgumentsForTest(tool, raw))
				if err == nil {
					t.Fatal("noncanonical UUID accepted")
				}
				assertMCPValidationError(t, tool, err.Error())
			})
		}
	}
}
