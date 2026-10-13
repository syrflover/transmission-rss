import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

import { readStoredPreference, THEME_STORAGE_KEY, writeStoredPreference, type PreferenceStore } from "./theme-store.ts";

function memoryStore(): PreferenceStore & { values: Map<string, string> } {
  const values = new Map<string, string>();
  return {
    values,
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => void values.set(key, value),
  };
}

/** A browser with site data off: every access throws. */
const blocked: PreferenceStore = {
  getItem: () => {
    throw new DOMException("blocked", "SecurityError");
  },
  setItem: () => {
    throw new DOMException("blocked", "SecurityError");
  },
};

test("a chosen mode is kept under the key and read back", () => {
  const store = memoryStore();
  for (const mode of ["dark", "light", "system"] as const) {
    writeStoredPreference(mode, store);
    assert.equal(store.values.get("trss-theme-v1"), mode);
    assert.equal(readStoredPreference(store), mode);
  }
});

test("nothing kept, or a value that is not a mode, reads as 시스템", () => {
  const store = memoryStore();
  assert.equal(readStoredPreference(store), "system");
  store.values.set(THEME_STORAGE_KEY, "sepia");
  assert.equal(readStoredPreference(store), "system");
  store.values.set(THEME_STORAGE_KEY, "Dark");
  assert.equal(readStoredPreference(store), "system");
});

test("blocked or missing storage reads as 시스템 and a choice does not throw", () => {
  assert.equal(readStoredPreference(blocked), "system");
  assert.doesNotThrow(() => writeStoredPreference("dark", blocked));
  assert.equal(readStoredPreference(null), "system");
  assert.doesNotThrow(() => writeStoredPreference("dark", null));
});

test("the script that applies the mode before first paint reads the same key", () => {
  const html = readFileSync(new URL("../../index.html", import.meta.url), "utf8");
  assert.ok(html.includes(`localStorage.getItem("${THEME_STORAGE_KEY}")`));
});
