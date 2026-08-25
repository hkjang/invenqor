import React from "react";
import {
  Boxes,
  Clock,
  Copy,
  Network,
  RefreshCw,
  Table as TableIcon,
  TrendingUp,
  Waves,
} from "lucide-react";
import {api} from "./api";
import {
  compactNumber,
  foldTail,
  growthGeometry,
  nearestIndex,
  divergingScale,
  edgeControlPoint,
  niceMaximum,
  sequentialStep,
  slotFor,
  toCSV,
  topologyLayout,
  treemapTiles,
  type Branch,
  type Bucket,
  type FlowPoint,
  type GraphEdge,
  type GraphNode,
} from "./vizModel";

type MatrixCell = {row: string; column: string; count: number; stale: number};
export type Visualization = {
  generated_at: string;
  window_days: number;
  stale_hours: number;
  totals: {assets: number; stale: number; fresh: number; unowned: number};
  dimensions: Record<string, Bucket[]>;
  matrix: {rows: string[]; columns: string[]; cells: MatrixCell[]; maximum: number};
  freshness: {label: string; max_hours: number; count: number}[];
  hierarchy: Branch[];
  flow: FlowPoint[];
  graph: {
    nodes: GraphNode[];
    edges: GraphEdge[];
    truncated: boolean;
    total_relations: number;
  };
};

type ViewID = "composition" | "matrix" | "freshness" | "flow" | "growth" | "topology";
type Filter = {
  days: number;
  environment: string;
  criticality: string;
  type: string;
  staleHours: number;
};

const views: {id: ViewID; label: string; icon: React.ElementType; job: string}[] = [
  {id: "composition", label: "구성", icon: Boxes, job: "무엇이 얼마나 있는가"},
  {id: "matrix", label: "위험 매트릭스", icon: Waves, job: "중요도와 환경의 교차"},
  {id: "freshness", label: "신선도", icon: Clock, job: "얼마나 최근에 확인했는가"},
  {id: "flow", label: "증감", icon: TrendingUp, job: "매일 늘고 줄어든 양"},
  {id: "growth", label: "성장 추이", icon: TrendingUp, job: "누적 자산 규모"},
  {id: "topology", label: "관계", icon: Network, job: "무엇이 무엇에 연결되는가"},
];

const initialFilter: Filter = {
  days: 30,
  environment: "",
  criticality: "",
  type: "",
  staleHours: 24,
};

// Fixed reference orders. Colour follows the entity through these lists, so a
// filter that removes a series never repaints the survivors.
const environmentOrder = [
  "production", "staging", "qa", "test", "development", "dr", "other", "unknown",
];
const typeOrder = [
  "host", "service", "software", "network", "storage", "database", "container",
  "unknown",
];
const criticalityLabels: Record<string, string> = {
  critical: "치명", high: "높음", medium: "보통", normal: "일반",
  low: "낮음", unknown: "미지정",
};

