import assert from "node:assert/strict";
import { test } from "node:test";

import { episodeLabel, type EpisodeSegment } from "./episodeLine.ts";

const seg = (text: string, count = 1, whole = true): EpisodeSegment => ({ text, count, whole });

test("no runs is no label", () => {
  assert.equal(episodeLabel([]), null);
});

test("runs the server names are joined by · and end in 화", () => {
  assert.deepEqual(episodeLabel([seg("11")]), { label: "11화", more: 0 });
  assert.deepEqual(episodeLabel([seg("2–3", 2)]), { label: "2–3화", more: 0 });
  assert.deepEqual(episodeLabel([seg("1–4", 4), seg("7"), seg("SP", 1, false)]), { label: "1–4·7·SP화", more: 0 });
});

test("a list of more than three runs names three and counts the episodes left out", () => {
  const runs = [seg("1–4", 4), seg("7"), seg("9–10", 2), seg("12–14", 3), seg("SP", 1, false)];
  assert.deepEqual(episodeLabel(runs), { label: "1–4·7·9–10화", more: 4 });
  assert.deepEqual(episodeLabel(runs.slice(0, 4)), { label: "1–4·7·9–10화", more: 3 });
  assert.deepEqual(episodeLabel(runs.slice(0, 3)), { label: "1–4·7·9–10화", more: 0 });
});
