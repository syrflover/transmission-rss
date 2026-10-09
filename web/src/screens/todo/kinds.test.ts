import assert from "node:assert/strict";
import { test } from "node:test";

import { todosOfWork, withKinds, type TodoKind, type TodoOf } from "./kinds.ts";

const of = (kind: string, work: string | null): TodoOf & { key: string } => ({
  key: `${kind}:${work}`,
  work: work === null ? null : { id: work },
});

// As the server orders them: the red kinds first.
const LIST = [
  of("auth", "w2"),
  of("receive_failed", "w1"),
  of("receive_failed", "w1"),
  of("replacement", "w1"),
  of("replacement", null),
  of("episode_check", "w3"),
  of("placement_check", "w3"),
  of("placement_check", "w1"),
  of("video_check", "w4"),
];

test("a work's to-dos are its own, in the list's order", () => {
  assert.deepEqual(
    todosOfWork(LIST, "w1").map((t) => t.key),
    ["receive_failed:w1", "receive_failed:w1", "replacement:w1", "placement_check:w1"],
  );
  assert.deepEqual(todosOfWork(LIST, "w9"), []);
});

test("kept works take the badges of the list read again, and the unchanged ones stay the same objects", () => {
  const a = { id: "a", todos: ["replacement"] as TodoKind[] };
  const b = { id: "b", todos: ["auth"] as TodoKind[] };
  const c = { id: "c", todos: [] as TodoKind[] };
  const works = [a, b, c];

  // a's 교체 승인 was decided; c has a new check.
  const next = withKinds(works, new Map<string, TodoKind[]>([["b", ["auth"]], ["c", ["episode_check"]]]));

  assert.deepEqual(next, [
    { id: "a", todos: [] },
    { id: "b", todos: ["auth"] },
    { id: "c", todos: ["episode_check"] },
  ]);
  assert.equal(next[1], b);
  // Nothing changed: the same list.
  assert.equal(withKinds(works, new Map([["a", ["replacement"]], ["b", ["auth"]]])), works);
});