export function VisualizationPage() {
  const [filter, setFilter] = React.useState<Filter>(initialFilter);
  const [data, setData] = React.useState<Visualization|null>(null);
  const [view, setView] = React.useState<ViewID>("composition");
  const [texture, setTexture] = React.useState(false);
  const [showTable, setShowTable] = React.useState(false);
  const [staleLens, setStaleLens] = React.useState(false);
  const [loading, setLoading] = React.useState(false);
  const [copied, setCopied] = React.useState("");
  const [error, setError] = React.useState("");

  const load = React.useCallback(async () => {
    const parameters = new URLSearchParams({
      days: String(filter.days),
      stale_hours: String(filter.staleHours),
    });
    if (filter.environment) parameters.set("environment", filter.environment);
    if (filter.criticality) parameters.set("criticality", filter.criticality);
    if (filter.type) parameters.set("type", filter.type);
    setLoading(true);
    try {
      setData(await api<Visualization>(`/api/v1/assets/visualization?${parameters}`));
      setError("");
    } catch (reason) {
      setError((reason as Error).message);
    } finally {
      setLoading(false);
    }
  }, [filter]);
  React.useEffect(() => { load(); }, [load]);

  const active = views.find(item => item.id === view) || views[0];
  const chips: {label: string; clear: () => void}[] = [];
  if (filter.environment) {
    chips.push({
      label: `환경 ${filter.environment}`,
      clear: () => setFilter(current => ({...current, environment: ""})),
    });
  }
  if (filter.criticality) {
    chips.push({
      label: `중요도 ${criticalityLabels[filter.criticality] || filter.criticality}`,
      clear: () => setFilter(current => ({...current, criticality: ""})),
    });
  }
  if (filter.type) {
    chips.push({
      label: `유형 ${filter.type}`,
      clear: () => setFilter(current => ({...current, type: ""})),
    });
  }
  const copyCSV = (headers: string[], rows: (string|number)[][], name: string) => {
    const csv = toCSV(headers, rows);
    navigator.clipboard?.writeText(csv).then(
      () => { setCopied(name); window.setTimeout(() => setCopied(""), 2500); },
      () => setError("클립보드에 복사할 수 없습니다."),
    );
  };

  return <section
    className="viz-root"
    data-viz-texture={texture ? "on" : "off"}
  >
    <div className="page-title with-action">
      <div>
        <p className="eyebrow dark">ASSET VISUALIZATION</p>
        <h1>자산 시각화</h1>
        <p>{active.job}. 모든 화면은 아래 필터 한 벌을 공유하므로 수치가 서로 어긋나지 않습니다.</p>
      </div>
      <div>
        <button className="secondary" onClick={load}><RefreshCw size={15}/>새로고침</button>
      </div>
    </div>

    {error && <div className="error action-message">{error}</div>}

    <div className="filter-bar viz-filters">
      <div className="viz-range" role="group" aria-label="기간">
        {[7, 30, 90, 180].map(days => <button
          key={days}
          className={filter.days === days ? "selected" : ""}
          onClick={() => setFilter(current => ({...current, days}))}
        >{days}일</button>)}
      </div>
      <select
        aria-label="환경"
        value={filter.environment}
        onChange={event => setFilter(current => ({...current, environment: event.target.value}))}
      >
        <option value="">모든 환경</option>
        {(data?.dimensions.environment || []).map(item =>
          <option key={item.label} value={item.label}>{item.label}</option>)}
      </select>
      <select
        aria-label="중요도"
        value={filter.criticality}
        onChange={event => setFilter(current => ({...current, criticality: event.target.value}))}
      >
        <option value="">모든 중요도</option>
        {(data?.dimensions.criticality || []).map(item =>
          <option key={item.label} value={item.label}>
            {criticalityLabels[item.label] || item.label}
          </option>)}
      </select>
      <select
        aria-label="자산 유형"
        value={filter.type}
        onChange={event => setFilter(current => ({...current, type: event.target.value}))}
      >
        <option value="">모든 유형</option>
        {(data?.dimensions.type || []).map(item =>
          <option key={item.label} value={item.label}>{item.label}</option>)}
      </select>
      <select
        aria-label="정지 판정 기준"
        value={filter.staleHours}
        onChange={event => setFilter(current => ({
          ...current, staleHours: Number(event.target.value),
        }))}
      >
        <option value="24">정지 기준 24시간</option>
        <option value="168">정지 기준 7일</option>
        <option value="720">정지 기준 30일</option>
      </select>
      <label className="auto-refresh" title="색 대신 질감으로 구분합니다. 색각 이상·인쇄·고대비 환경에서 사용하십시오.">
        <input type="checkbox" checked={texture}
          onChange={event => setTexture(event.target.checked)}/>질감 구분
      </label>
      {!!chips.length && <button className="secondary" onClick={() => setFilter(current => ({
        ...initialFilter, days: current.days, staleHours: current.staleHours,
      }))}>필터 초기화</button>}
    </div>

    {!!chips.length && <div className="viz-chips">
      {chips.map(chip => <button key={chip.label} onClick={chip.clear}>
        {chip.label}<span aria-hidden="true">×</span>
        <em className="visually-hidden">해제</em>
      </button>)}
    </div>}

    <div className="viz-kpis">
      <div className="viz-hero">
        <span>관리 자산</span>
        <strong>{(data?.totals.assets ?? 0).toLocaleString("ko-KR")}</strong>
        <small>{filter.days}일 기준 · 필터 적용 결과</small>
      </div>
      <KPI
        label="정지 자산"
        value={data?.totals.stale ?? 0}
        note={`${filter.staleHours}시간 이상 미확인`}
        tone={(data?.totals.stale ?? 0) > 0 ? "warn" : "ok"}
      />
      <KPI
        label="담당 미지정"
        value={data?.totals.unowned ?? 0}
        note="부서·담당자 없음"
        tone={(data?.totals.unowned ?? 0) > 0 ? "warn" : "ok"}
      />
      <KPI
        label="관계"
        value={data?.graph.total_relations ?? 0}
        note="유효 자산 관계"
        tone="ok"
      />
    </div>

    <div className="viz-tabs" role="tablist" aria-label="시각화 보기">
      {views.map(item => <button
        key={item.id}
        role="tab"
        aria-selected={view === item.id}
        className={view === item.id ? "selected" : ""}
        onClick={() => { setView(item.id); setShowTable(false); }}
      ><item.icon size={15}/>{item.label}</button>)}
    </div>

    <article className={`panel viz-panel${loading ? " loading" : ""}`}>
      <div className="panel-head">
        <h3>{active.label}</h3>
        <span className="viz-panel-actions">
          {copied === active.id && <em>복사했습니다</em>}
          <button onClick={() => setShowTable(value => !value)}>
            <TableIcon size={14}/>{showTable ? "그래프" : "표"}
          </button>
        </span>
      </div>
      {!data ? <div className="admin-empty">{error ? "불러오지 못했습니다." : "집계를 불러오는 중입니다."}</div>
        : data.totals.assets === 0 ? <div className="admin-empty">
            조건에 맞는 자산이 없습니다. 필터를 넓히거나 Agent 등록 상태를 확인하십시오.
          </div>
        : <>
          {view === "composition" && <Composition
            data={data} showTable={showTable}
            onCopy={copyCSV}
            onSelect={(environment, type) => setFilter(current => ({
              ...current, environment, type,
            }))}
          />}
          {view === "matrix" && <RiskMatrix
            data={data} showTable={showTable} staleLens={staleLens}
            onLens={setStaleLens} onCopy={copyCSV}
            onSelect={(criticality, environment) => setFilter(current => ({
              ...current, criticality, environment,
            }))}
          />}
          {view === "freshness" && <Freshness data={data} showTable={showTable} onCopy={copyCSV}/>}
          {view === "flow" && <ChangeFlow data={data} showTable={showTable} onCopy={copyCSV}/>}
          {view === "growth" && <Growth data={data} showTable={showTable} onCopy={copyCSV}/>}
          {view === "topology" && <Topology
            data={data} showTable={showTable} onCopy={copyCSV}
            onSelect={type => setFilter(current => ({...current, type}))}
          />}
        </>}
    </article>
    <TexturePatterns/>
  </section>;
}

