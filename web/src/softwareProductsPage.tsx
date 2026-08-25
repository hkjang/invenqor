import React from "react";
import {
  Boxes,
  CheckCircle2,
  ChevronRight,
  Package,
  RefreshCw,
  Search,
  Server,
  ShieldCheck,
  Waypoints,
  X,
} from "lucide-react";
import {api} from "./api";
import {formatDate, formatRelative, number} from "./format";

export type SoftwareEvidence = {
  kind: string;
  name: string;
  source_asset_id?: string;
};

export type SoftwareProduct = {
  id: string;
  asset_key: string;
  status: string;
  product_key: string;
  product_name: string;
  role: string;
  vendor: string;
  version: string;
  versions: string[];
  install_state: "installed" | "observed" | "unknown";
  runtime_state: "running" | "stopped" | "unknown";
  service_names: string[];
  process_names: string[];
  package_names: string[];
  executable_paths: string[];
  evidence: SoftwareEvidence[];
  detection_method: string;
  catalog_version: string;
  evidence_count: number;
  process_count: number;
  confidence: number;
  host: {id: string; name: string};
  last_seen_at: string;
};

export type SoftwareSummary = {
  products: number;
  instances: number;
  hosts: number;
  running: number;
  stopped: number;
  runtime_unknown: number;
  installed: number;
  observed_only: number;
  high_confidence: number;
  needs_review: number;
  with_process_evidence: number;
  mapped_processes: number;
  top_products: {
    product_key: string;
    product_name: string;
    role: string;
    vendor: string;
    instances: number;
    hosts: number;
    running: number;
    versions: string[];
  }[];
};

type SoftwareResponse = {
  summary: SoftwareSummary;
  items: SoftwareProduct[];
  total: number;
  limit: number;
  offset: number;
  has_more: boolean;
  filters: {roles: string[]; vendors: string[]};
};

const roleLabels: Record<string, string> = {
  database: "데이터베이스",
  search: "검색 엔진",
  reverse_proxy: "리버스 프록시",
  web_server: "웹 서버",
  web_proxy: "웹·프록시",
  application_server: "애플리케이션 서버",
  web_browser: "웹 브라우저",
  productivity: "업무 생산성",
  collaboration: "협업·커뮤니케이션",
  message_broker: "메시지 브로커",
  container_runtime: "컨테이너 런타임",
  orchestrator: "오케스트레이션",
  application_runtime: "애플리케이션 런타임",
  remote_access: "원격 접속",
  observability: "관측·모니터링",
  monitoring: "모니터링",
  security: "보안",
  backup: "백업·복구",
  ci_cd: "CI/CD",
  guest_tools: "가상화 게스트 도구",
  asset_management: "자산 관리",
  other: "기타",
};

const evidenceLabels: Record<string, string> = {
  service: "서비스",
  process: "프로세스",
  package: "설치 패키지",
  executable: "실행 파일",
};

export const softwareRoleLabel = (role: string) => roleLabels[role] || role || "기타";

export const confidencePresentation = (confidence: number) => {
  const percent = Math.max(0, Math.min(100, Math.round((confidence || 0) * 100)));
  return {
    percent,
    label: percent >= 90 ? "매우 높음" : percent >= 80 ? "높음" : percent >= 60 ? "검토 권장" : "근거 부족",
    tone: percent >= 80 ? "good" : "warn",
  };
};

export const softwareTopBuckets = (summary: SoftwareSummary | null) =>
  (summary?.top_products || []).map(item => ({
    label: item.product_name,
    count: item.instances,
  }));

const emptySummary: SoftwareSummary = {
  products: 0, instances: 0, hosts: 0, running: 0, stopped: 0,
  runtime_unknown: 0, installed: 0, observed_only: 0, high_confidence: 0,
  needs_review: 0, with_process_evidence: 0, top_products: [],
  mapped_processes: 0,
};

