import assert from "node:assert/strict";
import { test } from "node:test";

import type { ConfirmEpisode, ConfirmView, Placement } from "./placementTypes.ts";
import {
  NO_VIDEO,
  REPLACES,
  STORE_ONLY,
  initialChoice,
  initialChoices,
  leadText,
  mappedNote,
  requestRows,
  restSummary,
  tableRows,
  unsetCount,
  videoText,
} from "./placementTable.ts";

const placement = (position: number, over: Partial<Placement> = {}): Placement => ({
  position,
  file_id: `f${position}`,
  name: `file${position}.ass`,
  kind: "subtitle",
  format: "ass",
  episode: position,
  anissia_episode: null,
  question: null,
  named: null,
  named_is_episode: false,
  assignment: null,
  action: "apply",
  outcome: null,
  note: null,
  video: null,
  applied: null,
  stored: null,
  font_receipt: null,
  ...over,
});

const episode = (n: number, over: Partial<ConfirmEpisode> = {}): ConfirmEpisode => ({
  episode: n,
  video: `Show S01E0${n}.mkv`,
  videos: 1,
  subtitle: false,
  ...over,
});

const confirm = (positions: number[], over: Partial<ConfirmView> = {}): ConfirmView => ({
  scope: "whole",
  positions,
  total: 3,
  episodes: [episode(1), episode(2), episode(3)],
  ...over,
});

test("the table holds the asked rows, held ones first, then by episode and name", () => {
  const rows = [
    placement(1, { episode: 3, name: "b.ass" }),
    placement(2, { episode: 1, name: "z.ass" }),
    placement(3, { episode: 1, name: "a.ass" }),
    placement(4, {
      episode: null,
      question: "회차를 알 수 없어요",
      name: "q.ass",
    }),
    placement(5, { episode: null, action: "apply", name: "none.ass" }),
    placement(6, { episode: 2, name: "not asked.ass" }),
  ];
  const shown = tableRows(rows, confirm([1, 2, 3, 4, 5]));
  assert.deepEqual(
    shown.map((p) => p.position),
    [4, 3, 2, 1, 5],
  );
});

test("a file name with numbers sorts by number", () => {
  const rows = [placement(1, { episode: 1, name: "ep10.ass" }), placement(2, { episode: 1, name: "ep9.ass" })];
  assert.deepEqual(
    tableRows(rows, confirm([1, 2])).map((p) => p.position),
    [2, 1],
  );
});

test("initial choices: planned episode, skip for a store row, unset for a question", () => {
  assert.equal(initialChoice(placement(1, { episode: 2 })), 2);
  assert.equal(initialChoice(placement(1, { action: "store", episode: 2, note: "다른 형식" })), "skip");
  assert.equal(initialChoice(placement(1, { action: "store", episode: null })), "skip");
  assert.equal(initialChoice(placement(1, { episode: null, question: "회차를 알 수 없어요" })), "unset");
  assert.equal(initialChoice(placement(1, { episode: 2, question: "두 회차로 읽혀요" })), "unset");
  assert.equal(initialChoice(placement(1, { action: "apply", episode: null })), "unset");
});

test("unsetCount counts the rows nobody chose", () => {
  const rows = [
    placement(1),
    placement(2, { question: "?", episode: null }),
    placement(3, { question: "?", episode: null }),
  ];
  const choices = initialChoices(rows);
  assert.equal(unsetCount(choices), 2);
  choices.set(2, "skip");
  choices.set(3, 1);
  assert.equal(unsetCount(choices), 0);
});

test("the request sends an episode applied, and a skipped row on its planned episode", () => {
  const rows = [
    placement(1, { episode: 1 }),
    placement(2, { action: "store", episode: 2 }),
    placement(3, { action: "store", episode: null }),
    placement(4, { episode: null, question: "?" }),
  ];
  const choices = initialChoices(rows);
  assert.equal(requestRows(rows, choices), null);
  choices.set(4, 3);
  assert.deepEqual(requestRows(rows, choices), [
    { position: 1, episode: 1, apply: true },
    { position: 2, episode: 2, apply: false },
    { position: 3, episode: null, apply: false },
    { position: 4, episode: 3, apply: true },
  ]);
  choices.set(1, "skip");
  choices.set(4, "skip");
  assert.deepEqual(
    requestRows(rows, choices)?.filter((r) => r.position === 1 || r.position === 4),
    [
      { position: 1, episode: 1, apply: false },
      { position: 4, episode: null, apply: false },
    ],
  );
});

