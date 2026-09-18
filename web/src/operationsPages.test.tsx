import {renderToStaticMarkup} from "react-dom/server";
import {describe, expect, it} from "vitest";
import {
  AssetTable,
  Breakdown,
  DailyBars,
  DiagnosticDetail,
  QueryResultPanel,
  RiskSummary,
  type Asset,
  type DiagnosticLog,
  type QueryResult,
  type Statistics,
} from "./operationsPages";

const asset: Asset = {
  id: "11111111-1111-4111-8111-111111111111",
  asset_key: "host:web-01",
  name: "web-01",
  type: "host",
  status: "active",
  criticality: "critical",
  environment: "production",
  owner_department: "platform",
  location: "seoul",
  confidence: 0.92,
  attributes: {os_name: "Ubuntu"},
  custom_fields: {},
  source: "agent",
  first_seen_at: "2026-08-01T00:00:00Z",
  last_seen_at: "2026-08-26T00:00:00Z",
};

// Every optional or free-text field left blank. An asset created by hand, or
// one an Agent reported before classification filled anything in, looks like
// this - and blank is what the table has to render rather than "undefined".
const bare: Asset = {
  ...asset,
  id: "22222222-2222-4222-8222-222222222222",
  asset_key: "host:unnamed",
  name: "",
  criticality: "",
  environment: "",
  owner_department: "",
  location: "",
  confidence: 0,
  attributes: {},
  deleted_at: null,
};

const statistics: Statistics = {
  generated_at: "2026-08-26T00:00:00Z",
  assets: {
    total: 2, seen_24h: 1, stale: 1,
    by_type: [{label: "host", count: 2}],
    by_status: [{label: "active", count: 2}],
    by_environment: [{label: "production", count: 2}],
    by_criticality: [{label: "critical", count: 1}],
    by_source: [{label: "agent", count: 2}],
  },
  agents: {
    total: 1, healthy: 1, attention: 0,
    by_status: [{label: "active", count: 1}],
    by_os: [{label: "linux", count: 1}],
  },
  collection: {
    events_24h: 4, failed_24h: 1,
    daily: [{date: "2026-08-25", events: 4, failed: 1}],
  },
};

const emptyStatistics: Statistics = {
  ...statistics,
  assets: {
    total: 0, seen_24h: 0, stale: 0,
    by_type: [], by_status: [], by_environment: [],
    by_criticality: [], by_source: [],
  },
  agents: {total: 0, healthy: 0, attention: 0, by_status: [], by_os: []},
  collection: {events_24h: 0, failed_24h: 0, daily: []},
};

const noop = () => {};

// A throw inside render is a blank page: React unmounts the tree and the user
// sees nothing, with no message to report.
//
// The length check is what stops the rest from being vacuous. Every other
// assertion here is an absence, and a component that rendered nothing at all
// would satisfy all of them - which is the same blank page by a quieter route.
const clean = (markup: string) => {
  expect(markup.length).toBeGreaterThan(100);
  expect(markup).not.toContain("undefined");
  expect(markup).not.toContain("NaN");
  expect(markup).not.toContain("Infinity");
  expect(markup).not.toContain("null");
};

describe("AssetTable", () => {
  it("renders no assets at all", () => {
    clean(renderToStaticMarkup(
      <AssetTable items={[]} selected={[]} onToggle={noop} onSelect={noop}/>,
    ));
  });

  it("renders an asset whose optional fields are all blank", () => {
    const markup = renderToStaticMarkup(
      <AssetTable items={[bare]} selected={[]} onToggle={noop} onSelect={noop}/>,
    );
    clean(markup);
    expect(markup).toContain("host:unnamed");
  });

  it("renders a fully populated asset, selected", () => {
    const markup = renderToStaticMarkup(
      <AssetTable
        items={[asset, bare]} selected={[asset.id]}
        onToggle={noop} onSelect={noop}
      />,
    );
    clean(markup);
    expect(markup).toContain("web-01");
  });
});

describe("RiskSummary", () => {
  // null is the state before the first response arrives, and the one a failed
  // request leaves behind.
  it("renders before statistics have loaded", () => {
    clean(renderToStaticMarkup(<RiskSummary statistics={null}/>));
  });

  it("renders a new installation with nothing collected", () => {
    clean(renderToStaticMarkup(<RiskSummary statistics={emptyStatistics}/>));
  });

  it("renders real statistics", () => {
    clean(renderToStaticMarkup(<RiskSummary statistics={statistics}/>));
  });
});

describe("Breakdown and DailyBars", () => {
  it("render empty inputs", () => {
    clean(renderToStaticMarkup(<Breakdown items={[]}/>));
    clean(renderToStaticMarkup(<DailyBars items={[]}/>));
  });

  it("render populated inputs", () => {
    clean(renderToStaticMarkup(<Breakdown items={statistics.assets.by_type}/>));
    clean(renderToStaticMarkup(<DailyBars items={statistics.collection.daily}/>));
  });
});

const diagnostic: DiagnosticLog = {
  id: "55555555-5555-4555-8555-555555555555",
  occurred_at: "2026-08-26T00:00:00Z",
  level: "warning",
  component: "agent_enrollment",
  event_code: "AGENT_ALREADY_CLAIMED",
  message: "The agent identifier is already bound to another device claim.",
  request_id: "pod-a/abc123-000042",
  instance_id: "pod-a",
  agent_id: "agent-1",
  source_ip: "10.0.0.9",
  details: {
    path: "/v1/agent/enroll",
    agent_version: "0.2.18",
    remediation: "Delete agent-id and enrollment-claim.json on that host.",
  },
};