export function SoftwareProductsPage() {
  const [response, setResponse] = React.useState<SoftwareResponse|null>(null);
  const [query, setQuery] = React.useState("");
  const [role, setRole] = React.useState("");
  const [vendor, setVendor] = React.useState("");
  const [runtime, setRuntime] = React.useState("");
  const [confidence, setConfidence] = React.useState("");
  const [offset, setOffset] = React.useState(0);
  const [selected, setSelected] = React.useState<SoftwareProduct|null>(null);
  const [error, setError] = React.useState("");
  const parameters = React.useMemo(() => {
    const values = new URLSearchParams({limit: "50", offset: String(offset)});
    if (query) values.set("q", query);
    if (role) values.set("role", role);
    if (vendor) values.set("vendor", vendor);
    if (runtime) values.set("runtime_state", runtime);
    if (confidence) values.set("confidence", confidence);
    return values;
  }, [confidence, offset, query, role, runtime, vendor]);
  const load = React.useCallback(async () => {
    try {
      setResponse(await api<SoftwareResponse>(`/api/v1/assets/software-products?${parameters}`));
      setError("");
    } catch (reason) {
      setError((reason as Error).message);
    }
  }, [parameters]);
  React.useEffect(() => {
    const timer = window.setTimeout(load, 150);
    return () => window.clearTimeout(timer);
  }, [load]);
  const changeFilter = (apply: (value: string) => void) => (value: string) => {
    apply(value);
    setOffset(0);
  };
  const closeDrawer = React.useCallback(() => setSelected(null), []);
  const summary = response?.summary || emptySummary;
  const first = response?.total ? offset + 1 : 0;
  const last = offset + (response?.items.length || 0);
  return <section>
    <PageTitle action={<button className="secondary" onClick={load}><RefreshCw size={15}/>새로고침</button>}/>
    <div className="software-assurance">
      <ShieldCheck size={20}/><div><strong>내장 카탈로그 자동 식별</strong>
        <span>Agent가 보낸 서비스·프로세스·설치 패키지를 서로 교차 검증해 주요 제품으로 정규화합니다. 별도 수동 매핑이 필요하지 않습니다.</span></div>
    </div>
    {error && <div className="error action-message">{error}</div>}
    <div className="software-metrics">
      <Metric label="식별 제품" value={number(summary.products)} note={`${number(summary.instances)}개 설치·실행 인스턴스`} icon={Package}/>
      <Metric label="관리 호스트" value={number(summary.hosts)} note="runs_on 관계 확인" icon={Server}/>
      <Metric label="현재 실행" value={number(summary.running)} note={`중지 ${number(summary.stopped)} · 미확인 ${number(summary.runtime_unknown)}`} icon={CheckCircle2}/>
      <Metric label="프로세스 자동 매핑" value={number(summary.mapped_processes)} note={`${number(summary.with_process_evidence)}개 제품 인스턴스의 근거`} icon={Waypoints}/>
      <Metric label="높은 신뢰도" value={number(summary.high_confidence)} note={`검토 권장 ${number(summary.needs_review)}`} icon={ShieldCheck}/>
    </div>
    <div className="software-layout">
      <Panel title="제품별 운영 분포" action="인스턴스 기준">
        <TopProducts summary={summary}/>
      </Panel>
      <Panel title="식별 품질" action="자동 검증">
        <div className="software-quality">
          <QualityRow label="설치 확인" value={summary.installed} total={summary.instances}/>
          <QualityRow label="실행 관찰만" value={summary.observed_only} total={summary.instances}/>
          <QualityRow label="높은 신뢰도" value={summary.high_confidence} total={summary.instances}/>
          <QualityRow label="프로세스 근거" value={summary.with_process_evidence} total={summary.instances}/>
        </div>
      </Panel>
    </div>
    <div className="filter-bar software-filters">
      <div className="search"><Search size={18}/><input value={query}
        aria-label="주요 소프트웨어 검색"
        onChange={event => changeFilter(setQuery)(event.target.value)}
        placeholder="제품, 호스트, 버전, 서비스 또는 프로세스 검색"/></div>
      <select aria-label="제품 역할" value={role} onChange={event => changeFilter(setRole)(event.target.value)}>
        <option value="">전체 역할</option>
        {(response?.filters.roles || []).map(value => <option key={value} value={value}>{softwareRoleLabel(value)}</option>)}
      </select>
      <select aria-label="제품 제조사" value={vendor} onChange={event => changeFilter(setVendor)(event.target.value)}>
        <option value="">전체 제조사</option>
        {(response?.filters.vendors || []).map(value => <option key={value} value={value}>{value === "unknown" ? "제조사 미확인" : value}</option>)}
      </select>
      <select aria-label="제품 실행 상태" value={runtime} onChange={event => changeFilter(setRuntime)(event.target.value)}>
        <option value="">전체 실행 상태</option><option value="running">실행 중</option>
        <option value="stopped">중지</option><option value="unknown">미확인</option>
      </select>
      <select aria-label="자동 식별 신뢰도" value={confidence} onChange={event => changeFilter(setConfidence)(event.target.value)}>
        <option value="">전체 신뢰도</option><option value="high">높음 (80% 이상)</option>
        <option value="review">검토 권장 (80% 미만)</option>
      </select>
    </div>
    <Panel title={`주요 소프트웨어 ${number(response?.total)}건`}
      action={response?.total ? `${number(first)}–${number(last)} 표시` : "자동 식별 결과"}>
      <SoftwareTable items={response?.items || []} onSelect={setSelected}/>
      <div className="pagination"><button className="secondary" disabled={!offset}
        onClick={() => setOffset(Math.max(0, offset - 50))}>이전</button>
        <button className="secondary" disabled={!response?.has_more}
          onClick={() => setOffset(offset + 50)}>다음</button></div>
    </Panel>
    {selected && <SoftwareProductDrawer product={selected} onClose={closeDrawer}/>}
  </section>;
}

