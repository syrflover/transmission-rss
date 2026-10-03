import assert from "node:assert/strict";
import { test } from "node:test";

import {
  choiceOf,
  collisions,
  continueReason,
  exceptionsOf,
  mappingText,
  misfits,
  offsetOf,
  previewOf,
  previewText,
  userLine,
  type ExceptionRow,
} from "./mapping.ts";

const twelveThen = ["13", "14", "15", "24"];

test("continuing on from a 12-episode earlier season is an offset of minus 12", () => {
  assert.equal(offsetOf("continue", "", 12), -12);
  assert.equal(choiceOf(-12, 12), "continue");
  assert.equal(choiceOf(0, 12), "same");
  assert.equal(choiceOf(-5, 12), "custom");
});

test("continuing on is not offered when the earlier seasons' episode count is unknown, and says why", () => {
  assert.equal(offsetOf("continue", "", null), null);
  assert.match(continueReason(null) ?? "", /알 수 없어서/);
  assert.match(continueReason(0) ?? "", /앞 시즌이 없어서/);
  assert.equal(continueReason(12), null);
});

test("a custom offset is any integer within the range and nothing else", () => {
  assert.equal(offsetOf("custom", "-12", null), -12);
  assert.equal(offsetOf("custom", " 3 ", null), 3);
  assert.equal(offsetOf("custom", "1.5", null), null);
  assert.equal(offsetOf("custom", "", null), null);
  assert.equal(offsetOf("custom", "10000", null), null);
});

test("the preview shows where the current episodes go under the choice", () => {
  const previews = previewOf(twelveThen, -12, [], 12);
  assert.deepEqual(
    previews.map((p) => [p.episode, p.to]),
    [
      ["13", "1화"],
      ["14", "2화"],
      ["15", "3화"],
      ["24", "12화"],
    ],
  );
  assert.equal(previewText(previews), "13화 → 1화 · 14화 → 2화 · 15화 → 3화 · 24화 → 12화");
  assert.equal(previewText(previews, 3), "13화 → 1화 · 14화 → 2화 … 24화 → 12화");
});

test("the same number puts an episode past the season's count outside the season", () => {
  assert.deepEqual(
    previewOf(["12", "13"], 0, [], 12).map((p) => p.to),
    ["12화", "시즌 밖"],
  );
});

test("a decimal episode set to not received is left, and the others follow the default", () => {
  const previews = previewOf(["12", "13", "13.5"], 0, [{ episode: "13.5", target: null }], null);
  assert.deepEqual(
    previews.map((p) => [p.episode, p.to]),
    [
      ["12", "12화"],
      ["13", "13화"],
      ["13.5", "받지 않음"],
    ],
  );
});

test("an exception is applied before the offset", () => {
  const previews = previewOf(["13", "14"], -12, [{ episode: "014", target: 5 }], 12);
  assert.deepEqual(
    previews.map((p) => p.to),
    ["1화", "5화"],
  );
});

test("the episodes that fit nowhere are offered as exceptions to write", () => {
  assert.deepEqual(misfits(["12", "13.5", "SP", "24"], -12, [], 12), ["12", "13.5", "SP"]);
});

test("an episode an exception covers is not offered again", () => {
  assert.deepEqual(misfits(["13", "13.5", "SP"], -12, [{ episode: "13.5", target: null }], 12), ["SP"]);
});

test("two exceptions of one number are refused whatever they say, with a sentence", () => {
  const rows: ExceptionRow[] = [
    { episode: "013", target: "", skip: true },
    { episode: "13", target: "2", skip: false },
  ];
  const answer = exceptionsOf(rows);
  assert.equal(answer.ok, false);
  assert.match(answer.ok ? "" : answer.message, /예외가 둘이에요/);
});

