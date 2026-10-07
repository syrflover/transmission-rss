import assert from "node:assert/strict";
import { test } from "node:test";

import { creatorOf, headNames, isNameable, namedCreators, unknownFiles } from "./fileCreators.ts";

type File = Parameters<typeof creatorOf>[0];

const named = (name: string): File => ({ creator: { name }, applied: null });
const direct: File = { creator: null, applied: null };
/** An applied copy: the server gives `creator: null` for it, whatever the user named for the path. */
const applied = (creator: string | null): File => ({ creator: null, applied: { creator } });

/** A subscription that follows 코코렛: its line is the creator's name. */
const follow = { text: "코코렛", followed: "코코렛" };

test("a season applied only from the subscribed creator's copies shows that creator and no unknown one", () => {
  const files = [applied("코코렛"), applied("코코렛")];
  assert.deepEqual(headNames(files, follow), ["코코렛"]);
  assert.deepEqual(unknownFiles(files), []);
});

test("a subtitle put in by hand keeps the head's unknown creator, and only it is counted for `제작자 지정`", () => {
  const files = [applied("코코렛"), direct, direct];
  assert.deepEqual(headNames(files, follow), ["코코렛", "제작자 알 수 없음"]);
  assert.equal(unknownFiles(files).length, 2);
});

test("a file the user named and trss then replaced shows the applied copy's creator", () => {
  // The server sends `creator: null` and `applied` for it; the named creator no longer shows.
  const files = [applied("에루샤"), named("하느")];
  assert.equal(creatorOf(files[0]), "에루샤");
  assert.deepEqual(namedCreators(files), ["에루샤", "하느"]);
});

test("an applied copy of a stored copy with no creator shows as unknown but is not counted for naming", () => {
  const files = [applied(null)];
  assert.equal(creatorOf(files[0]), null);
  assert.deepEqual(headNames(files, undefined), ["제작자 알 수 없음"]);
  assert.deepEqual(unknownFiles(files), []);
});

test("only a file a person put there offers a creator change", () => {
  assert.equal(isNameable(direct), true);
  assert.equal(isNameable(named("하느")), true);
  assert.equal(isNameable(applied("하느")), false);
});
