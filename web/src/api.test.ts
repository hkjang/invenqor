import {afterEach, describe, expect, it, vi} from "vitest";
import {APIError, api} from "./api";

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

  it("replaces a stale caller token with the current session cookie after SSO", async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(JSON.stringify({saved: true}), {
        status: 200,
        headers: {"Content-Type": "application/json"},
      }),
    );
    vi.stubGlobal("document", {cookie: "invenqor_csrf=oidc-current-token"});
    vi.stubGlobal("fetch", fetchMock);

    await api("/api/v1/admin/settings", {
      method: "PATCH",
      headers: {"X-CSRF-Token": "expired-local-session-token"},
    });

    const request = fetchMock.mock.calls[0][1] as RequestInit;
    expect(new Headers(request.headers).get("X-CSRF-Token")).toBe("oidc-current-token");
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

  it("keeps the HTTP status when an ingress returns a non-JSON error page", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(
      new Response("<html>bad gateway</html>", {
        status: 502,
        headers: {"Content-Type": "text/html"},
      }),
    ));

    const error = await api("/api/v1/admin/settings").catch(reason => reason);

    expect(error).toBeInstanceOf(APIError);
    expect(error).toMatchObject({status: 502, body: "<html>bad gateway</html>"});
    expect((error as Error).message).toContain("HTTP 502");
  });

  it("reports a successful non-JSON response as an API routing error", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(
      new Response("<!doctype html><title>console</title>", {
        status: 200,
        headers: {"Content-Type": "text/html"},
      }),
    ));

    await expect(api("/api/v1/assets")).rejects.toMatchObject({
      name: "APIError",
      status: 200,
      message: expect.stringContaining("API/Ingress"),
    });
  });
});
