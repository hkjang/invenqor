import React from "react";
import {renderToStaticMarkup} from "react-dom/server";
import { describe, expect, it } from "vitest";
import {
  SETTINGS_READ_ONLY_MESSAGE,
  USERS_READ_ONLY_MESSAGE,
  SettingsPage,
  UsersPage,
  canAdmin,
  formatMappings,
  normalizeKeycloakSettings,
  parseMappings,
  parseNetworkEntries,
} from "./adminPages";

const readOnlyAccess = (permission: string) => ({
  permissions: [permission],
  superAdmin: false,
});

describe("admin mutation permissions", () => {
  it("renders settings.read as an explicit disabled read-only surface", () => {
    const html = renderToStaticMarkup(React.createElement(SettingsPage, {
      csrf: "csrf",
      systemInfo: null,
      userID: "reader",
      access: readOnlyAccess("settings.read"),
    }));

    expect(html).toContain('id="settings-readonly-notice"');
    expect(html).toContain(SETTINGS_READ_ONLY_MESSAGE);
    expect(html).toContain('aria-describedby="settings-readonly-notice"');
    expect(html).toContain(`title="${SETTINGS_READ_ONLY_MESSAGE}"`);
    expect(html).toContain("연결 테스트");
    expect(html).toContain('disabled="" aria-disabled="true"');
  });

  it("leaves the settings surface writable for settings.write and super admins", () => {
    const writer = renderToStaticMarkup(React.createElement(SettingsPage, {
      csrf: "csrf",
      systemInfo: null,
      userID: "writer",
      access: readOnlyAccess("settings.write"),
    }));
    expect(writer).not.toContain("settings-readonly-notice");
    expect(writer).not.toContain(SETTINGS_READ_ONLY_MESSAGE);
    expect(canAdmin({permissions: [], superAdmin: true}, "settings.write")).toBe(true);
  });

  it("keeps users.read searchable but disables every management entry point", () => {
    const html = renderToStaticMarkup(React.createElement(UsersPage, {
      csrf: "csrf",
      currentUserID: "reader",
      access: readOnlyAccess("users.read"),
    }));

    expect(html).toContain('id="users-readonly-notice"');
    expect(html).toContain(USERS_READ_ONLY_MESSAGE);
    expect(html).toContain('aria-describedby="users-readonly-notice"');
    expect(html).toContain(`title="${USERS_READ_ONLY_MESSAGE}"`);
    expect(html).toContain("사용자 검색");
    expect(html).toMatch(/<button class="primary compact" disabled="" aria-disabled="true"[^>]*>/);
  });

  it("enables user creation only for users.manage", () => {
    const html = renderToStaticMarkup(React.createElement(UsersPage, {
      csrf: "csrf",
      currentUserID: "manager",
      access: readOnlyAccess("users.manage"),
    }));
    expect(html).not.toContain("users-readonly-notice");
    expect(html).toContain('<button class="primary compact" aria-disabled="false">');
  });
});

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
