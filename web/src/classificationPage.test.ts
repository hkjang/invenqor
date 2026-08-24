import React from "react";
import {renderToStaticMarkup} from "react-dom/server";
import {describe, expect, it} from "vitest";
import {
  CLASSIFICATION_READ_ONLY_MESSAGE,
  RELATIONS_READ_ONLY_MESSAGE,
  ClassificationSettingsPanel,
  classificationAccessState,
  describeAssign,
  describeMatch,
} from "./classificationPage";

const rule = (match: unknown, assign: unknown) => ({
  id: "r", name: "n", description: "", priority: 10, enabled: true,
  system_rule: true, confidence: 1, assets: 0,
  match: match as never, assign: assign as never,
});

describe("classification mutation permissions", () => {
  it("separates settings.write from relations.write", () => {
    expect(classificationAccessState({
      permissions: ["settings.read", "settings.write"],
      superAdmin: false,
    })).toEqual({canWriteSettings: true, canWriteRelations: false});
    expect(classificationAccessState({
      permissions: ["settings.read", "relations.write"],
      superAdmin: false,
    })).toEqual({canWriteSettings: false, canWriteRelations: true});
    expect(classificationAccessState({permissions: [], superAdmin: true}))
      .toEqual({canWriteSettings: true, canWriteRelations: true});
  });

  it("keeps refresh readable while disabling classification mutations", () => {
    const html = renderToStaticMarkup(React.createElement(ClassificationSettingsPanel, {
      csrf: "csrf",
      access: {permissions: ["settings.read"], superAdmin: false},
    }));
    expect(html).toContain(CLASSIFICATION_READ_ONLY_MESSAGE);
    expect(html).toContain(RELATIONS_READ_ONLY_MESSAGE);
    expect(html).toContain('id="classification-readonly-notice"');
    expect(html).toContain('id="relations-readonly-notice"');
    expect(html).toMatch(/<button class="secondary">[^]*새로고침<\/button>/);
    expect(html).toContain(`title="${CLASSIFICATION_READ_ONLY_MESSAGE}"`);
    expect(html).toContain('aria-disabled="true"');
  });
});

describe("describeMatch", () => {
  it("reads a predicate back as one line an operator can check", () => {
    const text = describeMatch(rule(
      {categories: ["service"], name_patterns: ["postgres*", "*mysqld*"]},
      {},
    ));
    expect(text).toContain("수집범주 ∈ service");
    expect(text).toContain("postgres* | *mysqld*");
    expect(text).toContain("∧");
  });

  it("separates token matching from wildcard matching", () => {
    // The distinction matters: '*stg*' also matches 'postgresql'.
    expect(describeMatch(rule({name_tokens: ["prd", "prod"]}, {})))
      .toContain("이름 토큰 ∈ prd, prod");
  });

  it("says so when a rule matches everything", () => {
    expect(describeMatch(rule({}, {}))).toBe("모든 자산");
  });

  it("renders attribute predicates", () => {
    const text = describeMatch(rule(
      {attribute_equals: {"os_release.id": "ubuntu"},
       attribute_contains: {"os_release.version_id": "24."}},
      {},
    ));
    expect(text).toContain("os_release.id = ubuntu");
    expect(text).toContain("os_release.version_id ⊃ 24.");
  });
});

describe("describeAssign", () => {
  it("lists every field a rule sets, including the relationship", () => {
    const text = describeAssign(rule({}, {
      type: "database", tags: ["data-tier"], relate_to_host: true,
      relation: "runs_on",
    }));
    expect(text).toContain("유형 = database");
    expect(text).toContain("태그 + data-tier");
    expect(text).toContain("호스트 관계 = runs_on");
  });

  it("defaults the relationship label when a rule omits it", () => {
    expect(describeAssign(rule({}, {relate_to_host: true})))
      .toContain("호스트 관계 = runs_on");
  });

  it("says so when a rule assigns nothing", () => {
    expect(describeAssign(rule({}, {}))).toBe("변경 없음");
  });
});
