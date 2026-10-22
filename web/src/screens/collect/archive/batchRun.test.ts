import assert from "node:assert/strict";
import { test } from "node:test";

import type { Command } from "../../../lib/api-types/Command";

import { failureText, newlyFinished, runOfView, seenOf, type ArchiveBatchView } from "./batchRun.ts";

const view = (over: Partial<ArchiveBatchView> = {}): ArchiveBatchView => ({
  id: "batch-1",
  state: "running",
  total: 3,
  finished: 1,
  current: { rule_id: "r2", name: "두 번째 작품" },
  failure: null,
  results: [],
  ...over,
});

/** The move of a work folder as the worker ended it. */
const move = (id: string, state: "done" | "failed", result: string, reason: string | null): Command => ({
  id,
  kind: "work_archive",
  state,
  created_at: 0,
  updated_at: 0,
  finished_at: 0,
  outcome: { result, reason },
});

test("a batch under way is the run `옮기는 중이에요` shows: the total, the finished and the work in turn", () => {
  assert.deepEqual(runOfView(view()), {
    total: 3,
    finished: 1,
    current: "두 번째 작품",
    error: null,
    results: [],
    errorWorkId: null,
  });
  assert.deepEqual(runOfView(view({ state: "pending", finished: 0 })).current, "두 번째 작품");
});

test("a batch that archived every rule ends without an error and says what became of each work folder", () => {
  const done = view({
    state: "done",
    finished: 3,
    current: null,
    results: [
      { rule_id: "r1", name: "첫 번째 작품", command: move("batch-1-0", "done", "moved", null), work_id: "w1" },
      { rule_id: "r2", name: "전체", command: null, work_id: null },
      {
        rule_id: "r3",
        name: "세 번째 작품",
        command: move("batch-1-2", "done", "kept", "보관 폴더를 정하지 않아서 폴더는 옮기지 않았어요."),
        work_id: "w3",
      },
    ],
  });
  assert.deepEqual(runOfView(done), {
    total: 3,
    finished: 3,
    current: null,
    error: null,
    results: [
      { ruleId: "r1", text: "‘첫 번째 작품’ 규칙을 지우고 작품 폴더를 보관 폴더로 옮겼어요.", workId: "w1" },
      { ruleId: "r2", text: "‘전체’ 규칙을 지웠어요.", workId: null },
      {
        ruleId: "r3",
        text: "‘세 번째 작품’ 규칙을 지웠어요. 보관 폴더를 정하지 않아서 폴더는 옮기지 않았어요.",
        workId: "w3",
      },
    ],
    errorWorkId: null,
  });
});

test("a rule that could not be started says the server's sentence after its name", () => {
  const refused = view({
    state: "failed",
    current: null,
    failure: {
      rule_id: "r2",
      name: "두 번째 작품",
      reason: "이 규칙의 작품 폴더를 이미 보관 폴더로 옮기는 중이라서 보관하지 않았어요. 그 결과가 나온 뒤 다시 보관해 주세요.",
      kind: "refused",
      work_id: "w2",
    },
  });
  assert.deepEqual(runOfView(refused), {
    total: 3,
    finished: 1,
    current: null,
    error:
      "‘두 번째 작품’: 이 규칙의 작품 폴더를 이미 보관 폴더로 옮기는 중이라서 보관하지 않았어요. 그 결과가 나온 뒤 다시 보관해 주세요.",
    results: [],
    errorWorkId: "w2",
  });
});

test("a rule whose work folder could not move says the rule is deleted all the same, why, and where to move it", () => {
  const failed = view({
    state: "failed",
    current: null,
    failure: {
      rule_id: "r2",
      name: "두 번째 작품",
      reason: "Transmission이 폴더를 옮기지 못했어요.",
      kind: "archive_failed",
      work_id: "w2",
    },
  });
  assert.equal(
    runOfView(failed).error,
    "‘두 번째 작품’ 규칙은 지웠지만 작품 폴더를 옮기지 못했어요. Transmission이 폴더를 옮기지 못했어요. 작품 상세에서 다시 옮길 수 있어요.",
  );
  assert.equal(runOfView(failed).current, null);
  assert.equal(runOfView(failed).errorWorkId, "w2");
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
