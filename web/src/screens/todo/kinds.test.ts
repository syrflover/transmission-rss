import assert from "node:assert/strict";
import { test } from "node:test";

import { todosOfWork, type TodoOf } from "./kinds.ts";

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
