import React from "react";
import { createRoot } from "react-dom/client";
import {
  Activity, Boxes, ChevronDown, ChevronRight, LayoutDashboard,
  Copy, KeyRound, LogOut, Menu, Palette, RefreshCw, Search, Settings,
  ShieldCheck, Trash2, UserCog, Users, X,
} from "lucide-react";
import { api } from "./api";
import { SettingsPage, UsersPage } from "./adminPages";
import {AccountSecurityPage} from "./accountPage";
import {
  AgentsPage,
  AssetsPage,
  AuditPage,
  OperationsDashboard,
  QueryPage,
} from "./operationsPages";
import { ProductVersion, type SystemInfo } from "./productVersion";
import {PersonalizationPage} from "./personalizationPage";
import {
  applyPreferences,
  defaultPreferences,
  loadPreferences,
  savePreferences,
  type UserPreferences,
} from "./preferences";
import "./styles.css";
import "./personalization.css";

type Page = "dashboard" | "assets" | "agents" | "query" | "settings" | "users" | "keys" | "audit" | "account" | "preferences";
type User = { id: string; display_name: string; username: string; permissions: string[]; super_admin: boolean };
type ApiKey = { id: string; name: string; prefix: string; scopes: string[]; expires_at?: string; last_used_at?: string; revoked_at?: string };
type Scope = { name: string; description: string };
type BootstrapStatus = {required: boolean; token_file?: string};

function BootstrapSetup({onComplete}: {onComplete: () => void}) {
  const [token, setToken] = React.useState("");
  const [username, setUsername] = React.useState("admin");
  const [displayName, setDisplayName] = React.useState("Administrator");
  const [email, setEmail] = React.useState("");
  const [password, setPassword] = React.useState("");
  const [error, setError] = React.useState("");
  const submit = async (event: React.FormEvent) => {
    event.preventDefault(); setError("");
    try {
      await api("/api/v1/bootstrap/admin", {
        method: "POST",
        headers: {"Content-Type": "application/json", "X-Invenqor-Bootstrap-Token": token},
        body: JSON.stringify({username, display_name: displayName, email, password}),
      });
      onComplete();
    } catch (reason) { setError((reason as Error).message); }
  };
  return <main className="login-shell"><section className="login-story">
    <div className="brand-mark">IQ</div><p className="eyebrow">FIRST RUN SECURITY</p>
    <h1>운영을 시작할<br/><em>최초 관리자</em></h1>
    <p>일회성 bootstrap token으로 최고 관리자 계정을 생성합니다. 완료 후 token은 즉시 폐기됩니다.</p>
  </section><section className="login-panel"><form onSubmit={submit}>
    <p className="eyebrow dark">INITIAL SETUP</p><h2>최초 관리자 생성</h2>
    <label>Bootstrap token<input type="password" value={token} onChange={event => setToken(event.target.value)} required/></label>
    <label>관리자 ID<input value={username} onChange={event => setUsername(event.target.value)} required/></label>
    <label>표시 이름<input value={displayName} onChange={event => setDisplayName(event.target.value)}/></label>
    <label>Email<input type="email" value={email} onChange={event => setEmail(event.target.value)}/></label>
    <label>초기 비밀번호<input type="password" value={password} onChange={event => setPassword(event.target.value)}
      autoComplete="new-password" required/></label>
    {error && <div className="error">{error}</div>}
    <button className="primary">관리자 생성 <ChevronRight size={18}/></button>
  </form></section></main>;
}

