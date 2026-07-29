export type ThemePreference = "system" | "light" | "dark";
export type DensityPreference = "comfortable" | "compact";

export type UserPreferences = {
  theme: ThemePreference;
  density: DensityPreference;
  start_page: string;
  dashboard_refresh_seconds: number;
  reduce_motion: boolean;
};

export const defaultPreferences: UserPreferences = {
  theme: "system",
  density: "comfortable",
  start_page: "dashboard",
  dashboard_refresh_seconds: 60,
  reduce_motion: false,
};

export const normalizePreferences = (
  value: Partial<UserPreferences> | null | undefined,
): UserPreferences => ({
  theme: ["system", "light", "dark"].includes(value?.theme || "")
    ? value!.theme!
    : defaultPreferences.theme,
  density: ["comfortable", "compact"].includes(value?.density || "")
    ? value!.density!
    : defaultPreferences.density,
  start_page: typeof value?.start_page === "string" && value.start_page
    ? value.start_page
    : defaultPreferences.start_page,
  dashboard_refresh_seconds: [0, 30, 60, 300].includes(
    value?.dashboard_refresh_seconds ?? -1,
  ) ? value!.dashboard_refresh_seconds! : defaultPreferences.dashboard_refresh_seconds,
  reduce_motion: typeof value?.reduce_motion === "boolean"
    ? value.reduce_motion
    : defaultPreferences.reduce_motion,
});

const storageKey = (userID: string) => `invenqor.preferences.${userID}`;

export const loadPreferences = (userID: string): UserPreferences => {
  try {
    const raw = localStorage.getItem(storageKey(userID));
    return normalizePreferences(raw ? JSON.parse(raw) : null);
  } catch {
    return defaultPreferences;
  }
};

export const savePreferences = (
  userID: string,
  preferences: UserPreferences,
) => {
  localStorage.setItem(storageKey(userID), JSON.stringify(preferences));
};

export const applyPreferences = (preferences: UserPreferences) => {
  const root = document.documentElement;
  root.dataset.themePreference = preferences.theme;
  root.dataset.theme = preferences.theme === "system"
    ? window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light"
    : preferences.theme;
  root.dataset.density = preferences.density;
  root.dataset.reduceMotion = String(preferences.reduce_motion);
};
