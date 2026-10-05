import assert from "node:assert/strict";
import { test } from "node:test";

import type { Replacement, ReplacementVersion } from "./replacementTypes.ts";
import {
  againNotice,
  cardLayout,
  compareRows,
  limitNotices,
  openOnes,
  pathRows,
  pathWarnings,
  postLabel,
  stateLine,
  versionFacts,
  waitingPositions,
} from "./replacementView.ts";

const NOW = Date.UTC(2026, 9, 5, 3, 0); // 2026-10-05 12:00 in Seoul

const version = (over: Partial<ReplacementVersion> = {}): ReplacementVersion => ({
  received_at: NOW - 3_600_000,
  changed_at: null,
  size: 22 * 1024,
  lines: 24,
  creator: "제작자A",
  format: "ass",
  post: "https://blog.example/post/1",
  encoding: "UTF-8",
  managed: true,
  path: "/v/01.ass",
  stored: null,
  ...over,
});

const plan = (over: Partial<Replacement> = {}): Replacement => ({
  plan_id: "p1",
  version: 1,
  position: 0,
  state: "open",
  reason: null,
  new_revision: false,
  season: 1,
  episode: 3,
  again: null,
  current: version({ stored: "/v/.trss/a.ass" }),
  new: version({ received_at: NOW - 60_000, lines: 26 }),
  side_by_side: false,
  paths: [{ path: "/v/01.ass", action: "replace", managed: true, warning: null }],
  limits: [],
  ...over,
});

test("a version line has the time, size and lines, and leaves out what is not known", () => {
  assert.deepEqual(versionFacts(version(), NOW), ["오늘 11:00 받음", "22 KB", "대사 24줄"]);
  assert.deepEqual(versionFacts(version({ lines: null }), NOW), ["오늘 11:00 받음", "22 KB"]);
});

test("an unmanaged current file says the file's own time, not a receipt", () => {
  const unmanaged = version({ received_at: null, changed_at: NOW - 86_400_000, managed: false });
  assert.deepEqual(versionFacts(unmanaged, NOW), ["파일 시각 어제 12:00", "22 KB", "대사 24줄"]);
  assert.deepEqual(versionFacts(version({ received_at: null }), NOW), ["22 KB", "대사 24줄"]);
});

test("the same creator, post and format revision shows nothing beside the version lines", () => {
  const same = plan();
  assert.deepEqual(compareRows(same), []);
  assert.deepEqual(limitNotices(same), []);
  assert.deepEqual(pathWarnings(same.paths), []);
  assert.equal(againNotice(same), null);
});

test("the side-by-side rows appear only when the API says so", () => {
  const other = plan({
    side_by_side: true,
    new: version({ creator: "제작자B", format: "srt", post: "https://other.example/p/9/" }),
  });
  const rows = compareRows(other);
  assert.deepEqual(
    rows.map((r) => r.name),
    ["제작자", "형식", "게시물", "인코딩"],
  );
  assert.deepEqual([rows[0].current, rows[0].next], ["제작자A", "제작자B"]);
  assert.deepEqual([rows[1].current, rows[1].next], ["ASS", "SRT"]);
  assert.equal(rows[2].link, true);
  assert.equal(compareRows({ ...other, side_by_side: false }).length, 0);
});

test("an unmanaged current file's creator reads 출처 미상, a stored one without a creator 제작자 알 수 없음", () => {
  const unmanaged = plan({
    side_by_side: true,
    current: version({ creator: null, managed: false, received_at: null, changed_at: NOW, post: null, encoding: null }),
  });
  const rows = compareRows(unmanaged);
  assert.equal(rows[0].current, "출처 미상");
  assert.equal(rows[2].current, "—");
  const stored = plan({ side_by_side: true, new: version({ creator: null }) });
  assert.equal(compareRows(stored)[0].next, "제작자 알 수 없음");
});

test("only a path with a warning is a warning, and it names the exact path", () => {
  const warnings = pathWarnings([
    { path: "/v/01.ass", action: "replace", managed: false, warning: "overwrite_unmanaged" },
    { path: "/v/01.srt", action: "remove", managed: true, warning: "remove_applied" },
    { path: "/v/01.smi", action: "keep", managed: true, warning: null },
  ]);
  assert.deepEqual(
    warnings.map((w) => w.path),
    ["/v/01.ass", "/v/01.srt"],
  );
  assert.match(warnings[0].text, /제작자 알 수 없음/);
  assert.match(warnings[0].text, /다시 고를 수 있어요/);
  assert.match(warnings[1].text, /보관본은 그대로 남아요/);
});