function KPI({label, value, note, tone}: {
  label: string; value: number; note: string; tone: "ok"|"warn";
}) {
  return <div className={`viz-kpi ${tone}`}>
    <span>{label}</span>
    <strong>{compactNumber(value)}</strong>
    <small>{note}</small>
  </div>;
}

type CopyHandler = (headers: string[], rows: (string|number)[][], name: string) => void;

/** 구성: environment → type 두 단계 treemap. 면적이 곧 자산 수입니다. */
// The six views are exported so a test can render each one directly. Nothing
// else imports them: VisualizationPage fetches on mount, so rendering the page
// itself only ever exercises its loading state.
export function Composition({data, showTable, onCopy, onSelect}: {
  data: Visualization;
  showTable: boolean;
  onCopy: CopyHandler;
  onSelect: (environment: string, type: string) => void;
}) {
  const width = 900;
  const height = 380;
  const tiles = React.useMemo(
    () => treemapTiles(data.hierarchy, {x: 0, y: 0, width, height}, environmentOrder),
    [data.hierarchy],
  );
  const [hover, setHover] = React.useState<{x: number; y: number; text: string[]}|null>(null);
  const branches = foldTail(
    data.hierarchy.map(branch => ({label: branch.label, count: branch.count})),
    8,
  );
  const rows = data.hierarchy.flatMap(branch =>
    branch.children.map(child => [branch.label, child.label, child.count]));

  if (showTable) {
    return <ViewTable
      headers={["환경", "유형", "자산 수"]} rows={rows}
      onCopy={() => onCopy(["환경", "유형", "자산 수"], rows, "composition")}
    />;
  }
  return <div className="viz-body">
    <Legend
      items={branches.map(branch => ({
        label: branch.label,
        slot: slotFor(branch.label, environmentOrder),
        note: `${branch.count.toLocaleString("ko-KR")}건`,
      }))}
      shape="rect"
    />
    <div className="viz-canvas">
      <svg
        viewBox={`0 0 ${width} ${height}`}
        role="img"
        aria-label={`환경별 자산 구성 treemap, 총 ${data.totals.assets}건`}
        onPointerLeave={() => setHover(null)}
      >
        {tiles.map(tile => {
          const label = `${tile.branch} · ${tile.label}`;
          // Measure before placing: a label only goes inside the tile when the
          // rendered text plus padding actually fits, never clipped.
          const wide = tile.width > tile.label.length * 7.4 + 22 && tile.height > 34;
          return <g key={`${tile.branch}-${tile.label}`}>
            <rect
              x={tile.x} y={tile.y} width={tile.width} height={tile.height}
              rx={3}
              className={`viz-tile slot-${tile.slot + 1}`}
              tabIndex={0}
              onPointerMove={event => setHover({
                x: event.nativeEvent.offsetX,
                y: event.nativeEvent.offsetY,
                text: [`${tile.count.toLocaleString("ko-KR")}건`, label],
              })}
              onFocus={() => setHover({
                x: tile.x + tile.width / 2, y: tile.y,
                text: [`${tile.count.toLocaleString("ko-KR")}건`, label],
              })}
              onBlur={() => setHover(null)}
              onClick={() => onSelect(tile.branch, tile.label)}
            >
              <title>{`${label} ${tile.count}건`}</title>
            </rect>
            {wide && <text
              className={`viz-tile-label on-slot-${tile.slot + 1}`}
              x={tile.x + 10} y={tile.y + 20}
              pointerEvents="none"
            >{tile.label}</text>}
            {wide && <text
              className={`viz-tile-value on-slot-${tile.slot + 1}`}
              x={tile.x + 10} y={tile.y + 38}
              pointerEvents="none"
            >{tile.count.toLocaleString("ko-KR")}</text>}
          </g>;
        })}
      </svg>
      <Tooltip hover={hover}/>
    </div>
    <p className="viz-note">타일을 선택하면 해당 환경·유형으로 모든 화면이 좁혀집니다.</p>
  </div>;
}

