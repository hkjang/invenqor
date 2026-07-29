import React from "react";
import {
  CheckCircle2,
  Copy,
  Database,
  Eye,
  EyeOff,
  KeyRound,
  LockKeyholeOpen,
  Pencil,
  Power,
  RadioTower,
  RefreshCw,
  RotateCcw,
  Save,
  ServerCog,
  Shield,
  SlidersHorizontal,
  Trash2,
  UserPlus,
  Users,
} from "lucide-react";
import { api } from "./api";
import {
  consoleHash,
  loadSettingsTab,
  parseConsoleHash,
  saveSettingsTab,
  type SettingsTab,
} from "./navigationState";
import type { SystemInfo } from "./productVersion";

type PostgresTarget = {
  valid: boolean;
  host?: string;
  port?: number;
  database?: string;
  user?: string;
  tls?: boolean;
};
type PostgresStatus = {
  database_mode: string;
  schema: string;
  configured: boolean;
  saved_configured: boolean;
  environment_override: boolean;
  restart_required: boolean;
  effective: PostgresTarget | null;
  saved: PostgresTarget | null;
  startup_failure?: {
    code: string;
    summary: string;
    host?: string;
    checked_at: string;
  };
};
type AgentEnrollmentSettings = {
  enabled: boolean;
  mode: "disabled"|"open"|"token";
  token_configured: boolean;
  network_mode: "any"|"allowlist";
  allowed_networks: string[];
  trusted_proxies: string[];
  version: number;
  updated_at: string;
  updated_by: string;
  source: "database";
};
type KeycloakSettings = {
  enabled: boolean;
  issuer_url: string;
  realm: string;
  client_id: string;
  redirect_uri: string;
  logout_redirect_uri: string;
  scopes: string[];
  username_claim: string;
  email_claim: string;
  name_claim: string;
  group_claim: string;
  role_claim: string;
  role_mappings: Record<string, string>;
  group_mappings: Record<string, string>;
  auto_create_users: boolean;
  default_role: string;
  allowed_email_domains: string[];
  private_ca_pem: string;
  last_connection_test_at?: string;
  last_connection_ok: boolean;
};
const defaultKeycloakSettings: KeycloakSettings = {
  enabled: false,
  issuer_url: "",
  realm: "",
  client_id: "",
  redirect_uri: "",
  logout_redirect_uri: "",
  scopes: ["openid", "profile", "email"],
  username_claim: "preferred_username",
  email_claim: "email",
  name_claim: "name",
  group_claim: "groups",
  role_claim: "roles",
  role_mappings: {},
  group_mappings: {},
  auto_create_users: true,
  default_role: "viewer",
  allowed_email_domains: [],
  private_ca_pem: "",
  last_connection_ok: false,
};
export const normalizeKeycloakSettings = (
  value: Partial<KeycloakSettings> | null | undefined,
): KeycloakSettings => ({
  ...defaultKeycloakSettings,
  ...(value || {}),
  scopes: Array.isArray(value?.scopes) ? value.scopes : defaultKeycloakSettings.scopes,
  allowed_email_domains: Array.isArray(value?.allowed_email_domains)
    ? value.allowed_email_domains
    : [],
  role_mappings: value?.role_mappings && typeof value.role_mappings === "object"
    ? value.role_mappings
    : {},
  group_mappings: value?.group_mappings && typeof value.group_mappings === "object"
    ? value.group_mappings
    : {},
});
type ManagedUser = {
  id: string;
  username: string;
  display_name: string;
  email: string;
  active: boolean;
  super_admin: boolean;
  locked: boolean;
  provider: "local" | "keycloak";
  roles: string[];
  local_roles: string[];
  oidc_roles: string[];
  created_at: string;
  updated_at: string;
};
type ManagedRole = {
  id: string;
  name: string;
  description: string;
  system_role: boolean;
  permissions: string[];
};

const jsonRequest = (csrf: string, body: unknown, method = "POST"): RequestInit => ({
  method,
  headers: {
    "Content-Type": "application/json",
    "X-CSRF-Token": csrf,
  },
  body: JSON.stringify(body),
});

export function SettingsPage({
  csrf,
  systemInfo,
  userID,
}: {
  csrf: string;
  systemInfo: SystemInfo | null;
  userID: string;
}) {
  const [tab, setTab] = React.useState<SettingsTab>(() => loadSettingsTab(userID));
  const selectTab = React.useCallback((next: SettingsTab) => {
    setTab(next);
    saveSettingsTab(userID, next);
    const nextHash = consoleHash("settings", next);
    if (window.location.hash !== nextHash) window.location.hash = nextHash;
  }, [userID]);
  React.useEffect(() => {
    const synchronize = () => {
      const routed = parseConsoleHash(window.location.hash).settingsTab;
      if (routed) {
        setTab(routed);
        saveSettingsTab(userID, routed);
      }
    };
    window.addEventListener("hashchange", synchronize);
    return () => window.removeEventListener("hashchange", synchronize);
  }, [userID]);
  return <section>
    <AdminPageTitle
      kicker="CONTROL CENTER"
      title="운영 설정"
      subtitle="데이터베이스, Agent 등록과 조직 인증 정책을 한곳에서 검증하고 관리합니다."
    />
    <div className="settings-layout">
      <div className="settings-nav">
        <button className={tab === "postgresql" ? "active" : ""} onClick={() => selectTab("postgresql")}><Database size={17}/>PostgreSQL</button>
        <button className={tab === "agents" ? "active" : ""} onClick={() => selectTab("agents")}><RadioTower size={17}/>Agent 등록</button>
        <button className={tab === "keycloak" ? "active" : ""} onClick={() => selectTab("keycloak")}><KeyRound size={17}/>Keycloak</button>
        <button className={tab === "general" ? "active" : ""} onClick={() => selectTab("general")}><SlidersHorizontal size={17}/>고급 설정</button>
        <button className={tab === "system" ? "active" : ""} onClick={() => selectTab("system")}><ServerCog size={17}/>시스템 정보</button>
      </div>
      <div className="settings-content">
        {tab === "postgresql" && <PostgresSettings csrf={csrf}/>}
        {tab === "agents" && <AgentEnrollmentSettingsPanel csrf={csrf}/>}
        {tab === "keycloak" && <KeycloakSettingsPanel csrf={csrf}/>}
        {tab === "general" && <GeneralSettingsPanel csrf={csrf}/>}
        {tab === "system" && <SystemSettingsInfo info={systemInfo}/>}
      </div>
    </div>
  </section>;
}

