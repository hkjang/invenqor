import {renderToStaticMarkup} from "react-dom/server";
import {describe, expect, it} from "vitest";
import {
  AssetTable,
  Breakdown,
  DailyBars,
  RiskSummary,
  type Asset,
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
