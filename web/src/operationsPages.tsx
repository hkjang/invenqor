import React from "react";
import {
  Activity,
  AlertTriangle,
  Boxes,
  CheckCircle2,
  Copy,
  FileSearch,
  GitMerge,
  KeyRound,
  Link2,
  Network,
  Plus,
  RefreshCw,
  Save,
  Scissors,
  Search,
  ShieldCheck,
  Trash2,
  X,
} from "lucide-react";
import {api} from "./api";
import type {SystemInfo} from "./productVersion";

export type Asset = {
  id: string;
  asset_key: string;
  name: string;
  type: string;
  status: string;
  criticality: string;
  environment: string;
  owner_department: string;
  location: string;
  confidence: number;
  attributes: Record<string, unknown>;
  custom_fields: Record<string, unknown>;
  source: string;
  first_seen_at: string;
  last_seen_at: string;
  deleted_at?: string | null;
};

type Agent = {
  id: string;
  agent_id: string;
  hostname: string;
  status: string;
  version: string;
  os_name: string;
  architecture: string;
  auth_method: string;
  last_seen_at?: string;
  last_inventory_at?: string;
};
type Bucket = {label: string; count: number};
type Statistics = {
  generated_at: string;
  assets: {
    total: number;
    seen_24h: number;
    stale: number;
    by_type: Bucket[];
    by_status: Bucket[];
    by_environment: Bucket[];
    by_criticality: Bucket[];
    by_source: Bucket[];
  };
  agents: {
    total: number;
    healthy: number;
    attention: number;
    by_status: Bucket[];
    by_os: Bucket[];
  };
  collection: {
    events_24h: number;
    failed_24h: number;
    daily: {date: string; events: number; failed: number}[];
  };
};
type PermissionContext = {
  permissions: string[];
  superAdmin: boolean;
};

const can = (access: PermissionContext, permission: string) =>
  access.superAdmin || access.permissions.includes(permission);
const jsonRequest = (csrf: string, body: unknown, method = "POST"): RequestInit => ({
  method,
  headers: {"Content-Type": "application/json", "X-CSRF-Token": csrf},
  body: JSON.stringify(body),
});

export function OperationsDashboard({
  systemInfo,
  refreshSeconds = 60,
}: {
  systemInfo: SystemInfo|null;
  refreshSeconds?: number;
}) {
  const [statistics, setStatistics] = React.useState<Statistics|null>(null);
  const [recent, setRecent] = React.useState<Asset[]>([]);
  const [agents, setAgents] = React.useState<Agent[]>([]);
  const [error, setError] = React.useState("");
  const load = React.useCallback(async () => {
    setError("");
    try {
      const [stats, assets, fleet] = await Promise.all([
        api<Statistics>("/api/v1/dashboard/statistics"),
        api<{items: Asset[]}>("/api/v1/assets?limit=6"),
        api<{agents: Agent[]}>("/api/v1/admin/agents").catch(() => ({agents: []})),
      ]);
      setStatistics(stats); setRecent(assets.items); setAgents(fleet.agents);
    } catch (reason) {
      setError((reason as Error).message);
    }
  }, []);
  React.useEffect(() => { load(); }, [load]);
  React.useEffect(() => {
    if (!refreshSeconds) return;
    const timer = window.setInterval(load, refreshSeconds * 1000);
    return () => window.clearInterval(timer);
  }, [load, refreshSeconds]);
  const freshRate = statistics?.assets.total
    ? Math.round(statistics.assets.seen_24h / statistics.assets.total * 100)
    : 100;
  const healthyRate = statistics?.agents.total
    ? Math.round(statistics.agents.healthy / statistics.agents.total * 100)
    : 100;
  return <section>
    <PageTitle kicker="ASSET INTELLIGENCE" title="운영 통계"
      subtitle="자산 최신성, 수집 건전성, 구성 분포를 한 화면에서 판단합니다."
      action={<button className="secondary" onClick={load}><RefreshCw size={15}/>새로고침</button>}/>
    {systemInfo?.database_mode === "SQLITE_FALLBACK" &&
      <Notice tone="warning" title="SQLite 대체 모드">
        운영 PostgreSQL이 연결되지 않았습니다. 설정에서 연결을 검증하십시오.
      </Notice>}
    {error && <div className="error">{error}</div>}
    <div className="metrics executive">
      <Metric label="관리 자산" value={number(statistics?.assets.total)}
        note={`24시간 내 확인 ${number(statistics?.assets.seen_24h)}`} icon={Boxes}/>
      <Metric label="자산 최신성" value={`${freshRate}%`}
        note={`점검 필요 ${number(statistics?.assets.stale)}`} icon={CheckCircle2}/>
      <Metric label="정상 Agent" value={`${number(statistics?.agents.healthy)} / ${number(statistics?.agents.total)}`}
        note={`건전성 ${healthyRate}%`} icon={Activity}/>
      <Metric label="24시간 수집" value={number(statistics?.collection.events_24h)}
        note={`실패 ${number(statistics?.collection.failed_24h)}`} icon={Network}/>
    </div>
    <div className="analytics-grid">
      <Panel title="7일 수집 추이" action="Agent event">
        <DailyBars items={statistics?.collection.daily || []}/>
      </Panel>
      <Panel title="운영 주의 항목" action="우선순위">
        <RiskSummary statistics={statistics}/>
      </Panel>
      <Panel title="자산 유형" action="구성 비중">
        <Breakdown items={statistics?.assets.by_type || []}/>
      </Panel>
      <Panel title="운영 환경" action="배치 분포">
        <Breakdown items={statistics?.assets.by_environment || []}/>
      </Panel>
      <Panel title="중요도" action="비즈니스 영향">
        <Breakdown items={statistics?.assets.by_criticality || []}/>
      </Panel>
      <Panel title="수집 원천" action="데이터 계보">
        <Breakdown items={statistics?.assets.by_source || []}/>
      </Panel>
    </div>
    <div className="dashboard-grid">
      <Panel title="최근 확인 자산" action="최근 확인 순"><AssetTable items={recent}/></Panel>
      <Panel title="Agent 상태" action="30분 기준">
        <div className="agent-list">{agents.slice(0, 8).map(agent =>
          <div key={agent.id}><i className={recentEnough(agent.last_seen_at, 30) ? "ok" : ""}/>
            <div><strong>{agent.hostname || agent.agent_id}</strong>
              <span>{agent.os_name || "운영체제 확인 전"} · {formatDate(agent.last_seen_at)}</span></div>
            <Badge value={agent.status}/></div>)}
          {!agents.length && <Empty icon={Activity} text="등록된 Agent가 없습니다."/>}
        </div>
      </Panel>
    </div>
  </section>;
}

