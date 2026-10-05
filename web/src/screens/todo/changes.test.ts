import assert from "node:assert/strict";
import { test } from "node:test";

import {
  changesView,
  clock,
  cutLines,
  dialogueRows,
  exactClock,
  LINES_STEP,
  needsLines,
  NOT_COMPARED_YET,
  planChanges,
  timingRows,
  todoTags,
  type ChangesView,
  type ItemView,
} from "./changes.ts";
import type { Comparison, DialogueLine, ReplacementChanges, TimingLine } from "./replacementTypes.ts";

type Compared = Extract<Comparison, { state: "compared" }>;

/** A comparison of two ASS files; the parts a test cares about are overridden. */
const compared = (over: Partial<Compared> = {}): Comparison => ({
  state: "compared",
  current: { format: "ASS", encoding: "UTF-8", cues: 40 },
  new: { format: "ASS", encoding: "UTF-8", cues: 42 },
  dialogue: { added: 2, changed: 12, removed: 0 },
  timing: { count: 3 },
  styles: {
    added: ["Sign"],
    removed: [],
    changed: [{ name: "Default", fields: [{ field: "Fontname", old: "Arial", new: "Noto Sans CJK KR" }] }],
  },
  fonts: { added: ["Noto Sans CJK KR"], removed: ["Arial"] },
  not_compared: [],
  ...over,
});

const noChange = (): Comparison =>
  compared({
    dialogue: { added: 0, changed: 0, removed: 0 },
    timing: { count: 0 },
    styles: { added: [], removed: [], changed: [] },
    fonts: { added: [], removed: [] },
  });

function items(view: ChangesView): ItemView[] {
  assert.equal(view.kind, "items");
  return view.kind === "items" ? view.items : [];
}

const texts = (item: ItemView) => item.tags.map((t) => t.text);
const byKey = (view: ChangesView, key: string) => {
  const item = items(view).find((i) => i.key === key);
  assert.ok(item, `item ${key}`);
  return item;
};

const changes = (over: Partial<ReplacementChanges> = {}): ReplacementChanges => ({
  added: 0,
  changed: 0,
  removed: 0,
  timing: 0,
  styles: 0,
  fonts: 0,
  uncompared: 0,
  partial: 0,
  plans: 1,
  ...over,
});

// ---- the 교체 승인 card

test("the card's tags come in the spec's order and only for numbers that are not 0", () => {
  const all = changes({ added: 2, changed: 12, removed: 1, timing: 3, styles: 4, fonts: 5 });
  assert.deepEqual(todoTags(all), ["대사 추가 2", "대사 변경 12", "대사 삭제 1", "타이밍 3", "스타일 4", "폰트 5"]);
  assert.deepEqual(todoTags(changes({ fonts: 1, changed: 1 })), ["대사 변경 1", "폰트 1"]);
});

test("a card with nothing changed and nothing uncompared says 차이 없음", () => {
  assert.deepEqual(todoTags(changes()), ["차이 없음"]);
  assert.deepEqual(todoTags(changes({ plans: 4 })), ["차이 없음"]);
});

test("one plan's part of the card is counted as the server sums it", () => {
  assert.deepEqual(planChanges(compared()), changes({ added: 2, changed: 12, timing: 3, styles: 2, fonts: 2 }));
  assert.deepEqual(planChanges(null), changes({ uncompared: 1 }));
  assert.deepEqual(planChanges({ state: "unreadable", reason: "새 자막: 인코딩을 알 수 없어요" }), changes({ uncompared: 1 }));
  const srt = { format: "SRT", encoding: "UTF-8", cues: 40 } as const;
  // An ASS against an SRT: its styles and fonts were left out.
  assert.equal(planChanges(compared({ new: srt, styles: null, fonts: null })).partial, 1);
  // Two SRTs have no styles to leave out.
  assert.equal(planChanges(compared({ current: srt, new: srt, styles: null, fonts: null })).partial, 0);
  assert.equal(planChanges(compared({ not_compared: [{ item: "dialogue", reason: "ENCC" }] })).partial, 1);
});

test("a card of no open plan has no tag, not 차이 없음", () => {
  assert.deepEqual(todoTags(changes({ plans: 0 })), []);
});

