package httpapi

import (
	"encoding/json"
	"net/http/httptest"
	"testing"
	"time"
)

// The MCP tools are read by a model, and a model believes what the page tells
// it. `has_more` used to be `len(items) == limit`, which is a guess, not an
// answer: a result that happens to end exactly on the page boundary reported
// that more was waiting, so the caller asked once more and was handed an empty
// page. Reading one row past the page turns the guess into a fact.

type mcpAssetPage struct {
	Items      []assetView `json:"items"`
	Limit      int         `json:"limit"`
	Offset     int         `json:"offset"`
	HasMore    bool        `json:"has_more"`
	NextOffset int         `json:"next_offset"`
}

func mcpAssetSearchPage(t *testing.T, server *Server, raw string) mcpAssetPage {
	t.Helper()
	result, err := server.mcpAssetSearch(
		httptest.NewRequest("POST", "/mcp", nil),
		mcpArgumentsForTest("asset_search", []byte(raw)),
	)
	if err != nil {
		t.Fatal(err)
	}
	encoded, _ := json.Marshal(result)
	var page mcpAssetPage
	if err := json.Unmarshal(encoded, &page); err != nil {
		t.Fatal(err)
	}
	return page
}

func TestMCPAssetSearchReportsWhetherAnotherPageExists(t *testing.T) {
	runtime := newRuntime(t)
	server := testServer(t, runtime)
	now := time.Now().UTC()
	for index, name := range []string{"page-host-a", "page-host-b", "page-host-c"} {
		insertSoftwareTestAsset(t, server, name, name, "host", `{}`, 1,
			now.Add(-time.Duration(index)*time.Minute))
	}

	// Three rows read three at a time is a complete answer, even though the page
	// is full.
	exact := mcpAssetSearchPage(t, server, `{"limit":3}`)
	if len(exact.Items) != 3 || exact.HasMore || exact.NextOffset != 3 {
		t.Fatalf("a result ending exactly on the page boundary = %+v", exact)
	}

	first := mcpAssetSearchPage(t, server, `{"limit":2}`)
	if len(first.Items) != 2 || !first.HasMore || first.NextOffset != 2 {
		t.Fatalf("first page of three rows = %+v", first)
	}
	// The lookahead row must be dropped, not returned as a fourth item on a
	// two-row page.
	if first.Items[0].Name != "page-host-a" || first.Items[1].Name != "page-host-b" {
		t.Fatalf("first page items = %+v", first.Items)
	}

	last := mcpAssetSearchPage(t, server, `{"limit":2,"offset":2}`)
	if len(last.Items) != 1 || last.HasMore ||
		last.Items[0].Name != "page-host-c" {
		t.Fatalf("last page of three rows = %+v", last)
	}
}

type mcpAgentPage struct {
	Agents []struct {
		Hostname string `json:"hostname"`
	} `json:"agents"`
	Limit      int  `json:"limit"`
	Offset     int  `json:"offset"`
	Count      int  `json:"count"`
	HasMore    bool `json:"has_more"`
	NextOffset int  `json:"next_offset"`
}

func mcpAgentsListPage(t *testing.T, server *Server, raw string) mcpAgentPage {
	t.Helper()
	result, err := server.mcpAgentsList(
		httptest.NewRequest("POST", "/mcp", nil),
		mcpArgumentsForTest("agents_list", []byte(raw)),
	)
	if err != nil {
		t.Fatal(err)
	}
	encoded, _ := json.Marshal(result)
	var page mcpAgentPage
	if err := json.Unmarshal(encoded, &page); err != nil {
		t.Fatal(err)
	}
	return page
}

// agents_list promised a next page it had no way to serve: it reported
// `has_more` from the page size and accepted no offset at all, so a caller told
// there was more could only ask the identical question again.
func TestMCPAgentsListPagesPastTheFirstPage(t *testing.T) {
	runtime := newRuntime(t)
	server := testServer(t, runtime)
	now := time.Now().UTC()
	for index, name := range []string{"page-agent-a", "page-agent-b", "page-agent-c"} {
		insertSoftwareTestAsset(t, server, name, name, "host", `{}`, 1,
			now.Add(-time.Duration(index)*time.Minute))
	}

	exact := mcpAgentsListPage(t, server, `{"limit":3}`)
	if exact.Count != 3 || exact.HasMore || exact.NextOffset != 3 {
		t.Fatalf("a result ending exactly on the page boundary = %+v", exact)
	}

	first := mcpAgentsListPage(t, server, `{"limit":2}`)
	if first.Count != 2 || len(first.Agents) != 2 || !first.HasMore ||
		first.NextOffset != 2 {
		t.Fatalf("first page of three agents = %+v", first)
	}
	if first.Agents[0].Hostname != "page-agent-a" ||
		first.Agents[1].Hostname != "page-agent-b" {
		t.Fatalf("first page agents = %+v", first.Agents)
	}

	// The offset the first page hands back must actually reach the remainder.
	last := mcpAgentsListPage(t, server, `{"limit":2,"offset":2}`)
	if last.Count != 1 || last.HasMore || last.Offset != 2 ||
		last.Agents[0].Hostname != "page-agent-c" {
		t.Fatalf("page after next_offset = %+v", last)
	}
}

// The offset only reaches the remainder because the tool advertises it; a
// parameter the schema omits is rejected before the handler ever sees it.
func TestMCPAgentsListAdvertisesItsOffset(t *testing.T) {
	for _, tool := range mcpTools {
		if tool.Name != "agents_list" {
			continue
		}
		properties, _ := tool.InputSchema["properties"].(map[string]any)
		if _, declared := properties["offset"]; !declared {
			t.Fatal("agents_list reports has_more but declares no offset to act on it")
		}
		return
	}
	t.Fatal("agents_list is not offered")
}
