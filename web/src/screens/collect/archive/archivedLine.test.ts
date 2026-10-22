import assert from "node:assert/strict";
import { test } from "node:test";

import { archivedLine } from "./archivedLine.ts";

const ended = (state: "done" | "failed", result: string, reason: string | null) => ({
  state,
  outcome: { result, reason },
});

test("every line says the rule is deleted, and what became of the work folder", () => {
  assert.deepEqual(archivedLine("Clevatess", ended("done", "moved", null), null), {
    text: "‘Clevatess’ 규칙을 지우고 작품 폴더를 보관 폴더로 옮겼어요.",
    failed: false,
  });
  assert.deepEqual(archivedLine("Clevatess", { state: "running", outcome: null }, null), {
    text: "‘Clevatess’ 규칙을 지웠어요. 작품 폴더를 보관 폴더로 옮기는 중이에요.",
    failed: false,
  });
  assert.equal(
    archivedLine(
      "Clevatess",
      ended("done", "kept", "‘Clevatess/Season 03’ 규칙이 아직 이 작품 폴더에 받고 있어서 옮기지 않았어요."),
      null,
    ).text,
    "‘Clevatess’ 규칙을 지웠어요. ‘Clevatess/Season 03’ 규칙이 아직 이 작품 폴더에 받고 있어서 옮기지 않았어요.",
  );
});

test("a rule with no work folder says why nothing moves, or only that it is deleted", () => {
  assert.equal(
    archivedLine("전체", null, "저장 폴더가 수집 폴더 자체라서 옮길 작품 폴더가 없어요.").text,
    "‘전체’ 규칙을 지웠어요. 저장 폴더가 수집 폴더 자체라서 옮길 작품 폴더가 없어요.",
  );
  assert.equal(archivedLine("전체", null, null).text, "‘전체’ 규칙을 지웠어요.");
});

test("a move that failed says why and where to move it again", () => {
  const line = archivedLine("Clevatess", ended("failed", "failed", "Transmission이 폴더를 옮기지 못했어요."), null);
  assert.equal(line.failed, true);
  assert.equal(
    line.text,
    "‘Clevatess’ 규칙은 지웠지만 작품 폴더를 옮기지 못했어요. Transmission이 폴더를 옮기지 못했어요. 작품 상세에서 다시 옮길 수 있어요.",
  );
});
