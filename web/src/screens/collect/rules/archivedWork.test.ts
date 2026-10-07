import assert from "node:assert/strict";
import { test } from "node:test";

import { archivedWorkNotice, archivedWorkPath, receivesPastItemsNow, type ArchivedWork } from "./archivedWork.ts";

const archived = (merges: boolean): ArchivedWork => ({
  work: "Clevatess",
  archive_folder: "/srv/media/Shows",
  collect_folder: "/srv/media/Shows (current)",
  merges,
});

test("the typed folder is sent as typed, and nothing is asked while none is typed", () => {
  assert.equal(archivedWorkPath("Clevatess/Season 03"), "/rules/archived-work?directory=Clevatess%2FSeason%2003");
  assert.equal(archivedWorkPath("  ./Clevatess/Season 03 "), "/rules/archived-work?directory=.%2FClevatess%2FSeason%2003");
  assert.equal(archivedWorkPath("/abs/Clevatess"), "/rules/archived-work?directory=%2Fabs%2FClevatess");
  assert.equal(archivedWorkPath(""), null);
  assert.equal(archivedWorkPath("   "), null);
});

test("the form of a stored rule sends the folder the rule has now", () => {
  assert.equal(
    archivedWorkPath("Clevatess/Season 04", " Other/Season 01 "),
    "/rules/archived-work?directory=Clevatess%2FSeason%2004&from=Other%2FSeason%2001",
  );
  assert.equal(archivedWorkPath("A", null), "/rules/archived-work?directory=A");
});

test("only a rule that came back collecting receives past items at once", () => {
  assert.equal(receivesPastItemsNow("active"), true);
  assert.equal(receivesPastItemsNow("paused"), false);
  assert.equal(receivesPastItemsNow("archived"), false);
});

test("the notice names the folder to move and where it goes", () => {
  const { title, detail } = archivedWorkNotice(archived(false));
  assert.equal(title, "보관 폴더의 ‘Shows/Clevatess’를 수집 폴더(Shows (current))로 옮겨요.");
  assert.ok(detail.includes("아무것도 옮기지 않고"));
  assert.ok(!detail.includes("합쳐요"));
});

test("the notice says the two folders are merged when the collect folder has the work too", () => {
  const { detail } = archivedWorkNotice(archived(true));
  assert.ok(detail.includes("‘Clevatess’가 있어서 두 폴더를 하나로 합쳐요"));
});
