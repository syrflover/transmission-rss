import assert from "node:assert/strict";
import { test } from "node:test";

import { WEEKDAYS_LONG, WEEKDAYS_SHORT, weekdayLongFromMonday } from "./weekday.ts";

test("the weekdays are named from Sunday, the way Date.getDay() and Anissia count them", () => {
  assert.deepEqual(WEEKDAYS_SHORT, ["일", "월", "화", "수", "목", "금", "토"]);
  assert.deepEqual(WEEKDAYS_LONG, ["일요일", "월요일", "화요일", "수요일", "목요일", "금요일", "토요일"]);
  // 2026-10-10 is a Saturday.
  assert.equal(WEEKDAYS_SHORT[new Date(2026, 9, 10).getDay()], "토");
});

test("a number outside the week has no name, and each caller says what it shows then", () => {
  assert.equal(WEEKDAYS_SHORT[7], undefined);
  assert.equal(WEEKDAYS_LONG[-1], undefined);
  assert.equal(WEEKDAYS_LONG[8] ?? "?요일", "?요일");
});

test("the schedule counts its weekdays from Monday and has no name outside 0 to 6", () => {
  assert.deepEqual(
    [0, 1, 2, 3, 4, 5, 6].map(weekdayLongFromMonday),
    ["월요일", "화요일", "수요일", "목요일", "금요일", "토요일", "일요일"],
  );
  for (const outside of [-1, 7, 8, 1.5, Number.NaN]) assert.equal(weekdayLongFromMonday(outside), undefined);
});