function AgentEnrollmentSettingsPanel({csrf}: {csrf: string}) {
  const [policy, setPolicy] = React.useState<AgentEnrollmentSettings|null>(null);
  const [mode, setMode] = React.useState<AgentEnrollmentSettings["mode"]>("open");
  const [networkMode, setNetworkMode] = React.useState<AgentEnrollmentSettings["network_mode"]>("any");
  const [allowedNetworks, setAllowedNetworks] = React.useState("");
  const [trustedProxies, setTrustedProxies] = React.useState("");
  const [reason, setReason] = React.useState("");
  const [registrationToken, setRegistrationToken] = React.useState("");
  const [busy, setBusy] = React.useState(false);
  const [message, setMessage] = React.useState("");
  const [error, setError] = React.useState("");
  const load = React.useCallback(() =>
    api<AgentEnrollmentSettings>("/api/v1/admin/settings/agent-enrollment")
      .then(value => {
        setPolicy(value);
        setMode(value.mode);
        setNetworkMode(value.network_mode);
        setAllowedNetworks(value.allowed_networks.join("\n"));
        setTrustedProxies(value.trusted_proxies.join("\n"));
      }),
  []);
  React.useEffect(() => {
    load().catch(reason => setError((reason as Error).message));
  }, [load]);
  const save = async () => {
    setBusy(true); setError(""); setMessage("");
    try {
      const value = await api<AgentEnrollmentSettings>(
        "/api/v1/admin/settings/agent-enrollment",
        jsonRequest(csrf, {
          mode,
          network_mode: networkMode,
          allowed_networks: parseNetworkEntries(allowedNetworks),
          trusted_proxies: parseNetworkEntries(trustedProxies),
          reason,
        }, "PATCH"),
      );
      setPolicy(value); setMode(value.mode); setNetworkMode(value.network_mode);
      setAllowedNetworks(value.allowed_networks.join("\n"));
      setTrustedProxies(value.trusted_proxies.join("\n"));
      setReason("");
      setMessage(value.mode === "open"
        ? "토큰 없는 자동 등록을 활성화했습니다. Agent는 URL만으로 바로 등록됩니다."
        : value.mode === "token"
          ? "등록 토큰 보호 모드를 활성화했습니다."
          : "Agent 자동 등록을 비활성화했습니다.");
    } catch (reason) {
      setError((reason as Error).message);
    } finally {
      setBusy(false);
    }
  };
  const issueToken = async () => {
    if (policy?.token_configured &&
      !window.confirm("현재 등록 토큰을 즉시 교체합니까? 기존 등록 토큰은 더 이상 사용할 수 없습니다.")) return;
    setBusy(true); setError(""); setMessage(""); setRegistrationToken("");
    try {
      const value = await api<AgentEnrollmentSettings & {registration_token: string}>(
        "/api/v1/admin/settings/agent-enrollment/token",
        jsonRequest(csrf, {reason}),
      );
      setPolicy(value); setMode(value.mode); setRegistrationToken(value.registration_token);
      setReason("");
      setMessage("등록 토큰을 발급하고 토큰 보호 모드를 활성화했습니다.");
    } catch (reason) {
      setError((reason as Error).message);
    } finally {
      setBusy(false);
    }
  };
  const deleteToken = async () => {
    if (!window.confirm("등록 토큰을 폐기합니까? 자동 등록이 활성 상태라면 URL-only Open 모드로 전환됩니다.")) return;
    setBusy(true); setError(""); setMessage(""); setRegistrationToken("");
    try {
      const value = await api<AgentEnrollmentSettings>(
        "/api/v1/admin/settings/agent-enrollment/token",
        jsonRequest(csrf, {reason}, "DELETE"),
      );
      setPolicy(value); setMode(value.mode); setReason("");
      setMessage(value.mode === "open"
        ? "등록 토큰을 폐기하고 URL-only 자동 등록으로 전환했습니다."
        : "등록 토큰을 폐기했습니다.");
    } catch (reason) {
      setError((reason as Error).message);
    } finally {
      setBusy(false);
    }
  };
  if (!policy) {
    return <AdminPanel title="Agent 자동 등록" action="실시간 정책">
      <div className="settings-body">{error || "등록 정책을 불러오는 중입니다."}</div>
    </AdminPanel>;
  }
  const options: {value: AgentEnrollmentSettings["mode"]; title: string; description: string; badge?: string}[] = [
    {
      value: "open",
      title: "토큰 없이 자동 등록",
      description: "Agent config.toml에 Server URL만 입력하면 최초 통신 때 즉시 자산으로 등록됩니다.",
      badge: "URL-ONLY",
    },
    {
      value: "token",
      title: "등록 토큰 필요",
      description: "최초 등록에 공용 등록 토큰을 요구합니다. 이미 등록된 Agent의 장치 토큰에는 영향이 없습니다.",
      badge: "PROTECTED",
    },
    {
      value: "disabled",
      title: "자동 등록 비활성",
      description: "새 Agent 등록을 차단합니다. 기존 등록 Agent의 수집과 전송은 계속 허용됩니다.",
    },
  ];
  const policyChanged = mode !== policy.mode ||
    networkMode !== policy.network_mode ||
    parseNetworkEntries(allowedNetworks).join("\n") !== policy.allowed_networks.join("\n") ||
    parseNetworkEntries(trustedProxies).join("\n") !== policy.trusted_proxies.join("\n");
  return <AdminPanel
    title="Agent 자동 등록"
    action={`DB 정책 v${policy.version} · ${policy.mode.toUpperCase()}`}
  >
    <div className="settings-body agent-enrollment-settings">
      <div className="enrollment-status status-grid">
        <StatusItem label="현재 상태" value={policy.enabled ? "활성" : "비활성"} good={policy.enabled}/>
        <StatusItem label="등록 방식" value={policy.mode === "open" ? "URL-only" : policy.mode === "token" ? "Token 보호" : "차단"}/>
        <StatusItem label="접속 IP 정책" value={policy.network_mode === "any" ? "모든 IP" : `${policy.allowed_networks.length}개 규칙`} good={policy.network_mode === "allowlist"}/>
        <StatusItem label="정책 공유" value="DB · 모든 Pod" good/>
      </div>
      <div className="enrollment-mode-grid" role="radiogroup" aria-label="Agent 자동 등록 방식">
        {options.map(option => <label
          key={option.value}
          className={mode === option.value ? "enrollment-mode selected" : "enrollment-mode"}
        >
          <input
            type="radio"
            name="agent-enrollment-mode"
            value={option.value}
            checked={mode === option.value}
            disabled={option.value === "token" && !policy.token_configured}
            onChange={() => setMode(option.value)}
          />
          <span>
            <strong>{option.title}{option.badge && <b>{option.badge}</b>}</strong>
            <small>{option.description}</small>
          </span>
          {mode === option.value && <CheckCircle2 size={19}/>}
        </label>)}
      </div>
      {mode === "open" && <Notice tone="warning" title="URL-only Zero-touch 등록이 허용됩니다.">
        7070 포트에 접근할 수 있는 장치는 토큰 없이 등록할 수 있습니다. 신뢰된 내부망에서 사용하고 외부 노출 시 토큰 보호 모드를 사용하십시오.
      </Notice>}
      {!policy.token_configured && <Notice tone="info" title="Protected 모드를 사용하려면 먼저 토큰을 발급하십시오.">
        발급 버튼은 새 토큰을 한 번만 표시하고 즉시 토큰 보호 모드로 전환합니다.
      </Notice>}
      <div className="enrollment-network-policy">
        <div className="section-heading">
          <div>
            <strong>자동 등록 허용 IP / CIDR</strong>
            <small>Agent의 최초 접속 IP를 판정해 등록을 허용하고 즉시 IP 기반 host 자산을 생성합니다.</small>
          </div>
          <Shield size={18}/>
        </div>
        <div className="network-mode-switch" role="radiogroup" aria-label="자동 등록 IP 범위">
          <label className={networkMode === "any" ? "selected" : ""}>
            <input type="radio" name="network-mode" checked={networkMode === "any"}
              onChange={() => setNetworkMode("any")}/>
            <span><strong>모든 IP 허용</strong><small>네트워크 제한 없이 현재 등록 방식을 적용합니다.</small></span>
          </label>
          <label className={networkMode === "allowlist" ? "selected" : ""}>
            <input type="radio" name="network-mode" checked={networkMode === "allowlist"}
              onChange={() => setNetworkMode("allowlist")}/>
            <span><strong>지정 IP만 허용</strong><small>아래의 정확한 IP 또는 CIDR에 일치할 때만 등록합니다.</small></span>
          </label>
        </div>
        <div className="network-rule-grid">
          <label>허용 IP / CIDR <b>{parseNetworkEntries(allowedNetworks).length}</b>
            <textarea
              value={allowedNetworks}
              onChange={event => setAllowedNetworks(event.target.value)}
              placeholder={"10.20.30.40\n10.20.40.0/24\n2001:db8:100::/64"}
              rows={6}
            />
            <small>한 줄에 하나씩 입력합니다. 단일 IPv4/IPv6와 CIDR을 함께 사용할 수 있습니다.</small>
          </label>
          <label>신뢰 프록시 IP / CIDR <b>{parseNetworkEntries(trustedProxies).length}</b>
            <textarea
              value={trustedProxies}
              onChange={event => setTrustedProxies(event.target.value)}
              placeholder={"10.0.0.10\n10.0.1.0/24"}
              rows={6}
            />
            <small>Ingress/LB가 이 범위에서 접속할 때만 X-Forwarded-For를 신뢰합니다. 비워두면 직접 접속 IP로 판정합니다.</small>
          </label>
        </div>
        {networkMode === "allowlist" && parseNetworkEntries(allowedNetworks).length === 0 &&
          <Notice tone="error" title="허용 IP 또는 CIDR이 필요합니다.">
            빈 허용 목록은 저장되지 않습니다. Agent가 실제로 접속하는 주소 또는 대역을 추가하십시오.
          </Notice>}
      </div>
      {registrationToken && <div className="secret-reveal enrollment-token-reveal">
        <div>
          <strong>등록 토큰 — 지금 한 번만 표시됩니다</strong>
          <code>{registrationToken}</code>
        </div>
        <button className="secondary" onClick={() => navigator.clipboard.writeText(registrationToken)}>
          <Copy size={16}/>복사
        </button>
        <button className="secondary" onClick={() => setRegistrationToken("")}>닫기</button>
      </div>}
      <label className="enrollment-reason">변경 사유
        <input value={reason} onChange={event => setReason(event.target.value)}
          placeholder="예: 사내망 URL-only 자동 등록 허용"/>
      </label>
      <div className="form-actions enrollment-actions">
        <button className="secondary" disabled={busy} onClick={load}><RefreshCw size={16}/>새로고침</button>
        {policy.token_configured && <button className="secondary" disabled={busy} onClick={deleteToken}><Trash2 size={16}/>토큰 폐기</button>}
        <button className="secondary" disabled={busy} onClick={issueToken}><KeyRound size={16}/>{policy.token_configured ? "토큰 회전" : "토큰 발급"}</button>
        <button className="primary compact"
          disabled={busy || !policyChanged || (networkMode === "allowlist" && parseNetworkEntries(allowedNetworks).length === 0)}
          onClick={save}><Save size={16}/>정책 적용</button>
      </div>
      <ActionMessage message={message} error={error}/>
      <div className="agent-config-example">
        <strong>URL-only Agent 설정 예시</strong>
        <pre>{`[server]\nurl = "https://invenqor.example.com:7070"`}</pre>
        <small>Open 모드에서는 등록 토큰이나 사전 자산 생성이 필요하지 않습니다. Agent가 최초 수집을 전송하면 자동 등록됩니다.</small>
      </div>
    </div>
  </AdminPanel>;
}

