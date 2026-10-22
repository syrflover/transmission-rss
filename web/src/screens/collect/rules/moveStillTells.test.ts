import assert from "node:assert/strict";
import { test } from "node:test";

import { moveStillTells } from "./moveStillTells.ts";

const move = (direction: "start" | "resume", state: string) => ({
  direction,
  command: { state },
});

test("a failed start or resume tells only while the rule is still paused", () => {
  for (const direction of ["start", "resume"] as const) {
    assert.equal(moveStillTells(move(direction, "failed"), "paused"), true);
    // Turned on another way after the failure: the old failure is not about it.
    assert.equal(moveStillTells(move(direction, "failed"), "active"), false);
  }
});

test("a start or resume that moved tells while the rule collects", () => {
  for (const direction of ["start", "resume"] as const) {
    assert.equal(moveStillTells(move(direction, "done"), "active"), true);
    assert.equal(moveStillTells(move(direction, "done"), "paused"), false);
  }
});
