import assert from "node:assert/strict";
import { test } from "node:test";

import { segmentsText } from "./episodeKey.ts";

const whole = (text: string) => ({ text, whole: true });
const other = (text: string) => ({ text, whole: false });

test("a creator's runs of whole numbers are joined by · and end in 화", () => {
  assert.equal(segmentsText([whole("1–4"), whole("11–15")]), "1–4·11–15화");
  assert.equal(segmentsText([whole("0–2")]), "0–2화");
  assert.equal(segmentsText([whole("0")]), "0화");
});

test("every other text follows after · with a space around it, as written", () => {
  assert.equal(segmentsText([whole("1–4"), whole("11–15"), other("13.5"), other("SP")]), "1–4·11–15화 · 13.5 · SP");
  assert.equal(segmentsText([other("SP")]), "SP");
});

test("no runs is no text", () => {
  assert.equal(segmentsText([]), "");
});
