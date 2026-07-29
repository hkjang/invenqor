import React from "react";
import { createRoot } from "react-dom/client";
import {
  Activity, Boxes, ChevronRight, Database, FileSearch, LayoutDashboard,
  Copy, KeyRound, LogOut, Menu, RefreshCw, Search, Settings, ShieldCheck,
  Trash2, X,
} from "lucide-react";
import "./styles.css";

type Page = "dashboard" | "assets" | "agents" | "query" | "settings" | "keys" | "audit";
type User = { display_name: string; username: string; permissions: string[]; super_admin: boolean };
type SystemInfo = { database_mode: string; server_version: string };
type Asset = { id: string; name: string; type: string; status: string; criticality: string; environment: string; last_seen_at: string };
type Agent = { id: string; agent_id: string; hostname: string; status: string; version: string; os_name: string };
type ApiKey = { id: string; name: string; prefix: string; scopes: string[]; expires_at?: string; last_used_at?: string; revoked_at?: string };
type Scope = { name: string; description: string };

const api = async <T,>(path: string, init?: RequestInit): Promise<T> => {
  const response = await fetch(path, { credentials: "include", ...init });
  const text = await response.text();
  const body = text ? JSON.parse(text) : {};
  if (!response.ok) throw new Error(body?.error?.message || "요청을 처리하지 못했습니다.");
  return body;
};

