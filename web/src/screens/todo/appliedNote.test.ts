import assert from "node:assert/strict";
import { test } from "node:test";

import { appliedDetail, clusterByNote } from "./appliedNote.ts";

const ADOPTED = "이 회차에 같은 자막이 이미 있어 그 파일을 적용본으로 기록했어요";
const REPLACED = "기존 자막을 새 자막으로 교체했어요";

test("an applied file says its episode and its own note, each when it has one", () => {
  assert.equal(appliedDetail("2화", ADOPTED), `2화 · ${ADOPTED}`);
  assert.equal(appliedDetail("2화", null), "2화");
  assert.equal(appliedDetail(null, REPLACED), REPLACED);
  assert.equal(appliedDetail("2화", ""), "2화");
  assert.equal(appliedDetail(null, null), null);
});

test("an episode applied by adoption is told apart from one replaced before it", () => {
  const rows = [
    { episode: "1화", note: REPLACED },
    { episode: "2화", note: ADOPTED },
  ];
  // The first episode's note is not the second's.
  assert.deepEqual(
    rows.map((r) => appliedDetail(r.episode, r.note)),
    [`1화 · ${REPLACED}`, `2화 · ${ADOPTED}`],
  );
  assert.deepEqual(
    clusterByNote(rows).map((c) => [c.note, c.rows.map((r) => r.episode)]),
    [
      [REPLACED, ["1화"]],
      [ADOPTED, ["2화"]],
    ],
  );
});

test("twelve episodes replaced the same way are one cluster, in the order they came", () => {
  const rows = Array.from({ length: 12 }, (_, i) => ({ position: i, note: i === 5 ? ADOPTED : REPLACED }));
  const clusters = clusterByNote(rows);
  assert.deepEqual(
    clusters.map((c) => [c.note, c.rows.length]),
    [
      [REPLACED, 11],
      [ADOPTED, 1],
    ],
  );
  assert.deepEqual(
    clusters[0].rows.map((r) => r.position),
    [0, 1, 2, 3, 4, 6, 7, 8, 9, 10, 11],
  );
});

test("rows with no note, or a note of no words, share one cluster without a note", () => {
  const clusters = clusterByNote([
    { id: "a", note: null },
    { id: "b", note: "" },
    { id: "c", note: ADOPTED },
  ]);
  assert.deepEqual(
    clusters.map((c) => [c.note, c.rows.map((r) => r.id)]),
    [
      [null, ["a", "b"]],
      [ADOPTED, ["c"]],
    ],
  );
  assert.deepEqual(clusterByNote([]), []);
});
