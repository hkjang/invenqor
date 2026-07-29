import { describe, expect, it } from "vitest";
import {
  formatMappings,
  normalizeKeycloakSettings,
  parseMappings,
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