function Login({ onLogin, systemInfo, keycloakEnabled }: {
  onLogin: (user: User, csrf: string) => void;
  systemInfo: SystemInfo | null;
  keycloakEnabled: boolean;
}) {
  const [username, setUsername] = React.useState("");
  const [password, setPassword] = React.useState("");
  const [totpCode, setTOTPCode] = React.useState("");
  const [error, setError] = React.useState("");
  const submit = async (event: React.FormEvent) => {
    event.preventDefault(); setError("");
    try {
      const result = await api<{user: User; csrf_token: string}>("/api/v1/auth/local/login", {
        method: "POST", headers: {"Content-Type":"application/json"},
        body: JSON.stringify({ username, password, ...(totpCode ? {totp_code: totpCode} : {}) }),
      });
      onLogin(result.user, result.csrf_token);
    } catch (reason) { setError((reason as Error).message); }
  };
  return <main className="login-shell">
    <section className="login-story">
      <div className="brand-mark">IQ</div>
      <p className="eyebrow">ASSET INTELLIGENCE PLATFORM</p>
      <h1>흩어진 인프라를<br/><em>하나의 사실</em>로.</h1>
      <p>서버에서 서비스까지, 발견된 모든 자산의 현재와 변화를 신뢰할 수 있는 단일 화면에서 관리하세요.</p>
      <div className="signal-line"><span/><span/><span/><span/></div>
    </section>
    <section className="login-panel">
      <form onSubmit={submit}>
        <p className="eyebrow dark">SECURE CONSOLE</p><h2>관리 콘솔 로그인</h2>
        <p className="muted">승인된 로컬 계정 또는 조직의 Keycloak 계정을 사용하세요.</p>
        <label>사용자 ID<input value={username} onChange={e=>setUsername(e.target.value)} autoComplete="username" required /></label>
        <label>비밀번호<input type="password" value={password} onChange={e=>setPassword(e.target.value)} autoComplete="current-password" required /></label>
        <label>Authenticator 코드 <small className="optional">TOTP 사용 계정만 입력</small>
          <input inputMode="numeric" pattern="[0-9]{6}" maxLength={6} value={totpCode}
            onChange={e=>setTOTPCode(e.target.value)} autoComplete="one-time-code" placeholder="000000"/></label>
        {error && <div className="error">{error}</div>}
        <button className="primary" type="submit">안전하게 로그인 <ChevronRight size={18}/></button>
        {keycloakEnabled && <a className="sso" href="/api/v1/auth/keycloak/start">Keycloak으로 계속</a>}
        <ProductVersion info={systemInfo}/>
      </form>
    </section>
  </main>;
}

const navigation: {id: Page; label: string; icon: React.ElementType; permission: string}[] = [
  {id:"dashboard",label:"운영 현황",icon:LayoutDashboard,permission:"assets.read"},
  {id:"assets",label:"자산",icon:Boxes,permission:"assets.read"},
  {id:"agents",label:"Agent",icon:Activity,permission:"agents.read"},
  {id:"query",label:"Query DSL",icon:Search,permission:"queries.execute"},
  {id:"settings",label:"설정",icon:Settings,permission:"settings.read"},
  {id:"users",label:"사용자",icon:Users,permission:"users.read"},
  {id:"keys",label:"API · MCP 키",icon:KeyRound,permission:"api_keys.manage"},
  {id:"audit",label:"감사 로그",icon:ShieldCheck,permission:"audit.read"},
  {id:"account",label:"내 보안",icon:UserCog,permission:""},
];

