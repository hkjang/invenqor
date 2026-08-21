export type ConsolePage =
  | "dashboard"
  | "assets"
  | "software"
  | "visualization"
  | "agents"
  | "query"
  | "settings"
  | "users"
  | "keys"
  | "audit"
  | "logs"
  | "account"
  | "preferences";

export type SettingsTab =
  | "postgresql"
  | "agents"
  | "classification"
  | "keycloak"
  | "general"
  | "system";

const consolePages = new Set<ConsolePage>([
  "dashboard",
  "assets",
  "software",
  "visualization",
  "agents",
  "query",
  "settings",
  "users",
  "keys",
  "audit",
  "logs",
  "account",
  "preferences",
]);
const settingsTabs = new Set<SettingsTab>([
  "postgresql",
  "agents",
  "classification",
  "keycloak",
  "general",
  "system",
]);

export const parseConsoleHash = (
  hash: string,
): {page?: ConsolePage; settingsTab?: SettingsTab; query: URLSearchParams} => {
  // A page can carry parameters - an audit row links to the diagnostic log for
  // its request ID as "#/logs?request_id=…". Without splitting the query off,
  // the page name became "logs?request_id=…", matched nothing, and the link
  // silently did not navigate.
  const [route, search = ""] = hash.replace(/^#\/?/, "").split("?");
  const parts = route.split("/").filter(Boolean);
  const page = consolePages.has(parts[0] as ConsolePage)
    ? parts[0] as ConsolePage
    : undefined;
  const settingsTab = page === "settings" &&
    settingsTabs.has(parts[1] as SettingsTab)
    ? parts[1] as SettingsTab
    : undefined;
  return {page, settingsTab, query: new URLSearchParams(search)};
};

/** The parameters carried by the current hash, for a page that accepts them. */
export const consoleHashQuery = (): URLSearchParams =>
  typeof window === "undefined"
    ? new URLSearchParams()
    : parseConsoleHash(window.location.hash).query;

export const consoleHash = (
  page: ConsolePage,
  settingsTab?: SettingsTab,
): string => page === "settings" && settingsTab
  ? `#/settings/${settingsTab}`
  : `#/${page}`;

const pageKey = (userID: string) => `invenqor.navigation.${userID}`;
const settingsKey = (userID: string) => `invenqor.settings.tab.${userID}`;

export const loadLastPage = (userID: string): ConsolePage | undefined => {
  try {
    const value = localStorage.getItem(pageKey(userID)) as ConsolePage | null;
    return value && consolePages.has(value) ? value : undefined;
  } catch {
    return undefined;
  }
};

export const saveLastPage = (userID: string, page: ConsolePage) => {
  try {
    localStorage.setItem(pageKey(userID), page);
  } catch {
    // URL routing still preserves the menu when storage is unavailable.
  }
};

export const loadSettingsTab = (userID: string): SettingsTab => {
  const routed = typeof window === "undefined"
    ? undefined
    : parseConsoleHash(window.location.hash).settingsTab;
  if (routed) return routed;
  try {
    const value = localStorage.getItem(settingsKey(userID)) as SettingsTab | null;
    return value && settingsTabs.has(value) ? value : "postgresql";
  } catch {
    return "postgresql";
  }
};

export const saveSettingsTab = (userID: string, tab: SettingsTab) => {
  try {
    localStorage.setItem(settingsKey(userID), tab);
  } catch {
    // The URL remains the authoritative refresh state.
  }
};