export function parseNetworkEntries(value: string): string[] {
  return [...new Set(value.split(/[\n,]+/).map(entry => entry.trim()).filter(Boolean))].sort();
}

function PostgresSettings({csrf}: {csrf: string}) {
  const [status, setStatus] = React.useState<PostgresStatus|null>(null);
  const [dsn, setDSN] = React.useState("");
  const [reason, setReason] = React.useState("");
  const [showDSN, setShowDSN] = React.useState(false);
  const [busy, setBusy] = React.useState(false);
  const [message, setMessage] = React.useState("");
  const [error, setError] = React.useState("");
  const load = React.useCallback(() =>
    api<PostgresStatus>("/api/v1/admin/settings/postgresql").then(setStatus),
  []);
  React.useEffect(() => {
    load().catch(reason => setError((reason as Error).message));
  }, [load]);
  const run = async (save: boolean) => {
    setBusy(true); setError(""); setMessage("");
    try {
      const path = save
        ? "/api/v1/admin/settings/postgresql"
        : "/api/v1/admin/settings/postgresql/test";
      await api(path, jsonRequest(csrf, {dsn, reason}, save ? "PATCH" : "POST"));
      setMessage(save
        ? "암호화 저장했습니다. 표시된 조건에 따라 Server 재기동 후 적용됩니다."
        : "PostgreSQL 연결에 성공했습니다. 아직 설정은 저장하지 않았습니다.");
      if (save) {
        setDSN(""); setReason(""); await load();
      }
    } catch (reason) {
      setError((reason as Error).message);
    } finally {
      setBusy(false);
    }
  };
  const target = status?.effective;
  return <AdminPanel title="PostgreSQL 연결" action="연결 검증 후 암호화 저장">
    <div className="settings-body">
      <div className="status-grid">
        <StatusItem label="현재 모드" value={status?.database_mode || "확인 중"} good={status?.database_mode === "POSTGRES_ACTIVE"}/>
        <StatusItem label="현재 대상" value={formatPostgresTarget(target)} good={!!target?.valid}/>
        <StatusItem label="Schema" value={status?.schema || "public"}/>
        <StatusItem label="설정 원천" value={status?.environment_override ? "환경변수 우선" : status?.saved_configured ? "암호화 파일" : "미설정"}/>
      </div>
      {status?.startup_failure && <Notice tone="error" title={`${status.startup_failure.code} · ${status.startup_failure.summary}`}>
        {status.startup_failure.host ? `대상 호스트: ${status.startup_failure.host}` : "DSN 형식과 접속 정보를 확인하십시오."}
      </Notice>}
      {status?.environment_override && <Notice tone="warning" title="환경변수 설정이 우선 적용됩니다.">
        저장값을 적용하려면 `INVENQOR_POSTGRES_DSN`, `POSTGRES_DSN` 또는 `postgres_dsn`을 제거한 뒤 재기동해야 합니다.
      </Notice>}
      {status?.restart_required && <Notice tone="warning" title="저장된 변경의 재기동 적용이 필요합니다.">
        단일 Server를 순차 재기동하십시오. Kubernetes에서는 모든 Pod에 같은 Secret 환경변수를 배포하십시오.
      </Notice>}
      <Notice tone="info" title="SQLite 데이터는 자동 이전하지 않습니다.">
        기존 SQLite 운영 데이터를 PostgreSQL로 옮겨야 한다면 백업·이관을 완료한 뒤 전환하십시오. 새 PostgreSQL은 별도 사용자·자산 DB입니다.
      </Notice>
      <div className="admin-form">
        <label className="wide">PostgreSQL DSN
          <div className="secret-input">
            <input
              type={showDSN ? "text" : "password"}
              value={dsn}
              onChange={event => setDSN(event.target.value)}
              placeholder="postgres://user:password@host:5432/invenqor?sslmode=require"
              autoComplete="off"
            />
            <button type="button" onClick={() => setShowDSN(value => !value)} title={showDSN ? "숨기기" : "표시"}>
              {showDSN ? <EyeOff size={17}/> : <Eye size={17}/>}
            </button>
          </div>
        </label>
        <label className="wide">변경 사유
          <input value={reason} onChange={event => setReason(event.target.value)} placeholder="예: 운영 PostgreSQL 전환"/>
        </label>
      </div>
      <div className="form-actions">
        <button className="secondary" disabled={busy || !dsn.trim()} onClick={() => run(false)}><RefreshCw size={16}/>연결 테스트</button>
        <button className="primary compact" disabled={busy || !dsn.trim()} onClick={() => run(true)}><Save size={16}/>검증 후 저장</button>
      </div>
      <ActionMessage message={message} error={error}/>
      <div className="env-help">
        <strong>이미지 환경변수</strong>
        <code>INVENQOR_POSTGRES_DSN</code>
        <code>POSTGRES_DSN</code>
        <code>postgres_dsn</code>
      </div>
    </div>
  </AdminPanel>;
}

