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

export const api = async <T,>(
  path: string,
  init?: RequestInit,
): Promise<T> => {
  const response = await fetch(path, { credentials: "include", ...init });
  const text = await response.text();
  const body = text ? JSON.parse(text) : {};
  if (!response.ok) {
    throw new APIError(
      body?.error?.message ||
        body?.failure?.summary ||
        "요청을 처리하지 못했습니다.",
      response.status,
      body,
    );
  }
  return body;
};