export function AssetsPage({csrf, access}: {csrf: string; access: PermissionContext}) {
  const [items, setItems] = React.useState<Asset[]>([]);
  const [query, setQuery] = React.useState("");
  const [type, setType] = React.useState("");
  const [status, setStatus] = React.useState("");
  const [includeDeleted, setIncludeDeleted] = React.useState(false);
  const [offset, setOffset] = React.useState(0);
  const [hasMore, setHasMore] = React.useState(false);
  const [selected, setSelected] = React.useState<string[]>([]);
  const [detailID, setDetailID] = React.useState<string|null>(null);
  const [creating, setCreating] = React.useState(false);
  const [error, setError] = React.useState("");
  const load = React.useCallback(async () => {
    const params = new URLSearchParams({
      limit: "50", offset: String(offset), q: query, type, status,
      include_deleted: String(includeDeleted),
    });
    try {
      const result = await api<{items: Asset[]; has_more: boolean}>(`/api/v1/assets?${params}`);
      setItems(result.items); setHasMore(result.has_more); setError("");
    } catch (reason) { setError((reason as Error).message); }
  }, [includeDeleted, offset, query, status, type]);
  React.useEffect(() => {
    const timer = window.setTimeout(load, 180);
    return () => window.clearTimeout(timer);
  }, [load]);
  const merge = async () => {
    if (selected.length < 2) return;
    const reason = window.prompt("병합 사유를 입력하십시오.", "중복 자산 정리");
    if (reason === null || !window.confirm(`첫 번째 선택 자산으로 ${selected.length - 1}개 자산을 병합합니까?`)) return;
    try {
      await api("/api/v1/assets/merge", jsonRequest(csrf, {
        primary_id: selected[0], secondary_ids: selected.slice(1), reason,
      }));
      setSelected([]); await load();
    } catch (reason) { setError((reason as Error).message); }
  };
  return <section>
    <PageTitle kicker="CONFIGURATION ITEMS" title="자산 인벤토리"
      subtitle="검색부터 생성·수정·계보·관계·병합·분할까지 자산 수명주기를 관리합니다."
      action={<>{can(access, "assets.merge") && <button className="secondary" disabled={selected.length < 2} onClick={merge}><GitMerge size={15}/>선택 병합</button>}
        {can(access, "assets.write") && <button className="primary compact" onClick={() => setCreating(true)}><Plus size={15}/>자산 등록</button>}</>}/>
    <div className="filter-bar">
      <div className="search"><Search size={18}/><input placeholder="이름 또는 자산 키" value={query}
        onChange={event => {setQuery(event.target.value); setOffset(0);}}/></div>
      <input placeholder="유형" value={type} onChange={event => {setType(event.target.value); setOffset(0);}}/>
      <select value={status} onChange={event => {setStatus(event.target.value); setOffset(0);}}>
        <option value="">전체 상태</option><option value="active">active</option>
        <option value="discovered">discovered</option><option value="deleted">deleted</option>
      </select>
      <label><input type="checkbox" checked={includeDeleted} onChange={event => setIncludeDeleted(event.target.checked)}/>삭제 포함</label>
      <button className="secondary" onClick={load}><RefreshCw size={15}/></button>
    </div>
    {error && <div className="error action-message">{error}</div>}
    <Panel title={`${items.length}개 자산`} action={`offset ${offset}`}>
      <AssetTable items={items} selected={selected} onToggle={id =>
        setSelected(current => current.includes(id) ? current.filter(value => value !== id) : [...current, id])}
        onSelect={setDetailID}/>
      <div className="pagination">
        <button className="secondary" disabled={!offset} onClick={() => setOffset(Math.max(0, offset - 50))}>이전</button>
        <button className="secondary" disabled={!hasMore} onClick={() => setOffset(offset + 50)}>다음</button>
      </div>
    </Panel>
    {creating && <AssetEditor csrf={csrf} onClose={() => setCreating(false)} onSaved={async () => {
      setCreating(false); await load();
    }}/>}
    {detailID && <AssetDetail id={detailID} csrf={csrf} access={access}
      onClose={() => setDetailID(null)} onChanged={load}/>}
  </section>;
}

