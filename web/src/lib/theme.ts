import { useSyncExternalStore } from "react";

/**
 * Screen-mode preference: 시스템 → 라이트 → 다크.
 *
 * The choice is kept in localStorage (per browser) and applied to <html> as
 * `data-theme-pref` (the choice) and `data-theme` (the resolved light/dark).
 * In 시스템 mode the resolved value follows `prefers-color-scheme` live.
 *
 * `index.html` carries a tiny inline script that applies the same attributes
 * before first paint so a reload does not flash the wrong mode; keep its
 * storage key in sync with THEME_STORAGE_KEY.
 */
export type ThemePreference = "system" | "light" | "dark";
export type ResolvedTheme = "light" | "dark";

export const THEME_STORAGE_KEY = "trss-theme-v1";
export const THEME_ORDER: readonly ThemePreference[] = ["system", "light", "dark"];

export const THEME_LABEL: Record<ThemePreference, string> = {
  system: "시스템",
  light: "라이트",
  dark: "다크",
};

/** The label with the particle that follows it in "…(으)로 변경". */
export const THEME_LABEL_WITH_RO: Record<ThemePreference, string> = {
  system: "시스템으로",
  light: "라이트로",
  dark: "다크로",
};

const DARK_QUERY = "(prefers-color-scheme: dark)";

function isPreference(value: unknown): value is ThemePreference {
  return value === "system" || value === "light" || value === "dark";
}

function readStoredPreference(): ThemePreference {
  try {
    const stored = window.localStorage.getItem(THEME_STORAGE_KEY);
    if (isPreference(stored)) return stored;
  } catch {
    // Storage can be blocked (private window, site data off); fall back to 시스템.
  }
  return "system";
}

function writeStoredPreference(preference: ThemePreference) {
  try {
    window.localStorage.setItem(THEME_STORAGE_KEY, preference);
  } catch {
    // The choice still applies for this page view; it just cannot persist.
  }
}

function systemTheme(): ResolvedTheme {
  return window.matchMedia?.(DARK_QUERY).matches ? "dark" : "light";
}

export function resolveTheme(preference: ThemePreference): ResolvedTheme {
  return preference === "system" ? systemTheme() : preference;
}

function applyToDocument(preference: ThemePreference) {
  const root = document.documentElement;
  root.setAttribute("data-theme-pref", preference);
  root.setAttribute("data-theme", resolveTheme(preference));
}

let current: ThemePreference = "system";
let started = false;
const listeners = new Set<() => void>();

function setPreference(preference: ThemePreference) {
  current = preference;
  applyToDocument(preference);
  listeners.forEach((notify) => notify());
}

/** Read the stored choice, apply it, and start following system/other-tab changes. */
export function initTheme() {
  if (started) return;
  started = true;

  setPreference(readStoredPreference());

  const query = window.matchMedia?.(DARK_QUERY);
  query?.addEventListener("change", () => {
    if (current === "system") applyToDocument("system");
  });

  window.addEventListener("storage", (event) => {
    if (event.key === THEME_STORAGE_KEY) setPreference(readStoredPreference());
  });
}

export function cycleTheme() {
  const next = THEME_ORDER[(THEME_ORDER.indexOf(current) + 1) % THEME_ORDER.length];
  writeStoredPreference(next);
  setPreference(next);
}

export function nextTheme(preference: ThemePreference): ThemePreference {
  return THEME_ORDER[(THEME_ORDER.indexOf(preference) + 1) % THEME_ORDER.length];
}

function subscribe(notify: () => void) {
  listeners.add(notify);
  return () => listeners.delete(notify);
}

export function useThemePreference(): ThemePreference {
  return useSyncExternalStore(
    subscribe,
    () => current,
    () => "system",
  );
}
