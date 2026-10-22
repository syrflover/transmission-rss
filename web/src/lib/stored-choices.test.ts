import assert from "node:assert/strict";
import { test } from "node:test";

import { readStoredChoices, writeStoredChoices, type Choices } from "./stored-choices.ts";
import type { PreferenceStore } from "./theme-store.ts";

type List = { view: "grid" | "list"; sort: "subtitle" | "title"; filter: "all" | "airing" };

const KEY = "trss-test-list-v1";
const DEFAULTS: List = { view: "grid", sort: "subtitle", filter: "all" };
const CHOICES: Choices<List> = { view: ["grid", "list"], sort: ["subtitle", "title"], filter: ["all", "airing"] };

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

test("kept choices are written under the key and read back", () => {
  const store = memoryStore();
  const chosen: List = { view: "list", sort: "title", filter: "airing" };
  writeStoredChoices(store, KEY, chosen);
  assert.deepEqual(JSON.parse(store.values.get(KEY) ?? ""), chosen);
  assert.deepEqual(readStoredChoices(store, KEY, DEFAULTS, CHOICES), chosen);
});

test("nothing kept reads as the defaults", () => {
  assert.deepEqual(readStoredChoices(memoryStore(), KEY, DEFAULTS, CHOICES), DEFAULTS);
});

test("a value the screen no longer offers falls back alone and the other items stay", () => {
  const store = memoryStore();
  store.values.set(KEY, JSON.stringify({ view: "list", sort: "added", filter: "airing" }));
  assert.deepEqual(readStoredChoices(store, KEY, DEFAULTS, CHOICES), { view: "list", sort: "subtitle", filter: "airing" });
  store.values.set(KEY, JSON.stringify({ view: 1, filter: null, sort: "title" }));
  assert.deepEqual(readStoredChoices(store, KEY, DEFAULTS, CHOICES), { view: "grid", sort: "title", filter: "all" });
});

test("a kept value that is not an object of choices reads as the defaults", () => {
  const store = memoryStore();
  for (const text of ["not json", "[\"list\"]", "\"list\"", "3", "null", ""]) {
    store.values.set(KEY, text);
    assert.deepEqual(readStoredChoices(store, KEY, DEFAULTS, CHOICES), DEFAULTS, text);
  }
});

test("blocked or missing storage reads as the defaults and a choice does not throw", () => {
  assert.deepEqual(readStoredChoices(blocked, KEY, DEFAULTS, CHOICES), DEFAULTS);
  assert.doesNotThrow(() => writeStoredChoices(blocked, KEY, DEFAULTS));
  assert.deepEqual(readStoredChoices(null, KEY, DEFAULTS, CHOICES), DEFAULTS);
  assert.doesNotThrow(() => writeStoredChoices(null, KEY, DEFAULTS));
});
