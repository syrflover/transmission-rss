import assert from "node:assert/strict";
import { test } from "node:test";

import { nextMove, type Move, type MoveEvent } from "./move.ts";

/** The move after `events` in turn, from none. */
const after = (...events: MoveEvent<string>[]): Move<string> | null =>
  events.reduce<Move<string> | null>((move, event) => nextMove(move, event), null);

test("a person's switch lasts through the old page, its end and the new connection until the new page is drawn", () => {
  const asked = after({ type: "asked", on: "A" });
  assert.deepEqual(asked, { kind: "asked", from: "A" });
  // The old page goes on drawing while the worker moves the screen.
  assert.deepEqual(nextMove(asked, { type: "drawn", on: "A" }), asked);
  const ended = nextMove(asked, { type: "ended", on: "A", reason: "run" });
  assert.deepEqual(ended, { kind: "asked", from: null });
  assert.equal(nextMove(ended, { type: "drawn", on: "B" }), null);
});

test("a binding the server ended, or a page that went, is a move until another socket draws", () => {
  const ended = after({ type: "ended", on: "A", reason: "run" });
  assert.deepEqual(ended, { kind: "server", from: null });
  // The new binding ended too before it drew: still the same move.
  assert.deepEqual(nextMove(ended, { type: "ended", on: "B", reason: "run" }), ended);
  assert.equal(nextMove(ended, { type: "drawn", on: "C" }), null);
  // A page that closed itself goes before the worker moves the screen to the newest page left.
  assert.deepEqual(after({ type: "ended", on: "A", reason: "browser" }), { kind: "server", from: null });
});

test("closing the page shown ends its socket as browser, which goes on with the person's move", () => {
  const closing = after({ type: "asked", on: "A" }, { type: "ended", on: "A", reason: "browser" });
  assert.deepEqual(closing, { kind: "asked", from: null });
  assert.equal(nextMove(closing, { type: "drawn", on: "B" }), null);
});

test("a failed ask takes the move back only while its socket is open", () => {
  assert.equal(after({ type: "asked", on: "A" }, { type: "refused", on: "A" }), null);
  // The binding changed while the ask was on its way: the screen moves anyway.
  assert.deepEqual(after({ type: "asked", on: "A" }, { type: "ended", on: "A", reason: "run" }, { type: "refused", on: "A" }), {
    kind: "asked",
    from: null,
  });
  const served = after({ type: "ended", on: "A", reason: "run" });
  assert.deepEqual(nextMove(served, { type: "refused", on: "A" }), served);
});

test("another end, a lost socket or the limit ends the move so the screen says what happened", () => {
  for (const reason of ["unreachable", "stuck", "replaced"] as const) {
    assert.equal(after({ type: "asked", on: "A" }, { type: "ended", on: "A", reason }), null, reason);
  }
  assert.equal(after({ type: "asked", on: "A" }, { type: "lost" }), null);
  const asked = after({ type: "asked", on: "A" })!;
  assert.equal(nextMove(asked, { type: "expired", move: asked }), null);
  // A timer of an earlier step of the move does not end the later one.
  const ended = nextMove(asked, { type: "ended", on: "A", reason: "run" });
  assert.deepEqual(nextMove(ended, { type: "expired", move: asked }), ended);
});

test("no move is made by frames or a refusal alone", () => {
  assert.equal(after({ type: "drawn", on: "A" }), null);
  assert.equal(after({ type: "refused", on: "A" }), null);
});