test("a card never says 차이 없음 for a plan that was not compared", () => {
  assert.deepEqual(todoTags(changes({ uncompared: 1 })), ["비교 불가"]);
  assert.deepEqual(todoTags(changes({ uncompared: 3, plans: 3 })), ["비교 불가"]);
});

test("a card never says 차이 없음 for a plan compared only in part", () => {
  // An ASS against an SRT with the same dialogue: its styles and fonts were not compared.
  assert.deepEqual(todoTags(changes({ partial: 1 })), ["일부 비교 불가"]);
  assert.deepEqual(todoTags(changes({ timing: 2, partial: 1, plans: 3 })), ["타이밍 2", "일부 비교 불가 1"]);
  assert.deepEqual(todoTags(changes({ partial: 1, uncompared: 1, plans: 2 })), ["일부 비교 불가 1", "비교 불가 1"]);
});

test("the card counts the plans that could not be compared only when others were compared", () => {
  assert.deepEqual(todoTags(changes({ added: 2, uncompared: 1, plans: 3 })), ["대사 추가 2", "비교 불가 1"]);
  // Two compared plans that change nothing and one that was not compared.
  assert.deepEqual(todoTags(changes({ uncompared: 1, plans: 3 })), ["비교 불가 1"]);
  assert.deepEqual(todoTags(changes({ uncompared: 2, plans: 2 })), ["비교 불가"]);
});

// ---- the items

test("a compared plan has the four items, collapsed, with their number tags", () => {
  const view = changesView(compared());
  assert.deepEqual(
    items(view).map((i) => i.name),
    ["대사", "타이밍", "스타일", "폰트"],
  );
  assert.deepEqual(texts(byKey(view, "dialogue")), ["추가 2", "변경 12", "삭제 0"]);
  assert.deepEqual(texts(byKey(view, "timing")), ["바뀐 줄 3"]);
  assert.deepEqual(texts(byKey(view, "styles")), ["추가 1", "변경 1", "삭제 0"]);
  assert.deepEqual(texts(byKey(view, "fonts")), ["추가 1", "삭제 1"]);
});

test("the dialogue's tags show the zeros, dimmed", () => {
  const dialogue = byKey(changesView(compared()), "dialogue");
  assert.deepEqual(
    dialogue.tags.map((t) => t.dim),
    [false, false, true],
  );
});

test("an item with nothing to show does not expand, and one with something does", () => {
  const view = changesView(compared({ dialogue: { added: 0, changed: 0, removed: 0 }, timing: { count: 0 } }));
  assert.equal(byKey(view, "dialogue").detail, null);
  assert.equal(byKey(view, "timing").detail, null);
  assert.deepEqual(byKey(view, "styles").detail?.kind, "styles");
  assert.deepEqual(byKey(view, "fonts").detail?.kind, "fonts");
  for (const item of items(changesView(noChange()))) assert.equal(item.detail, null, item.key);
});

test("a compared plan with no difference shows zeros and nothing to expand, never 비교 불가", () => {
  const all = items(changesView(noChange()));
  assert.equal(all.length, 4);
  for (const item of all) {
    assert.ok(!texts(item).includes("비교 불가"), item.key);
    assert.ok(item.tags.every((t) => t.dim), item.key);
  }
});

test("styles and fonts keep their names and each changed style's fields", () => {
  const view = changesView(compared());
  assert.deepEqual(byKey(view, "styles").detail, {
    kind: "styles",
    added: ["Sign"],
    changed: [{ name: "Default", fields: [{ field: "Fontname", old: "Arial", new: "Noto Sans CJK KR" }] }],
    removed: [],
  });
  assert.deepEqual(byKey(view, "fonts").detail, { kind: "fonts", added: ["Noto Sans CJK KR"], removed: ["Arial"] });
});

