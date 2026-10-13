import assert from "node:assert/strict";
import { test } from "node:test";

import { folderOf, joinDirectory, matchingWorks, parseDirectory, suggestionPath } from "./saveFolder.ts";

test("only the form the fields write is read back into them", () => {
  assert.deepEqual(parseDirectory("Nige Jouzu no Wakagimi/Season 02"), { work: "Nige Jouzu no Wakagimi", season: 2 });
  assert.deepEqual(parseDirectory("Show/Season 00"), { work: "Show", season: 0 });
  // Another form keeps its own text: saving it unchanged must not rewrite it.
  for (const other of ["Show/Season 1", "Show/Season 100", "Show/Season 01/", "A/B/Season 01", " Show/Season 01", "", "./Season 01", "Show"]) {
    assert.equal(parseDirectory(other), null, other);
  }
  assert.equal(joinDirectory(" Show ", 3), "Show/Season 03");
});

test("the fields make a folder only from a single folder name and a season from 0 to 99", () => {
  assert.deepEqual(folderOf("Show", "2"), { directory: "Show/Season 02", problem: null });
  assert.deepEqual(folderOf("Show", " 12 "), { directory: "Show/Season 12", problem: null });
  for (const [work, season] of [
    ["", "1"],
    ["A/B", "1"],
    ["..", "1"],
    ["Show", ""],
    ["Show", "100"],
    ["Show", "-1"],
    ["Show", "1.5"],
  ]) {
    const made = folderOf(work, season);
    assert.equal(made.directory, "", `${work} ${season}`);
    assert.notEqual(made.problem, null, `${work} ${season}`);
  }
});

test("a work is found by its folder name or its title, whatever the case and spacing", () => {
  const works = [
    { name: "Nige Jouzu no Wakagimi", title: "도망을 잘 치는 도련님" },
    { name: "Clevatess", title: "Clevatess" },
  ];
  assert.deepEqual(matchingWorks(works, "nige  JOUZU").map((w) => w.name), ["Nige Jouzu no Wakagimi"]);
  assert.deepEqual(matchingWorks(works, "도련님").map((w) => w.name), ["Nige Jouzu no Wakagimi"]);
  assert.deepEqual(matchingWorks(works, "  "), []);
  assert.equal(matchingWorks(works, "e", 1).length, 1);
});

test("a suggestion is asked with the anime and its start date when there is one", () => {
  assert.equal(
    suggestionPath({ release: "Show S2" }, { anime_no: 7, start_date: "2026-01" }),
    "/save-folder/suggestion?release=Show+S2&anime_no=7&start=2026-01",
  );
  assert.equal(suggestionPath({ work: "Show" }, { anime_no: 7, start_date: null }), "/save-folder/suggestion?work=Show&anime_no=7");
  assert.equal(suggestionPath({ work: "A&B" }, null), "/save-folder/suggestion?work=A%26B");
});
