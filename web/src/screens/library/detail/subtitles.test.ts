import assert from "node:assert/strict";
import { test } from "node:test";

import {
  OTHER_FORMAT,
  actionMode,
  actionName,
  actionsOf,
  APPLY_WAIT_MS,
  applyOutcome,
  applyProgress,
  appliedShown,
  canSaveOrder,
  cardSummary,
  copyView,
  formatText,
  groupTitle,
  moveFormat,
  orderOwnerText,
  orderText,
  seasonGroups,
  storedOf,
  type SubtitleCopy,
  waitOver,
  type WorkSubtitles,
} from "./subtitles.ts";

/** A moment in the viewer's own calendar, as the texts are read in it. */
const at = (month: number, day: number, hour = 12, minute = 0) =>
  new Date(2025, month - 1, day, hour, minute).getTime();

const copy = (id: string, over: Partial<SubtitleCopy> = {}): SubtitleCopy => ({
  id,
  season: 1,
  episode: "02",
  name: `Show - ${id}.ass`,
  format: "ass",
  stored_at: at(9, 7),
  stored_path: `.trss/subtitles/하느/Show - ${id}.ass`,
  applied: [],
  choice: "apply",
  can_add: false,
  blocked: null,
  ...over,
});

const subtitles = (...creators: WorkSubtitles["creators"]): WorkSubtitles => ({
  format_order: { order: ["srt", "ass", "smi"], own: true },
  creators,
});

test("a format is its code in capitals, and any other code is `그 밖의 형식`", () => {
  assert.equal(formatText("ass"), "ASS");
  assert.equal(formatText("srt"), "SRT");
  assert.equal(formatText("smi"), "SMI");
  assert.equal(formatText("other"), OTHER_FORMAT);
  assert.equal(formatText("vtt"), OTHER_FORMAT);
  assert.equal(formatText("toString"), OTHER_FORMAT);
});

test("the order is written with arrows and says whose it is", () => {
  assert.equal(orderText(["srt", "ass", "smi"]), "SRT → ASS → SMI");
  assert.equal(orderOwnerText({ order: ["srt", "ass", "smi"], own: true }), "이 작품의 순서");
  assert.equal(orderOwnerText({ order: ["ass", "srt", "smi"], own: false }), "전역 순서");
});

test("a season's groups keep the server's order and leave out the creators with no copy in it", () => {
  const all = subtitles(
    { creator: "가나", copies: [copy("a", { season: 2 })] },
    { creator: "하느", copies: [copy("b"), copy("c", { season: 2 }), copy("d")] },
    { creator: null, copies: [copy("e", { season: 3 })] },
  );
  const first = seasonGroups(all, 1);
  assert.deepEqual(first.map((g) => [g.creator, g.copies.map((c) => c.id)]), [["하느", ["b", "d"]]]);
  const second = seasonGroups(all, 2);
  assert.deepEqual(second.map((g) => [g.creator, g.copies.map((c) => c.id)]), [["가나", ["a"]], ["하느", ["c"]]]);
  assert.deepEqual(seasonGroups(all, 9), []);
});

test("an unknown creator is `제작자 알 수 없음`", () => {
  assert.equal(groupTitle({ creator: null, copies: [] }), "제작자 알 수 없음");
  assert.equal(groupTitle({ creator: "하느", copies: [] }), "하느");
});

test("the folded card counts the creators and copies of the season, or says there are none", () => {
  const all = subtitles({ creator: "하느", copies: [copy("a"), copy("b")] }, { creator: null, copies: [copy("c")] });
  const groups = seasonGroups(all, 1);
  assert.equal(cardSummary(groups), "제작자 2명 · 자막 3개");
  assert.equal(cardSummary([]), "보관한 자막 없음");
});

