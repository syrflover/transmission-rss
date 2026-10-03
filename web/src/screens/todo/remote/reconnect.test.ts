import assert from "node:assert/strict";
import { test } from "node:test";

import { afterEnd, retryDelay } from "./reconnect.ts";

const screen = (state: "ready" | "preparing" | "closed" | "unavailable", run: string | null, bound: number | null = run === null ? null : 1000) => ({
  state,
  run,
  bound,
  note: null,
});

test("the delays grow and the attempts stop after five", () => {
  assert.deepEqual([1, 2, 3, 4, 5].map(retryDelay), [500, 1000, 2000, 4000, 8000]);
  assert.equal(retryDelay(6), null);
  assert.equal(retryDelay(0), null);
});

test("a closed socket reconnects to the binding the job still has", () => {
  assert.deepEqual(afterEnd({ waiting: true, screen: screen("ready", "r1") }, null), {
    kind: "connect",
    binding: { run: "r1", bound: 1000 },
  });
});

test("a reconnect never goes on when the job has no run bound: the person's opening of the page prepares it", () => {
  assert.deepEqual(afterEnd({ waiting: true, screen: screen("closed", null) }, null), { kind: "stop" });
  assert.deepEqual(afterEnd({ waiting: true, screen: screen("preparing", null) }, null), { kind: "stop" });
  assert.deepEqual(afterEnd({ waiting: true, screen: screen("unavailable", null) }, null), { kind: "stop" });
  assert.deepEqual(afterEnd({ waiting: true, screen: null }, null), { kind: "stop" });
});

test("a job that no longer waits for its check is not connected to", () => {
  assert.deepEqual(afterEnd({ waiting: false, screen: screen("ready", "r1") }, null), { kind: "stop" });
});

test("a binding the server ended is not connected to again, but one that took its place is", () => {
  const ended = { run: "r1", bound: 1000 };
  assert.deepEqual(afterEnd({ waiting: true, screen: screen("ready", "r1") }, ended), { kind: "stop" });
  assert.deepEqual(afterEnd({ waiting: true, screen: screen("ready", "r2", 2000) }, ended), {
    kind: "connect",
    binding: { run: "r2", bound: 2000 },
  });
});

test("another check of the job in the same run is connected to anew", () => {
  const ended = { run: "r1", bound: 1000 };
  assert.deepEqual(afterEnd({ waiting: true, screen: screen("ready", "r1", 3000) }, ended), {
    kind: "connect",
    binding: { run: "r1", bound: 3000 },
  });
});
