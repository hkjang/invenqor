import React from "react";
import {
  CheckCircle2,
  Copy,
  Download,
  KeyRound,
  LockKeyhole,
  RefreshCw,
  ShieldAlert,
  ShieldCheck,
} from "lucide-react";
import {api} from "./api";
import {formatDate} from "./format";
import {qrMatrix, qrPath, qrViewBox} from "./qrCode";

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

export type AccountSecurity = {
  password_configured: boolean;
  totp_enabled: boolean;
  recovery_codes_remaining: number;
  totp_verified_at?: string;
};

const defaultSecurity: AccountSecurity = {
  password_configured: true,
  totp_enabled: false,
  recovery_codes_remaining: 0,
};

// A QR the camera can read, rather than an otpauth:// URI to retype.
function ProvisioningCode({uri}: {uri: string}) {
  const matrix = React.useMemo(() => qrMatrix(uri), [uri]);
  return <svg
    className="totp-qr"
    viewBox={qrViewBox(matrix)}
    role="img"
    aria-label="Authenticator 등록용 QR 코드"
    shapeRendering="crispEdges"
  >
    <rect
      x={-4} y={-4}
      width={matrix.size + 8} height={matrix.size + 8}
      className="totp-qr-quiet"
    />
    <path d={qrPath(matrix)} className="totp-qr-module"/>
  </svg>;
}

function RecoveryCodes({codes}: {codes: string[]}) {
  const asText = codes.join("\n");
  return <>
    <div className="recovery-codes">{codes.map(value =>
      <code key={value}>{value}</code>)}</div>
    <div className="recovery-actions">
      <button className="secondary" onClick={() => navigator.clipboard.writeText(asText)}>
        <Copy size={15}/>모두 복사
      </button>
      <button className="secondary" onClick={() => {
        // A downloaded file is what actually gets stored somewhere safe; codes
        // left on screen are codes that were never written down.
        const blob = new Blob([asText + "\n"], {type: "text/plain;charset=utf-8"});
        const url = URL.createObjectURL(blob);
        const anchor = document.createElement("a");
        anchor.href = url;
        anchor.download = "invenqor-recovery-codes.txt";
        document.body.appendChild(anchor);
        anchor.click();
        anchor.remove();
        URL.revokeObjectURL(url);
      }}><Download size={15}/>파일로 저장</button>
    </div>
  </>;
}

