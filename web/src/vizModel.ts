// Pure layout and scale maths for the asset visualization page. Everything here
// is deterministic and free of DOM access so the geometry can be tested, and so
// the same data always draws the same picture.

export type Bucket = {label: string; count: number};
export type Branch = {label: string; count: number; children: Bucket[]};
export type Rect = {x: number; y: number; width: number; height: number};
export type TreemapTile = Rect & {
  label: string;
  count: number;
  branch: string;
  slot: number;
};

/** The categorical palette has a fixed length; a 9th series folds into 기타
 * rather than inventing a hue no colour-vision check can separate. */
export const seriesSlotCount = 8;

/**
 * Keeps the largest members and sums the tail into one labelled remainder, so a
 * long-tailed dimension never needs a ninth colour.
 */
export const foldTail = (
  buckets: Bucket[],
  keep: number,
  remainderLabel = "기타",
): Bucket[] => {
  const ordered = [...buckets].sort((left, right) => right.count - left.count);
  if (ordered.length <= keep) return ordered;
  const head = ordered.slice(0, keep - 1);
  const tail = ordered.slice(keep - 1);
  const remainder = tail.reduce((sum, item) => sum + item.count, 0);
  if (remainder <= 0) return head;
  return [...head, {label: `${remainderLabel} ${tail.length}종`, count: remainder}];
};

/** Stable slot assignment: colour follows the entity, never its rank in the
 * current filter, so removing a series never repaints the survivors. */
export const slotFor = (label: string, order: string[]): number => {
  const index = order.indexOf(label);
  if (index >= 0) return index % seriesSlotCount;
  // Unknown labels get a stable hash so a value keeps its colour between loads.
  let hash = 0;
  for (let position = 0; position < label.length; position += 1) {
    hash = (hash * 31 + label.charCodeAt(position)) % 100000;
  }
  return hash % seriesSlotCount;
};

/**
 * Squarified treemap (Bruls, Huizing, van Wijk). Produces tiles whose aspect
 * ratios stay near 1, which is what makes areas comparable by eye.
 */
export const squarify = (values: number[], area: Rect): Rect[] => {
  const total = values.reduce((sum, value) => sum + value, 0);
  if (total <= 0 || values.length === 0) return [];
  const scale = (area.width * area.height) / total;
  const scaled = values.map(value => value * scale);
  const result: Rect[] = new Array(values.length);
  let remaining = {...area};
  let index = 0;

  const worst = (row: number[], side: number): number => {
    const sum = row.reduce((total_, value) => total_ + value, 0);
    const maximum = Math.max(...row);
    const minimum = Math.min(...row);
    const squared = side * side;
    const sumSquared = sum * sum;
    return Math.max(
      (squared * maximum) / sumSquared,
      sumSquared / (squared * minimum),
    );
  };

  while (index < scaled.length) {
    const horizontal = remaining.width >= remaining.height;
    const side = horizontal ? remaining.height : remaining.width;
    const row: number[] = [scaled[index]];
    let next = index + 1;
    while (next < scaled.length) {
      const candidate = [...row, scaled[next]];
      if (worst(candidate, side) > worst(row, side)) break;
      row.push(scaled[next]);
      next += 1;
    }
    const rowSum = row.reduce((sum, value) => sum + value, 0);
    const thickness = side > 0 ? rowSum / side : 0;
    let offset = 0;
    row.forEach((value, position) => {
      const length = side > 0 ? value / thickness : 0;
      result[index + position] = horizontal
        ? {x: remaining.x, y: remaining.y + offset, width: thickness, height: length}
        : {x: remaining.x + offset, y: remaining.y, width: length, height: thickness};
      offset += length;
    });
    if (horizontal) {
      remaining = {
        x: remaining.x + thickness,
        y: remaining.y,
        width: Math.max(0, remaining.width - thickness),
        height: remaining.height,
      };
    } else {
      remaining = {
        x: remaining.x,
        y: remaining.y + thickness,
        width: remaining.width,
        height: Math.max(0, remaining.height - thickness),
      };
    }
    index += row.length;
  }
  return result;
};

