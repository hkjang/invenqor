import {describe, expect, it} from "vitest";
import {
  compactNumber,
  divergingScale,
  edgeControlPoint,
  foldTail,
  growthGeometry,
  nearestIndex,
  niceMaximum,
  sequentialStep,
  slotFor,
  squarify,
  toCSV,
  topologyLayout,
  treemapTiles,
} from "./vizModel";

describe("foldTail", () => {
  it("keeps the largest members and sums the rest into one remainder", () => {
    const folded = foldTail(
      [
        {label: "a", count: 10},
        {label: "b", count: 8},
        {label: "c", count: 3},
        {label: "d", count: 2},
        {label: "e", count: 1},
      ],
      3,
    );
    expect(folded.map(item => item.label)).toEqual(["a", "b", "기타 3종"]);
    expect(folded[2].count).toBe(6);
  });

  it("never invents a remainder when everything fits", () => {
    const folded = foldTail([{label: "a", count: 2}], 8);
    expect(folded).toHaveLength(1);
  });
});

describe("slotFor", () => {
  it("keeps a label on its slot no matter what else is on screen", () => {
    const order = ["production", "staging", "test"];
    expect(slotFor("staging", order)).toBe(1);
    // The same label resolves identically when the other series disappear.
    expect(slotFor("staging", order)).toBe(1);
  });

  it("is stable for labels outside the known order", () => {
    expect(slotFor("edge-site", [])).toBe(slotFor("edge-site", []));
    expect(slotFor("edge-site", [])).toBeLessThan(8);
  });
});

describe("squarify", () => {
  const area = {x: 0, y: 0, width: 400, height: 200};

  it("fills the area exactly once", () => {
    const values = [40, 25, 15, 10, 6, 4];
    const rects = squarify(values, area);
    const covered = rects.reduce((sum, rect) => sum + rect.width * rect.height, 0);
    expect(covered).toBeCloseTo(area.width * area.height, 4);
  });

  it("keeps every tile inside the area", () => {
    const rects = squarify([50, 30, 12, 8], area);
    rects.forEach(rect => {
      expect(rect.x).toBeGreaterThanOrEqual(-1e-6);
      expect(rect.y).toBeGreaterThanOrEqual(-1e-6);
      expect(rect.x + rect.width).toBeLessThanOrEqual(area.width + 1e-6);
      expect(rect.y + rect.height).toBeLessThanOrEqual(area.height + 1e-6);
    });
  });

  it("keeps areas proportional to the values", () => {
    const rects = squarify([60, 30, 10], area);
    const total = area.width * area.height;
    expect((rects[0].width * rects[0].height) / total).toBeCloseTo(0.6, 3);
    expect((rects[2].width * rects[2].height) / total).toBeCloseTo(0.1, 3);
  });

  it("returns nothing for an empty or zero-sum input", () => {
    expect(squarify([], area)).toEqual([]);
    expect(squarify([0, 0], area)).toEqual([]);
  });
});

describe("treemapTiles", () => {
  it("splits each environment into its asset types without overlapping", () => {
    const tiles = treemapTiles(
      [
        {
          label: "production",
          count: 30,
          children: [
            {label: "host", count: 20},
            {label: "service", count: 10},
          ],
        },
        {label: "test", count: 10, children: [{label: "host", count: 10}]},
      ],
      {x: 0, y: 0, width: 300, height: 200},
      ["production", "test"],
    );
    expect(tiles).toHaveLength(3);
    expect(tiles.filter(tile => tile.branch === "production")).toHaveLength(2);
    // Colour follows the environment, so both production tiles share a slot.
    const productionSlots = new Set(
      tiles.filter(tile => tile.branch === "production").map(tile => tile.slot),
    );
    expect(productionSlots.size).toBe(1);
    tiles.forEach(tile => {
      expect(tile.width).toBeGreaterThan(0);
      expect(tile.height).toBeGreaterThan(0);
    });
  });

  it("still draws a branch that reports no children", () => {
    const tiles = treemapTiles(
      [{label: "production", count: 5, children: []}],
      {x: 0, y: 0, width: 100, height: 100},
      ["production"],
    );
    expect(tiles).toHaveLength(1);
    expect(tiles[0].count).toBe(5);
  });
});

describe("sequentialStep", () => {
  it("gives zero its own step", () => {
    expect(sequentialStep(0, 40, 5)).toBe(0);
  });

  it("never returns a step below one for a present value", () => {
    expect(sequentialStep(1, 1000, 5)).toBe(1);
  });

  it("puts the maximum on the darkest step", () => {
    expect(sequentialStep(40, 40, 5)).toBe(5);
  });
});

describe("niceMaximum", () => {
  it("rounds up to a readable tick", () => {
    expect(niceMaximum(7)).toBe(10);
    expect(niceMaximum(23)).toBe(25);
    expect(niceMaximum(140)).toBe(150);
    expect(niceMaximum(0)).toBe(1);
  });

  it("stays close to the data so a curve is not squashed", () => {
    // 116 against a 200 axis used half the plot height.
    expect(niceMaximum(116)).toBe(125);
    expect(niceMaximum(116) / 116).toBeLessThan(1.35);
  });
});