export function AccountSecurityPage({
  csrf,
  security = defaultSecurity,
  onSecurityChange,
}: {
  csrf: string;
  security?: AccountSecurity;
  onSecurityChange?: () => void;
}) {
  const [currentPassword, setCurrentPassword] = React.useState("");
  const [newPassword, setNewPassword] = React.useState("");
  const [confirmPassword, setConfirmPassword] = React.useState("");
  const [setup, setSetup] = React.useState<TOTPSetup|null>(null);
  const [issuedCodes, setIssuedCodes] = React.useState<string[]>([]);
  const [code, setCode] = React.useState("");
  const [message, setMessage] = React.useState("");
  const [error, setError] = React.useState("");
  const mutate = async (work: () => Promise<unknown>, success: string) => {
    setError(""); setMessage("");
    try {
      await work();
      setMessage(success);
      onSecurityChange?.();
    } catch (reason) { setError((reason as Error).message); }
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
    setIssuedCodes([]);
    setSetup(await api<TOTPSetup>("/api/v1/auth/totp/setup", jsonRequest(csrf, {})));
  }, "Authenticator 앱으로 QR을 스캔한 뒤 6자리 코드를 입력하십시오.");
  const enableTOTP = () => mutate(async () => {
    await api("/api/v1/auth/totp/enable", jsonRequest(csrf, {code}));
    setSetup(null); setCode("");
  }, "TOTP 다중요소 인증을 활성화했습니다.");
  const disableTOTP = () => {
    const confirmation = window.prompt(
      "현재 Authenticator 6자리 코드 또는 복구 코드를 입력하십시오.",
    );
    if (!confirmation) return;
    mutate(() => api("/api/v1/auth/totp", jsonRequest(csrf, {code: confirmation}, "DELETE")),
      "TOTP 다중요소 인증을 비활성화했습니다.");
  };
  const regenerate = () => {
    const confirmation = window.prompt(
      "현재 Authenticator 6자리 코드를 입력하십시오. 기존 복구 코드는 모두 무효가 됩니다.",
    );
    if (!confirmation) return;
    mutate(async () => {
      const result = await api<{recovery_codes: string[]}>(
        "/api/v1/auth/totp/recovery-codes", jsonRequest(csrf, {code: confirmation}));
      setIssuedCodes(result.recovery_codes);
    }, "새 복구 코드를 발급했습니다. 기존 코드는 더 이상 사용할 수 없습니다.");
  };
  const codesLow = security.totp_enabled && security.recovery_codes_remaining <= 2;
  return <section>
    <div className="page-title"><p className="eyebrow dark">PERSONAL SECURITY</p>
      <h1>내 계정 보안</h1><p>비밀번호와 다중요소 인증을 직접 관리합니다.</p></div>
    {error && <div className="error action-message">{error}</div>}
    {message && <div className="success action-message"><CheckCircle2 size={16}/>{message}</div>}
    {!security.totp_enabled && !setup && <div className="admin-notice info">
      <strong>다중요소 인증이 설정되어 있지 않습니다.</strong>
      <span>비밀번호가 유출되어도 로그인을 막으려면 Authenticator 앱을 등록하십시오.</span>
    </div>}
    {codesLow && <div className="admin-notice warning">
      <strong>남은 복구 코드가 {security.recovery_codes_remaining}개입니다.</strong>
      <span>Authenticator 기기를 분실하면 로그인할 수 없습니다. 새 복구 코드를 발급하십시오.</span>
    </div>}
    <div className="account-grid">
      <article className="panel"><div className="panel-head"><h3>비밀번호</h3>
        <span>{security.password_configured ? "Argon2id 정책" : "외부 인증"}</span></div>
        {security.password_configured
          ? <form className="compact-form" onSubmit={changePassword}>
            <label>현재 비밀번호<input type="password" autoComplete="current-password"
              value={currentPassword} onChange={event => setCurrentPassword(event.target.value)} required/></label>
            <label>새 비밀번호<input type="password" autoComplete="new-password"
              value={newPassword} onChange={event => setNewPassword(event.target.value)} required/></label>
            <label>새 비밀번호 확인<input type="password" autoComplete="new-password"
              value={confirmPassword} onChange={event => setConfirmPassword(event.target.value)} required/></label>
            <button className="primary compact"><LockKeyhole size={15}/>비밀번호 변경</button>
          </form>
          : <div className="totp-body"><div className="security-illustration">
            {/* A Keycloak account holds no local password. The form used to be
                shown anyway and every attempt failed on the server. */}
            <ShieldAlert/><div><strong>이 계정은 Keycloak으로 로그인합니다.</strong>
              <span>비밀번호는 Keycloak에서 관리하므로 이 화면에서는 변경할 수 없습니다.</span></div>
          </div></div>}
      </article>
      <article className="panel"><div className="panel-head"><h3>TOTP 다중요소 인증</h3>
        <span>{security.totp_enabled ? "사용 중" : "미설정"}</span></div>
        <div className="totp-body">
          {!setup && <>
            <div className={security.totp_enabled
              ? "security-illustration enabled"
              : "security-illustration"}>
              {security.totp_enabled ? <ShieldCheck/> : <KeyRound/>}
              <div>
                <strong>{security.totp_enabled
                  ? "Authenticator 추가 인증이 켜져 있습니다."
                  : "Authenticator 기반 추가 인증"}</strong>
                <span>{security.totp_enabled
                  ? `${security.totp_verified_at
                    ? `${formatDate(security.totp_verified_at)} 등록 · `
                    : ""}복구 코드 ${security.recovery_codes_remaining}개 남음`
                  : "로그인 시 비밀번호와 6자리 일회용 코드를 함께 확인합니다."}</span>
              </div>
            </div>
            {/* Only the action that applies to the current state is offered:
                both buttons used to be shown regardless of it. */}
            <div className="form-actions">
              {security.totp_enabled ? <>
                <button className="secondary" onClick={disableTOTP}>TOTP 비활성화</button>
                <button className="primary compact" onClick={regenerate}>
                  <RefreshCw size={15}/>복구 코드 재발급</button>
              </> : <button className="primary compact" onClick={beginTOTP}>
                <KeyRound size={15}/>TOTP 설정 시작</button>}
            </div>
            {!!issuedCodes.length && <div className="totp-setup">
              <strong>새 복구 코드 — 지금 한 번만 표시됩니다.</strong>
              <RecoveryCodes codes={issuedCodes}/>
            </div>}
          </>}
          {setup && <div className="totp-setup">
            <strong>1. Authenticator 앱으로 아래 QR을 스캔하십시오.</strong>
            <ProvisioningCode uri={setup.provisioning_uri}/>
            <details className="manual-entry"><summary>카메라를 사용할 수 없는 경우 직접 입력</summary>
              <div className="copy-value"><code>{setup.secret}</code>
                <button onClick={() => navigator.clipboard.writeText(setup.secret)}
                  aria-label="Secret 복사"><Copy size={15}/></button></div>
              <div className="copy-value"><code>{setup.provisioning_uri}</code>
                <button onClick={() => navigator.clipboard.writeText(setup.provisioning_uri)}
                  aria-label="등록 URI 복사"><Copy size={15}/></button></div>
            </details>
            <strong>2. 복구 코드를 안전한 오프라인 위치에 보관하십시오.</strong>
            <RecoveryCodes codes={setup.recovery_codes}/>
            <strong>3. 앱에 표시된 6자리 코드를 입력해 활성화하십시오.</strong>
            <div className="inline-form"><input inputMode="numeric" pattern="[0-9]{6}" maxLength={6}
              value={code} onChange={event => setCode(event.target.value.replace(/\D/g, ""))}
              placeholder="000000" aria-label="Authenticator 6자리 코드"/>
              <button className="primary compact" disabled={code.length !== 6} onClick={enableTOTP}>활성화</button>
              <button className="secondary" onClick={() => {setSetup(null); setCode("");}}>취소</button></div>
            <p className="hint">복구 코드는 Authenticator 기기를 분실했을 때 로그인과
              비활성화에 사용합니다. 한 번 사용한 코드는 다시 쓸 수 없습니다.</p>
          </div>}
        </div>
      </article>
    </div>
  </section>;
}
