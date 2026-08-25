import {renderToStaticMarkup} from "react-dom/server";
import {describe, expect, it} from "vitest";
import {
  ChangeFlow,
  Composition,
  Freshness,
  Growth,
  RiskMatrix,
  Topology,
  type Visualization,
} from "./visualizationPage";

// A new installation has no assets, and it is the first thing anyone sees.
const empty: Visualization = {
  generated_at: "2026-08-26T00:00:00Z",
  window_days: 30,
  stale_hours: 24,
  totals: {assets: 0, stale: 0, fresh: 0, unowned: 0},
  dimensions: {},
  matrix: {rows: [], columns: [], cells: [], maximum: 0},
  freshness: [],
  hierarchy: [],
  flow: [],
  graph: {nodes: [], edges: [], truncated: false, total_relations: 0},
};

const populated: Visualization = {
  ...empty,
  totals: {assets: 3, stale: 1, fresh: 2, unowned: 1},
  dimensions: {environment: [{label: "production", count: 3}]},
  matrix: {
    rows: ["critical"],
    columns: ["production"],
    cells: [{row: "critical", column: "production", count: 3, stale: 1}],
    maximum: 3,
  },
  freshness: [{label: "24시간 이내", max_hours: 24, count: 2}],
  hierarchy: [{
    label: "production",
    count: 3,
    children: [{label: "host", count: 3}],
  }],
  flow: [{date: "2026-08-25", added: 2, removed: 1, total: 3}],
  graph: {
    nodes: [
      {
        id: "a", name: "web-01", type: "host", environment: "production",
        criticality: "critical", degree: 1, stale: false,
      },
      {
        id: "b", name: "nginx", type: "software_product",
        environment: "production", criticality: "high", degree: 1, stale: true,
      },
    ],
    edges: [{source: "a", target: "b", type: "runs_on"}],
    truncated: false,
    total_relations: 1,
  },
};

const noop = () => {};

// Every view renders both as a chart and as its table, and each is reached by a
// control the user can click, so both paths have to survive an empty result.
const views = [
  {
    name: "Composition",
    render: (data: Visualization, showTable: boolean) =>
      <Composition data={data} showTable={showTable} onCopy={noop} onSelect={noop}/>,
  },
  {
    name: "RiskMatrix",
    render: (data: Visualization, showTable: boolean) =>
      <RiskMatrix
        data={data} showTable={showTable} staleLens={false}
        onLens={noop} onCopy={noop} onSelect={noop}
      />,
  },
  {
    name: "Freshness",
    render: (data: Visualization, showTable: boolean) =>
      <Freshness data={data} showTable={showTable} onCopy={noop}/>,
  },
  {
    name: "ChangeFlow",
    render: (data: Visualization, showTable: boolean) =>
      <ChangeFlow data={data} showTable={showTable} onCopy={noop}/>,
  },
  {
    name: "Growth",
    render: (data: Visualization, showTable: boolean) =>
      <Growth data={data} showTable={showTable} onCopy={noop}/>,
  },
  {
    name: "Topology",
    render: (data: Visualization, showTable: boolean) =>
      <Topology data={data} showTable={showTable} onCopy={noop} onSelect={noop}/>,
  },
];

describe("visualization views", () => {
  for (const view of views) {
    for (const showTable of [false, true]) {
      const mode = showTable ? "table" : "chart";

      it(`${view.name} renders an empty result as ${mode}`, () => {
        // A throw here is a blank page with no message: React unmounts the tree
        // and the user has nothing to report.
        const markup = renderToStaticMarkup(view.render(empty, showTable));
        expect(markup).not.toContain("NaN");
        expect(markup).not.toContain("undefined");
        expect(markup).not.toContain("Infinity");
      });

      it(`${view.name} renders real data as ${mode}`, () => {
        const markup = renderToStaticMarkup(view.render(populated, showTable));
        expect(markup).not.toContain("NaN");
        expect(markup).not.toContain("undefined");
        expect(markup).not.toContain("Infinity");
        expect(markup.length).toBeGreaterThan(0);
      });
    }
  }
});