/** 위험 매트릭스: 중요도 × 환경. 순차 단계 하나로 크기를, 두 번째 렌즈로 정지 비율을 읽습니다. */
export function RiskMatrix({data, showTable, staleLens, onLens, onCopy, onSelect}: {
  data: Visualization;
  showTable: boolean;
  staleLens: boolean;
  onLens: (value: boolean) => void;
  onCopy: CopyHandler;
  onSelect: (criticality: string, environment: string) => void;
}) {
  const [hover, setHover] = React.useState<{x: number; y: number; text: string[]}|null>(null);
  const cellOf = (row: string, column: string) =>
    data.matrix.cells.find(cell => cell.row === row && cell.column === column);
  const staleMaximum = Math.max(1, ...data.matrix.cells.map(cell => cell.stale));
  const cell = 74;
  const gutterX = 118;
  const gutterY = 34;
  const width = gutterX + data.matrix.columns.length * cell;
  const height = gutterY + data.matrix.rows.length * cell;
  const rows = data.matrix.cells.map(item =>
    [item.row, item.column, item.count, item.stale]);

  if (showTable) {
    return <ViewTable
      headers={["중요도", "환경", "자산 수", "정지"]} rows={rows}
      onCopy={() => onCopy(["중요도", "환경", "자산 수", "정지"], rows, "matrix")}
    />;
  }
  return <div className="viz-body">
    <div className="viz-lens" role="group" aria-label="매트릭스 렌즈">
      <button className={staleLens ? "" : "selected"} onClick={() => onLens(false)}>자산 수</button>
      <button className={staleLens ? "selected" : ""} onClick={() => onLens(true)}>정지 자산</button>
      <SequentialKey
        maximum={staleLens ? staleMaximum : data.matrix.maximum}
        ramp={staleLens ? "warn" : "base"}
      />
    </div>
    <div className="viz-canvas">
      <svg
        viewBox={`0 0 ${width} ${height}`}
        role="img"
        aria-label="중요도와 환경 교차 자산 분포"
        onPointerLeave={() => setHover(null)}
      >
        {data.matrix.columns.map((column, index) => <text
          key={column} className="viz-axis-label"
          x={gutterX + index * cell + cell / 2} y={22} textAnchor="middle"
        >{column}</text>)}
        {data.matrix.rows.map((row, rowIndex) => <g key={row}>
          <text
            className="viz-axis-label" x={gutterX - 12}
            y={gutterY + rowIndex * cell + cell / 2 + 4} textAnchor="end"
          >{criticalityLabels[row] || row}</text>
          {data.matrix.columns.map((column, columnIndex) => {
            const found = cellOf(row, column);
            const value = staleLens ? (found?.stale ?? 0) : (found?.count ?? 0);
            const step = sequentialStep(
              value, staleLens ? staleMaximum : data.matrix.maximum, 5,
            );
            return <g key={column}>
              <rect
                x={gutterX + columnIndex * cell + 1}
                y={gutterY + rowIndex * cell + 1}
                width={cell - 2} height={cell - 2} rx={3}
                className={`viz-cell ${staleLens ? "warn" : "base"}-${step}`}
                tabIndex={0}
                onPointerMove={event => setHover({
                  x: event.nativeEvent.offsetX,
                  y: event.nativeEvent.offsetY,
                  text: [
                    `${(found?.count ?? 0).toLocaleString("ko-KR")}건`,
                    `${criticalityLabels[row] || row} · ${column}`,
                    `정지 ${(found?.stale ?? 0).toLocaleString("ko-KR")}건`,
                  ],
                })}
                onFocus={() => setHover({
                  x: gutterX + columnIndex * cell + cell / 2,
                  y: gutterY + rowIndex * cell,
                  text: [
                    `${(found?.count ?? 0).toLocaleString("ko-KR")}건`,
                    `${criticalityLabels[row] || row} · ${column}`,
                  ],
                })}
                onBlur={() => setHover(null)}
                onClick={() => onSelect(row, column)}
              ><title>{`${criticalityLabels[row] || row} ${column} ${value}건`}</title></rect>
              {value > 0 && <text
                className={`viz-cell-value ${staleLens ? "warn" : "base"}-on-${step}`}
                x={gutterX + columnIndex * cell + cell / 2}
                y={gutterY + rowIndex * cell + cell / 2 + 5}
                textAnchor="middle" pointerEvents="none"
              >{compactNumber(value)}</text>}
              {!staleLens && (found?.stale ?? 0) > 0 && <circle
                className="viz-cell-flag"
                cx={gutterX + columnIndex * cell + cell - 13}
                cy={gutterY + rowIndex * cell + 13}
                r={4} pointerEvents="none"
              ><title>정지 자산 포함</title></circle>}
            </g>;
          })}
        </g>)}
      </svg>
      <Tooltip hover={hover}/>
    </div>
    <p className="viz-note">
      점이 있는 칸에는 정지 자산이 포함되어 있습니다. 칸을 선택하면 해당 조합만 남깁니다.
    </p>
  </div>;
}

