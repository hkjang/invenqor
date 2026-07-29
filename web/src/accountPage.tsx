import React from "react";
import {CheckCircle2, Copy, KeyRound, LockKeyhole, ShieldCheck} from "lucide-react";
import {api} from "./api";

const jsonRequest = (csrf: string, body: unknown, method = "POST"): RequestInit => ({
  method,
  headers: {"Content-Type": "application/json", "X-CSRF-Token": csrf},
  body: JSON.stringify(body),
});

type TOTPSetup = {
  secret: string;
  provisioning_uri: string;
  recovery_codes: string[];
};

export function AccountSecurityPage({csrf}: {csrf: string}) {
  const [currentPassword, setCurrentPassword] = React.useState("");
  const [newPassword, setNewPassword] = React.useState("");
  const [confirmPassword, setConfirmPassword] = React.useState("");
  const [setup, setSetup] = React.useState<TOTPSetup|null>(null);
  const [code, setCode] = React.useState("");
  const [message, setMessage] = React.useState("");
  const [error, setError] = React.useState("");
  const mutate = async (work: () => Promise<unknown>, success: string) => {
    setError(""); setMessage("");
    try { await work(); setMessage(success); } catch (reason) { setError((reason as Error).message); }
  };
  const changePassword = (event: React.FormEvent) => {
    event.preventDefault();
    if (newPassword !== confirmPassword) {
      setError("새 비밀번호 확인 값이 일치하지 않습니다.");
      return;
    }
    mutate(async () => {
      await api("/api/v1/auth/password/change",
        jsonRequest(csrf, {current_password: currentPassword, new_password: newPassword}));
      setCurrentPassword(""); setNewPassword(""); setConfirmPassword("");
    }, "비밀번호를 변경했습니다. 기존 세션 정책에 따라 다시 로그인해야 할 수 있습니다.");
  };
  const beginTOTP = () => mutate(async () => {
    setSetup(await api<TOTPSetup>("/api/v1/auth/totp/setup", jsonRequest(csrf, {})));
  }, "Authenticator 앱에 등록한 뒤 6자리 코드를 확인하십시오.");
  const enableTOTP = () => mutate(async () => {
    await api("/api/v1/auth/totp/enable", jsonRequest(csrf, {code}));
    setSetup(null); setCode("");
  }, "TOTP 다중요소 인증을 활성화했습니다.");
  const disableTOTP = () => {
    const confirmation = window.prompt("현재 Authenticator 6자리 코드를 입력하십시오.");
    if (!confirmation) return;
    mutate(() => api("/api/v1/auth/totp", jsonRequest(csrf, {code: confirmation}, "DELETE")),
      "TOTP 다중요소 인증을 비활성화했습니다.");
  };
  return <section>
    <div className="page-title"><p className="eyebrow dark">PERSONAL SECURITY</p>
      <h1>내 계정 보안</h1><p>비밀번호와 다중요소 인증을 직접 관리합니다.</p></div>
    {error && <div className="error action-message">{error}</div>}
    {message && <div className="success action-message"><CheckCircle2 size={16}/>{message}</div>}
    <div className="account-grid">
      <article className="panel"><div className="panel-head"><h3>비밀번호 변경</h3><span>Argon2id 정책</span></div>
        <form className="compact-form" onSubmit={changePassword}>
          <label>현재 비밀번호<input type="password" autoComplete="current-password"
            value={currentPassword} onChange={event => setCurrentPassword(event.target.value)} required/></label>
          <label>새 비밀번호<input type="password" autoComplete="new-password"
            value={newPassword} onChange={event => setNewPassword(event.target.value)} required/></label>
          <label>새 비밀번호 확인<input type="password" autoComplete="new-password"
            value={confirmPassword} onChange={event => setConfirmPassword(event.target.value)} required/></label>
          <button className="primary compact"><LockKeyhole size={15}/>비밀번호 변경</button>
        </form>
      </article>
      <article className="panel"><div className="panel-head"><h3>TOTP 다중요소 인증</h3><span>RFC 6238</span></div>
        <div className="totp-body">
          {!setup && <><div className="security-illustration"><ShieldCheck/><div><strong>Authenticator 기반 추가 인증</strong>
            <span>로그인 시 비밀번호와 6자리 일회용 코드를 함께 확인합니다.</span></div></div>
            <div className="form-actions"><button className="secondary" onClick={disableTOTP}>TOTP 비활성화</button>
              <button className="primary compact" onClick={beginTOTP}><KeyRound size={15}/>TOTP 설정 시작</button></div></>}
          {setup && <div className="totp-setup">
            <strong>1. Authenticator 앱에 다음 URI 또는 Secret을 등록하십시오.</strong>
            <div className="copy-value"><code>{setup.provisioning_uri}</code>
              <button onClick={() => navigator.clipboard.writeText(setup.provisioning_uri)}><Copy size={15}/></button></div>
            <div className="copy-value"><code>{setup.secret}</code>
              <button onClick={() => navigator.clipboard.writeText(setup.secret)}><Copy size={15}/></button></div>
            <strong>2. 복구 코드를 안전한 오프라인 위치에 보관하십시오.</strong>
            <div className="recovery-codes">{setup.recovery_codes.map(value => <code key={value}>{value}</code>)}</div>
            <strong>3. 표시된 6자리 코드를 입력해 활성화하십시오.</strong>
            <div className="inline-form"><input inputMode="numeric" pattern="[0-9]{6}" maxLength={6}
              value={code} onChange={event => setCode(event.target.value)} placeholder="000000"/>
              <button className="primary compact" disabled={code.length !== 6} onClick={enableTOTP}>활성화</button></div>
          </div>}
        </div>
      </article>
    </div>
  </section>;
}