test("a copy is received on a day, with the time only when another of its episode and format came that day", () => {
  const one = copy("a", { stored_at: at(9, 7, 13, 5) });
  assert.equal(copyView(one, [one]).received, "9월 7일 받음");

  const other = copy("b", { stored_at: at(9, 7, 21, 40) });
  assert.equal(copyView(one, [one, other]).received, "9월 7일 13:05 받음");
  assert.equal(copyView(other, [one, other]).received, "9월 7일 21:40 받음");

  const nextDay = copy("c", { stored_at: at(9, 8) });
  assert.equal(copyView(one, [one, nextDay]).received, "9월 7일 받음");
  const otherEpisode = copy("d", { episode: "03", stored_at: at(9, 7, 21, 40) });
  assert.equal(copyView(one, [one, otherEpisode]).received, "9월 7일 받음");
  const otherFormat = copy("e", { format: "srt", stored_at: at(9, 7, 21, 40) });
  assert.equal(copyView(one, [one, otherFormat]).received, "9월 7일 받음");
  const otherSeason = copy("f", { season: 2, stored_at: at(9, 7, 21, 40) });
  assert.equal(copyView(one, [one, otherSeason]).received, "9월 7일 받음");
});

test("the same episode written `02` and `2` is one episode for the date", () => {
  const one = copy("a", { episode: "02", stored_at: at(9, 7, 9, 3) });
  const other = copy("b", { episode: "2", stored_at: at(9, 7, 18, 30) });
  assert.equal(copyView(one, [one, other]).received, "9월 7일 09:03 받음");
});

test("a copy names its episode and format", () => {
  const view = copyView(copy("a", { episode: "02", format: "smi" }), []);
  assert.equal(view.episode, "2화");
  assert.equal(view.format, "SMI");
  assert.equal(copyView(copy("b", { episode: "SP" }), []).episode, "SP");
  assert.equal(copyView(copy("c", { format: "vtt" }), []).format, OTHER_FORMAT);
});

test("a copy with no applied file is not `적용` and shows no places", () => {
  const view = copyView(copy("a"), []);
  assert.equal(view.isApplied, false);
  assert.equal(view.places, null);
});

test("the applied copy is `적용` and shows where it is applied and where it is stored, apart", () => {
  const applied = copy("a", {
    applied: [
      { path: "Season 01/Show S01E02.ass", applied_at: at(9, 8) },
      { path: "Season 01/Show S01E02.ko.ass", applied_at: at(9, 8) },
    ],
    choice: null,
  });
  const view = copyView(applied, [applied]);
  assert.equal(view.isApplied, true);
  assert.deepEqual(view.places, {
    applied: ["Season 01/Show S01E02.ass", "Season 01/Show S01E02.ko.ass"],
    stored: ".trss/subtitles/하느/Show - a.ass",
  });
});

test("the actions follow the server's choice: apply, compare, and add", () => {
  assert.deepEqual(actionsOf(copy("a", { choice: "apply" })), ["apply"]);
  assert.deepEqual(actionsOf(copy("a", { choice: "compare" })), ["compare"]);
  assert.deepEqual(actionsOf(copy("a", { choice: "compare", can_add: true })), ["compare", "add"]);
  assert.deepEqual(actionsOf(copy("a", { choice: "apply", can_add: true })), ["apply", "add"]);
  assert.deepEqual(actionsOf(copy("a", { choice: null, can_add: true })), ["add"]);
  // The applied copy itself: nothing to choose.
  assert.deepEqual(actionsOf(copy("a", { choice: null, applied: [{ path: "x.ass", applied_at: 1 }] })), []);
});

test("a blocked copy has the sentence and no buttons", () => {
  const blocked = copy("a", { choice: null, can_add: true, blocked: "영상이 아직 없어요." });
  assert.deepEqual(actionsOf(blocked), []);
  const view = copyView(blocked, [blocked]);
  assert.deepEqual(view.actions, []);
  assert.equal(view.blocked, "영상이 아직 없어요.");
});

test("an action's button is named with its copy's episode, format and when it was received", () => {
  const early = copy("a", { choice: "compare", stored_at: at(9, 7, 4, 50) });
  const late = copy("b", { choice: "compare", stored_at: at(9, 7, 21, 40) });
  const group = [early, late];
  assert.equal(actionName(copyView(early, group), "compare"), "2화 ASS 9월 7일 04:50 받음 교체 비교");
  // Two copies of one episode and format are told apart.
  assert.notEqual(actionName(copyView(early, group), "compare"), actionName(copyView(late, group), "compare"));
  assert.equal(actionName(copyView(early, [early]), "add"), "2화 ASS 9월 7일 받음 추가 적용");
});