test("styles and fonts that could not be compared say 비교 불가 and why, not zeros", () => {
  const reason = "SRT에는 스타일과 폰트가 없어 ASS와 비교하지 못했어요";
  const view = changesView(
    compared({
      styles: null,
      fonts: null,
      not_compared: [
        { item: "styles", reason },
        { item: "fonts", reason },
      ],
    }),
  );
  for (const key of ["styles", "fonts"]) {
    const item = byKey(view, key);
    assert.deepEqual(texts(item), ["비교 불가"]);
    assert.deepEqual(item.detail, { kind: "reason", reason });
  }
  // The dialogue and timing of the same plan were compared, and say so.
  assert.deepEqual(texts(byKey(view, "dialogue")), ["추가 2", "변경 12", "삭제 0"]);
});

test("an item that is null with no reason from the API still says it was not compared", () => {
  const view = changesView(compared({ styles: null, fonts: null }));
  assert.deepEqual(texts(byKey(view, "styles")), ["비교 불가"]);
  assert.equal(byKey(view, "styles").detail?.kind, "reason");
  assert.deepEqual(texts(byKey(view, "fonts")), ["비교 불가"]);
});

test("a language the dialogue could not pair keeps the counts, adds a tag, and says why when expanded", () => {
  const reason = "SMI 언어 ENCC는 상대 쪽에 대응하는 언어가 없어 비교하지 못했어요";
  const view = changesView(compared({ not_compared: [{ item: "dialogue", reason }] }));
  assert.deepEqual(texts(byKey(view, "dialogue")), ["추가 2", "변경 12", "삭제 0", "일부 비교 불가"]);
  assert.deepEqual(byKey(view, "dialogue").detail, { kind: "dialogue", notes: [reason] });
  // The timing lines are of the tracks that were compared: no tag and nothing to say.
  assert.deepEqual(texts(byKey(view, "timing")), ["바뀐 줄 3"]);
  assert.deepEqual(byKey(view, "timing").detail, { kind: "timing" });
  // With no line to list, the reason alone is enough to expand.
  const quiet = changesView(
    compared({
      dialogue: { added: 0, changed: 0, removed: 0 },
      timing: { count: 0 },
      not_compared: [{ item: "dialogue", reason }],
    }),
  );
  assert.deepEqual(byKey(quiet, "dialogue").detail, { kind: "dialogue", notes: [reason] });
});

test("a plan made before contents were compared has one 비교 불가 block and no items", () => {
  assert.deepEqual(changesView(null), { kind: "none", reason: NOT_COMPARED_YET });
  assert.equal(NOT_COMPARED_YET, "이 계획은 내용을 비교하기 전에 만들어졌어요");
});

test("an unreadable plan has one 비교 불가 block with the API's reason and no items", () => {
  const view = changesView({ state: "unreadable", reason: "새 자막: 이미지 자막이라 내용을 비교할 수 없어요" });
  assert.deepEqual(view, { kind: "none", reason: "새 자막: 이미지 자막이라 내용을 비교할 수 없어요" });
});

test("only the dialogue and the timing need the lines", () => {
  assert.deepEqual(
    (["dialogue", "timing", "styles", "fonts"] as const).map(needsLines),
    [true, true, false, false],
  );
});

// ---- times

test("a time is m:ss, and h:mm:ss from an hour", () => {
  assert.equal(clock(0), "0:00");
  assert.equal(clock(5_999), "0:05");
  assert.equal(clock(62_300), "1:02");
  assert.equal(clock(3_599_999), "59:59");
  assert.equal(clock(3_600_000), "1:00:00");
  assert.equal(clock(3_723_000), "1:02:03");
});

test("a timing time keeps the hour and the milliseconds", () => {
  assert.equal(exactClock(62_300), "0:01:02.300");
  assert.equal(exactClock(62_800), "0:01:02.800");
  assert.equal(exactClock(3_723_007), "1:02:03.007");
  assert.equal(exactClock(0), "0:00:00.000");
});

// ---- dialogue rows

const placed = (text: string, start: number, end = start + 1500) => ({ text, start, end });

test("a changed line has its text on both rows and the new line's start", () => {
  const [row] = dialogueRows([
    { kind: "changed", old: placed("안녕", 12_000), new: placed("안녕하세요", 12_000) },
  ]);
  assert.deepEqual(row?.old, { bar: false, text: "안녕" });
  assert.deepEqual(row?.new, { bar: false, text: "안녕하세요" });
  assert.equal(row?.time, "0:12");
  assert.equal(row?.class, null);
});

