// Silent SSO: a visitor who is already signed in at Keycloak should reach the
// console without seeing the login screen. The Server starts the flow with
// prompt=none, which never renders a screen - the provider either answers with
// a code at once or sends back error=login_required. That refusal is an
// ordinary answer, and the whole difficulty is never reacting to it by trying
// again: doing so bounces the browser between the provider and the console
// forever, and the user only sees the screen flicker.
//
// Three independent guards stop that loop:
//   1. one attempt per tab session, remembered in sessionStorage;
//   2. no attempt after the user signed out on purpose;
//   3. the callback lands a refusal on /?sso=none, so the address itself says
//      "do not try again" even when the browser storage was wiped in between.
// When the storage cannot be read at all the answer is "already attempted":
// private modes and blocked site data throw, and reading that as "not yet"
// would be the loop.

// sessionStorage, not localStorage: a fresh tab tries again, a reload after a
// refusal does not.
const ATTEMPTED_KEY = "invenqor.sso.silentAttempted";
const SIGNED_OUT_KEY = "invenqor.sso.signedOut";

export type SilentSsoStorage = Pick<Storage, "getItem" | "setItem" | "removeItem">;

// Browser navigations only. These prefixes are answered by the Server, not by
// the console, so a silent attempt from them could never come back to a page.
const nonPagePrefixes = ["/api/", "/v1/", "/health", "/mcp", "/momento"];

export type SilentSsoInput = {
  /** keycloak_auto_login from /api/v1/auth/methods. */
  autoLogin: boolean;
  /** window.location.pathname */
  pathname: string;
  /** window.location.search */
  search: string;
  /** Returns the tab's sessionStorage; throwing counts as "already attempted". */
  storage: () => SilentSsoStorage;
};

const readFlag = (storage: () => SilentSsoStorage, key: string): boolean => {
  try {
    return storage().getItem(key) === "true";
  } catch {
    // Fail closed: an unreadable store is treated as an attempt already made.
    return true;
  }
};

const writeFlag = (storage: () => SilentSsoStorage, key: string, value: boolean) => {
  try {
    if (value) storage().setItem(key, "true");
    else storage().removeItem(key);
  } catch {
    /* readFlag already fails closed, so nothing is lost here */
  }
};

const browserStorage = (): SilentSsoStorage => window.sessionStorage;

/** A return target is kept only when it stays on this origin. */
export const safeReturnTo = (value: string): string =>
  value.startsWith("/") && !value.startsWith("//") && !value.includes("\\") ? value : "/";

/** Where to come back to after a silent login: the page the visitor opened. */
export const currentReturnTo = (
  location: Pick<Location, "pathname" | "search" | "hash"> = window.location,
): string => safeReturnTo(location.pathname + location.search + location.hash);

/**
 * Decides whether to try signing in without showing the login screen. The
 * decision is pure so the guards can be tested without a browser.
 */
export const shouldAttemptSilentSso = (input: SilentSsoInput): boolean => {
  if (!input.autoLogin) return false;
  if (nonPagePrefixes.some(prefix => input.pathname.startsWith(prefix))) return false;
  const parameters = new URLSearchParams(input.search);
  // The callback leaves sso=none after a refusal; auth_error means a login
  // just failed. Neither address is a place to start another attempt from.
  if (parameters.has("sso") || parameters.has("auth_error")) return false;
  if (readFlag(input.storage, SIGNED_OUT_KEY)) return false;
  if (readFlag(input.storage, ATTEMPTED_KEY)) return false;
  return true;
};

/** Marks this tab session as attempted and returns the Server URL to visit. */
export const silentSsoStartURL = (
  returnTo: string,
  storage: () => SilentSsoStorage = browserStorage,
): string => {
  writeFlag(storage, ATTEMPTED_KEY, true);
  return `/api/v1/auth/keycloak/start?prompt=none&return_to=${encodeURIComponent(safeReturnTo(returnTo))}`;
};

/** Sends the browser to the provider as a top-level navigation - not a hidden
 *  iframe, so third-party cookie blocking and frame policies do not matter. */
export const beginSilentSso = (returnTo: string) => {
  window.location.assign(silentSsoStartURL(returnTo));
};

/** Records that the user signed out on purpose, which suppresses auto-login
 *  until a session exists again. */
export const markSignedOut = (storage: () => SilentSsoStorage = browserStorage) => {
  writeFlag(storage, SIGNED_OUT_KEY, true);
  writeFlag(storage, ATTEMPTED_KEY, true);
};

/** Clears the guards once a session exists, so the next signed-out visit in
 *  this tab may try again. */
export const clearSilentSsoState = (storage: () => SilentSsoStorage = browserStorage) => {
  writeFlag(storage, SIGNED_OUT_KEY, false);
  writeFlag(storage, ATTEMPTED_KEY, false);
};

export const silentSsoAttempted = (storage: () => SilentSsoStorage = browserStorage): boolean =>
  readFlag(storage, ATTEMPTED_KEY);

/** Decides from the live browser state. */
export const shouldAttemptSilentSsoHere = (autoLogin: boolean): boolean =>
  shouldAttemptSilentSso({
    autoLogin,
    pathname: window.location.pathname,
    search: window.location.search,
    storage: browserStorage,
  });
