import {renderToStaticMarkup} from "react-dom/server";
import {describe, expect, it} from "vitest";
import {
  MAIL_EVENTS,
  MailDeliveryTable,
  Notice,
  SystemSettingsInfo,
  TRACKING_PROVIDERS,
  TrackingViolationList,
  type TrackingViolation,
} from "./adminPages";
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

describe("TrackingViolationList", () => {
  const blocked: TrackingViolation = {
    origin: "https://momento.corp.example",
    directive: "connect-src",
    page: "https://invenqor.corp.example/",
    count: 12,
    first_seen: "2026-09-12T08:00:00Z",
    last_seen: "2026-09-12T08:05:00Z",
    allowed: false,
  };

  it("offers the one-click allow only for an origin the policy still blocks", () => {
    const markup = renderToStaticMarkup(
      <TrackingViolationList
        items={[blocked, {...blocked, directive: "script-src", allowed: true}]}
        canWrite busy={false} onAllow={() => undefined}
      />,
    );
    noUndefined(markup);
    expect(markup).toContain("https://momento.corp.example");
    expect(markup).toContain("connect-src");
    expect(markup).toContain("12회");
    expect(markup).toContain("허용됨");
    // One row is already allowed, so exactly one button remains.
    expect(markup.split("허용 목록에 추가").length - 1).toBe(1);
  });

  it("keeps the button visible but disabled for settings.read", () => {
    const markup = renderToStaticMarkup(
      <TrackingViolationList items={[blocked]} canWrite={false} busy={false} onAllow={() => undefined}/>,
    );
    expect(markup).toContain("허용 목록에 추가");
    expect(markup).toContain('aria-disabled="true"');
  });

  it("says so when nothing was blocked", () => {
    const markup = renderToStaticMarkup(
      <TrackingViolationList items={[]} canWrite busy={false} onAllow={() => undefined}/>,
    );
    expect(markup).toContain("차단 신고가 없습니다");
  });
});

describe("TRACKING_PROVIDERS", () => {
  // The self-hosted collector is the only choice that keeps data inside, so
  // it is the first thing an administrator sees.
  it("lists Momento first and never offers 'none' as a provider card", () => {
    expect(TRACKING_PROVIDERS[0].value).toBe("momento");
    expect(TRACKING_PROVIDERS.map(option => option.value)).not.toContain("none");
  });
});

describe("MailDeliveryTable", () => {
  it("renders the empty log and a log with a failure", () => {
    const empty = renderToStaticMarkup(<MailDeliveryTable page={{items: [], summary: {total: 0, status: {}}}}/>);
    expect(empty).toContain("시험 발송");
    const markup = renderToStaticMarkup(<MailDeliveryTable page={{
      items: [
        {id: "1", event: "account.locked", recipient: "ops@corp.example", subject: "[Invenqor] ops 계정이 잠겼습니다",
          actor_id: "", status: "failed", attempts: 2, error_message: "SMTP connect to relay:25 failed: connection refused",
          created_at: "2026-09-13T12:00:00Z", updated_at: "2026-09-13T12:00:05Z"},
        {id: "2", event: "test", recipient: "admin@corp.example", subject: "[Invenqor] SMTP 시험 발송",
          actor_id: "u1", status: "sent", attempts: 1, error_message: "",
          created_at: "2026-09-13T12:01:00Z", updated_at: "2026-09-13T12:01:01Z"},
      ],
      summary: {total: 2, status: {failed: 1, sent: 1}},
    }}/>);
    noUndefined(markup);
    expect(markup).toContain("badge bad");
    expect(markup).toContain("badge good");
    expect(markup).toContain("connection refused");
    expect(markup).not.toContain("undefined");
  });

  // Every switchable event the Server offers has a title the operator can
  // read; a bare event name would be the first thing a new installation shows.
  it("describes every event the Server sends", () => {
    for (const name of ["account.locked", "account.unlocked", "account.created", "agent_update.halted"]) {
      expect(MAIL_EVENTS[name]?.title).toBeTruthy();
    }
  });
});
