import type { PreferenceStore } from "./theme-store.ts";

/**
 * Keeping a screen's choices in this browser: one JSON object per screen under
 * its key, read item by item, so a kept value the screen no longer offers falls
 * back to that item's default alone, and blocked storage or a broken value to
 * every default. Imports no screen code, so it runs under `node --test`; the
 * screen hands over its defaults and the values each item may take.
 */

/** The values each kept item may take. */
export type Choices<T extends Record<string, string>> = { [K in keyof T]: readonly T[K][] };

export function readStoredChoices<T extends Record<string, string>>(
  store: PreferenceStore | null,
  key: string,
  defaults: T,
  choices: Choices<T>,
): T {
  let stored: Record<string, unknown> = {};
  try {
    const parsed: unknown = JSON.parse(store?.getItem(key) ?? "null");
    if (typeof parsed === "object" && parsed !== null) stored = parsed as Record<string, unknown>;
  } catch {
    // Storage can be blocked (private window, site data off) or hold something that is not JSON.
  }
  const read = { ...defaults };
  for (const item of Object.keys(defaults) as (keyof T & string)[]) {
    const value = stored[item];
    if ((choices[item] as readonly unknown[]).includes(value)) read[item] = value as T[typeof item];
  }
  return read;
}

export function writeStoredChoices<T extends Record<string, string>>(
  store: PreferenceStore | null,
  key: string,
  values: T,
) {
  try {
    store?.setItem(key, JSON.stringify(values));
  } catch {
    // The choices still apply for this page view; they just cannot persist.
  }
}