function KeycloakSettingsPanel({csrf}: {csrf: string}) {
  const [settings, setSettings] = React.useState<KeycloakSettings|null>(null);
  const [secretConfigured, setSecretConfigured] = React.useState(false);
  const [clientSecret, setClientSecret] = React.useState("");
  const [quickKeycloakURL, setQuickKeycloakURL] = React.useState("");
  const [quickRealm, setQuickRealm] = React.useState("");
  const [quickClientID, setQuickClientID] = React.useState("");
  const [applicationURL, setApplicationURL] = React.useState(
    typeof window === "undefined" ? "" : window.location.origin,
  );
  const [roleMappings, setRoleMappings] = React.useState("");
  const [groupMappings, setGroupMappings] = React.useState("");
  const [reason, setReason] = React.useState("");
  const [busy, setBusy] = React.useState(false);
  const [message, setMessage] = React.useState("");
  const [error, setError] = React.useState("");
  const load = React.useCallback(() =>
    api<{settings: KeycloakSettings; client_secret_configured: boolean}>(
      "/api/v1/admin/settings/keycloak",
    ).then(value => {
      const normalized = normalizeKeycloakSettings(value.settings);
      setSettings(normalized);
      setSecretConfigured(value.client_secret_configured);
      setRoleMappings(formatMappings(normalized.role_mappings));
      setGroupMappings(formatMappings(normalized.group_mappings));
      setQuickKeycloakURL(normalized.issuer_url);
      setQuickRealm(normalized.realm);
      setQuickClientID(normalized.client_id);
      if (normalized.redirect_uri) {
        try {
          setApplicationURL(new URL(normalized.redirect_uri).origin);
        } catch {
          // Keep the browser origin when a legacy redirect URI is malformed.
        }
      }
    }),
  []);
  React.useEffect(() => {
    load().catch(reason => setError((reason as Error).message));
  }, [load]);
  if (!settings) {
    return <AdminPanel title="Keycloak" action="OIDC"><div className="settings-body">{error || "설정을 불러오는 중입니다."}</div></AdminPanel>;
  }
  const change = <K extends keyof KeycloakSettings>(key: K, value: KeycloakSettings[K]) =>
    setSettings(current => current ? {...current, [key]: value} : current);
  const requestSettings = () => ({
    ...settings,
    role_mappings: parseMappings(roleMappings, "역할 매핑"),
    group_mappings: parseMappings(groupMappings, "그룹 매핑"),
  });
  const test = async () => {
    setBusy(true); setError(""); setMessage("");
    try {
      await api("/api/v1/admin/settings/keycloak/test", jsonRequest(csrf, {
        settings: requestSettings(),
      }));
      setMessage("Issuer discovery 연결 테스트에 성공했습니다.");
    } catch (reason) {
      setError((reason as Error).message);
    } finally {
      setBusy(false);
    }
  };
  const save = async () => {
    setBusy(true); setError(""); setMessage("");
    try {
      await api(
        "/api/v1/admin/settings/keycloak",
        jsonRequest(csrf, {
          settings: requestSettings(),
          reason,
          ...(clientSecret ? {client_secret: clientSecret} : {}),
        }, "PATCH"),
      );
      setClientSecret(""); setReason("");
      setMessage("Keycloak 설정을 저장했습니다. 새 로그인부터 적용됩니다.");
      await load();
    } catch (reason) {
      setError((reason as Error).message);
    } finally {
      setBusy(false);
    }
  };
  const autoConfigure = async () => {
    setBusy(true); setError(""); setMessage("");
    try {
      const value = await api<{
        settings: KeycloakSettings;
        client_secret_configured: boolean;
        discovery_issuer: string;
        redirect_uri: string;
      }>(
        "/api/v1/admin/settings/keycloak/auto-configure",
        jsonRequest(csrf, {
          keycloak_url: quickKeycloakURL,
          realm: quickRealm,
          client_id: quickClientID,
          application_url: applicationURL,
          private_ca_pem: settings.private_ca_pem,
          reason: reason || "Keycloak OIDC 빠른 연동",
          ...(clientSecret ? {client_secret: clientSecret} : {}),
        }),
      );
      setSettings(normalizeKeycloakSettings(value.settings));
      setSecretConfigured(value.client_secret_configured);
      setClientSecret(""); setReason("");
      setMessage(`OIDC Discovery와 TLS 검증을 완료하고 로그인을 활성화했습니다. Callback: ${value.redirect_uri}`);
      await load();
    } catch (reason) {
      setError((reason as Error).message);
    } finally {
      setBusy(false);
    }
  };
  return <AdminPanel title="Keycloak OIDC" action={secretConfigured ? "Client Secret 설정됨" : "Client Secret 미설정"}>
    <div className="settings-body">
      <div className="keycloak-quick-setup">
        <div className="section-heading">
          <div>
            <strong>최소 정보 빠른 연동</strong>
            <small>OIDC Discovery, Callback/Logout URI, 표준 Scope와 Claim을 자동 구성하고 연결 성공 후에만 저장합니다.</small>
          </div>
          <KeyRound size={19}/>
        </div>
        <div className="admin-form">
          <label className="wide">Keycloak 주소
            <input value={quickKeycloakURL} onChange={event => setQuickKeycloakURL(event.target.value)}
              placeholder="https://sso.example.com"/>
          </label>
          <label>Realm
            <input value={quickRealm} onChange={event => setQuickRealm(event.target.value)}
              placeholder="invenqor"/>
          </label>
          <label>Client ID
            <input value={quickClientID} onChange={event => setQuickClientID(event.target.value)}
              placeholder="invenqor"/>
          </label>
          <label className="wide">InvenQor 외부 주소
            <input value={applicationURL} onChange={event => setApplicationURL(event.target.value)}
              placeholder="https://invenqor.example.com"/>
            <small>현재 접속 주소가 기본값입니다. Ingress 외부 주소가 다르면 수정하십시오.</small>
          </label>
          <label className="wide">Client Secret
            <input type="password" value={clientSecret} onChange={event => setClientSecret(event.target.value)}
              placeholder={secretConfigured ? "기존 Secret 사용 · 회전할 때만 입력" : "Keycloak Client Secret"}
              autoComplete="new-password"/>
          </label>
        </div>
        <div className="form-actions">
          <button className="primary compact" disabled={
            busy || !quickKeycloakURL.trim() || !quickClientID.trim() ||
            !applicationURL.trim() || (!secretConfigured && !clientSecret.trim())
          } onClick={autoConfigure}>
            <Power size={16}/>자동 구성 · 검증 · 활성화
          </button>
        </div>
      </div>
      <div className="advanced-settings-heading">
        <strong>고급 정책</strong>
        <small>기본값을 세밀하게 조정하거나 Role/Group 매핑과 사설 CA를 구성합니다.</small>
      </div>
      <label className="toggle-row"><input type="checkbox" checked={settings.enabled} onChange={event => change("enabled", event.target.checked)}/><span><strong>Keycloak 로그인 활성화</strong><small>로컬 로그인은 비상 접근 경로로 유지됩니다.</small></span></label>
      <div className="admin-form">
        <label className="wide">Issuer URL<input value={settings.issuer_url} onChange={event => change("issuer_url", event.target.value)} placeholder="https://sso.example.com"/></label>
        <label>Realm<input value={settings.realm} onChange={event => change("realm", event.target.value)} placeholder="invenqor"/></label>
        <label>Client ID<input value={settings.client_id} onChange={event => change("client_id", event.target.value)}/></label>
        <label className="wide">Redirect URI<input value={settings.redirect_uri} onChange={event => change("redirect_uri", event.target.value)} placeholder="https://invenqor.example.com/api/v1/auth/keycloak/callback"/></label>
        <label className="wide">Logout Redirect URI<input value={settings.logout_redirect_uri} onChange={event => change("logout_redirect_uri", event.target.value)}/></label>
        <label>Scopes<input value={settings.scopes.join(", ")} onChange={event => change("scopes", splitList(event.target.value))}/></label>
        <label>기본 역할<input value={settings.default_role} onChange={event => change("default_role", event.target.value)}/></label>
        <label>Username Claim<input value={settings.username_claim} onChange={event => change("username_claim", event.target.value)}/></label>
        <label>Email Claim<input value={settings.email_claim} onChange={event => change("email_claim", event.target.value)}/></label>
        <label>Name Claim<input value={settings.name_claim} onChange={event => change("name_claim", event.target.value)}/></label>
        <label>Role Claim<input value={settings.role_claim} onChange={event => change("role_claim", event.target.value)}/></label>
        <label>Group Claim<input value={settings.group_claim} onChange={event => change("group_claim", event.target.value)}/></label>
        <label className="wide">허용 Email Domain
          <input
            value={settings.allowed_email_domains.join(", ")}
            onChange={event => change("allowed_email_domains", splitList(event.target.value))}
            placeholder="example.com, subsidiary.example.com (비우면 제한 없음)"
          />
        </label>
        <label className="wide">Keycloak Role → InvenQor Role
          <textarea
            className="mapping-editor"
            value={roleMappings}
            onChange={event => setRoleMappings(event.target.value)}
            placeholder={"realm-admin=super_admin\ninventory-viewer=viewer"}
            spellCheck={false}
          />
          <small>한 줄에 하나씩 `Keycloak 역할=InvenQor 역할` 형식으로 입력합니다.</small>
        </label>
        <label className="wide">Keycloak Group → InvenQor Role
          <textarea
            className="mapping-editor"
            value={groupMappings}
            onChange={event => setGroupMappings(event.target.value)}
            placeholder={"/invenqor/operators=operator\n/invenqor/auditors=auditor"}
            spellCheck={false}
          />
          <small>Keycloak 그룹의 전체 경로와 InvenQor 역할을 연결합니다.</small>
        </label>
        <label className="wide">사설 CA 인증서 (PEM)
          <textarea
            className="pem-editor"
            value={settings.private_ca_pem}
            onChange={event => change("private_ca_pem", event.target.value)}
            placeholder={"-----BEGIN CERTIFICATE-----\n...\n-----END CERTIFICATE-----"}
            spellCheck={false}
          />
          <small>사내 CA로 Keycloak TLS를 구성한 경우에만 입력합니다.</small>
        </label>
        <label className="wide">변경 사유<input value={reason} onChange={event => setReason(event.target.value)}/></label>
      </div>
      <label className="toggle-row"><input type="checkbox" checked={settings.auto_create_users} onChange={event => change("auto_create_users", event.target.checked)}/><span><strong>최초 로그인 사용자 자동 생성</strong><small>기본 역할과 mapping 정책을 적용합니다.</small></span></label>
      <div className="form-actions">
        <button className="secondary" disabled={busy || !settings.enabled} onClick={test}><RefreshCw size={16}/>연결 테스트</button>
        <button className="primary compact" disabled={busy} onClick={save}><Save size={16}/>설정 저장</button>
      </div>
      <ActionMessage message={message} error={error}/>
    </div>
  </AdminPanel>;
}

