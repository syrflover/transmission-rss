import assert from "node:assert/strict";
import { test } from "node:test";

import type { ConfirmEpisode, ConfirmView, Placement, Relocation } from "./placementTypes.ts";
import { NO_VIDEO, REPLACES } from "./placementTable.ts";
import { ALREADY_THERE, applyLine, moves, relocationLead, relocationRequest, removalLine } from "./relocation.ts";

const placement = (position: number, over: Partial<Placement> = {}): Placement => ({
  position,
  file_id: `f${position}`,
  name: `Show - 0${position}.ass`,
  kind: "subtitle",
  format: "ass",
  episode: position + 1,
  anissia_episode: String(position + 12),
  question: null,
  named: null,
  named_is_episode: false,
  assignment: "mapped",
  action: "apply",
  outcome: null,
  note: null,
  video: null,
  applied: null,
  stored: null,
  font_receipt: null,
  ...over,
});

const removal = (id: string, over: Partial<Relocation> = {}): Relocation => ({
  id,
  position: 1,
  episode: 2,
  path: "Season 1/Show S01E02.ass",
  state: "planned",
  reason: null,
  ...over,
});

const episode = (n: number, over: Partial<ConfirmEpisode> = {}): ConfirmEpisode => ({
  episode: n,
  video: `Show S01E0${n}.mkv`,
  videos: 1,
  subtitle: false,
  ...over,
});

const confirm = (positions: number[]): ConfirmView => ({
  scope: "relocate",
  positions,
  total: 12,
  episodes: [episode(1), episode(2), episode(3, { subtitle: true }), episode(4, { videos: 0, video: null })],
});

test("a planned move shows the copy taken off its old episode beside the episode it goes on", () => {
  const shown = moves([placement(1, { episode: 3 })], [removal("r1")], confirm([1]).episodes);
  assert.equal(shown.length, 1);
  assert.equal(shown[0].name, "Show - 01.ass");
  assert.deepEqual(
    shown[0].off.map((l) => [l.text, l.detail]),
    [["2화 적용본 제거", "Season 1/Show S01E02.ass"]],
  );
  // The new episode has a subtitle: applying there waits for a replacement's approval.
  assert.deepEqual([shown[0].on.text, shown[0].on.detail, shown[0].on.replaces], ["3화 적용", "Show S01E03.mkv", true]);
  assert.ok(REPLACES.length > 0);
});

test("an episode with no video says the subtitle waits for one", () => {
  const on = applyLine(placement(1, { episode: 4 }), confirm([1]).episodes);
  assert.deepEqual([on.text, on.detail, on.replaces], ["4화 적용", NO_VIDEO, false]);
});

test("moves go by their new episode, and a removal with no row says it is applied there already", () => {
  const shown = moves(
    [placement(2, { episode: 4, name: "b.ass" }), placement(1, { episode: 3, name: "a.ass" })],
    [
      removal("r2", { position: 2, episode: 3, path: "S1/b.ass" }),
      removal("r1", { position: 1, episode: 2, path: "S1/a.ass" }),
      removal("r3", { position: null, episode: 5, path: "S1/c.srt" }),
    ],
    confirm([1, 2]).episodes,
  );
  assert.deepEqual(
    shown.map((m) => [m.name, m.off.map((l) => l.text), m.on.text]),
    [
      ["a.ass", ["2화 적용본 제거"], "3화 적용"],
      ["b.ass", ["3화 적용본 제거"], "4화 적용"],
      ["c.srt", ["5화 적용본 제거"], ALREADY_THERE],
    ],
  );
});

test("after the confirmation each removal and row says what came of it", () => {
  assert.equal(removalLine(removal("r", { state: "done" })).text, "2화 적용본 지움");
  assert.equal(removalLine(removal("r", { state: "set_aside" })).text, "2화 적용본 지우는 중");
  const kept = removalLine(removal("r", { state: "kept", reason: "적용한 뒤 바뀐 파일이라 그대로 뒀어요" }));
  assert.deepEqual(
    [kept.text, kept.detail, kept.urgent],
    ["2화 적용본 그대로 둠", "Season 1/Show S01E02.ass · 적용한 뒤 바뀐 파일이라 그대로 뒀어요", false],
  );
  const held = removalLine(removal("r", { state: "held", reason: "지우지 못했어요" }));
  assert.equal(held.urgent, true);

  assert.equal(applyLine(placement(1, { episode: 3, outcome: "applied" }), null).text, "3화에 적용함");
  assert.equal(applyLine(placement(1, { episode: 3 }), null).text, "3화 적용 예정");
  assert.equal(applyLine(placement(1, { episode: 3, outcome: "failed", note: "x" }), null).urgent, true);
  const asked = applyLine(placement(1, { episode: 3, question: "바뀐 회차 대응으로 회차를 정하지 못했어요" }), null);
  assert.equal(asked.text, "3화 회차 확인 필요");
});

test("the confirmation sends every row as planned, applied, and every planned removal", () => {
  const rows = [placement(1, { episode: 3 }), placement(2, { episode: 4 }), placement(7)];
  const body = relocationRequest(rows, confirm([1, 2]), [
    removal("r1"),
    removal("r2", { position: 2 }),
    removal("old", { state: "done" }),
  ]);
  assert.deepEqual(body, {
    rows: [
      { position: 1, episode: 3, apply: true },
      { position: 2, episode: 4, apply: true },
    ],
    removals: ["r1", "r2"],
  });
});

test("the lead counts the copies the plan moves", () => {
  const lead = relocationLead([removal("r1"), removal("r2"), removal("r3", { state: "done" })]);
  assert.match(lead, /적용본 2개를 새 회차로 옮겨요/);
  assert.match(lead, /확인하기 전에는 옛 회차의 적용본을 그대로 둬요/);
});

test("a plan whose new episodes have the subtitle already says it only takes copies off", () => {
  const lead = relocationLead([removal("r1", { position: null }), removal("r2", { position: null })]);
  assert.match(lead, /옛 회차의 적용본 2개를 지워요/);
  assert.match(lead, /새 회차에는 이미 적용돼 있어요/);
  assert.doesNotMatch(lead, /붙여요/);
});