test("`추가 적용` asks for the add mode and the other actions for the apply mode", () => {
  assert.equal(actionMode("add"), "add");
  assert.equal(actionMode("apply"), "apply");
  assert.equal(actionMode("compare"), "apply");
});

test("an answer with `compare` goes to the job's comparison; any other is only sent", () => {
  assert.deepEqual(applyOutcome({ job_id: "j1", compare: true }), { kind: "compare", job: "j1" });
  assert.deepEqual(applyOutcome({ job_id: "j2", compare: false }), { kind: "sent", job: "j2" });
});

test("a format moves one place and stays when it cannot", () => {
  const order = ["srt", "ass", "smi"] as const;
  assert.deepEqual(moveFormat(order, 1, -1), ["ass", "srt", "smi"]);
  assert.deepEqual(moveFormat(order, 1, 1), ["srt", "smi", "ass"]);
  assert.deepEqual(moveFormat(order, 0, -1), ["srt", "ass", "smi"]);
  assert.deepEqual(moveFormat(order, 2, 1), ["srt", "ass", "smi"]);
  // The order it was given is not changed.
  assert.deepEqual(order, ["srt", "ass", "smi"]);
});

test("`저장` is offered for a changed order, and for the global one even when it is not changed", () => {
  const own = { order: ["srt", "ass", "smi"] as ("srt" | "ass" | "smi")[], own: true };
  assert.equal(canSaveOrder(["srt", "ass", "smi"], own), false);
  assert.equal(canSaveOrder(["ass", "srt", "smi"], own), true);
  const global = { order: ["ass", "srt", "smi"] as ("srt" | "ass" | "smi")[], own: false };
  assert.equal(canSaveOrder(["ass", "srt", "smi"], global), true);
  assert.equal(canSaveOrder(["srt", "ass", "smi"], global), true);
});

// --- the stored copies of an episode row ----------------------------------------------------------------------------

const held = (id: string, over: { awaiting_video?: boolean; approval_job?: string | null; compare?: boolean } = {}) => ({
  id,
  awaiting_video: false,
  approval_job: null,
  compare: false,
  ...over,
});

test("a stored copy to choose is on the row of an episode that has a subtitle, as `교체 비교`", () => {
  const episode = { subtitle: [{ path: "Season 01/Show S01E02.ass" }], stored: [held("a", { compare: true })] };
  const view = storedOf(episode);
  assert.equal(view.note, true);
  assert.equal(view.cell, true);
  assert.deepEqual(view.others.map((s) => [s.id, s.compare]), [["a", true]]);
});

test("a stored copy to choose is on the row of an episode with no subtitle too", () => {
  const episode = { subtitle: [], stored: [held("a")] };
  const view = storedOf(episode);
  assert.equal(view.note, true);
  assert.equal(view.cell, true);
});

test("copies waiting for the video or the approval stay in their own cells and not in the stored one", () => {
  const episode = {
    subtitle: [{ path: "x.ass" }],
    stored: [held("v", { awaiting_video: true }), held("p", { approval_job: "j1" })],
  };
  const view = storedOf(episode);
  assert.deepEqual(view.awaiting.map((s) => s.id), ["v"]);
  assert.deepEqual(view.approval.map((s) => s.id), ["p"]);
  assert.deepEqual(view.others, []);
  assert.equal(view.note, false);
  assert.equal(view.cell, false);
  // Beside one to choose, the others are still separate.
  const mixed = storedOf({ stored: [held("v", { awaiting_video: true }), held("c"), held("p", { approval_job: "j1" })] });
  assert.deepEqual(mixed.others.map((s) => s.id), ["c"]);
  assert.equal(mixed.note, true);
});

test("an episode with no stored copy says nothing", () => {
  const view = storedOf({ stored: [] });
  assert.equal(view.note, false);
  assert.equal(view.cell, false);
});

// --- waiting for the job that applies a copy ------------------------------------------------------------------------