test("the video text of a choice", () => {
  const episodes = [
    episode(1),
    episode(2, { video: null, videos: 0 }),
    episode(3, { video: null, videos: 2 }),
    episode(4, { subtitle: true }),
  ];
  assert.deepEqual(videoText(1, episodes), {
    text: "Show S01E01.mkv",
    replaces: false,
  });
  assert.deepEqual(videoText(2, episodes), { text: NO_VIDEO, replaces: false });
  assert.equal(NO_VIDEO, "영상 없음 · 영상이 생기면 붙여요");
  assert.deepEqual(videoText(3, episodes), {
    text: "영상 2개",
    replaces: false,
  });
  assert.deepEqual(videoText(4, episodes), {
    text: "Show S01E04.mkv",
    replaces: true,
  });
  assert.deepEqual(videoText("skip", episodes), {
    text: STORE_ONLY,
    replaces: false,
  });
  assert.equal(STORE_ONLY, "보관만 해요");
  assert.equal(videoText("skip", episodes).replaces, false);
  assert.equal(videoText("unset", episodes).replaces, false);
  assert.equal(videoText(9, episodes).text, "—");
  assert.equal(REPLACES, "이미 자막이 있어 교체를 비교해요");
});

test("a mapped row says what its name said while it stays on the planned episode", () => {
  const p = placement(1, { episode: 1, named: "13", assignment: "mapped" });
  assert.equal(mappedNote(p, 1), "이름 13 → 1화 (회차 대응)");
  assert.equal(mappedNote(p, 2), null);
  assert.equal(mappedNote(p, "skip"), null);
  // The server says the name's number is the episode (`01` on 1): there is nothing to explain.
  assert.equal(
    mappedNote(placement(1, { episode: 1, named: "01", named_is_episode: true, assignment: "mapped" }), 1),
    null,
  );
  assert.equal(mappedNote(placement(1, { episode: 1, named: "13", assignment: "explicit" }), 1), null);
  assert.equal(mappedNote(placement(1, { episode: 1, named: null, assignment: "mapped" }), 1), null);
});

test("the rest of a whole plan: kept files by kind and dropped files with why", () => {
  const rows = [
    placement(1),
    placement(2, {
      kind: "font",
      format: null,
      episode: null,
      name: "a.ttf",
      action: "store",
    }),
    placement(3, {
      kind: "font",
      format: null,
      episode: null,
      name: "b.otf",
      action: "store",
    }),
    placement(4, {
      kind: "attachment",
      format: null,
      episode: null,
      name: "readme.txt",
      action: "store",
    }),
    placement(5, {
      kind: "other",
      format: null,
      episode: null,
      name: "x.exe",
      action: "drop",
      note: "자막이 아니에요",
    }),
  ];
  assert.deepEqual(restSummary(rows, confirm([1])), {
    kept: [
      { kind: "폰트", names: ["a.ttf", "b.otf"] },
      { kind: "첨부", names: ["readme.txt"] },
    ],
    dropped: [{ name: "x.exe", note: "자막이 아니에요" }],
  });
  assert.deepEqual(restSummary(rows, confirm([1], { scope: "held" })), {
    kept: [],
    dropped: [],
  });
});

test("a whole plan with no subtitle has no rows, sends no rows and blocks nothing", () => {
  const rows = [
    placement(1, {
      kind: "font",
      format: null,
      episode: null,
      name: "a.ttf",
      action: "store",
    }),
    placement(2, {
      kind: "attachment",
      format: null,
      episode: null,
      name: "readme.txt",
      action: "store",
    }),
  ];
  const view = confirm([]);
  const table = tableRows(rows, view);
  assert.deepEqual(table, []);
  const choices = initialChoices(table);
  assert.equal(unsetCount(choices), 0);
  assert.deepEqual(requestRows(table, choices), []);
  assert.deepEqual(restSummary(rows, view).kept, [
    { kind: "폰트", names: ["a.ttf"] },
    { kind: "첨부", names: ["readme.txt"] },
  ]);
});

test("the lead says the plan chose the episodes only when it chose some", () => {
  const planned = placement(1, { episode: 1 });
  const held = placement(2, {
    episode: null,
    question: "파일 이름에 회차 번호가 없어요",
  });
  assert.match(leadText([planned], confirm([1])), /^파일 이름의 번호로 회차를 정했어요\. 확인하고/);
  assert.match(leadText([planned, held], confirm([1, 2])), /^파일 이름의 번호로 회차를 정했어요\. 정하지 못한 파일은/);
  assert.match(leadText([held], confirm([2])), /^파일 이름으로 회차를 정하지 못했어요\./);
  assert.match(leadText([held], confirm([2], { scope: "held" })), /^이 파일들은 회차를 직접 골라야 해요\./);
  assert.match(leadText([], confirm([])), /^받은 폰트와 첨부를/);
  for (const lead of [leadText([planned], confirm([1])), leadText([], confirm([]))]) {
    assert.match(lead, /확인하기 전에는 작품 폴더에 아무것도 쓰지 않아요\.$/);
  }
});