function App() {
  const [user, setUser] = React.useState<User|null>(null);
  const [csrf, setCsrf] = React.useState(sessionStorage.getItem("csrf") || "");
  const [page, setPage] = React.useState<Page>("dashboard");
  const [mobile, setMobile] = React.useState(false);
  const [systemInfo, setSystemInfo] = React.useState<SystemInfo|null>(null);
  const [keycloakEnabled, setKeycloakEnabled] = React.useState(false);
  const [bootstrap, setBootstrap] = React.useState<BootstrapStatus|null>(null);
  const [preferences, setPreferences] = React.useState<UserPreferences>(defaultPreferences);
  React.useEffect(()=>{ api<{user:User}>("/api/v1/auth/me").then(v=>setUser(v.user)).catch(()=>{}); },[]);
  React.useEffect(()=>{ api<SystemInfo>("/api/v1/system/info").then(setSystemInfo).catch(()=>{}); },[]);
  React.useEffect(()=>{ api<{keycloak:boolean}>("/api/v1/auth/methods").then(v=>setKeycloakEnabled(v.keycloak)).catch(()=>{}); },[]);
  React.useEffect(()=>{ api<BootstrapStatus>("/api/v1/bootstrap/status").then(setBootstrap).catch(()=>{}); },[]);
  React.useEffect(() => {
    if (!user) return;
    const loaded = loadPreferences(user.id);
    setPreferences(loaded);
    applyPreferences(loaded);
    const permitted = navigation.some(item =>
      item.id === loaded.start_page &&
      (!item.permission || user.super_admin || user.permissions.includes(item.permission)));
    setPage(permitted ? loaded.start_page as Page : "dashboard");
  }, [user?.id]);
  React.useEffect(() => {
    if (preferences.theme !== "system") return;
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const synchronize = () => applyPreferences(preferences);
    media.addEventListener("change", synchronize);
    return () => media.removeEventListener("change", synchronize);
  }, [preferences]);
  if (bootstrap?.required) return <BootstrapSetup onComplete={() => setBootstrap({required:false})}/>;
  if (!user) return <Login systemInfo={systemInfo} keycloakEnabled={keycloakEnabled} onLogin={(u,c)=>{setUser(u);setCsrf(c);sessionStorage.setItem("csrf",c)}}/>;
  const visibleNavigation=navigation.filter(item=>!item.permission||user.super_admin||user.permissions.includes(item.permission));
  const personalPage=page==="account"||page==="preferences";
  const activePage=personalPage||visibleNavigation.some(item=>item.id===page) ? page : visibleNavigation[0]?.id;
  const updatePreferences=(next:UserPreferences)=>{
    setPreferences(next);savePreferences(user.id,next);applyPreferences(next);
  };
  const logout=async()=>{
    const result=await api<{logout_url?:string}>("/api/v1/auth/logout",{method:"POST",headers:{"X-CSRF-Token":csrf}});
    sessionStorage.clear();
    setUser(null);
    if(result.logout_url) window.location.assign(result.logout_url);
  };
  return <div className="app-shell">
    <aside className={mobile?"sidebar open":"sidebar"}>
      <div className="brand"><div className="brand-mark small">IQ</div><div><strong>INVENQOR</strong><span>CONTROL PLANE</span></div><button className="mobile-close" onClick={()=>setMobile(false)}><X/></button></div>
      <nav>{visibleNavigation.map(item=><button key={item.id} className={activePage===item.id?"active":""} onClick={()=>{setPage(item.id);setMobile(false)}}><item.icon size={19}/>{item.label}</button>)}</nav>
      <div className="user-card"><div className="avatar">{(user.display_name||user.username)[0].toUpperCase()}</div><div><strong>{user.display_name||user.username}</strong><span>{user.super_admin?"최고 관리자":"사용자"}</span></div><button onClick={logout} title="로그아웃"><LogOut size={17}/></button></div>
    </aside>
    <main className="content"><header><button className="menu" onClick={()=>setMobile(true)}><Menu/></button><div><span className="crumb">INVENQOR / {activePage==="preferences"?"개인화":visibleNavigation.find(n=>n.id===activePage)?.label || "접근 권한 없음"}</span></div><div className="header-meta"><ProductVersion info={systemInfo} compact/><div className="live"><i/> LIVE</div>
      <ProfileMenu user={user} onNavigate={setPage} onLogout={logout}/></div></header>
      <PageErrorBoundary key={activePage}>
      {!activePage&&<Empty icon={ShieldCheck} text="이 계정에 콘솔 접근 권한이 없습니다."/>}
      {activePage==="dashboard"&&<OperationsDashboard systemInfo={systemInfo}
        refreshSeconds={preferences.dashboard_refresh_seconds}/>}
      {activePage==="assets"&&<AssetsPage csrf={csrf} access={{permissions:user.permissions,superAdmin:user.super_admin}}/>}
      {activePage==="agents"&&<AgentsPage csrf={csrf} systemInfo={systemInfo}
        access={{permissions:user.permissions,superAdmin:user.super_admin}}/>}
      {activePage==="query"&&<QueryPage csrf={csrf}/>} {activePage==="settings"&&<SettingsPage csrf={csrf} systemInfo={systemInfo}/>}
      {activePage==="users"&&<UsersPage csrf={csrf} currentUserID={user.id}/>}
      {activePage==="keys"&&<ApiKeys csrf={csrf}/>} {activePage==="audit"&&<AuditPage/>}
      {activePage==="account"&&<AccountSecurityPage csrf={csrf}/>}
      {activePage==="preferences"&&<PersonalizationPage preferences={preferences}
        pages={visibleNavigation.filter(item=>item.id!=="account").map(item=>({id:item.id,label:item.label}))}
        onChange={updatePreferences}/>}
      </PageErrorBoundary>
    </main>
  </div>;
}

