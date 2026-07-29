import {describe, expect, it} from "vitest";
import {
  ageMilliseconds,
  formatDate,
  formatRelative,
  formatSecond,
  percentOf,
  withinMinutes,
} from "./format";

describe("formatDate", () => {
  it("never throws on a value it cannot read", () => {
    // Intl.DateTimeFormat.format throws a RangeError on an invalid Date, so an
    // unguarded formatter took the whole page down over one bad cell.
    for (const value of ["", "  ", "sometime yesterday", "0000-00-00", null, undefined]) {
      expect(() => formatDate(value as string)).not.toThrow();
    }
    expect(formatDate("sometime yesterday")).toBe("sometime yesterday");
    expect(formatDate("")).toBe("—");
    expect(formatDate(null)).toBe("—");
  });

  it("reads a zoned timestamp as the instant it names", () => {
    // The same instant written two ways must render identically; a naive value
    // used to be read as local time, moving every row by the viewer's offset.
    const zoned = formatDate("2026-07-29T19:17:12Z");
    const offset = formatDate("2026-07-30T04:17:12+09:00");
    expect(zoned).toBe(offset);
  });

  it("keeps seconds when ordering matters", () => {
    expect(formatSecond("2026-07-29T19:17:12Z")).toContain("17");
    expect(formatSecond(null)).toBe("—");
  });
});

describe("formatRelative", () => {
  const now = Date.parse("2026-07-29T12:00:00Z");
  it("answers whether a value is current", () => {
    expect(formatRelative("2026-07-29T11:57:00Z", now)).toBe("3분 전");
    expect(formatRelative("2026-07-29T09:00:00Z", now)).toBe("3시간 전");
    expect(formatRelative("2026-07-27T12:00:00Z", now)).toBe("2일 전");
    expect(formatRelative("2026-07-29T11:59:55Z", now)).toBe("방금");
    expect(formatRelative(undefined, now)).toBe("—");
  });

  it("does not claim a future timestamp is in the past", () => {
    // Clock skew between a server and an agent produces these, and "5분 전" for
    // something that has not happened is how a skew goes unnoticed.
    expect(formatRelative("2026-07-29T12:05:00Z", now)).toBe("5분 후");
  });
});

describe("percentOf", () => {
  it("refuses to report a share of nothing", () => {
    // 100% freshness for an empty inventory reads as a healthy system.
    expect(percentOf(0, 0)).toBe("—");
    expect(percentOf(0, undefined)).toBe("—");
    expect(percentOf(3, 4)).toBe("75%");
  });
});

describe("age helpers", () => {
  const now = Date.parse("2026-07-29T12:00:00Z");
  it("measures age only for readable values", () => {
    expect(ageMilliseconds("2026-07-29T11:59:00Z", now)).toBe(60_000);
    expect(ageMilliseconds("not a date", now)).toBeNull();
    expect(withinMinutes("2026-07-29T11:59:00Z", 30, now)).toBe(true);
    expect(withinMinutes("2026-07-29T10:00:00Z", 30, now)).toBe(false);
    expect(withinMinutes(undefined, 30, now)).toBe(false);
  });
});