export function AgentsPage({
  csrf,
  access,
  systemInfo,
}: {
  csrf: string;
  access: PermissionContext;
  systemInfo: SystemInfo|null;
}) {
  const [items, setItems] = React.useState<Agent[]>([]);
  const [agentID, setAgentID] = React.useState("");
  const [hostname, setHostname] = React.useState("");
  const [secret, setSecret] = React.useState("");
  const [error, setError] = React.useState("");
  const [file, setFile] = React.useState<File|null>(null);
  const [version, setVersion] = React.useState("");
  const [architecture, setArchitecture] = React.useState("x86_64");
  const [channel, setChannel] = React.useState("stable");
  const [signature, setSignature] = React.useState("");
  const [rollout, setRollout] = React.useState(100);
  const load = React.useCallback(() =>
    api<{agents: Agent[]}>("/api/v1/admin/agents").then(value => setItems(value.agents)),
  []);
  React.useEffect(() => {
    load().catch(reason => setError((reason as Error).message));
    const timer = window.setInterval(() => load().catch(() => {}), 15000);
    return () => window.clearInterval(timer);
  }, [load]);
  const mutate = async (work: () => Promise<unknown>) => {
    setError("");
    try { await work(); await load(); } catch (reason) { setError((reason as Error).message); }
  };
  const provision = (event: React.FormEvent) => {
    event.preventDefault();
    mutate(async () => {
      const result = await api<{token: string}>("/api/v1/admin/agents",
        jsonRequest(csrf, {agent_id: agentID, hostname}));
      setSecret(result.token); setAgentID(""); setHostname("");
    });
  };
  const rotate = (agent: Agent) => {
    const grace = window.prompt("기존 토큰 유예시간(초)", "3600");
    if (grace === null) return;
    mutate(async () => {
      const result = await api<{token: string}>(`/api/v1/admin/agents/${agent.id}/tokens/rotate`,
        jsonRequest(csrf, {grace_seconds: Number(grace)}));
      setSecret(result.token);
    });
  };
  const registerCertificate = (agent: Agent) => {
    const fingerprint = window.prompt("SHA-256 인증서 fingerprint를 입력하십시오.");
    if (!fingerprint) return;
    const expiresAt = window.prompt("만료 시각 RFC3339 (선택)", "");
    mutate(() => api(`/api/v1/admin/agents/${agent.id}/certificates`,
      jsonRequest(csrf, {fingerprint, ...(expiresAt ? {expires_at: expiresAt} : {})})));
  };
  const publish = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!file) return;
    const body = new FormData();
    body.set("artifact", file); body.set("version", version); body.set("channel", channel);
    body.set("os", "linux"); body.set("architecture", architecture);
    body.set("signature", signature); body.set("rollout_percent", String(rollout));
    try {
      await api("/api/v1/admin/agent-updates", {
        method: "POST", headers: {"X-CSRF-Token": csrf}, body,
      });
      setFile(null); setVersion(""); setSignature("");
    } catch (reason) { setError((reason as Error).message); }
  };
  return <section>
    <PageTitle kicker="COLLECTION FLEET" title="Agent 관리"
      subtitle="자동 등록을 기본으로 사용하고, 예외 장비의 자격 증명과 배포를 중앙 통제합니다."
      action={<button className="secondary" onClick={load}><RefreshCw size={15}/>새로고침</button>}/>
    {systemInfo?.agent_enrollment_mode === "disabled"
      ? <Notice tone="warning" title="Agent 자동 등록이 비활성화되어 있습니다.">
          Server의 INVENQOR_AGENT_AUTO_ENROLLMENT를 true로 설정하거나 예외 장비를 수동 등록하십시오.
        </Notice>
      : <Notice tone="info" title={
        systemInfo?.agent_enrollment_mode === "token"
          ? "Zero-touch 자동 등록 · 공용 Token 보호"
          : "Zero-touch 자동 등록 · URL-only"
      }>
        {systemInfo?.agent_enrollment_mode === "token"
          ? "Agent URL과 사이트 enrollment token을 설정하면 장비 전용 Token을 자동 발급하고 보관합니다."
          : "Agent config.toml에 Server URL만 설정하면 최초 연결에서 장비 전용 Token을 자동 발급하고 보관합니다."}
      </Notice>}
    {secret && <SecretReveal secret={secret} onClose={() => setSecret("")}/>}
    {error && <div className="error action-message">{error}</div>}
    {can(access, "agents.manage") && <div className="agent-admin-grid">
      <Panel title="예외 장비 수동 등록" action="자동 등록 권장">
        <form className="compact-form" onSubmit={provision}>
          <label>Agent UUID<input value={agentID} onChange={event => setAgentID(event.target.value)} required/></label>
          <label>Hostname<input value={hostname} onChange={event => setHostname(event.target.value)}/></label>
          <button className="primary compact"><KeyRound size={15}/>장비 토큰 발급</button>
        </form>
      </Panel>
      <Panel title="서명된 Agent 업데이트" action="최대 128 MiB">
        <form className="compact-form update-form" onSubmit={publish}>
          <label>Artifact<input type="file" onChange={event => setFile(event.target.files?.[0] || null)} required/></label>
          <label>버전<input value={version} onChange={event => setVersion(event.target.value)} placeholder="0.2.3" required/></label>
          <label>채널<select value={channel} onChange={event => setChannel(event.target.value)}><option>stable</option><option>beta</option></select></label>
          <label>아키텍처<select value={architecture} onChange={event => setArchitecture(event.target.value)}><option>x86_64</option><option>aarch64</option></select></label>
          <label className="wide">Ed25519 Signature (Base64)<textarea value={signature} onChange={event => setSignature(event.target.value)} required/></label>
          <label>Rollout %<input type="number" min="0" max="100" value={rollout} onChange={event => setRollout(Number(event.target.value))}/></label>
          <button className="primary compact"><Save size={15}/>업데이트 게시</button>
        </form>
      </Panel>
    </div>}
    <div className="card-grid agent-cards">{items.map(agent =>
      <article className="agent-card" key={agent.id}>
        <div className="host-icon"><Activity/></div><Badge value={agent.status}/>
        <h3>{agent.hostname || "이름 없는 Agent"}</h3><p>{agent.agent_id}</p>
        <dl><dt>버전</dt><dd>{agent.version || "—"}</dd>
          <dt>운영체제</dt><dd>{agent.os_name || "—"} {agent.architecture}</dd>
          <dt>인증</dt><dd>{agent.auth_method || "—"}</dd>
          <dt>최근 연결</dt><dd>{formatDate(agent.last_seen_at)}</dd></dl>
        {can(access, "agents.manage") && <div className="card-actions">
          <button onClick={() => rotate(agent)}><RefreshCw size={14}/>토큰 회전</button>
          <button onClick={() => registerCertificate(agent)}><ShieldCheck size={14}/>mTLS</button>
          <button className={agent.status === "blocked" ? "" : "danger"} onClick={() =>
            mutate(() => api(`/api/v1/admin/agents/${agent.id}/${agent.status === "blocked" ? "unblock" : "block"}`,
              jsonRequest(csrf, {})))}>
            {agent.status === "blocked" ? "차단 해제" : "차단"}
          </button>
        </div>}
      </article>)}
      {!items.length && <Empty icon={Activity} text="등록된 Agent가 없습니다. 자동 등록 설정과 Agent 로그를 확인하십시오."/>}
    </div>
  </section>;
}