test("limits show only the ones that hold, and say which file's lines are unknown", () => {
  assert.deepEqual(limitNotices(plan({ limits: [] })), []);
  assert.deepEqual(
    limitNotices(plan({ limits: ["unknown_source"] })).map((n) => n.title),
    ["출처 미상"],
  );
  const lines = (r: Replacement) => limitNotices({ ...r, limits: ["lines_unknown"] })[0].text;
  assert.match(lines(plan({ new: version({ lines: null }) })), /^새 자막의 대사 줄 수/);
  assert.match(lines(plan({ current: version({ lines: null }) })), /^현재 자막의 대사 줄 수/);
  assert.match(lines(plan({ current: version({ lines: null }), new: version({ lines: null }) })), /^현재 자막과 새 자막의/);
});

test("a comparison made again says 다시 비교 필요, and 새 수정본 발견 for a newer revision", () => {
  assert.deepEqual(againNotice(plan({ again: { reason: "영상이 바뀌었어요", new_revision: false } })), {
    tags: ["다시 비교 필요"],
    reason: "영상이 바뀌었어요",
  });
  assert.deepEqual(againNotice(plan({ again: { reason: "새 수정본", new_revision: true } }))?.tags, [
    "새 수정본 발견",
    "다시 비교 필요",
  ]);
});

test("only an open plan has the card; the others have one short line", () => {
  assert.equal(stateLine(plan({ state: "open" })), null);
  assert.equal(stateLine(plan({ state: "approved" }))?.label, "승인한 교체를 반영하는 중이에요");
  assert.equal(stateLine(plan({ state: "kept" }))?.label, "현재 자막을 유지했어요");
  assert.equal(stateLine(plan({ state: "done" }))?.label, "새 자막으로 교체했어요");
  const revision = stateLine(plan({ state: "stale", new_revision: true, reason: "같은 출처의 새 수정본이 들어왔어요" }));
  assert.equal(revision?.label, "새 수정본 발견 · 다시 비교 필요");
  assert.match(revision?.detail ?? "", /같은 출처의 새 수정본이 들어왔어요/);
  assert.deepEqual(stateLine(plan({ state: "stale", reason: "영상이 바뀌었어요" })), {
    label: "다시 비교 필요",
    detail: "영상이 바뀌었어요",
    urgent: false,
  });
  assert.deepEqual(stateLine(plan({ state: "held", reason: "확인하지 못했어요" })), {
    label: "보류",
    detail: "확인하지 못했어요",
    urgent: false,
  });
  assert.equal(stateLine(plan({ state: "failed", reason: "공간이 없어요" }))?.urgent, true);
});

test("the open plans are the decisions, in episode order, and their rows are the waiting positions", () => {
  const list = [
    plan({ plan_id: "b", episode: 5, position: 4 }),
    plan({ plan_id: "a", episode: 2, position: 1 }),
    plan({ plan_id: "c", episode: 3, position: 2, state: "kept" }),
  ];
  assert.deepEqual(
    openOnes(list).map((r) => r.plan_id),
    ["a", "b"],
  );
  assert.deepEqual([...waitingPositions(list)].sort(), [1, 4]);
  assert.equal(waitingPositions([plan({ state: "approved" })]).size, 0);
});

test("the phone's buttons are fixed only for the one decision, and only the first card follows the page", () => {
  assert.deepEqual(cardLayout(1, 0), { sticky: true, fixed: true });
  assert.deepEqual(cardLayout(2, 0), { sticky: true, fixed: false });
  assert.deepEqual(cardLayout(2, 1), { sticky: false, fixed: false });
  // Nothing to decide: no fixed buttons.
  assert.equal(cardLayout(openOnes([plan({ state: "kept" })]).length, 0).fixed, false);
});

test("the paths part names each path's action and the stored copy that stays", () => {
  const rows = pathRows(
    plan({
      paths: [
        { path: "/v/01.ass", action: "replace", managed: true, warning: null },
        { path: "/v/01.srt", action: "remove", managed: true, warning: "remove_applied" },
      ],
    }),
  );
  assert.deepEqual(
    rows.map((r) => [r.term, r.path]),
    [
      ["교체", "/v/01.ass"],
      ["제거", "/v/01.srt"],
      ["현재 보관본", "/v/.trss/a.ass"],
    ],
  );
  assert.equal(rows[2].note, "그대로 남아요");
  // Without a stored copy (a file the app did not manage) there is no such row; a stale plan has no paths.
  assert.equal(pathRows(plan({ current: version({ stored: null }) })).length, 1);
  assert.deepEqual(pathRows(plan({ state: "stale" })), []);
});

test("a done plan says what it did, and the stored copy it names is the previous subtitle's", () => {
  const rows = pathRows(plan({ state: "done" }));
  assert.deepEqual(
    rows.map((r) => [r.term, r.note]),
    [
      ["교체함", null],
      ["이전 보관본", "그대로 남아요"],
    ],
  );
  // Keeping the current subtitle changed no path.
  assert.deepEqual(pathRows(plan({ state: "kept" })), []);
});

test("a post's link text drops the scheme and a trailing slash", () => {
  assert.equal(postLabel("https://blog.example/post/1/"), "blog.example/post/1");
  assert.equal(postLabel("http://x.test"), "x.test");
});
