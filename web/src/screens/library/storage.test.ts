import assert from "node:assert/strict";
import { test } from "node:test";

import {
  CHANGED_MESSAGE,
  GONE_NOTICE,
  NO_CREATOR,
  NO_EPISODE,
  canClean,
  cleanBody,
  cleanFailure,
  cleanableCount,
  cleanableText,
  cleaningText,
  confirmView,
  creatorText,
  episodeText,
  filesPath,
  groupByKind,
  isCleaning,
  kindTexts,
  receivedText,
  sentNotice,
  storageSignature,
  storageSummary,
  totalText,
  type CleanableEntry,
  type StorageOverview,
  type WorkStorage,
} from "./storage.ts";

/** A moment in the viewer's own calendar, as the texts are read in it. */
const at = (month: number, day: number, hour = 12, minute = 0) => new Date(2025, month - 1, day, hour, minute).getTime();

const entry = (id: string, over: Partial<CleanableEntry> = {}): CleanableEntry => ({
  id,
  name: `Show - ${id}.ass`,
  season: 1,
  episode: 2,
  creator: "하느",
  format: "ass",
  size: 23456,
  stored_at: at(9, 7),
  kind: "past",
  blocked: null,
  with: [{ id: `a-${id}`, name: `Show - ${id}.ass`, kind: "subtitle", size: 23456 }],
  kept: [],
  ...over,
});

test("the stored total is read in KB and MB", () => {
  assert.equal(totalText(1234567), "보관한 자막·폰트·첨부 1.2 MB");
  assert.equal(totalText(512), "보관한 자막·폰트·첨부 512 B");
  assert.equal(totalText(340 * 1024), "보관한 자막·폰트·첨부 340 KB");
});

test("an episode is `2화`, with its season only when there are several, and `회차 없음` without one", () => {
  assert.equal(episodeText(entry("a"), 1), "2화");
  assert.equal(episodeText(entry("a", { season: 2, episode: 3 }), 2), "시즌 2 3화");
  assert.equal(episodeText(entry("a", { episode: null }), 1), NO_EPISODE);
  assert.equal(episodeText(entry("a", { episode: null }), 3), NO_EPISODE);
});

test("a creator that is not known reads `제작자 알 수 없음`", () => {
  assert.equal(creatorText("하느"), "하느");
  assert.equal(creatorText(null), NO_CREATOR);
  assert.equal(creatorText(""), NO_CREATOR);
});

test("a received date has no time unless another copy of the episode came the same day", () => {
  const a = entry("a", { stored_at: at(9, 7, 9, 5) });
  const b = entry("b", { stored_at: at(9, 7, 21, 30) });
  const other = entry("c", { episode: 3, stored_at: at(9, 7, 22, 0) });
  const later = entry("d", { stored_at: at(9, 8, 9, 0) });
  assert.equal(receivedText(a, [a]), "9월 7일 받음");
  assert.equal(receivedText(a, [a, other, later]), "9월 7일 받음");
  assert.equal(receivedText(a, [a, b]), "9월 7일 09:05 받음");
  assert.equal(receivedText(b, [a, b]), "9월 7일 21:30 받음");
});

test("two copies of one episode in different seasons or unplaced copies of different names are not the same copy", () => {
  const a = entry("a", { season: 1, episode: 1, stored_at: at(10, 1, 8, 0) });
  const b = entry("b", { season: 2, episode: 1, stored_at: at(10, 1, 9, 0) });
  assert.equal(receivedText(a, [a, b]), "10월 1일 받음");
  const u = entry("u", { episode: null, name: "x.ass", kind: "unplaced", stored_at: at(10, 1, 8, 0) });
  const v = entry("v", { episode: null, name: "y.ass", kind: "unplaced", stored_at: at(10, 1, 9, 0) });
  const w = entry("w", { episode: null, name: "x.ass", kind: "unplaced", stored_at: at(10, 1, 10, 0) });
  assert.equal(receivedText(u, [u, v]), "10월 1일 받음");
  assert.equal(receivedText(u, [u, v, w]), "10월 1일 08:00 받음");
});

test("entries group by kind in a fixed order and sort by episode, then by when they were stored", () => {
  const groups = groupByKind([
    entry("u1", { kind: "unplaced", episode: null }),
    entry("p3", { kind: "past", episode: 3 }),
    entry("s1", { kind: "stored", episode: 1 }),
    entry("p2b", { kind: "past", episode: 2, stored_at: at(9, 9) }),
    entry("p2a", { kind: "past", episode: 2, stored_at: at(9, 8) }),
    entry("w1", { kind: "awaiting_video", episode: 5 }),
  ]);
  assert.deepEqual(
    groups.map((g) => [g.kind, g.label]),
    [
      ["past", "지난 수정본"],
      ["awaiting_video", "영상 대기"],
      ["stored", "보관만 함"],
      ["unplaced", "회차에 붙지 않음"],
    ],
  );
  assert.deepEqual(groups[0].entries.map((e) => e.id), ["p2a", "p2b", "p3"]);
  assert.deepEqual(groupByKind([]), []);
  assert.deepEqual(groupByKind([entry("s", { kind: "stored" })]).map((g) => g.kind), ["stored"]);
});