export function QueryPage({csrf}: {csrf: string}) {
  const [query, setQuery] = React.useState('type = "host" AND environment = "production"');
  const [limit, setLimit] = React.useState(100);
  const [result, setResult] = React.useState<Asset[]>([]);
  const [validation, setValidation] = React.useState<{valid: boolean; error?: string; ast?: unknown}|null>(null);
  const [error, setError] = React.useState("");
  const validate = async () => {
    try {
      setValidation(await api("/api/v1/query/validate", jsonRequest(csrf, {query, limit})));
      setError("");
    } catch (reason) { setError((reason as Error).message); }
  };
  const run = async () => {
    try {
      const value = await api<{items: Asset[]; ast: unknown}>("/api/v1/query/execute",
        jsonRequest(csrf, {query, limit}));
      setResult(value.items); setValidation({valid: true, ast: value.ast}); setError("");
    } catch (reason) { setError((reason as Error).message); }
  };
  return <section>
    <PageTitle kicker="SAFE DISCOVERY" title="Query DSL"
      subtitle="구문 검증과 제한된 실행을 분리해 안전하게 인벤토리를 탐색합니다."/>
    <div className="query-box"><div className="query-head"><FileSearch size={20}/><strong>질의 편집기</strong>
      <label>최대 <input type="number" min="1" max="500" value={limit} onChange={event => setLimit(Number(event.target.value))}/></label></div>
      <textarea value={query} onChange={event => {setQuery(event.target.value); setValidation(null);}} spellCheck={false}/>
      <div className="query-actions">
        {validation && <span className={validation.valid ? "valid-text" : "error-text"}>
          {validation.valid ? "구문이 유효합니다." : validation.error}</span>}
        {error && <span className="error-text">{error}</span>}
        <button className="secondary" onClick={validate}>구문 검증</button>
        <button className="primary compact" onClick={run}>질의 실행</button>
      </div>
    </div>
    {validation?.ast != null && <details className="json-details"><summary>파싱된 AST</summary><pre>{pretty(validation.ast)}</pre></details>}
    <Panel title={`결과 ${result.length}건`} action={`limit ${limit}`}><AssetTable items={result}/></Panel>
  </section>;
}

export function AuditPage() {
  const [items, setItems] = React.useState<AuditEvent[]>([]);
  const [query, setQuery] = React.useState("");
  const [limit, setLimit] = React.useState(200);
  const [error, setError] = React.useState("");
  const load = React.useCallback(() =>
    api<{items: AuditEvent[]}>(`/api/v1/admin/audit?limit=${limit}`)
      .then(value => {setItems(value.items); setError("");})
      .catch(reason => setError((reason as Error).message)),
  [limit]);
  React.useEffect(() => { load(); }, [load]);
  const visible = items.filter(item =>
    `${item.action} ${item.actor_name} ${item.resource_type} ${item.resource_id || ""}`
      .toLowerCase().includes(query.toLowerCase()));
  return <section>
    <PageTitle kicker="ACCOUNTABILITY" title="감사 로그"
      subtitle="행위자·대상·요청·변경 전후 값을 추적해 운영 책임성과 조사 가능성을 확보합니다."
      action={<button className="secondary" onClick={load}><RefreshCw size={15}/>새로고침</button>}/>
    <div className="filter-bar"><div className="search"><Search size={16}/><input value={query}
      onChange={event => setQuery(event.target.value)} placeholder="행위, 사용자, 자원 검색"/></div>
      <select value={limit} onChange={event => setLimit(Number(event.target.value))}>
        <option value="100">100건</option><option value="200">200건</option><option value="500">500건</option>
      </select></div>
    {error && <div className="error">{error}</div>}
    <Panel title={`${visible.length}개 이벤트`} action="최신 순">
      <div className="audit-table">{visible.map(item => <details key={item.id}>
        <summary><i className={item.result === "success" ? "ok" : "bad"}/>
          <time>{formatDate(item.occurred_at)}</time><strong>{item.action}</strong>
          <span>{item.actor_name || item.actor_type} → {item.resource_type}</span><Badge value={item.result}/></summary>
        <div className="audit-detail">
          <DataRow label="Resource" value={`${item.resource_type} / ${item.resource_id || "—"}`}/>
          <DataRow label="Request ID" value={item.request_id || "—"}/>
          <DataRow label="Source IP" value={item.source_ip || "—"}/>
          <DataRow label="Reason" value={item.reason || "—"}/>
          <pre>{pretty({before: item.before, after: item.after, metadata: item.metadata})}</pre>
        </div>
      </details>)}</div>
    </Panel>
  </section>;
}