type GeneralSetting = {
  key: string;
  value: unknown;
  secret: boolean;
  apply_mode: "immediate"|"restart"|"migration";
  pending: boolean;
  version: number;
  updated_at?: string;
};
type SettingVersion = {
  id: string;
  key: string;
  version: number;
  before: unknown;
  after: unknown;
  changed_by?: string;
  reason: string;
  created_at: string;
};

function GeneralSettingsPanel({csrf}: {csrf: string}) {
  const [items, setItems] = React.useState<GeneralSetting[]>([]);
  const [history, setHistory] = React.useState<SettingVersion[]>([]);
  const [key, setKey] = React.useState("");
  const [value, setValue] = React.useState("{}");
  const [secret, setSecret] = React.useState(false);
  const [applyMode, setApplyMode] = React.useState<GeneralSetting["apply_mode"]>("immediate");
  const [reason, setReason] = React.useState("");
  const [message, setMessage] = React.useState("");
  const [error, setError] = React.useState("");
  const load = React.useCallback(() => Promise.all([
    api<{items: GeneralSetting[]}>("/api/v1/admin/settings").then(result => setItems(result.items)),
    api<{items: SettingVersion[]}>("/api/v1/admin/settings/history").then(result => setHistory(result.items)),
  ]), []);
  React.useEffect(() => { load().catch(reason => setError((reason as Error).message)); }, [load]);
  const select = (item: GeneralSetting) => {
    setKey(item.key); setSecret(item.secret); setApplyMode(item.apply_mode);
    setValue(item.secret ? "\"새 비밀값을 입력\"" : JSON.stringify(item.value, null, 2));
  };
  const save = async (event: React.FormEvent) => {
    event.preventDefault(); setError(""); setMessage("");
    try {
      const parsed = JSON.parse(value);
      await api("/api/v1/admin/settings", jsonRequest(csrf, {
        settings: [{key, value: parsed, secret, apply_mode: applyMode, reason}],
      }, "PATCH"));
      setMessage("설정을 저장하고 새 버전을 기록했습니다.");
      setReason(""); await load();
    } catch (reason) { setError((reason as Error).message); }
  };
  const rollback = async (entry: SettingVersion) => {
    if (!window.confirm(`${entry.key} 설정을 버전 ${entry.version} 값으로 되돌립니까?`)) return;
    try {
      await api("/api/v1/admin/settings/rollback", jsonRequest(csrf, {
        key: entry.key, version: entry.version, reason: "관리 콘솔 설정 롤백",
      }));
      setMessage(`${entry.key} 설정을 롤백했습니다.`); await load();
    } catch (reason) { setError((reason as Error).message); }
  };
  return <div className="general-settings-grid">
    <AdminPanel title="설정 카탈로그" action={`${items.length}개`}>
      <div className="setting-catalog">{items.map(item => <button key={item.key} onClick={() => select(item)}>
        <div><strong>{item.key}</strong><span>v{item.version} · {item.apply_mode}</span></div>
        <div>{item.secret && <b>SECRET</b>}{item.pending && <b className="pending">PENDING</b>}</div>
      </button>)}{!items.length && <div className="admin-empty">등록된 고급 설정이 없습니다.</div>}</div>
    </AdminPanel>
    <AdminPanel title={key ? `설정 편집 · ${key}` : "새 설정"} action="JSON 값">
      <form className="settings-body" onSubmit={save}>
        <div className="admin-form">
          <label className="wide">Key<input value={key} onChange={event => setKey(event.target.value)}
            placeholder="agents.collection.interval" required/></label>
          <label>적용 방식<select value={applyMode} onChange={event => setApplyMode(event.target.value as GeneralSetting["apply_mode"])}>
            <option value="immediate">immediate</option><option value="restart">restart</option>
            <option value="migration">migration</option></select></label>
          <label className="toggle-row"><input type="checkbox" checked={secret} onChange={event => setSecret(event.target.checked)}/>
            <span><strong>비밀값</strong><small>암호화 저장·조회 마스킹</small></span></label>
          <label className="wide">JSON Value<textarea value={value} onChange={event => setValue(event.target.value)}
            spellCheck={false} required/></label>
          <label className="wide">변경 사유<input value={reason} onChange={event => setReason(event.target.value)} required/></label>
        </div>
        <div className="form-actions"><button className="primary compact"><Save size={15}/>버전 저장</button></div>
        <ActionMessage message={message} error={error}/>
      </form>
    </AdminPanel>
    <div className="general-history">
      <AdminPanel title="설정 변경 이력" action={`최근 ${history.length}건`}>
        <div className="history-list">{history.map(entry => <details key={entry.id}>
          <summary><div><strong>{entry.key}</strong><span>v{entry.version} · {formatAdminDate(entry.created_at)}</span></div>
            <span>{entry.reason || "변경 사유 없음"}</span><button onClick={event => {event.preventDefault(); rollback(entry);}}>
              <RotateCcw size={14}/>롤백</button></summary>
          <pre>{JSON.stringify({before: entry.before, after: entry.after}, null, 2)}</pre>
        </details>)}</div>
      </AdminPanel>
    </div>
  </div>;
}