function ProfileMenu({
  user,
  onNavigate,
  onLogout,
}: {
  user: User;
  onNavigate: (page: Page) => void;
  onLogout: () => Promise<void>;
}) {
  const [open, setOpen] = React.useState(false);
  const container = React.useRef<HTMLDivElement>(null);
  React.useEffect(() => {
    const close = (event: MouseEvent) => {
      if (!container.current?.contains(event.target as Node)) setOpen(false);
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", close);
    document.addEventListener("keydown", escape);
    return () => {
      document.removeEventListener("mousedown", close);
      document.removeEventListener("keydown", escape);
    };
  }, []);
  const navigate = (page: Page) => {
    onNavigate(page);
    setOpen(false);
  };
  return <div className="profile-context" ref={container}>
    <button className="profile-trigger" onClick={() => setOpen(value => !value)}
      aria-haspopup="menu" aria-expanded={open}>
      <span className="avatar">{(user.display_name || user.username)[0].toUpperCase()}</span>
      <span className="profile-trigger-copy"><strong>{user.display_name || user.username}</strong>
        <small>{user.super_admin ? "최고 관리자" : "사용자"}</small></span>
      <ChevronDown size={15}/>
    </button>
    {open && <div className="profile-menu" role="menu">
      <div className="profile-menu-identity"><strong>{user.display_name || user.username}</strong>
        <span>@{user.username}</span></div>
      <button role="menuitem" onClick={() => navigate("account")}><UserCog size={16}/>
        <span><strong>내 계정 보안</strong><small>비밀번호와 TOTP 관리</small></span></button>
      <button role="menuitem" onClick={() => navigate("preferences")}><Palette size={16}/>
        <span><strong>개인화</strong><small>테마·밀도·시작 화면</small></span></button>
      <div className="profile-menu-separator"/>
      <button className="logout" role="menuitem" onClick={() => {setOpen(false); void onLogout();}}>
        <LogOut size={16}/><span><strong>로그아웃</strong><small>현재 세션 종료</small></span></button>
    </div>}
  </div>;
}

class PageErrorBoundary extends React.Component<
  {children: React.ReactNode},
  {error: string}
> {
  state = {error: ""};

  static getDerivedStateFromError(error: Error) {
    return {error: error.message || "화면을 표시하지 못했습니다."};
  }

  componentDidCatch(error: Error) {
    console.error("page_render_failed", error);
  }

  render() {
    if (this.state.error) {
      return <section className="page-failure"><ShieldCheck/><h2>화면을 불러오지 못했습니다.</h2>
        <p>{this.state.error}</p><button className="secondary"
          onClick={() => window.location.reload()}>화면 다시 불러오기</button></section>;
    }
    return this.props.children;
  }
}

function ApiKeys({csrf}:{csrf:string}){
  const [keys,setKeys]=React.useState<ApiKey[]>([]);const [catalog,setCatalog]=React.useState<Scope[]>([]);
  const [name,setName]=React.useState("");const [selected,setSelected]=React.useState<string[]>(["assets.read","mcp.access"]);
  const [expires,setExpires]=React.useState("");const [secret,setSecret]=React.useState("");const [error,setError]=React.useState("");
  const load=React.useCallback(()=>Promise.all([
    api<{api_keys:ApiKey[]}>("/api/v1/admin/api-keys").then(v=>setKeys(v.api_keys)),
    api<{scopes:Scope[]}>("/api/v1/admin/api-key-scopes").then(v=>setCatalog(v.scopes)),
  ]),[]);
  React.useEffect(()=>{load().catch(e=>setError((e as Error).message))},[load]);
  const create=async(e:React.FormEvent)=>{e.preventDefault();setError("");try{const v=await api<{api_key:ApiKey;secret:string}>("/api/v1/admin/api-keys",{method:"POST",headers:{"Content-Type":"application/json","X-CSRF-Token":csrf},body:JSON.stringify({name,scopes:selected,...(expires?{expires_at:new Date(expires).toISOString()}:{})})});setSecret(v.secret);setName("");await load()}catch(reason){setError((reason as Error).message)}};
  const replaceScopes=async(key:ApiKey,scope:string)=>{try{if(key.scopes.includes(scope)){await api(`/api/v1/admin/api-keys/${key.id}/scopes/${encodeURIComponent(scope)}`,{method:"DELETE",headers:{"X-CSRF-Token":csrf}})}else{await api(`/api/v1/admin/api-keys/${key.id}/scopes`,{method:"POST",headers:{"Content-Type":"application/json","X-CSRF-Token":csrf},body:JSON.stringify({scopes:[scope]})})}await load()}catch(reason){setError((reason as Error).message)}};
  const rename=async(key:ApiKey)=>{const name=window.prompt("새 키 이름",key.name);if(!name||name===key.name)return;try{await api(`/api/v1/admin/api-keys/${key.id}`,{method:"PATCH",headers:{"Content-Type":"application/json","X-CSRF-Token":csrf},body:JSON.stringify({name})});await api(`/api/v1/admin/api-keys/${key.id}`);await load()}catch(reason){setError((reason as Error).message)}};
  const rotate=async(key:ApiKey)=>{const value=window.prompt("구 키 유예시간(초, 0~604800)", "3600");if(value===null)return;try{const v=await api<{secret:string}>(`/api/v1/admin/api-keys/${key.id}/rotate`,{method:"POST",headers:{"Content-Type":"application/json","X-CSRF-Token":csrf},body:JSON.stringify({grace_seconds:Number(value)})});setSecret(v.secret);await load()}catch(reason){setError((reason as Error).message)}};
  const revoke=async(key:ApiKey)=>{if(!window.confirm(`${key.name} 키를 즉시 폐기합니까?`))return;try{await api(`/api/v1/admin/api-keys/${key.id}`,{method:"DELETE",headers:{"X-CSRF-Token":csrf}});await load()}catch(reason){setError((reason as Error).message)}};
  return <section><PageTitle kicker="MACHINE IDENTITY" title="API · MCP 키" subtitle="연계 시스템과 AI Agent의 최소권한 키를 생성하고 회전합니다."/>
    {secret&&<div className="secret-reveal"><div><strong>새 Secret — 지금 한 번만 표시됩니다</strong><code>{secret}</code></div><button className="secondary" onClick={()=>navigator.clipboard.writeText(secret)}><Copy size={17}/> 복사</button><button className="secondary" onClick={()=>setSecret("")}><X size={17}/></button></div>}
    {error&&<div className="error">{error}</div>}
    <div className="key-layout"><Panel title="새 키 발급" action="원문 비저장"><form className="key-form" onSubmit={create}><label>키 이름<input value={name} onChange={e=>setName(e.target.value)} placeholder="예: cmdb-readonly" required/></label><label>만료 시각<input type="datetime-local" value={expires} onChange={e=>setExpires(e.target.value)}/></label><fieldset><legend>Scope</legend>{catalog.map(scope=><label className="scope-check" key={scope.name}><input type="checkbox" checked={selected.includes(scope.name)} onChange={()=>setSelected(selected.includes(scope.name)?selected.filter(v=>v!==scope.name):[...selected,scope.name])}/><span><strong>{scope.name}</strong><small>{scope.description}</small></span></label>)}</fieldset><button className="primary compact" disabled={!selected.length}>키 생성</button></form></Panel>
    <Panel title={`${keys.filter(k=>!k.revoked_at).length}개 활성 키`} action="scope 즉시 반영"><div className="key-list">{keys.map(key=><article className={key.revoked_at?"revoked":""} key={key.id}><div className="key-head"><div><KeyRound size={19}/><span><strong>{key.name}</strong><small>ivq_sk_{key.prefix}_… · 최근 사용 {formatDate(key.last_used_at||"")}</small></span></div><div><button title="이름 변경" onClick={()=>rename(key)} disabled={!!key.revoked_at}>이름</button><button title="회전" onClick={()=>rotate(key)} disabled={!!key.revoked_at}><RefreshCw size={16}/></button><button title="폐기" onClick={()=>revoke(key)} disabled={!!key.revoked_at}><Trash2 size={16}/></button></div></div><div className="scope-pills">{catalog.map(scope=><button key={scope.name} className={key.scopes.includes(scope.name)?"on":""} disabled={!!key.revoked_at} onClick={()=>replaceScopes(key,scope.name)}>{scope.name}</button>)}</div>{key.expires_at&&<small>만료 {formatDate(key.expires_at)}</small>}</article>)}</div></Panel></div>
  </section>
}
function PageTitle({kicker,title,subtitle}:{kicker:string;title:string;subtitle:string}){return <div className="page-title"><p className="eyebrow dark">{kicker}</p><h1>{title}</h1><p>{subtitle}</p></div>}
function Panel({title,action,children}:{title:string;action:string;children:React.ReactNode}){return <article className="panel"><div className="panel-head"><h3>{title}</h3><span>{action}</span></div>{children}</article>}
function Empty({icon:Icon,text}:{icon:React.ElementType;text:string}){return <div className="empty"><Icon/><p>{text}</p></div>}
const formatDate=(value:string)=>value?new Intl.DateTimeFormat("ko-KR",{month:"2-digit",day:"2-digit",hour:"2-digit",minute:"2-digit"}).format(new Date(value)):"—";
createRoot(document.getElementById("root")!).render(<React.StrictMode><App/></React.StrictMode>);