describe("DiagnosticDetail", () => {
  // The Server computes the fix for several of these codes. Buried in the JSON
  // dump it is the least visible field on screen, and it is the only one that
  // tells the reader what to do.
  it("gives the Server's remediation a row of its own", () => {
    const markup = renderToStaticMarkup(
      <DiagnosticDetail item={diagnostic} onFilterByRequest={noop}/>,
    );
    clean(markup);
    expect(markup).toContain("조치");
    expect(markup).toContain("Delete agent-id and enrollment-claim.json");
    expect(markup).toContain("pod-a/abc123-000042");
  });

  it("omits the row when the event carries no remediation", () => {
    const markup = renderToStaticMarkup(
      <DiagnosticDetail
        item={{...diagnostic, details: {path: "/health/ready"}}}
        onFilterByRequest={noop}
      />,
    );
    clean(markup);
    expect(markup).not.toContain("조치");
  });

  it("renders an event with no request ID, agent or source", () => {
    const markup = renderToStaticMarkup(
      <DiagnosticDetail
        item={{...diagnostic, request_id: "", agent_id: "", source_ip: "", details: {}}}
        onFilterByRequest={noop}
      />,
    );
    clean(markup);
    // No request ID means no cross-link to events sharing one.
    expect(markup).not.toContain("같은 request ID");
  });
});

// Shaped like a reply from POST /api/v1/query/execute: the page the Server
// read plus the size of the whole answer and whether rows exist past it.
const page = (result: Partial<QueryResult>): QueryResult => ({
  items: [asset, bare], total: 2, offset: 0, limit: 100, has_more: false, ...result,
});

describe("QueryResultPanel", () => {
  it("renders before the first run", () => {
    const markup = renderToStaticMarkup(
      <QueryResultPanel result={page({items: [], total: 0})} ran={false} failed={false}
        onPrevious={noop} onNext={noop}/>,
    );
    clean(markup);
    expect(markup).not.toContain("조건에 맞는 자산이 없습니다");
    expect(markup).not.toContain("다음");
  });

  it("renders a result that fits in one page", () => {
    const markup = renderToStaticMarkup(
      <QueryResultPanel result={page({})} ran={true} failed={false}
        onPrevious={noop} onNext={noop}/>,
    );
    clean(markup);
    expect(markup).toContain("결과 2건");
    expect(markup).toContain("1–2 표시 · limit 100");
    expect(markup).not.toContain("만 표시합니다");
    expect(markup).not.toContain("다음");
  });

  // 1,200 stale hosts and a limit of 100: the title used to say 100 and the
  // only advice was to narrow the expression.
  it("names the whole answer and offers the next page when rows exist past the limit", () => {
    const markup = renderToStaticMarkup(
      <QueryResultPanel result={page({total: 1200, limit: 100, has_more: true})}
        ran={true} failed={false} onPrevious={noop} onNext={noop}/>,
    );
    clean(markup);
    expect(markup).toContain("결과 1,200건");
    expect(markup).toContain("1–2 표시 · limit 100");
    expect(markup).toContain("1,200건 중 1–2번째만 표시합니다");
    expect(markup).toMatch(/<button[^>]*disabled[^>]*>이전<\/button>/);
    expect(markup).toMatch(/<button class="secondary">다음<\/button>/);
  });

  it("counts from the Server's offset on a later page and lets the reader go back", () => {
    const markup = renderToStaticMarkup(
      <QueryResultPanel result={page({total: 1200, offset: 1198, limit: 100, has_more: false})}
        ran={true} failed={false} onPrevious={noop} onNext={noop}/>,
    );
    clean(markup);
    expect(markup).toContain("결과 1,200건");
    expect(markup).toContain("1,199–1,200 표시");
    expect(markup).toMatch(/<button class="secondary">이전<\/button>/);
    expect(markup).toMatch(/<button[^>]*disabled[^>]*>다음<\/button>/);
  });

  it("tells an empty page past the end apart from no match at all", () => {
    const beyond = renderToStaticMarkup(
      <QueryResultPanel result={page({items: [], total: 3, offset: 100, limit: 100})}
        ran={true} failed={false} onPrevious={noop} onNext={noop}/>,
    );
    clean(beyond);
    expect(beyond).toContain("결과 3건");
    expect(beyond).toContain("이 페이지에는 자산이 없습니다");
    expect(beyond).not.toContain("조건에 맞는 자산이 없습니다");
    expect(beyond).toMatch(/<button class="secondary">이전<\/button>/);

    const none = renderToStaticMarkup(
      <QueryResultPanel result={page({items: [], total: 0})}
        ran={true} failed={false} onPrevious={noop} onNext={noop}/>,
    );
    clean(none);
    expect(none).toContain("결과 0건");
    expect(none).toContain("조건에 맞는 자산이 없습니다");
    expect(none).not.toContain("이 페이지에는 자산이 없습니다");
    expect(none).not.toContain("이전");
  });

  it("shows no count for a run the Server rejected", () => {
    const markup = renderToStaticMarkup(
      <QueryResultPanel result={page({items: [], total: 0})} ran={true} failed={true}
        onPrevious={noop} onNext={noop}/>,
    );
    clean(markup);
    expect(markup).not.toContain("결과 0건");
    expect(markup).not.toContain("조건에 맞는 자산이 없습니다");
  });
});
