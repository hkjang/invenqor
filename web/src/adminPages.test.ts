import { describe, expect, it } from "vitest";
import {
  formatMappings,
  normalizeKeycloakSettings,
  parseMappings,
  parseNetworkEntries,
} from "./adminPages";

describe("Keycloak mapping editor", () => {
  it("parses mappings while ignoring blanks and comments", () => {
    expect(parseMappings(`
      # Keycloak realm roles
      inventory-admin = asset_manager

      inventory-read=viewer
    `)).toEqual({
      "inventory-admin": "asset_manager",
      "inventory-read": "viewer",
    });
  });

  it("rejects malformed or duplicate mappings", () => {
    expect(() => parseMappings("missing-separator", "역할 매핑"))
      .toThrow("역할 매핑 1행");
    expect(() => parseMappings("reader=viewer\nreader=auditor"))
      .toThrow("중복");
  });

  it("formats mappings deterministically", () => {
    expect(formatMappings({viewer: "viewer", admin: "super_admin"}))
      .toBe("admin=super_admin\nviewer=viewer");
  });

  it("normalizes nullable collections returned by older settings", () => {
    const settings = normalizeKeycloakSettings({
      scopes: null,
      allowed_email_domains: null,
      role_mappings: null,
      group_mappings: null,
    } as never);
    expect(settings.scopes).toEqual(["openid", "profile", "email"]);
    expect(settings.allowed_email_domains).toEqual([]);
    expect(settings.role_mappings).toEqual({});
    expect(settings.group_mappings).toEqual({});
  });
});

describe("Agent enrollment network editor", () => {
  it("accepts lines and commas, removes blanks and duplicates", () => {
    expect(parseNetworkEntries(
      "10.20.30.40\n10.20.0.0/16, 2001:db8::/64\n10.20.30.40",
    )).toEqual(["10.20.0.0/16", "10.20.30.40", "2001:db8::/64"]);
  });
});
