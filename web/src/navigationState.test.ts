import { describe, expect, it } from "vitest";
import { consoleHash, parseConsoleHash } from "./navigationState";

describe("console navigation state", () => {
  it("round-trips the selected main and settings menus through the URL", () => {
    expect(parseConsoleHash(consoleHash("assets"))).toMatchObject({
      page: "assets",
      settingsTab: undefined,
    });
    expect(parseConsoleHash(consoleHash("settings", "keycloak"))).toMatchObject({
      page: "settings",
      settingsTab: "keycloak",
    });
    expect(parseConsoleHash(consoleHash("software")).page).toBe("software");
  });

  it("ignores unknown routes and invalid settings tabs", () => {
    expect(parseConsoleHash("#/unknown")).toMatchObject({
      page: undefined,
      settingsTab: undefined,
    });
    expect(parseConsoleHash("#/settings/unknown")).toMatchObject({
      page: "settings",
      settingsTab: undefined,
    });
  });
});

describe("parseConsoleHash with parameters", () => {
  it("keeps the page name when the hash carries a query", () => {
    // An audit row links to "#/logs?request_id=…". Treating the whole segment as
    // the page name matched nothing, so the link did not navigate at all.
    const parsed = parseConsoleHash("#/logs?request_id=abc-123");
    expect(parsed.page).toBe("logs");
    expect(parsed.query.get("request_id")).toBe("abc-123");
  });

  it("still reads a settings tab, and rejects an unknown page", () => {
    expect(parseConsoleHash("#/settings/keycloak?x=1").settingsTab).toBe("keycloak");
    expect(parseConsoleHash("#/nowhere?x=1").page).toBeUndefined();
    expect(parseConsoleHash("#/assets").query.toString()).toBe("");
  });
});