/** 신선도: 나이 구간별 자산 수. 구간은 순서가 있으므로 한 색의 단계로 칠합니다. */
export function Freshness({data, showTable, onCopy}: {
  data: Visualization; showTable: boolean; onCopy: CopyHandler;
}) {
  const [hover, setHover] = React.useState<{x: number; y: number; text: string[]}|null>(null);
  const rows = data.freshness.map(bucket => [bucket.label, bucket.count]);
  if (showTable) {
    return <ViewTable
      headers={["구간", "자산 수"]} rows={rows}
      onCopy={() => onCopy(["구간", "자산 수"], rows, "freshness")}
    />;
  }
  const width = 900;
  const barHeight = 24;
  const gap = 22;
  const left = 132;
  const height = data.freshness.length * (barHeight + gap);
  const maximum = niceMaximum(Math.max(1, ...data.freshness.map(item => item.count)));
  return <div className="viz-body">
    <div className="viz-canvas">
      <svg
        viewBox={`0 0 ${width} ${height}`} role="img"
        aria-label="마지막 확인 시점 구간별 자산 수"
        onPointerLeave={() => setHover(null)}
      >
        {[0, 0.5, 1].map(ratio => <line
          key={ratio} className="viz-grid"
          x1={left + (width - left - 60) * ratio} y1={0}
          x2={left + (width - left - 60) * ratio} y2={height - gap / 2}
        />)}
        {data.freshness.map((bucket, index) => {
          const length = (bucket.count / maximum) * (width - left - 60);
          const y = index * (barHeight + gap);
          return <g key={bucket.label}>
            <text className="viz-axis-label" x={left - 12} y={y + barHeight - 6} textAnchor="end">
              {bucket.label}
            </text>
            <rect
              x={left} y={y} width={Math.max(2, length)} height={barHeight}
              className={`viz-bar ordinal-${index + 1}`}
              rx={4}
              tabIndex={0}
              onPointerMove={event => setHover({
                x: event.nativeEvent.offsetX, y: event.nativeEvent.offsetY,
                text: [`${bucket.count.toLocaleString("ko-KR")}건`, bucket.label],
              })}
              onFocus={() => setHover({
                x: left + length, y,
                text: [`${bucket.count.toLocaleString("ko-KR")}건`, bucket.label],
              })}
              onBlur={() => setHover(null)}
            ><title>{`${bucket.label} ${bucket.count}건`}</title></rect>
            <text
              className="viz-value" x={left + Math.max(2, length) + 10}
              y={y + barHeight - 6}
            >{bucket.count.toLocaleString("ko-KR")}</text>
          </g>;
        })}
      </svg>
      <Tooltip hover={hover}/>
    </div>
    <p className="viz-note">
      오래된 구간일수록 진하게 칠합니다. 90일 초과 구간은 수집이 끊긴 장비이거나
      폐기 처리가 누락된 자산입니다.
    </p>
  </div>;
}

