import {renderToStaticMarkup} from "react-dom/server";
import {describe, expect, it} from "vitest";
import {Notice, SystemSettingsInfo} from "./adminPages";
import type {SystemInfo} from "./productVersion";

const info: SystemInfo = {
  database_mode: "POSTGRES_ACTIVE",
  server_version: "0.2.18",
  commit: "abcdef1234567890",
  build_time: "2026-08-26T00:00:00Z",
  agent_auto_enrollment: true,
  agent_enrollment_mode: "token",
  agent_enrollment_source: "database",
  agent_enrollment_policy_available: true,
  listen_address: "0.0.0.0:7070",
  port: 7070,
};

const noUndefined = (markup: string) => {
  expect(markup.length).toBeGreaterThan(100);
  expect(markup).not.toContain("undefined");
  expect(markup).not.toContain("NaN");
};

describe("SystemSettingsInfo", () => {
  // null is what the tab renders before the first response arrives, and what a
  // failed request leaves behind. Every row has to say something.
  it("renders before system info has loaded", () => {
    const markup = renderToStaticMarkup(<SystemSettingsInfo info={null}/>);
    noUndefined(markup);
    expect(markup).toContain("확인 중");
  });

  it("renders the running configuration", () => {
    const markup = renderToStaticMarkup(<SystemSettingsInfo info={info}/>);
    noUndefined(markup);
    expect(markup).toContain("0.2.18");
    expect(markup).toContain("0.0.0.0:7070");
    // The mode decides the wording, not the boolean beside it.
    expect(markup).toContain("공용 Token 보호");
  });

  it("reports the address from the port when only the port is known", () => {
    const markup = renderToStaticMarkup(
      <SystemSettingsInfo info={{...info, listen_address: undefined}}/>,
    );
    noUndefined(markup);
    expect(markup).toContain(":7070");
  });
});

describe("Notice", () => {
  // Failure-path text is only ever read by someone already in trouble, which is
  // why nothing else exercises it.
  for (const tone of ["info", "warning", "error"] as const) {
    it(`renders the ${tone} tone`, () => {
      const markup = renderToStaticMarkup(
        <Notice tone={tone} title="제목">본문</Notice>,
      );
      expect(markup).toContain("제목");
      expect(markup).toContain("본문");
      expect(markup).not.toContain("undefined");
    });
  }
});