test("a job that was only queued or is running is waited for, and the work is read once it is done", () => {
  // The server turns the job back to `pending` before it answers, so the first reads are not the end.
  assert.deepEqual(applyProgress({ state: "pending", note: "고른 보관본을 적용해요" }), { kind: "running" });
  assert.deepEqual(applyProgress({ state: "running", note: null }), { kind: "running" });
  assert.deepEqual(applyProgress({ state: "done", note: null }), { kind: "done" });
});

test("a failed job says why in its own sentence, and a plain one when it wrote none", () => {
  assert.deepEqual(applyProgress({ state: "failed", note: "영상 옆에 같은 이름의 파일이 있어요." }), {
    kind: "failed",
    text: "영상 옆에 같은 이름의 파일이 있어요.",
  });
  const plain = { kind: "failed", text: "적용하지 못했어요. 작업에서 까닭을 볼 수 있어요." };
  assert.deepEqual(applyProgress({ state: "failed", note: null }), plain);
  assert.deepEqual(applyProgress({ state: "failed", note: "  " }), plain);
  assert.equal(applyProgress({ state: "partial", note: null }).kind, "failed");
});

test("a job that stops for a person ends the wait and says what it waits for", () => {
  assert.deepEqual(applyProgress({ state: "waiting", note: "교체 승인을 기다려요." }), {
    kind: "waiting",
    text: "교체 승인을 기다려요.",
  });
  assert.equal(applyProgress({ state: "held", note: null }).kind, "waiting");
});

test("a state this build does not know is waited for, not taken as the end", () => {
  assert.deepEqual(applyProgress({ state: "paused", note: null }), { kind: "running" });
});

test("the wait is over after five minutes and not before", () => {
  assert.equal(APPLY_WAIT_MS, 300_000);
  assert.equal(waitOver(1000, 1000), false);
  assert.equal(waitOver(1000, 1000 + APPLY_WAIT_MS - 1), false);
  assert.equal(waitOver(1000, 1000 + APPLY_WAIT_MS), true);
});

// --- waiting for the work to show a finished apply (0127) -----------------------------------------------------------

/** A work whose season 1 has episodes with these subtitle files, and these stored copies. */
const workWith = (files: string[][], ...creators: WorkSubtitles["creators"]) => ({
  seasons: [{ episodes: files.map((paths) => ({ subtitle: paths.map((path) => ({ path })) })) }],
  subtitles: subtitles(...creators),
});

const applied = (path: string) => ({ path, applied_at: at(10, 8) });

test("a finished apply shows once the episode lists the file the job put beside the video, and not before", () => {
  const copies = [copy("c1", { episode: "09", applied: [applied("Season 01/Show S01E09.ass")], choice: null })];
  // Read right after the job ended: the job's record has the copy applied, the watch folder has not found the file.
  assert.equal(appliedShown(workWith([[], []], { creator: null, copies }), "c1"), false);
  assert.equal(appliedShown(workWith([["Season 01/Show S01E08.ass"], []], { creator: null, copies }), "c1"), false);
  assert.equal(appliedShown(workWith([[], ["Season 01/Show S01E09.ass"]], { creator: null, copies }), "c1"), true);
});

test("a copy applied in two places shows once both are listed", () => {
  const copies = [
    copy("c1", { applied: [applied("Season 01/Show S01E02.ass"), applied("Season 01/Show S01E02.ko.ass")] }),
  ];
  const card = { creator: "하느", copies };
  assert.equal(appliedShown(workWith([["Season 01/Show S01E02.ass"]], card), "c1"), false);
  assert.equal(
    appliedShown(workWith([["Season 01/Show S01E02.ass", "Season 01/Show S01E02.ko.ass"]], card), "c1"),
    true,
  );
});

test("a copy that is gone or was left stored, and a work with no card, leave nothing to wait for", () => {
  const stored = { creator: null, copies: [copy("c1")] };
  assert.equal(appliedShown(workWith([[]], stored), "c1"), true);
  assert.equal(appliedShown(workWith([[]], stored), "cleaned"), true);
  assert.equal(appliedShown({ seasons: [{ episodes: [{ subtitle: [] }] }] }, "c1"), true);
});