/** 증감: 기준선 위 신규, 아래 폐기. 방향이 부호이므로 발산 색을 씁니다. */
export function ChangeFlow({data, showTable, onCopy}: {
  data: Visualization; showTable: boolean; onCopy: CopyHandler;
}) {
  const [hover, setHover] = React.useState<{x: number; y: number; text: string[]}|null>(null);
  const rows = data.flow.map(day => [day.date, day.added, day.removed]);
  if (showTable) {
    return <ViewTable
      headers={["일자", "신규", "폐기"]} rows={rows}
      onCopy={() => onCopy(["일자", "신규", "폐기"], rows, "flow")}
    />;
  }
  const width = 900;
  const height = 260;
  const top = 18;
  const plot = height - top - 26;
  const slot = width / Math.max(1, data.flow.length);
  const barWidth = Math.max(2, Math.min(24, slot - 2));
  // One shared unit for both arms, but the area is split in proportion to each
  // arm's extent: a window with no retirements must not waste half the canvas.
  const scale = divergingScale(
    data.flow.map(day => day.added),
    data.flow.map(day => day.removed),
    plot,
  );
  const middle = top + scale.baseline;
  const peak = data.flow.reduce(
    (best, day) => (day.added > best.added ? day : best),
    data.flow[0],
  );
  return <div className="viz-body">
    <Legend items={[
      {label: "신규 등록 ↑", slot: -1, className: "positive"},
      {label: "폐기·삭제 ↓", slot: -1, className: "negative"},
    ]} shape="rect"/>
    <div className="viz-canvas">
      <svg
        viewBox={`0 0 ${width} ${height}`} role="img"
        aria-label="일자별 자산 신규 등록과 폐기"
        onPointerLeave={() => setHover(null)}
      >
        <line className="viz-grid" x1={44} y1={top} x2={width} y2={top}/>
        <text className="viz-axis-label" x={40} y={top + 4} textAnchor="end">
          {compactNumber(scale.positiveMaximum)}
        </text>
        {scale.negativeMaximum > 0 && <>
          <line
            className="viz-grid" x1={44} y1={top + plot} x2={width} y2={top + plot}
          />
          <text className="viz-axis-label" x={40} y={top + plot + 4} textAnchor="end">
            {compactNumber(scale.negativeMaximum)}
          </text>
        </>}
        <line className="viz-axis" x1={44} y1={middle} x2={width} y2={middle}/>
        {data.flow.map((day, index) => {
          const x = index * slot + (slot - barWidth) / 2;
          const up = day.added * scale.unit;
          const down = day.removed * scale.unit;
          const enter = (text: string[]) => (event: React.PointerEvent) => setHover({
            x: event.nativeEvent.offsetX, y: event.nativeEvent.offsetY, text,
          });
          const label = [
            `신규 ${day.added} · 폐기 ${day.removed}`,
            day.date,
          ];
          return <g key={day.date}>
            <rect
              x={x - 3} y={top} width={barWidth + 6} height={plot}
              fill="transparent" tabIndex={0}
              onPointerMove={enter(label)}
              onFocus={() => setHover({x: x + barWidth / 2, y: middle - up, text: label})}
              onBlur={() => setHover(null)}
            ><title>{`${day.date} 신규 ${day.added} 폐기 ${day.removed}`}</title></rect>
            {day.added > 0 && <rect
              className="viz-bar positive" x={x} y={middle - up}
              width={barWidth} height={Math.max(2, up)} rx={4}
              pointerEvents="none"
            />}
            {day.removed > 0 && <rect
              className="viz-bar negative" x={x} y={middle}
              width={barWidth} height={Math.max(2, down)} rx={4}
              pointerEvents="none"
            />}
          </g>;
        })}
        {peak && peak.added > 0 && <text
          className="viz-value"
          x={data.flow.indexOf(peak) * slot + slot / 2}
          y={middle - peak.added * scale.unit - 8}
          textAnchor="middle"
        >{peak.added.toLocaleString("ko-KR")}</text>}
        <text className="viz-axis-label" x={44} y={height - 8}>{data.flow[0]?.date}</text>
        <text
          className="viz-axis-label" x={width} y={height - 8} textAnchor="end"
        >{data.flow.at(-1)?.date}</text>
      </svg>
      <Tooltip hover={hover}/>
    </div>
    <p className="viz-note">
      가장 많이 늘어난 날에만 값을 표시합니다. 나머지 값은 표 또는 커서로 확인하십시오.
    </p>
  </div>;
}

/** 성장 추이: 누적 자산 규모. 마지막 점은 상단 지표와 같은 값입니다. */
export function Growth({data, showTable, onCopy}: {
  data: Visualization; showTable: boolean; onCopy: CopyHandler;
}) {
  const rows = data.flow.map(day => [day.date, day.total]);
  if (showTable) {
    return <ViewTable
      headers={["일자", "누적 자산"]} rows={rows}
      onCopy={() => onCopy(["일자", "누적 자산"], rows, "growth")}
    />;
  }
  const width = 900;
  const height = 300;
  const area = {x: 44, y: 24, width: width - 104, height: height - 66};
  const {points, maximum} = growthGeometry(data.flow, area);
  const [index, setIndex] = React.useState(-1);
  const path = points.map((point, position) =>
    `${position === 0 ? "M" : "L"}${point.x.toFixed(1)},${point.y.toFixed(1)}`).join(" ");
  const fill = `${path} L${(points.at(-1)?.x ?? area.x).toFixed(1)},${area.y + area.height} L${area.x},${area.y + area.height} Z`;
  const current = index >= 0 ? points[index] : points.at(-1);
  return <div className="viz-body">
    <div className="viz-canvas">
      <svg
        viewBox={`0 0 ${width} ${height}`} role="img"
        aria-label="누적 자산 규모 추이"
        onPointerMove={event => {
          const svg = event.currentTarget;
          const box = svg.getBoundingClientRect();
          const x = ((event.clientX - box.left) / box.width) * width;
          setIndex(nearestIndex(points.map(point => point.x), x));
        }}
        onPointerLeave={() => setIndex(-1)}
      >
        {[0, 0.5, 1].map(ratio => <g key={ratio}>
          <line
            className="viz-grid" x1={area.x} x2={area.x + area.width}
            y1={area.y + area.height * ratio} y2={area.y + area.height * ratio}
          />
          <text
            className="viz-axis-label" x={area.x - 8}
            y={area.y + area.height * ratio + 4} textAnchor="end"
          >{compactNumber(Math.round(maximum * (1 - ratio)))}</text>
        </g>)}
        <path className="viz-area" d={fill}/>
        <path className="viz-line" d={path}/>
        {current && <>
          <line
            className="viz-crosshair" x1={current.x} x2={current.x}
            y1={area.y} y2={area.y + area.height}
          />
          <circle className="viz-marker" cx={current.x} cy={current.y} r={5}/>
          <text
            className="viz-value"
            x={Math.min(width - 8, current.x + 12)} y={Math.max(area.y + 12, current.y - 12)}
          >{current.point.total.toLocaleString("ko-KR")}</text>
        </>}
        <text className="viz-axis-label" x={area.x} y={height - 12}>{data.flow[0]?.date}</text>
        <text
          className="viz-axis-label" x={area.x + area.width} y={height - 12} textAnchor="end"
        >{data.flow.at(-1)?.date}</text>
      </svg>
      {current && <div className="viz-readout">
        <strong>{current.point.total.toLocaleString("ko-KR")}</strong>
        <span>{current.point.date}</span>
        <em>신규 {current.point.added} · 폐기 {current.point.removed}</em>
      </div>}
    </div>
    <p className="viz-note">
      곡선의 마지막 값은 상단 “관리 자산”과 동일합니다. 과거 값은 현재 수량에서
      일별 증감을 되짚어 계산합니다.
    </p>
  </div>;
}

