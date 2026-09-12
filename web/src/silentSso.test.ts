import {describe, expect, it} from "vitest";
import {
  clearSilentSsoState,
  currentReturnTo,
  markSignedOut,
  safeReturnTo,
  shouldAttemptSilentSso,
  silentSsoAttempted,
  silentSsoStartURL,
  type SilentSsoStorage,
} from "./silentSso";

const memoryStorage = (): SilentSsoStorage => {
  const values = new Map<string, string>();
  return {
    getItem: key => values.get(key) ?? null,
    setItem: (key, value) => { values.set(key, value); },
    removeItem: key => { values.delete(key); },
  };
};

const blockedStorage = (): SilentSsoStorage => {
  throw new Error("SecurityError: sessionStorage is not available");
};

const input = (overrides: Partial<Parameters<typeof shouldAttemptSilentSso>[0]> = {}) => ({
  autoLogin: true,
  pathname: "/",
  search: "",
  storage: () => store,
  ...overrides,
});

let store = memoryStorage();

describe("silent SSO guards", () => {
  it("does nothing while auto_login is off", () => {
    store = memoryStorage();
    expect(shouldAttemptSilentSso(input({autoLogin: false}))).toBe(false);
    expect(shouldAttemptSilentSso(input())).toBe(true);
  });

  it("tries once per tab session, and not again after the attempt", () => {
    store = memoryStorage();
    expect(shouldAttemptSilentSso(input())).toBe(true);
    const url = silentSsoStartURL("/#/assets", () => store);
    expect(url).toBe("/api/v1/auth/keycloak/start?prompt=none&return_to=%2F%23%2Fassets");
    expect(silentSsoAttempted(() => store)).toBe(true);
    // A reload after the refusal lands here again; it must not start another.
    expect(shouldAttemptSilentSso(input())).toBe(false);
  });

  it("does not retry from the address the callback lands a refusal on", () => {
    store = memoryStorage();
    expect(shouldAttemptSilentSso(input({search: "?sso=none"}))).toBe(false);
    expect(shouldAttemptSilentSso(input({search: "?auth_error=KEYCLOAK_LOGIN_FAILED&request_id=x"}))).toBe(false);
    expect(shouldAttemptSilentSso(input({search: "?other=1"}))).toBe(true);
  });

  it("stays quiet after a deliberate sign-out until a session exists again", () => {
    store = memoryStorage();
    markSignedOut(() => store);
    expect(shouldAttemptSilentSso(input())).toBe(false);
    clearSilentSsoState(() => store);
    expect(shouldAttemptSilentSso(input())).toBe(true);
  });

  it("treats an unreadable store as an attempt already made", () => {
    expect(shouldAttemptSilentSso(input({storage: blockedStorage}))).toBe(false);
    // Writing to it must not throw either.
    expect(() => markSignedOut(blockedStorage)).not.toThrow();
    expect(() => clearSilentSsoState(blockedStorage)).not.toThrow();
    expect(() => silentSsoStartURL("/", blockedStorage)).not.toThrow();
  });

  it("never starts from Server-owned paths", () => {
    for (const pathname of ["/api/v1/auth/keycloak/callback", "/v1/agent/preflight", "/health/ready", "/mcp", "/momento/t.js"]) {
      store = memoryStorage();
      expect(shouldAttemptSilentSso(input({pathname}))).toBe(false);
    }
  });
});

describe("silent SSO return target", () => {
  it("carries the deep link the visitor opened", () => {
    expect(currentReturnTo({pathname: "/", search: "?x=1", hash: "#/assets"})).toBe("/?x=1#/assets");
  });

  it("only keeps same-origin paths", () => {
    expect(safeReturnTo("/#/audit")).toBe("/#/audit");
    expect(safeReturnTo("//evil.example/")).toBe("/");
    expect(safeReturnTo("https://evil.example/")).toBe("/");
    expect(safeReturnTo("/\\evil.example")).toBe("/");
    expect(safeReturnTo("")).toBe("/");
  });
});