export function ServerLogsPage() {
  const [items, setItems] = React.useState<DiagnosticLog[]>([]);
  const [instances, setInstances] = React.useState<string[]>([]);
  const [retention, setRetention] = React.useState({days: 30, maximum_events: 10000});
  const [query, setQuery] = React.useState("");
  const [level, setLevel] = React.useState("");
  const [component, setComponent] = React.useState("");
  const [instance, setInstance] = React.useState("");
  const [limit, setLimit] = React.useState(200);
  const [autoRefresh, setAutoRefresh] = React.useState(true);
  const [error, setError] = React.useState("");
  const load = React.useCallback(async () => {
    const parameters = new URLSearchParams({limit: String(limit)});
    if (query.trim()) parameters.set("q", query.trim());
    if (level) parameters.set("level", level);
    if (component) parameters.set("component", component);
    if (instance) parameters.set("instance_id", instance);
    try {
      const value = await api<{
        items: DiagnosticLog[];
        instances: string[];
        retention: {days: number; maximum_events: number};
      }>(`/api/v1/admin/diagnostics/logs?${parameters}`);
      setItems(value.items);
      setInstances(value.instances);
      setRetention(value.retention);
      setError("");
    } catch (reason) {
      setError((reason as Error).message);
    }
  }, [component, instance, level, limit, query]);
  React.useEffect(() => { load(); }, [load]);
  React.useEffect(() => {
    if (!autoRefresh) return;
    const timer = window.setInterval(load, 15_000);
    return () => window.clearInterval(timer);
  }, [autoRefresh, load]);
  const errors = items.filter(item => item.level === "error").length;
  const warnings = items.filter(item => item.level === "warning").length;
  return <section>
    <PageTitle
      kicker="MULTI-POD DIAGNOSTICS"
      title="Server 진단 로그"
      subtitle="모든 Server Pod의 Agent 등록·전송 실패와 운영 오류를 공용 DB에서 request ID로 추적합니다."
      action={<button className="secondary" onClick={load}><RefreshCw size={15}/>새로고침</button>}
    />
    <div className="status-grid diagnostic-summary">
      <div><span>조회 이벤트</span><strong>{items.length}</strong></div>
      <div><span>Error</span><strong>{errors}</strong></div>
      <div><span>Warning</span><strong>{warnings}</strong></div>
      <div><span>확인된 Pod</span><strong>{instances.length}</strong></div>
    </div>
    <div className="filter-bar diagnostic-filters">
      <div className="search"><Search size={16}/><input value={query}
        onChange={event => setQuery(event.target.value)}
        onKeyDown={event => event.key === "Enter" && load()}
        placeholder="request ID, Agent ID, 오류 코드, IP 검색"/></div>
      <select value={level} onChange={event => setLevel(event.target.value)}>
        <option value="">모든 수준</option><option value="error">Error</option>
        <option value="warning">Warning</option><option value="info">Info</option>
      </select>
      <select value={component} onChange={event => setComponent(event.target.value)}>
        <option value="">모든 구성요소</option>
        <option value="agent_enrollment">Agent 등록</option>
        <option value="agent_transport">Agent 전송</option>
        <option value="http">Server HTTP</option>
      </select>
      <select value={instance} onChange={event => setInstance(event.target.value)}>
        <option value="">모든 Pod</option>
        {instances.map(value => <option value={value} key={value}>{value}</option>)}
      </select>
      <select value={limit} onChange={event => setLimit(Number(event.target.value))}>
        <option value="100">100건</option><option value="200">200건</option>
        <option value="500">500건</option>
      </select>
      <label className="auto-refresh"><input type="checkbox" checked={autoRefresh}
        onChange={event => setAutoRefresh(event.target.checked)}/>15초 자동 갱신</label>
    </div>
    {error && <div className="error">{error}</div>}
    <Panel title={`${items.length}개 진단 이벤트`} action={`보존 ${retention.days}일 · 최대 ${retention.maximum_events.toLocaleString()}건`}>
      <div className="audit-table diagnostic-log-table">
        {items.map(item => <details key={item.id}>
          <summary>
            <i className={item.level === "info" ? "ok" : "bad"}/>
            <time>{formatDate(item.occurred_at)}</time>
            <strong>{item.event_code}</strong>
            <span>{item.instance_id} · {item.component}</span>
            <Badge value={item.level}/>
          </summary>
          <div className="audit-detail">
            <DataRow label="Message" value={item.message}/>
            <DataRow label="Pod / Instance" value={item.instance_id}/>
            <DataRow label="Request ID" value={item.request_id || "—"}/>
            <DataRow label="Agent ID" value={item.agent_id || "—"}/>
            <DataRow label="Source IP" value={item.source_ip || "—"}/>
            <DataRow label="Component" value={item.component}/>
            <pre>{pretty(item.details)}</pre>
          </div>
        </details>)}
        {!items.length && <Empty icon={ShieldCheck} text="조건에 맞는 진단 이벤트가 없습니다."/>}
      </div>
    </Panel>
  </section>;
}