test("a blocked entry has no `정리`", () => {
  assert.equal(canClean(entry("a")), true);
  assert.equal(canClean(entry("a", { blocked: "진행 중인 작업이 이 보관본을 써요" })), false);
});

test("a clean in progress says so, and one that was held says why", () => {
  assert.equal(cleaningText({ id: "c", name: "a.ass", state: "asked", reason: null }), "정리하는 중");
  assert.equal(
    cleaningText({ id: "c", name: "a.ass", state: "held", reason: "기록과 내용이 달라 지우지 않았어요" }),
    "기록과 내용이 달라 지우지 않았어요",
  );
  assert.equal(cleaningText({ id: "c", name: "a.ass", state: "held", reason: null }), "정리하지 않고 보류했어요.");
});

test("the page keeps reading the work only while a clean is asked", () => {
  const storage = (cleaning: WorkStorage["cleaning"]): WorkStorage => ({ total: 1, cleanable: [], cleaning });
  assert.equal(isCleaning(storage([])), false);
  assert.equal(isCleaning(storage([{ id: "c", name: "a", state: "held", reason: "x" }])), false);
  assert.equal(isCleaning(storage([{ id: "c", name: "a", state: "held", reason: "x" }, { id: "d", name: "b", state: "asked", reason: null }])), true);
});

test("the storage signature changes with the total, the entries and the states", () => {
  const base: WorkStorage = { total: 10, cleanable: [entry("a")], cleaning: [] };
  assert.equal(storageSignature(base), storageSignature({ ...base }));
  assert.notEqual(storageSignature(base), storageSignature({ ...base, total: 9 }));
  assert.notEqual(storageSignature(base), storageSignature({ ...base, cleanable: [] }));
  assert.notEqual(storageSignature(base), storageSignature({ ...base, cleanable: [entry("a", { blocked: "x" })] }));
  const asked: WorkStorage = { ...base, cleaning: [{ id: "c", name: "a", state: "asked", reason: null }] };
  assert.notEqual(storageSignature(asked), storageSignature({ ...asked, cleaning: [{ id: "c", name: "a", state: "held", reason: "x" }] }));
});

test("the confirmation names every file to delete with its size, their total and the files that stay", () => {
  const e = entry("a", {
    with: [
      { id: "s1", name: "Show - 02.ass", kind: "subtitle", size: 23456 },
      { id: "f1", name: "Only.ttf", kind: "font", size: 1024 * 1024 },
      { id: "t1", name: "logo.png", kind: "attachment", size: 500 },
      { id: "c1", name: "Show.sub", kind: "companion", size: 2000 },
    ],
    kept: [{ id: "f2", name: "A.ttf", kind: "font", reason: "다른 자막도 이 파일을 써요" }],
  });
  const view = confirmView(e);
  assert.equal(view.name, "Show - a.ass");
  assert.deepEqual(
    view.deletes.map((d) => [d.id, d.name, d.kindLabel, d.sizeText]),
    [
      ["s1", "Show - 02.ass", "자막", "23 KB"],
      ["f1", "Only.ttf", "폰트", "1 MB"],
      ["t1", "logo.png", "첨부", "500 B"],
      ["c1", "Show.sub", "구성 파일", "2 KB"],
    ],
  );
  assert.equal(view.totalText, "1 MB");
  assert.deepEqual(view.kept, [{ id: "f2", name: "A.ttf", kindLabel: "폰트", reason: "다른 자막도 이 파일을 써요" }]);
  assert.equal(view.subtitleStays, false);
  assert.equal(view.blocked, null);
});

test("a subtitle that another stored copy shares stays, and the confirmation says so once", () => {
  const view = confirmView(entry("a", { with: [{ id: "f1", name: "Only.ttf", kind: "font", size: 100 }] }));
  assert.equal(view.subtitleStays, true);
  // The server names it among the kept files with why: the kept line says it.
  const kept = confirmView(
    entry("a", {
      with: [{ id: "f1", name: "Only.ttf", kind: "font", size: 100 }],
      kept: [{ id: "s1", name: "Show - 02.ass", kind: "subtitle", reason: "다른 보관본이 같은 파일을 써요" }],
    }),
  );
  assert.equal(kept.subtitleStays, false);
  assert.deepEqual(kept.kept.map((k) => k.kindLabel), ["자막"]);
  assert.equal(view.totalText, "100 B");
  assert.equal(confirmView(entry("a", { with: [] })).totalText, "0 B");
});

test("a blocked entry's confirmation carries why it cannot be cleaned", () => {
  assert.equal(confirmView(entry("a", { blocked: "진행 중인 작업이 이 보관본을 써요" })).blocked, "진행 중인 작업이 이 보관본을 써요");
});

