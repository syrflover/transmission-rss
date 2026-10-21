import assert from "node:assert/strict";
import { test } from "node:test";

import { canRestart, drawnStepState, interruptedText } from "./interrupted.ts";

test("only an interrupted job can be started again", () => {
  assert.equal(canRestart({ state: "interrupted" }), true);
  for (const state of ["pending", "running", "waiting", "held", "failed", "partial", "done"] as const) {
    assert.equal(canRestart({ state }), false, state);
  }
});

test("the step an interrupted job was cut in is drawn stopped, and no other step or job changes", () => {
  assert.equal(drawnStepState("interrupted", "current"), "stopped");
  for (const state of ["upcoming", "waiting", "done", "failed", "partial"] as const) {
    assert.equal(drawnStepState("interrupted", state), state, state);
  }
  for (const job of ["running", "held", "waiting", "failed"] as const) {
    assert.equal(drawnStepState(job, "current"), "current", job);
  }
});

test("an interrupted job names the step it was cut in", () => {
  assert.equal(
    interruptedText("store"),
    "보관 단계를 실행하던 중에 멈췄어요. 다시 시작하면 끊긴 단계부터 이어서 해요.",
  );
  assert.equal(interruptedText(null), "실행하던 중에 멈췄어요. 다시 시작하면 끊긴 단계부터 이어서 해요.");
});
