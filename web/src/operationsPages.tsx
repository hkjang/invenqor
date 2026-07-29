import React from "react";
import {
  Activity,
  AlertTriangle,
  Boxes,
  CheckCircle2,
  Copy,
  Download,
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
import {
  downloadPath,
  formatDate,
  formatRelative,
  formatSecond,
  number,
  percentOf,
  withinMinutes,
} from "./format";
import {consoleHashQuery} from "./navigationState";
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
type EnrollmentDiagnostics = {
  window_hours: number;
  instance_id: string;
  totals: {
    events: number;
    succeeded: number;
    rejected: number;
    preflight_checks: number;
    preflight_blocked: number;
    transport_failed: number;
  };
  by_event_code: {
    event_code: string;
    level: string;
    count: number;
    message: string;
    remediation: string;
    last_occurred_at?: string;
    last_request_id?: string;
  }[];
  sources: {
    source_ip: string;
    agent_id: string;
    attempts: number;
    failures: number;
    last_event_code: string;
    last_level: string;
    last_message: string;
    last_request_id?: string;
    last_instance_id?: string;
    last_occurred_at?: string;
    remediation: string;
    agent_version?: string;
  }[];
  awaiting_inventory: {
    id: string;
    agent_id: string;
    hostname: string;
    status: string;
    auth_method: string;
    created_at?: string;
    last_seen_at?: string;
  }[];
  enrollment?: {mode: string; network_mode: string; allowed_networks: string[]};
};
type Bucket = {label: string; count: number};
type Release = {
  base: string;
  version: string;
  channel: string;
  architecture: string;
  rollout_percent: number;
  allow_downgrade?: boolean;
  notes?: string;
  signature_verified: boolean;
  published_at?: string;
  published_by?: string;
  adopted_agents: number;
  eligible_agents: number;
};
type ReleaseListing = {
  releases: Release[];
  agent_versions: Bucket[];
  agents: number;
  signature_verified: boolean;
};
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
  // Reporting 100% for an empty inventory reads as a healthy system rather than
  // an empty one, which is the opposite of what an operator needs to know.
  const freshRate = percentOf(statistics?.assets.seen_24h, statistics?.assets.total);
  const healthyRate = percentOf(statistics?.agents.healthy, statistics?.agents.total);
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
      <Metric label="자산 최신성" value={freshRate}
        note={`점검 필요 ${number(statistics?.assets.stale)}`} icon={CheckCircle2}/>
      <Metric label="정상 Agent" value={`${number(statistics?.agents.healthy)} / ${number(statistics?.agents.total)}`}
        note={`건전성 ${healthyRate}`} icon={Activity}/>
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
          <div key={agent.id}><i className={withinMinutes(agent.last_seen_at, 30) ? "ok" : ""}/>
            <div><strong>{agent.hostname || agent.agent_id}</strong>
              <span>{agent.os_name || "운영체제 확인 전"} · {formatRelative(agent.last_seen_at)}</span></div>
            <Badge value={agent.status}/></div>)}
          {!agents.length && <Empty icon={Activity} text="등록된 Agent가 없습니다."/>}
        </div>
      </Panel>
    </div>
  </section>;
}

const assetPageSize = 50;