export function SoftwareOverview({summary}: {summary: SoftwareSummary|null}) {
  return <article className="panel software-overview">
    <div className="panel-head"><h3>주요 소프트웨어</h3><a href="#/software">전체 보기</a></div>
    <div className="software-overview-summary">
      <div><strong>{number(summary?.products)}</strong><span>제품</span></div>
      <div><strong>{number(summary?.instances)}</strong><span>인스턴스</span></div>
      <div><strong>{number(summary?.running)}</strong><span>실행 중</span></div>
      <div><strong>{number(summary?.hosts)}</strong><span>호스트</span></div>
    </div>
    <TopProducts summary={summary || emptySummary} compact/>
    {!summary?.instances && <p className="software-empty-hint">다음 Agent 인벤토리에서 주요 제품을 자동 식별합니다.</p>}
  </article>;
}

// Exported so a test can render them directly. The page fetches on mount, so
// rendering it only ever exercises its loading state.
export function SoftwareTable({items, onSelect}: {items: SoftwareProduct[]; onSelect: (item: SoftwareProduct) => void}) {
  return <div className="table-wrap"><table className="software-table"><thead><tr>
    <th>제품</th><th>역할</th><th>호스트</th><th>버전</th><th>설치</th><th>실행</th><th>신뢰도 / 근거</th><th/>
  </tr></thead><tbody>{items.map(item => {
    const confidence = confidencePresentation(item.confidence);
    return <tr key={item.id} className="clickable" onClick={() => onSelect(item)}>
      <td><strong>{item.product_name}</strong><span>{item.vendor === "unknown" ? "제조사 미확인" : item.vendor}</span></td>
      <td>{softwareRoleLabel(item.role)}</td>
      <td><strong>{item.host.name || "호스트 관계 확인 전"}</strong><span>{formatRelative(item.last_seen_at)}</span></td>
      <td>{item.version || "—"}</td><td><State value={item.install_state}/></td>
      <td><State value={item.runtime_state}/></td>
      <td><div className={`confidence-pill ${confidence.tone}`}><strong>{confidence.percent}%</strong>
        <span>{item.evidence_count}개 근거</span></div></td><td><button type="button"
          className="software-detail-button" aria-label={`${item.product_name} 상세 보기`}
          onClick={event => {event.stopPropagation(); onSelect(item);}}><ChevronRight size={15}/></button></td>
    </tr>;
  })}</tbody></table>
    {!items.length && <Empty/>}</div>;
}

export function SoftwareProductDrawer({product, onClose}: {product: SoftwareProduct; onClose: () => void}) {
  const confidence = confidencePresentation(product.confidence);
  const drawerRef = React.useRef<HTMLElement>(null);
  const closeRef = React.useRef<HTMLButtonElement>(null);
  const titleID = React.useId();
  React.useEffect(() => {
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    closeRef.current?.focus();
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
        return;
      }
      if (event.key !== "Tab") return;
      const focusable = drawerRef.current?.querySelectorAll<HTMLElement>(
        'button:not([disabled]),a[href],input:not([disabled]),select:not([disabled]),textarea:not([disabled]),[tabindex]:not([tabindex="-1"])',
      );
      if (!focusable?.length) return;
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    };
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.removeEventListener("keydown", handleKeyDown);
      previous?.focus();
    };
  }, [onClose]);
  return <div className="drawer-backdrop" onMouseDown={event => {
    if (event.target === event.currentTarget) onClose();
  }}><aside ref={drawerRef} className="drawer" role="dialog" aria-modal="true" aria-labelledby={titleID}>
    <header><div><h2 id={titleID}>{product.product_name}</h2><small>{softwareRoleLabel(product.role)} · {product.product_key}</small></div>
      <button ref={closeRef} type="button" onClick={onClose} aria-label="상세 화면 닫기"><X/></button></header>
    <div className="drawer-content software-detail">
      <section className="software-confidence-card"><ShieldCheck/><div><span>자동 식별 신뢰도</span>
        <strong>{confidence.percent}% · {confidence.label}</strong>
        <small>{product.detection_method === "builtin_catalog" ? "Invenqor 내장 카탈로그" : product.detection_method}
          {product.catalog_version ? ` · ${product.catalog_version}` : ""}</small></div>
        <div className="confidence-meter"><i style={{width: `${confidence.percent}%`}}/></div></section>
      <div className="detail-grid">
        <DataRow label="제품 역할" value={softwareRoleLabel(product.role)}/><DataRow label="제조사" value={product.vendor === "unknown" ? "확인되지 않음" : product.vendor}/>
        <DataRow label="버전" value={product.version || "확인 전"}/><DataRow label="관리 호스트" value={product.host.name || "관계 확인 전"}/>
        <DataRow label="설치 상태" value={stateLabel(product.install_state)}/><DataRow label="실행 상태" value={stateLabel(product.runtime_state)}/>
        <DataRow label="최근 확인" value={formatDate(product.last_seen_at)}/><DataRow label="Asset key" value={product.asset_key}/>
        <DataRow label="매핑 프로세스" value={`${number(product.process_count)}개`}/><DataRow label="전체 식별 근거" value={`${number(product.evidence_count)}개`}/>
      </div>
      <EvidenceGroup title="서비스" values={product.service_names}/>
      <EvidenceGroup title="프로세스" values={product.process_names}/>
      <EvidenceGroup title="설치 패키지" values={product.package_names}/>
      <EvidenceGroup title="실행 경로" values={product.executable_paths}/>
      <section className="drawer-section"><h3>식별 근거 <span>{product.evidence_count}</span></h3>
        <p className="software-evidence-help">동일 호스트의 서비스·프로세스·패키지 신호를 내장 카탈로그와 대조한 결과입니다.</p>
        {product.evidence_count > product.evidence.length && <p className="software-evidence-help">
          전체 {number(product.evidence_count)}개 중 대표 근거 {number(product.evidence.length)}개를 표시합니다.
        </p>}
        <div className="software-evidence-list">{product.evidence.map((evidence, index) =>
          <div key={`${evidence.kind}-${evidence.name}-${index}`}><span>{evidenceLabels[evidence.kind] || evidence.kind}</span>
            <strong>{evidence.name}</strong><code>{evidence.source_asset_id || "collector evidence"}</code></div>)}
          {!product.evidence.length && <p>표시할 세부 근거가 없습니다.</p>}</div>
      </section>
    </div>
  </aside></div>;
}

