import assert from "node:assert/strict";
import { test } from "node:test";

import { asksOnRead, openGate } from "./prepareGate.ts";

type State = "ready" | "preparing" | "closed" | "unavailable";
const job = (state: State | null, run: string | null = null) => ({
  screen: state === null ? null : { state, run, bound: run === null ? null : 1000, note: null },
});

/** How many requests a page makes over the reads of the job it sees, in order. */
const requests = (first: unknown, reads: ReturnType<typeof job>[]) => {
  const gate = openGate(first);
  return reads.filter((read) => asksOnRead(gate, read)).length;
};

test("opening the page asks once, on the first read from the server, and polling never asks again", () => {
  const cached = job("ready", "r1");
  // The cached job shows first and is read again (the same object) before the server answers.
  const gate = openGate(cached);
  assert.equal(asksOnRead(gate, cached), false);
  assert.equal(asksOnRead(gate, job("closed")), true);
  for (let n = 0; n < 5; n++) assert.equal(asksOnRead(gate, job("closed")), false);
  assert.equal(requests(undefined, [job("ready", "r1"), job("ready", "r1"), job("preparing")]), 1);
});

test("the reads a reconnecting socket makes never ask", () => {
  const gate = openGate(undefined);
  assert.equal(asksOnRead(gate, job("ready", "r1")), true);
  // The socket closed on its own or was ended: it reads the job again, and the screen may have closed meanwhile.
  for (const read of [job("ready", "r1"), job("closed"), job("ready", "r2"), job("preparing")]) {
    assert.equal(asksOnRead(gate, read), false);
  }
});

test("a job with no screen, or a web with no server browser, asks nothing", () => {
  assert.equal(requests(undefined, [job(null), job("ready", "r1")]), 0);
  assert.equal(requests(undefined, [job("unavailable")]), 0);
});

test("opening the page again asks again", () => {
  assert.equal(requests(undefined, [job("closed")]), 1);
  assert.equal(requests(job("closed"), [job("closed")]), 1);
});