/** 관계: 결정적 방사 배치. 같은 데이터는 항상 같은 그림으로 그립니다. */
export function Topology({data, showTable, onCopy, onSelect}: {
  data: Visualization; showTable: boolean; onCopy: CopyHandler;
  onSelect: (type: string) => void;
}) {
  const [focus, setFocus] = React.useState<string>("");
  const [hover, setHover] = React.useState<{x: number; y: number; text: string[]}|null>(null);
  // The canvas grows with the node count so the rings never crowd; past this the
  // API's own cap keeps the request bounded.
  const size = data.graph.nodes.length > 60 ? 680 : 560;
  const placed = React.useMemo(
    () => topologyLayout(
      data.graph.nodes,
      {x: size / 2, y: size / 2},
      size / 2 - 40,
      typeOrder,
      environmentOrder,
    ),
    [data.graph.nodes, size],
  );
  const byID = React.useMemo(
    () => new Map(placed.map(node => [node.id, node])),
    [placed],
  );
  const rows = data.graph.edges.map(edge => [
    byID.get(edge.source)?.name || edge.source,
    edge.type,
    byID.get(edge.target)?.name || edge.target,
  ]);
  if (showTable) {
    return <ViewTable
      headers={["원천", "관계", "대상"]} rows={rows}
      onCopy={() => onCopy(["원천", "관계", "대상"], rows, "topology")}
    />;
  }
  const types = foldTail(
    Object.entries(
      data.graph.nodes.reduce<Record<string, number>>((counts, node) => {
        counts[node.type] = (counts[node.type] || 0) + 1;
        return counts;
      }, {}),
    ).map(([label, count]) => ({label, count})),
    8,
  );
  const connected = (id: string) => data.graph.edges.some(edge =>
    (edge.source === focus && edge.target === id) ||
    (edge.target === focus && edge.source === id));
  const maximumDegree = Math.max(1, ...data.graph.nodes.map(node => node.degree));
  return <div className="viz-body">
    <div className="viz-lens">
      <SequentialKey maximum={maximumDegree} ramp="base" label="연결 수"/>
    </div>
    <div className="viz-type-filter" role="group" aria-label="자산 유형 필터">
      {types.map(item => <button
        key={item.label}
        onClick={() => onSelect(item.label.startsWith("기타") ? "" : item.label)}
        disabled={item.label.startsWith("기타")}
      >{item.label}<em>{item.count}</em></button>)}
    </div>
    <div className="viz-canvas centered">
      <svg
        viewBox={`0 0 ${size} ${size}`} role="img"
        aria-label={`자산 관계 그래프, 노드 ${placed.length}개, 관계 ${data.graph.edges.length}개`}
        onPointerLeave={() => { setHover(null); setFocus(""); }}
      >
        {data.graph.edges.map((edge, index) => {
          const source = byID.get(edge.source);
          const target = byID.get(edge.target);
          if (!source || !target) return null;
          const dimmed = focus && edge.source !== focus && edge.target !== focus;
          const control = edgeControlPoint(source, target, {x: size / 2, y: size / 2});
          return <path
            key={`${edge.source}-${edge.target}-${index}`}
            className={`viz-edge${dimmed ? " dimmed" : ""}`}
            d={`M${source.x.toFixed(1)},${source.y.toFixed(1)} Q${control.x.toFixed(1)},${control.y.toFixed(1)} ${target.x.toFixed(1)},${target.y.toFixed(1)}`}
          />;
        })}
        {placed.map(node => {
          const dimmed = !!focus && focus !== node.id && !connected(node.id);
          return <g key={node.id} className={dimmed ? "dimmed" : ""}>
            <circle
              cx={node.x} cy={node.y} r={Math.max(12, node.radius + 8)}
              fill="transparent"
              tabIndex={0}
              onPointerMove={event => {
                setFocus(node.id);
                setHover({
                  x: event.nativeEvent.offsetX, y: event.nativeEvent.offsetY,
                  text: [
                    node.name,
                    `${node.type} · ${node.environment}`,
                    `연결 ${node.degree}건${node.stale ? " · 정지" : ""}`,
                  ],
                });
              }}
              onFocus={() => {
                setFocus(node.id);
                setHover({
                  x: node.x, y: node.y,
                  text: [node.name, `${node.type} · ${node.environment}`, `연결 ${node.degree}건`],
                });
              }}
              onBlur={() => { setFocus(""); setHover(null); }}
            ><title>{`${node.name} (${node.type}) 연결 ${node.degree}건`}</title></circle>
            <circle
              className={`viz-node base-${node.step}${node.stale ? " stale" : ""}`}
              cx={node.x} cy={node.y} r={node.radius} pointerEvents="none"
            />
          </g>;
        })}
      </svg>
      <Tooltip hover={hover}/>
    </div>
    <p className="viz-note">
      원의 면적과 진하기는 모두 연결 수입니다. 색을 유형에 배정하지 않는 이유는
      노드 그래프가 임의의 두 색을 나란히 놓는 형태여서 여덟 색을 색각 이상에서
      구분할 수 없기 때문입니다. 유형은 위 버튼으로 좁히고, 개별 값은 커서와 표에서
      확인하십시오. 점선 테두리는 정지 자산입니다.
      {data.graph.truncated && ` 관계 ${data.graph.total_relations.toLocaleString("ko-KR")}건 중 연결이 많은 상위 자산만 표시합니다.`}
      {` 배치는 환경과 연결 수 순서로 고정되어 새로 고쳐도 같은 그림입니다.`}
    </p>
  </div>;
}

