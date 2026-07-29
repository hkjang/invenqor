export type ConsolePage =
  | "dashboard"
  | "assets"
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
  | "keycloak"
  | "general"
  | "system";

const consolePages = new Set<ConsolePage>([
  "dashboard",
  "assets",
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
  "keycloak",
  "general",
  "system",
]);

export const parseConsoleHash = (
  hash: string,
): {page?: ConsolePage; settingsTab?: SettingsTab} => {
  const parts = hash.replace(/^#\/?/, "").split("/").filter(Boolean);
  const page = consolePages.has(parts[0] as ConsolePage)
    ? parts[0] as ConsolePage
    : undefined;
  const settingsTab = page === "settings" &&
    settingsTabs.has(parts[1] as SettingsTab)
    ? parts[1] as SettingsTab
    : undefined;
  return {page, settingsTab};
};

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
