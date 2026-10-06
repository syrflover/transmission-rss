import assert from "node:assert/strict";
import { test } from "node:test";

import {
  OTHER_FORMAT,
  actionMode,
  actionName,
  actionsOf,
  applyOutcome,
  canSaveOrder,
  cardSummary,
  copyView,
  formatText,
  groupTitle,
  moveFormat,
  orderOwnerText,
  orderText,
  seasonGroups,
  type SubtitleCopy,
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
