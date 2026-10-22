import assert from "node:assert/strict";
import { test } from "node:test";

import type { Command } from "../../../lib/api-types/Command";

import { archiveNotice } from "./archiveNotice.ts";

const command = (state: Command["state"], result: string | null = null, reason: string | null = null): Command => ({
  id: "c1",
  kind: "work_archive",
  state,
  created_at: 1,
  updated_at: 2,
  finished_at: state === "done" || state === "failed" ? 3 : null,
  outcome: result === null ? null : { result, reason },
});

const work = (stored: Command | null, available = true) => ({ archive: { available, command: stored } });

test("nothing is said while nothing was asked", () => {
  assert.equal(archiveNotice(work(null), { kind: "idle", error: null }), null);
});

test("a move that did not go through tells why, and only a failed one offers 다시 옮기기", () => {
  const failed = archiveNotice(work(command("failed", "failed", "겹치는 파일이 있어요.")), { kind: "idle", error: null });
  assert.equal(failed?.line?.title, "보관 폴더로 옮기지 못했어요.");
  assert.equal(failed?.line?.detail, "겹치는 파일이 있어요.");
  assert.equal(failed?.line?.failed, true);
  assert.equal(failed?.moveAgain, true);

  // No archive folder any more: the reason stays, the retry does not.
  const unavailable = archiveNotice(work(command("failed", "failed", "x"), false), { kind: "idle", error: null });
  assert.equal(unavailable?.moveAgain, false);

  const unknown = archiveNotice(work(command("failed")), { kind: "idle", error: null });
  assert.equal(unknown?.line?.detail, "까닭은 알 수 없어요.");

  const kept = archiveNotice(work(command("done", "kept", "‘A’ 규칙이 아직 받고 있어요.")), { kind: "idle", error: null });
  assert.equal(kept?.line?.title, "작품 폴더를 옮기지 않았어요.");
  assert.equal(kept?.line?.detail, "‘A’ 규칙이 아직 받고 있어요.");
  assert.equal(kept?.moveAgain, false);
});

test("the page's own move tells its progress and its end, before what the server shows", () => {
  const stored = command("failed", "failed", "old");
  assert.equal(archiveNotice(work(stored), { kind: "sending" })?.line?.title, "보관 폴더로 옮기는 중이에요.");
  assert.equal(archiveNotice(work(stored), { kind: "waiting" })?.moveAgain, false);
  const moved = archiveNotice(work(null, false), { kind: "ended", command: command("done", "moved") });
  assert.equal(moved?.line?.title, "보관 폴더로 옮겼어요.");
  assert.equal(moved?.line?.at, 3);
});

test("an open move the server shows is followed, not told as ended", () => {
  assert.equal(archiveNotice(work(command("running")), { kind: "idle", error: null }), null);
});

test("a refusal is told on its own", () => {
  const refused = archiveNotice(work(null), { kind: "idle", error: "이 작품 폴더에 받는 규칙이 바뀌었어요." });
  assert.equal(refused?.line, null);
  assert.equal(refused?.error, "이 작품 폴더에 받는 규칙이 바뀌었어요.");
});