export function EvidenceGroup({title, values}: {title: string; values: string[]}) {
  if (!values.length) return null;
  return <section className="drawer-section software-signals"><h3>{title} <span>{values.length}</span></h3>
    <div>{values.map(value => <code key={value}>{value}</code>)}</div></section>;
}

export function TopProducts({summary, compact = false}: {summary: SoftwareSummary; compact?: boolean}) {
  const items = (summary.top_products || []).slice(0, compact ? 5 : 10);
  const maximum = Math.max(1, ...items.map(item => item.instances));
  return <div className={`software-top-products ${compact ? "compact" : ""}`}>{items.map(item =>
    <div key={item.product_key}><div><strong>{item.product_name}</strong>
      <span>{softwareRoleLabel(item.role)} · 호스트 {number(item.hosts)}</span></div>
      <div className="software-bar"><i style={{width: `${item.instances / maximum * 100}%`}}/></div>
      <b>{number(item.instances)}</b></div>)}
    {!items.length && !compact && <Empty/>}</div>;
}

function QualityRow({label, value, total}: {label: string; value: number; total: number}) {
  const percent = total ? Math.round(value / total * 100) : 0;
  return <div><span><strong>{label}</strong><small>{number(value)}건</small></span>
    <div><i style={{width: `${percent}%`}}/></div><b>{percent}%</b></div>;
}

const stateLabel = (value: string) => ({
  installed: "설치 확인", observed: "실행 관찰", running: "실행 중",
  stopped: "중지", unknown: "미확인",
}[value] || value);

function State({value}: {value: string}) {
  return <span className={`software-state ${value}`}>{stateLabel(value)}</span>;
}

function Metric({label, value, note, icon: Icon}: {label: string; value: string; note: string; icon: React.ElementType}) {
  return <article className="metric"><div><span>{label}</span><strong>{value}</strong><small>{note}</small></div><Icon/></article>;
}

function PageTitle({action}: {action: React.ReactNode}) {
  return <div className="page-title with-action"><div><p className="eyebrow dark">SOFTWARE INTELLIGENCE</p>
    <h1>주요 소프트웨어</h1><p>프로세스·서비스·패키지를 제품 단위로 자동 정규화해 설치, 실행 상태와 근거를 관리합니다.</p></div>
    <div>{action}</div></div>;
}

function Panel({title, action, children}: {title: string; action: string; children: React.ReactNode}) {
  return <article className="panel"><div className="panel-head"><h3>{title}</h3><span>{action}</span></div>{children}</article>;
}

function DataRow({label, value}: {label: string; value: string}) {
  return <div className="data-row"><span>{label}</span><strong>{value}</strong></div>;
}

function Empty() {
  return <div className="empty"><Boxes/><p>자동 식별된 주요 소프트웨어가 없습니다.</p></div>;
}