function Legend({items, shape, onSelect}: {
  items: {label: string; slot: number; note?: string; className?: string}[];
  shape: "rect"|"dot";
  onSelect?: (label: string) => void;
}) {
  if (items.length < 2) return null;
  return <ul className="viz-legend">
    {items.map(item => <li key={item.label}>
      <button
        type="button"
        onClick={onSelect ? () => onSelect(item.label) : undefined}
        className={onSelect ? "selectable" : ""}
      >
        <i
          className={`viz-key ${shape}${item.slot >= 0 ? ` slot-${item.slot + 1}` : ""}${
            item.className ? ` ${item.className}` : ""}`}
          aria-hidden="true"
        />
        {item.label}
        {item.note && <em>{item.note}</em>}
      </button>
    </li>)}
  </ul>;
}

function SequentialKey({maximum, ramp, label}: {
  maximum: number; ramp: "base"|"warn"; label?: string;
}) {
  return <div className="viz-seq-key" aria-hidden="true">
    {label && <span>{label}</span>}
    <span>0</span>
    {[1, 2, 3, 4, 5].map(step => <i key={step} className={`viz-cell ${ramp}-${step}`}/>)}
    <span>{compactNumber(maximum)}</span>
  </div>;
}

function Tooltip({hover}: {hover: {x: number; y: number; text: string[]}|null}) {
  if (!hover) return null;
  return <div
    className="viz-tooltip"
    style={{left: `${hover.x}px`, top: `${hover.y}px`}}
    role="status"
  >
    <strong>{hover.text[0]}</strong>
    {hover.text.slice(1).map(line => <span key={line}>{line}</span>)}
  </div>;
}

function ViewTable({headers, rows, onCopy}: {
  headers: string[]; rows: (string|number)[][]; onCopy: () => void;
}) {
  return <div className="viz-table">
    <div className="viz-table-actions">
      <button onClick={onCopy}><Copy size={14}/>CSV 복사</button>
    </div>
    <div className="table-scroll">
      <table>
        <thead><tr>{headers.map(header => <th key={header}>{header}</th>)}</tr></thead>
        <tbody>{rows.map((row, index) => <tr key={index}>
          {row.map((value, position) => <td key={position}>
            {typeof value === "number" ? value.toLocaleString("ko-KR") : value}
          </td>)}
        </tr>)}</tbody>
      </table>
    </div>
  </div>;
}

/**
 * The accessibility channel: one hand-drawn diagonal fill per slot, tone on
 * tone, rotated per slot so the marks stay distinguishable without colour.
 * Switched on by the 질감 구분 toggle and by forced-colors.
 */
function TexturePatterns() {
  return <svg className="viz-defs" aria-hidden="true" focusable="false">
    <defs>
      {[1, 2, 3, 4, 5, 6, 7, 8].map(slot => <pattern
        key={slot}
        id={`viz-texture-${slot}`}
        width="7" height="7"
        patternUnits="userSpaceOnUse"
        patternTransform={`rotate(${slot % 2 === 0 ? 135 : 45})`}
      >
        <rect width="7" height="7" className={`viz-texture-ground slot-${slot}`}/>
        <line x1="0" y1="0" x2="0" y2="7" className={`viz-texture-ink slot-${slot}`}/>
      </pattern>)}
    </defs>
  </svg>;
}