function AssetEditor({csrf, asset, onClose, onSaved}: {
  csrf: string; asset?: Asset; onClose: () => void; onSaved: () => void;
}) {
  const [form, setForm] = React.useState({
    asset_key: asset?.asset_key || "", name: asset?.name || "", type: asset?.type || "host",
    status: asset?.status || "active", criticality: asset?.criticality || "normal",
    environment: asset?.environment || "other", owner_department: asset?.owner_department || "",
    location: asset?.location || "", attributes: pretty(asset?.attributes || {}),
    custom_fields: pretty(asset?.custom_fields || {}), reason: "",
  });
  const [error, setError] = React.useState("");
  const change = (key: string, value: string) => setForm(current => ({...current, [key]: value}));
  const save = async (event: React.FormEvent) => {
    event.preventDefault();
    try {
      const body = {...form, attributes: JSON.parse(form.attributes), custom_fields: JSON.parse(form.custom_fields)};
      await api(asset ? `/api/v1/assets/${asset.id}` : "/api/v1/assets",
        jsonRequest(csrf, body, asset ? "PATCH" : "POST"));
      onSaved();
    } catch (reason) { setError((reason as Error).message); }
  };
  return <Drawer title={asset ? "자산 수정" : "수동 자산 등록"} onClose={onClose}>
    <form className="drawer-form" onSubmit={save}>
      {!asset && <label>Asset key<input value={form.asset_key} onChange={event => change("asset_key", event.target.value)}/></label>}
      <label>이름<input value={form.name} onChange={event => change("name", event.target.value)} required/></label>
      <label>유형<input value={form.type} onChange={event => change("type", event.target.value)} disabled={!!asset} required/></label>
      <label>상태<input value={form.status} onChange={event => change("status", event.target.value)}/></label>
      <label>중요도<select value={form.criticality} onChange={event => change("criticality", event.target.value)}>
        <option>low</option><option>normal</option><option>high</option><option>critical</option></select></label>
      <label>환경<select value={form.environment} onChange={event => change("environment", event.target.value)}>
        <option>production</option><option>staging</option><option>development</option><option>other</option></select></label>
      <label>담당 부서<input value={form.owner_department} onChange={event => change("owner_department", event.target.value)}/></label>
      <label>위치<input value={form.location} onChange={event => change("location", event.target.value)}/></label>
      <label className="wide">Attributes JSON<textarea value={form.attributes} onChange={event => change("attributes", event.target.value)} spellCheck={false}/></label>
      <label className="wide">Custom fields JSON<textarea value={form.custom_fields} onChange={event => change("custom_fields", event.target.value)} spellCheck={false}/></label>
      <label className="wide">변경 사유<input value={form.reason} onChange={event => change("reason", event.target.value)} required/></label>
      {error && <div className="error wide">{error}</div>}
      <div className="form-actions wide"><button type="button" className="secondary" onClick={onClose}>취소</button>
        <button className="primary compact"><Save size={15}/>저장</button></div>
    </form>
  </Drawer>;
}

