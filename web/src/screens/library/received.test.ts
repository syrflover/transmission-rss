import assert from "node:assert/strict";
import { test } from "node:test";

import { carriesTime, receivedAt, sameDay } from "./received.ts";

const at = (month: number, day: number, hour = 0, minute = 0) => new Date(2026, month - 1, day, hour, minute).getTime();
const copy = (id: string, stored_at: number, group = "g") => ({ id, stored_at, group });
const sameGroup = (a: { group: string }, b: { group: string }) => a.group === b.group;

test("a received date carries its time only when asked to, and the clock is padded", () => {
  assert.equal(receivedAt(at(9, 7, 9, 5), false), "9월 7일 받음");
  assert.equal(receivedAt(at(9, 7, 9, 5), true), "9월 7일 09:05 받음");
  assert.equal(receivedAt(at(12, 31, 23, 59), true), "12월 31일 23:59 받음");
});

test("two moments are on the same day by the viewer's calendar, not by 24 hours", () => {
  assert.equal(sameDay(at(9, 7, 0, 0), at(9, 7, 23, 59)), true);
  assert.equal(sameDay(at(9, 7, 23, 59), at(9, 8, 0, 1)), false);
  assert.equal(sameDay(at(9, 7, 12), at(10, 7, 12)), false);
});

test("the date carries its time when another copy of the same group came the same day", () => {
  const a = copy("a", at(9, 7, 9, 5));
  const b = copy("b", at(9, 7, 21, 30));
  assert.equal(carriesTime(a, [a], sameGroup), false);
  assert.equal(carriesTime(a, [a, b], sameGroup), true);
  assert.equal(carriesTime(b, [a, b], sameGroup), true);
});

test("a copy of another group, or of another day, or the copy itself does not count", () => {
  const a = copy("a", at(9, 7, 9, 5));
  assert.equal(carriesTime(a, [a, copy("b", at(9, 7, 10), "other")], sameGroup), false);
  assert.equal(carriesTime(a, [a, copy("c", at(9, 8, 9, 5))], sameGroup), false);
  assert.equal(carriesTime(a, [a, copy("a", at(9, 7, 11))], sameGroup), false);
});

test("a card's own grouping decides which copies are the same", () => {
  const a = { ...copy("a", at(9, 7, 9)), format: "ass", creator: "x" };
  const b = { ...copy("b", at(9, 7, 10)), format: "srt", creator: "x" };
  assert.equal(carriesTime(a, [a, b], (p, q) => p.creator === q.creator), true);
  assert.equal(carriesTime(a, [a, b], (p, q) => p.creator === q.creator && p.format === q.format), false);
});