function Login({ onLogin }: { onLogin: (user: User, csrf: string) => void }) {
  const [username, setUsername] = React.useState("");
  const [password, setPassword] = React.useState("");
  const [error, setError] = React.useState("");
  const submit = async (event: React.FormEvent) => {
    event.preventDefault(); setError("");
    try {
      const result = await api<{user: User; csrf_token: string}>("/api/v1/auth/local/login", {
        method: "POST", headers: {"Content-Type":"application/json"},
        body: JSON.stringify({ username, password }),
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
        {error && <div className="error">{error}</div>}
        <button className="primary" type="submit">안전하게 로그인 <ChevronRight size={18}/></button>
        <a className="sso" href="/api/v1/auth/keycloak/start">Keycloak으로 계속</a>
      </form>
    </section>
  </main>;
}

const navigation: {id: Page; label: string; icon: React.ElementType}[] = [
  {id:"dashboard",label:"운영 현황",icon:LayoutDashboard},{id:"assets",label:"자산",icon:Boxes},
  {id:"agents",label:"Agent",icon:Activity},{id:"query",label:"Query DSL",icon:Search},
  {id:"settings",label:"설정",icon:Settings},{id:"keys",label:"API · MCP 키",icon:KeyRound},
  {id:"audit",label:"감사 로그",icon:ShieldCheck},
];

function App() {
  const [user, setUser] = React.useState<User|null>(null);
  const [csrf, setCsrf] = React.useState(sessionStorage.getItem("csrf") || "");
  const [page, setPage] = React.useState<Page>("dashboard");
  const [mobile, setMobile] = React.useState(false);
  React.useEffect(()=>{ api<{user:User}>("/api/v1/auth/me").then(v=>setUser(v.user)).catch(()=>{}); },[]);
  if (!user) return <Login onLogin={(u,c)=>{setUser(u);setCsrf(c);sessionStorage.setItem("csrf",c)}}/>;
  const logout=async()=>{await api("/api/v1/auth/logout",{method:"POST",headers:{"X-CSRF-Token":csrf}});sessionStorage.clear();setUser(null)};
  return <div className="app-shell">
    <aside className={mobile?"sidebar open":"sidebar"}>
      <div className="brand"><div className="brand-mark small">IQ</div><div><strong>INVENQOR</strong><span>CONTROL PLANE</span></div><button className="mobile-close" onClick={()=>setMobile(false)}><X/></button></div>
      <nav>{navigation.map(item=><button key={item.id} className={page===item.id?"active":""} onClick={()=>{setPage(item.id);setMobile(false)}}><item.icon size={19}/>{item.label}</button>)}</nav>
      <div className="user-card"><div className="avatar">{(user.display_name||user.username)[0].toUpperCase()}</div><div><strong>{user.display_name||user.username}</strong><span>{user.super_admin?"최고 관리자":"사용자"}</span></div><button onClick={logout} title="로그아웃"><LogOut size={17}/></button></div>
    </aside>
    <main className="content"><header><button className="menu" onClick={()=>setMobile(true)}><Menu/></button><div><span className="crumb">INVENQOR / {navigation.find(n=>n.id===page)?.label}</span></div><div className="live"><i/> LIVE</div></header>
      {page==="dashboard"&&<Dashboard/>}{page==="assets"&&<Assets/>}{page==="agents"&&<Agents/>}
      {page==="query"&&<Query csrf={csrf}/>} {page==="settings"&&<SettingsPage csrf={csrf}/>}
      {page==="keys"&&<ApiKeys csrf={csrf}/>} {page==="audit"&&<Audit/>}
    </main>
  </div>;
}

function Dashboard(){
  const [info,setInfo]=React.useState<SystemInfo|null>(null); const [assets,setAssets]=React.useState<Asset[]>([]); const [agents,setAgents]=React.useState<Agent[]>([]);
  React.useEffect(()=>{api<SystemInfo>("/api/v1/system/info").then(setInfo);api<{items:Asset[]}>("/api/v1/assets?limit=6").then(v=>setAssets(v.items));api<{agents:Agent[]}>("/api/v1/admin/agents").then(v=>setAgents(v.agents));},[]);
  const active=agents.filter(a=>a.status==="active").length;
  return <section><PageTitle kicker="COMMAND OVERVIEW" title="운영 현황" subtitle="인프라 자산과 수집 상태를 실시간으로 확인합니다."/>
    {info?.database_mode==="SQLITE_FALLBACK"&&<div className="warning"><Database size={20}/><div><strong>SQLite 대체 모드</strong><span>PostgreSQL이 구성되지 않았습니다. 설정에서 운영 DB 연결을 준비하세요.</span></div></div>}
    <div className="metrics"><Metric label="전체 자산" value={assets.length.toLocaleString()} note="현재 조회 범위" icon={Boxes}/><Metric label="활성 Agent" value={`${active}/${agents.length}`} note="정상 수집 중" icon={Activity}/><Metric label="DB 모드" value={info?.database_mode.replace("_"," ")||"확인 중"} note={`Server ${info?.server_version||"—"}`} icon={Database}/><Metric label="보안 상태" value="정상" note="RBAC · CSRF · 암호화" icon={ShieldCheck}/></div>
    <div className="dashboard-grid"><Panel title="최근 확인 자산" action="자산 인벤토리"><AssetTable items={assets}/></Panel><Panel title="Agent 상태" action="수집 노드"><div className="agent-list">{agents.slice(0,6).map(a=><div key={a.id}><i className={a.status==="active"?"ok":""}/><div><strong>{a.hostname||a.agent_id}</strong><span>{a.os_name||"운영체제 확인 전"}</span></div><b>{a.status}</b></div>)}</div></Panel></div>
  </section>;
}
function Assets(){const [items,setItems]=React.useState<Asset[]>([]);const [q,setQ]=React.useState("");React.useEffect(()=>{const timer=setTimeout(()=>api<{items:Asset[]}>(`/api/v1/assets?limit=100&q=${encodeURIComponent(q)}`).then(v=>setItems(v.items)),180);return()=>clearTimeout(timer)},[q]);return <section><PageTitle kicker="CONFIGURATION ITEMS" title="자산 인벤토리" subtitle="원천 데이터를 정규화한 대표 자산과 최신 상태입니다."/><div className="toolbar"><div className="search"><Search size={18}/><input placeholder="이름 또는 자산 키 검색" value={q} onChange={e=>setQ(e.target.value)}/></div><button className="secondary">필터</button><button className="primary compact">자산 등록</button></div><Panel title={`${items.length}개 자산`} action="최근 확인 순"><AssetTable items={items}/></Panel></section>}
function AssetTable({items}:{items:Asset[]}){return <div className="table-wrap"><table><thead><tr><th>자산</th><th>유형</th><th>환경</th><th>중요도</th><th>상태</th><th>최근 확인</th></tr></thead><tbody>{items.map(a=><tr key={a.id}><td><strong>{a.name}</strong><span>{a.id.slice(0,13)}…</span></td><td>{a.type}</td><td>{a.environment}</td><td>{a.criticality}</td><td><Badge value={a.status}/></td><td>{formatDate(a.last_seen_at)}</td></tr>)}</tbody></table>{!items.length&&<Empty icon={Boxes} text="표시할 자산이 없습니다."/>}</div>}
function Agents(){const [items,setItems]=React.useState<Agent[]>([]);React.useEffect(()=>{api<{agents:Agent[]}>("/api/v1/admin/agents").then(v=>setItems(v.agents))},[]);return <section><PageTitle kicker="COLLECTION FLEET" title="Agent 관리" subtitle="등록 자격 증명과 수집 노드의 상태를 관리합니다."/><div className="card-grid">{items.map(a=><article className="agent-card" key={a.id}><div className="host-icon"><Activity/></div><Badge value={a.status}/><h3>{a.hostname||"이름 없는 Agent"}</h3><p>{a.agent_id}</p><dl><dt>버전</dt><dd>{a.version||"—"}</dd><dt>운영체제</dt><dd>{a.os_name||"—"}</dd></dl></article>)}{!items.length&&<Empty icon={Activity} text="등록된 Agent가 없습니다."/>}</div></section>}
function Query({csrf}:{csrf:string}){const [query,setQuery]=React.useState('type = "host" AND environment = "production"');const [result,setResult]=React.useState<Asset[]>([]);const [error,setError]=React.useState("");const run=async()=>{try{setError("");const v=await api<{items:Asset[]}>("/api/v1/query/execute",{method:"POST",headers:{"Content-Type":"application/json","X-CSRF-Token":csrf},body:JSON.stringify({query})});setResult(v.items)}catch(e){setError((e as Error).message)}};return <section><PageTitle kicker="SAFE DISCOVERY" title="Query DSL" subtitle="허용된 자산 필드만 사용해 안전하게 인벤토리를 탐색합니다."/><div className="query-box"><div className="query-head"><FileSearch size={20}/><strong>질의 편집기</strong><span>SQL 실행 불가 · 최대 500건</span></div><textarea value={query} onChange={e=>setQuery(e.target.value)} spellCheck={false}/><div className="query-actions">{error&&<span className="error-text">{error}</span>}<button className="primary compact" onClick={run}>질의 실행</button></div></div><Panel title={`결과 ${result.length}건`} action="바인딩 파라미터 적용"><AssetTable items={result}/></Panel></section>}
function SettingsPage({csrf}:{csrf:string}){const [items,setItems]=React.useState<{key:string;value:unknown;apply_mode:string;secret:boolean}[]>([]);React.useEffect(()=>{api<{items:typeof items}>("/api/v1/admin/settings").then(v=>setItems(v.items))},[]);return <section><PageTitle kicker="CONTROL CENTER" title="운영 설정" subtitle="인증, 수집, 데이터베이스, 보존 정책을 한 곳에서 관리합니다."/><div className="settings-layout"><div className="settings-nav">{["일반","PostgreSQL","인증","Agent","자산","수집","보안","백업"].map((v,i)=><button className={i===0?"active":""} key={v}><Settings size={17}/>{v}</button>)}</div><Panel title="현재 적용 설정" action="비밀값 자동 마스킹"><div className="setting-list">{items.map(item=><div key={item.key}><div><strong>{item.key}</strong><span>{item.apply_mode} · {item.secret?"암호화됨":"일반 값"}</span></div><code>{item.secret?"••••••••":JSON.stringify(item.value)}</code></div>)}{!items.length&&<Empty icon={Settings} text="저장된 사용자 설정이 없습니다. 제품 기본값으로 실행 중입니다."/>}</div></Panel></div></section>}
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
  const replaceScopes=async(key:ApiKey,scope:string)=>{const scopes=key.scopes.includes(scope)?key.scopes.filter(v=>v!==scope):[...key.scopes,scope];try{await api(`/api/v1/admin/api-keys/${key.id}`,{method:"PATCH",headers:{"Content-Type":"application/json","X-CSRF-Token":csrf},body:JSON.stringify({scopes})});await load()}catch(reason){setError((reason as Error).message)}};
  const rotate=async(key:ApiKey)=>{const value=window.prompt("구 키 유예시간(초, 0~604800)", "3600");if(value===null)return;try{const v=await api<{secret:string}>(`/api/v1/admin/api-keys/${key.id}/rotate`,{method:"POST",headers:{"Content-Type":"application/json","X-CSRF-Token":csrf},body:JSON.stringify({grace_seconds:Number(value)})});setSecret(v.secret);await load()}catch(reason){setError((reason as Error).message)}};
  const revoke=async(key:ApiKey)=>{if(!window.confirm(`${key.name} 키를 즉시 폐기합니까?`))return;try{await api(`/api/v1/admin/api-keys/${key.id}`,{method:"DELETE",headers:{"X-CSRF-Token":csrf}});await load()}catch(reason){setError((reason as Error).message)}};
  return <section><PageTitle kicker="MACHINE IDENTITY" title="API · MCP 키" subtitle="연계 시스템과 AI Agent의 최소권한 키를 생성하고 회전합니다."/>
    {secret&&<div className="secret-reveal"><div><strong>새 Secret — 지금 한 번만 표시됩니다</strong><code>{secret}</code></div><button className="secondary" onClick={()=>navigator.clipboard.writeText(secret)}><Copy size={17}/> 복사</button><button className="secondary" onClick={()=>setSecret("")}><X size={17}/></button></div>}
    {error&&<div className="error">{error}</div>}
    <div className="key-layout"><Panel title="새 키 발급" action="원문 비저장"><form className="key-form" onSubmit={create}><label>키 이름<input value={name} onChange={e=>setName(e.target.value)} placeholder="예: cmdb-readonly" required/></label><label>만료 시각<input type="datetime-local" value={expires} onChange={e=>setExpires(e.target.value)}/></label><fieldset><legend>Scope</legend>{catalog.map(scope=><label className="scope-check" key={scope.name}><input type="checkbox" checked={selected.includes(scope.name)} onChange={()=>setSelected(selected.includes(scope.name)?selected.filter(v=>v!==scope.name):[...selected,scope.name])}/><span><strong>{scope.name}</strong><small>{scope.description}</small></span></label>)}</fieldset><button className="primary compact" disabled={!selected.length}>키 생성</button></form></Panel>
    <Panel title={`${keys.filter(k=>!k.revoked_at).length}개 활성 키`} action="scope 즉시 반영"><div className="key-list">{keys.map(key=><article className={key.revoked_at?"revoked":""} key={key.id}><div className="key-head"><div><KeyRound size={19}/><span><strong>{key.name}</strong><small>ivq_sk_{key.prefix}_… · 최근 사용 {formatDate(key.last_used_at||"")}</small></span></div><div><button title="회전" onClick={()=>rotate(key)} disabled={!!key.revoked_at}><RefreshCw size={16}/></button><button title="폐기" onClick={()=>revoke(key)} disabled={!!key.revoked_at}><Trash2 size={16}/></button></div></div><div className="scope-pills">{catalog.map(scope=><button key={scope.name} className={key.scopes.includes(scope.name)?"on":""} disabled={!!key.revoked_at} onClick={()=>replaceScopes(key,scope.name)}>{scope.name}</button>)}</div>{key.expires_at&&<small>만료 {formatDate(key.expires_at)}</small>}</article>)}</div></Panel></div>
  </section>
}
function Audit(){const [items,setItems]=React.useState<{id:string;occurred_at:string;action:string;actor_name:string;resource_type:string;result:string}[]>([]);React.useEffect(()=>{api<{items:typeof items}>("/api/v1/admin/audit?limit=200").then(v=>setItems(v.items))},[]);return <section><PageTitle kicker="ACCOUNTABILITY" title="감사 로그" subtitle="인증과 관리자 변경 작업의 추적 가능한 기록입니다."/><Panel title={`${items.length}개 이벤트`} action="최신 순"><div className="timeline">{items.map(i=><div key={i.id}><i/><time>{formatDate(i.occurred_at)}</time><div><strong>{i.action}</strong><span>{i.actor_name||"system"} · {i.resource_type}</span></div><Badge value={i.result}/></div>)}</div></Panel></section>}
function PageTitle({kicker,title,subtitle}:{kicker:string;title:string;subtitle:string}){return <div className="page-title"><p className="eyebrow dark">{kicker}</p><h1>{title}</h1><p>{subtitle}</p></div>}
function Metric({label,value,note,icon:Icon}:{label:string;value:string;note:string;icon:React.ElementType}){return <article className="metric"><div><span>{label}</span><strong>{value}</strong><small>{note}</small></div><Icon/></article>}
function Panel({title,action,children}:{title:string;action:string;children:React.ReactNode}){return <article className="panel"><div className="panel-head"><h3>{title}</h3><span>{action}</span></div>{children}</article>}
function Badge({value}:{value:string}){return <span className={`badge ${["active","success","processed","정상"].includes(value)?"good":value==="blocked"||value==="failed"?"bad":""}`}>{value}</span>}
function Empty({icon:Icon,text}:{icon:React.ElementType;text:string}){return <div className="empty"><Icon/><p>{text}</p></div>}
const formatDate=(value:string)=>value?new Intl.DateTimeFormat("ko-KR",{month:"2-digit",day:"2-digit",hour:"2-digit",minute:"2-digit"}).format(new Date(value)):"—";
createRoot(document.getElementById("root")!).render(<React.StrictMode><App/></React.StrictMode>);
