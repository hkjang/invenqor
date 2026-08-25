export class APIError extends Error {
  readonly status: number;
  readonly body: unknown;

  constructor(message: string, status: number, body: unknown) {
    super(message);
    this.name = "APIError";
    this.status = status;
    this.body = body;
  }
}

export const csrfTokenFromCookie = (): string => {
  const encoded = document.cookie
    .split(";")
    .map(value => value.trim())
    .find(value => value.startsWith("invenqor_csrf="))
    ?.slice("invenqor_csrf=".length);
  return encoded ? decodeURIComponent(encoded) : "";
};

export const api = async <T,>(
  path: string,
  init?: RequestInit,
): Promise<T> => {
  const headers = new Headers(init?.headers);
  const method = (init?.method || "GET").toUpperCase();
  if (!["GET", "HEAD", "OPTIONS"].includes(method)) {
    // The cookie belongs to the currently authenticated session. OIDC can
    // replace that session during a redirect while an older token remains in
    // React/sessionStorage, so a fresh cookie must override a stale caller
    // header instead of being used only when the header is absent.
    const csrf = csrfTokenFromCookie();
    if (csrf) headers.set("X-CSRF-Token", csrf);
  }
  const response = await fetch(path, {
    credentials: "include",
    ...init,
    headers,
  });
  const text = await response.text();
  let body: unknown = {};
  let invalidJSON = false;
  if (text.trim()) {
    try {
      body = JSON.parse(text);
    } catch {
      body = text;
      invalidJSON = true;
    }
  }
  if (!response.ok) {
    const payload = body && typeof body === "object"
      ? body as {
        error?: {message?: string};
        failure?: {summary?: string};
        request_id?: string;
      }
      : undefined;
    const message = payload?.error?.message ||
      payload?.failure?.summary ||
      `서버 요청에 실패했습니다. (HTTP ${response.status}) API/Ingress 경로와 Server 로그를 확인하십시오.`;
    // The Server puts a request ID on every error, and the console's own
    // guidance tells the reader to search 진단 로그 for it. It was never shown:
    // every caller displays (reason as Error).message and nothing reads
    // APIError.body, so the id was carried and dropped. Appending it here
    // reaches all of them without touching a single call site.
    const requestID = typeof payload?.request_id === "string"
      ? payload.request_id.trim()
      : "";
    throw new APIError(
      requestID ? `${message} (요청 ID: ${requestID})` : message,
      response.status,
      body,
    );
  }
  if (invalidJSON) {
    throw new APIError(
      "Server가 JSON이 아닌 응답을 반환했습니다. API/Ingress 경로 설정을 확인하십시오.",
      response.status,
      body,
    );
  }
  return body as T;
};