function AssetDetail({id, csrf, access, onClose, onChanged}: {
  id: string; csrf: string; access: PermissionContext; onClose: () => void; onChanged: () => void;
}) {
  const [asset, setAsset] = React.useState<Asset|null>(null);
  const [sources, setSources] = React.useState<AssetSource[]>([]);
  const [history, setHistory] = React.useState<AssetHistory[]>([]);
  const [relations, setRelations] = React.useState<AssetRelation[]>([]);
  const [editing, setEditing] = React.useState(false);
  const [targetID, setTargetID] = React.useState("");
  const [relationType, setRelationType] = React.useState("depends_on");
  const [splitSources, setSplitSources] = React.useState<string[]>([]);
  const [error, setError] = React.useState("");
  const load = React.useCallback(async () => {
    try {
      const detail = await api<{asset: Asset; sources: AssetSource[]}>(`/api/v1/assets/${id}`);
      setAsset(detail.asset); setSources(detail.sources);
      const [historyResult, relationResult] = await Promise.all([
        api<{items: AssetHistory[]}>(`/api/v1/assets/${id}/history`),
        can(access, "relations.read")
          ? api<{items: AssetRelation[]}>(`/api/v1/assets/${id}/relations`)
          : Promise.resolve({items: []}),
      ]);
      setHistory(historyResult.items); setRelations(relationResult.items); setError("");
    } catch (reason) { setError((reason as Error).message); }
  }, [access, id]);
  React.useEffect(() => { load(); }, [load]);
  const mutate = async (work: () => Promise<unknown>) => {
    try { await work(); await load(); await onChanged(); } catch (reason) { setError((reason as Error).message); }
  };
  if (editing && asset) return <AssetEditor csrf={csrf} asset={asset} onClose={() => setEditing(false)}
    onSaved={async () => {setEditing(false); await load(); await onChanged();}}/>;
  return <Drawer title={asset?.name || "자산 상세"} onClose={onClose}>
    {error && <div className="error">{error}</div>}
    {asset && <><div className="detail-actions">
      {can(access, "assets.write") && <button className="secondary" onClick={() => setEditing(true)}>수정</button>}
      {asset.deleted_at
        ? can(access, "assets.write") && <button className="secondary" onClick={() =>
          mutate(() => api(`/api/v1/assets/${id}/restore`, jsonRequest(csrf, {})))}>복원</button>
        : can(access, "assets.delete") && <button className="danger-button" onClick={() => {
          if (window.confirm("이 자산을 삭제 상태로 전환합니까?")) mutate(() =>
            api(`/api/v1/assets/${id}`, {method: "DELETE", headers: {"X-CSRF-Token": csrf}}));
        }}><Trash2 size={14}/>삭제</button>}
    </div>
    <div className="detail-grid">
      <DataRow label="Asset key" value={asset.asset_key}/><DataRow label="유형" value={asset.type}/>
      <DataRow label="상태" value={asset.status}/><DataRow label="중요도" value={asset.criticality}/>
      <DataRow label="환경" value={asset.environment}/><DataRow label="원천" value={asset.source}/>
      <DataRow label="담당 부서" value={asset.owner_department || "—"}/><DataRow label="위치" value={asset.location || "—"}/>
      <DataRow label="최초 확인" value={formatDate(asset.first_seen_at)}/><DataRow label="최근 확인" value={formatDate(asset.last_seen_at)}/>
    </div>
    <details className="json-details"><summary>Attributes / Custom fields</summary>
      <pre>{pretty({attributes: asset.attributes, custom_fields: asset.custom_fields})}</pre></details>
    <section className="drawer-section"><h3>수집 원천 <span>{sources.length}</span></h3>
      {sources.map(source => <label className="source-row" key={source.id}>
        {can(access, "assets.merge") && <input type="checkbox" checked={splitSources.includes(source.id)}
          onChange={() => setSplitSources(current => current.includes(source.id)
            ? current.filter(value => value !== source.id) : [...current, source.id])}/>}
        <div><strong>{source.source_name || source.category}</strong>
          <span>{source.category} · {formatDate(source.last_seen_at)}</span></div>
        <details><summary>payload</summary><pre>{pretty(source.payload)}</pre></details>
      </label>)}
      {can(access, "assets.merge") && splitSources.length > 0 && <button className="secondary" onClick={() => {
        const name = window.prompt("분리할 새 자산 이름");
        const type = window.prompt("분리할 새 자산 유형", asset.type);
        if (name && type) mutate(() => api(`/api/v1/assets/${id}/split`, jsonRequest(csrf, {
          source_ids: splitSources, name, type, reason: "관리 콘솔 자산 원천 분리",
        })));
      }}><Scissors size={14}/>선택 원천 분리</button>}
    </section>
    {can(access, "relations.read") && <section className="drawer-section"><h3>자산 관계 <span>{relations.length}</span></h3>
      {relations.map(relation => <div className="relation-row" key={relation.id}><Link2 size={14}/>
        <div><strong>{relation.source_asset.name} — {relation.relation_type} → {relation.target_asset.name}</strong>
          <span>신뢰도 {Math.round(relation.confidence * 100)}%</span></div>
        {can(access, "relations.write") && <button onClick={() => mutate(() =>
          api(`/api/v1/assets/${id}/relations/${relation.id}`, {method: "DELETE", headers: {"X-CSRF-Token": csrf}}))}><X size={14}/></button>}
      </div>)}
      {can(access, "relations.write") && <form className="inline-form" onSubmit={event => {
        event.preventDefault(); mutate(async () => {
          await api(`/api/v1/assets/${id}/relations`, jsonRequest(csrf, {
            target_asset_id: targetID, relation_type: relationType, confidence: 1,
            reason: "관리 콘솔 관계 생성",
          })); setTargetID("");
        });
      }}><input value={targetID} onChange={event => setTargetID(event.target.value)} placeholder="대상 Asset UUID" required/>
        <input value={relationType} onChange={event => setRelationType(event.target.value)} required/>
        <button className="primary compact"><Plus size={14}/>관계 추가</button></form>}
    </section>}
    <section className="drawer-section"><h3>변경 이력 <span>{history.length}</span></h3>
      <div className="history-list">{history.map(change => <details key={change.id}><summary>
        <Badge value={change.change_type}/><strong>{formatDate(change.occurred_at)}</strong><span>{change.reason || change.actor_type}</span>
      </summary><pre>{pretty({before: change.before, after: change.after})}</pre></details>)}</div>
    </section></>}
  </Drawer>;
}

function AssetTable({items, selected, onToggle, onSelect}: {
  items: Asset[]; selected?: string[]; onToggle?: (id: string) => void; onSelect?: (id: string) => void;
}) {
  return <div className="table-wrap"><table><thead><tr>
    {onToggle && <th aria-label="선택"/>}<th>자산</th><th>유형</th><th>환경</th><th>중요도</th><th>상태</th><th>최근 확인</th>
  </tr></thead><tbody>{items.map(asset => <tr key={asset.id} className={onSelect ? "clickable" : ""}>
    {onToggle && <td><input type="checkbox" checked={selected?.includes(asset.id)}
      onChange={() => onToggle(asset.id)} onClick={event => event.stopPropagation()}/></td>}
    <td onClick={() => onSelect?.(asset.id)}><strong>{asset.name}</strong><span>{asset.asset_key || asset.id}</span></td>
    <td onClick={() => onSelect?.(asset.id)}>{asset.type}</td><td onClick={() => onSelect?.(asset.id)}>{asset.environment}</td>
    <td onClick={() => onSelect?.(asset.id)}>{asset.criticality}</td><td onClick={() => onSelect?.(asset.id)}><Badge value={asset.status}/></td>
    <td onClick={() => onSelect?.(asset.id)}>{formatDate(asset.last_seen_at)}</td></tr>)}</tbody></table>
    {!items.length && <Empty icon={Boxes} text="표시할 자산이 없습니다."/>}
  </div>;
}