function SystemSettingsInfo({info}: {info: SystemInfo|null}) {
  const [health, setHealth] = React.useState<Record<string, unknown>>({});
  const [error, setError] = React.useState("");
  const load = React.useCallback(async () => {
    try {
      const [live, ready, database] = await Promise.all([
        api<Record<string, unknown>>("/health/live"),
        api<Record<string, unknown>>("/health/ready"),
        api<Record<string, unknown>>("/health/database"),
      ]);
      setHealth({live, ready, database}); setError("");
    } catch (reason) { setError((reason as Error).message); }
  }, []);
  React.useEffect(() => { load(); }, [load]);
  return <AdminPanel title="실행 정보" action="읽기 전용">
    <div className="setting-list">
      <InfoRow label="Server 버전" value={info?.server_version || "확인 중"}/>
      <InfoRow label="Commit" value={info?.commit || "unknown"}/>
      <InfoRow label="Build time" value={info?.build_time || "unknown"}/>
      <InfoRow label="Database mode" value={info?.database_mode || "확인 중"}/>
      <InfoRow label="서비스 포트" value="7070"/>
      <InfoRow label="Agent 자동 등록" value={
        info?.agent_enrollment_mode === "open" ? "활성 · URL-only" :
          info?.agent_enrollment_mode === "token" ? "활성 · 공용 Token 보호" :
            info?.agent_auto_enrollment ? "활성" : "비활성"
      }/>
      <InfoRow label="Liveness" value={String((health.live as Record<string, unknown>|undefined)?.status || "확인 중")}/>
      <InfoRow label="Readiness" value={String((health.ready as Record<string, unknown>|undefined)?.status || "확인 중")}/>
      <InfoRow label="Database health" value={String((health.database as Record<string, unknown>|undefined)?.mode || "확인 중")}/>
    </div>
    <div className="form-actions settings-body"><button className="secondary" onClick={load}><RefreshCw size={15}/>상태 다시 확인</button></div>
    {error && <div className="error action-message">{error}</div>}
  </AdminPanel>;
}

