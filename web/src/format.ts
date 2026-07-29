// One place for every value the console renders from a timestamp or a count.
//
// There were three copies of a date formatter, and only one of them guarded an
// unparseable value. `Intl.DateTimeFormat.format` throws a RangeError on an
// invalid Date, so a single unexpected timestamp took a whole page down into the
// error boundary rather than showing one odd cell. The server now returns zoned
// RFC 3339 everywhere, but a formatter that cannot survive bad input is a page
// that cannot survive one bad row.

const dateTimeFormat = new Intl.DateTimeFormat("ko-KR", {
  year: "2-digit", month: "2-digit", day: "2-digit",
  hour: "2-digit", minute: "2-digit",
});
const dateFormat = new Intl.DateTimeFormat("ko-KR", {
  year: "numeric", month: "2-digit", day: "2-digit",
});
const secondFormat = new Intl.DateTimeFormat("ko-KR", {
  year: "2-digit", month: "2-digit", day: "2-digit",
  hour: "2-digit", minute: "2-digit", second: "2-digit",
});

const parse = (value?: string | null): Date | null => {
  if (!value) return null;
  const parsed = new Date(value);
  return Number.isNaN(parsed.getTime()) ? null : parsed;
};

/** Date and time, or the raw value when it cannot be read as a date. */
export const formatDate = (value?: string | null): string => {
  if (!value) return "—";
  const parsed = parse(value);
  return parsed ? dateTimeFormat.format(parsed) : value;
};

export const formatDateOnly = (value?: string | null): string => {
  if (!value) return "—";
  const parsed = parse(value);
  return parsed ? dateFormat.format(parsed) : value;
};

/** Second precision, for audit and diagnostic rows where ordering matters. */
export const formatSecond = (value?: string | null): string => {
  if (!value) return "—";
  const parsed = parse(value);
  return parsed ? secondFormat.format(parsed) : value;
};

/**
 * How long ago, in the words an operator uses. "3분 전" answers "is this
 * current?" without the reader subtracting two timestamps.
 */
export const formatRelative = (
  value?: string | null,
  now: number = Date.now(),
): string => {
  const parsed = parse(value);
  if (!parsed) return "—";
  const seconds = Math.round((now - parsed.getTime()) / 1000);
  const future = seconds < 0;
  const magnitude = Math.abs(seconds);
  const [amount, unit] = magnitude < 60
    ? [magnitude, "초"]
    : magnitude < 3600
      ? [Math.floor(magnitude / 60), "분"]
      : magnitude < 86400
        ? [Math.floor(magnitude / 3600), "시간"]
        : [Math.floor(magnitude / 86400), "일"];
  if (magnitude < 10) return "방금";
  return future ? `${amount}${unit} 후` : `${amount}${unit} 전`;
};

export const number = (value?: number | null): string =>
  (value || 0).toLocaleString("ko-KR");

/**
 * A share of a whole, or a dash when there is no whole. Reporting 100%
 * freshness for an empty inventory reads as a healthy system rather than an
 * empty one.
 */
export const percentOf = (part?: number | null, whole?: number | null): string => {
  if (!whole) return "—";
  return `${Math.round((part || 0) / whole * 100)}%`;
};

/** Milliseconds since the value, or null when it cannot be read. */
export const ageMilliseconds = (
  value?: string | null,
  now: number = Date.now(),
): number | null => {
  const parsed = parse(value);
  return parsed ? now - parsed.getTime() : null;
};

export const withinMinutes = (
  value: string | undefined | null,
  minutes: number,
  now: number = Date.now(),
): boolean => {
  const age = ageMilliseconds(value, now);
  return age !== null && age <= minutes * 60_000;
};

/** Triggers a browser download for a same-origin export endpoint. */
export const downloadPath = (path: string) => {
  const anchor = document.createElement("a");
  anchor.href = path;
  anchor.rel = "noopener";
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
};