export function AssetsPage({csrf, access}: {csrf: string; access: PermissionContext}) {
  const [items, setItems] = React.useState<Asset[]>([]);
  const [total, setTotal] = React.useState(0);
  const [query, setQuery] = React.useState("");
  const [type, setType] = React.useState("");
  const [status, setStatus] = React.useState("");
  const [environment, setEnvironment] = React.useState("");
  const [criticality, setCriticality] = React.useState("");
  const [sort, setSort] = React.useState("recent");
  const [includeDeleted, setIncludeDeleted] = React.useState(false);
  const [offset, setOffset] = React.useState(0);
  const [hasMore, setHasMore] = React.useState(false);
  const [selected, setSelected] = React.useState<string[]>([]);
  const [detailID, setDetailID] = React.useState<string|null>(null);
  const [creating, setCreating] = React.useState(false);
  const [error, setError] = React.useState("");
  // Held in one place so the CSV export downloads exactly the rows on screen.
  const parameters = React.useMemo(() => {
    const values = new URLSearchParams();
    if (query) values.set("q", query);
    if (type) values.set("type", type);
    if (status) values.set("status", status);
    if (environment) values.set("environment", environment);
    if (criticality) values.set("criticality", criticality);
    if (sort) values.set("sort", sort);
    if (includeDeleted) values.set("include_deleted", "true");
    return values;
  }, [criticality, environment, includeDeleted, query, sort, status, type]);
  const load = React.useCallback(async () => {
    const values = new URLSearchParams(parameters);
    values.set("limit", String(assetPageSize));
    values.set("offset", String(offset));
    try {
      const result = await api<{items: Asset[]; has_more: boolean; total: number}>(
        `/api/v1/assets?${values}`);
      setItems(result.items); setHasMore(result.has_more);
      setTotal(result.total); setError("");
    } catch (reason) { setError((reason as Error).message); }
  }, [offset, parameters]);
  React.useEffect(() => {
    const timer = window.setTimeout(load, 180);
    return () => window.clearTimeout(timer);
  }, [load]);
  // Any filter change makes the current page meaningless, so the page resets.
  const changeFilter = <T,>(apply: (value: T) => void) => (value: T) => {
    apply(value); setOffset(0);
  };
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
  const first = total ? offset + 1 : 0;
  const last = offset + items.length;
  return <section>
    <PageTitle kicker="CONFIGURATION ITEMS" title="자산 인벤토리"
      subtitle="검색부터 생성·수정·계보·관계·병합·분할까지 자산 수명주기를 관리합니다."
      action={<>{can(access, "assets.merge") && <button className="secondary" disabled={selected.length < 2} onClick={merge}><GitMerge size={15}/>선택 병합</button>}
        <button className="secondary" onClick={() => downloadPath(`/api/v1/assets.csv?${parameters}`)}>
          <Download size={15}/>CSV 내려받기</button>
        {can(access, "assets.write") && <button className="primary compact" onClick={() => setCreating(true)}><Plus size={15}/>자산 등록</button>}</>}/>
    <div className="filter-bar asset-filters">
      <div className="search"><Search size={18}/><input placeholder="이름 또는 자산 키" value={query}
        onChange={event => changeFilter(setQuery)(event.target.value)}/></div>
      <input placeholder="유형" value={type}
        onChange={event => changeFilter(setType)(event.target.value)}/>
      {/* Environment and criticality are what classification sets, so the
          inventory has to be filterable by them. */}
      <select value={environment}
        onChange={event => changeFilter(setEnvironment)(event.target.value)}>
        <option value="">전체 환경</option><option value="production">production</option>
        <option value="staging">staging</option><option value="development">development</option>
        <option value="other">other</option>
      </select>
      <select value={criticality}
        onChange={event => changeFilter(setCriticality)(event.target.value)}>
        <option value="">전체 중요도</option><option value="critical">critical</option>
        <option value="high">high</option><option value="normal">normal</option>
        <option value="low">low</option>
      </select>
      <select value={status} onChange={event => changeFilter(setStatus)(event.target.value)}>
        <option value="">전체 상태</option><option value="active">active</option>
        <option value="discovered">discovered</option><option value="deleted">deleted</option>
      </select>
      <select value={sort} onChange={event => changeFilter(setSort)(event.target.value)}>
        <option value="recent">최근 확인 순</option><option value="oldest">오래된 확인 순</option>
        <option value="discovered">최근 발견 순</option><option value="name">이름 순</option>
        <option value="type">유형 순</option><option value="criticality">중요도 순</option>
      </select>
      <label><input type="checkbox" checked={includeDeleted}
        onChange={event => changeFilter(setIncludeDeleted)(event.target.checked)}/>삭제 포함</label>
      <button className="secondary" onClick={load}><RefreshCw size={15}/></button>
    </div>
    {error && <div className="error action-message">{error}</div>}
    {/* "50개 자산 · offset 0" said nothing about the size of the result. */}
    <Panel title={`자산 ${number(total)}건`}
      action={total ? `${number(first)}–${number(last)} 표시` : "결과 없음"}>
      <AssetTable items={items} selected={selected} onToggle={id =>
        setSelected(current => current.includes(id) ? current.filter(value => value !== id) : [...current, id])}
        onSelect={setDetailID}/>
      <div className="pagination">
        <button className="secondary" disabled={!offset}
          onClick={() => setOffset(Math.max(0, offset - assetPageSize))}>이전</button>
        <button className="secondary" disabled={!hasMore}
          onClick={() => setOffset(offset + assetPageSize)}>다음</button>
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
  const [signatureFile, setSignatureFile] = React.useState<File|null>(null);
  const [notes, setNotes] = React.useState("");
  const [allowDowngrade, setAllowDowngrade] = React.useState(false);
  const [rollout, setRollout] = React.useState(10);
  const [releases, setReleases] = React.useState<Release[]>([]);
  const [releaseInfo, setReleaseInfo] = React.useState<ReleaseListing|null>(null);
  const load = React.useCallback(() =>
    api<{agents: Agent[]}>("/api/v1/admin/agents").then(value => setItems(value.agents)),
  []);
  const loadReleases = React.useCallback(() =>
    api<ReleaseListing>("/api/v1/admin/agent-updates").then(value => {
      setReleaseInfo(value);
      setReleases(value.releases);
    }).catch(() => {}),
  []);
  React.useEffect(() => {
    load().catch(reason => setError((reason as Error).message));
    loadReleases();
    const timer = window.setInterval(() => {
      load().catch(() => {});
      loadReleases();
    }, 15000);
    return () => window.clearInterval(timer);
  }, [load, loadReleases]);
  const setReleaseRollout = (release: Release, percent: number) => mutate(async () => {
    await api(`/api/v1/admin/agent-updates/${release.base}`, jsonRequest(csrf, {
      rollout_percent: percent,
      reason: percent === 0 ? "관리 콘솔 배포 중단" : `관리 콘솔 rollout ${percent}%`,
    }, "PATCH"));
    await loadReleases();
  });
  const retireRelease = (release: Release) => {
    if (!window.confirm(`${release.version} (${release.architecture}) 릴리즈를 삭제합니까?`)) return;
    return mutate(async () => {
      await api(`/api/v1/admin/agent-updates/${release.base}`, {
        method: "DELETE", headers: {"X-CSRF-Token": csrf},
      });
      await loadReleases();
    });
  };
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
    body.set("artifact", file);
    body.set("version", version);
    body.set("channel", channel);
    body.set("architecture", architecture);
    body.set("rollout_percent", String(rollout));
    if (notes) body.set("notes", notes);
    if (allowDowngrade) body.set("allow_downgrade", "true");
    // A signature file is what the signing step actually produces, so accept it
    // directly instead of making the operator base64 it by hand.
    if (signatureFile) body.set("signature_file", signatureFile);
    else body.set("signature", signature);
    try {
      await api("/api/v1/admin/agent-updates", {
        method: "POST", headers: {"X-CSRF-Token": csrf}, body,
      });
      setFile(null); setVersion(""); setSignature(""); setSignatureFile(null);
      setNotes(""); setAllowDowngrade(false);
      await loadReleases();
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
    <EnrollmentDiagnosticsPanel/>
    {can(access, "agents.manage") && <div className="agent-admin-grid">
      <Panel title="예외 장비 수동 등록" action="자동 등록 권장">
        <form className="compact-form" onSubmit={provision}>
          <label>Agent UUID<input value={agentID} onChange={event => setAgentID(event.target.value)} required/></label>
          <label>Hostname<input value={hostname} onChange={event => setHostname(event.target.value)}/></label>
          <button className="primary compact"><KeyRound size={15}/>장비 토큰 발급</button>
        </form>
      </Panel>
      <Panel title="서명된 Agent 업데이트 게시" action="최대 128 MiB">
        <form className="compact-form update-form" onSubmit={publish}>
          <label>Artifact<input type="file" onChange={event => setFile(event.target.files?.[0] || null)} required/></label>
          <label>버전<input value={version} onChange={event => setVersion(event.target.value)} placeholder="0.2.7" required/></label>
          <label>채널<select value={channel} onChange={event => setChannel(event.target.value)}><option>stable</option><option>beta</option></select></label>
          <label>아키텍처<select value={architecture} onChange={event => setArchitecture(event.target.value)}><option>x86_64</option><option>aarch64</option></select></label>
          <label>서명 파일 <small className="optional">.sig 업로드</small>
            <input type="file" onChange={event => setSignatureFile(event.target.files?.[0] || null)}/></label>
          <label>최초 Rollout %<input type="number" min="0" max="100" value={rollout}
            onChange={event => setRollout(Number(event.target.value))}/></label>
          {!signatureFile && <label className="wide">Ed25519 Signature (Base64)
            <textarea value={signature} onChange={event => setSignature(event.target.value)}
              placeholder="서명 파일을 올리면 비워 두어도 됩니다" required/></label>}
          <label className="wide">릴리즈 메모<input value={notes}
            onChange={event => setNotes(event.target.value)} placeholder="무엇이 바뀌었는지"/></label>
          <label className="wide inline-check">
            <input type="checkbox" checked={allowDowngrade}
              onChange={event => setAllowDowngrade(event.target.checked)}/>
            이전 버전으로 되돌리는 롤백 릴리즈
          </label>
          <button className="primary compact"><Save size={15}/>게시</button>
        </form>
        <p className="hint update-hint">
          {releaseInfo?.signature_verified
            ? "Server에 서명 공개키가 설정되어 있어 게시 시점에 서명을 검증합니다. 잘못된 서명은 여기서 거부됩니다."
            : "Server에 서명 공개키가 없어 게시 시점 검증을 할 수 없습니다. INVENQOR_UPDATE_PUBLIC_KEY를 설정하면 잘못된 서명을 fleet 전체가 실패하기 전에 잡아냅니다."}
          {" 처음에는 10% 정도로 게시하고 아래에서 단계적으로 넓히십시오."}
        </p>
      </Panel>
    </div>}
    {!!releases.length && <Panel
      title={`게시된 릴리즈 ${releases.length}건`}
      action={`Agent ${(releaseInfo?.agents ?? 0).toLocaleString("ko-KR")}대`}
    >
      <div className="release-list">
        {releases.map(release => {
          const share = releaseInfo?.agents
            ? Math.round(release.adopted_agents * 100 / releaseInfo.agents)
            : 0;
          return <div key={release.base} className={release.rollout_percent === 0 ? "release halted" : "release"}>
            <div className="release-head">
              <strong>{release.version}<span>{release.architecture} · {release.channel}</span></strong>
              <div className="release-flags">
                {release.signature_verified
                  ? <em className="ok" title="게시 시점에 서명을 검증했습니다.">서명 검증됨</em>
                  : <em title="Server에 공개키가 없어 검증하지 못했습니다.">서명 미검증</em>}
                {release.allow_downgrade && <em className="warn">롤백</em>}
                {release.rollout_percent === 0 && <em className="warn">중단</em>}
              </div>
              {can(access, "agents.manage") && <div className="release-actions">
                {[10, 25, 50, 100].map(percent => <button
                  key={percent}
                  className={release.rollout_percent === percent ? "selected" : ""}
                  onClick={() => setReleaseRollout(release, percent)}
                >{percent}%</button>)}
                <button className="danger" onClick={() => setReleaseRollout(release, 0)}>중단</button>
                <button className="danger" onClick={() => retireRelease(release)}>
                  <Trash2 size={13}/>
                </button>
              </div>}
            </div>
            <div className="release-progress">
              <div className="meter" role="img"
                aria-label={`적용 ${release.adopted_agents}대, rollout ${release.rollout_percent}%`}>
                <i style={{width: `${Math.min(100, share)}%`}}/>
                <b style={{width: `${Math.min(100, release.rollout_percent)}%`}}/>
              </div>
              <span>
                적용 {release.adopted_agents.toLocaleString("ko-KR")}대 · 대상
                {" "}{release.rollout_percent}% ({release.eligible_agents.toLocaleString("ko-KR")}대)
                {release.notes ? ` · ${release.notes}` : ""}
              </span>
            </div>
          </div>;
        })}
      </div>
      {!!releaseInfo?.agent_versions.length && <div className="breakdown version-breakdown">
        {releaseInfo.agent_versions.slice(0, 6).map((bucket, index) =>
          <div key={bucket.label}>
            <span className={`chart-color c${index % 8}`}/>
            <strong>{bucket.label}</strong><b>{bucket.count.toLocaleString("ko-KR")}</b>
          </div>)}
      </div>}
      <p className="hint">
        진한 막대는 이미 그 버전을 보고한 Agent 비율, 옅은 막대는 현재 rollout 대상
        비율입니다. 문제가 보이면 <strong>중단</strong>을 누르십시오. 즉시 아무 Agent도
        해당 릴리즈를 제안받지 않으며, 이미 적용된 Agent는 롤백 릴리즈를 게시해 되돌립니다.
      </p>
    </Panel>}
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

// An Agent that never registers cannot appear in the Agent list, so the only
// way to see it is the record of its attempts. This panel puts that record, and
// the matching remedy, next to the fleet it is missing from.
function EnrollmentDiagnosticsPanel() {
  const [data, setData] = React.useState<EnrollmentDiagnostics|null>(null);
  const [hours, setHours] = React.useState(24);
  const [error, setError] = React.useState("");
  const load = React.useCallback(async () => {
    try {
      setData(await api<EnrollmentDiagnostics>(
        `/api/v1/admin/diagnostics/enrollment?hours=${hours}`));
      setError("");
    } catch (reason) {
      setError((reason as Error).message);
    }
  }, [hours]);
  React.useEffect(() => { load(); }, [load]);
  React.useEffect(() => {
    const timer = window.setInterval(() => { load(); }, 30_000);
    return () => window.clearInterval(timer);
  }, [load]);
  const failing = (data?.sources || []).filter(source => source.failures > 0);
  const blocked = (data?.totals.rejected || 0) + (data?.totals.preflight_blocked || 0);
  const origin = typeof window === "undefined" ? "" : window.location.origin;
  return <Panel title="등록 진단" action={`최근 ${data?.window_hours ?? hours}시간`}>
    {error && <div className="error">{error}</div>}
    <div className="status-grid enrollment-diagnostics-summary">
      <div><span>등록 성공</span><strong>{number(data?.totals.succeeded)}</strong></div>
      <div><span>등록 거부</span><strong>{number(data?.totals.rejected)}</strong></div>
      <div><span>사전 점검 차단</span><strong>{number(data?.totals.preflight_blocked)}</strong></div>
      <div><span>전송 실패</span><strong>{number(data?.totals.transport_failed)}</strong></div>
      <div><span>첫 수집 대기</span><strong>{number(data?.awaiting_inventory.length)}</strong></div>
    </div>
    <div className="filter-bar">
      <select value={hours} onChange={event => setHours(Number(event.target.value))}>
        <option value="1">최근 1시간</option><option value="24">최근 24시간</option>
        <option value="168">최근 7일</option><option value="720">최근 30일</option>
      </select>
      <button className="secondary" onClick={load}><RefreshCw size={15}/>새로고침</button>
    </div>
    {blocked === 0 && !failing.length
      ? <Notice tone="info" title="차단된 등록 시도가 없습니다.">
          Agent가 보이지 않는다면 해당 장비에서 <code>invenqor-agent --diagnose</code>를
          실행하십시오. Server에 도달조차 못한 경우 Agent 측 로그와
          <code>/var/lib/invenqor-agent/status.json</code>에 원인이 기록됩니다.
        </Notice>
      : <div className="audit-table enrollment-source-table">
          {failing.map(source => <details key={`${source.source_ip}|${source.agent_id}`}>
            <summary>
              <i className="bad"/>
              <time>{formatDate(source.last_occurred_at)}</time>
              <strong>{source.source_ip || "출처 불명"}</strong>
              <span>{source.last_event_code} · 시도 {source.attempts} · 실패 {source.failures}</span>
              <Badge value={source.last_level}/>
            </summary>
            <div className="audit-detail">
              <DataRow label="Agent ID" value={source.agent_id || "등록 전"}/>
              <DataRow label="Agent 버전" value={source.agent_version || "—"}/>
              <DataRow label="Server 메시지" value={source.last_message}/>
              <DataRow label="Request ID" value={source.last_request_id || "—"}/>
              <DataRow label="처리 Pod" value={source.last_instance_id || "—"}/>
              <DataRow label="조치" value={source.remediation}/>
            </div>
          </details>)}
        </div>}
    {!!data?.by_event_code.length && <div className="breakdown enrollment-code-breakdown">
      {data.by_event_code.slice(0, 8).map((code, index) =>
        <div key={code.event_code} title={code.remediation}>
          <span className={`chart-color c${index % 8}`}/>
          <strong>{code.event_code}</strong><b>{number(code.count)}</b>
        </div>)}
    </div>}
    {!!data?.awaiting_inventory.length && <div className="agent-list awaiting-inventory">
      {data.awaiting_inventory.slice(0, 8).map(agent =>
        <div key={agent.id}><i/>
          <div><strong>{agent.hostname || agent.agent_id}</strong>
            <span>등록 {formatDate(agent.created_at)} · 아직 수집 이벤트가 없습니다</span></div>
          <Badge value={agent.status}/></div>)}
    </div>}
    <p className="hint enrollment-diagnostics-hint">
      Agent 장비에서 <code>invenqor-agent --diagnose</code>, 임의 장비에서{" "}
      <code>curl -s {origin}/v1/agent/preflight</code>를 실행하면 등록 가능 여부와
      Server가 인식한 출처 IP를 즉시 확인할 수 있습니다.
    </p>
  </Panel>;
}

// Recipes for the questions this inventory is actually asked, so the page starts
// from a working query rather than a blank box.
const queryExamples: {label: string; query: string}[] = [
  {label: "운영 환경의 치명 자산", query: 'environment = "production" AND criticality = "critical"'},
  {label: "24시간 이상 미확인", query: 'last_seen_at < "now - 24h"'},
  {label: "최근 7일 신규 발견", query: 'first_seen_at >= "now - 168h"'},
  {label: "분류 확신도가 낮은 자산", query: "confidence < 0.6"},
  {label: "담당 부서가 없는 운영 자산", query: 'environment = "production" AND owner_department = ""'},
  {label: "특정 운영체제", query: 'attributes.os_name = "Ubuntu"'},
];

export function QueryPage({csrf}: {csrf: string}) {
  const [query, setQuery] = React.useState('type = "host" AND environment = "production"');
  const [limit, setLimit] = React.useState(100);
  const [result, setResult] = React.useState<Asset[]>([]);
  const [ran, setRan] = React.useState(false);
  const [grammar, setGrammar] = React.useState<QueryGrammar|null>(null);
  const [validation, setValidation] = React.useState<{valid: boolean; error?: string; ast?: unknown}|null>(null);
  const [error, setError] = React.useState("");
  // The grammar comes from the Server, so the reference cannot drift from the
  // parser that rejects a query.
  React.useEffect(() => {
    api<QueryGrammar>("/api/v1/query/schema").then(setGrammar).catch(() => {});
  }, []);
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
      setResult(value.items); setValidation({valid: true, ast: value.ast});
      setRan(true); setError("");
    } catch (reason) { setError((reason as Error).message); setRan(true); }
  };
  const insert = (text: string) => {
    setQuery(current => current.trim() ? `${current.trim()} AND ${text}` : text);
    setValidation(null);
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
    <div className="query-aids">
      <Panel title="자주 쓰는 질의" action="눌러서 편집기에 넣기">
        <div className="query-examples">{queryExamples.map(example =>
          <button key={example.label} onClick={() => {setQuery(example.query); setValidation(null);}}>
            <strong>{example.label}</strong><code>{example.query}</code></button>)}</div>
      </Panel>
      <Panel title="사용 가능한 필드"
        action={grammar ? `${grammar.combinator} 결합 · 최대 ${grammar.max_clauses}절` : "Server 기준"}>
        <div className="query-fields">{(grammar?.fields || []).map(field =>
          <button key={field.name} onClick={() => insert(field.example)} title={field.example}>
            <code>{field.name}</code><span>{field.description}</span>
            <em>{field.kind}</em></button>)}</div>
        <p className="hint">연산자 {(grammar?.operators || []).join(" ")} · 시각 필드는
          {" "}{grammar?.relative_now || '"now - 24h"'} 형태의 상대 시간을 받습니다.
          조건은 {grammar?.combinator || "AND"}로만 결합됩니다.</p>
      </Panel>
    </div>
    {validation?.ast != null && <details className="json-details"><summary>파싱된 AST</summary><pre>{pretty(validation.ast)}</pre></details>}
    <Panel title={`결과 ${number(result.length)}건`}
      action={result.length === limit ? `limit ${limit}에서 잘렸을 수 있음` : `limit ${limit}`}>
      <AssetTable items={result}/>
      {ran && !result.length && !error && <p className="hint">
        구문은 유효하지만 조건에 맞는 자산이 없습니다.</p>}
    </Panel>
  </section>;
}

const auditPageSize = 100;

export function AuditPage() {
  const [items, setItems] = React.useState<AuditEvent[]>([]);
  const [total, setTotal] = React.useState(0);
  const [facets, setFacets] = React.useState<AuditFacets|null>(null);
  const [query, setQuery] = React.useState("");
  const [action, setAction] = React.useState("");
  const [resourceType, setResourceType] = React.useState("");
  const [result, setResult] = React.useState("");
  const [from, setFrom] = React.useState("");
  const [to, setTo] = React.useState("");
  const [offset, setOffset] = React.useState(0);
  const [hasMore, setHasMore] = React.useState(false);
  const [error, setError] = React.useState("");
  const parameters = React.useMemo(() => {
    const values = new URLSearchParams();
    if (query.trim()) values.set("q", query.trim());
    if (action) values.set("action", action);
    if (resourceType) values.set("resource_type", resourceType);
    if (result) values.set("result", result);
    if (from) values.set("from", from);
    if (to) values.set("to", to);
    return values;
  }, [action, from, query, resourceType, result, to]);
  const load = React.useCallback(async () => {
    const values = new URLSearchParams(parameters);
    values.set("limit", String(auditPageSize));
    values.set("offset", String(offset));
    try {
      const value = await api<{
        items: AuditEvent[]; total: number; has_more: boolean; facets: AuditFacets;
      }>(`/api/v1/admin/audit?${values}`);
      setItems(value.items); setTotal(value.total);
      setHasMore(value.has_more); setFacets(value.facets); setError("");
    } catch (reason) { setError((reason as Error).message); }
  }, [offset, parameters]);
  // Debounced so typing a request ID does not fire a query per keystroke.
  React.useEffect(() => {
    const timer = window.setTimeout(load, 200);
    return () => window.clearTimeout(timer);
  }, [load]);
  const changeFilter = <T,>(apply: (value: T) => void) => (value: T) => {
    apply(value); setOffset(0);
  };
  const clear = () => {
    setQuery(""); setAction(""); setResourceType(""); setResult("");
    setFrom(""); setTo(""); setOffset(0);
  };
  const filtered = [...parameters.keys()].length > 0;
  const first = total ? offset + 1 : 0;
  return <section>
    <PageTitle kicker="ACCOUNTABILITY" title="감사 로그"
      subtitle="행위자·대상·요청·변경 전후 값을 추적해 운영 책임성과 조사 가능성을 확보합니다."
      action={<>
        <button className="secondary"
          onClick={() => downloadPath(`/api/v1/admin/audit.csv?${parameters}`)}>
          <Download size={15}/>CSV 내려받기</button>
        <button className="secondary" onClick={load}><RefreshCw size={15}/>새로고침</button>
      </>}/>
    {/* Every one of these used to filter only the rows already downloaded, so a
        search for an older event found nothing and read as "no such record". */}
    <div className="filter-bar audit-filters">
      <div className="search"><Search size={16}/><input value={query}
        onChange={event => changeFilter(setQuery)(event.target.value)}
        placeholder="행위, 사용자, 자원, request ID, IP, 사유 검색"/></div>
      <select value={action} onChange={event => changeFilter(setAction)(event.target.value)}>
        <option value="">모든 행위</option>
        {(facets?.actions || []).map(bucket =>
          <option key={bucket.label} value={bucket.label}>
            {bucket.label} ({bucket.count})</option>)}
      </select>
      <select value={resourceType}
        onChange={event => changeFilter(setResourceType)(event.target.value)}>
        <option value="">모든 자원</option>
        {(facets?.resource_types || []).map(bucket =>
          <option key={bucket.label} value={bucket.label}>
            {bucket.label} ({bucket.count})</option>)}
      </select>
      <select value={result} onChange={event => changeFilter(setResult)(event.target.value)}>
        <option value="">모든 결과</option>
        {(facets?.results || []).map(bucket =>
          <option key={bucket.label} value={bucket.label}>
            {bucket.label} ({bucket.count})</option>)}
      </select>
      <label>부터<input type="date" value={from}
        onChange={event => changeFilter(setFrom)(event.target.value)}/></label>
      <label>까지<input type="date" value={to}
        onChange={event => changeFilter(setTo)(event.target.value)}/></label>
      {filtered && <button className="secondary" onClick={clear}><X size={15}/>조건 해제</button>}
    </div>
    {error && <div className="error">{error}</div>}
    <Panel title={`이벤트 ${number(total)}건`}
      action={total ? `${number(first)}–${number(offset + items.length)} 표시` : "결과 없음"}>
      <div className="audit-table">{items.map(item => <details key={item.id}>
        <summary><i className={item.result === "success" ? "ok" : "bad"}/>
          <time>{formatSecond(item.occurred_at)}</time><strong>{item.action}</strong>
          <span>{item.actor_name || item.actor_type} → {item.resource_type}</span><Badge value={item.result}/></summary>
        <div className="audit-detail">
          <DataRow label="Resource" value={`${item.resource_type} / ${item.resource_id || "—"}`}/>
          <DataRow label="Request ID" value={item.request_id || "—"}/>
          <DataRow label="Source IP" value={item.source_ip || "—"}/>
          <DataRow label="Reason" value={item.reason || "—"}/>
          {/* The same request ID identifies the Server-side diagnostic entries
              for this action, which is the next thing an investigator wants. */}
          {item.request_id && <div className="audit-cross-link">
            <a href={`#/logs?request_id=${encodeURIComponent(item.request_id)}`}>
              이 요청의 Server 진단 로그 보기</a></div>}
          <pre>{pretty({before: item.before, after: item.after, metadata: item.metadata})}</pre>
        </div>
      </details>)}
        {!items.length && <Empty icon={FileSearch}
          text={filtered
            ? "조건에 맞는 감사 이벤트가 없습니다. 기간과 행위 조건을 확인하십시오."
            : "감사 이벤트가 없습니다."}/>}</div>
      <div className="pagination">
        <button className="secondary" disabled={!offset}
          onClick={() => setOffset(Math.max(0, offset - auditPageSize))}>이전</button>
        <button className="secondary" disabled={!hasMore}
          onClick={() => setOffset(offset + auditPageSize)}>다음</button>
      </div>
    </Panel>
  </section>;
}

// A component name written into the console drifts the moment the server starts
// recording a new one: agent_preflight and keycloak were both being recorded and
// neither could be selected. Known names get a label; anything else still
// appears, under its own name.
const diagnosticComponentLabels: Record<string, string> = {
  agent_enrollment: "Agent 등록",
  agent_transport: "Agent 전송",
  agent_preflight: "Agent 사전 점검",
  keycloak: "Keycloak 로그인",
  http: "Server HTTP",
  server: "Server 일반",
};

export function ServerLogsPage() {
  const [items, setItems] = React.useState<DiagnosticLog[]>([]);
  const [total, setTotal] = React.useState(0);
  const [facets, setFacets] = React.useState<DiagnosticFacets>({
    instances: [], components: [], event_codes: [],
  });
  const [retention, setRetention] = React.useState({days: 30, maximum_events: 10000});
  // Opened from an audit row as "#/logs?request_id=…", the page starts scoped to
  // that request rather than making the investigator paste the ID again.
  const [query, setQuery] = React.useState(
    () => consoleHashQuery().get("request_id") || "",
  );
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
        total: number;
        facets: DiagnosticFacets;
        retention: {days: number; maximum_events: number};
      }>(`/api/v1/admin/diagnostics/logs?${parameters}`);
      setItems(value.items);
      setTotal(value.total ?? value.items.length);
      setFacets(value.facets || {instances: [], components: [], event_codes: []});
      setRetention(value.retention);
      setError("");
    } catch (reason) {
      setError((reason as Error).message);
    }
  }, [component, instance, level, limit, query]);
  // Debounced: a pasted request ID should not fire a query per character.
  React.useEffect(() => {
    const timer = window.setTimeout(load, 200);
    return () => window.clearTimeout(timer);
  }, [load]);
  React.useEffect(() => {
    if (!autoRefresh) return;
    const timer = window.setInterval(load, 15_000);
    return () => window.clearInterval(timer);
  }, [autoRefresh, load]);
  const errors = items.filter(item => item.level === "error").length;
  const warnings = items.filter(item => item.level === "warning").length;
  const truncated = total > items.length;
  return <section>
    <PageTitle
      kicker="MULTI-POD DIAGNOSTICS"
      title="Server 진단 로그"
      subtitle="모든 Server Pod의 Agent 등록·전송 실패와 운영 오류를 공용 DB에서 request ID로 추적합니다."
      action={<button className="secondary" onClick={load}><RefreshCw size={15}/>새로고침</button>}
    />
    <div className="status-grid diagnostic-summary">
      <div><span>조건 일치</span><strong>{number(total)}</strong></div>
      <div><span>Error</span><strong>{errors}</strong></div>
      <div><span>Warning</span><strong>{warnings}</strong></div>
      <div><span>확인된 Pod</span><strong>{facets.instances.length}</strong></div>
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
        {facets.components.map(value => <option value={value} key={value}>
          {diagnosticComponentLabels[value] || value}</option>)}
      </select>
      <select value={instance} onChange={event => setInstance(event.target.value)}>
        <option value="">모든 Pod</option>
        {facets.instances.map(value => <option value={value} key={value}>{value}</option>)}
      </select>
      <select value={limit} onChange={event => setLimit(Number(event.target.value))}>
        <option value="100">100건</option><option value="200">200건</option>
        <option value="500">500건</option>
      </select>
      <label className="auto-refresh"><input type="checkbox" checked={autoRefresh}
        onChange={event => setAutoRefresh(event.target.checked)}/>15초 자동 갱신</label>
    </div>
    {error && <div className="error">{error}</div>}
    {/* Without this, a truncated page looks like the whole answer. */}
    {truncated && <Notice tone="info" title={`조건에 ${number(total)}건이 일치하며 최신 ${number(items.length)}건만 표시합니다.`}>
      조건을 좁히거나 표시 건수를 늘리십시오.
    </Notice>}
    <Panel title={`${number(items.length)}개 진단 이벤트`} action={`보존 ${retention.days}일 · 최대 ${retention.maximum_events.toLocaleString()}건`}>
      <div className="audit-table diagnostic-log-table">
        {items.map(item => <details key={item.id}>
          <summary>
            <i className={item.level === "info" ? "ok" : "bad"}/>
            <time>{formatSecond(item.occurred_at)}</time>
            <strong>{item.event_code}</strong>
            <span>{item.instance_id} · {diagnosticComponentLabels[item.component] || item.component}</span>
            <Badge value={item.level}/>
          </summary>
          <div className="audit-detail">
            <DataRow label="Message" value={item.message}/>
            <DataRow label="Pod / Instance" value={item.instance_id}/>
            <DataRow label="Request ID" value={item.request_id || "—"}/>
            <DataRow label="Agent ID" value={item.agent_id || "—"}/>
            <DataRow label="Source IP" value={item.source_ip || "—"}/>
            <DataRow label="Component" value={item.component}/>
            {item.request_id && <div className="audit-cross-link">
              <button className="link" onClick={() => setQuery(item.request_id)}>
                같은 request ID의 이벤트만 보기</button></div>}
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
    {/* "3분 전" answers the freshness question directly, and the exact instant
        stays available on hover instead of being cut off by the column. */}
    <td onClick={() => onSelect?.(asset.id)} title={formatDate(asset.last_seen_at)}>
      {formatRelative(asset.last_seen_at)}</td></tr>)}</tbody></table>
    {!items.length && <Empty icon={Boxes} text="표시할 자산이 없습니다."/>}
  </div>;
}

