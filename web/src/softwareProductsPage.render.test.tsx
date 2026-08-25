import {renderToStaticMarkup} from "react-dom/server";
import {describe, expect, it} from "vitest";
import {
  EvidenceGroup,
  SoftwareProductDrawer,
  SoftwareTable,
  TopProducts,
  type SoftwareProduct,
  type SoftwareSummary,
} from "./softwareProductsPage";

const product: SoftwareProduct = {
  id: "33333333-3333-4333-8333-333333333333",
  asset_key: "software:nginx",
  status: "active",
  product_key: "nginx",
  product_name: "NGINX",
  role: "web_server",
  vendor: "F5",
  version: "1.26.2",
  versions: ["1.26.2"],
  install_state: "installed",
  runtime_state: "running",
  service_names: ["nginx"],
  process_names: ["nginx"],
  package_names: ["nginx-core"],
  executable_paths: ["/usr/sbin/nginx"],
  evidence: [{kind: "process", name: "nginx", source_asset_id: "pid-1"}],
  detection_method: "builtin_catalog",
  catalog_version: "2026.08",
  evidence_count: 1,
  process_count: 1,
  confidence: 0.95,
  host: {id: "host-1", name: "web-01"},
  last_seen_at: "2026-08-26T00:00:00Z",
};

// What a product looks like before classification has resolved anything: no
// vendor, no version, no evidence, and no host relation yet. The console has
// its own wording for each of those and none of it should read "undefined".
const unresolved: SoftwareProduct = {
  ...product,
  id: "44444444-4444-4444-8444-444444444444",
  product_name: "Unknown service",
  vendor: "unknown",
  version: "",
  versions: [],
  install_state: "unknown",
  runtime_state: "unknown",
  service_names: [],
  process_names: [],
  package_names: [],
  executable_paths: [],
  evidence: [],
  evidence_count: 0,
  process_count: 0,
  confidence: 0,
  host: {id: "", name: ""},
};

const summary: SoftwareSummary = {
  products: 1, instances: 1, hosts: 1, running: 1, stopped: 0,
  runtime_unknown: 0, installed: 1, observed_only: 0, high_confidence: 1,
  needs_review: 0, with_process_evidence: 1, mapped_processes: 1,
  top_products: [{
    product_key: "nginx", product_name: "NGINX", role: "web_server",
    vendor: "F5", instances: 1, hosts: 1, running: 1, versions: ["1.26.2"],
  }],
};

const emptySummary: SoftwareSummary = {
  products: 0, instances: 0, hosts: 0, running: 0, stopped: 0,
  runtime_unknown: 0, installed: 0, observed_only: 0, high_confidence: 0,
  needs_review: 0, with_process_evidence: 0, mapped_processes: 0,
  top_products: [],
};

const noop = () => {};

// A throw inside render is a blank page: React unmounts the tree and the user
// has no message to report. The length check is what stops the rest from being
// vacuous - they are all absences, and rendering nothing satisfies every one.
const clean = (markup: string) => {
  expect(markup.length).toBeGreaterThan(50);
  expect(markup).not.toContain("undefined");
  expect(markup).not.toContain("NaN");
  expect(markup).not.toContain("Infinity");
};

describe("SoftwareTable", () => {
  it("renders with no products", () => {
    clean(renderToStaticMarkup(<SoftwareTable items={[]} onSelect={noop}/>));
  });

  it("renders a product nothing has been resolved for", () => {
    const markup = renderToStaticMarkup(
      <SoftwareTable items={[unresolved]} onSelect={noop}/>,
    );
    clean(markup);
    // The console has wording for each unresolved field; it must be used.
    expect(markup).toContain("제조사 미확인");
    expect(markup).toContain("호스트 관계 확인 전");
  });

  it("renders a fully resolved product", () => {
    const markup = renderToStaticMarkup(
      <SoftwareTable items={[product]} onSelect={noop}/>,
    );
    clean(markup);
    expect(markup).toContain("NGINX");
  });
});

describe("SoftwareProductDrawer", () => {
  it("renders a product with no evidence and no host", () => {
    clean(renderToStaticMarkup(
      <SoftwareProductDrawer product={unresolved} onClose={noop}/>,
    ));
  });

  it("renders a product with evidence", () => {
    const markup = renderToStaticMarkup(
      <SoftwareProductDrawer product={product} onClose={noop}/>,
    );
    clean(markup);
    expect(markup).toContain("nginx");
  });
});

describe("TopProducts", () => {
  it("renders nothing collected yet", () => {
    clean(renderToStaticMarkup(<TopProducts summary={emptySummary}/>));
  });

  it("renders real products, full and compact", () => {
    clean(renderToStaticMarkup(<TopProducts summary={summary}/>));
    clean(renderToStaticMarkup(<TopProducts summary={summary} compact/>));
  });
});

describe("EvidenceGroup", () => {
  it("renders nothing at all when there is no evidence", () => {
    // A section with a zero count would be noise in a drawer of sections.
    expect(renderToStaticMarkup(<EvidenceGroup title="프로세스" values={[]}/>)).toBe("");
  });

  it("renders the values it was given", () => {
    const markup = renderToStaticMarkup(
      <EvidenceGroup title="프로세스" values={["nginx", "nginx: worker"]}/>,
    );
    clean(markup);
    expect(markup).toContain("nginx: worker");
  });
});