test("an exception row needs a season episode or the not-received choice, and an empty row is left out", () => {
  assert.equal(exceptionsOf([{ episode: "13.5", target: "", skip: false }]).ok, false);
  assert.equal(exceptionsOf([{ episode: "13.5", target: "0", skip: false }]).ok, false);
  assert.equal(exceptionsOf([{ episode: "", target: "2", skip: false }]).ok, false);
  const ok = exceptionsOf([
    { episode: "", target: "", skip: false },
    { episode: "13.5", target: "", skip: true },
    { episode: "14", target: "3", skip: false },
  ]);
  assert.deepEqual(ok.ok ? ok.exceptions : null, [
    { episode: "13.5", target: null },
    { episode: "14", target: 3 },
  ]);
});

test("the group line after continuing on from a 12-episode season is 직접 정함 · 13화 → 1화", () => {
  assert.equal(userLine(-12, [], twelveThen), "직접 정함 · 13화 → 1화");
});

test("the group line says 같은 번호 for an offset of zero and gives the exceptions compactly", () => {
  assert.equal(userLine(0, [], twelveThen), "직접 정함 · 같은 번호");
  assert.equal(userLine(0, [{ episode: "13.5", target: null }], twelveThen), "직접 정함 · 같은 번호 · 예외 13.5 받지 않음");
  assert.equal(
    userLine(
      0,
      [
        { episode: "13.5", target: null },
        { episode: "14", target: 3 },
        { episode: "SP", target: null },
        { episode: "OVA", target: null },
      ],
      twelveThen,
    ),
    "직접 정함 · 같은 번호 · 예외 13.5 받지 않음, 14 → 3화 외 2개",
  );
});

test("the group line of an app-decided mapping keeps its grounds, and a user's gives the exceptions", () => {
  const base = { offset: -12, evidence: "앞 시즌 12화", exceptions: [] };
  assert.equal(mappingText({ ...base, kind: "auto" }, twelveThen), "자동 · 앞 시즌 12화");
  assert.equal(mappingText({ ...base, kind: "undecided", offset: null }, twelveThen), "회차 대응 미정 · 앞 시즌 12화");
  assert.equal(mappingText({ ...base, kind: "user" }, twelveThen), "직접 정함 · 13화 → 1화");
});

test("an exception's target past the season's count is shown as received, with the count named", () => {
  const previews = previewOf(["12", "13.5"], 0, [{ episode: "13.5", target: 32 }], 12);
  assert.deepEqual(
    previews.map((p) => [p.episode, p.to, p.lands]),
    [
      ["12", "12화", true],
      ["13.5", "32화 (시즌 회차 수 밖)", false],
    ],
  );
  // What an exception covers is not offered as a misfit again.
  assert.deepEqual(misfits(["12", "13.5"], 0, [{ episode: "13.5", target: 32 }], 12), []);
});

test("the group line names the first episode that lands inside the season and no exception covers, by its number", () => {
  // 012 lands outside (0), 013 is covered by an exception, 014 is the first that lands.
  assert.equal(userLine(-12, [{ episode: "13", target: 5 }], ["012", "013", "014"], 12), "직접 정함 · 14화 → 2화 · 예외 13 → 5화");
  // The episode text is written by its number, not as posted.
  assert.equal(userLine(-12, [], ["013", "014"], 12), "직접 정함 · 13화 → 1화");
  // Past the count it does not land: the next one does.
  assert.equal(userLine(-12, [], ["30", "24"], 12), "직접 정함 · 24화 → 12화");
});

test("two episodes landing on one season episode are warned about, not refused", () => {
  const previews = previewOf(["13", "13.5"], 0, [{ episode: "13.5", target: 13 }], 24);
  assert.deepEqual(collisions(previews), ["13화에 두 회차가 들어와요 (13, 13.5)"]);
  assert.deepEqual(collisions(previewOf(["13", "14"], 0, [], 24)), []);
  // A not-received episode and an episode outside the season collide with nothing.
  assert.deepEqual(collisions(previewOf(["13", "13.5", "30"], 0, [{ episode: "13.5", target: null }], 24)), []);
});

test("the group line gives the offset when no episode shows it", () => {
  assert.equal(userLine(5, [], []), "직접 정함 · +5화 차이");
});
