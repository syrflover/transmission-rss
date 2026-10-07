import assert from "node:assert/strict";
import { test } from "node:test";

import { moveStillTells } from "./moveStillTells.ts";

const move = (direction: "archive" | "restore" | "start" | "resume", state: string) => ({
  direction,
  command: { state },
});

test("a failed start or resume tells only while the rule is still paused", () => {
  for (const direction of ["start", "resume"] as const) {
    assert.equal(moveStillTells(move(direction, "failed"), "paused"), true);
    // Turned on another way after the failure: the old failure is not about it.
    assert.equal(moveStillTells(move(direction, "failed"), "active"), false);
    assert.equal(moveStillTells(move(direction, "failed"), "archived"), false);
  }
});

test("a start or resume that moved tells while the rule collects", () => {
  for (const direction of ["start", "resume"] as const) {
    assert.equal(moveStillTells(move(direction, "done"), "active"), true);
    assert.equal(moveStillTells(move(direction, "done"), "paused"), false);
    assert.equal(moveStillTells(move(direction, "done"), "archived"), false);
  }
});

test("an archive and a restore tell as before", () => {
  assert.equal(moveStillTells(move("archive", "done"), "archived"), true);
  assert.equal(moveStillTells(move("archive", "done"), "active"), false);
  assert.equal(moveStillTells(move("archive", "failed"), "active"), true);
  assert.equal(moveStillTells(move("restore", "done"), "active"), true);
  assert.equal(moveStillTells(move("restore", "done"), "archived"), false);
  assert.equal(moveStillTells(move("restore", "failed"), "archived"), true);
});
