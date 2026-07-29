import { describe, expect, it } from "vitest";
import { consoleHash, parseConsoleHash } from "./navigationState";

describe("console navigation state", () => {
  it("round-trips the selected main and settings menus through the URL", () => {
    expect(parseConsoleHash(consoleHash("assets"))).toEqual({
      page: "assets",
      settingsTab: undefined,
    });
    expect(parseConsoleHash(consoleHash("settings", "keycloak"))).toEqual({
      page: "settings",
      settingsTab: "keycloak",
    });
  });

  it("ignores unknown routes and invalid settings tabs", () => {
    expect(parseConsoleHash("#/unknown")).toEqual({
      page: undefined,
      settingsTab: undefined,
    });
    expect(parseConsoleHash("#/settings/unknown")).toEqual({
      page: "settings",
      settingsTab: undefined,
    });
  });
});
