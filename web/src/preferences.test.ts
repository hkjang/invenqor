import {describe, expect, it} from "vitest";
import {defaultPreferences, normalizePreferences} from "./preferences";

describe("personalization preferences", () => {
  it("uses safe defaults for invalid persisted values", () => {
    expect(normalizePreferences({
      theme: "neon",
      density: "tiny",
      dashboard_refresh_seconds: 7,
    } as never)).toEqual(defaultPreferences);
  });

  it("keeps supported user choices", () => {
    expect(normalizePreferences({
      theme: "dark",
      density: "compact",
      start_page: "assets",
      dashboard_refresh_seconds: 300,
      reduce_motion: true,
    })).toMatchObject({
      theme: "dark",
      density: "compact",
      start_page: "assets",
      dashboard_refresh_seconds: 300,
      reduce_motion: true,
    });
  });
});