function Breakdown({items}: {items: Bucket[]}) {
  const total = items.reduce((sum, item) => sum + item.count, 0);
  return <div className="breakdown">{items.slice(0, 7).map((item, index) =>
    <div key={item.label}><span className={`chart-color c${index % 6}`}/><strong>{item.label}</strong>
      <div><i style={{width: `${total ? item.count / total * 100 : 0}%`}}/></div>
      <b>{number(item.count)}</b><small>{total ? Math.round(item.count / total * 100) : 0}%</small></div>)}
    {!items.length && <Empty icon={Boxes} text="집계 데이터가 없습니다."/>}</div>;
}
function DailyBars({items}: {items: Statistics["collection"]["daily"]}) {
  const max = Math.max(1, ...items.map(item => item.events));
  return <div className="daily-bars">{items.map(item => <div key={item.date}>
    <div className="bar-stack" title={`${item.events} events / ${item.failed} failed`}>
      <i style={{height: `${Math.max(3, item.events / max * 100)}%`}}/>
      {item.failed > 0 && <b style={{height: `${Math.max(3, item.failed / max * 100)}%`}}/>}
    </div><strong>{item.events}</strong><span>{item.date.slice(5)}</span></div>)}</div>;
}
function RiskSummary({statistics}: {statistics: Statistics|null}) {
  const critical = statistics?.assets.by_criticality.find(item => item.label === "critical")?.count || 0;
  const failed = statistics?.collection.failed_24h || 0;
  const attention = statistics?.agents.attention || 0;
  const stale = statistics?.assets.stale || 0;
  const rows = [
    {label: "중요 자산", value: critical, note: "critical 등급", tone: critical ? "warn" : "ok"},
    {label: "수집 실패", value: failed, note: "최근 24시간", tone: failed ? "bad" : "ok"},
    {label: "Agent 점검", value: attention, note: "30분 이상 미연결", tone: attention ? "warn" : "ok"},
    {label: "최신성 점검", value: stale, note: "24시간 이상 미확인", tone: stale ? "warn" : "ok"},
  ];
  return <div className="risk-list">{rows.map(row => <div key={row.label} className={row.tone}>
    <AlertTriangle size={16}/><span><strong>{row.label}</strong><small>{row.note}</small></span><b>{number(row.value)}</b></div>)}</div>;
}
function Metric({label, value, note, icon: Icon}: {label: string; value: string; note: string; icon: React.ElementType}) {
  return <article className="metric"><div><span>{label}</span><strong>{value}</strong><small>{note}</small></div><Icon/></article>;
}
function Panel({title, action, children}: {title: string; action: string; children: React.ReactNode}) {
  return <article className="panel"><div className="panel-head"><h3>{title}</h3><span>{action}</span></div>{children}</article>;
}
function PageTitle({kicker, title, subtitle, action}: {
  kicker: string; title: string; subtitle: string; action?: React.ReactNode;
}) {
  return <div className="page-title with-action"><div><p className="eyebrow dark">{kicker}</p><h1>{title}</h1><p>{subtitle}</p></div>{action && <div>{action}</div>}</div>;
}
function Drawer({title, onClose, children}: {title: string; onClose: () => void; children: React.ReactNode}) {
  return <div className="drawer-backdrop" role="dialog" aria-modal="true"><aside className="drawer">
    <header><h2>{title}</h2><button onClick={onClose} aria-label="닫기"><X/></button></header>
    <div className="drawer-content">{children}</div></aside></div>;
}
function Badge({value}: {value: string}) {
  return <span className={`badge ${["active", "success", "processed", "정상", "info"].includes(value) ? "good" :
    ["blocked", "failed", "deleted", "warning", "error"].includes(value) ? "bad" : ""}`}>{value}</span>;
}
function Empty({icon: Icon, text}: {icon: React.ElementType; text: string}) {
  return <div className="empty"><Icon/><p>{text}</p></div>;
}
function Notice({tone, title, children}: {tone: "info"|"warning"; title: string; children: React.ReactNode}) {
  return <div className={`admin-notice ${tone}`}><strong>{title}</strong><span>{children}</span></div>;
}
function SecretReveal({secret, onClose}: {secret: string; onClose: () => void}) {
  return <div className="secret-reveal"><div><strong>새 Secret — 지금 한 번만 표시됩니다</strong><code>{secret}</code></div>
    <button className="secondary" onClick={() => navigator.clipboard.writeText(secret)}><Copy size={16}/>복사</button>
    <button className="secondary" onClick={onClose}><X size={16}/></button></div>;
}
function DataRow({label, value}: {label: string; value: string}) {
  return <div className="data-row"><span>{label}</span><strong>{value}</strong></div>;
}
type AssetSource = {id: string; category: string; source_name: string; payload: unknown; last_seen_at: string};
type AssetHistory = {id: string; change_type: string; before: unknown; after: unknown; actor_type: string; reason: string; occurred_at: string};
type AssetRelation = {id: string; relation_type: string; confidence: number; source_asset: {name: string}; target_asset: {name: string}};
type AuditEvent = {
  id: string; occurred_at: string; actor_type: string; actor_name: string; action: string;
  resource_type: string; resource_id?: string; request_id: string; source_ip: string;
  result: string; reason: string; before: unknown; after: unknown; metadata: unknown;
};

type DiagnosticLog = {
  id: string;
  occurred_at: string;
  level: "info" | "warning" | "error";
  component: string;
  event_code: string;
  message: string;
  request_id: string;
  instance_id: string;
  agent_id: string;
  source_ip: string;
  details: Record<string, unknown>;
};
const pretty = (value: unknown) => JSON.stringify(value ?? {}, null, 2);
const formatDate = (value?: string|null) => value
  ? new Intl.DateTimeFormat("ko-KR", {year: "2-digit", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit"}).format(new Date(value))
  : "—";
const number = (value?: number) => (value || 0).toLocaleString("ko-KR");
const recentEnough = (value: string|undefined, minutes: number) =>
  !!value && Date.now() - new Date(value).getTime() <= minutes * 60_000;
