/**
 * Keeping the screen-mode choice: one value per browser under
 * THEME_STORAGE_KEY. Apart from src/lib/theme.ts so that tests can run it
 * without a browser.
 */
export type ThemePreference = "system" | "light" | "dark";

export const THEME_STORAGE_KEY = "trss-theme-v1";

/** Where the choice is kept: the browser's localStorage, or a stand-in in tests. */
export interface PreferenceStore {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

function isPreference(value: unknown): value is ThemePreference {
  return value === "system" || value === "light" || value === "dark";
}

export function readStoredPreference(store: PreferenceStore | null): ThemePreference {
  try {
    const stored = store?.getItem(THEME_STORAGE_KEY);
    if (isPreference(stored)) return stored;
  } catch {
    // Storage can be blocked (private window, site data off); fall back to 시스템.
  }
  return "system";
}

export function writeStoredPreference(preference: ThemePreference, store: PreferenceStore | null) {
  try {
    store?.setItem(THEME_STORAGE_KEY, preference);
  } catch {
    // The choice still applies for this page view; it just cannot persist.
  }
}