describe("divergingScale", () => {
  it("gives an empty arm no space while keeping one shared unit", () => {
    const scale = divergingScale([3, 8, 5], [], 200);
    expect(scale.negativeMaximum).toBe(0);
    expect(scale.baseline).toBeCloseTo(200, 6);
    expect(8 * scale.unit).toBeCloseTo((8 / scale.positiveMaximum) * 200, 6);
  });

  it("splits the area in proportion to both arms", () => {
    const scale = divergingScale([10], [5], 300);
    expect(scale.positiveMaximum).toBe(10);
    expect(scale.negativeMaximum).toBe(5);
    expect(scale.baseline).toBeCloseTo(200, 6);
    // The same unit must apply above and below the baseline.
    expect(10 * scale.unit).toBeCloseTo(200, 6);
    expect(5 * scale.unit).toBeCloseTo(100, 6);
  });
});

describe("edgeControlPoint", () => {
  it("keeps a short hop near the rim and bows a long hop inward", () => {
    const center = {x: 100, y: 100};
    const short = edgeControlPoint({x: 100, y: 20}, {x: 140, y: 30}, center);
    const long = edgeControlPoint({x: 100, y: 20}, {x: 100, y: 180}, center);
    const shortPull = Math.hypot(short.x - center.x, short.y - center.y);
    const longPull = Math.hypot(long.x - center.x, long.y - center.y);
    expect(shortPull).toBeGreaterThan(longPull);
    // A diameter-length edge collapses its control point onto the centre.
    expect(longPull).toBeCloseTo(0, 6);
  });
});

describe("growthGeometry", () => {
  const flow = [
    {date: "2026-07-01", added: 1, removed: 0, total: 10},
    {date: "2026-07-02", added: 2, removed: 0, total: 12},
    {date: "2026-07-03", added: 0, removed: 1, total: 11},
  ];

  it("spans the plot area and puts the largest total highest", () => {
    const area = {x: 10, y: 20, width: 200, height: 100};
    const {points, maximum} = growthGeometry(flow, area);
    expect(points[0].x).toBe(10);
    expect(points[2].x).toBe(210);
    expect(maximum).toBeGreaterThanOrEqual(12);
    expect(points[1].y).toBeLessThan(points[0].y);
  });

  it("survives a single sample", () => {
    const {points} = growthGeometry([flow[0]], {x: 0, y: 0, width: 100, height: 50});
    expect(points).toHaveLength(1);
    expect(Number.isFinite(points[0].x)).toBe(true);
  });
});

describe("nearestIndex", () => {
  it("snaps to the closest sample so the reader aims at a date", () => {
    expect(nearestIndex([0, 10, 20, 30], 13)).toBe(1);
    expect(nearestIndex([0, 10, 20, 30], 16)).toBe(2);
    expect(nearestIndex([], 5)).toBe(-1);
  });
});

describe("topologyLayout", () => {
  const nodes = [
    {id: "1", name: "web-01", type: "host", environment: "production", criticality: "critical", degree: 4, stale: false},
    {id: "2", name: "db-01", type: "host", environment: "production", criticality: "high", degree: 2, stale: false},
    {id: "3", name: "svc", type: "service", environment: "test", criticality: "low", degree: 0, stale: true},
  ];

  it("is deterministic", () => {
    const first = topologyLayout(nodes, {x: 100, y: 100}, 80, ["host", "service"], ["production", "test"]);
    const second = topologyLayout(nodes, {x: 100, y: 100}, 80, ["host", "service"], ["production", "test"]);
    expect(first).toEqual(second);
  });

  it("groups by environment, then by connection count", () => {
    const placed = topologyLayout(nodes, {x: 0, y: 0}, 50, ["host", "service"], ["production", "test"]);
    expect(placed.map(node => node.name)).toEqual(["web-01", "db-01", "svc"]);
  });

  it("scales node area, not radius, with degree", () => {
    const placed = topologyLayout(nodes, {x: 0, y: 0}, 50, ["host", "service"], ["production", "test"]);
    const busiest = placed[0].radius - 5;
    const quiet = placed[1].radius - 5;
    expect(busiest / quiet).toBeCloseTo(Math.sqrt(4 / 2), 3);
  });

  it("keeps every node inside the radius", () => {
    const placed = topologyLayout(nodes, {x: 100, y: 100}, 60, [], []);
    placed.forEach(node => {
      const distance = Math.hypot(node.x - 100, node.y - 100);
      expect(distance).toBeLessThanOrEqual(60.001);
    });
  });
});

describe("toCSV", () => {
  it("quotes separators and escapes quotes", () => {
    const csv = toCSV(["label", "count"], [['a,b', 2], ['say "hi"', 3]]);
    expect(csv.split("\n")[1]).toBe('"a,b",2');
    expect(csv.split("\n")[2]).toBe('"say ""hi""",3');
  });
});

describe("compactNumber", () => {
  it("keeps small numbers exact and compacts large ones", () => {
    expect(compactNumber(842)).toBe("842");
    expect(compactNumber(12_400)).toBe("12K");
    expect(compactNumber(2_400_000)).toBe("2.4M");
  });
});