/** Two-level treemap: environments carry the colour, their asset types fill them. */
export const treemapTiles = (
  branches: Branch[],
  area: Rect,
  order: string[],
  gap = 2,
): TreemapTile[] => {
  const folded = foldTail(
    branches.map(branch => ({label: branch.label, count: branch.count})),
    seriesSlotCount,
  );
  const byLabel = new Map(branches.map(branch => [branch.label, branch]));
  const outer = squarify(folded.map(item => item.count), area);
  const tiles: TreemapTile[] = [];
  folded.forEach((item, index) => {
    const frame = inset(outer[index], gap / 2);
    const branch = byLabel.get(item.label);
    const children = branch?.children?.length
      ? foldTail(branch.children, 6)
      : [{label: item.label, count: item.count}];
    const inner = squarify(children.map(child => child.count), frame);
    children.forEach((child, position) => {
      const cell = inset(inner[position], gap / 2);
      if (cell.width <= 0 || cell.height <= 0) return;
      tiles.push({
        ...cell,
        label: child.label,
        count: child.count,
        branch: item.label,
        slot: slotFor(item.label, order),
      });
    });
  });
  return tiles;
};

const inset = (rect: Rect | undefined, amount: number): Rect => {
  if (!rect) return {x: 0, y: 0, width: 0, height: 0};
  return {
    x: rect.x + amount,
    y: rect.y + amount,
    width: Math.max(0, rect.width - amount * 2),
    height: Math.max(0, rect.height - amount * 2),
  };
};

/**
 * Maps a value onto a discrete sequential step. Zero is its own step so "no
 * assets here" never looks like "a few assets here".
 */
export const sequentialStep = (
  value: number,
  maximum: number,
  steps: number,
): number => {
  if (value <= 0) return 0;
  if (maximum <= 0) return 1;
  const ratio = value / maximum;
  return Math.min(steps, Math.max(1, Math.ceil(ratio * steps)));
};

/**
 * Nice round axis maximum, so ticks land on numbers a reader can hold. The
 * fractional factors matter: without 1.25 a series peaking at 116 would be
 * plotted against 200 and use half the height it deserves.
 */
export const niceMaximum = (value: number): number => {
  if (value <= 0) return 1;
  const magnitude = 10 ** Math.floor(Math.log10(value));
  for (const factor of [1, 1.25, 1.5, 2, 2.5, 3, 4, 5, 10]) {
    const candidate = magnitude * factor;
    if (candidate >= value) return candidate;
  }
  return magnitude * 10;
};

/**
 * Splits a plot area between the two arms of a diverging chart in proportion to
 * each arm's extent, so one shared value scale is kept while an empty arm costs
 * no space. Half a chart of blank canvas reads as a broken chart.
 */
export const divergingScale = (
  positives: number[],
  negatives: number[],
  height: number,
): {baseline: number; unit: number; positiveMaximum: number; negativeMaximum: number} => {
  const positiveMaximum = niceMaximum(Math.max(0, ...positives));
  const rawNegative = Math.max(0, ...negatives);
  const negativeMaximum = rawNegative > 0 ? niceMaximum(rawNegative) : 0;
  const span = positiveMaximum + negativeMaximum;
  const unit = span > 0 ? height / span : 0;
  return {
    baseline: positiveMaximum * unit,
    unit,
    positiveMaximum,
    negativeMaximum,
  };
};

export type FlowPoint = {date: string; added: number; removed: number; total: number};

/** Polyline points for the cumulative curve, plus the value scale it used. */
export const growthGeometry = (
  flow: FlowPoint[],
  area: Rect,
): {points: {x: number; y: number; point: FlowPoint}[]; maximum: number} => {
  const maximum = niceMaximum(Math.max(1, ...flow.map(item => item.total)));
  const step = flow.length > 1 ? area.width / (flow.length - 1) : 0;
  return {
    maximum,
    points: flow.map((point, index) => ({
      x: area.x + step * index,
      y: area.y + area.height - (point.total / maximum) * area.height,
      point,
    })),
  };
};

/** Index of the sample nearest a pointer position, for the crosshair. */
export const nearestIndex = (
  xs: number[],
  x: number,
): number => {
  if (!xs.length) return -1;
  let best = 0;
  let distance = Math.abs(xs[0] - x);
  for (let index = 1; index < xs.length; index += 1) {
    const candidate = Math.abs(xs[index] - x);
    if (candidate < distance) {
      distance = candidate;
      best = index;
    }
  }
  return best;
};