export function UsersPage({
  csrf,
  currentUserID,
}: {
  csrf: string;
  currentUserID: string;
}) {
  const [users, setUsers] = React.useState<ManagedUser[]>([]);
  const [roles, setRoles] = React.useState<ManagedRole[]>([]);
  const [username, setUsername] = React.useState("");
  const [displayName, setDisplayName] = React.useState("");
  const [email, setEmail] = React.useState("");
  const [password, setPassword] = React.useState("");
  const [selectedRoles, setSelectedRoles] = React.useState<string[]>(["viewer"]);
  const [query, setQuery] = React.useState("");
  const [busy, setBusy] = React.useState(false);
  const [message, setMessage] = React.useState("");
  const [error, setError] = React.useState("");
  const load = React.useCallback(() => Promise.all([
    api<{users: ManagedUser[]}>("/api/v1/admin/users").then(value => setUsers(value.users)),
    api<{roles: ManagedRole[]}>("/api/v1/admin/roles").then(value => setRoles(value.roles)),
  ]), []);
  React.useEffect(() => {
    load().catch(reason => setError((reason as Error).message));
  }, [load]);
  const mutate = async (work: () => Promise<unknown>, success: string) => {
    setBusy(true); setError(""); setMessage("");
    try {
      await work(); setMessage(success); await load();
    } catch (reason) {
      setError((reason as Error).message);
    } finally {
      setBusy(false);
    }
  };
  const create = (event: React.FormEvent) => {
    event.preventDefault();
    return mutate(async () => {
      await api("/api/v1/admin/users", jsonRequest(csrf, {
        username, display_name: displayName, email, password,
        roles: selectedRoles, reason: "관리 콘솔 사용자 생성",
      }));
      setUsername(""); setDisplayName(""); setEmail(""); setPassword("");
      setSelectedRoles(["viewer"]);
    }, "사용자를 생성했습니다.");
  };
  const updateRoles = (user: ManagedUser, role: string) => {
    const localRoles = user.local_roles || [];
    const next = localRoles.includes(role)
      ? localRoles.filter(value => value !== role)
      : [...localRoles, role];
    // Permissions come only from roles, so removing the last one would leave an
    // account that can sign in and reach nothing. Say so before the round trip.
    if (!next.length && !(user.oidc_roles || []).length) {
      setMessage("");
      setError("역할을 최소 하나는 유지해야 합니다. 접근을 막으려면 계정을 비활성화하십시오.");
      return Promise.resolve();
    }
    return mutate(
      () => api(`/api/v1/admin/users/${user.id}`, jsonRequest(csrf, {
        roles: next, reason: "관리 콘솔 역할 변경",
      }, "PATCH")),
      "사용자 역할을 변경했습니다.",
    );
  };
  const toggleActive = (user: ManagedUser) => mutate(
    () => api(`/api/v1/admin/users/${user.id}`, jsonRequest(csrf, {
      active: !user.active,
      reason: user.active ? "관리 콘솔 계정 비활성화" : "관리 콘솔 계정 활성화",
    }, "PATCH")),
    user.active ? "사용자를 비활성화하고 세션·API 키를 폐기했습니다." : "사용자를 활성화했습니다.",
  );
  const editProfile = (user: ManagedUser) => {
    const nextName = window.prompt("표시 이름", user.display_name);
    if (nextName === null) return;
    const nextEmail = window.prompt("Email", user.email);
    if (nextEmail === null) return;
    return mutate(
      () => api(`/api/v1/admin/users/${user.id}`, jsonRequest(csrf, {
        display_name: nextName, email: nextEmail, reason: "관리 콘솔 프로필 변경",
      }, "PATCH")),
      "사용자 정보를 변경했습니다.",
    );
  };
  const resetPassword = (user: ManagedUser) => {
    const next = window.prompt(`${user.username}의 새 임시 비밀번호`);
    if (!next) return;
    return mutate(
      () => api(`/api/v1/admin/users/${user.id}/password`, jsonRequest(csrf, {
        password: next, reason: "관리자 비밀번호 초기화",
      })),
      "비밀번호를 변경하고 기존 세션을 모두 폐기했습니다.",
    );
  };
  const unlock = (user: ManagedUser) => mutate(
    () => api(`/api/v1/admin/users/${user.id}/unlock`, jsonRequest(csrf, {})),
    "계정 잠금을 해제했습니다.",
  );
  const remove = (user: ManagedUser) => {
    if (!window.confirm(`${user.username} 사용자를 삭제합니까? 세션과 API 키가 즉시 폐기됩니다.`)) return;
    return mutate(
      () => api(`/api/v1/admin/users/${user.id}`, {
        method: "DELETE",
        headers: {"X-CSRF-Token": csrf},
      }),
      "사용자를 삭제했습니다.",
    );
  };
  const filtered = users.filter(user =>
    `${user.username} ${user.display_name} ${user.email}`.toLowerCase().includes(query.toLowerCase()),
  );
  return <section>
    <AdminPageTitle kicker="IDENTITY GOVERNANCE" title="사용자 관리" subtitle="로컬·Keycloak 사용자, 역할, 활성 상태와 자격 증명을 통제합니다."/>
    <ActionMessage message={message} error={error}/>
    <div className="user-admin-layout">
      <AdminPanel title="로컬 사용자 생성" action="Argon2id 비밀번호">
        <form className="user-create-form" onSubmit={create}>
          <label>사용자 ID<input value={username} onChange={event => setUsername(event.target.value)} required minLength={3}/></label>
          <label>표시 이름<input value={displayName} onChange={event => setDisplayName(event.target.value)}/></label>
          <label>Email<input type="email" value={email} onChange={event => setEmail(event.target.value)}/></label>
          <label>초기 비밀번호<input type="password" value={password} onChange={event => setPassword(event.target.value)} required autoComplete="new-password"/></label>
          <fieldset className="role-selector"><legend><span>역할</span>
            <small>{selectedRoles.length}개 선택</small></legend>
            <div className="role-options">{roles.map(role => {
              const selected = selectedRoles.includes(role.name);
              return <label className={`role-check${selected ? " selected" : ""}`}
                key={role.id} title={role.permissions.join(", ")}>
                <input type="checkbox" checked={selected} onChange={() =>
                  setSelectedRoles(selected
                    ? selectedRoles.filter(value => value !== role.name)
                    : [...selectedRoles, role.name])}/>
                <span className="role-checkmark"><CheckCircle2 size={16}/></span>
                <span className="role-copy"><strong>{role.name}</strong>
                  <small>{role.description}</small>
                  <em>{role.permissions.length} permissions</em></span>
              </label>;
            })}</div>
          </fieldset>
          <button className="primary compact" disabled={busy || !selectedRoles.length}><UserPlus size={16}/>사용자 생성</button>
        </form>
      </AdminPanel>
      <AdminPanel title={`${users.length}명 사용자`} action="RBAC · 세션 즉시 반영">
        <div className="user-search"><Users size={17}/><input value={query} onChange={event => setQuery(event.target.value)} placeholder="사용자 검색"/></div>
        <div className="managed-users">{filtered.map(user =>
          <article key={user.id} className={!user.active ? "inactive" : ""}>
            <div className="managed-user-head">
              <div className="avatar">{(user.display_name || user.username)[0].toUpperCase()}</div>
              <div><strong>{user.display_name || user.username}{user.id === currentUserID && <em>현재 사용자</em>}</strong><span>{user.username} · {user.email || "Email 없음"} · {user.provider}</span></div>
              <div className="user-state"><b className={user.active ? "good" : "bad"}>{user.active ? "활성" : "비활성"}</b>{user.locked && <b className="bad">잠김</b>}</div>
            </div>
            <div className="role-pills">{roles.map(role => {
              const local = (user.local_roles || []).includes(role.name);
              const oidc = (user.oidc_roles || []).includes(role.name);
              return <button
                key={role.id}
                className={local || oidc ? "on" : ""}
                disabled={busy || (oidc && !local)}
                onClick={() => updateRoles(user, role.name)}
                title={oidc ? `${role.description} · Keycloak에서 부여됨` : role.description}
              >{role.name}{oidc && <sup>SSO</sup>}</button>;
            })}</div>
            {!!user.oidc_roles?.length && <small className="oidc-role-note">SSO 표시는 Keycloak 클레임에서 동기화되며 이 화면에서 제거할 수 없습니다.</small>}
            <div className="user-actions">
              <button disabled={user.provider === "keycloak"} title={user.provider === "keycloak" ? "프로필은 Keycloak에서 관리" : "정보 수정"} onClick={() => editProfile(user)}><Pencil size={15}/>정보</button>
              <button title={user.active ? "비활성화" : "활성화"} disabled={user.id === currentUserID} onClick={() => toggleActive(user)}><Power size={15}/>{user.active ? "비활성" : "활성"}</button>
              {user.locked && <button onClick={() => unlock(user)}><LockKeyholeOpen size={15}/>잠금 해제</button>}
              <button disabled={user.provider !== "local"} title={user.provider === "local" ? "비밀번호 초기화" : "Keycloak에서 관리"} onClick={() => resetPassword(user)}><Shield size={15}/>비밀번호</button>
              <button className="danger" disabled={user.id === currentUserID} onClick={() => remove(user)}><Trash2 size={15}/>삭제</button>
            </div>
          </article>,
        )}{!filtered.length && <div className="admin-empty">조건에 맞는 사용자가 없습니다.</div>}</div>
      </AdminPanel>
    </div>
  </section>;
}

