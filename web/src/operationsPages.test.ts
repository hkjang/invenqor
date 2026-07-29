import {describe, expect, it} from "vitest";
import {breakdownRows} from "./operationsPages";

describe("breakdownRows", () => {
  const buckets = (counts: number[]) =>
    counts.map((count, index) => ({label: `type-${index}`, count}));

  it("shows every category when they all fit", () => {
    const items = buckets([5, 4, 3]);
    expect(breakdownRows(items)).toEqual(items);
  });

  it("folds the tail into one row instead of dropping it", () => {
    // The panel used to render the top seven and drop the rest, so the visible
    // percentages did not add up to 100 and nothing said why.
    const items = buckets([10, 9, 8, 7, 6, 5, 4, 3, 2, 1]);
    const rows = breakdownRows(items);
    expect(rows).toHaveLength(8);
    expect(rows[7]).toEqual({label: "기타 3종", count: 6});
    const shown = rows.reduce((sum, row) => sum + row.count, 0);
    const all = items.reduce((sum, row) => sum + row.count, 0);
    expect(shown).toBe(all);
  });

  it("names how many categories the tail stands for", () => {
    const rows = breakdownRows(buckets([1, 1, 1, 1]), 3);
    expect(rows.map(row => row.label)).toEqual(["type-0", "type-1", "기타 2종"]);
  });
});
