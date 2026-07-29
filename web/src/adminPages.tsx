import React from "react";
import {
  CheckCircle2,
  Database,
  Eye,
  EyeOff,
  KeyRound,
  LockKeyholeOpen,
  Pencil,
  Power,
  RefreshCw,
  Save,
  ServerCog,
  Shield,
  Trash2,
  UserPlus,
  Users,
} from "lucide-react";
import { api } from "./api";
import type { SystemInfo } from "./productVersion";

type SettingsTab = "postgresql" | "keycloak" | "system";
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
}: {
  csrf: string;
  systemInfo: SystemInfo | null;
}) {
  const [tab, setTab] = React.useState<SettingsTab>("postgresql");
  return <section>
    <AdminPageTitle
      kicker="CONTROL CENTER"
      title="운영 설정"
      subtitle="실제로 적용되는 데이터베이스와 인증 설정을 검증하고 관리합니다."
    />
    <div className="settings-layout">
      <div className="settings-nav">
        <button className={tab === "postgresql" ? "active" : ""} onClick={() => setTab("postgresql")}><Database size={17}/>PostgreSQL</button>
        <button className={tab === "keycloak" ? "active" : ""} onClick={() => setTab("keycloak")}><KeyRound size={17}/>Keycloak</button>
        <button className={tab === "system" ? "active" : ""} onClick={() => setTab("system")}><ServerCog size={17}/>시스템 정보</button>
      </div>
      <div className="settings-content">
        {tab === "postgresql" && <PostgresSettings csrf={csrf}/>}
        {tab === "keycloak" && <KeycloakSettingsPanel csrf={csrf}/>}
        {tab === "system" && <SystemSettingsInfo info={systemInfo}/>}
      </div>
    </div>
  </section>;
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
      setSettings(value.settings);
      setSecretConfigured(value.client_secret_configured);
      setRoleMappings(formatMappings(value.settings.role_mappings));
      setGroupMappings(formatMappings(value.settings.group_mappings));
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
  return <AdminPanel title="Keycloak OIDC" action={secretConfigured ? "Client Secret 설정됨" : "Client Secret 미설정"}>
    <div className="settings-body">
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
        <label className="wide">Client Secret<input type="password" value={clientSecret} onChange={event => setClientSecret(event.target.value)} placeholder={secretConfigured ? "변경할 때만 입력" : "필수 Secret"} autoComplete="new-password"/></label>
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

function SystemSettingsInfo({info}: {info: SystemInfo|null}) {
  return <AdminPanel title="실행 정보" action="읽기 전용">
    <div className="setting-list">
      <InfoRow label="Server 버전" value={info?.server_version || "확인 중"}/>
      <InfoRow label="Commit" value={info?.commit || "unknown"}/>
      <InfoRow label="Build time" value={info?.build_time || "unknown"}/>
      <InfoRow label="Database mode" value={info?.database_mode || "확인 중"}/>
      <InfoRow label="서비스 포트" value="7070"/>
    </div>
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
          <fieldset><legend>역할</legend>{roles.map(role =>
            <label className="role-check" key={role.id} title={role.permissions.join(", ")}>
              <input type="checkbox" checked={selectedRoles.includes(role.name)} onChange={() => setSelectedRoles(selectedRoles.includes(role.name) ? selectedRoles.filter(value => value !== role.name) : [...selectedRoles, role.name])}/>
              <span><strong>{role.name}</strong><small>{role.description}</small></span>
            </label>,
          )}</fieldset>
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
