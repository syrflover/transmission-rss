import { useSyncExternalStore } from "react";

import {
  readStoredPreference,
  THEME_STORAGE_KEY,
  writeStoredPreference,
  type PreferenceStore,
  type ThemePreference,
} from "./theme-store";

/**
 * Screen-mode preference: 시스템, 라이트 or 다크, chosen in 설정.
 *
 * The choice is kept in localStorage (per browser, src/lib/theme-store.ts) and
 * applied to <html> as `data-theme-pref` (the choice) and `data-theme` (the
 * resolved light/dark). In 시스템 mode the resolved value follows
 * `prefers-color-scheme` live.
 *
 * `index.html` carries a tiny inline script that applies the same attributes
 * before first paint so a reload does not flash the wrong mode; keep its
 * storage key in sync with THEME_STORAGE_KEY.
 */
export type { ThemePreference };
export type ResolvedTheme = "light" | "dark";

export const THEME_ORDER: readonly ThemePreference[] = ["system", "light", "dark"];

export const THEME_LABEL: Record<ThemePreference, string> = {
  system: "시스템",
  light: "라이트",
  dark: "다크",
};

const DARK_QUERY = "(prefers-color-scheme: dark)";

/** The browser's localStorage, or null where reading it throws; the library list keeps its choices there too. */
export function browserStore(): PreferenceStore | null {
  try {
    return window.localStorage;
  } catch {
    // Reading localStorage itself throws when site data is off.
    return null;
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

  setPreference(readStoredPreference(browserStore()));

  const query = window.matchMedia?.(DARK_QUERY);
  query?.addEventListener("change", () => {
    if (current === "system") applyToDocument("system");
  });

  window.addEventListener("storage", (event) => {
    if (event.key === THEME_STORAGE_KEY) setPreference(readStoredPreference(browserStore()));
  });
}

/** Apply a mode at once and keep it for this browser. */
export function chooseTheme(preference: ThemePreference) {
  writeStoredPreference(preference, browserStore());
  setPreference(preference);
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