/**
 * Assigns the eight validated hues in fixed order and folds everything past
 * them into one "기타" row. Showing the top seven and dropping the rest silently
 * meant the visible percentages did not add up to 100 and nothing said why.
 */
export const breakdownRows = (items: Bucket[], slots = 8): Bucket[] => {
  if (items.length <= slots) return items;
  const shown = items.slice(0, slots - 1);
  const rest = items.slice(slots - 1);
  return [...shown, {
    label: `기타 ${rest.length}종`,
    count: rest.reduce((sum, item) => sum + item.count, 0),
  }];
};

function Breakdown({items}: {items: Bucket[]}) {
  const total = items.reduce((sum, item) => sum + item.count, 0);
  const rows = breakdownRows(items);
  return <div className="breakdown">{rows.map((item, index) =>
    <div key={item.label}><span className={`chart-color c${index % 8}`}/><strong>{item.label}</strong>
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
type QueryGrammar = {
  fields: {name: string; kind: string; description: string; example: string}[];
  operators: string[];
  combinator: string;
  max_clauses: number;
  max_length: number;
  relative_now: string;
};
type AuditFacets = {
  actions: Bucket[];
  resource_types: Bucket[];
  results: Bucket[];
};
type DiagnosticFacets = {
  instances: string[];
  components: string[];
  event_codes: string[];
};
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
