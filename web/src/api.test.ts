import {afterEach, describe, expect, it, vi} from "vitest";
import {api} from "./api";

describe("API client", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("adds the browser CSRF cookie to state-changing requests", async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(JSON.stringify({saved: true}), {
        status: 200,
        headers: {"Content-Type": "application/json"},
      }),
    );
    vi.stubGlobal("document", {cookie: "theme=dark; invenqor_csrf=csrf%2Btoken"});
    vi.stubGlobal("fetch", fetchMock);

    await api("/api/v1/admin/settings", {method: "PATCH"});

    const request = fetchMock.mock.calls[0][1] as RequestInit;
    expect(new Headers(request.headers).get("X-CSRF-Token")).toBe("csrf+token");
  });

  it("does not attach CSRF headers to read-only requests", async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(JSON.stringify({items: []}), {
        status: 200,
        headers: {"Content-Type": "application/json"},
      }),
    );
    vi.stubGlobal("document", {cookie: "invenqor_csrf=csrf-token"});
    vi.stubGlobal("fetch", fetchMock);

    await api("/api/v1/assets");

    const request = fetchMock.mock.calls[0][1] as RequestInit;
    expect(new Headers(request.headers).has("X-CSRF-Token")).toBe(false);
  });
});
