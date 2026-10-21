import assert from "node:assert/strict";
import { test } from "node:test";

import { failureText, newlyFinished, runOfView, seenOf, type ArchiveBatchView } from "./batchRun.ts";

const view = (over: Partial<ArchiveBatchView> = {}): ArchiveBatchView => ({
  id: "batch-1",
  state: "running",
  total: 3,
  finished: 1,
  current: { rule_id: "r2", name: "두 번째 작품" },
  failure: null,
  ...over,
});

test("a batch under way is the run `보관하는 중이에요` shows: the total, the finished and the work in turn", () => {
  assert.deepEqual(runOfView(view()), { total: 3, finished: 1, current: "두 번째 작품", error: null });
  assert.deepEqual(runOfView(view({ state: "pending", finished: 0 })).current, "두 번째 작품");
});

test("a batch that archived every rule is a run that ended without an error", () => {
  const done = view({ state: "done", finished: 3, current: null });
  assert.deepEqual(runOfView(done), { total: 3, finished: 3, current: null, error: null });
});

test("a rule that could not be started says the server's sentence after its name", () => {
  const refused = view({
    state: "failed",
    current: null,
    failure: {
      rule_id: "r2",
      name: "두 번째 작품",
      reason: "이 규칙은 이미 보관하거나 복원하거나 작품 폴더를 옮기는 중이에요. 그 결과가 나올 때까지 기다려 주세요.",
      kind: "refused",
    },
  });
  assert.deepEqual(runOfView(refused), {
    total: 3,
    finished: 1,
    current: null,
    error:
      "‘두 번째 작품’: 이 규칙은 이미 보관하거나 복원하거나 작품 폴더를 옮기는 중이에요. 그 결과가 나올 때까지 기다려 주세요.",
  });
});

test("a rule whose archive failed says it could not be moved, why, and where to look", () => {
  const failed = view({
    state: "failed",
    current: null,
    failure: { rule_id: "r2", name: "두 번째 작품", reason: "Transmission이 폴더를 옮기지 못했어요.", kind: "archive_failed" },
  });
  assert.equal(
    runOfView(failed).error,
    "‘두 번째 작품’을 옮기지 못했어요. Transmission이 폴더를 옮기지 못했어요. 규칙 상세에서 상태를 확인해 주세요.",
  );
  assert.equal(runOfView(failed).current, null);
});

test("a failed batch the server named no rule for still ends the run with a sentence", () => {
  assert.notEqual(failureText(null), "");
  assert.equal(runOfView(view({ state: "failed", current: null, failure: null })).error, failureText(null));
});

test("the rules a screen that sent the batch knows leave the suggestions as the batch finishes them", () => {
  const known = ["r1", "r2", "r3"];
  const first = view({ finished: 0, current: { rule_id: "r1", name: "a" } });
  assert.deepEqual(newlyFinished(known, { finished: 0, current: null }, first), []);

  const second = view({ finished: 1, current: { rule_id: "r2", name: "b" } });
  assert.deepEqual(newlyFinished(known, seenOf(first), second), ["r1"]);

  // Two events read as one: both rules that finished are named, in order.
  const end = view({ state: "done", finished: 3, current: null });
  assert.deepEqual(newlyFinished(known, seenOf(second), end), ["r2", "r3"]);

  // A view that shows nothing new names nothing.
  assert.deepEqual(newlyFinished(known, seenOf(second), second), []);
});

test("a screen that found the batch running names the rule that was in turn when the batch moved one on", () => {
  const found = view({ finished: 1, current: { rule_id: "r2", name: "b" } });
  const next = view({ finished: 2, current: { rule_id: "r3", name: "c" } });
  assert.deepEqual(newlyFinished(null, seenOf(found), next), ["r2"]);

  // It cannot tell which rules a jump over several finished.
  const far = view({ state: "done", finished: 3, current: null });
  assert.deepEqual(newlyFinished(null, seenOf(found), far), []);
});
