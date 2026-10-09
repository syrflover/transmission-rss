import assert from "node:assert/strict";
import { test } from "node:test";

import { when } from "../../lib/time.ts";
import { unpackText, type UnpackResult } from "./unpack.ts";

const NOW = Date.UTC(2026, 9, 6, 3, 0); // 12:00 in Seoul
const HOUR = 3_600_000;
const FULL = "압축을 풀 자리에 쓰지 못했어요: No space left on device (os error 28)";

const unpack = (fields: Partial<UnpackResult>): UnpackResult => ({
  state: "done",
  reason: null,
  tries: 0,
  max_tries: 3,
  retry_at: null,
  first: null,
  files: null,
  subtitles: null,
  fonts: null,
  ...fields,
});

test("a try this machine failed says which, why, and when the next goes", () => {
  assert.deepEqual(unpackText(unpack({ state: "retry", reason: FULL, tries: 1, retry_at: NOW + HOUR }), NOW), {
    text: "다시 풀기를 기다려요 (3번 중 1번째 시도 실패)",
    reason: `${FULL} · 다음 시도: ${when(NOW + HOUR, NOW)} 또는 worker가 다시 시작할 때`,
    urgent: false,
  });
  assert.equal(when(NOW + HOUR, NOW), "오늘 13:00");
  // A worker started since, or the hour went by: the next run tries it.
  for (const retry_at of [null, NOW - HOUR, NOW]) {
    assert.equal(
      unpackText(unpack({ state: "retry", reason: FULL, tries: 2, retry_at }), NOW).reason,
      `${FULL} · 다음 시도: 다음 실행 때`,
    );
  }
});

test("풀지 못함 says how many tries it took when this machine failed them, and only then", () => {
  assert.deepEqual(unpackText(unpack({ state: "failed", reason: FULL, tries: 3 }), NOW), {
    text: "풀지 못함",
    reason: `${FULL} · 3번 시도했어요`,
    urgent: true,
  });
  assert.equal(unpackText(unpack({ state: "failed", reason: "암호가 걸려 있어요" }), NOW).reason, "암호가 걸려 있어요");
});

test("an unpacked archive says how many files it held, whatever tries it took", () => {
  assert.deepEqual(unpackText(unpack({ files: 14, subtitles: 12, fonts: 2, tries: 2 }), NOW), {
    text: "파일 14개를 풀었어요 (자막 12 · 폰트 2)",
    reason: null,
    urgent: false,
  });
});