function AdminPageTitle({kicker, title, subtitle}: {kicker: string; title: string; subtitle: string}) {
  return <div className="page-title"><p className="eyebrow dark">{kicker}</p><h1>{title}</h1><p>{subtitle}</p></div>;
}
function AdminPanel({title, action, children}: {title: string; action: string; children: React.ReactNode}) {
  return <article className="panel"><div className="panel-head"><h3>{title}</h3><span>{action}</span></div>{children}</article>;
}
function StatusItem({label, value, good}: {label: string; value: string; good?: boolean}) {
  return <div><span>{label}</span><strong className={good ? "status-good" : ""}>{value}</strong></div>;
}
function Notice({tone, title, children}: {tone: "info"|"warning"|"error"; title: string; children: React.ReactNode}) {
  return <div className={`admin-notice ${tone}`}><strong>{title}</strong><span>{children}</span></div>;
}
function ActionMessage({message, error}: {message: string; error: string}) {
  if (error) return <div className="error action-message">{error}</div>;
  if (message) return <div className="success action-message"><CheckCircle2 size={17}/>{message}</div>;
  return null;
}
function InfoRow({label, value}: {label: string; value: string}) {
  return <div><div><strong>{label}</strong><span>현재 실행 프로세스 기준</span></div><code>{value}</code></div>;
}
const formatPostgresTarget = (target: PostgresTarget|null|undefined) =>
  target?.valid
    ? `${target.user || "user"}@${target.host || "host"}:${target.port || 5432}/${target.database || ""}`
    : target ? "DSN 오류" : "미설정";
const splitList = (value: string) => value.split(",").map(item => item.trim()).filter(Boolean);
const formatAdminDate = (value: string) => value
  ? new Intl.DateTimeFormat("ko-KR", {year: "2-digit", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit"}).format(new Date(value))
  : "—";
export const formatMappings = (mappings: Record<string, string>) =>
  Object.entries(mappings || {})
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([external, internal]) => `${external}=${internal}`)
    .join("\n");
export const parseMappings = (value: string, label = "매핑") => {
  const mappings: Record<string, string> = {};
  value.split(/\r?\n/).forEach((line, index) => {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) return;
    const separator = trimmed.indexOf("=");
    const external = separator >= 0 ? trimmed.slice(0, separator).trim() : "";
    const internal = separator >= 0 ? trimmed.slice(separator + 1).trim() : "";
    if (!external || !internal) {
      throw new Error(`${label} ${index + 1}행은 '외부 값=InvenQor 역할' 형식이어야 합니다.`);
    }
    if (Object.prototype.hasOwnProperty.call(mappings, external)) {
      throw new Error(`${label} ${index + 1}행의 외부 값 '${external}'이 중복되었습니다.`);
    }
    mappings[external] = internal;
  });
  return mappings;
};