test("the request sends exactly the ids the confirmation listed, in its order", () => {
  const e = entry("a", {
    with: [
      { id: "s1", name: "a.ass", kind: "subtitle", size: 1 },
      { id: "f1", name: "a.ttf", kind: "font", size: 1 },
    ],
    kept: [{ id: "f2", name: "b.ttf", kind: "font", reason: "x" }],
  });
  assert.deepEqual(cleanBody(e), { assets: ["s1", "f1"] });
  assert.deepEqual(cleanBody(e).assets, confirmView(e).deletes.map((d) => d.id));
});

test("a conflict with the new entry shows that entry in the confirmation", () => {
  const current = entry("a", { with: [{ id: "s2", name: "a.ass", kind: "subtitle", size: 5 }] });
  const result = cleanFailure({ code: "conflict", message: CHANGED_MESSAGE, current });
  assert.equal(result.kind, "changed");
  if (result.kind === "changed") assert.deepEqual(cleanBody(result.entry), { assets: ["s2"] });
});

test("a conflict that is the reason it cannot be cleaned shows that reason", () => {
  const result = cleanFailure({ code: "conflict", message: "진행 중인 작업이 이 보관본을 써요" });
  assert.deepEqual(result, { kind: "refused", message: "진행 중인 작업이 이 보관본을 써요" });
});

test("a changed conflict without a readable new entry reads the work again", () => {
  assert.deepEqual(cleanFailure({ code: "conflict", message: CHANGED_MESSAGE }), { kind: "reload", message: CHANGED_MESSAGE });
  assert.deepEqual(cleanFailure({ code: "conflict", message: CHANGED_MESSAGE, current: { id: 3 } }), {
    kind: "reload",
    message: CHANGED_MESSAGE,
  });
  assert.deepEqual(cleanFailure({ code: "conflict", message: CHANGED_MESSAGE, current: null }).kind, "reload");
});

test("a stored file that is gone closes the confirmation, and other errors show their sentence", () => {
  assert.deepEqual(cleanFailure({ code: "not_found", message: "없어요" }), { kind: "gone", message: GONE_NOTICE });
  assert.deepEqual(cleanFailure({ code: "network", message: "서버에 연결하지 못했어요." }), {
    kind: "message",
    message: "서버에 연결하지 못했어요.",
  });
  assert.deepEqual(cleanFailure({ code: "invalid", message: "고를 수 없어요." }), { kind: "message", message: "고를 수 없어요." });
});

test("a work's kinds read `자막 12개 · 340 KB` in a fixed order, and a kind with no file is left out", () => {
  assert.deepEqual(
    kindTexts([
      { kind: "cover", count: 1, size: 80 * 1024 },
      { kind: "subtitle", count: 12, size: 340 * 1024 },
      { kind: "font", count: 0, size: 0 },
      { kind: "attachment", count: 2, size: 2048 },
    ]),
    [
      { kind: "subtitle", text: "자막 12개 · 340 KB" },
      { kind: "attachment", text: "첨부 2개 · 2 KB" },
      { kind: "cover", text: "표지 1개 · 80 KB" },
    ],
  );
  assert.deepEqual(kindTexts([]), []);
});

test("the cleanable count is only said when there is one", () => {
  assert.equal(cleanableText(3), "정리할 파일 3개");
  assert.equal(cleanableText(0), null);
});

test("the settings summary counts the works and their stored size, and has none without a work", () => {
  const work = (id: string, total: number, cleanable: number) => ({ id, name: id, total, kinds: [], cleanable });
  const overview: StorageOverview = { works: [work("a", 1024 * 1024, 2), work("b", 1024 * 1024, 0)] };
  assert.equal(storageSummary(overview), "작품 2개 · 2 MB");
  assert.equal(cleanableCount(overview), 2);
  assert.equal(storageSummary({ works: [] }), null);
  assert.equal(cleanableCount({ works: [] }), 0);
});

test("a work's 파일 card has its own address", () => {
  assert.equal(filesPath("/library/w1"), "/library/w1#files");
});

test("the page says a clean ended once neither the stored file nor the clean is listed", () => {
  const sent = { id: "c1", entryId: "a", name: "Show - 01.ass" };
  const storage = (over: Partial<WorkStorage>): WorkStorage => ({ total: 0, cleanable: [], cleaning: [], ...over });
  assert.equal(sentNotice(null, storage({})), null);
  // Not read since: the page still lists the stored file.
  assert.equal(sentNotice(sent, storage({ cleanable: [entry("a")] })), null);
  // The worker has not carried it out, or held it: its row says so.
  assert.equal(sentNotice(sent, storage({ cleaning: [{ id: "c1", name: "Show - 01.ass", state: "asked", reason: null }] })), null);
  assert.equal(
    sentNotice(sent, storage({ cleaning: [{ id: "c1", name: "Show - 01.ass", state: "held", reason: "기록과 내용이 달라 지우지 않았어요" }] })),
    null,
  );
  assert.equal(sentNotice(sent, storage({ cleanable: [entry("b")] })), "Show - 01.ass 정리를 마쳤어요.");
});