test("an added line's old row is a bar, and a removed line's new row is a bar", () => {
  const rows = dialogueRows([
    { kind: "added", new: placed("네", 5_000) },
    { kind: "removed", old: placed("아니", 3_723_000), class: "KRCC" },
  ]);
  assert.deepEqual(rows[0]?.old, { bar: true });
  assert.deepEqual(rows[0]?.new, { bar: false, text: "네" });
  assert.equal(rows[0]?.time, "0:05");
  assert.deepEqual(rows[1]?.old, { bar: false, text: "아니" });
  assert.deepEqual(rows[1]?.new, { bar: true });
  assert.equal(rows[1]?.time, "1:02:03");
  assert.equal(rows[1]?.class, "KRCC");
});

test("a changed line that moved shows both starts", () => {
  const [row] = dialogueRows([{ kind: "changed", old: placed("a", 12_000), new: placed("b", 13_400) }]);
  assert.equal(row?.time, "0:12 → 0:13");
});

test("multi-line texts keep their line breaks", () => {
  const [row] = dialogueRows([{ kind: "changed", old: placed("첫째\n둘째", 0), new: placed("첫째\n셋째", 0) }]);
  assert.deepEqual(row?.old, { bar: false, text: "첫째\n둘째" });
  assert.deepEqual(row?.new, { bar: false, text: "첫째\n셋째" });
});

test("dialogue rows have distinct keys and keep the API's order", () => {
  const lines: DialogueLine[] = [
    { kind: "added", new: placed("a", 2_000) },
    { kind: "added", new: placed("a", 2_000) },
    { kind: "removed", old: placed("b", 1_000) },
  ];
  const rows = dialogueRows(lines);
  assert.equal(new Set(rows.map((r) => r.key)).size, 3);
  assert.deepEqual(
    rows.map((r) => r.time),
    ["0:02", "0:02", "0:01"],
  );
});

// ---- timing rows

const timing = (over: Partial<TimingLine> = {}): TimingLine => ({
  text: "응",
  old: { start: 62_300, end: 64_000 },
  new: { start: 62_800, end: 64_000 },
  ...over,
});

test("a timing line says only what moved, from and to", () => {
  const [row] = timingRows([timing()]);
  assert.deepEqual(row?.moves, [{ name: "시작", from: "0:01:02.300", to: "0:01:02.800" }]);
  const [end] = timingRows([timing({ new: { start: 62_300, end: 64_500 } })]);
  assert.deepEqual(end?.moves, [{ name: "끝", from: "0:01:04.000", to: "0:01:04.500" }]);
  const [both] = timingRows([timing({ new: { start: 62_800, end: 64_500 } })]);
  assert.deepEqual(
    both?.moves.map((m) => m.name),
    ["시작", "끝"],
  );
});

test("a timing line leaves out an end that moved no more than the engine's tolerance", () => {
  const [row] = timingRows([timing({ new: { start: 62_800, end: 64_010 } })]);
  assert.deepEqual(
    row?.moves.map((m) => m.name),
    ["시작"],
  );
});

test("a timing line keeps its text and class", () => {
  const [row] = timingRows([timing({ text: "여러 줄\n텍스트", class: "ENCC" })]);
  assert.equal(row?.text, "여러 줄\n텍스트");
  assert.equal(row?.class, "ENCC");
  assert.equal(timingRows([timing()])[0]?.class, null);
});

// ---- the first 200 lines

test("the first 200 lines show, and the button says how many more it adds", () => {
  assert.equal(LINES_STEP, 200);
  assert.deepEqual(cutLines(150, 0), { visible: 150, more: 0 });
  assert.deepEqual(cutLines(200, 0), { visible: 200, more: 0 });
  assert.deepEqual(cutLines(201, 0), { visible: 200, more: 1 });
  assert.deepEqual(cutLines(1000, 0), { visible: 200, more: 200 });
});

test("each press of the button adds the next lines until none are left", () => {
  assert.deepEqual(cutLines(450, 400), { visible: 400, more: 50 });
  assert.deepEqual(cutLines(450, 600), { visible: 450, more: 0 });
  assert.deepEqual(cutLines(0, 0), { visible: 0, more: 0 });
});
