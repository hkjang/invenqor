import {readdirSync, readFileSync, statSync} from "node:fs";
import {join} from "node:path";
import {describe, expect, it} from "vitest";
import {diagnosticComponentLabels} from "./operationsPages";

// Every component the Server records must have a Korean label here.
//
// The console falls back to the raw identifier, so a missing label is not a
// crash - it is one row reading "event_spool" among rows reading "Agent 등록",
// which is the kind of thing nobody reports and nobody notices until they are
// already reading the log to find a problem.
//
// Both sides come from the source: the labels from this module, the components
// from the Go that records them.
const goFiles = (directory: string): string[] => {
  const found: string[] = [];
  for (const entry of readdirSync(directory)) {
    const path = join(directory, entry);
    if (statSync(path).isDirectory()) {
      found.push(...goFiles(path));
    } else if (entry.endsWith(".go") && !entry.endsWith("_test.go")) {
      found.push(path);
    }
  }
  return found;
};

describe("diagnostic component labels", () => {
  it("covers every component the Server records", () => {
    const recorded = new Set<string>();
    for (const path of goFiles(join("..", "server", "internal"))) {
      const source = readFileSync(path, "utf8");
      for (const match of source.matchAll(/Component:\s*"([a-z_]+)"/g)) {
        recorded.add(match[1]);
      }
    }

    expect(recorded.size).toBeGreaterThan(0);
    const missing = [...recorded].filter(name => !(name in diagnosticComponentLabels));
    expect(missing, `these components have no Korean label`).toEqual([]);
  });
});