export type GraphNode = {
  id: string;
  name: string;
  type: string;
  environment: string;
  criticality: string;
  degree: number;
  stale: boolean;
};
export type GraphEdge = {source: string; target: string; type: string};
export type PlacedNode = GraphNode & {
  x: number;
  y: number;
  radius: number;
  slot: number;
  step: number;
};

/**
 * Pulls an edge's control point partway toward the centre. With the centre
 * itself as the control point every edge crosses the middle and the graph
 * becomes a hairball; bundling by the chord's own midpoint keeps short hops
 * near the rim and lets only long hops cut inward.
 */
export const edgeControlPoint = (
  source: {x: number; y: number},
  target: {x: number; y: number},
  center: {x: number; y: number},
  pull = 0.62,
): {x: number; y: number} => ({
  x: (source.x + target.x) / 2 + (center.x - (source.x + target.x) / 2) * pull,
  y: (source.y + target.y) / 2 + (center.y - (source.y + target.y) / 2) * pull,
});

/**
 * Deterministic radial layout. A force simulation would redraw differently on
 * every load and cost a dependency; ordering nodes by environment then degree
 * puts related assets side by side and always produces the same picture.
 */
export const topologyLayout = (
  nodes: GraphNode[],
  center: {x: number; y: number},
  radius: number,
  typeOrder: string[],
  environmentOrder: string[],
): PlacedNode[] => {
  const ranked = [...nodes].sort((left, right) => {
    const leftEnvironment = environmentOrder.indexOf(left.environment);
    const rightEnvironment = environmentOrder.indexOf(right.environment);
    const leftRank = leftEnvironment < 0 ? environmentOrder.length : leftEnvironment;
    const rightRank = rightEnvironment < 0 ? environmentOrder.length : rightEnvironment;
    if (leftRank !== rightRank) return leftRank - rightRank;
    if (right.degree !== left.degree) return right.degree - left.degree;
    return left.name.localeCompare(right.name);
  });
  const maximumDegree = Math.max(1, ...ranked.map(node => node.degree));
  // Nodes need roughly 26 units of arc each to stay separate, so the ring count
  // follows the node count instead of a fixed split.
  const perRing = Math.max(12, Math.floor((2 * Math.PI * radius) / 26));
  const rings = Math.min(3, Math.max(1, Math.ceil(ranked.length / perRing)));
  const outerFirst: number[] = [];
  for (let ring = 0; ring < rings; ring += 1) {
    outerFirst.push(Math.round((ranked.length * (rings - ring)) / ((rings * (rings + 1)) / 2)));
  }
  return ranked.map((node, index) => {
    let ring = 0;
    let seen = 0;
    while (ring < rings - 1 && index >= seen + outerFirst[ring]) {
      seen += outerFirst[ring];
      ring += 1;
    }
    const ringCount = Math.max(1, outerFirst[ring]);
    const position = index - seen;
    const ringRadius = radius * (1 - ring * (0.34 / Math.max(1, rings - 1)) * (rings > 1 ? 1 : 0));
    const angle = (position / ringCount) * Math.PI * 2 - Math.PI / 2;
    return {
      ...node,
      x: center.x + Math.cos(angle) * ringRadius,
      y: center.y + Math.sin(angle) * ringRadius,
      // Area, not radius, tracks degree: doubling the radius would look like
      // four times the connections.
      radius: 5 + Math.sqrt(node.degree / maximumDegree) * 9,
      slot: slotFor(node.type, typeOrder),
      // A node-link diagram places arbitrary pairs side by side, so it is an
      // all-pairs form and cannot seat eight categorical hues. Connection count
      // is encoded on one sequential ramp instead; type identity lives in the
      // legend filter, the tooltip and the table.
      step: sequentialStep(node.degree, maximumDegree, 5),
    };
  });
};

/** CSV of the current view, for pasting into a report. */
export const toCSV = (headers: string[], rows: (string | number)[][]): string => {
  const escape = (value: string | number) => {
    const text = String(value ?? "");
    return /[",\n]/.test(text) ? `"${text.replaceAll('"', '""')}"` : text;
  };
  return [headers, ...rows]
    .map(row => row.map(escape).join(","))
    .join("\n");
};

export const compactNumber = (value: number): string => {
  if (Math.abs(value) >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
  if (Math.abs(value) >= 10_000) return `${Math.round(value / 1_000)}K`;
  return value.toLocaleString("ko-KR");
};
